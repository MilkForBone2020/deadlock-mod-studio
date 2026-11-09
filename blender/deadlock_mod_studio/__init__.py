# SPDX-License-Identifier: GPL-3.0-or-later
"""Deadlock Mod Studio: edit a hero model extracted by the app, check it, and
send it back to the app to build.

The app writes `dms_project.json` next to an extracted hero and opens Blender
with it. This add-on imports the hero, keeps a copy of the original as a
reference, and saves a work file. The Deadlock tab in the 3D view's sidebar
checks the model against the game's rules and sends it back.

Nothing here touches the game's files: it reads the project folder and writes
only into the project's `blender` and `return` folders.
"""

import hashlib
import json
import os
import time

import bpy
from bpy.props import EnumProperty, StringProperty

from . import fit
from mathutils import Vector

ADDON_VERSION = "0.1.8"
MAX_WEIGHTS = 4
# Fairness rule: a custom model stays close to the original's size. Smaller
# is the unfair direction; accessories that stick out make you easier to see.
SIZE_TOLERANCE = 0.10
SIZE_GROWTH_LIMIT = 0.40
BONE_MOVE_TOLERANCE = 0.001  # metres (about 0.04 game units)
MODEL_COLLECTION = "Your model"
REFERENCE_COLLECTION = "Reference (original)"
INCOMING_COLLECTION = "Incoming model"
OFF_COLLECTION = "Hero parts switched off"

_project_cache = {}


# ---------------------------------------------------------------- project ---

def project_path(scene=None):
    scene = scene or bpy.context.scene
    return scene.get("dms_project", "")


def load_project(scene=None):
    """The project description written by the app, or None."""
    path = project_path(scene)
    if not path or not os.path.isfile(path):
        return None
    mtime = os.path.getmtime(path)
    cached = _project_cache.get(path)
    if cached and cached[0] == mtime:
        return cached[1]
    with open(path, "r", encoding="utf-8") as f:
        data = json.load(f)
    _project_cache[path] = (mtime, data)
    return data


def rig(scene=None):
    scene = scene or bpy.context.scene
    name = scene.get("dms_rig", "")
    obj = bpy.data.objects.get(name)
    if obj and obj.type == "ARMATURE":
        return obj
    for o in scene.objects:
        if o.type == "ARMATURE":
            return o
    return None


def model_meshes(scene=None):
    """Mesh objects that make up my model (not the reference copy)."""
    scene = scene or bpy.context.scene
    coll = bpy.data.collections.get(MODEL_COLLECTION)
    if coll is None:
        return []
    return [o for o in coll.all_objects if o.type == "MESH"]


def reference_meshes():
    coll = bpy.data.collections.get(REFERENCE_COLLECTION)
    return [o for o in coll.all_objects if o.type == "MESH"] if coll else []


def part_names(project):
    return [p["name"] for p in project.get("parts", [])] if project else []


def bounds(objects):
    """World-space bounding box (min, max) of mesh objects as posed and
    deformed by the rig, or None."""
    lo = Vector((1e9, 1e9, 1e9))
    hi = Vector((-1e9, -1e9, -1e9))
    found = False
    dg = bpy.context.evaluated_depsgraph_get()
    for o in objects:
        ev = o.evaluated_get(dg)
        for corner in ev.bound_box:
            p = ev.matrix_world @ Vector(corner)
            lo = Vector(map(min, lo, p))
            hi = Vector(map(max, hi, p))
            found = True
    return (lo, hi) if found else None


def _clear_scene():
    for o in list(bpy.data.objects):
        bpy.data.objects.remove(o, do_unlink=True)
    for block in (bpy.data.meshes, bpy.data.armatures, bpy.data.materials, bpy.data.actions, bpy.data.images):
        for b in list(block):
            if b.users == 0:
                block.remove(b)
    for c in list(bpy.data.collections):
        bpy.data.collections.remove(c)


def open_project(path):
    """Open a project: its work file if there is one, else import the hero."""
    with open(path, "r", encoding="utf-8") as f:
        project = json.load(f)
    work = project.get("workBlend", "")
    if work and os.path.isfile(work):
        bpy.ops.wm.open_mainfile(filepath=work)
        bpy.context.scene["dms_project"] = path
        return "opened"
    import_hero(path, project)
    if work:
        os.makedirs(os.path.dirname(work), exist_ok=True)
        bpy.ops.wm.save_as_mainfile(filepath=work)
    return "imported"


def import_hero(path, project):
    scene = bpy.context.scene
    _clear_scene()
    scene["dms_project"] = path
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=project["glb"], import_shading="NORMALS", disable_bone_shape=True, import_pack_images=False)
    new = [o for o in bpy.data.objects if o not in before]
    arm = next((o for o in new if o.type == "ARMATURE"), None)
    if arm is None:
        raise RuntimeError("The hero file has no skeleton")
    arm.name = f"{project.get('heroName') or project['hero']} rig"
    scene["dms_rig"] = arm.name

    model = bpy.data.collections.new(MODEL_COLLECTION)
    reference = bpy.data.collections.new(REFERENCE_COLLECTION)
    scene.collection.children.link(model)
    scene.collection.children.link(reference)

    prefix = project["modelName"] + "."
    parts = set(part_names(project))
    meshes = [o for o in new if o.type == "MESH"]
    for o in new:
        for c in list(o.users_collection):
            c.objects.unlink(o)
        model.objects.link(o)
    for o in meshes:
        part = o.name[len(prefix):] if o.name.startswith(prefix) else o.name
        part = part.split(".")[0]
        o.name = part
        o.data.name = part
        o["dms_part"] = part if part in parts else ""
        o["dms_hero_mesh"] = True

    # The original, kept as a see-through reference that follows the pose.
    for o in meshes:
        ref = o.copy()
        ref.data = o.data.copy()
        ref.name = f"{o.name} (original)"
        ref.display_type = "WIRE"
        ref.hide_select = True
        ref.hide_render = True
        ref["dms_ref_part"] = o.get("dms_part", "")
        for k in ("dms_part", "dms_hero_mesh"):
            if k in ref:
                del ref[k]
        reference.objects.link(ref)
    reference.hide_viewport = False
    for ref in reference.objects:
        ref.hide_set(True)

    # Rest pose, original bones locked in edit mode.
    rest_pose(arm)

    # Remember the original skeleton and size to check against.
    scene["dms_bones"] = json.dumps(
        {b.name: {"parent": b.parent.name if b.parent else "", "head": [round(v, 5) for v in b.head_local]} for b in arm.data.bones}
    )
    box = bounds(meshes)
    if box:
        scene["dms_ref_size"] = list(box[1] - box[0])
    # Thin stick bones: the default octahedrons hide the model.
    arm.data.display_type = "STICK"
    arm.show_in_front = False
    arm.lock_location = (True, True, True)
    arm.lock_rotation = (True, True, True)
    arm.lock_scale = (True, True, True)
    _lock_bones(arm, True)
    for a in bpy.data.actions:
        a.use_fake_user = True
    # Remember each material's colour image, to spot repaints later.
    mats = {}
    for m in bpy.data.materials:
        img = base_color_image(m)
        if img:
            h = hashlib.md5(bytes(img.packed_file.data)).hexdigest() if img.packed_file else file_hash(bpy.path.abspath(img.filepath))
            mats[m.name] = {"color": {"image": img.name, "hash": h}}
    scene["dms_materials"] = json.dumps(mats)
    scene["dms_materials_all"] = json.dumps(sorted(m.name for m in bpy.data.materials))
    scene["dms_imported"] = time.strftime("%Y-%m-%dT%H:%M:%S")


def rest_pose(arm):
    if arm.animation_data:
        arm.animation_data.action = None
    for b in arm.pose.bones:
        b.location = (0, 0, 0)
        b.rotation_quaternion = (1, 0, 0, 0)
        b.rotation_euler = (0, 0, 0)
        b.scale = (1, 1, 1)
    bpy.context.view_layer.update()


def _lock_bones(arm, lock):
    """Lock the original bones against edit-mode moves."""
    view = bpy.context.view_layer
    prev = view.objects.active
    view.objects.active = arm
    try:
        bpy.ops.object.mode_set(mode="EDIT")
        for eb in arm.data.edit_bones:
            eb.lock = lock
        bpy.ops.object.mode_set(mode="OBJECT")
    except RuntimeError:
        pass
    finally:
        view.objects.active = prev


# ----------------------------------------------------------------- checks ---

class Issue:
    def __init__(self, level, text, fix="", target=""):
        self.level = level  # "error" | "warn" | "info"
        self.text = text
        self.fix = fix  # operator idname suffix, or ""
        self.target = target  # object name the fix applies to


def base_color_image(mat):
    """The image plugged straight into a material's Base Color, or None."""
    if not mat or not mat.use_nodes or not mat.node_tree:
        return None
    for node in mat.node_tree.nodes:
        if node.type == "BSDF_PRINCIPLED":
            inp = node.inputs.get("Base Color")
            if inp and inp.is_linked:
                src = inp.links[0].from_node
                if src.type == "TEX_IMAGE" and src.image:
                    return src.image
    return None


def principled(mat):
    if not mat or not mat.use_nodes or not mat.node_tree:
        return None
    return next((n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED"), None)


def base_name(name):
    """A material name without Blender's .001 suffix."""
    head, dot, tail = name.rpartition(".")
    return head if dot and tail.isdigit() and len(tail) == 3 else name


def hero_material(name, scene=None):
    """Is this one of the hero's own materials (as imported)?"""
    scene = scene or bpy.context.scene
    names = json.loads(scene.get("dms_materials_all", "[]"))
    if not names:
        # Work files from before 0.1.5: the reference copy uses the hero's materials.
        names = list(json.loads(scene.get("dms_materials", "{}")))
        names += [s.material.name for o in reference_meshes() for s in o.material_slots if s.material]
    return base_name(name) in {base_name(n) for n in names}


def file_hash(path):
    try:
        with open(path, "rb") as f:
            return hashlib.md5(f.read()).hexdigest()
    except OSError:
        return ""


def image_changed(img, original_hash):
    """Has this image been painted, replaced or edited since import?"""
    if img.size[0] == 0 or img.size[1] == 0:
        return False  # no pixels loaded (file missing): nothing to send
    if img.is_dirty:
        return True
    if not original_hash:
        return False
    if img.packed_file is not None:
        return hashlib.md5(bytes(img.packed_file.data)).hexdigest() != original_hash
    return file_hash(bpy.path.abspath(img.filepath)) != original_hash


def save_image_copy(img, path):
    """Write an image's current pixels (unsaved paint included) to a new PNG
    without changing the image itself."""
    import numpy as np

    w, h = img.size
    buf = np.empty(w * h * 4, dtype=np.float32)
    img.pixels.foreach_get(buf)
    out = bpy.data.images.new("dms_export_tmp", w, h, alpha=True)
    try:
        out.colorspace_settings.name = img.colorspace_settings.name
        out.pixels.foreach_set(buf)
        out.filepath_raw = path
        out.file_format = "PNG"
        out.save()
    finally:
        bpy.data.images.remove(out)


def is_pow2(n):
    return n > 0 and (n & (n - 1)) == 0


def material_images(mat):
    if not mat or not mat.use_nodes or not mat.node_tree:
        return []
    return [n.image for n in mat.node_tree.nodes if n.type == "TEX_IMAGE" and n.image]


def signed_volume(obj):
    """Volume of a mesh; negative when its faces point inward (inside out)."""
    me = obj.data
    v = [x.co for x in me.vertices]
    total = 0.0
    for p in me.polygons:
        idx = p.vertices
        for k in range(1, len(idx) - 1):
            total += v[idx[0]].dot(v[idx[k]].cross(v[idx[k + 1]])) / 6.0
    if obj.matrix_world.determinant() < 0:
        total = -total
    return total


def fix_normals(obj):
    """Point every face outward (Blender's Recalculate Outside)."""
    import bmesh

    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bm.to_mesh(obj.data)
    bm.free()
    obj.data.update()


def weight_stats(obj, bone_names):
    """(unweighted vertices, vertices over the limit, groups that aren't bones)."""
    idx_to_name = {g.index: g.name for g in obj.vertex_groups}
    stray = sorted({g.name for g in obj.vertex_groups if g.name not in bone_names})
    unweighted = over = 0
    for v in obj.data.vertices:
        n = sum(1 for g in v.groups if g.weight > 0 and idx_to_name.get(g.group) in bone_names)
        if n == 0:
            unweighted += 1
        elif n > MAX_WEIGHTS:
            over += 1
    return unweighted, over, stray


def run_checks(scene=None):
    scene = scene or bpy.context.scene
    project = load_project(scene)
    issues = []
    if project is None:
        return [Issue("error", "No project. Open this hero from the app with Edit in Blender.")]
    arm = rig(scene)
    if arm is None:
        return [Issue("error", "The hero's skeleton (rig) is missing.")]

    # Skeleton: every original bone kept, same parent, not moved.
    original = json.loads(scene.get("dms_bones", "{}"))
    bones = arm.data.bones
    missing = [n for n in original if n not in bones]
    if missing:
        issues.append(Issue("error", f"{len(missing)} original bone(s) deleted or renamed, e.g. {', '.join(missing[:4])}"))
    reparented = [n for n, b in original.items() if n in bones and (bones[n].parent.name if bones[n].parent else "") != b["parent"]]
    if reparented:
        issues.append(Issue("error", f"{len(reparented)} bone(s) have a different parent, e.g. {', '.join(reparented[:4])}"))
    moved = [n for n, b in original.items() if n in bones and (Vector(bones[n].head_local) - Vector(b["head"])).length > BONE_MOVE_TOLERANCE]
    if moved:
        issues.append(Issue("error", f"{len(moved)} original bone(s) moved, e.g. {', '.join(moved[:4])}. Animations won't fit the model."))
    extra = [b.name for b in bones if b.name not in original]
    if extra:
        issues.append(Issue("info", f"{len(extra)} new bone(s) added (fine for cloth or jiggle parts)"))

    meshes = model_meshes(scene)
    if not meshes:
        issues.append(Issue("error", f"There's nothing in the '{MODEL_COLLECTION}' collection."))
        return issues

    bone_names = {b.name for b in bones}
    parts = set(part_names(project))
    used_parts = set()
    tris = 0
    materials = set()
    for o in meshes:
        part = o.get("dms_part", "")
        if part not in parts:
            issues.append(Issue("error", f"{o.name}: not assigned to a part", "", o.name))
        else:
            used_parts.add(part)
        mods = [m for m in o.modifiers if m.type == "ARMATURE" and m.object == arm]
        if not mods:
            issues.append(Issue("error", f"{o.name}: isn't deformed by the rig (add an Armature modifier using {arm.name})", "", o.name))
        unweighted, over, stray = weight_stats(o, bone_names)
        if unweighted:
            issues.append(Issue("error", f"{o.name}: {unweighted} vertices aren't weighted to any bone", "select_unweighted", o.name))
        if over:
            issues.append(Issue("error", f"{o.name}: {over} vertices use more than {MAX_WEIGHTS} bones", "limit_weights", o.name))
        if stray:
            issues.append(Issue("warn", f"{o.name}: vertex groups that aren't bones are ignored ({', '.join(stray[:3])})"))
        if o.name.split(".")[0] not in parts and signed_volume(o) < 0:
            issues.append(Issue("error", f"{o.name}: faces point inward (inside out). The game hides back faces, so you'd see through it", "fix_normals", o.name))
        tris += sum(len(p.vertices) - 2 for p in o.data.polygons)
        if not o.material_slots or not any(s.material for s in o.material_slots):
            issues.append(Issue("error", f"{o.name}: has no material (add one in the Material tab)", "", o.name))
        elif any(s.material is None for s in o.material_slots):
            issues.append(Issue("error", f"{o.name}: has an empty material slot", "", o.name))
        for s in o.material_slots:
            if s.material:
                materials.add(s.material.name)

    switched_off = set(json.loads(scene.get("dms_fit_report", "{}")).get("replaced", []))
    for name in sorted(parts - used_parts):
        if name in switched_off:
            issues.append(Issue("info", f"'{name}' is switched off (replaced by your model)"))
        else:
            issues.append(Issue("warn", f"Nothing is assigned to '{name}': that part will be empty in game"))

    for m in sorted(materials):
        missing = base_color_image(bpy.data.materials.get(m)) if not hero_material(m, scene) else None
        if missing is not None and (missing.size[0] == 0 or missing.size[1] == 0):
            issues.append(Issue("error", f"Material {m}: its image {missing.name} is missing. Open it again in the Image Editor (Image › Replace)"))
        for img in material_images(bpy.data.materials.get(m)):
            w, h = img.size
            if w and h and not (is_pow2(w) and is_pow2(h)):
                issues.append(Issue("error", f"Texture {img.name} is {w}×{h}: sizes must be powers of two (512, 1024, 2048…)"))
            elif max(w, h) > 4096:
                issues.append(Issue("warn", f"Texture {img.name} is {w}×{h}: over 4096 costs a lot of memory"))
    if len(materials) > 8:
        issues.append(Issue("warn", f"{len(materials)} materials: each is drawn separately, consider baking some together"))

    # Size: close to the original (fairness rule).
    ref = scene.get("dms_ref_size")
    box = bounds(meshes)
    if ref and box:
        size = box[1] - box[0]
        h0, h1 = ref[2], size.z
        if h0 > 0:
            change = (h1 - h0) / h0
            if change < -SIZE_TOLERANCE:
                issues.append(Issue("error", f"Height is {change:+.0%} vs the original: a smaller hero is harder to hit, keep within -{SIZE_TOLERANCE:.0%}"))
            elif change > SIZE_GROWTH_LIMIT:
                issues.append(Issue("error", f"Height is {change:+.0%} vs the original: keep within +{SIZE_GROWTH_LIMIT:.0%}"))
            elif abs(change) > SIZE_TOLERANCE / 2:
                issues.append(Issue("warn", f"Height is {change:+.0%} vs the original (hitboxes stay the original's)"))
    new_mats = [m for m in sorted(materials) if not hero_material(m, scene)]
    if new_mats:
        issues.append(Issue("info", f"New material(s): {', '.join(new_mats[:4])} (made from the hero's main material)"))

    issues.append(Issue("info", f"{len(meshes)} object(s), {tris:,} triangles, {len(materials)} material(s)"))
    return issues


_last_issues = {}


def store_issues(scene, issues):
    _last_issues[scene.name] = issues


# -------------------------------------------------------------- operators ---

class DMS_OT_open_project(bpy.types.Operator):
    """Open a Deadlock Mod Studio project (dms_project.json)"""

    bl_idname = "dms.open_project"
    bl_label = "Open Deadlock project"
    filepath: StringProperty(subtype="FILE_PATH")
    filter_glob: StringProperty(default="dms_project.json", options={"HIDDEN"})

    def invoke(self, context, event):
        context.window_manager.fileselect_add(self)
        return {"RUNNING_MODAL"}

    def execute(self, context):
        try:
            how = open_project(self.filepath)
        except Exception as e:  # shown to the user
            self.report({"ERROR"}, f"Couldn't open the project: {e}")
            return {"CANCELLED"}
        self.report({"INFO"}, "Project opened" if how == "opened" else "Hero imported and work file saved")
        return {"FINISHED"}


class DMS_OT_set_part(bpy.types.Operator):
    """Choose which part of the hero the selected objects replace"""

    bl_idname = "dms.set_part"
    bl_label = "Set part"
    bl_options = {"REGISTER", "UNDO"}

    def items(self, context):
        project = load_project(context.scene)
        return [(p, p, "") for p in part_names(project)] or [("", "No project", "")]

    part: EnumProperty(name="Part", items=items)

    def execute(self, context):
        objs = [o for o in context.selected_objects if o.type == "MESH"]
        if not objs:
            self.report({"WARNING"}, "Select the mesh objects first")
            return {"CANCELLED"}
        model = bpy.data.collections.get(MODEL_COLLECTION)
        for o in objs:
            o["dms_part"] = self.part
            if model and o.name not in model.objects and REFERENCE_COLLECTION not in [c.name for c in o.users_collection]:
                model.objects.link(o)
        self.report({"INFO"}, f"{len(objs)} object(s) set to {self.part}")
        return {"FINISHED"}


def attach_to_bone(obj, arm, bone_name, scene=None):
    """Make `obj` follow one bone rigidly: parented to the rig, deformed by
    it, every vertex weighted 1.0 to the bone. The object stays where it is
    on screen, whatever pose the rig is in."""
    pb = arm.pose.bones[bone_name]
    rest = arm.data.bones[bone_name].matrix_local
    bpy.context.view_layer.update()
    # Where it is now (posed) -> where it is at rest, in the rig's space.
    to_rest = rest @ pb.matrix.inverted() @ arm.matrix_world.inverted() @ obj.matrix_world
    for m in [m for m in obj.modifiers if m.type == "ARMATURE"]:
        obj.modifiers.remove(m)
    if obj.data.users > 1:
        obj.data = obj.data.copy()
    obj.data.transform(to_rest, shape_keys=True)
    if to_rest.determinant() < 0:
        obj.data.flip_normals()  # a mirrored object would come out inside out
    obj.parent = arm
    obj.parent_type = "OBJECT"
    obj.matrix_parent_inverse.identity()
    obj.matrix_basis.identity()
    obj.vertex_groups.clear()
    g = obj.vertex_groups.new(name=bone_name)
    g.add([v.index for v in obj.data.vertices], 1.0, "REPLACE")
    mod = obj.modifiers.new("Armature", "ARMATURE")
    mod.object = arm
    # Into my model, and into the part that uses this bone most.
    model = bpy.data.collections.get(MODEL_COLLECTION)
    if model and obj.name not in model.objects:
        for c in list(obj.users_collection):
            c.objects.unlink(obj)
        model.objects.link(obj)
    project = load_project(scene)
    if obj.get("dms_part", "") not in part_names(project):
        best, count = "", -1
        for o in model_meshes(scene):
            part = o.get("dms_part", "")
            grp = o.vertex_groups.get(bone_name)
            if o == obj or not part or grp is None:
                continue
            n = 0
            for v in o.data.vertices:
                for e in v.groups:
                    if e.group == grp.index and e.weight > 0.5:
                        n += 1
            if n > count:
                best, count = part, n
        obj["dms_part"] = best or (part_names(project) or [""])[0]
    return obj["dms_part"]


class DMS_OT_attach_to_bone(bpy.types.Operator):
    """Attach the selected objects to one bone so they move with it, like a hat on the head. Keeps them where they are"""

    bl_idname = "dms.attach_to_bone"
    bl_label = "Attach to bone"
    bl_options = {"REGISTER", "UNDO"}
    bl_property = "bone"

    def items(self, context):
        arm = rig(context.scene)
        return [(b.name, b.name, "") for b in arm.data.bones] if arm else [("", "No rig", "")]

    bone: EnumProperty(name="Bone", items=items)

    def invoke(self, context, event):
        context.window_manager.invoke_search_popup(self)
        return {"RUNNING_MODAL"}

    def execute(self, context):
        arm = rig(context.scene)
        objs = [o for o in context.selected_objects if o.type == "MESH" and REFERENCE_COLLECTION not in [c.name for c in o.users_collection]]
        if not arm or not self.bone or self.bone not in arm.data.bones:
            self.report({"WARNING"}, "Pick one of the rig's bones")
            return {"CANCELLED"}
        # Never the hero's own meshes: weighting a whole body to one bone
        # would make it move like a statue.
        project = load_project(context.scene)
        own = set(part_names(project))
        skipped = [o.name for o in objs if o.name.split(".")[0] in own or len(o.vertex_groups) > 1]
        objs = [o for o in objs if o.name not in skipped]
        if not objs:
            self.report({"WARNING"}, "Select only your new object (like the hat). The hero's own parts are already rigged" if skipped else "Select the mesh objects first")
            return {"CANCELLED"}
        if context.object and context.object.mode != "OBJECT":
            bpy.ops.object.mode_set(mode="OBJECT")
        parts = sorted({attach_to_bone(o, arm, self.bone, context.scene) for o in objs})
        note = f"; left alone: {', '.join(skipped[:4])} (already rigged)" if skipped else ""
        self.report({"INFO"}, f"{len(objs)} object(s) attached to {self.bone} (part: {', '.join(parts)}){note}")
        return {"FINISHED"}


# ------------------------------------------------------------ fit a model ---

def incoming_meshes():
    coll = bpy.data.collections.get(INCOMING_COLLECTION)
    return [o for o in coll.all_objects if o.type == "MESH"] if coll else []


def hero_part_meshes(scene=None):
    """{part: [the hero's own meshes for it]} still in my model."""
    out = {}
    for o in model_meshes(scene):
        part = o.get("dms_part", "")
        # Older work files have no flag: the hero's meshes are named after their part.
        if o.get("dms_hero_mesh") or (part and o.name.split(".")[0] == part):
            out.setdefault(part, []).append(o)
    return out


def reference_by_part():
    out = {}
    for o in reference_meshes():
        part = o.get("dms_ref_part") or o.name.replace(" (original)", "").split(".")[0]
        out[part] = o
    return out


def is_weapon_part(objs, arm):
    """Mostly weighted to weapon bones (a gun, a baton): kept by default."""
    for o in objs:
        names = {g.index: g.name.lower() for g in o.vertex_groups}
        weapon = total = 0
        verts = o.data.vertices
        for i in range(0, len(verts), 7):
            v = verts[i]
            if v.groups:
                g = max(v.groups, key=lambda x: x.weight)
                total += 1
                if any(k in names.get(g.group, "") for k in ("weapon", "gun", "muzzle", "baton", "blade", "sword")):
                    weapon += 1
        if total and weapon * 2 > total:
            return True
    return False


def bring_in(path):
    """Append a model's meshes and their rigs from another .blend into an
    'Incoming model' collection."""
    with bpy.data.libraries.load(path, link=False) as (src, dst):
        dst.objects = list(src.objects)
    objs = [o for o in dst.objects if o is not None]
    meshes = [o for o in objs if o.type == "MESH" and not o.name.startswith("WGT-") and len(o.data.polygons) > 0]
    arms = fit.model_armatures(meshes)
    keep = set(meshes) | set(arms)
    coll = bpy.data.collections.get(INCOMING_COLLECTION) or bpy.data.collections.new(INCOMING_COLLECTION)
    if coll.name not in bpy.context.scene.collection.children:
        bpy.context.scene.collection.children.link(coll)
    for o in objs:
        if o in keep:
            coll.objects.link(o)
        else:
            bpy.data.objects.remove(o)
    for o in keep:
        o.hide_set(False)
    # Image paths relative to the other file: point them at it.
    src_dir = os.path.dirname(os.path.abspath(path))
    for o in meshes:
        for slot in o.material_slots:
            for img in material_images(slot.material):
                if img.packed_file is None and img.filepath.startswith("//"):
                    rel = img.filepath[2:].replace("\\", "/")
                    cand = os.path.normpath(os.path.join(src_dir, rel))
                    if os.path.isfile(cand):
                        img.filepath = cand
                        img.reload()
    return meshes, arms


class DMS_OT_fit_import(bpy.types.Operator):
    """Bring in a character from another .blend (its meshes and rig) to fit onto the hero"""

    bl_idname = "dms.fit_import"
    bl_label = "Bring in model"
    filepath: StringProperty(subtype="FILE_PATH")
    filter_glob: StringProperty(default="*.blend", options={"HIDDEN"})

    def invoke(self, context, event):
        context.window_manager.fileselect_add(self)
        return {"RUNNING_MODAL"}

    def execute(self, context):
        if not self.filepath.lower().endswith(".blend"):
            self.report({"ERROR"}, "Pick a .blend file")
            return {"CANCELLED"}
        meshes, arms = bring_in(self.filepath)
        if not meshes:
            self.report({"ERROR"}, "No meshes found in that file")
            return {"CANCELLED"}
        self.report({"INFO"}, f"Brought in {len(meshes)} mesh(es) and {len(arms)} rig(s). Next: Fit to hero")
        return {"FINISHED"}


def run_fit(scene, into_part, replace):
    """Fit the incoming model onto the hero; replaced hero parts are switched
    off. Returns the report."""
    arm = rig(scene)
    meshes = incoming_meshes()
    if not meshes:
        raise RuntimeError("Bring in a model first")
    rest_pose(arm)
    parts = hero_part_meshes(scene)
    refs = reference_by_part()
    height_from = [refs[p] for p in replace if p in refs] or [o for p in replace for o in parts.get(p, [])]
    old_arms = fit.model_armatures(meshes)
    report = fit.fit_to_hero(meshes, arm, height_from, scene)
    model = bpy.data.collections[MODEL_COLLECTION]
    for o in meshes:
        for c in list(o.users_collection):
            c.objects.unlink(o)
        model.objects.link(o)
        o["dms_part"] = into_part
    # The hero's own meshes for the parts I replace: switched off.
    off = bpy.data.collections.get(OFF_COLLECTION) or bpy.data.collections.new(OFF_COLLECTION)
    if off.name not in scene.collection.children:
        scene.collection.children.link(off)
    for p in replace:
        for o in parts.get(p, []):
            for c in list(o.users_collection):
                c.objects.unlink(o)
            off.objects.link(o)
            o.hide_set(True)
    layer = bpy.context.view_layer.layer_collection.children.get(off.name)
    if layer:
        layer.exclude = True
    # What's left of the incoming model (its rigs) goes.
    for a in old_arms:
        if a.name in bpy.data.objects and not any(m.type == "ARMATURE" and m.object == a for o in bpy.data.objects if o.type == "MESH" for m in o.modifiers):
            bpy.data.objects.remove(a)
    inc = bpy.data.collections.get(INCOMING_COLLECTION)
    if inc:
        for o in list(inc.all_objects):
            bpy.data.objects.remove(o)
        bpy.data.collections.remove(inc)
    report["part"] = into_part
    report["replaced"] = sorted(replace)
    scene["dms_fit_report"] = json.dumps(report, default=str)
    return report


class DMS_OT_fit_to_hero(bpy.types.Operator):
    """Fit the brought-in model onto the hero's skeleton: scale to its height, move every joint onto the hero's, and move the weights onto the hero's bones"""

    bl_idname = "dms.fit_to_hero"
    bl_label = "Fit to hero"
    bl_options = {"REGISTER", "UNDO"}

    def part_items(self, context):
        return [(p, p, "") for p in part_names(load_project(context.scene))] or [("", "No project", "")]

    into_part: EnumProperty(name="Put it in", items=part_items)
    replace: EnumProperty(name="Switch off", items=part_items, options={"ENUM_FLAG"})

    def invoke(self, context, event):
        parts = hero_part_meshes(context.scene)
        names = part_names(load_project(context.scene))
        if not names:
            return {"CANCELLED"}
        arm = rig(context.scene)
        sizes = {p: sum(len(o.data.polygons) for o in parts.get(p, [])) for p in names}
        self.into_part = max(names, key=lambda p: sizes.get(p, 0))
        self.replace = {p for p in names if p in parts and not is_weapon_part(parts[p], arm)}
        return context.window_manager.invoke_props_dialog(self, width=360)

    def draw(self, context):
        col = self.layout.column()
        col.prop(self, "into_part")
        col.label(text="Switch off the hero's own parts:")
        col.prop(self, "replace")

    def execute(self, context):
        if not incoming_meshes():
            self.report({"ERROR"}, "Bring in a model first")
            return {"CANCELLED"}
        if context.object and context.object.mode != "OBJECT":
            bpy.ops.object.mode_set(mode="OBJECT")
        try:
            r = run_fit(context.scene, self.into_part, set(self.replace))
        except RuntimeError as e:
            self.report({"ERROR"}, str(e))
            return {"CANCELLED"}
        store_issues(context.scene, run_checks(context.scene))
        self.report({"INFO"}, f"Fitted: {len(r['mapped'])} bones matched, {len(r['followers'])} following, scaled ×{r['scale']:.3f}")
        return {"FINISHED"}


class DMS_OT_check(bpy.types.Operator):
    """Check the model against the game's rules"""

    bl_idname = "dms.check"
    bl_label = "Check model"

    def execute(self, context):
        issues = run_checks(context.scene)
        store_issues(context.scene, issues)
        errors = sum(1 for i in issues if i.level == "error")
        self.report({"WARNING"} if errors else {"INFO"}, f"{errors} problem(s)" if errors else "No problems found")
        return {"FINISHED"}


def limit_weights(obj, bone_names, limit=MAX_WEIGHTS):
    """Keep each vertex's strongest bone weights (bones only) and normalise."""
    groups = {g.index: g for g in obj.vertex_groups}
    changed = 0
    for v in obj.data.vertices:
        ws = [(g.group, g.weight) for g in v.groups if g.weight > 0 and groups.get(g.group) and groups[g.group].name in bone_names]
        if len(ws) <= limit:
            continue
        ws.sort(key=lambda t: -t[1])
        keep = ws[:limit]
        total = sum(w for _, w in keep) or 1.0
        for gi, _ in ws[limit:]:
            groups[gi].remove([v.index])
        for gi, w in keep:
            groups[gi].add([v.index], w / total, "REPLACE")
        changed += 1
    return changed


class DMS_OT_limit_weights(bpy.types.Operator):
    """Keep each vertex's 4 strongest bone weights and rebalance them"""

    bl_idname = "dms.limit_weights"
    bl_label = "Limit to 4 bones"
    bl_options = {"REGISTER", "UNDO"}
    target: StringProperty()

    def execute(self, context):
        obj = bpy.data.objects.get(self.target)
        arm = rig(context.scene)
        if not obj or not arm:
            return {"CANCELLED"}
        if obj.mode != "OBJECT":
            bpy.ops.object.mode_set(mode="OBJECT")
        n = limit_weights(obj, {b.name for b in arm.data.bones})
        store_issues(context.scene, run_checks(context.scene))
        self.report({"INFO"}, f"Fixed {n} vertices on {obj.name}")
        return {"FINISHED"}


class DMS_OT_fix_normals(bpy.types.Operator):
    """Make this object's faces point outward (Recalculate Outside)"""

    bl_idname = "dms.fix_normals"
    bl_label = "Fix normals"
    bl_options = {"REGISTER", "UNDO"}
    target: StringProperty()

    def execute(self, context):
        obj = bpy.data.objects.get(self.target)
        if not obj or obj.type != "MESH":
            return {"CANCELLED"}
        mode = obj.mode
        if mode != "OBJECT":
            bpy.ops.object.mode_set(mode="OBJECT")
        fix_normals(obj)
        if signed_volume(obj) < 0:
            obj.data.flip_normals()
        store_issues(context.scene, run_checks(context.scene))
        self.report({"INFO"}, f"{obj.name}: faces point outward")
        return {"FINISHED"}


class DMS_OT_select_unweighted(bpy.types.Operator):
    """Select the vertices that aren't weighted to any bone"""

    bl_idname = "dms.select_unweighted"
    bl_label = "Select unweighted"
    target: StringProperty()

    def execute(self, context):
        obj = bpy.data.objects.get(self.target)
        arm = rig(context.scene)
        if not obj or not arm:
            return {"CANCELLED"}
        if context.object and context.object.mode != "OBJECT":
            bpy.ops.object.mode_set(mode="OBJECT")
        names = {b.name for b in arm.data.bones}
        idx = {g.index: g.name for g in obj.vertex_groups}
        n = 0
        for v in obj.data.vertices:
            v.select = not any(g.weight > 0 and idx.get(g.group) in names for g in v.groups)
            n += v.select
        for o in context.selected_objects:
            o.select_set(False)
        obj.select_set(True)
        context.view_layer.objects.active = obj
        bpy.ops.object.mode_set(mode="EDIT")
        self.report({"INFO"}, f"{n} unweighted vertices selected")
        return {"FINISHED"}


# Body regions by Valve's standard bone names (shared by every hero).
REGIONS = {
    "LEGS": ("leg_", "ankle_", "ball_", "knee", "foot", "toe"),
    "ARMS": ("arm_", "hand_", "finger_", "clavicle_", "elbow", "wrist", "palm"),
    "TORSO": ("spine_", "pelvis", "chest", "back", "belly", "waist"),
    "HEAD": ("head", "neck_", "jaw", "eye", "mouth", "brow", "lip"),
    "GUN": ("weapon", "gun", "muzzle", "barrel", "pump", "trigger", "mag"),
}


def region_of(bone):
    b = bone.lower()
    for region, prefixes in REGIONS.items():
        if any(b.startswith(p) or p in b for p in prefixes):
            return region
    return ""


def faces_in_region(obj, region):
    """Indices of faces whose vertices mostly belong to bones of `region`."""
    names = {g.index: g.name for g in obj.vertex_groups}
    vert_region = []
    for v in obj.data.vertices:
        best, bw = "", 0.0
        for g in v.groups:
            if g.weight > bw and g.group in names:
                best, bw = names[g.group], g.weight
        vert_region.append(region_of(best) if best else "")
    out = []
    for p in obj.data.polygons:
        hits = sum(1 for i in p.vertices if vert_region[i] == region)
        if hits * 2 > len(p.vertices):
            out.append(p.index)
    return out


def texture_users(scene=None):
    """[(material, image, [object names])] for my model's textured materials."""
    found = {}
    for o in model_meshes(scene):
        for slot in o.material_slots:
            m = slot.material
            img = base_color_image(m)
            if img is None:
                continue
            found.setdefault(m.name, (m, img, []))[2].append(o.name)
    return [found[k] for k in sorted(found)]


class DMS_OT_paint_texture(bpy.types.Operator):
    """Paint this texture: selects an object that uses it, switches to Texture Paint and makes it the paint canvas"""

    bl_idname = "dms.paint_texture"
    bl_label = "Paint texture"
    material: StringProperty()

    def execute(self, context):
        mat = bpy.data.materials.get(self.material)
        img = base_color_image(mat)
        objs = [o for o in model_meshes(context.scene) if any(s.material == mat for s in o.material_slots)]
        if not mat or not img or not objs:
            return {"CANCELLED"}
        obj = context.object if context.object in objs else objs[0]
        if context.object and context.object.mode != "OBJECT":
            bpy.ops.object.mode_set(mode="OBJECT")
        for o in context.selected_objects:
            o.select_set(False)
        obj.hide_set(False)
        obj.select_set(True)
        context.view_layer.objects.active = obj
        obj.active_material_index = next(i for i, s in enumerate(obj.material_slots) if s.material == mat)
        for n in mat.node_tree.nodes:
            if n.type == "TEX_IMAGE" and n.image == img:
                mat.node_tree.nodes.active = n
        bpy.ops.object.mode_set(mode="TEXTURE_PAINT")
        ts = context.scene.tool_settings.image_paint
        ts.mode = "MATERIAL"
        for area in context.screen.areas if context.screen else []:
            if area.type == "IMAGE_EDITOR":
                area.spaces.active.image = img
        self.report({"INFO"}, f"Painting {img.name} on {obj.name}")
        return {"FINISHED"}


class DMS_OT_edit_outside(bpy.types.Operator):
    """Open this texture in the image editor set in Blender's preferences (File Paths). Save it there; Send to app picks up the change"""

    bl_idname = "dms.edit_outside"
    bl_label = "Edit outside"
    material: StringProperty()

    def execute(self, context):
        img = base_color_image(bpy.data.materials.get(self.material))
        if not img:
            return {"CANCELLED"}
        path = bpy.path.abspath(img.filepath)
        if not os.path.isfile(path):
            self.report({"ERROR"}, "This texture isn't a file on disk (re-import the project from the app)")
            return {"CANCELLED"}
        try:
            bpy.ops.image.external_edit(filepath=path)
        except RuntimeError as e:
            self.report({"ERROR"}, f"{e}. Set an image editor in Preferences › File Paths, or open {path} yourself.")
            return {"CANCELLED"}
        return {"FINISHED"}


class DMS_OT_paint_region(bpy.types.Operator):
    """Paint only this region of the body (by bone weights), or everything"""

    bl_idname = "dms.paint_region"
    bl_label = "Paint region"
    region: StringProperty()

    def execute(self, context):
        obj = context.object
        if not obj or obj.type != "MESH":
            self.report({"WARNING"}, "Select a mesh first (use Paint on a texture)")
            return {"CANCELLED"}
        mesh = obj.data
        if self.region == "ALL":
            mesh.use_paint_mask = False
            return {"FINISHED"}
        mode = obj.mode
        if mode != "OBJECT":
            bpy.ops.object.mode_set(mode="OBJECT")
        faces = set(faces_in_region(obj, self.region))
        for p in mesh.polygons:
            p.select = p.index in faces
        mesh.use_paint_mask = True
        if mode != "OBJECT":
            bpy.ops.object.mode_set(mode=mode)
        self.report({"INFO"}, f"{len(faces)} faces in {self.region.lower()}" if faces else f"No {self.region.lower()} faces on {obj.name}")
        return {"FINISHED"}


class DMS_OT_toggle_reference(bpy.types.Operator):
    """Show or hide the original hero as a wireframe"""

    bl_idname = "dms.toggle_reference"
    bl_label = "Show original"

    def execute(self, context):
        refs = reference_meshes()
        show = any(r.hide_get() for r in refs)
        for r in refs:
            r.hide_set(not show)
        return {"FINISHED"}


class DMS_OT_pose(bpy.types.Operator):
    """Pose the rig with one of the hero's animations, or back to rest"""

    bl_idname = "dms.pose"
    bl_label = "Pose"
    action: StringProperty()

    def execute(self, context):
        arm = rig(context.scene)
        if not arm:
            return {"CANCELLED"}
        if not self.action:
            rest_pose(arm)
            return {"FINISHED"}
        act = bpy.data.actions.get(self.action)
        if not act:
            return {"CANCELLED"}
        arm.animation_data_create()
        arm.animation_data.action = act
        if hasattr(arm.animation_data, "action_slot") and act.slots:
            arm.animation_data.action_slot = act.slots[0]
        start, end = (int(x) for x in act.frame_range)
        context.scene.frame_start, context.scene.frame_end = start, max(end, start + 1)
        context.scene.frame_set(start)
        return {"FINISHED"}


def send_to_app(scene=None):
    """Export my model and a manifest into the project's return folder."""
    scene = scene or bpy.context.scene
    project = load_project(scene)
    arm = rig(scene)
    meshes = model_meshes(scene)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    out_dir = os.path.join(project["returnDir"], stamp)
    os.makedirs(out_dir, exist_ok=True)
    glb = os.path.join(out_dir, "model.glb")

    # Export from rest pose, only the rig and my meshes.
    saved_action = arm.animation_data.action if arm.animation_data else None
    if arm.animation_data:
        arm.animation_data.action = None
    view = bpy.context.view_layer
    hidden = [o for o in [arm, *meshes] if o.hide_get()]
    for o in hidden:
        o.hide_set(False)
    for o in view.objects:
        o.select_set(False)
    for o in [arm, *meshes]:
        o.select_set(True)
    view.objects.active = arm
    try:
        bpy.ops.export_scene.gltf(
            filepath=glb,
            export_format="GLB",
            use_selection=True,
            export_animations=False,
            export_skins=True,
            export_all_influences=False,
            export_morph=True,
            export_yup=True,
            export_apply=False,
            export_materials="EXPORT",
            export_image_format="AUTO",
            export_extras=True,
            export_def_bones=False,
            export_leaf_bone=False,
        )
    finally:
        for o in hidden:
            o.hide_set(True)
        if saved_action is not None:
            arm.animation_data.action = saved_action

    # Repainted or replaced colour textures.
    originals = json.loads(scene.get("dms_materials", "{}"))
    material_files = []
    used = sorted({s.material.name for o in meshes for s in o.material_slots if s.material})
    for name in used:
        mat = bpy.data.materials[name]
        img = base_color_image(mat)
        entry = {"name": name, "color": None, "new": not hero_material(name, scene)}
        bsdf = principled(mat)
        if entry["new"]:
            # A new material: plain values from the shader where no texture is plugged in.
            if bsdf is not None:
                for key, socket in (("roughness", "Roughness"), ("metallic", "Metallic")):
                    inp = bsdf.inputs.get(socket)
                    if inp is not None and not inp.is_linked:
                        entry[key] = float(inp.default_value)
                inp = bsdf.inputs.get("Base Color")
                if img is None and inp is not None:
                    entry["colorValue"] = [float(x) for x in inp.default_value]
            elif img is None:
                entry["colorValue"] = [float(x) for x in mat.diffuse_color]
        if img is not None:
            orig = originals.get(name, {}).get("color", {})
            replaced = img.name != orig.get("image") and img.size[0] > 0
            if replaced or image_changed(img, orig.get("hash", "")):
                os.makedirs(os.path.join(out_dir, "textures"), exist_ok=True)
                rel = f"textures/{bpy.path.clean_name(name)}_color.png"
                disk = bpy.path.abspath(img.filepath)
                if not img.is_dirty and img.packed_file is None and os.path.isfile(disk):
                    # Edited in another program: the file is newer than Blender's copy.
                    import shutil

                    rel = rel[:-4] + os.path.splitext(disk)[1].lower()
                    shutil.copyfile(disk, os.path.join(out_dir, rel))
                else:
                    save_image_copy(img, os.path.join(out_dir, rel))
                entry["color"] = rel
        material_files.append(entry)

    manifest = {
        "format": 1,
        "projectId": project["id"],
        "sent": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "blender": bpy.app.version_string,
        "addon": ADDON_VERSION,
        "glb": "model.glb",
        "rig": arm.name,
        "objects": [{"object": o.name, "part": o.get("dms_part", ""), "triangles": sum(len(p.vertices) - 2 for p in o.data.polygons)} for o in meshes],
        "materials": used,
        "materialFiles": material_files,
    }
    with open(os.path.join(out_dir, "manifest.json"), "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2)
    if bpy.data.filepath:
        bpy.ops.wm.save_mainfile()
    return out_dir


class DMS_OT_send(bpy.types.Operator):
    """Check the model, then send it to Deadlock Mod Studio to build"""

    bl_idname = "dms.send"
    bl_label = "Send to app"

    def execute(self, context):
        issues = run_checks(context.scene)
        store_issues(context.scene, issues)
        errors = [i for i in issues if i.level == "error"]
        if errors:
            self.report({"ERROR"}, f"Fix {len(errors)} problem(s) first (see Checks)")
            return {"CANCELLED"}
        if context.object and context.object.mode != "OBJECT":
            bpy.ops.object.mode_set(mode="OBJECT")
        try:
            out = send_to_app(context.scene)
        except Exception as e:
            self.report({"ERROR"}, f"Couldn't export: {e}")
            return {"CANCELLED"}
        context.scene["dms_last_sent"] = time.strftime("%Y-%m-%d %H:%M")
        self.report({"INFO"}, f"Sent to the app ({os.path.basename(out)})")
        return {"FINISHED"}


# ------------------------------------------------------------------ panel ---

class DMS_PT_main(bpy.types.Panel):
    bl_label = "Deadlock Mod Studio"
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "Deadlock"

    def draw(self, context):
        layout = self.layout
        scene = context.scene
        project = load_project(scene)
        if project is None:
            layout.label(text="No project open.")
            layout.label(text="Use Edit in Blender in the app,")
            layout.label(text="or open its dms_project.json:")
            layout.operator("dms.open_project", icon="FILE_FOLDER")
            return

        box = layout.box()
        box.label(text=project.get("name", ""), icon="ARMATURE_DATA")
        box.label(text=f"Hero: {project.get('heroName') or project.get('hero')}")
        sent = scene.get("dms_last_sent")
        if sent:
            box.label(text=f"Last sent: {sent}", icon="CHECKMARK")

        # Parts
        box = layout.box()
        box.label(text="Parts", icon="MESH_DATA")
        obj = context.object
        if obj and obj.type == "MESH" and REFERENCE_COLLECTION not in [c.name for c in obj.users_collection]:
            row = box.row()
            row.label(text=f"{obj.name}: {obj.get('dms_part') or 'not assigned'}")
            row.operator_menu_enum("dms.set_part", "part", text="Set")
            row = box.row(align=True)
            row.operator_context = "EXEC_DEFAULT"  # straight to the head, no search
            op = row.operator("dms.attach_to_bone", text="Attach to head", icon="BONE_DATA")
            op.bone = "head"
            row.operator_context = "INVOKE_DEFAULT"
            row.operator("dms.attach_to_bone", text="Attach to bone…")
        meshes = model_meshes(scene)
        for p in project.get("parts", []):
            n = sum(1 for o in meshes if o.get("dms_part") == p["name"])
            shown = ", ".join(s.split(": ")[-1] for s in p.get("shownIn", []))
            row = box.row()
            row.label(text=p["name"], icon="CHECKBOX_HLT" if n else "CHECKBOX_DEHLT")
            row.label(text=f"{n} obj · {shown}" if shown else f"{n} obj")

        # Fit a model from another file
        box = layout.box()
        box.label(text=f"Fit a model to {project.get('heroName') or 'the hero'}", icon="OUTLINER_OB_ARMATURE")
        inc = incoming_meshes()
        row = box.row(align=True)
        row.operator("dms.fit_import", text="Bring in model…", icon="APPEND_BLEND")
        sub = row.row(align=True)
        sub.enabled = bool(inc)
        sub.operator("dms.fit_to_hero", text="Fit to hero", icon="CON_ARMATURE")
        if inc:
            box.label(text=f"Ready: {len(inc)} mesh(es) from {len(fit.model_armatures(inc))} rig(s)")
        rep_json = scene.get("dms_fit_report")
        if rep_json:
            r = json.loads(rep_json)
            box.label(text=f"Last fit: {len(r.get('mapped', []))} bones matched, {len(r.get('followers', {}))} following, ×{r.get('scale', 1):.3f}", icon="CHECKMARK")

        # Reference and poses
        box = layout.box()
        box.label(text="Compare and pose", icon="POSE_HLT")
        refs = reference_meshes()
        hidden = all(r.hide_get() for r in refs) if refs else True
        box.operator("dms.toggle_reference", text="Show original" if hidden else "Hide original", icon="HIDE_OFF" if hidden else "HIDE_ON")
        row = box.row(align=True)
        row.operator("dms.pose", text="Rest").action = ""
        for a in bpy.data.actions[:4]:
            row.operator("dms.pose", text=a.name.replace("primary_", "").replace("_", " ")).action = a.name

        # Textures
        box = layout.box()
        box.label(text="Textures", icon="TEXTURE")
        users = texture_users(scene)
        if not users:
            box.label(text="No textured materials")
        for mat, img, objs in users:
            col = box.column(align=True)
            row = col.row(align=True)
            new = "" if hero_material(mat.name, scene) else " (new)"
            row.label(text=f"{mat.name}{new}: {', '.join(objs)}", icon="MATERIAL")
            row = col.row(align=True)
            row.operator("dms.paint_texture", text="Paint", icon="BRUSH_DATA").material = mat.name
            row.operator("dms.edit_outside", text="Edit outside", icon="IMAGE").material = mat.name
        if context.object and context.object.mode == "TEXTURE_PAINT":
            box.label(text=f"Paint only ({context.object.name}):")
            row = box.row(align=True)
            for r, label in (("LEGS", "Legs"), ("ARMS", "Arms"), ("TORSO", "Torso"), ("HEAD", "Head"), ("GUN", "Gun"), ("ALL", "All")):
                row.operator("dms.paint_region", text=label).region = r

        # Checks
        box = layout.box()
        row = box.row()
        row.label(text="Checks", icon="VIEWZOOM")
        row.operator("dms.check", text="Check")
        issues = _last_issues.get(scene.name)
        if issues is None:
            box.label(text="Not checked yet")
        else:
            icon = {"error": "ERROR", "warn": "INFO", "info": "CHECKMARK"}
            for i in issues:
                row = box.row()
                row.label(text=i.text, icon=icon[i.level])
                if i.fix:
                    label = {"limit_weights": "Fix", "select_unweighted": "Select", "fix_normals": "Fix"}[i.fix]
                    row.operator(f"dms.{i.fix}", text=label).target = i.target

        layout.separator()
        col = layout.column()
        col.scale_y = 1.4
        col.operator("dms.send", icon="EXPORT")


classes = (
    DMS_OT_open_project,
    DMS_OT_set_part,
    DMS_OT_attach_to_bone,
    DMS_OT_fit_import,
    DMS_OT_fit_to_hero,
    DMS_OT_check,
    DMS_OT_limit_weights,
    DMS_OT_select_unweighted,
    DMS_OT_fix_normals,
    DMS_OT_paint_texture,
    DMS_OT_edit_outside,
    DMS_OT_paint_region,
    DMS_OT_toggle_reference,
    DMS_OT_pose,
    DMS_OT_send,
    DMS_PT_main,
)


def register():
    for c in classes:
        bpy.utils.register_class(c)


def unregister():
    for c in reversed(classes):
        bpy.utils.unregister_class(c)
