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

pub const METRES_PER_UNIT: f64 = 0.0254;

pub fn to_game(v: [f64; 3]) -> [f64; 3] {
    [v[2] / METRES_PER_UNIT, v[0] / METRES_PER_UNIT, v[1] / METRES_PER_UNIT]
}

fn to_game_dir(v: [f64; 3]) -> [f64; 3] {
    [v[2], v[0], v[1]]
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
/// `materials` maps a material name (as in Blender) to its `.vmat` path.
pub fn swap(original: &[u8], meshes: &[&SkinnedMesh], materials: &HashMap<String, String>, tangent_sign: f64) -> Result<Swap> {
    let mut d = dmx::parse(original)?;
    let model = d.elements[0].element("model").ok_or_else(|| Error::msg("DMX has no model"))?;
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
            let key = base_material_name(mat).to_string();
            let path = materials
                .get(&key)
                .ok_or_else(|| Error::msg(format!("{}: material \"{mat}\" isn't one of the hero's materials (new materials aren't supported yet)", m.object)))?
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
    d.compact();
    Ok(Swap::Replaced { dmx: dmx::write(&d), vertices: n, triangles: all_tris.len() / 3 })
}
