//! Put a mesh edited in Blender into a copy of the hero's original DMX: the
//! skeleton, joint list and file settings stay as they were; the vertex
//! data and face sets are replaced.
//!
//! Blender's glTF is Y-up in metres; the game's DMX is in inches with
//! game = (gltf.z, gltf.x, gltf.y) / 0.0254 (checked against Seven's files:
//! every vertex lands within 0.0001 units, UVs unchanged).

use crate::dmx::{self, Attr};
use crate::glb::SkinnedMesh;
use crate::{Error, Result};
use std::collections::{HashMap, HashSet};
use crate::glb::Mat4;

pub const METRES_PER_UNIT: f64 = 0.0254;

pub fn to_game(v: [f64; 3]) -> [f64; 3] {
    [v[2] / METRES_PER_UNIT, v[0] / METRES_PER_UNIT, v[1] / METRES_PER_UNIT]
}

fn to_game_dir(v: [f64; 3]) -> [f64; 3] {
    [v[2], v[0], v[1]]
}

type M3 = [[f64; 3]; 3];

fn m3_mul(a: &M3, b: &M3) -> M3 {
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    r
}

fn m3_t(a: &M3) -> M3 {
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = a[j][i];
        }
    }
    r
}

fn m3_v(a: &M3, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| a[i][0] * v[0] + a[i][1] * v[1] + a[i][2] * v[2])
}

/// Quaternion (x, y, z, w) to rotation matrix.
fn quat_m3(q: [f64; 4]) -> M3 {
    let [x, y, z, w] = q;
    [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
        [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
        [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
    ]
}

fn m3_quat(m: &M3) -> [f64; 4] {
    let tr = m[0][0] + m[1][1] + m[2][2];
    let q = if tr > 0.0 {
        let s = (tr + 1.0).sqrt() * 2.0;
        [(m[2][1] - m[1][2]) / s, (m[0][2] - m[2][0]) / s, (m[1][0] - m[0][1]) / s, 0.25 * s]
    } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
        let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
        [0.25 * s, (m[0][1] + m[1][0]) / s, (m[0][2] + m[2][0]) / s, (m[2][1] - m[1][2]) / s]
    } else if m[1][1] > m[2][2] {
        let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
        [(m[0][1] + m[1][0]) / s, 0.25 * s, (m[1][2] + m[2][1]) / s, (m[0][2] - m[2][0]) / s]
    } else {
        let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
        [(m[0][2] + m[2][0]) / s, (m[1][2] + m[2][1]) / s, 0.25 * s, (m[1][0] - m[0][1]) / s]
    };
    let n = q.iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-12);
    q.map(|x| x / n)
}

fn raw_floats(ty: u8, v: &[f64]) -> Attr {
    Attr::Raw { ty, bytes: v.iter().flat_map(|x| (*x as f32).to_le_bytes()).collect() }
}

fn read_floats(a: Option<&Attr>, n: usize) -> Vec<f64> {
    match a {
        Some(Attr::Raw { bytes, .. }) if bytes.len() >= n * 4 => bytes.chunks_exact(4).take(n).map(|c| f32::from_le_bytes(c.try_into().unwrap()) as f64).collect(),
        _ => vec![0.0; n],
    }
}

/// Add joints (and any missing parents among them) to a DMX skeleton,
/// placed where they are in the glTF. Joint transforms are local to the
/// parent; positions in game units.
fn add_joints(d: &mut dmx::Dmx, model: usize, wanted: &[String], glb: &HashMap<String, (Option<String>, Mat4)>) -> Result<()> {
    // World transform of every existing joint (rotation, position).
    let joints = d.elements[model].elements("jointList");
    let mut parent_of: HashMap<usize, usize> = HashMap::new();
    for &j in &joints {
        for c in d.elements[j].elements("children") {
            parent_of.insert(c, j);
        }
    }
    let mut world: HashMap<String, (M3, [f64; 3])> = HashMap::new();
    fn local(d: &dmx::Dmx, j: usize) -> (M3, [f64; 3]) {
        let t = d.elements[j].element("transform");
        let (p, q) = match t {
            Some(t) => (read_floats(d.elements[t].get("position"), 3), read_floats(d.elements[t].get("orientation"), 4)),
            None => (vec![0.0; 3], vec![0.0, 0.0, 0.0, 1.0]),
        };
        (quat_m3([q[0], q[1], q[2], q[3]]), [p[0], p[1], p[2]])
    }
    for &j in &joints {
        let mut chain = vec![j];
        let mut c = j;
        while let Some(&p) = parent_of.get(&c) {
            chain.push(p);
            c = p;
        }
        let mut r: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let mut t = [0.0; 3];
        for &k in chain.iter().rev() {
            let (lr, lt) = local(d, k);
            let moved = m3_v(&r, lt);
            t = [t[0] + moved[0], t[1] + moved[1], t[2] + moved[2]];
            r = m3_mul(&r, &lr);
        }
        world.insert(d.elements[j].name.clone(), (r, t));
    }
    let by_name: HashMap<String, usize> = joints.iter().map(|&j| (d.elements[j].name.clone(), j)).collect();

    // Bones to add, parents first.
    let mut order: Vec<String> = vec![];
    fn visit(n: &str, glb: &HashMap<String, (Option<String>, Mat4)>, have: &HashMap<String, usize>, order: &mut Vec<String>, depth: usize) {
        if depth > 256 || have.contains_key(n) || order.iter().any(|o| o == n) {
            return;
        }
        if let Some((Some(p), _)) = glb.get(n) {
            visit(p, glb, have, order, depth + 1);
        }
        order.push(n.to_string());
    }
    for w in wanted {
        visit(w, glb, &by_name, &mut order, 0);
    }
    let p: M3 = [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]; // glTF -> game axes
    let mut index = by_name.clone();
    let mut list = joints.iter().map(|&j| j as i32).collect::<Vec<i32>>();
    for n in &order {
        let (parent, m) = &glb[n];
        let parent = parent.clone().filter(|p| index.contains_key(p)).ok_or_else(|| Error::msg(format!("New bone {n} doesn't hang from one of the hero's bones")))?;
        // World transform in game space.
        let mut rg: M3 = [[0.0; 3]; 3];
        for i in 0..3 {
            let len = (0..3).map(|r| m[r][i] * m[r][i]).sum::<f64>().sqrt().max(1e-12);
            for r in 0..3 {
                rg[r][i] = m[r][i] / len;
            }
        }
        let rw = m3_mul(&m3_mul(&p, &rg), &m3_t(&p));
        let tw = to_game([m[0][3], m[1][3], m[2][3]]);
        let (pr, pt) = world[&parent];
        // Valve's bones point along +X to the next bone, and cloth chains
        // need that ("Joint must be aligned along X axis"): turn the new
        // bone so X runs to its child (or on from its parent at the end).
        let child = glb.iter().filter(|(_, (p, _))| p.as_deref() == Some(n.as_str())).map(|(_, (_, cm))| to_game([cm[0][3], cm[1][3], cm[2][3]])).next();
        let dir = match child {
            Some(c) => [c[0] - tw[0], c[1] - tw[1], c[2] - tw[2]],
            None => [tw[0] - pt[0], tw[1] - pt[1], tw[2] - pt[2]],
        };
        let rw = along_x(&rw, dir);
        let lr = m3_mul(&m3_t(&pr), &rw);
        let lt = m3_v(&m3_t(&pr), [tw[0] - pt[0], tw[1] - pt[1], tw[2] - pt[2]]);
        world.insert(n.clone(), (rw, tw));
        let q = m3_quat(&lr);
        let tr = d.add("DmeTransform", n, vec![("position".into(), raw_floats(dmx::T_VEC3, &lt)), ("orientation".into(), raw_floats(13, &q))]);
        let j = d.add(
            "DmeJoint",
            n,
            vec![
                ("transform".into(), Attr::Element(tr as i32)),
                ("shape".into(), Attr::Element(-1)),
                ("visible".into(), Attr::Raw { ty: dmx::T_BOOL, bytes: vec![1] }),
                ("children".into(), dmx::i32_array(dmx::T_ELEMENT, &[])),
            ],
        );
        let pj = index[&parent];
        let mut kids: Vec<i32> = d.elements[pj].elements("children").into_iter().map(|k| k as i32).collect();
        kids.push(j as i32);
        d.elements[pj].set("children", dmx::i32_array(dmx::T_ELEMENT, &kids));
        list.push(j as i32);
        index.insert(n.clone(), j);
    }
    d.elements[model].set("jointList", dmx::i32_array(dmx::T_ELEMENT, &list));
    Ok(())
}

/// `r` turned so its X axis points along `dir`, keeping the old axis that is
/// most across `dir` as Y. Unchanged if `dir` is too short.
fn along_x(r: &M3, dir: [f64; 3]) -> M3 {
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if len < 1e-6 {
        return *r;
    }
    let x = [dir[0] / len, dir[1] / len, dir[2] / len];
    let col = |i: usize| [r[0][i], r[1][i], r[2][i]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let up = (0..3).map(col).min_by(|a, b| dot(*a, x).abs().partial_cmp(&dot(*b, x).abs()).unwrap()).unwrap();
    let d = dot(up, x);
    let mut y = [up[0] - d * x[0], up[1] - d * x[1], up[2] - d * x[2]];
    let yl = dot(y, y).sqrt().max(1e-12);
    y = [y[0] / yl, y[1] / yl, y[2] / yl];
    let z = [x[1] * y[2] - x[2] * y[1], x[2] * y[0] - x[0] * y[2], x[0] * y[1] - x[1] * y[0]];
    [[x[0], y[0], z[0]], [x[1], y[1], z[1]], [x[2], y[2], z[2]]]
}

/// `gigawatt_prisoner.001` -> `gigawatt_prisoner`
pub fn base_material_name(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((b, n)) if n.len() == 3 && n.chars().all(|c| c.is_ascii_digit()) => b,
        _ => name,
    }
}

#[derive(Debug, PartialEq)]
pub enum Swap {
    /// Same geometry and materials as the original: keep the original files.
    Unchanged,
    Replaced { dmx: Vec<u8>, vertices: usize, triangles: usize },
}

struct Stream {
    values: Vec<f32>,
    per: usize,
    indices: Vec<i32>,
}

fn stream(vd: &dmx::Element, name: &str, per: usize) -> Option<Stream> {
    let values = dmx::floats(vd.get(&format!("{name}$0"))?);
    let indices = vd.get(&format!("{name}$0Indices")).map(dmx::ints).unwrap_or_else(|| (0..(values.len() / per) as i32).collect());
    Some(Stream { values, per, indices })
}

/// A vertex for comparison: position, UV, (joint, weight) sorted by joint.
type Fingerprint = ([f64; 3], [f32; 2], Vec<(i32, f32)>);

/// Nearest-neighbour lookup on a grid of 2-unit cells.
struct Grid {
    cells: HashMap<(i64, i64, i64), Vec<usize>>,
    points: Vec<[f64; 3]>,
}

impl Grid {
    fn new(points: Vec<[f64; 3]>) -> Grid {
        let mut cells: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
        for (i, p) in points.iter().enumerate() {
            cells.entry(Self::cell(*p)).or_default().push(i);
        }
        Grid { cells, points }
    }
    fn cell(p: [f64; 3]) -> (i64, i64, i64) {
        ((p[0] / 2.0).floor() as i64, (p[1] / 2.0).floor() as i64, (p[2] / 2.0).floor() as i64)
    }
    /// Points within `r` of `p` (r below the 2-unit cell size).
    fn within(&self, p: [f64; 3], r: f64) -> Vec<usize> {
        let mut out = vec![];
        let lo = Self::cell([p[0] - r, p[1] - r, p[2] - r]);
        let hi = Self::cell([p[0] + r, p[1] + r, p[2] + r]);
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                for z in lo.2..=hi.2 {
                    for &i in self.cells.get(&(x, y, z)).map(|v| v.as_slice()).unwrap_or(&[]) {
                        if (0..3).all(|k| (self.points[i][k] - p[k]).abs() <= r) {
                            out.push(i);
                        }
                    }
                }
            }
        }
        out
    }

    fn nearest(&self, p: [f64; 3]) -> Option<usize> {
        let c = Self::cell(p);
        let d2 = |i: usize| (0..3).map(|k| (self.points[i][k] - p[k]).powi(2)).sum::<f64>();
        for r in 1..=8i64 {
            let mut best: Option<(usize, f64)> = None;
            for dx in -r..=r {
                for dy in -r..=r {
                    for dz in -r..=r {
                        if let Some(list) = self.cells.get(&(c.0 + dx, c.1 + dy, c.2 + dz)) {
                            for &i in list {
                                let d = d2(i);
                                if best.is_none_or(|b| d < b.1) {
                                    best = Some((i, d));
                                }
                            }
                        }
                    }
                }
            }
            if let Some(b) = best {
                return Some(b.0);
            }
        }
        (0..self.points.len()).min_by(|&a, &b| d2(a).total_cmp(&d2(b)))
    }
}

/// Per-vertex tangents (xyz + handedness) from positions, normals and UVs.
pub fn tangents(pos: &[[f64; 3]], nrm: &[[f64; 3]], uv: &[[f32; 2]], tris: &[u32]) -> Vec<[f64; 4]> {
    let n = pos.len();
    let mut t = vec![[0.0f64; 3]; n];
    let mut b = vec![[0.0f64; 3]; n];
    for tri in tris.chunks_exact(3) {
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        let e1 = [0, 1, 2].map(|k| pos[i1][k] - pos[i0][k]);
        let e2 = [0, 1, 2].map(|k| pos[i2][k] - pos[i0][k]);
        let (du1, dv1) = ((uv[i1][0] - uv[i0][0]) as f64, (uv[i1][1] - uv[i0][1]) as f64);
        let (du2, dv2) = ((uv[i2][0] - uv[i0][0]) as f64, (uv[i2][1] - uv[i0][1]) as f64);
        let det = du1 * dv2 - du2 * dv1;
        if det.abs() < 1e-12 {
            continue;
        }
        let r = 1.0 / det;
        let sd = [0, 1, 2].map(|k| (e1[k] * dv2 - e2[k] * dv1) * r);
        let td = [0, 1, 2].map(|k| (e2[k] * du1 - e1[k] * du2) * r);
        for i in [i0, i1, i2] {
            for k in 0..3 {
                t[i][k] += sd[k];
                b[i][k] += td[k];
            }
        }
    }
    (0..n)
        .map(|i| {
            let nn = nrm[i];
            let dot = t[i][0] * nn[0] + t[i][1] * nn[1] + t[i][2] * nn[2];
            let mut tt = [0, 1, 2].map(|k| t[i][k] - nn[k] * dot);
            let len = (tt[0] * tt[0] + tt[1] * tt[1] + tt[2] * tt[2]).sqrt();
            if len < 1e-9 {
                // Any direction perpendicular to the normal.
                let a = if nn[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
                let d = a[0] * nn[0] + a[1] * nn[1] + a[2] * nn[2];
                tt = [0, 1, 2].map(|k| a[k] - nn[k] * d);
            }
            let len = (tt[0] * tt[0] + tt[1] * tt[1] + tt[2] * tt[2]).sqrt().max(1e-12);
            tt = tt.map(|x| x / len);
            let c = [nn[1] * tt[2] - nn[2] * tt[1], nn[2] * tt[0] - nn[0] * tt[2], nn[0] * tt[1] - nn[1] * tt[0]];
            let w = if c[0] * b[i][0] + c[1] * b[i][1] + c[2] * b[i][2] < 0.0 { -1.0 } else { 1.0 };
            [tt[0], tt[1], tt[2], w]
        })
        .collect()
}

/// Replace the mesh in `original` (a part's DMX) with `meshes`.
/// `materials` maps a material name (as in Blender, or a hero material's
/// base name) to its `.vmat` path.
pub fn swap(original: &[u8], meshes: &[&SkinnedMesh], materials: &HashMap<String, String>, tangent_sign: f64) -> Result<Swap> {
    swap_with_bones(original, meshes, materials, tangent_sign, &HashMap::new(), &[])
}

/// `swap`, adding bones the meshes use that the part's skeleton doesn't
/// have yet (bones added in Blender for physics). `glb_joints`: every joint
/// of the glTF (name -> parent, world matrix at rest).
/// `also`: bones to add even if no vertex uses them (cloth chain bones).
pub fn swap_with_bones(original: &[u8], meshes: &[&SkinnedMesh], materials: &HashMap<String, String>, tangent_sign: f64, glb_joints: &HashMap<String, (Option<String>, Mat4)>, also: &[String]) -> Result<Swap> {
    let mut d = dmx::parse(original)?;
    let model = d.elements[0].element("model").ok_or_else(|| Error::msg("DMX has no model"))?;
    let existing: HashSet<String> = d.elements[model].elements("jointList").iter().map(|&i| d.elements[i].name.clone()).collect();
    let mut wanted: Vec<String> = vec![];
    for m in meshes {
        for v in &m.vertices {
            for (j, _) in &v.weights {
                let b = &m.bones[*j];
                if !existing.contains(b) && glb_joints.contains_key(b) && !wanted.contains(b) {
                    wanted.push(b.clone());
                }
            }
        }
    }
    if !wanted.is_empty() {
        for b in also {
            if !existing.contains(b) && glb_joints.contains_key(b) && !wanted.contains(b) {
                wanted.push(b.clone());
            }
        }
        add_joints(&mut d, model, &wanted, glb_joints)?;
    }
    let joint_names: Vec<String> = d.elements[model].elements("jointList").iter().map(|&i| d.elements[i].name.clone()).collect();
    let joint_index: HashMap<&str, usize> = joint_names.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
    let mesh_el = *d.find_type("DmeMesh").first().ok_or_else(|| Error::msg("DMX has no mesh"))?;
    let vd_el = d.elements[mesh_el].element("currentState").ok_or_else(|| Error::msg("DMX mesh has no vertex data"))?;

    // Gather the new geometry in game space.
    let mut pos = vec![];
    let mut nrm = vec![];
    let mut uv = vec![];
    let mut infl: Vec<[(i32, f32); 4]> = vec![];
    let mut sets: Vec<(String, Vec<u32>)> = vec![];
    for m in meshes {
        let base = pos.len() as u32;
        for v in &m.vertices {
            pos.push(to_game(v.pos));
            nrm.push(to_game_dir(v.normal));
            uv.push(v.uv);
            let mut w4 = [(0i32, 0f32); 4];
            let kept: Vec<&(usize, f32)> = v.weights.iter().take(4).collect();
            let total: f32 = kept.iter().map(|x| x.1).sum::<f32>().max(1e-9);
            for (k, (j, w)) in kept.into_iter().enumerate() {
                let bone = &m.bones[*j];
                let idx = *joint_index.get(bone.as_str()).ok_or_else(|| {
                    Error::msg(format!("{}: bone {bone} isn't in the hero's skeleton (new bones aren't supported yet)", m.object))
                })?;
                w4[k] = (idx as i32, w / total);
            }
            infl.push(w4);
        }
        for (mat, tris) in &m.sets {
            if mat.is_empty() {
                return Err(Error::msg(format!("{} has faces without a material. Give it a material in Blender.", m.object)));
            }
            // New materials go by their exact name, the hero's by base name
            // (Blender may have added .001).
            let path = materials
                .get(mat.as_str())
                .or_else(|| materials.get(base_material_name(mat)))
                .ok_or_else(|| Error::msg(format!("{}: material \"{mat}\" wasn't sent from Blender", m.object)))?
                .clone();
            let shifted: Vec<u32> = tris.iter().map(|i| i + base).collect();
            match sets.iter_mut().find(|s| s.0 == path) {
                Some(s) => s.1.extend(shifted),
                None => sets.push((path, shifted)),
            }
        }
    }
    if pos.is_empty() {
        return Err(Error::msg("No geometry for this part"));
    }

    // Unchanged? The same vertices (position, UV and bone weights) and the
    // same materials.
    let vd = d.elements[vd_el].clone();
    let orig_pos = stream(&vd, "position", 3).ok_or_else(|| Error::msg("DMX has no positions"))?;
    let orig_points: Vec<[f64; 3]> = orig_pos
        .indices
        .iter()
        .map(|&i| {
            let i = i as usize * 3;
            [orig_pos.values[i] as f64, orig_pos.values[i + 1] as f64, orig_pos.values[i + 2] as f64]
        })
        .collect();
    let orig_uv = stream(&vd, "texcoord", 2);
    let orig_jc = match vd.get("jointCount") {
        Some(Attr::Raw { bytes, .. }) if bytes.len() == 4 => i32::from_le_bytes(bytes[..4].try_into().unwrap()).max(0) as usize,
        _ => 0,
    };
    let orig_bi = vd.get("blendindices$0").map(dmx::ints).unwrap_or_default();
    let orig_bw = vd.get("blendweights$0").map(dmx::floats).unwrap_or_default();
    // One vertex as (position, uv, bone weights sorted by bone).
    let mut orig_v: Vec<Fingerprint> = vec![];
    for (k, point) in orig_points.iter().enumerate() {
        let pi = orig_pos.indices[k] as usize;
        let uvv = orig_uv
            .as_ref()
            .map(|u| {
                let j = u.indices.get(k).copied().unwrap_or(0) as usize * 2;
                [u.values.get(j).copied().unwrap_or(0.0), u.values.get(j + 1).copied().unwrap_or(0.0)]
            })
            .unwrap_or([0.0, 0.0]);
        let mut ws: Vec<(i32, f32)> = (0..orig_jc)
            .filter_map(|t| Some((*orig_bi.get(pi * orig_jc + t)?, *orig_bw.get(pi * orig_jc + t)?)))
            .filter(|w| w.1 > 0.0)
            .collect();
        ws.sort_by_key(|w| w.0);
        orig_v.push((*point, uvv, ws));
    }
    let new_v: Vec<Fingerprint> = (0..pos.len())
        .map(|i| {
            let mut ws: Vec<(i32, f32)> = infl[i].iter().copied().filter(|w| w.1 > 0.0).collect();
            ws.sort_by_key(|w| w.0);
            (pos[i], uv[i], ws)
        })
        .collect();
    let same = |a: &Fingerprint, b: &Fingerprint| {
        (0..3).all(|k| (a.0[k] - b.0[k]).abs() < 0.01)
            && (a.1[0] - b.1[0]).abs() < 0.001
            && (a.1[1] - b.1[1]).abs() < 0.001
            && a.2.len() == b.2.len()
            && a.2.iter().zip(&b.2).all(|(x, y)| x.0 == y.0 && (x.1 - y.1).abs() < 0.005)
    };
    // Every vertex on each side has an identical partner on the other.
    let all_matched = |from: &[Fingerprint], to: &[Fingerprint]| -> usize {
        let grid = Grid::new(to.iter().map(|v| v.0).collect());
        from.iter().filter(|v| !grid.within(v.0, 0.01).iter().any(|&j| same(v, &to[j]))).count()
    };
    let orig_mats: HashSet<String> = d.find_type("DmeMaterial").iter().filter_map(|&i| d.elements[i].str("mtlName").map(|s| s.replace('\\', "/").to_lowercase())).collect();
    let new_mats: HashSet<String> = sets.iter().map(|s| s.0.to_lowercase()).collect();
    if orig_mats == new_mats {
        let missing_new = all_matched(&new_v, &orig_v);
        let missing_orig = if missing_new == 0 { all_matched(&orig_v, &new_v) } else { usize::MAX };
        if std::env::var("DMS_DEBUG").is_ok() {
            eprintln!("unchanged check: {missing_new} new and {missing_orig} original vertices without a partner");
        }
        if missing_new == 0 && missing_orig == 0 {
            return Ok(Swap::Unchanged);
        }
    }

    // Vertex colour (tint mask) carries over from the nearest original vertex.
    let colors = stream(&vd, "color", 4).map(|c| {
        let grid = Grid::new(orig_points.clone());
        pos.iter()
            .map(|p| {
                let k = grid.nearest(*p).unwrap_or(0);
                let ci = c.indices.get(k).copied().unwrap_or(0) as usize * c.per;
                [0, 1, 2, 3].map(|x| c.values.get(ci + x).copied().unwrap_or(1.0))
            })
            .collect::<Vec<[f32; 4]>>()
    });

    let all_tris: Vec<u32> = sets.iter().flat_map(|s| s.1.iter().copied()).collect();
    let tan = tangents(&pos, &nrm, &uv, &all_tris);
    let n = pos.len();
    let ident: Vec<i32> = (0..n as i32).collect();
    let flat3 = |v: &Vec<[f64; 3]>| v.iter().flat_map(|x| x.map(|y| y as f32)).collect::<Vec<f32>>();
    let mut format = vec!["position$0", "texcoord$0", "normal$0", "tangent$0", "blendindices$0", "blendweights$0"];
    let mut attrs: Vec<(String, Attr)> = vec![
        ("vertexFormat".into(), Attr::StrArray(vec![])),
        ("jointCount".into(), Attr::Raw { ty: dmx::T_INT, bytes: 4i32.to_le_bytes().to_vec() }),
        ("flipVCoordinates".into(), Attr::Raw { ty: dmx::T_BOOL, bytes: vec![0] }),
        ("position$0".into(), dmx::f32_array(dmx::T_VEC3, &flat3(&pos))),
        ("position$0Indices".into(), dmx::i32_array(dmx::T_INT, &ident)),
        ("texcoord$0".into(), dmx::f32_array(dmx::T_VEC2, &uv.iter().flat_map(|x| *x).collect::<Vec<f32>>())),
        ("texcoord$0Indices".into(), dmx::i32_array(dmx::T_INT, &ident)),
        ("normal$0".into(), dmx::f32_array(dmx::T_VEC3, &flat3(&nrm))),
        ("normal$0Indices".into(), dmx::i32_array(dmx::T_INT, &ident)),
        ("tangent$0".into(), dmx::f32_array(dmx::T_VEC4, &tan.iter().flat_map(|t| [t[0] as f32, t[1] as f32, t[2] as f32, (t[3] * tangent_sign) as f32]).collect::<Vec<f32>>())),
        ("tangent$0Indices".into(), dmx::i32_array(dmx::T_INT, &ident)),
        ("blendindices$0".into(), dmx::i32_array(dmx::T_INT, &infl.iter().flat_map(|w| w.map(|x| x.0)).collect::<Vec<i32>>())),
        ("blendweights$0".into(), dmx::f32_array(dmx::T_FLOAT, &infl.iter().flat_map(|w| w.map(|x| x.1)).collect::<Vec<f32>>())),
    ];
    if let Some(c) = &colors {
        format.push("color$0");
        attrs.push(("color$0".into(), dmx::f32_array(dmx::T_VEC4, &c.iter().flat_map(|x| *x).collect::<Vec<f32>>())));
        attrs.push(("color$0Indices".into(), dmx::i32_array(dmx::T_INT, &ident)));
    }
    attrs[0].1 = Attr::StrArray(format.iter().map(|s| s.to_string()).collect());
    d.elements[vd_el].attrs = attrs;

    // Face sets, one per material.
    let mut face_sets = vec![];
    for (path, tris) in &sets {
        let faces: Vec<i32> = tris.chunks_exact(3).flat_map(|t| [t[0] as i32, t[1] as i32, t[2] as i32, -1]).collect();
        let mat = d.add("DmeMaterial", "material", vec![("mtlName".into(), Attr::Str(path.clone()))]);
        let fs = d.add("DmeFaceSet", "", vec![("faces".into(), dmx::i32_array(dmx::T_INT, &faces)), ("material".into(), Attr::Element(mat as i32))]);
        face_sets.push(fs as i32);
    }
    d.elements[mesh_el].set("faceSets", dmx::i32_array(dmx::T_ELEMENT, &face_sets));
    // A part can hold several meshes (Celeste's body: skin, hair, clothes...).
    // The new geometry went into the first; the others go.
    let others: std::collections::HashSet<usize> = d.find_type("DmeDag").into_iter().filter(|&g| d.elements[g].element("shape").is_some_and(|m| m != mesh_el)).collect();
    // Their nodes stay (the joint list's order matters), without a mesh.
    for g in others {
        d.elements[g].set("shape", Attr::Element(-1));
    }
    d.compact();
    Ok(Swap::Replaced { dmx: dmx::write(&d), vertices: n, triangles: all_tris.len() / 3 })
}

#[cfg(test)]
mod along_x_tests {
    use super::*;
    #[test]
    fn new_bones_point_along_x() {
        let r: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let m = along_x(&r, [0.0, 0.0, -3.0]);
        assert_eq!([m[0][0], m[1][0], m[2][0]], [0.0, 0.0, -1.0]);
        let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        assert!((det - 1.0).abs() < 1e-9, "a rotation");
        assert_eq!(along_x(&r, [0.0; 3]), r);
    }
}
