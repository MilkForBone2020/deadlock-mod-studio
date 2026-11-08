//! Dev helper: print a KV3 resource as an indented tree (depth-limited).
fn show(k: &modcore::kv3::Kv, d: usize, max: usize, key: &str, filter: &str) {
    use modcore::kv3::Kv;
    let pad = "  ".repeat(d);
    match k {
        Kv::Object(m) => { println!("{pad}{key} {{{}}}", m.len()); if d < max { for (kk, v) in m { if d==0 && !filter.is_empty() && !kk.contains(filter) { continue; } show(v, d+1, max, kk, filter) } } }
        Kv::Array(a) => { println!("{pad}{key} [{}]", a.len()); if d < max { for v in a.iter().take(8) { show(v, d+1, max, "-", filter) } } }
        other => println!("{pad}{key} = {:?}", other),
    }
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let kv = modcore::kv3::parse_resource(&std::fs::read(&a[1]).unwrap()).unwrap();
    show(&kv, 0, a[2].parse().unwrap(), "root", a.get(3).map(|s| s.as_str()).unwrap_or(""));
}
