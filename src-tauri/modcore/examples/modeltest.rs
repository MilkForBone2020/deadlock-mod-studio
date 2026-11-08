//! Dev helper: run the real extract on a game folder with a real S2V CLI.
//! cargo run --example modeltest -- <s2v exe> <game dir> <model> <project dir>
use std::path::Path;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let r = modcore::model::extract(Path::new(&a[1]), Path::new(&a[2]), &a[3], Path::new(&a[4]), &modcore::model::run_tool, &|m| eprintln!("> {m}")).unwrap();
    let s = &r.summary;
    println!("bones {} meshes {} lods {} attachments {} hitboxes {} ragdoll {} cloth {} tris {}", s.bones.len(), s.meshes, s.lods, s.attachments.len(), s.hitboxes, s.ragdoll_bodies, s.cloth_nodes, s.triangles_lod0);
    println!("body groups {:?}\nmaterials {:?}\ngraphs {:?}", s.body_groups, s.materials, s.anim_graphs);
    println!("mesh files {:#?}", r.mesh_files);
    let mut doc = modcore::kv3text::parse(&std::fs::read_to_string(&r.template).unwrap()).unwrap();
    modcore::model::use_backup_animations(&mut doc, &a[3]).unwrap();
    std::fs::write(Path::new(&a[4]).join("prepared.vmdl"), modcore::kv3text::to_string(&doc)).unwrap();
    let p = modcore::model::write_blender_project(Path::new(&a[4]), "test1", "Seven test", "gigawatt", "Seven", &a[3]).unwrap();
    println!("{}", std::fs::read_to_string(p).unwrap().lines().take(40).collect::<Vec<_>>().join("\n"));
}
