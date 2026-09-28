//! Long-lived peer identity — an X25519 keypair persisted on disk.

use anyhow::{Context, Result, anyhow};
use rand::Rng;
use std::path::{Path, PathBuf};
use x25519_dalek::{PublicKey, StaticSecret};

pub const IDENTITY_FILE: &str = "identity.key";

pub fn default_home() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".opendesk"))
        .unwrap_or_else(|| PathBuf::from(".opendesk"))
}

#[derive(Clone)]
pub struct Identity {
    secret: StaticSecret,
    public: PublicKey,
}

impl Identity {
    pub fn generate() -> Self {
        let secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }

    pub fn from_private_bytes(bytes: &[u8; 32]) -> Self {
        let secret = StaticSecret::from(*bytes);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }

    pub fn public_bytes(&self) -> [u8; 32] {
        *self.public.as_bytes()
    }

    pub fn private_bytes(&self) -> [u8; 32] {
        self.secret.to_bytes()
    }

    pub fn diffie_hellman(&self, peer_public: &[u8; 32]) -> [u8; 32] {
        let peer_pub = PublicKey::from(*peer_public);
        *self.secret.diffie_hellman(&peer_pub).as_bytes()
    }

    pub fn load_or_create(home: Option<&Path>) -> Result<Self> {
        let home_dir = home.map(|p| p.to_path_buf()).unwrap_or_else(default_home);
        let key_path = home_dir.join(IDENTITY_FILE);

        if key_path.exists() {
            let data = std::fs::read(&key_path)
                .with_context(|| format!("failed to read identity key at {:?}", key_path))?;
            if data.len() != 32 {
                return Err(anyhow!(
                    "identity key at {:?} must be 32 bytes, got {}",
                    key_path,
                    data.len()
                ));
            }
            let mut bytes = [0u8; 32];
            bytes.copy_from_slice(&data);
            return Ok(Self::from_private_bytes(&bytes));
        }

        std::fs::create_dir_all(&home_dir)
            .with_context(|| format!("failed to create directory {:?}", home_dir))?;

        let identity = Self::generate();
        identity.save(Some(&home_dir))?;
        Ok(identity)
    }

    pub fn save(&self, home: Option<&Path>) -> Result<()> {
        let home_dir = home.map(|p| p.to_path_buf()).unwrap_or_else(default_home);
        std::fs::create_dir_all(&home_dir)
            .with_context(|| format!("failed to create directory {:?}", home_dir))?;
        let key_path = home_dir.join(IDENTITY_FILE);
        std::fs::write(&key_path, self.private_bytes())
            .with_context(|| format!("failed to write identity key to {:?}", key_path))?;
        Ok(())
    }
}

/// Return a fresh numeric pairing code as a zero-padded string (default 6 digits).
pub fn generate_pairing_code(digits: usize) -> String {
    let mut rng = rand::thread_rng();
    let max_val = 10u32.pow(digits as u32);
    let n = rng.gen_range(0..max_val);
    format!("{:0width$}", n, width = digits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_generate_and_dh() {
        let id1 = Identity::generate();
        let id2 = Identity::generate();

        let ss1 = id1.diffie_hellman(&id2.public_bytes());
        let ss2 = id2.diffie_hellman(&id1.public_bytes());

        assert_eq!(ss1, ss2);
    }

    #[test]
    fn test_generate_pairing_code() {
        let code = generate_pairing_code(6);
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }
}
