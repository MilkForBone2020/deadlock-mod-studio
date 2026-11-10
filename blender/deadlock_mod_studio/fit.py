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

# Auto-Rig Pro: deform bones end in .x (centre) or .l/.r; limbs are the
# *_stretch bones (the *_twist ones follow them).
ARP = {
    "root": "hips", "spine_01": "spine1", "spine_02": "spine2", "spine_03": "spine3",
    "neck": "neck", "subneck_1": "neck2", "head": "head", "shoulder": "shoulder",
    "arm_stretch": "upperarm", "forearm_stretch": "forearm", "hand": "hand",
    "thigh_stretch": "thigh", "leg_stretch": "shin", "foot": "foot", "toes_01": "toe",
}
for _f in ("thumb", "index", "middle", "ring", "pinky"):
    ARP[f"{_f}1"] = ARP[f"c_{_f}1"] = f"{_f}1"
    ARP[f"c_{_f}2"] = f"{_f}2"
    ARP[f"c_{_f}3"] = f"{_f}3"
    if _f != "thumb":
        ARP[f"c_{_f}1_base"] = f"{_f}0"

# Whole-character controllers some rigs weight meshes to.
ROOT_CONTROLLERS = {"c_traj", "c_pos", "root", "root_bone", "master", "armature"}

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
    if n.endswith(".x") and n[:-2] in ARP:
        n, flavour = n[:-2], "arp"
    base, side = split_side(n)
    if flavour == "generic" and side and base in ARP and re.search(r"\.[lr]$", n):
        flavour = "arp"
    if flavour == "arp":
        key = ARP.get(base)
    elif flavour == "rigify":
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


def side_rigs(arms):
    """Split the model's rigs into the main body rig and side rigs that follow
    it (a ponytail on its own wiggle-bone rig whose root bone is a Child Of
    the head). Returns (main rigs, {side rig: {bone: main bone it follows}})."""
    if len(arms) < 2:
        return list(arms), {}
    score = {a: sum(1 for b in a.data.bones if canonical(b.name)) for a in arms}
    top = max(arms, key=lambda a: score[a])
    main = [top]
    side = {}
    for a in arms:
        if a is top:
            continue
        anchors = {}
        for pb in a.pose.bones:
            for c in pb.constraints:
                t = getattr(c, "target", None)
                if t is top and not c.mute and getattr(c, "subtarget", "") in top.data.bones:
                    anchors[pb.name] = c.subtarget
        if a.parent is top and a.parent_type == "BONE" and a.parent_bone in top.data.bones:
            for b in a.data.bones:
                if b.parent is None:
                    anchors.setdefault(b.name, a.parent_bone)
        for c in a.constraints:
            if getattr(c, "target", None) is top and not c.mute and getattr(c, "subtarget", "") in top.data.bones:
                for b in a.data.bones:
                    if b.parent is None:
                        anchors.setdefault(b.name, c.subtarget)
        if anchors:
            side[a] = anchors
        else:
            main.append(a)
    return main, side


def bake_rest(obj, arms, posed=()):
    """Replace the mesh with its rest-pose shape, every modifier but the
    armature applied and the object transform baked in. Vertex groups stay.
    Rigs in `posed` (side rigs following the body) keep deforming the mesh
    in their followed pose, so their parts stay where they're seen."""
    saved = {a: a.data.pose_position for a in arms}
    for a in arms:
        a.data.pose_position = "POSE" if a in posed else "REST"
    shown = {}
    for m in obj.modifiers:
        if m.type == "ARMATURE" and m.object not in posed:
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
    # Constraints (Child Of a bone...), keyframes and drivers on the object
    # would move it again later; the hero's rig moves it from now on.
    obj.constraints.clear()
    obj.animation_data_clear()
    obj.parent = None
    # A part attached to a bone of its old rig must not stay "attached to a
    # bone" once it hangs from the hero's rig (it would jump to that bone).
    obj.parent_type = "OBJECT"
    obj.parent_bone = ""
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


FINGERS = ("thumb", "index", "middle", "ring", "pinky")

# Body parts kept exactly as modelled in "keep shape" mode (only scaled).
TORSO = {"hips", "spine1", "spine2", "spine3", "neck", "neck2", "head"}
LIMB_ROOTS = {"upperarm", "thigh"}
LIMBS = {"upperarm", "upperarm2", "forearm", "forearm2", "thigh", "thigh2", "shin", "shin2"}
LIMB_ENDS = {"hand", "foot"}


def fit_to_hero(meshes, hero, hero_meshes, scene=None, mode="SHAPE", physics="SEPARATE"):
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

    # Source deform bones, in world space at rest. A side rig that follows
    # the body (wiggle-bone hair) is read where it sits on the body at rest.
    main, side = side_rigs(arms)
    saved = {a: a.data.pose_position for a in arms}
    for a in arms:
        a.data.pose_position = "POSE" if a in side else "REST"
    bpy.context.view_layer.update()
    src = {}
    for a in arms:
        mw = a.matrix_world
        for b in a.data.bones:
            if not b.use_deform:
                continue
            if a in side:
                m = mw @ a.pose.bones[b.name].matrix
                src[b.name] = {"arm": a, "bone": b, "head": m.translation.copy(), "tail": mw @ a.pose.bones[b.name].tail, "z": m.to_3x3().col[2].normalized(), "anchor": side[a].get(b.name)}
            else:
                src[b.name] = {"arm": a, "bone": b, "head": mw @ b.head_local, "tail": mw @ b.tail_local, "z": (mw.to_3x3() @ b.matrix_local.to_3x3()).col[2].normalized()}
    for a, p in saved.items():
        a.data.pose_position = p
    report["side_rigs"] = {a.name: dict(v) for a, v in side.items()}
    used_groups = set()
    for o in meshes:
        names = {g.index: g.name for g in o.vertex_groups}
        for v in o.data.vertices:
            for g in v.groups:
                if g.weight > 0.01 and names.get(g.group) in src:
                    used_groups.add(names[g.group])

    for o in meshes:
        bake_rest(o, arms, posed=set(side))

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
    # Source characters stand on their origin; a part's lowest point can be
    # below it (Mina's gun rests on the floor at the weapon bone).
    if abs(hlo) < 0.3 * max(hhi - hlo, 1e-6):
        hlo = 0.0
    s = (hhi - hlo) / max(hi - lo, 1e-6)
    # By the body, feet to head joint, when both have one: hats, bows and
    # tall hair don't make the body smaller (which stretches every limb).
    head_role = roles.get(("head", ""))
    if head_role in src and "head" in pts:
        mine = src[head_role]["head"].z - lo
        theirs = pts["head"].z - hlo
        if mine > 1e-6 and theirs > 1e-6:
            s = theirs / mine
            report["scaled_by"] = "head"
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

    # How high each ankle sits above the model's own soles (feet are on the
    # hero's ground here, before centring).
    ankle_h = {sd: src[roles[("foot", sd)]]["head"].z - hlo for sd in SIDES if ("foot", sd) in roles}

    # Centre it on the hero front to back and side to side: line up the
    # hips, thighs and shoulders (a model built off-centre would otherwise
    # bend around joints that aren't inside it).
    pairs = []
    for k in (("hips", ""), ("thigh", "L"), ("thigh", "R"), ("upperarm", "L"), ("upperarm", "R"), ("shoulder", "L"), ("shoulder", "R")):
        if k in roles and k in segs:
            pairs.append(segs[k].head - src[roles[k]]["head"])
    if pairs:
        off = sum(pairs, Vector()) / len(pairs)
        shift = Matrix.Translation((off.x, off.y, off.z if mode != "SKELETON" else 0.0))
        report["centred"] = [round(off.x, 4), round(off.y, 4), round(off.z, 4)]
        for o in meshes:
            o.data.transform(shift)
        for d in src.values():
            d["head"], d["tail"] = shift @ d["head"], shift @ d["tail"]

    # Keep shape: each foot keeps its shape and angle, so its sole should
    # land on the ground under the hero's ankle (high heels, chunky boots):
    # the ankle goes to the model's own ankle height there and the shin
    # takes up the difference.
    if mode == "SHAPE":
        for sd, h in ankle_h.items():
            fk = ("foot", sd)
            if fk not in segs or h <= 0:
                continue
            p = segs[fk].head.copy()
            p.z = hlo + h
            segs[fk].head = p
            for k in (("shin2", sd), ("shin", sd)):
                if k in segs:
                    segs[k].tail = p
                    break
        report["ankles"] = {sd: round(h, 4) for sd, h in ankle_h.items()}

    # A flat copy of the deform bones to fit with.
    # Unweighted bones between weighted ones stay, so chains aren't broken.
    # (Only between: a rig's top bone above everything, like Auto-Rig Pro's
    # c_traj, isn't part of the body.)
    weighted = set(used_groups)
    for n in list(weighted):
        path = []
        b = src[n]["bone"].parent
        while b is not None and b.name in src:
            path.append(b.name)
            if b.name in weighted:
                used_groups.update(path)
                path = []
            b = b.parent
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
        # A rig's ground/root controller (Auto-Rig Pro's c_traj) carrying
        # weights: the body part it stands for is the hips.
        if name.lower() in ROOT_CONTROLLERS and roles.get(("hips", "")) in mapped:
            return roles[("hips", "")]
        d = src[name]
        b = d["bone"].parent
        top = d["bone"]
        while top.parent is not None and top.parent.name in src:
            top = top.parent
        anchor = src[top.name].get("anchor")
        if b is None and anchor:
            # A side rig: its chain follows the body bone its root followed.
            detached[name] = top.name
            if anchor in mapped:
                return anchor
            if anchor in src and anchor != name:
                return leader_of(anchor)
            # A control bone (Rigify's "head"): the deform bone it stands for.
            c = canonical(anchor)
            if c and roles.get(c) in mapped:
                return roles[c]
            body = main[0]
            if anchor in body.data.bones:
                p = body.matrix_world @ body.data.bones[anchor].head_local
                return min(mapped, key=lambda m: (src[m]["head"] - p).length, default=None)
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
        detached[name] = top.name
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

    detached = {}  # bone -> top bone of its own little rig (a bow, cables)
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
    twists = {}
    for n, lead in followers.items():
        rest = fit_data.bones[n].matrix_local.copy()
        if lead is None:
            poses[n] = rest
            continue
        lb = fit_data.bones[lead]
        poses[n] = rigid_delta(poses[lead], lb.matrix_local) @ rest
        if inline(fit_data.bones[n], lb):
            # A twist bone along a limb (Auto-Rig Pro, Unreal...) moves and
            # stretches exactly like the limb bone, so the limb is one piece
            # (what reaches past the wrist goes to the hand, below).
            poses[n] = poses[lead] @ lb.matrix_local.inverted() @ rest
            twists.setdefault(lead, []).append(n)
    for n, m in poses.items():
        fit.pose.bones[n].matrix = m
    view.update()

    # A stretched limb bone stretches every vertex weighted to it, even past
    # its end (a shoe partly on the shin is dragged to the floor): weight
    # past the end goes to the next part (hand, foot), which keeps its shape.
    by_role_m = {role_of[m]: m for m in mapped if m in role_of}
    NEXT = {"upperarm": ("upperarm2", "forearm"), "upperarm2": ("forearm",), "forearm": ("forearm2", "hand"), "forearm2": ("hand",),
            "thigh": ("thigh2", "shin"), "thigh2": ("shin",), "shin": ("shin2", "foot"), "shin2": ("foot",)}
    moved = 0
    for n in mapped:
        key, side = role_of.get(n, ("", ""))
        if key not in NEXT or n not in poses or abs(poses[n].to_scale().y - 1.0) < 0.02:
            continue
        nxt = next((by_role_m[(k, side)] for k in NEXT[key] if (k, side) in by_role_m), None)
        if nxt is None:
            continue
        b0 = fit_data.bones[n]
        a0, ab = b0.head_local, b0.tail_local - b0.head_local
        for o, gname in [(o, gn) for o in meshes for gn in [n] + twists.get(n, [])]:
            g = o.vertex_groups.get(gname)
            if g is None:
                continue
            to = o.vertex_groups.get(nxt) or o.vertex_groups.new(name=nxt)
            for v in o.data.vertices:
                w = next((x.weight for x in v.groups if x.group == g.index), 0.0)
                if w <= 0:
                    continue
                u = (o.matrix_world @ v.co - a0).dot(ab) / max(ab.length_squared, 1e-12)
                # Blended over the last stretch of the bone, so there's no seam.
                t = min(max((u - 0.5) / 0.6, 0.0), 1.0)
                if t > 0:
                    if t >= 1.0:
                        g.remove([v.index])
                    else:
                        g.add([v.index], w * (1.0 - t), "REPLACE")
                    to.add([v.index], w * t, "ADD")
                    moved += 1
    if moved:
        report["past_end_moved"] = moved

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

    if False:
        lo2, _ = mesh_height(meshes)
        lift = Matrix.Translation((0, 0, hlo - lo2))
        for o in meshes:
            o.data.transform(lift)
        report["lift"] = hlo - lo2

    # Weights onto the hero's bones.
    target = {n: seg.weight for n, seg in mapped.items()}
    # (Fingers keep their own bones: the fit puts each knuckle on the hero's,
    # so the hero's finger animation bends them at the right places.)
    for n, lead in followers.items():
        target[n] = target.get(lead) if lead else None
    keep = {}
    if physics in ("SEPARATE", "ALL", True):
        keep.update(detached)
    if physics == "ALL":
        # Extra body bones too (breasts, a tail...): their own bones, no chain yet.
        keep.update({n: n for n in followers if n not in detached and followers[n]})
    if keep:
        # Bones on their own rig (a bow and its cables) become new bones on
        # the hero, hanging from the bone they follow, ready for physics.
        new_names = add_physics_bones(hero, fit, fit_data, poses, src, keep, followers, target)
        report["physics"] = physics_chains(src, detached, new_names, target, followers) if detached else []
        report["kept_bones"] = sorted(new_names.values())
        for n, nn in new_names.items():
            target[n] = nn
    for o in meshes:
        remap_weights(o, target)
        o.parent = hero
        o.parent_type = "OBJECT"
        o.parent_bone = ""
        o.matrix_parent_inverse = hero.matrix_world.inverted()
        o.matrix_basis = Matrix.Identity(4)
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
        # Rigs whose deform bones don't hang from each other (Auto-Rig Pro:
        # the forearm hangs from the ground controller): by anatomy.
        key, side = role_of.get(n, ("", ""))
        for k in ANATOMY.get(key, ()):
            m = by_role.get((k, side if k not in TORSO else ""))
            if m is not None and m != n:
                return m
        return None

    poses = {}
    by_role = {k: m for m, k in role_of.items() if m in mapped}

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
        if parent is not None and parent not in seen:
            # Where the parent, stretched or not, puts this bone's head.
            head = pose(parent, seen + (n,)) @ (fit_data.bones[parent].matrix_local.inverted() @ rest.translation)
        stretch = 1.0
        side = role_of.get(n, ("", ""))[1]
        if not fit_limbs and key in ("forearm", "forearm2", "shin", "shin2"):
            # Wrists and ankles land on the hero's: the forearm and shin
            # take up the difference in length, everything else keeps shape.
            base = key.rstrip("2")
            first = by_role.get((base, side))
            second = by_role.get((base + "2", side))
            last = second or first
            end = mapped[last].tail
            if key == base:
                a, b = head, (head.lerp(end, 0.5) if second else end)
            else:
                a = pose(first, seen + (n,)).translation.lerp(end, 0.5) if first else head
                b = end
            stretch = (b - a).length / max(fit_data.bones[n].length, 1e-9)
            cur_dir = (carried.to_3x3() @ Vector((0, 1, 0))).normalized()
            rot = cur_dir.rotation_difference((b - a).normalized()).to_matrix() @ carried.to_3x3()
            poses[n] = Matrix.Translation(a) @ rot.to_4x4() @ Matrix.Diagonal((1.0, stretch, 1.0, 1.0))
            return poses[n]
        if key == "shoulder":
            # Reach from where the shoulder piece is to the hero's shoulder joint.
            target = seg.tail - head
            stretch = target.length / max(fit_data.bones[n].length, 1e-9)
            new_dir = target.normalized()
        else:
            if key in LIMB_ROOTS or key in LIMB_ENDS or (fit_limbs and key in LIMBS):
                head = seg.head
            new_dir = (seg.tail - seg.head).normalized()
            if key in ("foot", "toe") and not fit_limbs:
                # Feet keep their own angle (high heels stay heels; the
                # animations turn the foot anyway), they just move to the ankle.
                new_dir = (carried.to_3x3() @ Vector((0, 1, 0))).normalized()
            if key[:-1] in FINGERS:
                # Each knuckle on the hero's, so the hero's finger animation
                # bends the fingers where they bend.
                head = seg.head
                if seg.tail is not None and not seg.rigid:
                    stretch = min(max((seg.tail - seg.head).length / max(fit_data.bones[n].length, 1e-9), 0.5), 2.0)
            if key == "hand":
                # The whole hand (palm and all) the size of the hero's, evenly.
                mid = by_role.get(("middle1", side)) or by_role.get(("index1", side))
                kn = mapped[mid].head if mid else None
                if kn is not None:
                    mine = (fit_data.bones[mid].head_local - fit_data.bones[n].head_local).length
                    theirs = (kn - seg.head).length
                    if mine > 1e-6:
                        k = min(max(theirs / mine, 0.6), 1.6)
                        cur_dir = (carried.to_3x3() @ Vector((0, 1, 0))).normalized()
                        aim = (kn - seg.head).normalized()
                        here = (carried.to_3x3() @ (fit_data.bones[n].matrix_local.inverted().to_3x3() @ (fit_data.bones[mid].head_local - fit_data.bones[n].head_local))).normalized()
                        rot = here.rotation_difference(aim).to_matrix() @ carried.to_3x3()
                        poses[n] = Matrix.Translation(seg.head) @ rot.to_4x4() @ Matrix.Diagonal((k, k, k, 1.0))
                        return poses[n]
            if fit_limbs and key in LIMBS:
                stretch = (seg.tail - seg.head).length / max(fit_data.bones[n].length, 1e-9)
            elif key in LIMB_ROOTS:
                # The whole limb shares the difference in length (thigh and
                # shin alike), instead of the shin alone taking it all.
                lower = "forearm" if key == "upperarm" else "shin"
                keys = [k for k in (key, key + "2", lower, lower + "2") if (k, side) in by_role]
                last = by_role.get((lower + "2", side)) or by_role.get((lower, side))
                if last is not None:
                    total = sum(fit_data.bones[by_role[(k, side)]].length for k in keys)
                    reach = (mapped[last].tail - seg.head).length
                    stretch = min(max(reach / max(total, 1e-9), 0.7), 1.6)
        cur_dir = (carried.to_3x3() @ Vector((0, 1, 0))).normalized()
        rot = cur_dir.rotation_difference(new_dir).to_matrix() @ carried.to_3x3()
        poses[n] = Matrix.Translation(head) @ rot.to_4x4() @ Matrix.Diagonal((1.0, stretch, 1.0, 1.0))
        return poses[n]

    for n in mapped:
        pose(n)
    return poses


def inline(bone, lead):
    """Whether `bone` lies along `lead` (a twist bone on a limb segment):
    within its length, close to its line, pointing the same way."""
    a, t = lead.head_local, lead.tail_local
    ab = t - a
    L = ab.length
    if L < 1e-6:
        return False
    u = (bone.head_local - a).dot(ab) / (L * L)
    off = ((bone.head_local - a) - ab * u).length
    same_way = (bone.tail_local - bone.head_local).normalized().dot(ab.normalized()) > 0.9 if bone.length > 1e-6 else True
    return -0.05 <= u <= 1.05 and off <= 0.25 * L and same_way


# Each body part's parent (nearest first), for rigs whose deform bones are
# laid out flat.
ANATOMY = {
    "spine1": ("hips",), "spine2": ("spine1", "hips"), "spine3": ("spine2", "spine1"),
    "neck": ("spine3", "spine2", "spine1"), "neck2": ("neck",), "head": ("neck2", "neck", "spine3", "spine2"),
    "shoulder": ("spine3", "spine2", "spine1"), "upperarm": ("shoulder", "spine3", "spine2"),
    "upperarm2": ("upperarm",), "forearm": ("upperarm2", "upperarm"), "forearm2": ("forearm",),
    "hand": ("forearm2", "forearm"), "thigh": ("hips",), "thigh2": ("thigh",), "shin": ("thigh2", "thigh"),
    "shin2": ("shin",), "foot": ("shin2", "shin"), "toe": ("foot",),
}
for _f in ("thumb", "index", "middle", "ring", "pinky"):
    ANATOMY[f"{_f}0"] = ("hand",)
    ANATOMY[f"{_f}1"] = (f"{_f}0", "hand")
    ANATOMY[f"{_f}2"] = (f"{_f}1",)
    ANATOMY[f"{_f}3"] = (f"{_f}2",)


def safe_bone_name(name):
    return "dms_" + re.sub(r"[^A-Za-z0-9]+", "_", name).strip("_").lower()


def add_physics_bones(hero, fit, fit_data, poses, src, detached, followers, target):
    """Add `detached` bones to the hero's rig where the fit put them,
    parented like in their own rig; each little rig's top bone hangs from
    the hero bone it followed. Returns {source bone: new bone name}."""
    names = {n: safe_bone_name(n) for n in detached}
    view = bpy.context.view_layer
    prev = view.objects.active
    for o in view.objects:
        o.select_set(False)
    view.objects.active = hero
    bpy.ops.object.mode_set(mode="EDIT")
    eb = hero.data.edit_bones
    inv = hero.matrix_world.inverted()
    for n, nn in names.items():
        if nn in eb:
            eb.remove(eb[nn])
        m = inv @ fit.matrix_world @ poses[n]
        b = eb.new(nn)
        length = max(fit_data.bones[n].length, 1e-3)
        b.head = m.translation
        b.tail = m @ Vector((0, length, 0))
        b.align_roll(m.to_3x3() @ Vector((0, 0, 1)))
        b.use_deform = True
    for n, nn in names.items():
        parent = src[n]["bone"].parent
        if parent is not None and parent.name in names:
            eb[nn].parent = eb[names[parent.name]]
        else:
            hero_parent = target.get(followers.get(n)) or target.get(n)
            if hero_parent and hero_parent in eb:
                eb[nn].parent = eb[hero_parent]
        eb[nn].use_connect = False
    bpy.ops.object.mode_set(mode="OBJECT")
    view.objects.active = prev
    return names


def physics_chains(src, detached, names, target, followers):
    """One cloth chain per little rig: its top bone pinned to the hero bone
    it hangs from; short branches (a bow loop) wobble but hold their shape,
    long ones (cables) dangle. The app turns presets into Valve's cloth
    settings."""
    chains = []
    for top in sorted(set(detached.values())):
        if top not in names:
            continue
        kids = {}
        for n in names:
            p = src[n]["bone"].parent
            if p is not None and p.name in names:
                kids.setdefault(p.name, []).append(n)

        def depth_below(n):
            return 1 + max((depth_below(c) for c in kids.get(n, [])), default=0)

        joints = [{"bone": names[top], "parent": "", "simulate": False, "preset": "pinned", "index": 0}]

        def walk(n, branch_len, d):
            dangle = branch_len >= 3
            if dangle and d == 1:
                joints.append({"bone": names[n], "parent": names[src[n]["bone"].parent.name], "simulate": False, "preset": "pinned", "index": 0})
            else:
                joints.append({"bone": names[n], "parent": names[src[n]["bone"].parent.name], "simulate": True, "preset": "dangle" if dangle else "wobbly", "index": d - (2 if dangle else 1)})
            for c in sorted(kids.get(n, [])):
                walk(c, branch_len, d + 1)

        for c in sorted(kids.get(top, [])):
            walk(c, depth_below(c), 1)
        chains.append({"name": names[top], "parent": target.get(followers.get(top)) or "", "joints": joints})
    return chains


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
