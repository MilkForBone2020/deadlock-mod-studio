# SPDX-License-Identifier: GPL-3.0-or-later
"""Physics chains: make bones through a selected part of a mesh (a ponytail,
a cable, a strip of cape), weight it to them, and record a cloth chain for
the app. The app writes Valve's cloth settings from the chain's preset.

Chains are kept in the scene as JSON under "dms_physics" (the same format
Fit to hero records):
  [{"name", "parent", "joints": [{"bone", "parent", "simulate", "preset", "index"}]}]
"""

import heapq
import json
import re

import bpy
from mathutils import Vector

PRESETS = [
    ("dangle", "Dangling", "Hangs and sways: cables, ponytails, straps"),
    ("wobbly", "Wobbly", "Jiggles but holds its shape: bows, ears, antennae"),
    ("stiff", "Stiff", "Barely moves: armour flaps, stiff ribbons"),
    ("floppy", "Floppy", "Very loose: long cloth, streamers"),
]


def load(scene):
    return json.loads(scene.get("dms_physics", "[]"))


def save(scene, chains):
    scene["dms_physics"] = json.dumps(chains)


def safe(name):
    return re.sub(r"[^a-z0-9]+", "_", name.lower()).strip("_") or "chain"


def selected_vertices(obj):
    """Indices of the selected vertices (works from Edit Mode)."""
    if obj.mode == "EDIT":
        obj.update_from_editmode()
    return [v.index for v in obj.data.vertices if v.select]


def main_bone(obj, verts, rig):
    """The rig bone the given vertices follow most."""
    names = {g.index: g.name for g in obj.vertex_groups}
    score = {}
    for i in verts:
        for g in obj.data.vertices[i].groups:
            n = names.get(g.group)
            if n in rig.data.bones:
                score[n] = score.get(n, 0.0) + g.weight
    return max(score, key=score.get) if score else ""


def path_params(obj, verts, start_point):
    """For each selected vertex, how far along the part it is (0 at the end
    nearest `start_point`, 1 at the far end), measured along the mesh's
    edges so curved parts are followed."""
    me = obj.data
    mw = obj.matrix_world
    sel = set(verts)
    co = {i: mw @ me.vertices[i].co for i in verts}
    nbrs = {i: [] for i in verts}
    for e in me.edges:
        a, b = e.vertices
        if a in sel and b in sel:
            d = (co[a] - co[b]).length
            nbrs[a].append((b, d))
            nbrs[b].append((a, d))
    # Start from the vertices nearest the parent (a small band, so a thick
    # part's whole root ring starts at 0).
    dists = sorted(verts, key=lambda i: (co[i] - start_point).length)
    seeds = dists[: max(1, len(verts) // 30)]
    dist = {i: float("inf") for i in verts}
    heap = []
    for s in seeds:
        dist[s] = 0.0
        heapq.heappush(heap, (0.0, s))
    while heap:
        d, i = heapq.heappop(heap)
        if d > dist[i]:
            continue
        for j, w in nbrs[i]:
            nd = d + w
            if nd < dist[j]:
                dist[j] = nd
                heapq.heappush(heap, (nd, j))
    # Pieces not joined by edges: straight-line distance from the start.
    for i in verts:
        if dist[i] == float("inf"):
            dist[i] = (co[i] - co[seeds[0]]).length
    top = max(dist.values()) or 1.0
    return {i: dist[i] / top for i in verts}, co


def make_chain(obj, rig, name, parent, count, preset, scene=None):
    """Make `count` bones through the selected vertices of `obj`, hanging
    from `parent`, weight those vertices along them, and record the chain."""
    scene = scene or bpy.context.scene
    verts = selected_vertices(obj)
    if len(verts) < 3:
        raise RuntimeError("Select the part that should move first (in Edit Mode)")
    if parent not in rig.data.bones:
        raise RuntimeError(f"{parent} isn't one of the rig's bones")
    count = max(2, min(12, int(count)))
    base = f"dms_{safe(name)}"
    chains = [c for c in load(scene) if c["name"] != base]
    pb = rig.data.bones[parent]
    start = rig.matrix_world @ pb.head_local
    tip = rig.matrix_world @ pb.tail_local
    # Start from whichever end of the part is nearest the parent bone.
    t, co = path_params(obj, verts, (start + tip) / 2)

    # Joint positions: the middle of each band along the part.
    joints = []
    for k in range(count + 1):
        lo, hi = (k - 0.5) / count, (k + 0.5) / count
        band = [co[i] for i in verts if lo <= t[i] <= hi] or [co[min(verts, key=lambda i: abs(t[i] - k / count))]]
        joints.append(sum(band, Vector()) / len(band))

    # Bones (one per segment, the last one pointing on from the previous).
    if obj.mode == "EDIT":
        bpy.ops.object.mode_set(mode="OBJECT")
    view = bpy.context.view_layer
    prev = view.objects.active
    for o in view.objects:
        o.select_set(False)
    view.objects.active = rig
    bpy.ops.object.mode_set(mode="EDIT")
    eb = rig.data.edit_bones
    inv = rig.matrix_world.inverted()
    names = [f"{base}_{k}" for k in range(count)]
    for n in names:
        if n in eb:
            eb.remove(eb[n])
    last = None
    for k, n in enumerate(names):
        b = eb.new(n)
        b.head = inv @ joints[k]
        b.tail = inv @ joints[k + 1]
        if (b.tail - b.head).length < 1e-4:
            b.tail = b.head + Vector((0, 0, 0.01))
        b.parent = last or eb[parent]
        b.use_connect = False
        b.use_deform = True
        last = b
    bpy.ops.object.mode_set(mode="OBJECT")
    view.objects.active = prev

    # Weights: each vertex between the two bones nearest its place along
    # the part; the selection no longer follows its old bones.
    groups = [obj.vertex_groups.get(n) or obj.vertex_groups.new(name=n) for n in names]
    for g in obj.vertex_groups:
        if g.name not in names:
            g.remove(verts)
    for i in verts:
        x = t[i] * count - 0.5
        k = max(0, min(count - 1, int(x)))
        f = max(0.0, min(1.0, x - k))
        if k + 1 < count:
            groups[k].add([i], 1.0 - f, "REPLACE")
            groups[k + 1].add([i], f, "REPLACE")
        else:
            groups[k].add([i], 1.0, "REPLACE")
    # Groups left empty by the move (old bones' groups) go.
    used = {g.group for v in obj.data.vertices for g in v.groups if g.weight > 0}
    for g in list(obj.vertex_groups):
        if g.index not in used and g.name not in names and g.name not in rig.data.bones:
            obj.vertex_groups.remove(g)
    if not any(m.type == "ARMATURE" and m.object == rig for m in obj.modifiers):
        obj.modifiers.new("Armature", "ARMATURE").object = rig

    entry = {
        "name": base,
        "parent": parent,
        "joints": [{"bone": names[0], "parent": "", "simulate": False, "preset": "pinned", "index": 0}]
        + [{"bone": n, "parent": names[k], "simulate": True, "preset": preset, "index": k} for k, n in enumerate(names[1:])],
    }
    chains.append(entry)
    save(scene, chains)
    return entry


def original_bones(scene):
    return set(json.loads(scene.get("dms_bones", "{}")))


def chains_from_bones(scene, rig, bone_names, preset):
    """Turn bones the model already has (not the hero's own) into cloth
    chains. Each selected branch gets a small pinned anchor bone at its
    base, hanging from the bone the branch hung from, so its first bone
    moves too (single jiggle bones included). Bones are renamed dms_...
    (Blender renames their vertex groups to match). Returns the chains."""
    originals = original_bones(scene)
    picked = [b for b in bone_names if b in rig.data.bones]
    hero_own = [b for b in picked if b in originals]
    if hero_own:
        raise RuntimeError(f"{', '.join(hero_own[:3])}: the hero's own bones are animated, so they can't be physics bones")
    if not picked:
        raise RuntimeError("Select the bones that should move (Pose Mode)")
    sel = set(picked)
    roots = [b for b in picked if not (rig.data.bones[b].parent and rig.data.bones[b].parent.name in sel)]
    if any(rig.data.bones[r].parent is None for r in roots):
        raise RuntimeError("Each chain needs a bone to hang from: parent its top bone to a body bone first")

    view = bpy.context.view_layer
    prev = view.objects.active
    if bpy.context.object and bpy.context.object.mode != "OBJECT":
        bpy.ops.object.mode_set(mode="OBJECT")
    # Safe names first (vertex groups follow).
    rename = {}
    for b in picked:
        new = b if b.startswith("dms_") else f"dms_{safe(b)}"
        n, k = new, 2
        while n != b and n in rig.data.bones:
            n, k = f"{new}_{k}", k + 1
        rig.data.bones[b].name = n
        rename[b] = n
    picked = [rename[b] for b in picked]
    roots = [rename[r] for r in roots]

    for o in view.objects:
        o.select_set(False)
    view.objects.active = rig
    bpy.ops.object.mode_set(mode="EDIT")
    eb = rig.data.edit_bones
    anchors = {}
    for r in roots:
        b = eb[r]
        parent = b.parent
        a_name = f"{r}_anchor"
        if a_name in eb:
            eb.remove(eb[a_name])
        a = eb.new(a_name)
        a.head = b.head
        a.tail = b.head + (b.tail - b.head).normalized() * min(0.02, (b.tail - b.head).length / 3)
        a.roll = b.roll
        a.parent = parent
        a.use_deform = True
        b.parent = a
        b.use_connect = False
        anchors[r] = (a_name, parent.name)
    bpy.ops.object.mode_set(mode="OBJECT")
    view.objects.active = prev

    chains = load(scene)
    made = []
    kids = {}
    for b in picked:
        p = rig.data.bones[b].parent
        if p is not None and p.name in picked:
            kids.setdefault(p.name, []).append(b)
    for r in roots:
        a_name, hero_parent = anchors[r]
        joints = [{"bone": a_name, "parent": "", "simulate": False, "preset": "pinned", "index": 0}]

        def walk(b, parent, d):
            joints.append({"bone": b, "parent": parent, "simulate": True, "preset": preset, "index": d})
            for c in sorted(kids.get(b, [])):
                walk(c, b, d + 1)

        walk(r, a_name, 0)
        entry = {"name": r, "parent": hero_parent, "joints": joints}
        chains = [c for c in chains if c["name"] != r]
        chains.append(entry)
        made.append(entry)
    save(scene, chains)
    return made


def remove_chain(scene, rig, name):
    """Delete a chain's bones; their weights go back to the bone it hung from."""
    chains = load(scene)
    chain = next((c for c in chains if c["name"] == name), None)
    if chain is None:
        return
    bones = [j["bone"] for j in chain["joints"]]
    parent = chain.get("parent") or ""
    for o in scene.objects:
        if o.type != "MESH":
            continue
        groups = [o.vertex_groups.get(b) for b in bones]
        groups = [g for g in groups if g]
        if not groups:
            continue
        idx = {g.index for g in groups}
        back = o.vertex_groups.get(parent) or (o.vertex_groups.new(name=parent) if parent else None)
        for v in o.data.vertices:
            w = sum(g.weight for g in v.groups if g.group in idx)
            if w > 0 and back is not None:
                back.add([v.index], w, "ADD")
        for g in groups:
            o.vertex_groups.remove(g)
    view = bpy.context.view_layer
    prev = view.objects.active
    view.objects.active = rig
    bpy.ops.object.mode_set(mode="EDIT")
    for b in bones:
        if b in rig.data.edit_bones:
            rig.data.edit_bones.remove(rig.data.edit_bones[b])
    bpy.ops.object.mode_set(mode="OBJECT")
    view.objects.active = prev
    save(scene, [c for c in chains if c["name"] != name])


def set_preset(scene, name, preset):
    chains = load(scene)
    for c in chains:
        if c["name"] == name:
            for j in c["joints"]:
                if j["simulate"]:
                    j["preset"] = preset
    save(scene, chains)
