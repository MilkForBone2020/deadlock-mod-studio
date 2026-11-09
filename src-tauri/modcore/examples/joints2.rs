use modcore::dmx::Attr;
fn show(d: &modcore::dmx::Dmx, i: usize, depth: usize, max: usize) {
    let e = &d.elements[i];
    println!("{}{} '{}' [{}]", "  ".repeat(depth), e.ty, e.name, i);
    if depth >= max { return; }
    for (k, v) in &e.attrs {
        match v {
            Attr::Element(x) if *x >= 0 => { println!("{}  .{k} ->", "  ".repeat(depth)); show(d, *x as usize, depth + 2, max); }
            Attr::Array { ty: 1, count, .. } => println!("{}  .{k} = [{} elements] first {:?}", "  ".repeat(depth), count, e.elements(k).iter().take(3).map(|j| format!("{} '{}'", d.elements[*j].ty, d.elements[*j].name)).collect::<Vec<_>>()),
            Attr::Raw { ty, bytes } => println!("{}  .{k} raw ty{ty} {:?}", "  ".repeat(depth), bytes.chunks(4).map(|c| if c.len()==4 { f32::from_le_bytes(c.try_into().unwrap()) } else { 0.0 }).collect::<Vec<_>>()),
            Attr::Str(s) => println!("{}  .{k} = {s:?}", "  ".repeat(depth)),
            Attr::Array { ty, count, .. } => println!("{}  .{k} array ty{ty} n{count}", "  ".repeat(depth)),
            _ => println!("{}  .{k} other", "  ".repeat(depth)),
        }
    }
}
fn main() {
    let d = modcore::dmx::parse(&std::fs::read(std::env::args().nth(1).unwrap()).unwrap()).unwrap();
    let model = d.elements[0].element("model").unwrap();
    show(&d, model, 0, 0);
    let jl = d.elements[model].elements("jointList");
    for &j in jl.iter().take(3) { show(&d, j, 1, 3); }
    let last = jl[jl.len() - 9];
    show(&d, last, 1, 3);
    show(&d, 4, 0, 1); show(&d, 6, 0, 1);
    let bs = d.elements[model].elements("baseStates"); println!("baseStates {:?}", bs);
    for &b in &bs { let e=&d.elements[b]; for (k,_) in &e.attrs { println!("  bs.{k} n={}", e.elements(k).len()); } let t=e.elements("transforms"); for &x in t.iter().take(2) { show(&d, x, 2, 1); } }
    for k in ["baseStates"] { for &b in d.elements[model].elements(k).iter().take(1) { show(&d, b, 1, 1); } }
}
