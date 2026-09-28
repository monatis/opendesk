//! On-disk store of trusted peers and default peer configuration.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub use super::identity::default_home;

pub const TRUSTED_PEERS_FILE: &str = "trusted-peers.json";
pub const DEFAULT_PEER_FILE: &str = "default-peer";
pub const DESCRIPTION_FILE: &str = "description.txt";

/// Eight colon-separated groups of four hex digits — matches Python's fingerprint function:
/// `":".join(h[i : i + 4] for i in range(0, 16, 4))`
pub fn fingerprint(public_key: &[u8]) -> String {
    let hex = data_encoding::HEXLOWER.encode(public_key);
    let mut parts = Vec::new();
    for i in (0..16).step_by(4) {
        if i + 4 <= hex.len() {
            parts.push(&hex[i..i + 4]);
        }
    }
    parts.join(":")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedPeer {
    pub public_key: String, // 64 hex chars
    #[serde(default)]
    pub name: String,
    #[serde(default = "now_ts")]
    pub paired_at: f64,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub description_override: String,
    #[serde(default)]
    pub last_host: String,
    #[serde(default)]
    pub last_port: u16,
    #[serde(default)]
    pub rendezvous_url: String,
}

fn now_ts() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

impl TrustedPeer {
    pub fn public_bytes(&self) -> Result<[u8; 32]> {
        let bytes = data_encoding::HEXLOWER
            .decode(self.public_key.as_bytes())
            .or_else(|_| data_encoding::HEXUPPER.decode(self.public_key.as_bytes()))
            .with_context(|| format!("invalid hex public key: {}", self.public_key))?;
        if bytes.len() != 32 {
            anyhow::bail!("public key must be 32 bytes, got {}", bytes.len());
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(arr)
    }

    pub fn fingerprint(&self) -> String {
        if let Ok(bytes) = self.public_bytes() {
            fingerprint(&bytes)
        } else {
            "invalid-key".to_string()
        }
    }

    pub fn effective_description(&self) -> &str {
        if !self.description_override.is_empty() {
            &self.description_override
        } else {
            &self.description
        }
    }
}

pub fn default_peer_name(public_key: &[u8; 32]) -> String {
    let hex = data_encoding::HEXLOWER.encode(public_key);
    format!("peer-{}", &hex[..6])
}

#[derive(Clone, Debug)]
pub struct TrustedPeers {
    home: PathBuf,
}

impl TrustedPeers {
    pub fn new(home: Option<&Path>) -> Self {
        let home = home.map(|p| p.to_path_buf()).unwrap_or_else(default_home);
        Self { home }
    }

    pub fn path(&self) -> PathBuf {
        self.home.join(TRUSTED_PEERS_FILE)
    }

    fn default_file(&self) -> PathBuf {
        self.home.join(DEFAULT_PEER_FILE)
    }

    pub fn list(&self) -> Vec<TrustedPeer> {
        let path = self.path();
        if !path.exists() {
            return Vec::new();
        }
        let data = match std::fs::read_to_string(&path) {
            Ok(d) => d,
            Err(_) => return Vec::new(),
        };
        serde_json::from_str(&data).unwrap_or_default()
    }

    pub fn save(&self, peers: &[TrustedPeer]) -> Result<()> {
        std::fs::create_dir_all(&self.home)
            .with_context(|| format!("failed to create dir {:?}", self.home))?;
        let json = serde_json::to_string_pretty(peers)?;
        std::fs::write(self.path(), json)
            .with_context(|| format!("failed to write {:?}", self.path()))?;
        Ok(())
    }

    pub fn contains(&self, public_key: &[u8; 32]) -> bool {
        let hex = data_encoding::HEXLOWER.encode(public_key);
        self.list().iter().any(|p| p.public_key.eq_ignore_ascii_case(&hex))
    }

    pub fn find(&self, public_key: &[u8; 32]) -> Option<TrustedPeer> {
        let hex = data_encoding::HEXLOWER.encode(public_key);
        self.list().into_iter().find(|p| p.public_key.eq_ignore_ascii_case(&hex))
    }

    pub fn find_by_name(&self, name: &str) -> Option<TrustedPeer> {
        self.list().into_iter().find(|p| p.name == name)
    }

    pub fn find_by_name_or_key(&self, query: &str) -> Option<TrustedPeer> {
        self.list().into_iter().find(|p| {
            p.name == query
                || p.public_key.eq_ignore_ascii_case(query)
                || p.fingerprint() == query
                || p.public_key.starts_with(query)
        })
    }

    pub fn add(
        &self,
        public_key: &[u8; 32],
        name: &str,
        rendezvous_url: &str,
    ) -> Result<TrustedPeer> {
        let hex_key = data_encoding::HEXLOWER.encode(public_key);
        let mut peers = self.list();

        for p in &mut peers {
            if p.public_key.eq_ignore_ascii_case(&hex_key) {
                if !name.is_empty() {
                    p.name = name.to_string();
                }
                if !rendezvous_url.is_empty() {
                    p.rendezvous_url = rendezvous_url.to_string();
                }
                let updated = p.clone();
                self.save(&peers)?;
                return Ok(updated);
            }
        }

        let peer = TrustedPeer {
            public_key: hex_key,
            name: if name.is_empty() {
                default_peer_name(public_key)
            } else {
                name.to_string()
            },
            paired_at: now_ts(),
            description: String::new(),
            description_override: String::new(),
            last_host: String::new(),
            last_port: 0,
            rendezvous_url: rendezvous_url.to_string(),
        };

        peers.push(peer.clone());
        self.save(&peers)?;
        Ok(peer)
    }

    pub fn remove(&self, public_key_or_name: &str) -> Result<bool> {
        let mut peers = self.list();
        let initial_len = peers.len();
        let target_names: Vec<String> = peers
            .iter()
            .filter(|p| {
                p.name == public_key_or_name
                    || p.public_key.eq_ignore_ascii_case(public_key_or_name)
                    || p.fingerprint() == public_key_or_name
            })
            .map(|p| p.name.clone())
            .collect();

        peers.retain(|p| {
            p.name != public_key_or_name
                && !p.public_key.eq_ignore_ascii_case(public_key_or_name)
                && p.fingerprint() != public_key_or_name
        });

        if peers.len() == initial_len {
            return Ok(false);
        }

        self.save(&peers)?;

        // Clear default if it was pointing to removed peer
        if let Some(def) = self.get_default()
            && target_names.contains(&def) {
                let _ = self.clear_default();
            }

        Ok(true)
    }

    pub fn rename(&self, public_key_or_name: &str, new_name: &str) -> Result<bool> {
        let mut peers = self.list();
        let mut found = false;
        for p in &mut peers {
            if p.name == public_key_or_name
                || p.public_key.eq_ignore_ascii_case(public_key_or_name)
                || p.fingerprint() == public_key_or_name
            {
                p.name = new_name.to_string();
                found = true;
                break;
            }
        }
        if found {
            self.save(&peers)?;
        }
        Ok(found)
    }

    pub fn cache_description(&self, public_key: &[u8; 32], description: &str) -> Result<bool> {
        let hex = data_encoding::HEXLOWER.encode(public_key);
        let mut peers = self.list();
        for p in &mut peers {
            if p.public_key.eq_ignore_ascii_case(&hex) && p.description != description {
                p.description = description.to_string();
                self.save(&peers)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn cache_endpoint(&self, public_key: &[u8; 32], host: &str, port: u16) -> Result<bool> {
        let hex = data_encoding::HEXLOWER.encode(public_key);
        let mut peers = self.list();
        for p in &mut peers {
            if p.public_key.eq_ignore_ascii_case(&hex) && (p.last_host != host || p.last_port != port) {
                p.last_host = host.to_string();
                p.last_port = port;
                self.save(&peers)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn cache_rendezvous(&self, public_key: &[u8; 32], url: &str) -> Result<bool> {
        let hex = data_encoding::HEXLOWER.encode(public_key);
        let mut peers = self.list();
        for p in &mut peers {
            if p.public_key.eq_ignore_ascii_case(&hex) && p.rendezvous_url != url {
                p.rendezvous_url = url.to_string();
                self.save(&peers)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn set_description_override(&self, name: &str, text: &str) -> Result<bool> {
        let mut peers = self.list();
        for p in &mut peers {
            if p.name == name {
                p.description_override = text.to_string();
                self.save(&peers)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn clear_description_override(&self, name: &str) -> Result<bool> {
        self.set_description_override(name, "")
    }

    pub fn get_default(&self) -> Option<String> {
        let file = self.default_file();
        if !file.exists() {
            return None;
        }
        let name = std::fs::read_to_string(&file).ok()?.trim().to_string();
        if name.is_empty() {
            return None;
        }
        if self.find_by_name(&name).is_some() {
            Some(name)
        } else {
            None
        }
    }

    pub fn set_default(&self, name: &str) -> Result<bool> {
        if self.find_by_name(name).is_none() {
            return Ok(false);
        }
        std::fs::create_dir_all(&self.home)?;
        std::fs::write(self.default_file(), format!("{}\n", name))?;
        Ok(true)
    }

    pub fn clear_default(&self) -> Result<bool> {
        let file = self.default_file();
        if file.exists() {
            std::fs::remove_file(file)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

pub fn read_description(home: Option<&Path>) -> String {
    let home = home.map(|p| p.to_path_buf()).unwrap_or_else(default_home);
    let path = home.join(DESCRIPTION_FILE);
    if path.exists() {
        std::fs::read_to_string(path).unwrap_or_default().trim().to_string()
    } else {
        String::new()
    }
}

pub fn write_description(home: Option<&Path>, text: &str) -> Result<()> {
    let home = home.map(|p| p.to_path_buf()).unwrap_or_else(default_home);
    std::fs::create_dir_all(&home)?;
    let path = home.join(DESCRIPTION_FILE);
    std::fs::write(path, format!("{}\n", text.trim()))?;
    Ok(())
}

pub fn clear_description(home: Option<&Path>) -> Result<bool> {
    let home = home.map(|p| p.to_path_buf()).unwrap_or_else(default_home);
    let path = home.join(DESCRIPTION_FILE);
    if path.exists() {
        std::fs::remove_file(path)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fingerprint() {
        let key = [0x12u8; 32];
        let fp = fingerprint(&key);
        assert_eq!(fp, "1212:1212:1212:1212");
    }
}
