//! Dev helper: summarise a model's PHYS block (ragdoll + cloth).
use modcore::kv3::{self, Kv};
fn keys(k: &Kv) -> Vec<String> { k.as_object().map(|o| o.iter().map(|(a, _)| a.clone()).collect()).unwrap_or_default() }
fn len(k: Option<&Kv>) -> usize { k.and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0) }
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let res = std::fs::read(&a[1]).unwrap();
    let p = kv3::parse(kv3::resource_block(&res, b"PHYS").unwrap()).unwrap();
    println!("PHYS keys: {:?}", keys(&p));
    println!("parts={} constraints={} boneNames={}", len(p.get("m_parts")), len(p.get("m_constraints2")), len(p.get("m_boneNames")));
    let bn: Vec<&str> = p.get("m_boneNames").and_then(|v| v.as_array()).unwrap_or(&[]).iter().filter_map(|b| b.as_str()).collect();
    println!("ragdoll bones: {}", bn.join(", "));
    if let Some(fe) = p.get("m_pFeModel") {
        println!("cloth (m_pFeModel) keys: {}", keys(fe).len());
        let ctrl: Vec<&str> = fe.get("m_CtrlName").and_then(|v| v.as_array()).unwrap_or(&[]).iter().filter_map(|b| b.as_str()).collect();
        println!("cloth controls {}: {}", ctrl.len(), ctrl.iter().take(400).cloned().collect::<Vec<_>>().join(", "));
        for k in ["m_nNodeCount", "m_nStaticNodes", "m_nRodCount", "m_nQuadCount", "m_nSimdQuadCount1", "m_nTreeDepth"] { if let Some(v) = fe.get(k) { println!("  {k} = {v:?}"); } }
        println!("  collision spheres={} capsules(tapered)={}", len(fe.get("m_SphereRigids")), len(fe.get("m_TaperedCapsuleRigids")));
    } else { println!("no cloth"); }
}
