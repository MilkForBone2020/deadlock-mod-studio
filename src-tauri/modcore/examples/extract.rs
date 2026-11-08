//! Dev helper: extract files from a VPK into a folder.
//! cargo run --example extract -- <pak01_dir.vpk> <out_dir> <path>...
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let d = modcore::vpk::read_directory(std::path::Path::new(&a[1])).unwrap();
    let out = std::path::Path::new(&a[2]);
    for p in &a[3..] {
        let e = d.find(p).unwrap_or_else(|| panic!("not found: {p}"));
        let bytes = d.extract(e).unwrap();
        let dest = out.join(p);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, &bytes).unwrap();
        println!("{p}: {} bytes", bytes.len());
    }
}
