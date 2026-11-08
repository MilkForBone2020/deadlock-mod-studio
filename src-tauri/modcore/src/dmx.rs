//! Valve's binary DMX (encoding `binary` version 9), as written by Source 2
//! Viewer for model meshes (`format model 22`).
//!
//! Layout: a text header line ending in NUL; prefix elements; a string
//! table; the element list (type, name, GUID); then each element's
//! attributes (name, type byte, value). Single strings, element types and
//! names are string-table indices; strings inside arrays are inline.
//! Array types are the scalar type + 32.

use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum Attr {
    /// Element reference: index into `elements`, -1 for none.
    Element(i32),
    Str(String),
    /// Any other single value, kept as its raw bytes.
    Raw { ty: u8, bytes: Vec<u8> },
    StrArray(Vec<String>),
    /// A fixed-size array (`ty` is the scalar type), raw little-endian bytes.
    Array { ty: u8, count: u32, bytes: Vec<u8> },
    BinArray(Vec<Vec<u8>>),
}

pub const T_ELEMENT: u8 = 1;
pub const T_INT: u8 = 2;
pub const T_FLOAT: u8 = 3;
pub const T_BOOL: u8 = 4;
pub const T_STRING: u8 = 5;
pub const T_BINARY: u8 = 6;
pub const T_VEC2: u8 = 9;
pub const T_VEC3: u8 = 10;
pub const T_VEC4: u8 = 11;
const ARRAY: u8 = 32;

fn size_of(ty: u8) -> Option<usize> {
    Some(match ty {
        1..=3 | 7 | 8 => 4,
        4 | 16 => 1,
        9 => 8,
        10 | 12 => 12,
        11 | 13 => 16,
        14 => 64,
        15 => 8,
        _ => return None,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub ty: String,
    pub name: String,
    pub guid: [u8; 16],
    pub attrs: Vec<(String, Attr)>,
}

impl Element {
    pub fn get(&self, key: &str) -> Option<&Attr> {
        self.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    pub fn set(&mut self, key: &str, v: Attr) {
        match self.attrs.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = v,
            None => self.attrs.push((key.to_string(), v)),
        }
    }
    pub fn element(&self, key: &str) -> Option<usize> {
        match self.get(key) {
            Some(Attr::Element(i)) if *i >= 0 => Some(*i as usize),
            _ => None,
        }
    }
    pub fn elements(&self, key: &str) -> Vec<usize> {
        match self.get(key) {
            Some(Attr::Array { ty: T_ELEMENT, bytes, .. }) => {
                bytes.chunks_exact(4).map(|c| i32::from_le_bytes(c.try_into().unwrap())).filter(|&i| i >= 0).map(|i| i as usize).collect()
            }
            _ => vec![],
        }
    }
    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(Attr::Str(s)) => Some(s),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dmx {
    pub header: String,
    pub elements: Vec<Element>,
}

struct R<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let s = self.d.get(self.p..self.p + n).ok_or_else(|| Error::msg("DMX: unexpected end of file"))?;
        self.p += n;
        Ok(s)
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn cstr(&mut self) -> Result<String> {
        let rest = &self.d[self.p..];
        let n = rest.iter().position(|&b| b == 0).ok_or_else(|| Error::msg("DMX: unterminated string"))?;
        let s = String::from_utf8_lossy(&rest[..n]).into_owned();
        self.p += n + 1;
        Ok(s)
    }
    fn count(&mut self) -> Result<usize> {
        let n = self.i32()?;
        if n < 0 {
            return Err(Error::msg("DMX: negative count"));
        }
        Ok(n as usize)
    }
}

pub fn parse(d: &[u8]) -> Result<Dmx> {
    let mut r = R { d, p: 0 };
    let header = r.cstr()?;
    if !header.contains("encoding binary 9") {
        return Err(Error::msg(format!("Unsupported DMX: {}", header.trim())));
    }
    if r.count()? != 0 {
        return Err(Error::msg("DMX: prefix elements aren't supported"));
    }
    let nstr = r.count()?;
    let mut strs = Vec::with_capacity(nstr);
    for _ in 0..nstr {
        strs.push(r.cstr()?);
    }
    let s = |i: i32| -> Result<String> { strs.get(i as usize).cloned().ok_or_else(|| Error::msg("DMX: bad string index")) };
    let nel = r.count()?;
    let mut elements = Vec::with_capacity(nel);
    for _ in 0..nel {
        let ty = s(r.i32()?)?;
        let name = s(r.i32()?)?;
        let guid: [u8; 16] = r.take(16)?.try_into().unwrap();
        elements.push(Element { ty, name, guid, attrs: vec![] });
    }
    for el in elements.iter_mut() {
        let na = r.count()?;
        for _ in 0..na {
            let key = s(r.i32()?)?;
            let ty = r.u8()?;
            let v = match ty {
                T_ELEMENT => {
                    let i = r.i32()?;
                    if i == -2 {
                        return Err(Error::msg("DMX: external element references aren't supported"));
                    }
                    Attr::Element(i)
                }
                T_STRING => Attr::Str(s(r.i32()?)?),
                T_BINARY => {
                    let n = r.count()?;
                    Attr::Raw { ty, bytes: r.take(n)?.to_vec() }
                }
                t if t > ARRAY => {
                    let base = t - ARRAY;
                    let n = r.count()?;
                    match base {
                        T_STRING => {
                            let mut v = Vec::with_capacity(n);
                            for _ in 0..n {
                                v.push(r.cstr()?);
                            }
                            Attr::StrArray(v)
                        }
                        T_BINARY => {
                            let mut v = Vec::with_capacity(n);
                            for _ in 0..n {
                                let m = r.count()?;
                                v.push(r.take(m)?.to_vec());
                            }
                            Attr::BinArray(v)
                        }
                        _ => {
                            let sz = size_of(base).ok_or_else(|| Error::msg(format!("DMX: unknown array type {t}")))?;
                            Attr::Array { ty: base, count: n as u32, bytes: r.take(sz * n)?.to_vec() }
                        }
                    }
                }
                _ => {
                    let sz = size_of(ty).ok_or_else(|| Error::msg(format!("DMX: unknown type {ty}")))?;
                    Attr::Raw { ty, bytes: r.take(sz)?.to_vec() }
                }
            };
            el.attrs.push((key, v));
        }
    }
    Ok(Dmx { header, elements })
}

pub fn write(dmx: &Dmx) -> Vec<u8> {
    use std::collections::HashMap;
    let mut table: Vec<&str> = vec![];
    let mut index: HashMap<&str, i32> = HashMap::new();
    for e in &dmx.elements {
        for s in [e.ty.as_str(), e.name.as_str()] {
            if !index.contains_key(s) {
                index.insert(s, table.len() as i32);
                table.push(s);
            }
        }
        for (k, v) in &e.attrs {
            let mut strs = vec![k.as_str()];
            if let Attr::Str(s) = v {
                strs.push(s.as_str());
            }
            for s in strs {
                if !index.contains_key(s) {
                    index.insert(s, table.len() as i32);
                    table.push(s);
                }
            }
        }
    }
    let mut out = Vec::new();
    out.extend_from_slice(dmx.header.as_bytes());
    out.push(0);
    out.extend(0i32.to_le_bytes());
    out.extend((table.len() as i32).to_le_bytes());
    for s in &table {
        out.extend_from_slice(s.as_bytes());
        out.push(0);
    }
    out.extend((dmx.elements.len() as i32).to_le_bytes());
    for e in &dmx.elements {
        out.extend(index[e.ty.as_str()].to_le_bytes());
        out.extend(index[e.name.as_str()].to_le_bytes());
        out.extend_from_slice(&e.guid);
    }
    for e in &dmx.elements {
        out.extend((e.attrs.len() as i32).to_le_bytes());
        for (k, v) in &e.attrs {
            out.extend(index[k.as_str()].to_le_bytes());
            match v {
                Attr::Element(i) => {
                    out.push(T_ELEMENT);
                    out.extend(i.to_le_bytes());
                }
                Attr::Str(s) => {
                    out.push(T_STRING);
                    out.extend(index[s.as_str()].to_le_bytes());
                }
                Attr::Raw { ty, bytes } => {
                    out.push(*ty);
                    if *ty == T_BINARY {
                        out.extend((bytes.len() as i32).to_le_bytes());
                    }
                    out.extend_from_slice(bytes);
                }
                Attr::StrArray(v) => {
                    out.push(T_STRING + ARRAY);
                    out.extend((v.len() as i32).to_le_bytes());
                    for s in v {
                        out.extend_from_slice(s.as_bytes());
                        out.push(0);
                    }
                }
                Attr::Array { ty, count, bytes } => {
                    out.push(ty + ARRAY);
                    out.extend((*count as i32).to_le_bytes());
                    out.extend_from_slice(bytes);
                }
                Attr::BinArray(v) => {
                    out.push(T_BINARY + ARRAY);
                    out.extend((v.len() as i32).to_le_bytes());
                    for b in v {
                        out.extend((b.len() as i32).to_le_bytes());
                        out.extend_from_slice(b);
                    }
                }
            }
        }
    }
    out
}

// ----- typed array helpers -----

pub fn f32_array(ty: u8, values: &[f32]) -> Attr {
    let per = size_of(ty).unwrap() / 4;
    Attr::Array { ty, count: (values.len() / per) as u32, bytes: values.iter().flat_map(|v| v.to_le_bytes()).collect() }
}

pub fn i32_array(ty: u8, values: &[i32]) -> Attr {
    Attr::Array { ty, count: values.len() as u32, bytes: values.iter().flat_map(|v| v.to_le_bytes()).collect() }
}

/// Floats of a float-based array (vectors flattened).
pub fn floats(a: &Attr) -> Vec<f32> {
    match a {
        Attr::Array { bytes, .. } => bytes.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect(),
        _ => vec![],
    }
}

pub fn ints(a: &Attr) -> Vec<i32> {
    match a {
        Attr::Array { bytes, .. } => bytes.chunks_exact(4).map(|c| i32::from_le_bytes(c.try_into().unwrap())).collect(),
        _ => vec![],
    }
}

fn new_guid(seed: usize) -> [u8; 16] {
    let u = uuid::Uuid::new_v4();
    let mut g = *u.as_bytes();
    g[0] ^= (seed & 0xff) as u8;
    g
}

impl Dmx {
    pub fn add(&mut self, ty: &str, name: &str, attrs: Vec<(String, Attr)>) -> usize {
        let n = self.elements.len();
        self.elements.push(Element { ty: ty.into(), name: name.into(), guid: new_guid(n), attrs });
        n
    }

    pub fn find_type(&self, ty: &str) -> Vec<usize> {
        (0..self.elements.len()).filter(|&i| self.elements[i].ty == ty).collect()
    }

    /// Drop elements nothing refers to (starting from element 0, the root),
    /// renumbering references.
    pub fn compact(&mut self) {
        let n = self.elements.len();
        let mut keep = vec![false; n];
        let mut stack = vec![0usize];
        while let Some(i) = stack.pop() {
            if i >= n || keep[i] {
                continue;
            }
            keep[i] = true;
            for (_, v) in &self.elements[i].attrs {
                match v {
                    Attr::Element(j) if *j >= 0 => stack.push(*j as usize),
                    Attr::Array { ty: T_ELEMENT, bytes, .. } => {
                        stack.extend(bytes.chunks_exact(4).map(|c| i32::from_le_bytes(c.try_into().unwrap())).filter(|&j| j >= 0).map(|j| j as usize))
                    }
                    _ => {}
                }
            }
        }
        let mut map = vec![-1i32; n];
        let mut next = 0;
        for i in 0..n {
            if keep[i] {
                map[i] = next;
                next += 1;
            }
        }
        let old = std::mem::take(&mut self.elements);
        for (i, mut e) in old.into_iter().enumerate() {
            if !keep[i] {
                continue;
            }
            for (_, v) in e.attrs.iter_mut() {
                match v {
                    Attr::Element(j) if *j >= 0 => *j = map[*j as usize],
                    Attr::Array { ty: T_ELEMENT, bytes, .. } => {
                        let re: Vec<u8> = bytes
                            .chunks_exact(4)
                            .flat_map(|c| {
                                let j = i32::from_le_bytes(c.try_into().unwrap());
                                (if j >= 0 { map[j as usize] } else { j }).to_le_bytes()
                            })
                            .collect();
                        *bytes = re;
                    }
                    _ => {}
                }
            }
            self.elements.push(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Dmx {
        let mut d = Dmx { header: "<!-- dmx encoding binary 9 format model 22 -->\n".into(), elements: vec![] };
        let vd = d.add(
            "DmeVertexData",
            "bind",
            vec![
                ("vertexFormat".into(), Attr::StrArray(vec!["position$0".into()])),
                ("jointCount".into(), Attr::Raw { ty: T_INT, bytes: 1i32.to_le_bytes().to_vec() }),
                ("position$0".into(), f32_array(T_VEC3, &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0])),
                ("blob".into(), Attr::Raw { ty: T_BINARY, bytes: vec![1, 2, 3] }),
            ],
        );
        let root = d.add("DmElement", "root", vec![("mesh".into(), Attr::Element(vd as i32)), ("label".into(), Attr::Str("hi".into()))]);
        d.elements.swap(vd, root);
        d.elements[0].attrs[0].1 = Attr::Element(1);
        d.add("DmElement", "orphan", vec![]);
        d
    }

    #[test]
    fn round_trips_and_compacts() {
        let d = sample();
        let back = parse(&write(&d)).unwrap();
        assert_eq!(back, d);
        assert_eq!(floats(back.elements[1].get("position$0").unwrap()), vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
        let mut c = back.clone();
        c.compact();
        assert_eq!(c.elements.len(), 2, "orphan dropped");
        assert_eq!(c.elements[0].element("mesh"), Some(1));
        assert!(parse(b"<!-- dmx encoding keyvalues2 1 -->\0").is_err());
    }

    /// Luke's real Seven gun, if staged on this machine.
    #[test]
    fn real_file_round_trips() {
        let p = std::path::Path::new("/mnt/user-data/uploads/models/be5e81c5b30b/original/models/heroes_staging/gigawatt_prisoner/gigawatt_prisoner_gun1.dmx");
        if !p.exists() {
            return;
        }
        let bytes = std::fs::read(p).unwrap();
        let d = parse(&bytes).unwrap();
        assert_eq!(d.elements.len(), 503);
        let again = write(&d);
        assert_eq!(again.len(), bytes.len(), "same size");
        assert_eq!(parse(&again).unwrap(), d);
    }
}
