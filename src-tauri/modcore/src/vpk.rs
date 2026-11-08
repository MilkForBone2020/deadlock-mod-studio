//! VPK (v1 / v2) reading and writing.
//!
//! Reading: the directory tree, plus file contents from either the dir file
//! itself or numbered archives (`pak01_000.vpk` ...), as Deadlock ships them.
//! Writing: single-file v2 VPKs with all data inline and the same checksum
//! section community mod VPKs use (tree MD5, archive-MD5-section MD5, whole
//! file MD5), so the game treats them like any other addon.

use crate::{Error, IoCtx, Result};
use md5::{Digest, Md5};
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub const SIGNATURE: u32 = 0x55AA_1234;
/// Archive index meaning "the data is in the dir file itself".
pub const INLINE: u16 = 0x7FFF;

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.data.len() {
            return Err(Error::msg("VPK directory ends early (file is damaged or not a VPK)"));
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn cstr(&mut self) -> Result<String> {
        let rest = &self.data[self.pos..];
        let end = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| Error::msg("VPK directory string is not terminated"))?;
        let s = String::from_utf8_lossy(&rest[..end]).into_owned();
        self.pos += end + 1;
        Ok(s)
    }
}

/// One file inside a VPK.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Lower-case internal path, e.g. `sounds/abilities/x.vsnd_c`.
    pub path: String,
    pub crc: u32,
    pub preload: Vec<u8>,
    pub archive_index: u16,
    pub offset: u32,
    pub length: u32,
}

impl Entry {
    pub fn size(&self) -> u64 {
        self.preload.len() as u64 + self.length as u64
    }
}

/// A parsed VPK directory.
#[derive(Debug)]
pub struct Directory {
    pub dir_path: PathBuf,
    pub version: u32,
    /// Byte offset in the dir file where inline file data starts.
    pub data_start: u64,
    pub entries: Vec<Entry>,
}

/// Quick check of the 4-byte signature.
pub fn is_vpk(path: &Path) -> Result<bool> {
    let mut f = std::fs::File::open(path).at(path)?;
    let mut buf = [0u8; 4];
    if f.read(&mut buf).at(path)? < 4 {
        return Ok(false);
    }
    Ok(u32::from_le_bytes(buf) == SIGNATURE)
}

/// Read only the header and tree of a VPK (not the file data).
pub fn read_directory(path: &Path) -> Result<Directory> {
    let mut f = std::fs::File::open(path).at(path)?;
    let mut head = [0u8; 12];
    f.read_exact(&mut head).at(path)?;
    let tree_size = u32::from_le_bytes([head[8], head[9], head[10], head[11]]) as usize;
    let version = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);
    let header_len = if version == 2 { 28 } else { 12 };
    let mut buf = vec![0u8; header_len + tree_size];
    f.seek(SeekFrom::Start(0)).at(path)?;
    f.read_exact(&mut buf).map_err(|_| Error::msg("VPK directory is larger than the file"))?;
    let mut d = parse_directory(&buf)?;
    d.dir_path = path.to_path_buf();
    Ok(d)
}

/// Parse a VPK held in memory (header + tree, data optional).
pub fn parse_directory(data: &[u8]) -> Result<Directory> {
    let mut c = Cursor { data, pos: 0 };
    if c.u32()? != SIGNATURE {
        return Err(Error::msg("Not a VPK file (wrong signature)"));
    }
    let version = c.u32()?;
    let tree_size = c.u32()? as usize;
    match version {
        1 => {}
        2 => {
            c.take(16)?;
        }
        v => return Err(Error::msg(format!("Unsupported VPK version {v}"))),
    }
    let tree_start = c.pos;
    let tree_end = tree_start + tree_size;
    if tree_end > data.len() {
        return Err(Error::msg("VPK directory is larger than the file"));
    }

    let mut entries = Vec::new();
    loop {
        let ext = c.cstr()?;
        if ext.is_empty() {
            break;
        }
        loop {
            let dir = c.cstr()?;
            if dir.is_empty() {
                break;
            }
            loop {
                let name = c.cstr()?;
                if name.is_empty() {
                    break;
                }
                let crc = c.u32()?;
                let preload_len = c.u16()? as usize;
                let archive_index = c.u16()?;
                let offset = c.u32()?;
                let length = c.u32()?;
                if c.u16()? != 0xFFFF {
                    return Err(Error::msg("VPK entry is malformed"));
                }
                let preload = c.take(preload_len)?.to_vec();
                let mut full = String::new();
                if dir != " " {
                    full.push_str(&dir);
                    full.push('/');
                }
                full.push_str(&name);
                if ext != " " {
                    full.push('.');
                    full.push_str(&ext);
                }
                entries.push(Entry { path: full.to_lowercase(), crc, preload, archive_index, offset, length });
            }
        }
        if c.pos > tree_end {
            return Err(Error::msg("VPK directory overruns its declared size"));
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Directory { dir_path: PathBuf::new(), version, data_start: tree_end as u64, entries })
}

/// List every internal file path in a VPK, e.g. `sounds/abilities/x.vsnd_c`.
pub fn list_files(path: &Path) -> Result<Vec<String>> {
    Ok(read_directory(path)?.entries.into_iter().map(|e| e.path).collect())
}

pub fn list_files_from_bytes(data: &[u8]) -> Result<Vec<String>> {
    Ok(parse_directory(data)?.entries.into_iter().map(|e| e.path).collect())
}

impl Directory {
    pub fn find(&self, path: &str) -> Option<&Entry> {
        let p = path.to_lowercase().replace('\\', "/");
        self.entries.binary_search_by(|e| e.path.as_str().cmp(&p)).ok().map(|i| &self.entries[i])
    }

    /// Path of numbered archive `n` next to a `*_dir.vpk`.
    fn archive_path(&self, n: u16) -> Result<PathBuf> {
        let name = self.dir_path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let stem = name
            .strip_suffix("_dir.vpk")
            .ok_or_else(|| Error::msg("This VPK refers to archives but isn't named *_dir.vpk"))?;
        Ok(self.dir_path.with_file_name(format!("{stem}_{n:03}.vpk")))
    }

    /// Read one file's full contents.
    pub fn extract(&self, e: &Entry) -> Result<Vec<u8>> {
        let mut out = e.preload.clone();
        if e.length > 0 {
            let (path, start) = if e.archive_index == INLINE {
                (self.dir_path.clone(), self.data_start + e.offset as u64)
            } else {
                (self.archive_path(e.archive_index)?, e.offset as u64)
            };
            let mut f = std::fs::File::open(&path).at(&path)?;
            f.seek(SeekFrom::Start(start)).at(&path)?;
            let mut buf = vec![0u8; e.length as usize];
            f.read_exact(&mut buf).at(&path)?;
            out.extend_from_slice(&buf);
        }
        if crc32fast::hash(&out) != e.crc && e.crc != 0 {
            return Err(Error::msg(format!("{} failed its checksum (the game files may need verifying)", e.path)));
        }
        Ok(out)
    }
}

/// Write a single-file v2 VPK containing `files` (internal path -> bytes).
pub fn write_vpk(out: &Path, files: &[(String, Vec<u8>)]) -> Result<()> {
    let bytes = build_vpk(files)?;
    let tmp = out.with_extension("vpk.tmp");
    std::fs::write(&tmp, bytes).at(&tmp)?;
    std::fs::rename(&tmp, out).at(out)?;
    Ok(())
}

/// Build a v2 VPK in memory.
pub fn build_vpk(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    // ext -> dir -> [(name, file index)]
    let mut tree: BTreeMap<String, BTreeMap<String, Vec<(String, usize)>>> = BTreeMap::new();
    for (i, (path, _)) in files.iter().enumerate() {
        let p = path.to_lowercase().replace('\\', "/");
        let p = p.trim_start_matches('/');
        if p.is_empty() {
            return Err(Error::msg("Empty path in VPK"));
        }
        let (dir, file) = match p.rfind('/') {
            Some(k) => (p[..k].to_string(), &p[k + 1..]),
            None => (" ".to_string(), p),
        };
        let (name, ext) = match file.rfind('.') {
            Some(k) => (file[..k].to_string(), file[k + 1..].to_string()),
            None => (file.to_string(), " ".to_string()),
        };
        tree.entry(ext).or_default().entry(dir).or_default().push((name, i));
    }

    let mut t = Vec::new();
    let mut data = Vec::new();
    for (ext, dirs) in &tree {
        t.extend(ext.as_bytes());
        t.push(0);
        for (dir, names) in dirs {
            t.extend(dir.as_bytes());
            t.push(0);
            for (name, i) in names {
                let content = &files[*i].1;
                if data.len() + content.len() > u32::MAX as usize {
                    return Err(Error::msg("Mod is too large for a single VPK (over 4 GB)"));
                }
                t.extend(name.as_bytes());
                t.push(0);
                t.extend(crc32fast::hash(content).to_le_bytes());
                t.extend(0u16.to_le_bytes()); // no preload
                t.extend(INLINE.to_le_bytes());
                t.extend((data.len() as u32).to_le_bytes());
                t.extend((content.len() as u32).to_le_bytes());
                t.extend(0xFFFFu16.to_le_bytes());
                data.extend_from_slice(content);
            }
            t.push(0);
        }
        t.push(0);
    }
    t.push(0);

    let mut out = Vec::with_capacity(28 + t.len() + data.len() + 48);
    out.extend(SIGNATURE.to_le_bytes());
    out.extend(2u32.to_le_bytes());
    out.extend((t.len() as u32).to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes()); // file data section size
    out.extend(0u32.to_le_bytes()); // archive MD5 section size (no archives)
    out.extend(48u32.to_le_bytes()); // other MD5 section size
    out.extend(0u32.to_le_bytes()); // signature section size
    out.extend(&t);
    out.extend(&data);
    // Other MD5 section: tree, archive-MD5 section (empty), whole file so far.
    let tree_md5 = Md5::digest(&t);
    let archive_md5 = Md5::digest([]);
    out.extend(tree_md5);
    out.extend(archive_md5);
    let whole = Md5::digest(&out);
    out.extend(whole);
    Ok(out)
}

/// Check the three MD5s at the end of a single-file v2 VPK. Used in tests
/// against real community VPKs to prove our layout matches theirs.
pub fn verify_md5s(data: &[u8]) -> Result<bool> {
    let mut c = Cursor { data, pos: 0 };
    if c.u32()? != SIGNATURE || c.u32()? != 2 {
        return Err(Error::msg("Not a v2 VPK"));
    }
    let tree = c.u32()? as usize;
    let file_data = c.u32()? as usize;
    let archive_md5 = c.u32()? as usize;
    let other = c.u32()? as usize;
    if other != 48 {
        return Err(Error::msg("No checksum section"));
    }
    let tree_start = 28;
    let am_start = tree_start + tree + file_data;
    let other_start = am_start + archive_md5;
    if other_start + 48 > data.len() {
        return Err(Error::msg("VPK is truncated"));
    }
    let ok_tree = Md5::digest(&data[tree_start..tree_start + tree])[..] == data[other_start..other_start + 16];
    let ok_am = Md5::digest(&data[am_start..other_start])[..] == data[other_start + 16..other_start + 32];
    let ok_whole = Md5::digest(&data[..other_start + 32])[..] == data[other_start + 32..other_start + 48];
    Ok(ok_tree && ok_am && ok_whole)
}

/// Build a tiny VPK in memory. Used by tests here and elsewhere.
#[doc(hidden)]
pub fn build_test_vpk(paths: &[&str]) -> Vec<u8> {
    let files: Vec<(String, Vec<u8>)> =
        paths.iter().map(|p| (p.to_string(), format!("data for {p}").into_bytes())).collect();
    build_vpk(&files).expect("test vpk")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_paths_and_data() {
        let files = vec![
            ("sounds/abilities/gigawatt/storm_cloud_cast.vsnd_c".to_string(), b"cast".to_vec()),
            ("sounds/abilities/gigawatt/storm_cloud_lp.vsnd_c".to_string(), vec![7u8; 5000]),
            ("readme".to_string(), b"hi".to_vec()),
            ("Models/Heroes/X.vmdl_c".to_string(), vec![]),
        ];
        let bytes = build_vpk(&files).unwrap();
        assert!(verify_md5s(&bytes).unwrap());
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("pak01_dir.vpk");
        std::fs::write(&p, &bytes).unwrap();
        let d = read_directory(&p).unwrap();
        let paths: Vec<&str> = d.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "models/heroes/x.vmdl_c",
                "readme",
                "sounds/abilities/gigawatt/storm_cloud_cast.vsnd_c",
                "sounds/abilities/gigawatt/storm_cloud_lp.vsnd_c",
            ]
        );
        let lp = d.find("sounds/abilities/gigawatt/storm_cloud_lp.vsnd_c").unwrap();
        assert_eq!(d.extract(lp).unwrap(), vec![7u8; 5000]);
        assert_eq!(d.extract(d.find("README").unwrap()).unwrap(), b"hi");
    }

    #[test]
    fn rejects_non_vpk() {
        assert!(list_files_from_bytes(b"PK\x03\x04hello").is_err());
        assert!(list_files_from_bytes(&[0x34, 0x12, 0xAA, 0x55, 9, 0, 0, 0, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn rejects_truncated() {
        let bytes = build_test_vpk(&["a/b.txt"]);
        let tree = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
        assert!(list_files_from_bytes(&bytes[..28 + tree - 3]).is_err());
    }

    #[test]
    fn detects_corrupt_data() {
        let mut bytes = build_vpk(&[("a.txt".into(), b"hello".to_vec())]).unwrap();
        let tree = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
        bytes[28 + tree] ^= 0xFF;
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("x.vpk");
        std::fs::write(&p, &bytes).unwrap();
        let d = read_directory(&p).unwrap();
        assert!(d.extract(&d.entries[0]).is_err());
    }

    /// Real files from Luke's PC, only present on the dev machine.
    #[test]
    fn real_community_vpk_layout_matches() {
        let p = Path::new("/mnt/user-data/uploads/citadel/addons/snd-85838_pak01_dir.vpk");
        if !p.exists() {
            return;
        }
        let bytes = std::fs::read(p).unwrap();
        assert!(verify_md5s(&bytes).unwrap(), "our MD5 layout must match community VPKs");
        let d = read_directory(p).unwrap();
        // Re-pack its contents with our writer and read them back identically.
        let files: Vec<(String, Vec<u8>)> =
            d.entries.iter().map(|e| (e.path.clone(), d.extract(e).unwrap())).collect();
        let ours = build_vpk(&files).unwrap();
        assert!(verify_md5s(&ours).unwrap());
        assert_eq!(list_files_from_bytes(&ours).unwrap(), d.entries.iter().map(|e| e.path.clone()).collect::<Vec<_>>());
    }
}
