# SPDX-License-Identifier: GPL-3.0-or-later
"""Fit to hero: put a character model with its own rig (Rigify, Mixamo,
VRChat/Unity humanoid) onto a Deadlock hero's skeleton.

The hero's bones can't move (the animations are made for them), so the
model is moved onto them instead:

1. Prepare: bake the model in its rest pose with its modifiers and object
   transforms applied.
2. Map: match the model's deform bones to the hero's by name.
3. Scale to the hero's height, turn to face the same way.
4. Fit: pose a clean copy of the model's deform bones so every joint sits on
   the hero's joint (limbs stretch to the hero's lengths; head, hands and
   toes only move and turn), and bake that into the mesh.
5. Rebind: rename and merge the weights onto the hero's bones and switch the
   mesh to the hero's rig.

Bones with no match (breasts, a bow, a tail) follow the nearest matched bone
rigidly.
"""

import re

import bpy
from mathutils import Matrix, Vector

SIDES = ("L", "R")

# ------------------------------------------------------------- bone names ---

RIGIFY = {
    "spine": "hips", "spine.001": "spine1", "spine.002": "spine2", "spine.003": "spine3",
    "spine.004": "neck", "spine.005": "neck2", "spine.006": "head",
    "shoulder": "shoulder", "upper_arm": "upperarm", "upper_arm.001": "upperarm2",
    "forearm": "forearm", "forearm.001": "forearm2", "hand": "hand",
    "palm.01": "index0", "palm.02": "middle0", "palm.03": "ring0", "palm.04": "pinky0",
    "thigh": "thigh", "thigh.001": "thigh2", "shin": "shin", "shin.001": "shin2",
    "foot": "foot", "toe": "toe",
}
for _f, _k in (("thumb", "thumb"), ("f_index", "index"), ("f_middle", "middle"), ("f_ring", "ring"), ("f_pinky", "pinky")):
    for _i in (1, 2, 3):
        RIGIFY[f"{_f}.0{_i}"] = f"{_k}{_i}"

MIXAMO = {
    "hips": "hips", "spine": "spine1", "spine1": "spine2", "spine2": "spine3", "neck": "neck", "head": "head",
    "shoulder": "shoulder", "arm": "upperarm", "forearm": "forearm", "hand": "hand",
    "upleg": "thigh", "leg": "shin", "foot": "foot", "toebase": "toe",
}
for _f, _k in (("handthumb", "thumb"), ("handindex", "index"), ("handmiddle", "middle"), ("handring", "ring"), ("handpinky", "pinky")):
    for _i in (1, 2, 3):
        MIXAMO[f"{_f}{_i}"] = f"{_k}{_i}"

# Unity/VRChat humanoid and common hand-made names (side removed, lowercase,
# no spaces or underscores).
GENERIC = {
    "hips": "hips", "pelvis": "hips", "spine": "spine1", "chest": "spine2", "upperchest": "spine3",
    "neck": "neck", "head": "head", "shoulder": "shoulder", "clavicle": "shoulder",
    "upperarm": "upperarm", "arm": "upperarm", "lowerarm": "forearm", "forearm": "forearm", "elbow": "forearm",
    "hand": "hand", "wrist": "hand", "upperleg": "thigh", "thigh": "thigh", "leg": "thigh",
    "lowerleg": "shin", "knee": "shin", "shin": "shin", "calf": "shin", "foot": "foot", "ankle": "foot",
    "toe": "toe", "toes": "toe", "toebase": "toe",
}
for _f, _k in (("thumb", "thumb"), ("index", "index"), ("middle", "middle"), ("ring", "ring"), ("little", "pinky"), ("pinky", "pinky")):
    for _i, _w in ((1, "proximal"), (2, "intermediate"), (3, "distal")):
        GENERIC[f"{_f}{_i}"] = f"{_k}{_i}"
        GENERIC[f"{_f}{_w}"] = f"{_k}{_i}"
        GENERIC[f"{_f}finger{_i}"] = f"{_k}{_i}"


def split_side(name):
    """(name without its side, 'L' | 'R' | '')."""
    n = name
    for pat, side in (
        (r"\.(L|l)(?=\.|$)", "L"), (r"\.(R|r)(?=\.|$)", "R"),
        (r"[_ ](L|l)(?=[._ ]|$)", "L"), (r"[_ ](R|r)(?=[._ ]|$)", "R"),
        (r"^(L|l)[_ .]", "L"), (r"^(R|r)[_ .]", "R"),
        (r"^[Ll]eft[_ ]?", "L"), (r"^[Rr]ight[_ ]?", "R"),
        (r"[_ ]?[Ll]eft$", "L"), (r"[_ ]?[Rr]ight$", "R"),
    ):
        if re.search(pat, n):
            return re.sub(pat, "", n, count=1), side
    return n, ""


def canonical(name):
    """A source bone's role: ('upperarm', 'L'), ('head', ''), or None."""
    n = name
    flavour = "generic"
    for prefix in ("DEF-", "ORG-", "MCH-"):
        if n.startswith(prefix):
            n, flavour = n[len(prefix):], "rigify"
    if n.lower().startswith("mixamorig"):
        n, flavour = n.split(":", 1)[-1], "mixamo"
    for prefix in ("J_Bip_C_", "J_Bip_L_", "J_Bip_R_", "Bip01 ", "Bip001 "):
        if n.startswith(prefix):
            if "_L_" in prefix:
                n = "Left" + n[len(prefix):]
            elif "_R_" in prefix:
                n = "Right" + n[len(prefix):]
            else:
                n = n[len(prefix):]
    base, side = split_side(n)
    if flavour == "rigify":
        key = RIGIFY.get(base)
    elif flavour == "mixamo":
        key = MIXAMO.get(base.lower())
    else:
        key = GENERIC.get(re.sub(r"[ _.\-]", "", base.lower()))
    if key is None:
        return None
    centre = key in ("hips", "spine1", "spine2", "spine3", "neck", "neck2", "head")
    if centre != (side == ""):
        return None
    return key, side


# --------------------------------------------------------------- targets ---

class Segment:
    """Where one source bone goes: from `head` to `tail` (points on the hero
    skeleton), stretched to fit or rigid, and which hero bone gets its
    weights."""

    def __init__(self, head, tail, weight, rigid=False):
        self.head, self.tail, self.weight, self.rigid = head, tail, weight, rigid


def hero_points(hero):
    """Hero bone name -> head position (world), in the rest pose."""
    mw = hero.matrix_world
    return {b.name: mw @ b.head_local for b in hero.data.bones}


def plan_segments(roles, pts):
    """{(key, side): Segment} for the roles the source rig has."""
    out = {}

    def have(*names):
        return all(n in pts for n in names)

    def chain(keys, points, weights):
        """Spread the present source bones of `keys` over the hero points."""
        present = [k for k in keys if k in roles]
        points = [p for p in points if p in pts]
        if not present or len(points) < 2:
            return
        n = len(present)
        for i, k in enumerate(present):
            a = round(i * (len(points) - 1) / n)
            b = round((i + 1) * (len(points) - 1) / n)
            if a == b:
                b = min(a + 1, len(points) - 1)
            ha, hb = pts[points[a]], pts[points[b]]
            if a == b:
                continue
            # Two source bones over one hero bone: split it in the middle.
            out[k] = Segment(ha, hb, weights(k, points[a]))

    centre = lambda key: (key, "")
    # Torso: hips..spine3 over pelvis..neck.
    chain([centre(k) for k in ("hips", "spine1", "spine2", "spine3")], ["pelvis", "spine_1", "spine_2", "spine_3", "neck_0"], lambda k, p: p)
    # Neck: one or two bones over neck_0..head.
    neck = [centre(k) for k in ("neck", "neck2") if centre(k) in roles]
    if neck and have("neck_0", "head"):
        a, b = pts["neck_0"], pts["head"]
        for i, k in enumerate(neck):
            out[k] = Segment(a.lerp(b, i / len(neck)), a.lerp(b, (i + 1) / len(neck)), "neck_0")
    if centre("head") in roles and "head" in pts:
        tip = pts.get("head_end")
        out[centre("head")] = Segment(pts["head"], tip, "head", rigid=True)

    for s in SIDES:
        def twin(main, twist):
            return f"{main}_{s}_TWIST1" if f"{main}_{s}_TWIST1" in pts else f"{main}_{s}"

        def halves(k1, k2, a, b, w1, w2):
            ks = [k for k in ((k1, s), (k2, s)) if k in roles]
            if not ks or not have(a, b):
                return
            pa, pb = pts[a], pts[b]
            for i, k in enumerate(ks):
                w = w1 if k[0] == k1 else w2
                out[k] = Segment(pa.lerp(pb, i / len(ks)), pa.lerp(pb, (i + 1) / len(ks)), w)

        if ("shoulder", s) in roles and have(f"clavicle_{s}", f"arm_upper_{s}"):
            out[("shoulder", s)] = Segment(pts[f"clavicle_{s}"], pts[f"arm_upper_{s}"], f"clavicle_{s}")
        halves("upperarm", "upperarm2", f"arm_upper_{s}", f"arm_lower_{s}", f"arm_upper_{s}", twin("arm_upper", s))
        halves("forearm", "forearm2", f"arm_lower_{s}", f"hand_{s}", f"arm_lower_{s}", twin("arm_lower", s))
        if ("hand", s) in roles and f"hand_{s}" in pts:
            tip = next((pts[n] for n in (f"finger_middle_meta_{s}", f"finger_middle_0_{s}", f"hand_end_{s}") if n in pts), None)
            out[("hand", s)] = Segment(pts[f"hand_{s}"], tip, f"hand_{s}", rigid=True)
        for f in ("thumb", "index", "middle", "ring", "pinky"):
            meta, j0, j1, j2, end = (f"finger_{f}_{x}_{s}" for x in ("meta", 0, 1, 2, "end"))
            if (f + "0", s) in roles and have(meta, j0):
                out[(f + "0", s)] = Segment(pts[meta], pts[j0], meta)
            joints = [j0, j1, j2, end]
            for i in (1, 2, 3):
                k = (f"{f}{i}", s)
                if k in roles and joints[i - 1] in pts:
                    nxt = pts.get(joints[i])
                    out[k] = Segment(pts[joints[i - 1]], nxt, joints[i - 1], rigid=nxt is None)
        halves("thigh", "thigh2", f"leg_upper_{s}", f"leg_lower_{s}", f"leg_upper_{s}", twin("leg_upper", s))
        halves("shin", "shin2", f"leg_lower_{s}", f"ankle_{s}", f"leg_lower_{s}", f"leg_lower_{s}")
        if ("foot", s) in roles and have(f"ankle_{s}", f"ball_{s}"):
            out[("foot", s)] = Segment(pts[f"ankle_{s}"], pts[f"ball_{s}"], f"ankle_{s}")
        if ("toe", s) in roles and f"ball_{s}" in pts:
            out[("toe", s)] = Segment(pts[f"ball_{s}"], pts.get(f"ball_end_{s}"), f"ball_{s}", rigid=True)
    return out


# ------------------------------------------------------------------- fit ---

def model_armatures(meshes):
    arms = []
    for o in meshes:
        for m in o.modifiers:
            if m.type == "ARMATURE" and m.object and m.object not in arms:
                arms.append(m.object)
    return arms


def bake_rest(obj, arms):
    """Replace the mesh with its rest-pose shape, every modifier but the
    armature applied and the object transform baked in. Vertex groups stay."""
    saved = {a: a.data.pose_position for a in arms}
    for a in arms:
        a.data.pose_position = "REST"
    shown = {}
    for m in obj.modifiers:
        if m.type == "ARMATURE":
            shown[m] = m.show_viewport
            m.show_viewport = False
    bpy.context.view_layer.update()
    dg = bpy.context.evaluated_depsgraph_get()
    mesh = bpy.data.meshes.new_from_object(obj.evaluated_get(dg), preserve_all_data_layers=True, depsgraph=dg)
    for m, v in shown.items():
        m.show_viewport = v
    for a, p in saved.items():
        a.data.pose_position = p
    mw = obj.matrix_world.copy()
    mesh.transform(mw)
    if mw.determinant() < 0:
        mesh.flip_normals()
    old = obj.data
    obj.modifiers.clear()
    obj.parent = None
    obj.matrix_world = Matrix.Identity(4)
    obj.data = mesh
    if old.users == 0:
        bpy.data.meshes.remove(old)
    # Material slots no face uses (outline helpers and the like).
    used = {p.material_index for p in mesh.polygons}
    for i in reversed(range(len(mesh.materials))):
        if i not in used:
            mesh.materials.pop(index=i)
    # Shape keys don't survive the bake.
    return mesh


def mesh_height(objs):
    lo, hi = 1e9, -1e9
    for o in objs:
        for v in o.data.vertices:
            z = (o.matrix_world @ v.co).z
            lo, hi = min(lo, z), max(hi, z)
    return lo, hi


def rigid_delta(pose, rest):
    """The move (rotation + translation, no stretch) from rest to pose."""
    loc, rot, _ = pose.decompose()
    m = Matrix.Translation(loc) @ rot.to_matrix().to_4x4()
    rl, rr, _ = rest.decompose()
    r = Matrix.Translation(rl) @ rr.to_matrix().to_4x4()
    return m @ r.inverted()


# Body parts kept exactly as modelled in "keep shape" mode (only scaled).
TORSO = {"hips", "spine1", "spine2", "spine3", "neck", "neck2", "head"}
LIMB_ROOTS = {"upperarm", "thigh"}
LIMBS = {"upperarm", "upperarm2", "forearm", "forearm2", "thigh", "thigh2", "shin", "shin2"}
LIMB_ENDS = {"hand", "foot"}


def fit_to_hero(meshes, hero, hero_meshes, scene=None, mode="SHAPE"):
    """Fit `meshes` (rigged to their own armatures) onto `hero`. Returns a
    report dict. `hero_meshes`: the hero's original meshes, for its height.

    mode "SHAPE": keep the model's own shape and proportions. It's scaled to
    the hero's height; the torso, neck and head stay as they are; arms,
    legs, hands, feet and fingers only turn (at their own joints) to the
    hero's rest pose.
    mode "SKELETON": move every joint onto the hero's; limbs take the
    hero's lengths."""
    report = {"mapped": [], "followers": {}, "unused": [], "scale": 1.0, "turned": False, "mode": mode}
    arms = model_armatures(meshes)
    if not arms:
        raise RuntimeError("The model isn't rigged: its meshes have no Armature modifier")

    # Source deform bones, in world space at rest.
    src = {}
    for a in arms:
        mw = a.matrix_world
        for b in a.data.bones:
            if b.use_deform:
                src[b.name] = {"arm": a, "bone": b, "head": mw @ b.head_local, "tail": mw @ b.tail_local, "z": (mw.to_3x3() @ b.matrix_local.to_3x3()).col[2].normalized()}
    used_groups = set()
    for o in meshes:
        names = {g.index: g.name for g in o.vertex_groups}
        for v in o.data.vertices:
            for g in v.groups:
                if g.weight > 0 and names.get(g.group) in src:
                    used_groups.add(names[g.group])

    for o in meshes:
        bake_rest(o, arms)

    # Roles of the source bones.
    roles = {}
    for name in src:
        c = canonical(name)
        if c and c not in roles and name.startswith(("DEF-", "mixamorig")) or (c and c not in roles and not any(name.startswith(p) for p in ("ORG-", "MCH-"))):
            roles[c] = name
    pts = hero_points(hero)
    segs = plan_segments(roles, pts)

    # Scale to the hero's height (feet on the ground), and face the same way.
    lo, hi = mesh_height(meshes)
    hlo, hhi = mesh_height(hero_meshes) if hero_meshes else (0.0, pts.get("head_end", Vector((0, 0, 1.8))).z)
    s = (hhi - hlo) / max(hi - lo, 1e-6)
    turn = Matrix.Identity(4)
    lx = [src[roles[k]]["head"].x for k in roles if k[1] == "L"]
    hx = [pts[n].x for n in pts if n.endswith("_L")]
    if lx and hx and (sum(lx) / len(lx)) * (sum(hx) / len(hx)) < 0:
        turn = Matrix.Rotation(3.14159265, 4, "Z")
        report["turned"] = True
    place = Matrix.Translation((0, 0, hlo)) @ turn @ Matrix.Scale(s, 4) @ Matrix.Translation((0, 0, -lo))
    report["scale"] = s
    for o in meshes:
        o.data.transform(place)
    for d in src.values():
        d["head"], d["tail"] = place @ d["head"], place @ d["tail"]
        d["z"] = (turn.to_3x3() @ d["z"]).normalized()

    # Centre it on the hero front to back and side to side: line up the
    # hips, thighs and shoulders (a model built off-centre would otherwise
    # bend around joints that aren't inside it).
    pairs = []
    for k in (("hips", ""), ("thigh", "L"), ("thigh", "R"), ("upperarm", "L"), ("upperarm", "R"), ("shoulder", "L"), ("shoulder", "R")):
        if k in roles and k in segs:
            pairs.append(segs[k].head - src[roles[k]]["head"])
    if pairs:
        off = sum(pairs, Vector()) / len(pairs)
        shift = Matrix.Translation((off.x, off.y, 0.0))
        report["centred"] = [round(off.x, 4), round(off.y, 4)]
        for o in meshes:
            o.data.transform(shift)
        for d in src.values():
            d["head"], d["tail"] = shift @ d["head"], shift @ d["tail"]

    # A flat copy of the deform bones to fit with.
    bones = sorted(used_groups)
    fit_data = bpy.data.armatures.new("dms_fit")
    fit = bpy.data.objects.new("dms_fit", fit_data)
    scene = scene or bpy.context.scene
    scene.collection.objects.link(fit)
    view = bpy.context.view_layer
    for o in view.objects:
        o.select_set(False)
    view.objects.active = fit
    bpy.ops.object.mode_set(mode="EDIT")
    for n in bones:
        d = src[n]
        eb = fit_data.edit_bones.new(n)
        eb.head, eb.tail = d["head"], d["tail"]
        if (eb.tail - eb.head).length < 1e-5:
            eb.tail = eb.head + Vector((0, 0, 0.01))
        eb.align_roll(d["z"])
    bpy.ops.object.mode_set(mode="OBJECT")

    # Leaders: each unmatched bone follows a matched one.
    mapped = {roles[k]: segs[k] for k in segs if roles[k] in fit_data.bones}
    report["mapped"] = sorted(mapped)

    def leader_of(name):
        d = src[name]
        b = d["bone"].parent
        while b is not None:
            c = canonical(b.name)
            if c and c in segs and roles.get(c) in mapped:
                return roles[c]
            if b.name in mapped:
                return b.name
            b = b.parent
        # Nothing matched above it (a bow on its own rig): the whole chain
        # follows the matched bone closest to its top bone, so it stays in one piece.
        top = d["bone"]
        while top.parent is not None and top.parent.name in src:
            top = top.parent
        d = src[top.name]
        best, dist = None, 1e9
        for m in mapped:
            a, t = src[m]["head"], src[m]["tail"]
            ab = t - a
            u = max(0.0, min(1.0, (d["head"] - a).dot(ab) / max(ab.length_squared, 1e-12)))
            dd = (d["head"] - (a + ab * u)).length
            if dd < dist:
                best, dist = m, dd
        return best

    followers = {n: leader_of(n) for n in bones if n not in mapped}
    report["followers"] = followers

    role_of = {roles[k]: k for k in roles}
    poses = {}
    if mode == "SHAPE":
        poses = shape_poses(fit_data, mapped, role_of, src, fit_limbs=False)
    elif mode == "LIMBS":
        poses = shape_poses(fit_data, mapped, role_of, src, fit_limbs=True)
    # Pose: matched bones onto the hero's joints.
    for n, seg in (mapped.items() if mode == "SKELETON" else []):
        rest = fit_data.bones[n].matrix_local.copy()
        rest_dir = (rest.to_3x3() @ Vector((0, 1, 0))).normalized()
        length = fit_data.bones[n].length
        if seg.tail is not None and (seg.tail - seg.head).length > 1e-6:
            new_dir = (seg.tail - seg.head).normalized()
            stretch = 1.0 if seg.rigid else (seg.tail - seg.head).length / max(length, 1e-9)
        else:
            new_dir, stretch = rest_dir, 1.0
        rot = rest_dir.rotation_difference(new_dir).to_matrix() @ rest.to_3x3()
        m = Matrix.Translation(seg.head) @ rot.to_4x4() @ Matrix.Diagonal((1.0, stretch, 1.0, 1.0))
        poses[n] = m
    for n, lead in followers.items():
        rest = fit_data.bones[n].matrix_local.copy()
        if lead is None:
            poses[n] = rest
            continue
        poses[n] = rigid_delta(poses[lead], fit_data.bones[lead].matrix_local) @ rest
    for n, m in poses.items():
        fit.pose.bones[n].matrix = m
    view.update()

    # Bake the fit into the meshes.
    dg = None
    for o in meshes:
        mod = o.modifiers.new("dms_fit", "ARMATURE")
        mod.object = fit
        view.update()
        dg = bpy.context.evaluated_depsgraph_get()
        mesh = bpy.data.meshes.new_from_object(o.evaluated_get(dg), preserve_all_data_layers=True, depsgraph=dg)
        o.modifiers.remove(mod)
        old = o.data
        o.data = mesh
        if old.users == 0:
            bpy.data.meshes.remove(old)

    if mode == "SHAPE":
        # Turning the feet to the hero's (heels, say) moves the soles: stand
        # the model back on the ground.
        lo2, _ = mesh_height(meshes)
        lift = Matrix.Translation((0, 0, hlo - lo2))
        for o in meshes:
            o.data.transform(lift)
        report["lift"] = hlo - lo2

    # Weights onto the hero's bones.
    target = {n: seg.weight for n, seg in mapped.items()}
    if mode == "SHAPE":
        # The model's fingers aren't on the hero's finger joints, so the
        # hero's finger animation would twist them: they follow the hand.
        for n in mapped:
            key, side = role_of.get(n, ("", ""))
            if key[:-1] in ("thumb", "index", "middle", "ring", "pinky") and f"hand_{side}" in hero.data.bones:
                target[n] = f"hand_{side}"
    for n, lead in followers.items():
        target[n] = target.get(lead) if lead else None
    for o in meshes:
        remap_weights(o, target)
        o.parent = hero
        o.matrix_parent_inverse = hero.matrix_world.inverted()
        mod = o.modifiers.new("Armature", "ARMATURE")
        mod.object = hero

    bpy.data.objects.remove(fit)
    bpy.data.armatures.remove(fit_data)
    report["unused"] = sorted(k for k in pts if k not in {seg.weight for seg in segs.values()})
    return report


def shape_poses(fit_data, mapped, role_of, src, fit_limbs=False):
    """Keep-shape poses: torso bones stay put; the shoulder piece reaches to
    the hero's shoulder joint, so each arm (and leg) starts on the hero's
    joint; every limb bone turns to point the way the hero's bone does,
    without stretching. `fit_limbs`: arm and leg segments also stretch so
    elbows, wrists, knees and ankles sit on the hero's joints (hands and
    feet keep their shape). Parents first, so each bone rides on its
    parent."""
    def mapped_parent(n):
        b = src[n]["bone"].parent
        while b is not None:
            if b.name in mapped and b.name != n:
                return b.name
            c = canonical(b.name)
            for m, k in role_of.items():
                # (Rigify: ORG-palm.01 is the parent of DEF-palm.01 itself.)
                if k == c and m in mapped and m != n:
                    return m
            b = b.parent
        return None

    poses = {}

    def pose(n, seen=()):
        """Pose of `n`, its matched parent's first (Rigify's palms and
        fingers sit at the same depth, so plain depth order isn't enough)."""
        if n in poses:
            return poses[n]
        seg = mapped[n]
        rest = fit_data.bones[n].matrix_local.copy()
        parent = mapped_parent(n)
        if parent is not None and parent not in seen:
            carry = rigid_delta(pose(parent, seen + (n,)), fit_data.bones[parent].matrix_local)
        else:
            carry = Matrix.Identity(4)
        carried = carry @ rest
        key = role_of.get(n, ("", ""))[0]
        if key in TORSO or seg.tail is None or (seg.tail - seg.head).length < 1e-6:
            poses[n] = carried
            return carried
        head = carried.translation
        stretch = 1.0
        if key == "shoulder":
            # Reach from where the shoulder piece is to the hero's shoulder joint.
            target = seg.tail - head
            stretch = target.length / max(fit_data.bones[n].length, 1e-9)
            new_dir = target.normalized()
        else:
            if key in LIMB_ROOTS or (fit_limbs and (key in LIMBS or key in LIMB_ENDS)):
                head = seg.head
            new_dir = (seg.tail - seg.head).normalized()
            if fit_limbs and key in LIMBS:
                stretch = (seg.tail - seg.head).length / max(fit_data.bones[n].length, 1e-9)
        cur_dir = (carried.to_3x3() @ Vector((0, 1, 0))).normalized()
        rot = cur_dir.rotation_difference(new_dir).to_matrix() @ carried.to_3x3()
        poses[n] = Matrix.Translation(head) @ rot.to_4x4() @ Matrix.Diagonal((1.0, stretch, 1.0, 1.0))
        return poses[n]

    for n in mapped:
        pose(n)
    return poses


def remap_weights(obj, target, limit=4):
    """Rename and merge vertex groups onto hero bones (`target`: source
    group -> hero bone or None). Groups that aren't bones are dropped. At
    most `limit` bones per vertex, normalised."""
    names = {g.index: g.name for g in obj.vertex_groups}
    per_vertex = []
    for v in obj.data.vertices:
        w = {}
        for g in v.groups:
            t = target.get(names.get(g.group))
            if t and g.weight > 0:
                w[t] = w.get(t, 0.0) + g.weight
        top = sorted(w.items(), key=lambda x: -x[1])[:limit]
        total = sum(x[1] for x in top)
        per_vertex.append([(b, x / total) for b, x in top] if total > 0 else [])
    obj.vertex_groups.clear()
    groups = {}
    for i, ws in enumerate(per_vertex):
        for b, x in ws:
            g = groups.get(b) or obj.vertex_groups.new(name=b)
            groups[b] = g
            g.add([i], x, "REPLACE")
