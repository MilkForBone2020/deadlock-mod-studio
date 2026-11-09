//! Dev helper: apply a model sent back from Blender to a project.
//! cargo run --example applyreturn -- <project dir> <return folder name>
use std::path::Path;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let proj = Path::new(&a[1]);
    let model = "models/heroes_staging/gigawatt_prisoner/gigawatt_prisoner.vmdl";
    let src = proj.join("original/models/heroes_staging/gigawatt_prisoner/gigawatt_prisoner.vmdl");
    let mut doc = modcore::kv3text::parse(&std::fs::read_to_string(&src).unwrap()).unwrap();
    let out = Path::new("/tmp/swapout");
    let _ = std::fs::remove_dir_all(out);
    let r = modcore::model::apply_returned(&mut doc, proj, model, &proj.join("return").join(&a[2]), out, &Default::default()).unwrap();
    for p in r { println!("{} {} {} {}", p.part, p.status, p.vertices, p.triangles); }
}
