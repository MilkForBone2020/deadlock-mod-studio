//! Reader for Valve's binary KeyValues3 (versions 1–5, uncompressed, LZ4 or
//! Zstandard), as stored in the DATA block of compiled resources such as
//! sound event files (`.vsndevts_c`).
//!
//! Ported from ValveResourceFormat's `BinaryKV3.cs` (MIT licence,
//! Copyright (c) 2015 ValveResourceFormat Contributors). Read-only, and
//! Binary blob contents are not decoded (blob nodes parse as empty).

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum Kv {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    Str(String),
    Blob(Vec<u8>),
    Array(Vec<Kv>),
    Object(Vec<(String, Kv)>),
}

impl Kv {
    pub fn get(&self, key: &str) -> Option<&Kv> {
        match self {
            Kv::Object(m) => m.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Kv::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_object(&self) -> Option<&[(String, Kv)]> {
        match self {
            Kv::Object(m) => Some(m),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&[Kv]> {
        match self {
            Kv::Array(a) => Some(a),
            _ => None,
        }
    }
    /// Every string value anywhere inside this node.
    pub fn strings(&self, out: &mut Vec<String>) {
        match self {
            Kv::Str(s) => out.push(s.clone()),
            Kv::Array(a) => a.iter().for_each(|v| v.strings(out)),
            Kv::Object(m) => m.iter().for_each(|(_, v)| v.strings(out)),
            _ => {}
        }
    }
}

fn bad(msg: &str) -> Error {
    Error::msg(format!("KV3: {msg}"))
}

/// A little-endian cursor over one lane.
#[derive(Clone, Copy, Default)]
struct Lane<'a> {
    d: &'a [u8],
}

impl<'a> Lane<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.d.len() {
            return Err(bad("lane ran out of data"));
        }
        let (a, b) = self.d.split_at(n);
        self.d = b;
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes(b.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    fn u64(&mut self) -> Result<u64> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes(b.try_into().unwrap()))
    }
}

#[derive(Clone, Copy, Default)]
struct Buffers<'a> {
    b1: Lane<'a>,
    b2: Lane<'a>,
    b4: Lane<'a>,
    b8: Lane<'a>,
}

struct Ctx<'a> {
    version: u32,
    types: Lane<'a>,
    object_lengths: Lane<'a>,
    strings: Vec<String>,
    buf: Buffers<'a>,
    aux: Buffers<'a>,
    use_aux_main: bool,
    blobs: Vec<Vec<u8>>,
    next_blob: usize,
}

fn align(off: usize, a: usize) -> usize {
    off.div_ceil(a) * a
}

fn decompress(method: u32, input: &[u8], out_len: usize) -> Result<Vec<u8>> {
    match method {
        0 => {
            if input.len() < out_len {
                return Err(bad("uncompressed buffer is short"));
            }
            Ok(input[..out_len].to_vec())
        }
        1 => {
            let mut out = vec![0u8; out_len];
            let n = lz4_flex::block::decompress_into(input, &mut out).map_err(|e| bad(&format!("LZ4: {e}")))?;
            if n != out_len {
                return Err(bad("LZ4 size mismatch"));
            }
            Ok(out)
        }
        2 => {
            use std::io::Read;
            let mut r = input;
            let mut dec = ruzstd::StreamingDecoder::new(&mut r).map_err(|e| bad(&format!("zstd: {e}")))?;
            let mut out = vec![0u8; out_len];
            dec.read_exact(&mut out).map_err(|e| bad(&format!("zstd: {e}")))?;
            Ok(out)
        }
        m => Err(bad(&format!("unknown compression {m}"))),
    }
}

fn read_strings(lane: &mut Lane, count: usize) -> Result<Vec<String>> {
    let mut v = Vec::with_capacity(count);
    for _ in 0..count {
        let end = lane.d.iter().position(|&b| b == 0).ok_or_else(|| bad("unterminated string"))?;
        let s = String::from_utf8_lossy(&lane.d[..end]).into_owned();
        lane.d = &lane.d[end + 1..];
        v.push(s);
    }
    Ok(v)
}

/// Parse a KV3 binary block (starting at its magic).
pub fn parse(data: &[u8]) -> Result<Kv> {
    let mut h = Lane { d: data };
    let magic = h.u32()?;
    if magic & 0xFFFF_FF00 != 0x4B56_3300 {
        return Err(bad("not binary KV3 (or unsupported legacy VKV3)"));
    }
    let version = magic & 0xFF;
    if !(1..=5).contains(&version) {
        return Err(bad(&format!("unsupported version {version}")));
    }
    h.take(16)?; // format GUID
    let method = h.u32()?;
    let (count_b1, count_b4, count_b8, count_types, size_uncompressed_total, size_compressed_total, count_blocks, size_blobs);
    let mut count_b2 = 0usize;
    if version == 1 {
        count_b1 = h.i32()? as usize;
        count_b4 = h.i32()? as usize;
        count_b8 = h.i32()? as usize;
        size_uncompressed_total = h.i32()? as usize;
        size_compressed_total = h.d.len();
        count_types = 0;
        count_blocks = 0;
        size_blobs = 0;
    } else {
        let dict = h.u16()?;
        if dict != 0 {
            return Err(bad("compression dictionaries are not supported"));
        }
        let _frame = h.u16()?;
        count_b1 = h.i32()? as usize;
        count_b4 = h.i32()? as usize;
        count_b8 = h.i32()? as usize;
        count_types = h.i32()? as usize;
        h.u16()?;
        h.u16()?;
        size_uncompressed_total = h.i32()? as usize;
        size_compressed_total = h.i32()? as usize;
        count_blocks = h.i32()? as usize;
        size_blobs = h.i32()? as usize;
    }
    let mut size_block_sizes = 0usize;
    if version >= 4 {
        count_b2 = h.i32()? as usize;
        size_block_sizes = h.i32()? as usize;
    }
    let (mut su1, mut sc1, mut su2, mut sc2) = (size_uncompressed_total, size_compressed_total, 0usize, 0usize);
    let (mut c1b2, mut c2b2, mut c4b2, mut c8b2, mut cobj2) = (0usize, 0usize, 0usize, 0usize, 0usize);
    if version >= 5 {
        su1 = h.i32()? as usize;
        sc1 = h.i32()? as usize;
        su2 = h.i32()? as usize;
        sc2 = h.i32()? as usize;
        c1b2 = h.i32()? as usize;
        c2b2 = h.i32()? as usize;
        c4b2 = h.i32()? as usize;
        c8b2 = h.i32()? as usize;
        h.i32()?;
        cobj2 = h.i32()? as usize;
        h.i32()?;
        h.i32()?;
    }
    let _ = size_block_sizes;
    // Blob contents (mesh/physics payloads) aren't decoded: blob nodes come
    // back empty, which is fine for reading names and settings.
    let _ = (count_blocks, size_blobs);

    // Buffer 1
    let input1 = h.take(sc1.min(h.d.len()))?;
    let raw1 = decompress(method, input1, su1)?;
    let mut off = 0usize;
    let mut b1 = Buffers::default();
    if count_b1 > 0 {
        b1.b1 = Lane { d: raw1.get(off..off + count_b1).ok_or_else(|| bad("bad lane 1"))? };
        off += count_b1;
    }
    if count_b2 > 0 {
        off = align(off, 2);
        b1.b2 = Lane { d: raw1.get(off..off + count_b2 * 2).ok_or_else(|| bad("bad lane 2"))? };
        off += count_b2 * 2;
    }
    if count_b4 > 0 {
        off = align(off, 4);
        b1.b4 = Lane { d: raw1.get(off..off + count_b4 * 4).ok_or_else(|| bad("bad lane 4"))? };
        off += count_b4 * 4;
    }
    if count_b8 > 0 {
        off = align(off, 8);
        b1.b8 = Lane { d: raw1.get(off..off + count_b8 * 8).ok_or_else(|| bad("bad lane 8"))? };
        off += count_b8 * 8;
    } else if version < 5 {
        off = align(off, 8);
    }
    let count_strings = b1.b4.u32()? as usize;

    let raw2;
    let (strings, types, object_lengths, buf, aux);
    if version >= 5 {
        strings = read_strings(&mut b1.b1, count_strings)?;
        let input2 = h.take(sc2.min(h.d.len()))?;
        raw2 = decompress(method, input2, su2)?;
        let mut o = cobj2 * 4;
        let ol = Lane { d: raw2.get(..o).ok_or_else(|| bad("bad object lengths"))? };
        let mut b2 = Buffers::default();
        if c1b2 > 0 {
            b2.b1 = Lane { d: raw2.get(o..o + c1b2).ok_or_else(|| bad("bad b2 lane 1"))? };
            o += c1b2;
        }
        if c2b2 > 0 {
            o = align(o, 2);
            b2.b2 = Lane { d: raw2.get(o..o + c2b2 * 2).ok_or_else(|| bad("bad b2 lane 2"))? };
            o += c2b2 * 2;
        }
        if c4b2 > 0 {
            o = align(o, 4);
            b2.b4 = Lane { d: raw2.get(o..o + c4b2 * 4).ok_or_else(|| bad("bad b2 lane 4"))? };
            o += c4b2 * 4;
        }
        if c8b2 > 0 {
            o = align(o, 8);
            b2.b8 = Lane { d: raw2.get(o..o + c8b2 * 8).ok_or_else(|| bad("bad b2 lane 8"))? };
            o += c8b2 * 8;
        }
        types = Lane { d: raw2.get(o..o + count_types).ok_or_else(|| bad("bad types"))? };
        object_lengths = ol;
        buf = b2;
        aux = b1;
        let mut ctx = Ctx {
            version,
            types,
            object_lengths,
            strings,
            buf,
            aux,
            use_aux_main: false,
            blobs: vec![],
            next_blob: 0,
        };
        let (t, _) = read_type(&mut ctx)?;
        return read_value(&mut ctx, t, false);
    }

    // Versions 1–4: one buffer, strings then types follow the lanes.
    let mut rest = Lane { d: &raw1[off..] };
    let before = rest.d.len();
    strings = read_strings(&mut rest, count_strings)?;
    let strings_len = before - rest.d.len();
    let types_len = if version == 1 {
        size_uncompressed_total.saturating_sub(off + strings_len + 4)
    } else {
        count_types.saturating_sub(strings_len)
    };
    types = Lane { d: rest.take(types_len)? };
    let mut ctx = Ctx {
        version,
        types,
        object_lengths: Lane::default(),
        strings,
        buf: b1,
        aux: b1,
        use_aux_main: true,
        blobs: vec![],
        next_blob: 0,
    };
    let (t, _) = read_type(&mut ctx)?;
    read_value(&mut ctx, t, false)
}

fn read_type(c: &mut Ctx) -> Result<(u8, u8)> {
    let mut b = c.types.u8()?;
    let mut flag = 0u8;
    if c.version >= 3 {
        if b & 0x80 != 0 {
            flag = c.types.u8()?;
        }
        if b & 0x40 != 0 {
            c.types.u8()?;
        }
        b &= 0x3F;
    } else {
        if b & 0x80 != 0 {
            flag = c.types.u8()?;
        }
        b &= 0x7F;
    }
    Ok((b, flag))
}

fn string(c: &Ctx, id: i32) -> String {
    if id >= 0 && (id as usize) < c.strings.len() {
        c.strings[id as usize].clone()
    } else {
        String::new()
    }
}

/// Primitive values come from the main buffer, or from the auxiliary buffer
/// for elements of ARRAY_TYPE_AUXILIARY_BUFFER (`aux`).
fn read_value(c: &mut Ctx, t: u8, aux: bool) -> Result<Kv> {
    macro_rules! lane {
        ($l:ident) => {
            if aux && !c.use_aux_main {
                &mut c.aux.$l
            } else {
                &mut c.buf.$l
            }
        };
    }
    Ok(match t {
        1 => Kv::Null,
        13 => Kv::Bool(true),
        14 => Kv::Bool(false),
        15 => Kv::Int(0),
        16 => Kv::Int(1),
        17 => Kv::Float(0.0),
        18 => Kv::Float(1.0),
        26 => Kv::UInt(0),
        27 => Kv::UInt(1),
        2 => Kv::Bool(lane!(b1).u8()? != 0),
        22 => Kv::Int(lane!(b1).u8()? as i8 as i64),
        23 => Kv::UInt(lane!(b1).u8()? as u64),
        20 => Kv::Int(lane!(b2).u16()? as i16 as i64),
        21 => Kv::UInt(lane!(b2).u16()? as u64),
        11 => Kv::Int(lane!(b4).i32()? as i64),
        12 => Kv::UInt(lane!(b4).u32()? as u64),
        19 => Kv::Float(f32::from_bits(lane!(b4).u32()?) as f64),
        3 => Kv::Int(lane!(b8).u64()? as i64),
        4 => Kv::UInt(lane!(b8).u64()?),
        5 => Kv::Float(f64::from_bits(lane!(b8).u64()?)),
        6 => {
            let id = c.buf.b4.i32()?;
            Kv::Str(string(c, id))
        }
        7 if c.version < 2 => {
            let n = c.buf.b4.i32()?.max(0) as usize;
            Kv::Blob(c.buf.b1.take(n)?.to_vec())
        }
        7 => {
            let b = c.blobs.get(c.next_blob).cloned().unwrap_or_default();
            c.next_blob += 1;
            Kv::Blob(b)
        }
        8 => {
            let n = c.buf.b4.i32()?.max(0) as usize;
            let mut a = Vec::with_capacity(n.min(4096));
            for _ in 0..n {
                let (st, _) = read_type(c)?;
                a.push(read_value(c, st, false)?);
            }
            Kv::Array(a)
        }
        10 | 24 | 25 => {
            let n = if t == 10 { c.buf.b4.i32()?.max(0) as usize } else { c.buf.b1.u8()? as usize };
            let (st, _) = read_type(c)?;
            let sub_aux = t == 25;
            let mut a = Vec::with_capacity(n.min(4096));
            for _ in 0..n {
                a.push(read_value(c, st, sub_aux)?);
            }
            Kv::Array(a)
        }
        9 => {
            let n = if c.version >= 5 { c.object_lengths.i32()? } else { c.buf.b4.i32()? }.max(0) as usize;
            let mut m = Vec::with_capacity(n.min(4096));
            for _ in 0..n {
                let (st, _) = read_type(c)?;
                let key_id = c.buf.b4.i32()?;
                let key = string(c, key_id);
                let v = read_value(c, st, false)?;
                m.push((key, v));
            }
            Kv::Object(m)
        }
        other => return Err(bad(&format!("unknown node type {other}"))),
    })
}

/// Find a block (e.g. `DATA`) in a compiled Source 2 resource and return it.
pub fn resource_block<'a>(res: &'a [u8], kind: &[u8; 4]) -> Result<&'a [u8]> {
    let mut h = Lane { d: res };
    let _size = h.u32()?;
    let _hv = h.u16()?;
    let _ver = h.u16()?;
    let block_off = h.u32()? as usize;
    let count = h.u32()? as usize;
    let base = 8 + block_off;
    for i in 0..count {
        let o = base + i * 12;
        let entry = res.get(o..o + 12).ok_or_else(|| bad("bad block table"))?;
        let rel = u32::from_le_bytes(entry[4..8].try_into().unwrap()) as usize;
        let size = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
        if &entry[..4] == kind {
            let start = o + 4 + rel;
            return res.get(start..start + size).ok_or_else(|| bad("block outside file"));
        }
    }
    Err(bad(&format!("no {} block", String::from_utf8_lossy(kind))))
}

/// Parse a compiled resource's DATA block as KV3.
pub fn parse_resource(res: &[u8]) -> Result<Kv> {
    parse(resource_block(res, b"DATA")?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Seven's real sound events, extracted from Luke's game files.
    #[test]
    fn real_sound_events_parse() {
        let p = std::path::Path::new("/tmp/gigawatt.vsndevts_c");
        if !p.exists() {
            return;
        }
        let kv = parse_resource(&std::fs::read(p).unwrap()).unwrap();
        let events = kv.as_object().unwrap();
        assert!(events.len() > 20, "{} events", events.len());
        let mut all = vec![];
        kv.strings(&mut all);
        assert!(all.iter().any(|s| s.contains("storm_cloud")), "should reference storm cloud sounds");
        for (name, ev) in events.iter().take(400) {
            if let Some(files) = ev.get("vsnd_files") {
                let mut s = vec![];
                files.strings(&mut s);
                if name.to_lowercase().contains("storm") {
                    eprintln!("{name}: {s:?}");
                }
            }
        }
    }
}
