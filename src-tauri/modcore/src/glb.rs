//! Just enough glTF 2.0 (binary `.glb`) reading for the models Blender sends
//! back: nodes, skinned meshes, materials and their accessors.

use std::collections::{HashMap, HashSet};
use crate::{Error, Result};
use serde_json::Value;

pub struct Glb {
    pub json: Value,
    pub bin: Vec<u8>,
}

pub type Mat4 = [[f64; 4]; 4];

fn bad(m: &str) -> Error {
    Error::msg(format!("glTF: {m}"))
}

pub fn parse(bytes: &[u8]) -> Result<Glb> {
    let u = |o: usize| -> Result<usize> {
        bytes.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize).ok_or_else(|| bad("file is cut short"))
    };
    if bytes.get(..4) != Some(b"glTF") {
        return Err(bad("not a .glb file"));
    }
    let jlen = u(12)?;
    if bytes.get(16..20) != Some(b"JSON") {
        return Err(bad("first chunk isn't JSON"));
    }
    let json: Value = serde_json::from_slice(bytes.get(20..20 + jlen).ok_or_else(|| bad("bad JSON chunk"))?)?;
    let mut bin = vec![];
    let off = 20 + jlen;
    if off + 8 <= bytes.len() {
        let blen = u(off)?;
        if bytes.get(off + 4..off + 8) == Some(b"BIN\0") {
            bin = bytes.get(off + 8..off + 8 + blen).ok_or_else(|| bad("bad BIN chunk"))?.to_vec();
        }
    }
    Ok(Glb { json, bin })
}

fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut r = [[0.0; 4]; 4];
    for (i, row) in r.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..4).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    r
}

pub fn identity() -> Mat4 {
    let mut m = [[0.0; 4]; 4];
    for (i, row) in m.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    m
}

impl Glb {
    fn arr(&self, key: &str) -> &[Value] {
        self.json.get(key).and_then(|v| v.as_array()).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// An accessor as floats (normalized integers scaled to 0..1), with its
    /// component count.
    pub fn floats(&self, idx: usize) -> Result<(Vec<f32>, usize)> {
        let a = self.arr("accessors").get(idx).ok_or_else(|| bad("missing accessor"))?;
        let count = a["count"].as_u64().unwrap_or(0) as usize;
        let n = match a["type"].as_str().unwrap_or("") {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            "MAT4" => 16,
            t => return Err(bad(&format!("unsupported accessor type {t}"))),
        };
        let ct = a["componentType"].as_u64().unwrap_or(0);
        let size = match ct {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            _ => return Err(bad("unsupported component type")),
        };
        let normalized = a["normalized"].as_bool().unwrap_or(false);
        let mut out = vec![0f32; count * n];
        let Some(bvi) = a["bufferView"].as_u64() else {
            return Ok((out, n)); // all zeros (sparse not supported)
        };
        let bv = self.arr("bufferViews").get(bvi as usize).ok_or_else(|| bad("missing buffer view"))?;
        let base = bv["byteOffset"].as_u64().unwrap_or(0) as usize + a["byteOffset"].as_u64().unwrap_or(0) as usize;
        let stride = bv["byteStride"].as_u64().map(|s| s as usize).unwrap_or(size * n);
        for i in 0..count {
            for c in 0..n {
                let o = base + i * stride + c * size;
                let b = self.bin.get(o..o + size).ok_or_else(|| bad("accessor outside the buffer"))?;
                out[i * n + c] = match ct {
                    5126 => f32::from_le_bytes(b.try_into().unwrap()),
                    5121 => b[0] as f32 / if normalized { 255.0 } else { 1.0 },
                    5123 => u16::from_le_bytes(b.try_into().unwrap()) as f32 / if normalized { 65535.0 } else { 1.0 },
                    5125 => u32::from_le_bytes(b.try_into().unwrap()) as f32,
                    5120 => (b[0] as i8) as f32 / if normalized { 127.0 } else { 1.0 },
                    _ => (i16::from_le_bytes(b.try_into().unwrap())) as f32 / if normalized { 32767.0 } else { 1.0 },
                };
            }
        }
        Ok((out, n))
    }

    /// Indices as integers (exact for u32 too).
    pub fn indices(&self, idx: usize) -> Result<Vec<u32>> {
        let a = self.arr("accessors").get(idx).ok_or_else(|| bad("missing accessor"))?;
        let count = a["count"].as_u64().unwrap_or(0) as usize;
        let ct = a["componentType"].as_u64().unwrap_or(0);
        let size = match ct {
            5121 => 1,
            5123 => 2,
            5125 => 4,
            _ => return Err(bad("bad index type")),
        };
        let bv = self.arr("bufferViews").get(a["bufferView"].as_u64().unwrap_or(u64::MAX) as usize).ok_or_else(|| bad("missing buffer view"))?;
        let base = bv["byteOffset"].as_u64().unwrap_or(0) as usize + a["byteOffset"].as_u64().unwrap_or(0) as usize;
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let o = base + i * size;
            let b = self.bin.get(o..o + size).ok_or_else(|| bad("indices outside the buffer"))?;
            out.push(match size {
                1 => b[0] as u32,
                2 => u16::from_le_bytes(b.try_into().unwrap()) as u32,
                _ => u32::from_le_bytes(b.try_into().unwrap()),
            });
        }
        Ok(out)
    }

    pub fn node_name(&self, i: usize) -> String {
        self.arr("nodes").get(i).and_then(|n| n["name"].as_str()).unwrap_or("").to_string()
    }

    fn local(&self, i: usize) -> Mat4 {
        let n = &self.arr("nodes")[i];
        if let Some(m) = n["matrix"].as_array() {
            let v: Vec<f64> = m.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect();
            let mut r = [[0.0; 4]; 4];
            for (c, col) in v.chunks(4).enumerate().take(4) {
                for (rr, val) in col.iter().enumerate() {
                    r[rr][c] = *val;
                }
            }
            return r;
        }
        let get = |k: &str, d: &[f64]| -> Vec<f64> {
            n[k].as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect()).unwrap_or_else(|| d.to_vec())
        };
        let t = get("translation", &[0.0, 0.0, 0.0]);
        let q = get("rotation", &[0.0, 0.0, 0.0, 1.0]);
        let s = get("scale", &[1.0, 1.0, 1.0]);
        let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
        let r = [
            [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
            [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
            [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
        ];
        let mut m = identity();
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] = r[i][j] * s[j];
            }
            m[i][3] = t[i];
        }
        m
    }

    /// World matrix of every node.
    pub fn world_matrices(&self) -> Vec<Mat4> {
        let nodes = self.arr("nodes");
        let mut parent = vec![usize::MAX; nodes.len()];
        for (i, n) in nodes.iter().enumerate() {
            for c in n["children"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
                if let Some(c) = c.as_u64() {
                    if (c as usize) < parent.len() {
                        parent[c as usize] = i;
                    }
                }
            }
        }
        (0..nodes.len())
            .map(|i| {
                let mut m = self.local(i);
                let mut p = parent[i];
                let mut guard = 0;
                while p != usize::MAX && guard < 1024 {
                    m = mul(&self.local(p), &m);
                    p = parent[p];
                    guard += 1;
                }
                m
            })
            .collect()
    }
}

/// Every skin joint: name -> (parent joint name, world matrix at rest).
pub fn joints(g: &Glb) -> HashMap<String, (Option<String>, Mat4)> {
    let world = g.world_matrices();
    let nodes = g.arr("nodes");
    let mut parent = vec![usize::MAX; nodes.len()];
    for (i, n) in nodes.iter().enumerate() {
        for c in n["children"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
            if let Some(c) = c.as_u64() {
                if (c as usize) < parent.len() {
                    parent[c as usize] = i;
                }
            }
        }
    }
    let mut joint_nodes = HashSet::new();
    for skin in g.arr("skins") {
        for j in skin["joints"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
            if let Some(j) = j.as_u64() {
                joint_nodes.insert(j as usize);
            }
        }
    }
    joint_nodes
        .iter()
        .filter(|&&j| j < nodes.len())
        .map(|&j| {
            let p = parent[j];
            let pname = (p != usize::MAX && joint_nodes.contains(&p)).then(|| g.node_name(p));
            (g.node_name(j), (pname, world[j]))
        })
        .collect()
}

/// One vertex of a skinned mesh, in the bind pose, in glTF space.
#[derive(Clone, Debug, PartialEq)]
pub struct Vertex {
    pub pos: [f64; 3],
    pub normal: [f64; 3],
    pub uv: [f32; 2],
    /// (bone name index into `SkinnedMesh::bones`, weight), strongest first.
    pub weights: Vec<(usize, f32)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkinnedMesh {
    /// The object (node) name in Blender.
    pub object: String,
    pub bones: Vec<String>,
    pub vertices: Vec<Vertex>,
    /// (material name, triangle vertex indices)
    pub sets: Vec<(String, Vec<u32>)>,
}

fn apply(m: &Mat4, v: [f64; 3], w: f64) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2] + m[i][3] * w)
}

/// Every skinned mesh node, with vertices moved into the bind pose.
pub fn skinned_meshes(g: &Glb) -> Result<Vec<SkinnedMesh>> {
    let world = g.world_matrices();
    let nodes = g.arr("nodes");
    let materials = g.arr("materials");
    let mut out = vec![];
    for (ni, n) in nodes.iter().enumerate() {
        let Some(mi) = n["mesh"].as_u64() else { continue };
        let object = g.node_name(ni);
        let Some(si) = n["skin"].as_u64() else {
            return Err(Error::msg(format!("{object} isn't skinned to the rig")));
        };
        let skin = &g.arr("skins")[si as usize];
        let joints: Vec<usize> = skin["joints"].as_array().map(|a| a.iter().filter_map(|x| x.as_u64().map(|x| x as usize)).collect()).unwrap_or_default();
        let bones: Vec<String> = joints.iter().map(|&j| g.node_name(j)).collect();
        let ibm: Vec<Mat4> = match skin["inverseBindMatrices"].as_u64() {
            Some(a) => {
                let (f, _) = g.floats(a as usize)?;
                f.chunks(16)
                    .map(|c| {
                        let mut m = [[0.0; 4]; 4];
                        for col in 0..4 {
                            for row in 0..4 {
                                m[row][col] = c[col * 4 + row] as f64;
                            }
                        }
                        m
                    })
                    .collect()
            }
            None => vec![identity(); joints.len()],
        };
        let skinmats: Vec<Mat4> = joints.iter().zip(&ibm).map(|(&j, b)| mul(&world[j], b)).collect();
        let mesh = &g.arr("meshes")[mi as usize];
        let mut vertices = vec![];
        let mut sets: Vec<(String, Vec<u32>)> = vec![];
        for prim in mesh["primitives"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
            if prim["mode"].as_u64().unwrap_or(4) != 4 {
                return Err(Error::msg(format!("{object}: only triangle meshes are supported")));
            }
            let attrs = &prim["attributes"];
            let get = |k: &str| attrs[k].as_u64().map(|x| x as usize);
            let (pos, _) = g.floats(get("POSITION").ok_or_else(|| bad("mesh without positions"))?)?;
            let count = pos.len() / 3;
            let nrm = match get("NORMAL") {
                Some(a) => g.floats(a)?.0,
                None => vec![0.0; count * 3],
            };
            let uv = match get("TEXCOORD_0") {
                Some(a) => g.floats(a)?.0,
                None => vec![0.0; count * 2],
            };
            let mut infl: Vec<Vec<(usize, f32)>> = vec![vec![]; count];
            for set in 0..4 {
                let (Some(ja), Some(wa)) = (get(&format!("JOINTS_{set}")), get(&format!("WEIGHTS_{set}"))) else { break };
                let (jv, _) = g.floats(ja)?;
                let (wv, _) = g.floats(wa)?;
                for v in 0..count {
                    for k in 0..4 {
                        let w = wv[v * 4 + k];
                        if w > 0.0 {
                            infl[v].push((jv[v * 4 + k] as usize, w));
                        }
                    }
                }
            }
            let base = vertices.len() as u32;
            for v in 0..count {
                let mut ws = std::mem::take(&mut infl[v]);
                ws.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                let total: f32 = ws.iter().map(|x| x.1).sum();
                if total <= 0.0 {
                    return Err(Error::msg(format!("{object}: some vertices aren't weighted to any bone")));
                }
                let p = [pos[v * 3] as f64, pos[v * 3 + 1] as f64, pos[v * 3 + 2] as f64];
                let nn = [nrm[v * 3] as f64, nrm[v * 3 + 1] as f64, nrm[v * 3 + 2] as f64];
                let (mut bp, mut bn) = ([0.0; 3], [0.0; 3]);
                for &(j, w) in &ws {
                    let m = skinmats.get(j).ok_or_else(|| bad("joint index out of range"))?;
                    let w = (w / total) as f64;
                    let a = apply(m, p, 1.0);
                    let b = apply(m, nn, 0.0);
                    for i in 0..3 {
                        bp[i] += a[i] * w;
                        bn[i] += b[i] * w;
                    }
                }
                let len = (bn[0] * bn[0] + bn[1] * bn[1] + bn[2] * bn[2]).sqrt().max(1e-12);
                vertices.push(Vertex {
                    pos: bp,
                    normal: bn.map(|x| x / len),
                    uv: [uv[v * 2], uv[v * 2 + 1]],
                    weights: ws.into_iter().map(|(j, w)| (j, w / total)).collect(),
                });
            }
            let idx = match prim["indices"].as_u64() {
                Some(a) => g.indices(a as usize)?,
                None => (0..count as u32).collect(),
            };
            let mat = prim["material"].as_u64().and_then(|m| materials.get(m as usize)).and_then(|m| m["name"].as_str()).unwrap_or("").to_string();
            let tris: Vec<u32> = idx.iter().map(|i| i + base).collect();
            match sets.iter_mut().find(|s| s.0 == mat) {
                Some(s) => s.1.extend(tris),
                None => sets.push((mat, tris)),
            }
        }
        out.push(SkinnedMesh { object, bones, vertices, sets });
    }
    Ok(out)
}

/// Bytes of one accessor element, with its size.
fn element_bytes(g: &Glb, acc: &Value) -> Result<(Vec<Vec<u8>>, usize)> {
    let count = acc["count"].as_u64().unwrap_or(0) as usize;
    let comps = match acc["type"].as_str().unwrap_or("") {
        "SCALAR" => 1,
        "VEC2" => 2,
        "VEC3" => 3,
        "VEC4" => 4,
        "MAT4" => 16,
        _ => return Err(bad("unsupported accessor type")),
    };
    let csize = match acc["componentType"].as_u64().unwrap_or(0) {
        5120 | 5121 => 1,
        5122 | 5123 => 2,
        _ => 4,
    };
    let esize = comps * csize;
    let bv = &g.arr("bufferViews")[acc["bufferView"].as_u64().ok_or_else(|| bad("accessor without data"))? as usize];
    let base = bv["byteOffset"].as_u64().unwrap_or(0) as usize + acc["byteOffset"].as_u64().unwrap_or(0) as usize;
    let stride = bv["byteStride"].as_u64().map(|s| s as usize).unwrap_or(esize);
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let o = base + i * stride;
        out.push(g.bin.get(o..o + esize).ok_or_else(|| bad("accessor outside the buffer"))?.to_vec());
    }
    Ok((out, esize))
}

/// Blender won't keep two faces on the same three vertices, so importing
/// drops the back-to-back duplicate faces Valve uses for double-sided parts.
/// Give each such duplicate its own copies of its vertices. Returns the new
/// file and how many faces were split off.
pub fn split_duplicate_faces(bytes: &[u8]) -> Result<(Vec<u8>, usize)> {
    use std::collections::{BTreeMap, HashSet};
    let mut g = parse(bytes)?;
    let mut json = g.json.clone();
    let mut added_faces = 0;
    let mut new_bin = g.bin.clone();
    let mut new_views: Vec<Value> = vec![];
    let mut push_view = |data: &[u8], new_bin: &mut Vec<u8>, views_len: usize| -> usize {
        while !new_bin.len().is_multiple_of(4) {
            new_bin.push(0);
        }
        let off = new_bin.len();
        new_bin.extend_from_slice(data);
        new_views.push(serde_json::json!({ "buffer": 0, "byteOffset": off, "byteLength": data.len() }));
        views_len + new_views.len() - 1
    };
    let views_len = g.arr("bufferViews").len();
    let meshes = g.arr("meshes").to_vec();
    let mut accessor_updates: Vec<(usize, Value)> = vec![];
    let mut new_accessors: Vec<Value> = vec![];
    let accessors_len = g.arr("accessors").len();
    for (mi, mesh) in meshes.iter().enumerate() {
        let prims = mesh["primitives"].as_array().cloned().unwrap_or_default();
        // Group primitives that share the same vertex attributes.
        let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (pi, p) in prims.iter().enumerate() {
            if p["indices"].is_null() || p["mode"].as_u64().unwrap_or(4) != 4 {
                continue;
            }
            groups.entry(p["attributes"].to_string()).or_default().push(pi);
        }
        for pis in groups.values() {
            let attrs: Vec<(String, usize)> = prims[pis[0]]["attributes"]
                .as_object()
                .map(|o| o.iter().filter_map(|(k, v)| v.as_u64().map(|a| (k.clone(), a as usize))).collect())
                .unwrap_or_default();
            let mut data: Vec<(usize, Vec<Vec<u8>>, usize)> = vec![];
            for (_, a) in &attrs {
                let acc = &g.arr("accessors")[*a];
                let (els, size) = element_bytes(&g, acc)?;
                data.push((*a, els, size));
            }
            let mut seen: HashSet<[u32; 3]> = HashSet::new();
            let mut changed = false;
            let mut new_indices: Vec<Vec<u32>> = vec![];
            for &pi in pis {
                let idx = g.indices(prims[pi]["indices"].as_u64().unwrap() as usize)?;
                let mut out = Vec::with_capacity(idx.len());
                for t in idx.chunks_exact(3) {
                    let mut key = [t[0], t[1], t[2]];
                    key.sort_unstable();
                    if seen.insert(key) {
                        out.extend_from_slice(t);
                    } else {
                        changed = true;
                        added_faces += 1;
                        for &v in t {
                            let n = data[0].1.len() as u32;
                            for (_, els, _) in data.iter_mut() {
                                let copy = els[v as usize].clone();
                                els.push(copy);
                            }
                            out.push(n);
                        }
                    }
                }
                new_indices.push(out);
            }
            if !changed {
                continue;
            }
            // New attribute data (same accessors, more elements).
            for (a, els, _) in &data {
                let flat: Vec<u8> = els.concat();
                let view = push_view(&flat, &mut new_bin, views_len);
                let mut acc = g.arr("accessors")[*a].clone();
                acc["bufferView"] = serde_json::json!(view);
                acc["byteOffset"] = serde_json::json!(0);
                acc["count"] = serde_json::json!(els.len());
                accessor_updates.push((*a, acc));
            }
            // New u32 index accessors.
            for (k, &pi) in pis.iter().enumerate() {
                let flat: Vec<u8> = new_indices[k].iter().flat_map(|i| i.to_le_bytes()).collect();
                let view = push_view(&flat, &mut new_bin, views_len);
                let acc_idx = accessors_len + new_accessors.len();
                new_accessors.push(serde_json::json!({ "bufferView": view, "componentType": 5125, "count": new_indices[k].len(), "type": "SCALAR" }));
                json["meshes"][mi]["primitives"][pi]["indices"] = serde_json::json!(acc_idx);
            }
        }
    }
    if added_faces == 0 {
        return Ok((bytes.to_vec(), 0));
    }
    for (a, acc) in accessor_updates {
        json["accessors"][a] = acc;
    }
    if let Some(arr) = json["accessors"].as_array_mut() {
        arr.extend(new_accessors);
    }
    if let Some(arr) = json["bufferViews"].as_array_mut() {
        arr.extend(new_views);
    }
    while !new_bin.len().is_multiple_of(4) {
        new_bin.push(0);
    }
    json["buffers"][0]["byteLength"] = serde_json::json!(new_bin.len());
    g.json = json;
    g.bin = new_bin;
    Ok((write(&g), added_faces))
}

pub fn write(g: &Glb) -> Vec<u8> {
    let mut j = serde_json::to_vec(&g.json).unwrap_or_default();
    while !j.len().is_multiple_of(4) {
        j.push(b' ');
    }
    let total = 12 + 8 + j.len() + if g.bin.is_empty() { 0 } else { 8 + g.bin.len() };
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend(2u32.to_le_bytes());
    out.extend((total as u32).to_le_bytes());
    out.extend((j.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend(j);
    if !g.bin.is_empty() {
        out.extend((g.bin.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&g.bin);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One triangle and its back face on the same three vertices.
    fn double_sided() -> Vec<u8> {
        let pos: Vec<f32> = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let idx: Vec<u16> = vec![0, 1, 2, 0, 2, 1];
        let mut bin: Vec<u8> = pos.iter().flat_map(|f| f.to_le_bytes()).collect();
        bin.extend(idx.iter().flat_map(|i| i.to_le_bytes()));
        let json = serde_json::json!({
            "asset": {"version": "2.0"},
            "buffers": [{"byteLength": bin.len()}],
            "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 36}, {"buffer": 0, "byteOffset": 36, "byteLength": 12}],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0,0,0], "max": [1,1,0]},
                {"bufferView": 1, "componentType": 5123, "count": 6, "type": "SCALAR"}
            ],
            "meshes": [{"name": "m", "primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
            "nodes": [{"name": "obj", "mesh": 0}],
        });
        write(&Glb { json, bin })
    }

    #[test]
    fn splits_back_to_back_faces() {
        let (out, n) = split_duplicate_faces(&double_sided()).unwrap();
        assert_eq!(n, 1);
        let g = parse(&out).unwrap();
        let prim = &g.json["meshes"][0]["primitives"][0];
        let idx = g.indices(prim["indices"].as_u64().unwrap() as usize).unwrap();
        assert_eq!(idx, vec![0, 1, 2, 3, 4, 5]);
        let (pos, _) = g.floats(prim["attributes"]["POSITION"].as_u64().unwrap() as usize).unwrap();
        assert_eq!(pos.len(), 18);
        assert_eq!(&pos[9..12], &[0.0, 0.0, 0.0], "copy of vertex 0");
        assert_eq!(&pos[12..15], &[0.0, 1.0, 0.0], "copy of vertex 2");
        // Already split: nothing more to do.
        assert_eq!(split_duplicate_faces(&out).unwrap().1, 0);
        assert!(parse(b"nope").is_err());
    }
}
