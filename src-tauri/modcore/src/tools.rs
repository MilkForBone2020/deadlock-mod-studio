//! Outside tools the app uses, installed into its data folder.
//!
//! Source 2 Viewer's command-line tool (MIT licence) is downloaded from its
//! official GitHub release on first use. The app pins one version and
//! checks the download's SHA-256 before unpacking anything.

use crate::model::{S2V_EXE, S2V_SHA256, S2V_VERSION};
use crate::{Error, IoCtx, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub fn s2v_dir(tools: &Path) -> PathBuf {
    tools.join(format!("source2viewer-{S2V_VERSION}"))
}

/// The installed tool, if it's there.
pub fn s2v_exe(tools: &Path) -> Option<PathBuf> {
    let p = s2v_dir(tools).join(S2V_EXE);
    p.is_file().then_some(p)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Check and unpack a downloaded release zip. Returns the exe's path.
pub fn install_s2v(zip_bytes: &[u8], tools: &Path) -> Result<PathBuf> {
    install_zip(zip_bytes, S2V_SHA256, &s2v_dir(tools), S2V_EXE)
}

fn install_zip(zip_bytes: &[u8], sha: &str, dest: &Path, exe: &str) -> Result<PathBuf> {
    let got = sha256_hex(zip_bytes);
    if got != sha {
        return Err(Error::msg(format!(
            "The Source 2 Viewer download didn't match its expected checksum, so it wasn't installed.\nExpected {sha}\nGot      {got}"
        )));
    }
    let tmp = dest.with_extension("partial");
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp).at(&tmp)?;
    }
    std::fs::create_dir_all(&tmp).at(&tmp)?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|e| Error::msg(format!("Bad zip: {e}")))?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| Error::msg(format!("Bad zip: {e}")))?;
        let Some(rel) = f.enclosed_name() else { continue };
        let out = tmp.join(rel);
        if f.is_dir() {
            std::fs::create_dir_all(&out).at(&out)?;
            continue;
        }
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p).at(p)?;
        }
        let mut w = std::fs::File::create(&out).at(&out)?;
        std::io::copy(&mut f, &mut w).at(&out)?;
    }
    if !tmp.join(exe).is_file() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(Error::msg(format!("The download didn't contain {exe}")));
    }
    if dest.exists() {
        std::fs::remove_dir_all(dest).at(dest)?;
    }
    std::fs::rename(&tmp, dest).at(dest)?;
    Ok(dest.join(exe))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            for (name, data) in files {
                z.start_file(*name, opts).unwrap();
                z.write_all(data).unwrap();
            }
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn installs_only_a_matching_download() {
        let t = tempfile::tempdir().unwrap();
        let zip = make_zip(&[("tool.exe", b"exe"), ("lib.dll", b"dll"), ("../evil.txt", b"x")]);
        let dest = t.path().join("tools").join("x-1");
        // Wrong checksum: nothing installed.
        assert!(install_zip(&zip, "00", &dest, "tool.exe").unwrap_err().to_string().contains("checksum"));
        assert!(!dest.exists());
        // Right checksum.
        let exe = install_zip(&zip, &sha256_hex(&zip), &dest, "tool.exe").unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"exe");
        assert!(dest.join("lib.dll").exists());
        assert!(!t.path().join("tools").join("evil.txt").exists(), "paths outside the folder are skipped");
        // Missing exe.
        let bad = make_zip(&[("other.exe", b"x")]);
        assert!(install_zip(&bad, &sha256_hex(&bad), &dest, "tool.exe").is_err());
        assert!(dest.join("tool.exe").exists(), "a failed install leaves the old one");
    }

    #[test]
    fn pinned_release_checksum_looks_right() {
        assert_eq!(S2V_SHA256.len(), 64);
        assert!(crate::model::S2V_URL.contains(S2V_VERSION));
    }
}
