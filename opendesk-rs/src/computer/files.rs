//! Chunked file transfer primitives, run on the machine that owns the file.
//!
//! Writes go to `<path>.opendesk-part` and only replace `<path>` once the
//! whole file's SHA-256 matches what the sender computed, so an interrupted
//! or corrupted transfer never leaves a half-written file in place.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Bytes per read/write RPC.
pub const CHUNK: usize = 1024 * 1024;
const PART_SUFFIX: &str = ".opendesk-part";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FileStat {
    pub path: String,
    pub exists: bool,
    pub is_dir: bool,
    pub size: u64,
    /// Lowercase hex SHA-256 of the contents (files only).
    pub sha256: String,
}

/// Expand a leading `~` to this machine's home directory.
pub fn expand(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"))
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    if path == "~"
        && let Some(home) = dirs::home_dir()
    {
        return home;
    }
    PathBuf::from(path)
}

fn part_path(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(PART_SUFFIX);
    PathBuf::from(p)
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(data_encoding::HEXLOWER.encode(&h.finalize()))
}

pub fn stat(path: &str) -> Result<FileStat> {
    let p = expand(path);
    let Ok(meta) = std::fs::metadata(&p) else {
        return Ok(FileStat {
            path: p.display().to_string(),
            ..Default::default()
        });
    };
    Ok(FileStat {
        path: p.display().to_string(),
        exists: true,
        is_dir: meta.is_dir(),
        size: meta.len(),
        sha256: if meta.is_file() {
            sha256_file(&p)?
        } else {
            String::new()
        },
    })
}

/// Up to `len` (at most [`CHUNK`]) bytes of `path` starting at `offset`.
pub fn read_chunk(path: &str, offset: u64, len: usize) -> Result<Vec<u8>> {
    let p = expand(path);
    let mut f = std::fs::File::open(&p).with_context(|| format!("open {}", p.display()))?;
    f.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; len.min(CHUNK)];
    let mut filled = 0;
    while filled < buf.len() {
        let n = f.read(&mut buf[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    buf.truncate(filled);
    Ok(buf)
}

/// Write `data` at `offset` of the pending `<path>.opendesk-part`. Offset 0
/// starts a new transfer; any other offset must equal the bytes written so
/// far, so chunks cannot be lost or reordered silently.
pub fn write_chunk(path: &str, offset: u64, data: &[u8]) -> Result<u64> {
    let p = expand(path);
    let part = part_path(&p);
    if offset == 0 {
        if let Some(parent) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        std::fs::File::create(&part).with_context(|| format!("create {}", part.display()))?;
    }
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&part)
        .with_context(|| format!("open {}", part.display()))?;
    let have = f.metadata()?.len();
    if have != offset {
        bail!(
            "out-of-order chunk for {}: have {have} bytes, got offset {offset}",
            p.display()
        );
    }
    f.write_all(data)?;
    Ok(have + data.len() as u64)
}

/// Verify the pending file against `sha256` and move it into place.
pub fn commit(path: &str, sha256: &str) -> Result<FileStat> {
    let p = expand(path);
    let part = part_path(&p);
    let got = sha256_file(&part)?;
    if !got.eq_ignore_ascii_case(sha256) {
        let _ = std::fs::remove_file(&part);
        bail!(
            "checksum mismatch for {}: expected {sha256}, got {got}; transfer discarded",
            p.display()
        );
    }
    // A rename over an existing file is atomic on the same volume; on
    // Windows it fails if the target is open, which is reported.
    std::fs::rename(&part, &p)
        .with_context(|| format!("replace {} (is it open in another program?)", p.display()))?;
    stat(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunked_write_commit_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("sub").join("out.bin");
        let t = target.to_str().unwrap();
        let data: Vec<u8> = (0..(CHUNK * 2 + 123)).map(|i| (i % 251) as u8).collect();
        let sha = data_encoding::HEXLOWER.encode(&Sha256::digest(&data));

        let mut off = 0u64;
        for chunk in data.chunks(CHUNK) {
            off = write_chunk(t, off, chunk).unwrap();
        }
        assert!(
            !target.exists(),
            "nothing replaces the target before commit"
        );
        let st = commit(t, &sha).unwrap();
        assert_eq!(
            (st.size, st.sha256.as_str()),
            (data.len() as u64, sha.as_str())
        );

        let mut back = Vec::new();
        let mut off = 0u64;
        loop {
            let c = read_chunk(t, off, CHUNK).unwrap();
            if c.is_empty() {
                break;
            }
            off += c.len() as u64;
            back.extend(c);
        }
        assert_eq!(back, data);
    }

    #[test]
    fn a_bad_checksum_keeps_the_old_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("keep.txt");
        std::fs::write(&target, b"original").unwrap();
        let t = target.to_str().unwrap();
        write_chunk(t, 0, b"replacement").unwrap();
        let err = commit(t, "00").unwrap_err().to_string();
        assert!(err.contains("checksum mismatch"), "{err}");
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        assert!(!part_path(&target).exists());
    }

    #[test]
    fn out_of_order_chunks_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let t = dir.path().join("x").to_str().unwrap().to_string();
        write_chunk(&t, 0, b"abc").unwrap();
        assert!(write_chunk(&t, 10, b"def").is_err());
    }

    #[test]
    fn stat_of_missing_file() {
        let st = stat("definitely-missing-opendesk-file.bin").unwrap();
        assert!(!st.exists);
    }
}
