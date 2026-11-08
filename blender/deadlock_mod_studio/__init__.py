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

import json
import os
import time

import bpy
from bpy.props import EnumProperty, StringProperty
from mathutils import Vector

ADDON_VERSION = "0.1.0"
MAX_WEIGHTS = 4
# Fairness rule: a custom model stays close to the original's size.
SIZE_TOLERANCE = 0.10
BONE_MOVE_TOLERANCE = 0.001  # metres (about 0.04 game units)
MODEL_COLLECTION = "Your model"
REFERENCE_COLLECTION = "Reference (original)"

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
    bpy.ops.import_scene.gltf(filepath=project["glb"], import_shading="NORMALS", disable_bone_shape=True)
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

    # The original, kept as a see-through reference that follows the pose.
    for o in meshes:
        ref = o.copy()
        ref.data = o.data.copy()
        ref.name = f"{o.name} (original)"
        ref.display_type = "WIRE"
        ref.hide_select = True
        ref.hide_render = True
        if "dms_part" in ref:
            del ref["dms_part"]
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
    arm.lock_location = (True, True, True)
    arm.lock_rotation = (True, True, True)
    arm.lock_scale = (True, True, True)
    _lock_bones(arm, True)
    for a in bpy.data.actions:
        a.use_fake_user = True
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


def is_pow2(n):
    return n > 0 and (n & (n - 1)) == 0


def material_images(mat):
    if not mat or not mat.use_nodes or not mat.node_tree:
        return []
    return [n.image for n in mat.node_tree.nodes if n.type == "TEX_IMAGE" and n.image]


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
        tris += sum(len(p.vertices) - 2 for p in o.data.polygons)
        if not o.material_slots or not any(s.material for s in o.material_slots):
            issues.append(Issue("warn", f"{o.name}: has no material"))
        for s in o.material_slots:
            if s.material:
                materials.add(s.material.name)

    for name in sorted(parts - used_parts):
        issues.append(Issue("warn", f"Nothing is assigned to '{name}': that part will be empty in game"))

    for m in sorted(materials):
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
            if abs(change) > SIZE_TOLERANCE:
                issues.append(Issue("error", f"Height is {change:+.0%} vs the original: keep within ±{SIZE_TOLERANCE:.0%}"))
            elif abs(change) > SIZE_TOLERANCE / 2:
                issues.append(Issue("warn", f"Height is {change:+.0%} vs the original"))

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

    manifest = {
        "format": 1,
        "projectId": project["id"],
        "sent": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "blender": bpy.app.version_string,
        "addon": ADDON_VERSION,
        "glb": "model.glb",
        "rig": arm.name,
        "objects": [{"object": o.name, "part": o.get("dms_part", ""), "triangles": sum(len(p.vertices) - 2 for p in o.data.polygons)} for o in meshes],
        "materials": sorted({s.material.name for o in meshes for s in o.material_slots if s.material}),
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
        meshes = model_meshes(scene)
        for p in project.get("parts", []):
            n = sum(1 for o in meshes if o.get("dms_part") == p["name"])
            shown = ", ".join(s.split(": ")[-1] for s in p.get("shownIn", []))
            row = box.row()
            row.label(text=p["name"], icon="CHECKBOX_HLT" if n else "CHECKBOX_DEHLT")
            row.label(text=f"{n} obj · {shown}" if shown else f"{n} obj")

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
                    label = {"limit_weights": "Fix", "select_unweighted": "Select"}[i.fix]
                    row.operator(f"dms.{i.fix}", text=label).target = i.target

        layout.separator()
        col = layout.column()
        col.scale_y = 1.4
        col.operator("dms.send", icon="EXPORT")


classes = (
    DMS_OT_open_project,
    DMS_OT_set_part,
    DMS_OT_check,
    DMS_OT_limit_weights,
    DMS_OT_select_unweighted,
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
