use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use thiserror::Error;
use x25519_dalek::{PublicKey, StaticSecret};

#[derive(Error, Debug)]
pub enum CryptoError {
    #[error("Decryption failed: invalid tag or ciphertext")]
    DecryptionFailed,
    #[error("Nonce counter exhausted")]
    NonceExhausted,
}

pub struct KeyPair {
    pub secret: StaticSecret,
    pub public: PublicKey,
}

impl KeyPair {
    pub fn generate() -> Self {
        let rng = rand::rngs::OsRng;
        let secret = StaticSecret::random_from_rng(rng);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }
}

pub fn make_nonce(counter: u64) -> Nonce {
    let mut nonce = [0u8; 12];
    nonce[4..12].copy_from_slice(&counter.to_be_bytes());
    *Nonce::from_slice(&nonce)
}

pub struct EncryptedChannel {
    send_cipher: ChaCha20Poly1305,
    recv_cipher: ChaCha20Poly1305,
    send_counter: u64,
    recv_counter: u64,
}

impl EncryptedChannel {
    pub fn new(send_key: &[u8; 32], recv_key: &[u8; 32]) -> Self {
        Self {
            send_cipher: ChaCha20Poly1305::new(send_key.into()),
            recv_cipher: ChaCha20Poly1305::new(recv_key.into()),
            send_counter: 0,
            recv_counter: 0,
        }
    }

    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let nonce = make_nonce(self.send_counter);
        self.send_counter = self
            .send_counter
            .checked_add(1)
            .ok_or(CryptoError::NonceExhausted)?;

        self.send_cipher
            .encrypt(&nonce, plaintext)
            .map_err(|_| CryptoError::DecryptionFailed)
    }

    pub fn decrypt(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let nonce = make_nonce(self.recv_counter);
        let pt = self
            .recv_cipher
            .decrypt(&nonce, ciphertext)
            .map_err(|_| CryptoError::DecryptionFailed)?;

        self.recv_counter = self
            .recv_counter
            .checked_add(1)
            .ok_or(CryptoError::NonceExhausted)?;

        Ok(pt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_python_crypto_interop() {
        let key = [b'k'; 32];
        let mut channel = EncryptedChannel::new(&key, &key);

        // Ciphertext from Python's ChaCha20Poly1305
        let hex_ct = "c2f1d99b3a86313e83c55541fbfd9c703b42768364e232bcd937ebd35d2f";
        let ct: Vec<u8> = (0..hex_ct.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex_ct[i..i + 2], 16).unwrap())
            .collect();

        let pt = channel.decrypt(&ct).expect("failed to decrypt Python ciphertext");
        assert_eq!(pt, b"hello opendesk");
    }

    #[test]
    fn test_channel_bidirectional_flow() {
        let k1 = [1u8; 32];
        let k2 = [2u8; 32];

        let mut client = EncryptedChannel::new(&k1, &k2);
        let mut server = EncryptedChannel::new(&k2, &k1);

        let ct1 = client.encrypt(b"request-payload-1").unwrap();
        let pt1 = server.decrypt(&ct1).unwrap();
        assert_eq!(pt1, b"request-payload-1");

        let ct2 = server.encrypt(b"response-payload-1").unwrap();
        let pt2 = client.decrypt(&ct2).unwrap();
        assert_eq!(pt2, b"response-payload-1");
    }
}
