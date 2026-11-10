//! Hero model mods: find a hero's model, take it apart with Source 2
//! Viewer's command-line tool, and rebuild it with CSDK's resource compiler.
//!
//! Extract writes, per project:
//!   original/            the decompiled ModelDoc source (`.vmdl` + mesh DMX)
//!                        and materials, laid out like the game's files
//!   original/<model>_c   the compiled original, kept as the animation source
//!   blender/<name>.glb   meshes, skeleton and materials for Blender
//!   return/              where edited meshes come back (later phases)
//!
//! Build copies the template into a CSDK addon, points its animations at a
//! copy of the original (`<name>_backup.vmdl`, the community method), puts
//! the files the compiler needs to resolve next to it, compiles, and packs
//! only what the build produced plus the backup.

use crate::kv3::{self, Kv as BKv};
use crate::kv3text::{self, Kv};
use crate::{csdk, steam, vpk, Error, IoCtx, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Source 2 Viewer's command-line tool, built from its official source at
/// commit fcf7323 by `.github/workflows/build-s2v.yml` (the 20.1 release
/// can't rebuild cloth yet). The app downloads it from this repo's release
/// and checks the SHA-256 before unpacking.
pub const S2V_VERSION: &str = "fcf7323";
pub const S2V_URL: &str =
    "https://github.com/MilkForBone2020/deadlock-mod-studio/releases/download/s2v-fcf7323/s2v-cli-win-x64.zip";
pub const S2V_SHA256: &str = "41164c06014dbcc89c4817573fc41910e97c4a722c6c70f8731bb612ffcadc24";
pub const S2V_EXE: &str = "Source2Viewer-CLI.exe";

/// A few animations to include in the Blender file, so the skeleton comes
/// with it and poses can be tested. Names a hero doesn't have are skipped.
pub const BLENDER_ANIMS: &str = "primary_stand_idle,primary_run_n,primary_crouch_idle,ui_hero_pose";

/// The animations to put in the Blender file, picked from the hero's own
/// (names differ between heroes): an idle, a run, a crouch and the menu pose.
pub fn blender_anims(doc: &kv3text::Doc) -> String {
    let mut all = vec![];
    doc.root.walk(&mut all);
    let names: Vec<String> = all
        .into_iter()
        .filter(|n| n.class().as_deref() == Some("AnimFile"))
        .filter_map(|n| n.get("name").and_then(|v| v.as_str()))
        .collect();
    let wanted: [&[&str]; 4] = [
        &["primary_stand_idle", "stand_idle", "primary_idle", "idle"],
        &["primary_run_n", "run_n", "run"],
        &["primary_crouch_idle", "crouch_idle", "crouch"],
        &["ui_hero_pose", "hero_pose"],
    ];
    let mut out: Vec<String> = vec![];
    for patterns in wanted {
        let pick = patterns
            .iter()
            .find_map(|p| names.iter().find(|n| n.as_str() == *p).or_else(|| names.iter().find(|n| n.contains(p) && !n.starts_with("self_"))));
        if let Some(n) = pick {
            if !out.contains(n) {
                out.push(n.clone());
            }
        }
    }
    if out.is_empty() {
        BLENDER_ANIMS.to_string()
    } else {
        out.join(",")
    }
}

/// How one group of physics bones (a chain's wobbly or dangling bones) is
/// tuned in the app: a preset and multipliers on its settings.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PhysicsTuning {
    /// "wobbly", "dangle", "stiff", "floppy", "skirt"; empty = as Blender suggested.
    #[serde(default)]
    pub preset: String,
    #[serde(default = "one")]
    pub stiffness: f32,
    #[serde(default = "one")]
    pub damping: f32,
    #[serde(default = "one")]
    pub gravity: f32,
    #[serde(default = "one")]
    pub wander: f32,
}

fn one() -> f32 {
    1.0
}

impl Default for PhysicsTuning {
    fn default() -> Self {
        PhysicsTuning { preset: String::new(), stiffness: 1.0, damping: 1.0, gravity: 1.0, wander: 1.0 }
    }
}

/// Cloth settings of one joint: (goal_strength, goal_damping, gravity_z,
/// stray_radius, collision_radius, friction, bend_spring). Scaled like
/// Valve's own on Celeste and Seven (damping is small: 0.001-0.03).
pub fn joint_params(preset: &str, index: u32, t: &PhysicsTuning) -> [f32; 7] {
    let i = index as f32;
    let base = match preset {
        "wobbly" => [(0.8 - 0.1 * i).max(0.6), 0.01, 0.3, 2.0 + i, 0.0, 0.0, 1.0],
        "dangle" => [[0.4, 0.3, 0.2, 0.1][(index as usize).min(3)], 0.003, 1.0, 4.0 + 2.0 * i, 1.0, 0.1, 0.05],
        "stiff" => [0.9, 0.02, 0.2, 1.0, 0.0, 0.0, 1.0],
        "floppy" => [0.05, 0.001, 1.0, 12.0 + 2.0 * i, 1.0, 0.1, 0.0],
        // Hangs and sways but keeps its drape; short reach so legs don't
        // poke through and it doesn't flare out.
        "skirt" => [[0.6, 0.45, 0.35, 0.3][(index as usize).min(3)], 0.008, 0.6, 1.5 + 0.75 * i, 1.0, 0.1, 0.3],
        _ => [0.0; 7],
    };
    [(base[0] * t.stiffness).clamp(0.0, 1.0), base[1] * t.damping, base[2] * t.gravity, base[3] * t.wander, base[4], base[5], base[6]]
}

fn set_all(obj: &mut kv3text::Kv, key: &str, value: kv3text::Kv) {
    if let kv3text::Kv::Object(pairs) = obj {
        let mut found = false;
        for (k, v) in pairs.iter_mut() {
            if k == key {
                *v = value.clone();
                found = true;
            }
        }
        if !found {
            pairs.push((key.to_string(), value));
        }
    }
}

fn kv_num(x: f32) -> kv3text::Kv {
    let mut t = format!("{:.4}", x);
    while t.ends_with('0') && !t.ends_with(".0") {
        t.pop();
    }
    kv3text::Kv::Raw(t)
}

/// Write cloth chains for the physics bones into the model source, copying
/// the layout of one of the hero's own chains. Returns how many chains were
/// written, or why none could be.
pub fn add_cloth_chains(
    doc: &mut kv3text::Doc,
    chains: &[crate::blender::PhysicsChain],
    tuning: &std::collections::BTreeMap<String, PhysicsTuning>,
    positions: &std::collections::HashMap<String, [f64; 3]>,
) -> std::result::Result<usize, String> {
    if chains.is_empty() {
        return Ok(0);
    }
    // A chain of the hero's to copy.
    let mut all = vec![];
    doc.root.walk(&mut all);
    let Some(template) = all.iter().find(|n| n.class().as_deref() == Some("ClothChain")).map(|n| (*n).clone()) else {
        // Older heroes (Haze) write cloth as single nodes joined by springs.
        if all.iter().any(|n| n.class().as_deref() == Some("ClothNode")) {
            return add_cloth_nodes(doc, chains, tuning, positions);
        }
        return Err("this hero has no cloth to copy the layout from, so the physics bones stay rigid".into());
    };
    let joints_t = template.get("chain").and_then(|c| c.get("joints")).and_then(|j| j.as_array()).cloned().unwrap_or_default();
    let root_t = joints_t.first().cloned().ok_or("the hero's cloth has no joints to copy")?;
    let child_t = joints_t.get(1).cloned().unwrap_or_else(|| root_t.clone());
    let mut made = vec![];
    for c in chains {
        let mut node = template.clone();
        node.set("name", kv3text::Kv::str(&c.name));
        node.set("root_bone", kv3text::Kv::str(&c.joints.first().map(|j| j.bone.clone()).unwrap_or_default()));
        let mut joints = vec![];
        for j in &c.joints {
            let mut o = if j.parent.is_empty() { root_t.clone() } else { child_t.clone() };
            set_all(&mut o, "joint_name", kv3text::Kv::str(&j.bone));
            if !j.parent.is_empty() {
                set_all(&mut o, "joint_parent", kv3text::Kv::str(&j.parent));
            }
            set_all(&mut o, "simulate", kv3text::Kv::Raw(if j.simulate { "true" } else { "false" }.into()));
            let key = format!("{}/{}", if c.group.is_empty() { &c.name } else { &c.group }, j.preset);
            let t = tuning.get(&key).cloned().unwrap_or_default();
            let preset = if j.simulate && !t.preset.is_empty() { t.preset.as_str() } else { j.preset.as_str() };
            let p = if j.simulate { joint_params(preset, j.index, &t) } else { [0.0; 7] };
            for (k, v) in ["goal_strength", "goal_damping", "gravity_z", "stray_radius", "collision_radius", "friction", "bend_spring"].iter().zip(p) {
                set_all(&mut o, k, kv_num(v));
            }
            if !j.simulate {
                set_all(&mut o, "gravity_z", kv_num(1.0));
            }
            joints.push(o);
        }
        if let Some(chain) = node.get_mut("chain") {
            chain.set("joints", kv3text::Kv::Array(joints));
        }
        made.push(node);
    }
    // Next to the hero's own chains.
    let n = made.len();
    let mut pending = Some(made);
    doc.root.walk_mut(&mut |k: &mut kv3text::Kv| {
        if pending.is_none() {
            return;
        }
        if let Some(children) = k.children_mut() {
            if children.iter().any(|c| c.class().as_deref() == Some("ClothChain")) {
                children.extend(pending.take().unwrap());
            }
        }
    });
    if pending.is_some() {
        return Err("couldn't find where the hero's cloth chains live".into());
    }
    Ok(n)
}

fn tuned(c: &crate::blender::PhysicsChain, j: &crate::blender::PhysicsJoint, tuning: &std::collections::BTreeMap<String, PhysicsTuning>) -> [f32; 7] {
    let key = format!("{}/{}", if c.group.is_empty() { &c.name } else { &c.group }, j.preset);
    let t = tuning.get(&key).cloned().unwrap_or_default();
    let preset = if j.simulate && !t.preset.is_empty() { t.preset.as_str() } else { j.preset.as_str() };
    if j.simulate {
        joint_params(preset, j.index, &t)
    } else {
        [0.0; 7]
    }
}

/// Physics bones for heroes whose cloth is made of `ClothNode`s (one per
/// bone, static or simulated) joined by `ClothSpring` rods, like Haze's hoses.
fn add_cloth_nodes(
    doc: &mut kv3text::Doc,
    chains: &[crate::blender::PhysicsChain],
    tuning: &std::collections::BTreeMap<String, PhysicsTuning>,
    positions: &std::collections::HashMap<String, [f64; 3]>,
) -> std::result::Result<usize, String> {
    let mut all = vec![];
    doc.root.walk(&mut all);
    let nodes: Vec<kv3text::Kv> = all.iter().filter(|n| n.class().as_deref() == Some("ClothNode")).map(|n| (*n).clone()).collect();
    let is_static = |n: &kv3text::Kv| matches!(n.get("is_static_node"), Some(kv3text::Kv::Raw(r)) if r == "true");
    let static_t = nodes.iter().find(|n| is_static(n)).or(nodes.first()).cloned().ok_or("the hero's cloth has no nodes to copy")?;
    let moving_t = nodes.iter().find(|n| !is_static(n)).cloned().unwrap_or_else(|| static_t.clone());
    let spring_t = all.iter().find(|n| n.class().as_deref() == Some("ClothSpring")).map(|n| (*n).clone()).unwrap_or_else(|| {
        kv3text::Kv::Object(vec![("_class".into(), kv3text::Kv::str("ClothSpring"))])
    });
    let mut new_nodes = vec![];
    let mut springs = vec![];
    for c in chains {
        for j in &c.joints {
            let mut o = if j.simulate { moving_t.clone() } else { static_t.clone() };
            let p = tuned(c, j, tuning);
            o.set("name", kv3text::Kv::str(&j.bone));
            o.set("cloth_node_root_bone", kv3text::Kv::str(&j.bone));
            o.set("is_static_node", kv3text::Kv::Raw(if j.simulate { "false" } else { "true" }.into()));
            o.set("goal_strength", kv_num(p[0]));
            o.set("goal_damping", kv_num(p[1]));
            o.set("gravity_z", kv_num(if j.simulate { p[2] } else { 1.0 }));
            o.set("stray_radius", kv_num(p[3]));
            o.set("has_stray_radius", kv3text::Kv::Raw(if p[3] > 0.0 { "true" } else { "false" }.into()));
            o.set("collision_radius", kv_num(p[4]));
            o.set("friction", kv_num(p[5]));
            // Not oriented by the template's own nodes; collides with the
            // hero's capsules like its other cloth; even mass.
            for k in ["node_base_x0", "node_base_y0", "node_base_x1", "node_base_y1"] {
                o.set(k, kv3text::Kv::str(""));
            }
            o.set("transform_alignment", kv3text::Kv::Raw("0".into()));
            for k in ["cloth_collision_layer0", "cloth_collision_layer1", "cloth_collision_layer2", "cloth_collision_layer3"] {
                o.set(k, kv3text::Kv::Raw("true".into()));
            }
            o.set("mass", kv_num(1.0));
            new_nodes.push(o);
            // A chain's top bone hangs from the hero's bone; the rest from
            // the bone above, with a rod that keeps their distance.
            // No rod between two pinned bones: both follow the animation, and
            // the compiler refuses it ("Cannot connect two static nodes").
            let parent_moves = c.joints.iter().any(|p| p.bone == j.parent && p.simulate);
            if !j.parent.is_empty() && (j.simulate || parent_moves) {
                let mut sp = spring_t.clone();
                sp.set("name", kv3text::Kv::str(&format!("rod_{}", j.bone)));
                sp.set("cloth_node_0", kv3text::Kv::str(&j.parent));
                sp.set("cloth_node_1", kv3text::Kv::str(&j.bone));
                sp.set("stiffness", kv_num(1.0));
                // The rod keeps the bones' rest distance (game units). Left
                // to the compiler it comes out as zero and pulls them together.
                match (positions.get(&j.parent), positions.get(&j.bone)) {
                    (Some(a), Some(b)) => {
                        let d = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt() / 0.0254;
                        sp.set("enable_advanced_parameters", kv3text::Kv::Raw("true".into()));
                        sp.set("is_length_explicit", kv3text::Kv::Raw("true".into()));
                        sp.set("min_length", kv_num(d as f32));
                        sp.set("max_length", kv_num(d as f32));
                    }
                    _ => {
                        sp.set("enable_advanced_parameters", kv3text::Kv::Raw("false".into()));
                        sp.set("is_length_explicit", kv3text::Kv::Raw("false".into()));
                    }
                }
                springs.push(sp);
            }
        }
    }
    let n = chains.len();
    // Nodes next to the hero's nodes, springs next to its springs (or the
    // nodes' folder when it has none).
    let mut pending_nodes = Some(new_nodes);
    let mut pending_springs = Some(springs);
    doc.root.walk_mut(&mut |k: &mut kv3text::Kv| {
        if let Some(children) = k.children_mut() {
            if pending_nodes.is_some() && children.iter().any(|c| c.class().as_deref() == Some("ClothNode")) {
                children.extend(pending_nodes.take().unwrap());
            }
            if pending_springs.is_some() && children.iter().any(|c| c.class().as_deref() == Some("ClothSpring")) {
                children.extend(pending_springs.take().unwrap());
            }
        }
    });
    if pending_nodes.is_some() {
        return Err("couldn't find where the hero's cloth nodes live".into());
    }
    if let Some(sp) = pending_springs {
        doc.root.walk_mut(&mut |k: &mut kv3text::Kv| {
            if let Some(children) = k.children_mut() {
                if children.iter().any(|c| c.class().as_deref() == Some("ClothNode")) && !children.iter().any(|c| c.class().as_deref() == Some("ClothSpring")) {
                    children.extend(sp.clone());
                }
            }
        });
    }
    Ok(n)
}

/// A hero and where its model lives, from `scripts/heroes.vdata`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HeroModel {
    pub code: String,
    pub model: String,
    /// Hero icons (`file://{images}/heroes/x_card.psd` ...), by field name.
    pub icons: Vec<(String, String)>,
}

pub fn hero_models(heroes_vdata: &BKv) -> Vec<HeroModel> {
    let mut out = vec![];
    for (key, h) in heroes_vdata.as_object().unwrap_or(&[]) {
        let Some(code) = key.strip_prefix("hero_") else { continue };
        let Some(model) = h.get("m_strModelName").and_then(|v| v.as_str()) else { continue };
        if model.is_empty() || !model.ends_with(".vmdl") {
            continue;
        }
        let icons = ["m_strIconHeroCard", "m_strIconHeroCardCritical", "m_strIconHeroCardGloat", "m_strIconImageSmall", "m_strMinimapImage", "m_strTopBarVertical"]
            .iter()
            .filter_map(|f| h.get(f).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(|s| (f.to_string(), s.to_string())))
            .collect();
        out.push(HeroModel { code: code.to_string(), model: model.to_string(), icons });
    }
    out
}

/// Paths a compiled resource refers to (its RERL block).
pub fn external_refs(res: &[u8]) -> Result<Vec<String>> {
    let block = match kv3::resource_block(res, b"RERL") {
        Ok(b) => b,
        Err(_) => return Ok(vec![]),
    };
    let u32at = |o: usize| -> Result<usize> {
        block.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize).ok_or_else(|| Error::msg("bad RERL block"))
    };
    let (rel, count) = (u32at(0)?, u32at(4)?);
    let mut out = vec![];
    for i in 0..count {
        let e = rel + i * 16;
        let name_at = e + 8 + u32at(e + 8)?;
        let tail = block.get(name_at..).ok_or_else(|| Error::msg("bad RERL name"))?;
        let end = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
        out.push(String::from_utf8_lossy(&tail[..end]).into_owned());
    }
    Ok(out)
}

/// What a compiled hero model contains, for the app to show and check.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelSummary {
    pub bones: Vec<String>,
    pub meshes: usize,
    pub lods: usize,
    pub body_groups: Vec<String>,
    pub materials: Vec<String>,
    pub attachments: Vec<String>,
    pub hitboxes: usize,
    pub ragdoll_bodies: usize,
    pub cloth_nodes: usize,
    pub anim_graphs: Vec<String>,
    pub triangles_lod0: u64,
}

fn num(v: &BKv) -> u64 {
    match v {
        BKv::Int(i) => *i as u64,
        BKv::UInt(u) => *u,
        BKv::Float(f) => *f as u64,
        _ => 0,
    }
}

/// Every block of one type, in order.
fn blocks<'a>(res: &'a [u8], kind: &[u8; 4]) -> Vec<&'a [u8]> {
    let u = |o: usize| res.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize).unwrap_or(0);
    let (bo, n) = (u(8), u(12));
    let mut out = vec![];
    for i in 0..n {
        let o = 8 + bo + i * 12;
        if res.get(o..o + 4) == Some(&kind[..]) {
            let st = o + 4 + u(o + 4);
            if let Some(b) = res.get(st..st + u(o + 8)) {
                out.push(b);
            }
        }
    }
    out
}

/// Whether a decompiled model source carries cloth (`Cloth...` nodes).
pub fn has_cloth_nodes(doc: &kv3text::Doc) -> bool {
    let mut all = vec![];
    doc.root.walk(&mut all);
    all.iter().any(|n| n.class().is_some_and(|c| c.starts_with("Cloth")))
}

pub fn summarise(res: &[u8]) -> Result<ModelSummary> {
    let data = kv3::parse_resource(res)?;
    let mut s = ModelSummary::default();
    if let Some(sk) = data.get("m_modelSkeleton") {
        s.bones = sk.get("m_boneName").and_then(|v| v.as_array()).unwrap_or(&[]).iter().filter_map(|b| b.as_str().map(String::from)).collect();
    }
    s.lods = data.get("m_lodGroupSwitchDistances").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    // "DEFAULT_@0_#&BASE" -> "DEFAULT: BASE"
    s.body_groups = data
        .get("m_meshGroups")
        .and_then(|v| v.as_array())
        .unwrap_or(&[])
        .iter()
        .filter_map(|g| g.as_str())
        .map(|g| match g.split_once("_@") {
            Some((group, rest)) => format!("{group}: {}", rest.split("#&").nth(1).unwrap_or(rest)),
            None => g.to_string(),
        })
        .collect();
    s.anim_graphs = data.get("m_animGraph2Refs").and_then(|v| v.as_array()).unwrap_or(&[]).iter().filter_map(|g| g.get("m_hGraph").and_then(|v| v.as_str()).map(String::from)).collect();
    let lod_masks: Vec<u64> = data.get("m_refLODGroupMasks").and_then(|v| v.as_array()).unwrap_or(&[]).iter().map(num).collect();
    s.materials = external_refs(res)?.into_iter().filter(|r| r.ends_with(".vmat")).collect();

    for (i, block) in blocks(res, b"MDAT").into_iter().enumerate() {
        let Ok(mesh) = kv3::parse(block) else { continue };
        s.meshes += 1;
        if s.attachments.is_empty() {
            s.attachments = mesh
                .get("m_attachments")
                .and_then(|v| v.as_array())
                .unwrap_or(&[])
                .iter()
                .filter_map(|a| a.get("value").unwrap_or(a).get("m_name").and_then(|n| n.as_str()).map(String::from))
                .collect();
        }
        if s.hitboxes == 0 {
            for hs in mesh.get("m_hitboxsets").and_then(|v| v.as_array()).unwrap_or(&[]) {
                s.hitboxes += hs.get("value").unwrap_or(hs).get("m_HitBoxes").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
            }
        }
        if lod_masks.get(i).copied().unwrap_or(1) & 1 == 1 {
            for so in mesh.get("m_sceneObjects").and_then(|v| v.as_array()).unwrap_or(&[]) {
                for dc in so.get("m_drawCalls").and_then(|v| v.as_array()).unwrap_or(&[]) {
                    s.triangles_lod0 += dc.get("m_nIndexCount").map(num).unwrap_or(0) / 3;
                }
            }
        }
    }
    if let Some(phys) = blocks(res, b"PHYS").first().and_then(|b| kv3::parse(b).ok()) {
        s.ragdoll_bodies = phys.get("m_parts").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        s.cloth_nodes = phys.get("m_pFeModel").and_then(|f| f.get("m_CtrlName")).and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    }
    Ok(s)
}

/// Runs a tool with arguments. Returns (success, output).
pub type ToolRunner<'a> = dyn Fn(&Path, &[String]) -> Result<(bool, String)> + 'a;

/// The real tool runner, hidden window on Windows.
pub fn run_tool(exe: &Path, args: &[String]) -> Result<(bool, String)> {
    let mut cmd = std::process::Command::new(exe);
    if let Some(d) = exe.parent() {
        cmd.current_dir(d);
    }
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().at(exe)?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    Ok((out.status.success(), text))
}

/// `models/a/b.vmdl` -> ("models/a", "b")
fn split_model(model: &str) -> Result<(String, String)> {
    let m = model.replace('\\', "/").to_lowercase();
    let stem = m.strip_suffix(".vmdl").ok_or_else(|| Error::msg(format!("{model} isn't a model (.vmdl)")))?;
    if !stem.starts_with("models/") || stem.contains("..") {
        return Err(Error::msg(format!("{model} isn't under models/")));
    }
    let (dir, name) = stem.rsplit_once('/').unwrap();
    Ok((dir.to_string(), name.to_string()))
}

pub fn backup_path(model: &str) -> Result<String> {
    let (dir, name) = split_model(model)?;
    Ok(format!("{dir}/{name}_backup.vmdl"))
}

fn native(root: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(root.to_path_buf(), |p, s| p.join(s))
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExtractReport {
    pub summary: ModelSummary,
    pub blender_file: String,
    pub template: String,
    pub mesh_files: Vec<String>,
    /// The original has cloth but the decompiled source doesn't.
    pub cloth_missing: bool,
    pub log: String,
}

/// Take a hero's model apart into `project_dir`.
pub fn extract(
    s2v: &Path,
    game_dir: &Path,
    model: &str,
    project_dir: &Path,
    runner: &ToolRunner,
    progress: &dyn Fn(&str),
) -> Result<ExtractReport> {
    let (dir, name) = split_model(model)?;
    let pak = steam::citadel_dir(game_dir).join("pak01_dir.vpk");
    progress("Reading the model from the game");
    let index = vpk::read_directory(&pak)?;
    let entry = index
        .find(&format!("{dir}/{name}.vmdl_c"))
        .ok_or_else(|| Error::msg(format!("{model} isn't in the game's files (was it renamed in an update?)")))?;
    let compiled = index.extract(entry)?;
    let summary = summarise(&compiled)?;

    let original = project_dir.join("original");
    if original.exists() {
        std::fs::remove_dir_all(&original).at(&original)?;
    }
    std::fs::create_dir_all(&original).at(&original)?;
    for d in ["blender", "return"] {
        std::fs::create_dir_all(project_dir.join(d)).at(project_dir)?;
    }
    let kept = native(&original, &format!("{dir}/{name}.vmdl_c"));
    std::fs::create_dir_all(kept.parent().unwrap()).at(&kept)?;
    std::fs::write(&kept, &compiled).at(&kept)?;

    // Decompile the model and its materials. Two or more filters make the
    // tool write a folder tree instead of a single file.
    progress("Decompiling the model (about half a minute)");
    let mut filters = vec![format!("{dir}/{name}.vmdl_c")];
    filters.extend(summary.materials.iter().map(|m| format!("{m}_c")));
    if filters.len() < 2 {
        filters.push(format!("{dir}/{name}.vnmskel_c"));
    }
    let args = vec![
        "-i".into(),
        pak.display().to_string(),
        "-f".into(),
        filters.join(","),
        "-d".into(),
        "-o".into(),
        original.display().to_string(),
    ];
    let (_, out1) = runner(s2v, &args)?;
    let template = native(&original, &format!("{dir}/{name}.vmdl"));
    if !template.is_file() {
        return Err(Error::msg(format!("Source 2 Viewer didn't write the model source.\n\n{}", tail(&out1))));
    }

    let anims = kv3text::parse(&std::fs::read_to_string(&template).at(&template)?).map(|d| blender_anims(&d)).unwrap_or_else(|_| BLENDER_ANIMS.to_string());
    progress("Exporting for Blender");
    let glb = project_dir.join("blender").join(format!("{name}.glb"));
    let args = vec![
        "-i".into(),
        pak.display().to_string(),
        "-f".into(),
        format!("{dir}/{name}.vmdl_c"),
        "-o".into(),
        glb.display().to_string(),
        "-d".into(),
        "--gltf_export_format".into(),
        "glb".into(),
        "--gltf_export_materials".into(),
        "--gltf_export_animations".into(),
        "--gltf_animation_list".into(),
        anims,
    ];
    let (_, out2) = runner(s2v, &args)?;
    if !glb.is_file() {
        return Err(Error::msg(format!("Source 2 Viewer didn't write the Blender file.\n\n{}", tail(&out2))));
    }
    // Keep Valve's double-sided faces through Blender's import.
    progress("Preparing the Blender file");
    let raw = std::fs::read(&glb).at(&glb)?;
    if crate::glb::parse(&raw).is_ok() {
        let (fixed, _) = crate::glb::split_duplicate_faces(&raw)?;
        std::fs::write(&glb, fixed).at(&glb)?;
    }

    // Animations come from the backup at build time, so the decompiled
    // animation files aren't needed: drop them to save ~200 MB.
    progress("Tidying up");
    let text = std::fs::read_to_string(&template).at(&template)?;
    let doc = kv3text::parse(&text)?;
    let mesh_files = mesh_files(&doc);
    let keep: BTreeSet<String> = mesh_files.iter().map(|f| f.to_lowercase()).collect();
    let model_folder = native(&original, &dir);
    for e in std::fs::read_dir(&model_folder).at(&model_folder)?.flatten() {
        let p = e.path();
        let fname = e.file_name().to_string_lossy().to_lowercase();
        let rel = format!("{dir}/{fname}");
        if p.is_dir() && (fname == "clips" || fname == "dmx") {
            let _ = std::fs::remove_dir_all(&p);
        } else if fname.ends_with(".dmx") && !keep.contains(&rel) {
            let _ = std::fs::remove_file(&p);
        }
    }
    std::fs::write(project_dir.join("summary.json"), serde_json::to_vec_pretty(&summary)?).at(project_dir)?;

    let cloth_missing = summary.cloth_nodes > 0 && !has_cloth_nodes(&doc);
    Ok(ExtractReport {
        cloth_missing,
        summary,
        blender_file: glb.display().to_string(),
        template: template.display().to_string(),
        mesh_files,
        log: format!("{}\n{}", tail(&out1), tail(&out2)),
    })
}

fn tail(s: &str) -> String {
    let lines: Vec<&str> = s.lines().filter(|l| !l.contains("Dump written")).collect();
    lines[lines.len().saturating_sub(40)..].join("\n")
}

fn root_children(doc: &Kv) -> Option<&Vec<Kv>> {
    doc.get("rootNode").and_then(|r| r.children())
}

/// Mesh and physics files the model source uses (outside its animations).
pub fn mesh_files(doc: &kv3text::Doc) -> Vec<String> {
    let mut out = BTreeSet::new();
    for list in root_children(&doc.root).map(|v| v.as_slice()).unwrap_or(&[]) {
        if list.class().as_deref() == Some("AnimationList") {
            continue;
        }
        let mut all = vec![];
        list.walk(&mut all);
        for n in all {
            if let Some(f) = n.get("filename").and_then(|v| v.as_str()) {
                let f = f.replace('\\', "/");
                let l = f.to_lowercase();
                if [".dmx", ".fbx", ".obj", ".smd"].iter().any(|e| l.ends_with(e)) {
                    out.insert(f);
                }
            }
        }
    }
    out.into_iter().collect()
}

/// Point the model's animations at the backup copy of the original
/// instead of recompiling every animation.
pub fn use_backup_animations(doc: &mut kv3text::Doc, model: &str) -> Result<()> {
    let backup = backup_path(model)?;
    let include = Kv::Object(vec![("_class".into(), Kv::str("AnimIncludeModel")), ("model".into(), Kv::str(&backup))]);
    let root = doc.root.get_mut("rootNode").ok_or_else(|| Error::msg("model source has no rootNode"))?;
    let kids = root.children_mut().ok_or_else(|| Error::msg("model source has no nodes"))?;
    match kids.iter_mut().find(|k| k.class().as_deref() == Some("AnimationList")) {
        Some(list) => list.set("children", Kv::Array(vec![include])),
        None => kids.push(Kv::Object(vec![
            ("_class".into(), Kv::str("AnimationList")),
            ("children".into(), Kv::Array(vec![include])),
        ])),
    }
    Ok(())
}

/// One mesh part of a hero (`outer_costume`, `gun`...), its lower detail
/// versions, and the body-group choices that show it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    pub name: String,
    pub lods: Vec<String>,
    /// e.g. `["DEFAULT: BASE", "DEFAULT: NO_GUN"]`
    pub shown_in: Vec<String>,
}

/// Parts and body groups from a decompiled model source.
pub fn parts(doc: &kv3text::Doc) -> Vec<Part> {
    let kids = root_children(&doc.root).cloned().unwrap_or_default();
    let mut names: Vec<String> = vec![];
    for list in kids.iter().filter(|k| k.class().as_deref() == Some("RenderMeshList")) {
        let mut all = vec![];
        list.walk(&mut all);
        for n in all {
            if n.class().as_deref() == Some("RenderMeshFile") {
                if let Some(name) = n.get("name").and_then(|v| v.as_str()) {
                    names.push(name);
                }
            }
        }
    }
    let is_lod = |n: &str| n.rsplit_once("_lod").is_some_and(|(_, d)| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()));
    let mut out: Vec<Part> = names
        .iter()
        .filter(|n| !is_lod(n))
        .map(|n| Part {
            name: n.clone(),
            lods: names.iter().filter(|m| is_lod(m) && m.rsplit_once("_lod").map(|(b, _)| b) == Some(n.as_str())).cloned().collect(),
            shown_in: vec![],
        })
        .collect();
    for list in kids.iter().filter(|k| k.class().as_deref() == Some("BodyGroupList")) {
        for group in list.children().map(|v| v.as_slice()).unwrap_or(&[]) {
            let gname = group.get("name").and_then(|v| v.as_str()).unwrap_or_default();
            for choice in group.children().map(|v| v.as_slice()).unwrap_or(&[]) {
                let cname = choice.get("name").and_then(|v| v.as_str()).unwrap_or_default();
                for m in choice.get("meshes").and_then(|v| v.as_array()).map(|v| v.as_slice()).unwrap_or(&[]) {
                    if let Some(p) = m.as_str().and_then(|m| out.iter_mut().find(|p| p.name == m)) {
                        p.shown_in.push(format!("{gname}: {cname}"));
                    }
                }
            }
        }
    }
    out
}

/// What the Blender add-on reads to open a project (`dms_project.json`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BlenderProject {
    pub format: u32,
    pub id: String,
    pub name: String,
    pub hero: String,
    pub hero_name: String,
    pub model: String,
    /// The model's file name without folders or extension, e.g. `gigawatt_prisoner`.
    pub model_name: String,
    pub glb: String,
    pub return_dir: String,
    pub work_blend: String,
    pub parts: Vec<Part>,
    pub body_groups: Vec<String>,
    pub attachments: Vec<String>,
    pub bones: Vec<String>,
}

pub const PROJECT_FILE: &str = "dms_project.json";

/// Write `dms_project.json` for the Blender add-on from an extracted project.
pub fn write_blender_project(project_dir: &Path, id: &str, name: &str, hero: &str, hero_name: &str, model: &str) -> Result<PathBuf> {
    let (dir, mname) = split_model(model)?;
    let template = native(&project_dir.join("original"), &format!("{dir}/{mname}.vmdl"));
    let doc = kv3text::parse(&std::fs::read_to_string(&template).at(&template)?)?;
    let summary: ModelSummary = serde_json::from_slice(&std::fs::read(project_dir.join("summary.json")).at(project_dir)?)?;
    let bp = BlenderProject {
        format: 1,
        id: id.to_string(),
        name: name.to_string(),
        hero: hero.to_string(),
        hero_name: hero_name.to_string(),
        model: model.to_string(),
        model_name: mname.clone(),
        glb: project_dir.join("blender").join(format!("{mname}.glb")).display().to_string(),
        return_dir: project_dir.join("return").display().to_string(),
        work_blend: project_dir.join("blender").join(format!("{mname}_work.blend")).display().to_string(),
        parts: parts(&doc),
        body_groups: summary.body_groups.clone(),
        attachments: summary.attachments.clone(),
        bones: summary.bones.clone(),
    };
    let out = project_dir.join(PROJECT_FILE);
    std::fs::write(&out, serde_json::to_vec_pretty(&bp)?).at(&out)?;
    Ok(out)
}

/// What happened to one part in a build with an edited model.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PartResult {
    pub part: String,
    /// "replaced", "kept" (unchanged, original files used) or "removed".
    pub status: String,
    pub vertices: usize,
    pub triangles: usize,
}

/// Remove mesh names from the model source: their RenderMeshFiles, body
/// group choices and LOD groups.
pub fn remove_meshes(doc: &mut kv3text::Doc, names: &[String]) {
    let gone = |n: &str| names.iter().any(|x| x == n);
    doc.root.walk_mut(&mut |obj| {
        if let Some(kids) = obj.children_mut() {
            kids.retain(|k| !(k.class().as_deref() == Some("RenderMeshFile") && k.get("name").and_then(|n| n.as_str()).is_some_and(|n| gone(&n))));
        }
        if let Some(Kv::Array(m)) = obj.get_mut("meshes") {
            m.retain(|x| !x.as_str().is_some_and(|n| gone(&n)));
        }
        if let Some(Kv::Array(m)) = obj.get_mut("mesh_references") {
            m.retain(|x| !x.get("mesh_name").and_then(|n| n.as_str()).is_some_and(|n| gone(&n)));
        }
    });
}

/// Point a part (and its lower detail levels) at a new mesh file.
pub fn set_part_file(doc: &mut kv3text::Doc, part: &Part, file: &str) {
    let names: Vec<String> = std::iter::once(part.name.clone()).chain(part.lods.iter().cloned()).collect();
    doc.root.walk_mut(&mut |obj| {
        if obj.class().as_deref() == Some("RenderMeshFile") && obj.get("name").and_then(|n| n.as_str()).is_some_and(|n| names.contains(&n)) {
            obj.set("filename", Kv::str(file));
        }
    });
}

fn part_file(doc: &kv3text::Doc, part: &str) -> Option<String> {
    let mut all = vec![];
    doc.root.walk(&mut all);
    all.into_iter()
        .find(|n| n.class().as_deref() == Some("RenderMeshFile") && n.get("name").and_then(|v| v.as_str()).as_deref() == Some(part))
        .and_then(|n| n.get("filename").and_then(|v| v.as_str()))
        .map(|f| f.replace('\\', "/"))
}

/// Swap in the model sent back from Blender (`returned` is the folder with
/// manifest.json and the glb). New mesh files go into `content_root`.
pub fn apply_returned(
    doc: &mut kv3text::Doc,
    project_dir: &Path,
    model: &str,
    returned: &Path,
    content_root: &Path,
    new_materials: &std::collections::HashMap<String, String>,
) -> Result<Vec<PartResult>> {
    let (dir, name) = split_model(model)?;
    let manifest: crate::blender::Returned = serde_json::from_slice(&std::fs::read(returned.join("manifest.json")).at(returned)?)?;
    let glb_path = returned.join(&manifest.glb);
    let g = crate::glb::parse(&std::fs::read(&glb_path).at(&glb_path)?)?;
    let meshes = crate::glb::skinned_meshes(&g)?;
    let glb_joints = crate::glb::joints(&g);
    // Cloth chain bones, weighted or not, must be in the skeleton.
    let physics_bones: Vec<String> = manifest.physics.iter().flat_map(|c| c.joints.iter().map(|j| j.bone.clone())).collect();
    let summary: ModelSummary = serde_json::from_slice(&std::fs::read(project_dir.join("summary.json")).at(project_dir)?)?;
    let mut materials: std::collections::HashMap<String, String> = summary
        .materials
        .iter()
        .map(|p| (p.rsplit('/').next().unwrap_or(p).trim_end_matches(".vmat").to_string(), p.clone()))
        .collect();
    materials.extend(new_materials.iter().map(|(k, v)| (k.clone(), v.clone())));
    let part_of: std::collections::HashMap<&str, &str> = manifest.objects.iter().map(|o| (o.object.as_str(), o.part.as_str())).collect();
    let parts = parts(doc);
    for m in &meshes {
        match part_of.get(m.object.as_str()) {
            Some(p) if parts.iter().any(|x| x.name == *p) => {}
            _ => return Err(Error::msg(format!("{} isn't assigned to one of the hero's parts", m.object))),
        }
    }
    let original = project_dir.join("original");
    let mut results = vec![];
    for part in &parts {
        let mine: Vec<&crate::glb::SkinnedMesh> = meshes.iter().filter(|m| part_of.get(m.object.as_str()) == Some(&part.name.as_str())).collect();
        if mine.is_empty() {
            let names: Vec<String> = std::iter::once(part.name.clone()).chain(part.lods.iter().cloned()).collect();
            remove_meshes(doc, &names);
            results.push(PartResult { part: part.name.clone(), status: "removed".into(), vertices: 0, triangles: 0 });
            continue;
        }
        let file = part_file(doc, &part.name).ok_or_else(|| Error::msg(format!("The model source has no mesh file for {}", part.name)))?;
        let orig_path = native(&original, &file);
        let orig = std::fs::read(&orig_path).at(&orig_path)?;
        match crate::meshswap::swap_with_bones(&orig, &mine, &materials, 1.0, &glb_joints, &physics_bones).map_err(|e| Error::msg(format!("{}: {e}", part.name)))? {
            crate::meshswap::Swap::Unchanged => {
                results.push(PartResult { part: part.name.clone(), status: "kept".into(), vertices: 0, triangles: 0 });
            }
            crate::meshswap::Swap::Replaced { dmx, vertices, triangles } => {
                let rel = format!("{dir}/{name}_{}_dms.dmx", part.name);
                let out = native(content_root, &rel);
                std::fs::create_dir_all(out.parent().unwrap()).at(&out)?;
                std::fs::write(&out, dmx).at(&out)?;
                set_part_file(doc, part, &rel);
                results.push(PartResult { part: part.name.clone(), status: "replaced".into(), vertices, triangles });
            }
        }
    }
    if results.iter().all(|r| r.status == "removed") {
        return Err(Error::msg("The model from Blender has no parts"));
    }
    Ok(results)
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ModelBuildReport {
    /// Cloth chains written for physics bones.
    #[serde(default)]
    pub physics_chains: usize,
    /// Parts swapped from the Blender model (empty for an unchanged rebuild).
    pub parts: Vec<PartResult>,
    /// Materials rebuilt with textures changed in Blender.
    pub materials: Vec<crate::material::MaterialResult>,
    pub vpk: String,
    pub vpk_size: u64,
    pub files: Vec<String>,
    pub warnings: Vec<String>,
    pub log: String,
}

/// Build a model mod from an extracted project. `slug` names the CSDK addon.
#[allow(clippy::too_many_arguments)]
pub fn build(
    csdk_dir: &Path,
    game_dir: &Path,
    model: &str,
    project_dir: &Path,
    slug: &str,
    returned: Option<&Path>,
    physics: &std::collections::BTreeMap<String, PhysicsTuning>,
    out_vpk: &Path,
    runner: &csdk::Runner,
    progress: &dyn Fn(&str),
) -> Result<ModelBuildReport> {
    if !csdk::is_csdk_dir(csdk_dir) {
        return Err(Error::msg(format!(
            "CSDK wasn't found at {} (game\\bin\\win64\\resourcecompiler.exe is missing)",
            csdk_dir.display()
        )));
    }
    let (dir, name) = split_model(model)?;
    let original = project_dir.join("original");
    let template = native(&original, &format!("{dir}/{name}.vmdl"));
    let compiled_original = native(&original, &format!("{dir}/{name}.vmdl_c"));
    if !template.is_file() || !compiled_original.is_file() {
        return Err(Error::msg("This hero hasn't been extracted yet"));
    }

    let addon = csdk::addon_name(slug);
    let content_root = csdk_dir.join("content").join("citadel_addons").join(&addon);
    let game_root = csdk_dir.join("game").join("citadel_addons").join(&addon);
    for d in [&content_root, &game_root] {
        if d.exists() {
            std::fs::remove_dir_all(d).at(d)?;
        }
    }

    progress("Preparing the model source");
    let mut doc = kv3text::parse(&std::fs::read_to_string(&template).at(&template)?)?;
    use_backup_animations(&mut doc, model)?;
    let mut part_results = vec![];
    let mut material_results = vec![];
    let mut physics_chains = 0;
    let mut physics_note: Option<String> = None;
    if let Some(r) = returned {
        let manifest: crate::blender::Returned = serde_json::from_slice(&std::fs::read(r.join("manifest.json")).at(r)?)?;
        progress("Preparing your materials and textures");
        let summary: ModelSummary = serde_json::from_slice(&std::fs::read(project_dir.join("summary.json")).at(project_dir)?)?;
        let applied = crate::material::apply(r, &manifest.material_files, &summary.materials, model, &original, &content_root)?;
        progress("Converting your model from Blender");
        part_results = apply_returned(&mut doc, project_dir, model, r, &content_root, &applied.new_paths)?;
        material_results = applied.results;
        if !manifest.physics.is_empty() {
            progress("Adding physics");
            // Bone rest positions (glTF metres), for rod lengths.
            let positions: std::collections::HashMap<String, [f64; 3]> = std::fs::read(r.join(&manifest.glb))
                .ok()
                .and_then(|b| crate::glb::parse(&b).ok())
                .map(|g| crate::glb::joints(&g).into_iter().map(|(n, (_, m))| (n, [m[0][3], m[1][3], m[2][3]])).collect())
                .unwrap_or_default();
            match add_cloth_chains(&mut doc, &manifest.physics, physics, &positions) {
                Ok(n) => physics_chains = n,
                Err(e) => physics_note = Some(format!("Physics skipped: {e}")),
            }
        }
    }
    let overridden: BTreeSet<String> = material_results.iter().map(|m| format!("{}_c", m.path.to_lowercase())).collect();
    let src = native(&content_root, &format!("{dir}/{name}.vmdl"));
    std::fs::create_dir_all(src.parent().unwrap()).at(&src)?;
    std::fs::write(&src, kv3text::to_string(&doc)).at(&src)?;
    for f in mesh_files(&doc) {
        if native(&content_root, &f).is_file() {
            continue; // written by apply_returned
        }
        let from = native(&original, &f);
        if !from.is_file() {
            return Err(Error::msg(format!("A mesh file the model needs is missing: {f}")));
        }
        let to = native(&content_root, &f);
        std::fs::create_dir_all(to.parent().unwrap()).at(&to)?;
        std::fs::copy(&from, &to).at(&to)?;
    }

    // What the compiler needs to resolve, from the game: the backup of the
    // original (animations) and everything the original refers to. These
    // are not packed (apart from the backup).
    progress("Copying what the compiler needs");
    let compiled = std::fs::read(&compiled_original).at(&compiled_original)?;
    let backup = format!("{}_c", backup_path(model)?);
    let mut placed: BTreeSet<String> = BTreeSet::new();
    let put = |rel: &str, bytes: &[u8]| -> Result<()> {
        let p = native(&game_root, rel);
        std::fs::create_dir_all(p.parent().unwrap()).at(&p)?;
        std::fs::write(&p, bytes).at(&p)
    };
    put(&backup, &compiled)?;
    let pak = steam::citadel_dir(game_dir).join("pak01_dir.vpk");
    let index = vpk::read_directory(&pak)?;
    for r in external_refs(&compiled)? {
        let rel = format!("{}_c", r.to_lowercase());
        if overridden.contains(&rel) {
            continue; // rebuilt from my textures below
        }
        if let Some(e) = index.find(&rel) {
            put(&rel, &index.extract(e)?)?;
            placed.insert(rel);
        }
    }

    let mut mat_log = String::new();
    for m in &material_results {
        progress(&format!("Compiling material {}", m.material));
        let src = native(&content_root, &m.path);
        let (ok, log) = runner(csdk_dir, &src)?;
        mat_log.push_str(&format!("== {}\n{}\n", m.path, tail(&log)));
        if !ok || !native(&game_root, &format!("{}_c", m.path)).is_file() {
            return Err(Error::msg(format!("CSDK couldn't compile the material {}.\n\n{}", m.material, tail(&log))));
        }
    }

    progress("Compiling the model (this can take a few minutes)");
    let (ok, log) = runner(csdk_dir, &src)?;
    let out_rel = format!("{dir}/{name}.vmdl_c");
    let out_file = native(&game_root, &out_rel);
    if !ok || !out_file.is_file() {
        return Err(Error::msg(format!("CSDK couldn't compile the model. Is Steam running?\n\n{}", tail(&log))));
    }

    progress("Packing VPK");
    let mut packed: Vec<(String, Vec<u8>)> = vec![];
    for (rel, path) in files_under(&game_root)? {
        if placed.contains(&rel) {
            continue;
        }
        packed.push((rel, std::fs::read(&path).at(&path)?));
    }
    packed.sort_by(|a, b| a.0.cmp(&b.0));
    if let Some(p) = out_vpk.parent() {
        std::fs::create_dir_all(p).at(p)?;
    }
    vpk::write_vpk(out_vpk, &packed)?;
    let log = format!("{mat_log}{log}");
    let warnings = log
        .lines()
        .filter(|l| {
            let u = l.to_uppercase();
            u.contains("WARNING") || u.contains("ERROR")
        })
        .map(|l| l.trim().to_string())
        .take(50)
        .collect::<Vec<String>>();
    let mut warnings = warnings;
    if let Some(n) = physics_note {
        warnings.insert(0, n);
    }
    Ok(ModelBuildReport {
        physics_chains,
        parts: part_results,
        materials: material_results,
        vpk: out_vpk.display().to_string(),
        vpk_size: std::fs::metadata(out_vpk).at(out_vpk)?.len(),
        files: packed.into_iter().map(|(p, _)| p).collect(),
        warnings,
        log: tail(&log),
    })
}

/// Every file under `root`, as (lowercase game path, real path).
fn files_under(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut out = vec![];
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).at(&d)?.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(root) {
                let rel = rel.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect::<Vec<_>>().join("/");
                out.push((rel, p));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VMDL: &str = r#"<!-- kv3 encoding:text:version{e21c7f3c-8a33-41c5-9977-a76d3a32aa0d} format:modeldoc28:version{fb63b6ca-f435-4aa0-a2c7-c66ddc651dca} -->
{
	rootNode =
	{
		_class = "RootNode"
		children =
		[
			{
				_class = "RenderMeshList"
				children =
				[
					{
						_class = "RenderMeshFile"
						name = "body"
						filename = "models/heroes_staging/hero/hero_body.dmx"
					},
				]
			},
			{
				_class = "PhysicsShapeList"
				children =
				[
					{
						_class = "PhysicsHullFile"
						filename = "models/heroes_staging/hero/hero_hull.dmx"
					},
				]
			},
			{
				_class = "AnimationList"
				default_root_bone_name = ""
				children =
				[
					{
						_class = "AnimFile"
						name = "idle"
						source_filename = "models/heroes_staging/hero/hero_idle.dmx"
					},
				]
			},
			{
				_class = "NmSkeletonList"
				children =
				[
					{
						_class = "NmSkeletonReference"
						filename = "models/heroes_staging/hero/hero.vnmskel"
					},
				]
			},
		]
	}
}
"#;

    #[test]
    fn finds_meshes_and_swaps_animations() {
        let mut doc = kv3text::parse(VMDL).unwrap();
        assert_eq!(mesh_files(&doc), vec!["models/heroes_staging/hero/hero_body.dmx", "models/heroes_staging/hero/hero_hull.dmx"]);
        use_backup_animations(&mut doc, "models/heroes_staging/hero/hero.vmdl").unwrap();
        let text = kv3text::to_string(&doc);
        assert!(text.contains("models/heroes_staging/hero/hero_backup.vmdl"));
        assert!(!text.contains("hero_idle.dmx"));
        assert!(text.contains("default_root_bone_name"));
    }

    #[test]
    fn parts_and_body_groups() {
        let src = r#"{ rootNode = { _class = "RootNode" children = [
            { _class = "RenderMeshList" children = [
                { _class = "RenderMeshFile" name = "body" filename = "a.dmx" },
                { _class = "RenderMeshFile" name = "gun" filename = "b.dmx" },
                { _class = "RenderMeshFile" name = "body_lod1" filename = "c.dmx" },
                { _class = "Folder" children = [ { _class = "RenderMeshFile" name = "gun_lod1" filename = "d.dmx" } ] },
            ] },
            { _class = "BodyGroupList" children = [ { _class = "BodyGroup" name = "DEFAULT" children = [
                { _class = "BodyGroupChoice" name = "BASE" meshes = [ "body", "gun" ] },
                { _class = "BodyGroupChoice" name = "NO_GUN" meshes = [ "body" ] },
            ] } ] },
        ] } }"#;
        let p = parts(&kv3text::parse(src).unwrap());
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].name, "body");
        assert_eq!(p[0].lods, vec!["body_lod1"]);
        assert_eq!(p[0].shown_in, vec!["DEFAULT: BASE", "DEFAULT: NO_GUN"]);
        assert_eq!(p[1].lods, vec!["gun_lod1"]);
        assert_eq!(p[1].shown_in, vec!["DEFAULT: BASE"]);
    }

    #[test]
    fn swaps_and_removes_part_files() {
        let src = r#"{ rootNode = { _class = "RootNode" children = [
            { _class = "RenderMeshList" children = [
                { _class = "RenderMeshFile" name = "body" filename = "models/x/body.dmx" },
                { _class = "RenderMeshFile" name = "body_lod1" filename = "models/x/body_lod1.dmx" },
                { _class = "RenderMeshFile" name = "gun" filename = "models/x/gun.dmx" },
            ] },
            { _class = "BodyGroupList" children = [ { _class = "BodyGroup" name = "DEFAULT" children = [
                { _class = "BodyGroupChoice" name = "BASE" meshes = [ "body", "gun" ] },
            ] } ] },
            { _class = "LODGroupList" children = [ { _class = "LODGroup" mesh_references = [ { mesh_name = "body" }, { mesh_name = "gun" } ] } ] },
        ] } }"#;
        let mut doc = kv3text::parse(src).unwrap();
        let ps = parts(&doc);
        set_part_file(&mut doc, &ps[0], "models/x/x_body_dms.dmx");
        remove_meshes(&mut doc, &["gun".to_string()]);
        assert_eq!(mesh_files(&doc), vec!["models/x/x_body_dms.dmx"]);
        let text = kv3text::to_string(&doc);
        assert!(!text.contains("\"gun\""), "{text}");
        assert_eq!(part_file(&doc, "body").as_deref(), Some("models/x/x_body_dms.dmx"));
        assert_eq!(crate::meshswap::base_material_name("gigawatt_prisoner.001"), "gigawatt_prisoner");
        assert_eq!(crate::meshswap::base_material_name("a.b"), "a.b");
    }

    #[test]
    fn model_paths() {
        assert_eq!(backup_path("models/heroes_staging/gigawatt_prisoner/gigawatt_prisoner.vmdl").unwrap(), "models/heroes_staging/gigawatt_prisoner/gigawatt_prisoner_backup.vmdl");
        assert!(split_model("models/../x.vmdl").is_err());
        assert!(split_model("sounds/x.vmdl").is_err());
        assert!(split_model("models/x.vmat").is_err());
    }

    /// Fake tools: the "decompiler" writes the sample source, a mesh and a
    /// glb; the "compiler" writes a vmdl_c where the real one would.
    #[test]
    fn extract_and_build_end_to_end() {
        let t = tempfile::tempdir().unwrap();
        let model = "models/heroes_staging/hero/hero.vmdl";
        // Fake game: the compiled model (with a material and skeleton ref) + deps
        let game = t.path().join("game");
        std::fs::create_dir_all(steam::citadel_dir(&game)).unwrap();
        let compiled = fake_compiled(&["models/heroes_staging/hero/hero.vnmskel", "materials/hero.vmat"]);
        let pak = vpk::build_vpk(&[
            ("models/heroes_staging/hero/hero.vmdl_c".into(), compiled.clone()),
            ("models/heroes_staging/hero/hero.vnmskel_c".into(), b"skel".to_vec()),
            ("materials/hero.vmat_c".into(), b"mat".to_vec()),
        ])
        .unwrap();
        std::fs::write(steam::citadel_dir(&game).join("pak01_dir.vpk"), pak).unwrap();

        let s2v = t.path().join("s2v.exe");
        let decompiler = |_: &Path, args: &[String]| -> Result<(bool, String)> {
            let out = PathBuf::from(&args[args.iter().position(|a| a == "-o").unwrap() + 1]);
            if args.iter().any(|a| a == "--gltf_export_format") {
                std::fs::write(&out, b"glTF").unwrap();
            } else {
                assert!(args[3].contains("materials/hero.vmat_c"), "materials decompiled too: {}", args[3]);
                let d = out.join("models/heroes_staging/hero");
                std::fs::create_dir_all(d.join("clips")).unwrap();
                std::fs::create_dir_all(d.join("dmx/animation")).unwrap();
                std::fs::write(d.join("hero.vmdl"), VMDL).unwrap();
                for f in ["hero_body.dmx", "hero_hull.dmx", "hero_idle.dmx"] {
                    std::fs::write(d.join(f), b"dmx").unwrap();
                }
            }
            Ok((true, "ok".into()))
        };
        let project = t.path().join("project");
        let r = extract(&s2v, &game, model, &project, &decompiler, &|_| {}).unwrap();
        assert_eq!(r.mesh_files.len(), 2);
        let folder = project.join("original/models/heroes_staging/hero");
        assert!(!folder.join("hero_idle.dmx").exists(), "animation DMX dropped");
        assert!(!folder.join("clips").exists() && !folder.join("dmx").exists());
        assert!(folder.join("hero_body.dmx").exists() && folder.join("hero.vmdl_c").exists());
        assert!(project.join("blender/hero.glb").exists() && project.join("return").is_dir());

        // Build
        let csdk_dir = t.path().join("csdk");
        std::fs::create_dir_all(csdk_dir.join("content")).unwrap();
        std::fs::create_dir_all(csdk::compiler_path(&csdk_dir).parent().unwrap()).unwrap();
        std::fs::write(csdk::compiler_path(&csdk_dir), b"").unwrap();
        let compiler = |csdk_dir: &Path, file: &Path| -> Result<(bool, String)> {
            let rel = file.strip_prefix(csdk_dir.join("content")).unwrap();
            let text = std::fs::read_to_string(file).unwrap();
            assert!(text.contains("AnimIncludeModel"));
            let out = csdk_dir.join("game").join(rel).with_extension("vmdl_c");
            assert!(out.parent().unwrap().join("hero_backup.vmdl_c").exists(), "backup is in place before compiling");
            assert!(out.parent().unwrap().join("hero.vnmskel_c").exists(), "deps are in place before compiling");
            std::fs::write(&out, b"compiled").unwrap();
            Ok((true, "WARNING: something minor\nOK: 1 compiled, 0 failed".into()))
        };
        let out = t.path().join("out/hero.vpk");
        let b = build(&csdk_dir, &game, model, &project, "Seven test", None, &Default::default(), &out, &compiler, &|_| {}).unwrap();
        assert_eq!(b.files, vec!["models/heroes_staging/hero/hero.vmdl_c", "models/heroes_staging/hero/hero_backup.vmdl_c"]);
        assert_eq!(b.warnings, vec!["WARNING: something minor"]);
        let packed = vpk::read_directory(&out).unwrap();
        assert_eq!(packed.extract(packed.find("models/heroes_staging/hero/hero_backup.vmdl_c").unwrap()).unwrap(), compiled);
        assert!(csdk_dir.join("content/citadel_addons/dms_seven_test/models/heroes_staging/hero/hero_body.dmx").exists());
    }

    /// A minimal compiled resource: a DATA block (empty KV3 object) and a
    /// RERL block listing `refs`.
    fn fake_compiled(refs: &[&str]) -> Vec<u8> {
        // RERL: offset(8) count, entries of 16 bytes (id u64, name offset u32, pad), names.
        let mut rerl = vec![];
        rerl.extend(8u32.to_le_bytes());
        rerl.extend((refs.len() as u32).to_le_bytes());
        let names_at = 8 + refs.len() * 16;
        let mut names = vec![];
        for (i, r) in refs.iter().enumerate() {
            let entry = 8 + i * 16;
            rerl.extend(0u64.to_le_bytes());
            let rel = (names_at + names.len()) - (entry + 8);
            rerl.extend((rel as u32).to_le_bytes());
            rerl.extend(0u32.to_le_bytes());
            names.extend(r.as_bytes());
            names.push(0);
        }
        rerl.extend(names);
        let data = kv3_empty_object();
        // Header: size, hv 12, ver 0, block offset 8, count 2; table; blocks.
        let table_at = 16;
        let blocks_at = table_at + 24;
        let mut out = vec![];
        let total = blocks_at + rerl.len() + data.len();
        out.extend((total as u32).to_le_bytes());
        out.extend(12u16.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out.extend(8u32.to_le_bytes());
        out.extend(2u32.to_le_bytes());
        for (i, (tag, off, len)) in [(b"RERL", blocks_at, rerl.len()), (b"DATA", blocks_at + rerl.len(), data.len())].iter().enumerate() {
            let entry = table_at + i * 12;
            out.extend(*tag);
            out.extend(((off - (entry + 4)) as u32).to_le_bytes());
            out.extend((*len as u32).to_le_bytes());
        }
        out.extend(rerl);
        out.extend(data);
        out
    }

    /// Binary KV3 v1 (uncompressed) holding `{}`.
    fn kv3_empty_object() -> Vec<u8> {
        let mut v = vec![];
        v.extend(0x4B56_3301u32.to_le_bytes()); // "KV3" v1
        v.extend([0u8; 16]); // format guid
        v.extend(0u32.to_le_bytes()); // method: none
        v.extend(0i32.to_le_bytes()); // bytes
        v.extend(2i32.to_le_bytes()); // ints: string count + object length
        v.extend(0i32.to_le_bytes()); // doubles
        let mut body = vec![];
        body.extend(0u32.to_le_bytes()); // 0 strings
        body.extend(0u32.to_le_bytes()); // object with 0 members
        body.push(9); // type: object
        body.extend(0xFFEE_DD00u32.to_le_bytes()); // end marker
        v.extend((body.len() as i32).to_le_bytes()); // uncompressed size
        v.extend(body);
        v
    }

    #[test]
    fn picks_each_heros_own_animations() {
        let mk = |names: &[&str]| {
            let nodes: Vec<String> = names.iter().map(|n| format!("{{ _class = \"AnimFile\" name = \"{n}\" }}")).collect();
            kv3text::parse(&format!("<!-- kv3 encoding:text:version{{e21c7f3c-8a33-41c5-9977-a76d3a32aa0d}} format:modeldoc29:version{{3cec427c-1b0e-4d48-a90a-0436f33a6041}} -->\n{{ rootNode = {{ children = [ {} ] }} }}", nodes.join(", "))).unwrap()
        };
        assert_eq!(blender_anims(&mk(&["self_primary_aim_idle", "primary_stand_idle", "primary_run_n", "ui_hero_pose"])), "primary_stand_idle,primary_run_n,ui_hero_pose");
        assert_eq!(blender_anims(&mk(&["melee_idle", "baton_run_n", "baton_crouch_idle"])), "melee_idle,baton_run_n,baton_crouch_idle");
        assert_eq!(blender_anims(&mk(&[])), BLENDER_ANIMS);
    }

    #[test]
    fn cloth_chains_copy_the_heros_layout() {
        let text = "<!-- kv3 encoding:text:version{e21c7f3c-8a33-41c5-9977-a76d3a32aa0d} format:modeldoc29:version{3cec427c-1b0e-4d48-a90a-0436f33a6041} -->\n{ rootNode = { children = [ { _class = \"Softbody\" children = [ { _class = \"Folder\" children = [ { _class = \"ClothChain\" name = \"hair\" root_bone = \"hair_0\" chain = { joints = [ { joint_name = \"hair_0\" simulate = false goal_strength = 0.6 collision_radius = 0.0 collision_radius = 0.5 }, { joint_name = \"hair_1\" joint_parent = \"hair_0\" simulate = true goal_strength = 0.1 } ] attrs = { } } } ] } ] } ] } }";
        let mut doc = kv3text::parse(text).unwrap();
        let chains = vec![crate::blender::PhysicsChain {
            name: "dms_root".into(),
            parent: "head".into(),
            group: String::new(),
            joints: vec![
                crate::blender::PhysicsJoint { bone: "dms_root".into(), parent: "".into(), simulate: false, preset: "pinned".into(), index: 0 },
                crate::blender::PhysicsJoint { bone: "dms_cable_2".into(), parent: "dms_root".into(), simulate: true, preset: "dangle".into(), index: 0 },
            ],
        }];
        let mut tuning = std::collections::BTreeMap::new();
        tuning.insert("dms_root/dangle".to_string(), PhysicsTuning { stiffness: 0.5, ..Default::default() });
        assert_eq!(add_cloth_chains(&mut doc, &chains, &tuning, &Default::default()), Ok(1));
        let out = kv3text::to_string(&doc);
        assert!(out.contains("name = \"dms_root\"") && out.contains("root_bone = \"dms_root\""));
        assert!(out.contains("joint_parent = \"dms_root\""));
        assert!(out.contains("goal_strength = 0.2"), "0.4 x 0.5 stiffness: {out}");
        assert!(out.contains("goal_damping = 0.003"));
        let mine = &out[out.find("name = \"dms_root\"").unwrap()..];
        assert!(!mine.contains("collision_radius = 0.5"), "duplicate keys all set");
        assert!(kv3text::parse(&out).is_ok());
        assert_eq!(out.matches("_class = \"ClothChain\"").count(), 2);
        // A skirt's strands share their group's tuning.
        let mut doc2 = kv3text::parse(text).unwrap();
        let mut strands = vec![chains[0].clone(), chains[0].clone()];
        strands[0].name = "dms_skirt_0".into();
        strands[1].name = "dms_skirt_1".into();
        for c in &mut strands {
            c.group = "dms_skirt".into();
            c.joints[1].preset = "skirt".into();
        }
        let mut gt = std::collections::BTreeMap::new();
        gt.insert("dms_skirt/skirt".to_string(), PhysicsTuning { stiffness: 0.5, ..Default::default() });
        assert_eq!(add_cloth_chains(&mut doc2, &strands, &gt, &Default::default()), Ok(2));
        assert_eq!(kv3text::to_string(&doc2).matches("goal_strength = 0.3").count(), 2, "0.6 x 0.5 on both strands");
        let empty: Vec<crate::blender::PhysicsChain> = vec![];
        assert_eq!(add_cloth_chains(&mut kv3text::parse(text).unwrap(), &empty, &tuning, &Default::default()), Ok(0));

        // Haze-style cloth: nodes in a folder, springs beside it.
        let haze = "<!-- kv3 encoding:text:version{e21c7f3c-8a33-41c5-9977-a76d3a32aa0d} format:modeldoc29:version{3cec427c-1b0e-4d48-a90a-0436f33a6041} -->\n{ rootNode = { children = [ { _class = \"Softbody\" children = [ { _class = \"ClothParams\" }, { _class = \"Folder\" children = [ { _class = \"ClothNode\" name = \"hose_a\" cloth_node_root_bone = \"hose_a\" is_static_node = true goal_strength = 0.6 }, { _class = \"ClothNode\" name = \"hose_mid\" cloth_node_root_bone = \"hose_mid\" is_static_node = false goal_strength = 0.0 mass = 1.0 } ] }, { _class = \"ClothSpring\" name = \"rod_1\" cloth_node_0 = \"hose_a\" cloth_node_1 = \"hose_mid\" stiffness = 0.05 enable_advanced_parameters = true is_length_explicit = true min_length = 10.0 max_length = 10.0 } ] } ] } }";
        let mut hd = kv3text::parse(haze).unwrap();
        let pos: std::collections::HashMap<String, [f64; 3]> = [("dms_root".to_string(), [0.0, 1.0, 0.0]), ("dms_cable_2".to_string(), [0.0, 0.9, 0.0])].into_iter().collect();
        assert_eq!(add_cloth_chains(&mut hd, &chains, &tuning, &pos), Ok(1));
        let out = kv3text::to_string(&hd);
        assert!(kv3text::parse(&out).is_ok());
        assert_eq!(out.matches("_class = \"ClothNode\"").count(), 4);
        assert_eq!(out.matches("_class = \"ClothSpring\"").count(), 2);
        assert!(out.contains("cloth_node_root_bone = \"dms_cable_2\""));
        assert!(out.contains("cloth_node_0 = \"dms_root\"") && out.contains("cloth_node_1 = \"dms_cable_2\""));
        let mine = &out[out.find("name = \"dms_cable_2\"").unwrap()..];
        assert!(mine.contains("is_static_node = false") && mine.contains("goal_strength = 0.2") && mine.contains("mass = 1.0"));
        let root = &out[out.find("name = \"dms_root\"").unwrap()..];
        assert!(root.contains("is_static_node = true"));
        let rod = &out[out.find("name = \"rod_dms_cable_2\"").unwrap()..];
        assert!(rod.contains("is_length_explicit = true") && rod.contains("min_length = 3.937") && rod.contains("max_length = 3.937"), "10 cm in game units: {rod}");

        // Two pinned bones in a row (a ponytail's root and first bone): no rod between them.
        let mut pinned = chains.clone();
        pinned[0].joints.insert(1, crate::blender::PhysicsJoint { bone: "dms_hair_000".into(), parent: "dms_root".into(), simulate: false, preset: "pinned".into(), index: 0 });
        pinned[0].joints[2].parent = "dms_hair_000".into();
        let mut hd2 = kv3text::parse(haze).unwrap();
        add_cloth_chains(&mut hd2, &pinned, &tuning, &Default::default()).unwrap();
        let out2 = kv3text::to_string(&hd2);
        assert!(!out2.contains("rod_dms_hair_000"), "static-static rod");
        assert!(out2.contains("cloth_node_0 = \"dms_hair_000\""));
    }
}

