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
