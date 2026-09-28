//! Mutual-authentication handshake implementation.
//!
//! Two modes:
//! 1. Pairing (pair_server / pair_client): 3-message PSK-authenticated flow.
//! 2. Reconnect (auth_server / auth_client): 2-message static-key flow.

use anyhow::{Context, Result, anyhow};
use chacha20poly1305::{
    ChaCha20Poly1305, Nonce,
    aead::{Aead, KeyInit},
};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use serde_bytes::ByteBuf;
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

use super::crypto::EncryptedChannel;
use super::identity::Identity;
use super::storage::TrustedPeers;

pub const HANDSHAKE_VERSION: u64 = 1;
pub const PSK_ITERATIONS: u32 = 200_000;
pub const PSK_SALT: &[u8] = b"opendesk-psk-v1";

#[allow(async_fn_in_trait)]
pub trait Transport {
    async fn send(&mut self, data: &[u8]) -> Result<()>;
    async fn recv(&mut self) -> Result<Vec<u8>>;
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HandshakeMessage {
    pub v: u64,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub e: Option<ByteBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ct: Option<ByteBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s: Option<ByteBuf>,
}

pub struct Session {
    pub channel: EncryptedChannel,
    pub peer_public: [u8; 32],
    pub is_pairing: bool,
}

pub fn derive_psk(code: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(code.as_bytes(), PSK_SALT, PSK_ITERATIONS, &mut out);
    out
}

pub fn hkdf_sha256(salt: &[u8], ikm: &[u8], info: &[u8], length: usize) -> Vec<u8> {
    let hk = Hkdf::<Sha256>::new(Some(salt), ikm);
    let mut okm = vec![0u8; length];
    hk.expand(info, &mut okm)
        .expect("valid length for hkdf expansion");
    okm
}

fn gen_ephemeral() -> (StaticSecret, [u8; 32]) {
    let eph = StaticSecret::random_from_rng(rand::rngs::OsRng);
    let eph_pub = *PublicKey::from(&eph).as_bytes();
    (eph, eph_pub)
}

fn dh(private: &StaticSecret, peer_pub_bytes: &[u8; 32]) -> [u8; 32] {
    let peer_pub = PublicKey::from(*peer_pub_bytes);
    *private.diffie_hellman(&peer_pub).as_bytes()
}

fn session_keys(
    is_server: bool,
    transcript: &[u8],
    dh_chain: &[[u8; 32]],
    psk: Option<&[u8]>,
) -> ([u8; 32], [u8; 32]) {
    let mut ikm = Vec::new();
    for d in dh_chain {
        ikm.extend_from_slice(d);
    }
    if let Some(p) = psk {
        ikm.extend_from_slice(p);
    }

    let keys = hkdf_sha256(transcript, &ikm, b"opendesk-session-keys", 64);
    let mut c2s = [0u8; 32];
    let mut s2c = [0u8; 32];
    c2s.copy_from_slice(&keys[0..32]);
    s2c.copy_from_slice(&keys[32..64]);

    if is_server { (s2c, c2s) } else { (c2s, s2c) }
}

async fn send_msg<T: Transport + ?Sized>(conn: &mut T, msg: HandshakeMessage) -> Result<()> {
    let packed = rmp_serde::to_vec_named(&msg)
        .with_context(|| format!("failed to serialize handshake message {:?}", msg.kind))?;
    conn.send(&packed).await
}

async fn recv_msg<T: Transport + ?Sized>(
    conn: &mut T,
    expected_kind: &str,
) -> Result<HandshakeMessage> {
    let data = conn.recv().await?;
    let msg: HandshakeMessage = rmp_serde::from_slice(&data)
        .with_context(|| format!("malformed handshake message; expected {}", expected_kind))?;

    if msg.v != HANDSHAKE_VERSION {
        return Err(anyhow!(
            "unsupported handshake version {}; expected {}",
            msg.v,
            HANDSHAKE_VERSION
        ));
    }
    if msg.kind != expected_kind {
        return Err(anyhow!(
            "expected handshake kind '{}', got '{}'",
            expected_kind,
            msg.kind
        ));
    }
    Ok(msg)
}

fn require_bytes_32(buf: Option<ByteBuf>, field_name: &str) -> Result<[u8; 32]> {
    let bytes =
        buf.ok_or_else(|| anyhow!("missing field '{}' in handshake message", field_name))?;
    if bytes.len() != 32 {
        return Err(anyhow!(
            "field '{}' must be 32 bytes, got {}",
            field_name,
            bytes.len()
        ));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

// ---------------------------------------------------------------------------
// Pairing Handshake
// ---------------------------------------------------------------------------

pub async fn pair_server<T: Transport + ?Sized>(
    conn: &mut T,
    identity: &Identity,
    code: &str,
) -> Result<Session> {
    let psk = derive_psk(code);
    let (eph, eph_pub) = gen_ephemeral();

    // 1. Receive pair_hello
    let msg1 = recv_msg(conn, "pair_hello").await?;
    let e_c = require_bytes_32(msg1.e, "e")?;

    // 2. Derive k1 and encrypt static public key
    let dh_ee = dh(&eph, &e_c);
    let k1 = hkdf_sha256(&psk, &dh_ee, b"opendesk-pair-k1", 32);
    let cipher1 = ChaCha20Poly1305::new_from_slice(&k1)?;
    let zero_nonce = Nonce::from_slice(&[0u8; 12]);
    let ct1 = cipher1
        .encrypt(zero_nonce, identity.public_bytes().as_ref())
        .map_err(|e| anyhow!("aead encrypt failed: {}", e))?;

    send_msg(
        conn,
        HandshakeMessage {
            v: HANDSHAKE_VERSION,
            kind: "pair_offer".to_string(),
            e: Some(ByteBuf::from(eph_pub)),
            ct: Some(ByteBuf::from(ct1.clone())),
            s: None,
        },
    )
    .await?;

    // 3. Receive pair_finish
    let msg3 = recv_msg(conn, "pair_finish").await?;
    let ct2 = msg3
        .ct
        .ok_or_else(|| anyhow!("missing ct in pair_finish"))?;

    let mut salt2 = Vec::with_capacity(psk.len() + ct1.len());
    salt2.extend_from_slice(&psk);
    salt2.extend_from_slice(&ct1);

    let k2 = hkdf_sha256(&salt2, &dh_ee, b"opendesk-pair-k2", 32);
    let cipher2 = ChaCha20Poly1305::new_from_slice(&k2)?;
    let client_static_bytes = cipher2
        .decrypt(zero_nonce, ct2.as_ref())
        .map_err(|_| anyhow!("client could not prove pairing code (wrong_code)"))?;

    if client_static_bytes.len() != 32 {
        return Err(anyhow!("decrypted client static key is not 32 bytes"));
    }
    let mut client_static = [0u8; 32];
    client_static.copy_from_slice(&client_static_bytes);

    // Session keys derivation
    let mut transcript_data = Vec::new();
    transcript_data.extend_from_slice(&eph_pub);
    transcript_data.extend_from_slice(&e_c);
    transcript_data.extend_from_slice(&ct1);
    transcript_data.extend_from_slice(&ct2);
    let transcript = Sha256::digest(&transcript_data);

    let dh_chain = [
        dh(&eph, &e_c),
        identity.diffie_hellman(&e_c),
        dh(&eph, &client_static),
    ];

    let (send_key, recv_key) = session_keys(true, &transcript, &dh_chain, Some(&psk));

    Ok(Session {
        channel: EncryptedChannel::new(&send_key, &recv_key),
        peer_public: client_static,
        is_pairing: true,
    })
}

pub async fn pair_client<T: Transport + ?Sized>(
    conn: &mut T,
    identity: &Identity,
    code: &str,
) -> Result<Session> {
    let psk = derive_psk(code);
    let (eph, eph_pub) = gen_ephemeral();

    // 1. Send pair_hello
    send_msg(
        conn,
        HandshakeMessage {
            v: HANDSHAKE_VERSION,
            kind: "pair_hello".to_string(),
            e: Some(ByteBuf::from(eph_pub)),
            ct: None,
            s: None,
        },
    )
    .await?;

    // 2. Receive pair_offer
    let msg2 = recv_msg(conn, "pair_offer").await?;
    let e_s = require_bytes_32(msg2.e, "e")?;
    let ct1 = msg2.ct.ok_or_else(|| anyhow!("missing ct in pair_offer"))?;

    let dh_ee = dh(&eph, &e_s);
    let k1 = hkdf_sha256(&psk, &dh_ee, b"opendesk-pair-k1", 32);
    let cipher1 = ChaCha20Poly1305::new_from_slice(&k1)?;
    let zero_nonce = Nonce::from_slice(&[0u8; 12]);
    let server_static_bytes = cipher1
        .decrypt(zero_nonce, ct1.as_ref())
        .map_err(|_| anyhow!("server PSK does not match — wrong code?"))?;

    if server_static_bytes.len() != 32 {
        return Err(anyhow!("decrypted server static key is not 32 bytes"));
    }
    let mut server_static = [0u8; 32];
    server_static.copy_from_slice(&server_static_bytes);

    // 3. Encrypt client static and send pair_finish
    let mut salt2 = Vec::with_capacity(psk.len() + ct1.len());
    salt2.extend_from_slice(&psk);
    salt2.extend_from_slice(&ct1);

    let k2 = hkdf_sha256(&salt2, &dh_ee, b"opendesk-pair-k2", 32);
    let cipher2 = ChaCha20Poly1305::new_from_slice(&k2)?;
    let ct2 = cipher2
        .encrypt(zero_nonce, identity.public_bytes().as_ref())
        .map_err(|e| anyhow!("aead encrypt failed: {}", e))?;

    send_msg(
        conn,
        HandshakeMessage {
            v: HANDSHAKE_VERSION,
            kind: "pair_finish".to_string(),
            e: None,
            ct: Some(ByteBuf::from(ct2.clone())),
            s: None,
        },
    )
    .await?;

    // Session keys derivation
    let mut transcript_data = Vec::new();
    transcript_data.extend_from_slice(&e_s);
    transcript_data.extend_from_slice(&eph_pub);
    transcript_data.extend_from_slice(&ct1);
    transcript_data.extend_from_slice(&ct2);
    let transcript = Sha256::digest(&transcript_data);

    let dh_chain = [
        dh(&eph, &e_s),
        dh(&eph, &server_static),
        identity.diffie_hellman(&e_s),
    ];

    let (send_key, recv_key) = session_keys(false, &transcript, &dh_chain, Some(&psk));

    Ok(Session {
        channel: EncryptedChannel::new(&send_key, &recv_key),
        peer_public: server_static,
        is_pairing: true,
    })
}

// ---------------------------------------------------------------------------
// Reconnect Handshake
// ---------------------------------------------------------------------------

pub async fn auth_server<T: Transport + ?Sized>(
    conn: &mut T,
    identity: &Identity,
    trusted: &TrustedPeers,
) -> Result<Session> {
    let (eph, eph_pub) = gen_ephemeral();

    // 1. Receive auth_hello
    let msg1 = recv_msg(conn, "auth_hello").await?;
    let e_c = require_bytes_32(msg1.e, "e")?;
    let s_c = require_bytes_32(msg1.s, "s")?;

    if !trusted.contains(&s_c) {
        // Send ambiguous offer rejection without leaking reason
        let _ = send_msg(
            conn,
            HandshakeMessage {
                v: HANDSHAKE_VERSION,
                kind: "auth_offer".to_string(),
                e: Some(ByteBuf::from(eph_pub)),
                ct: Some(ByteBuf::from(Vec::new())),
                s: None,
            },
        )
        .await;
        return Err(anyhow!(
            "untrusted_peer: client static key not in trusted-peers"
        ));
    }

    let dh_chain = [
        dh(&eph, &e_c),
        identity.diffie_hellman(&e_c),
        dh(&eph, &s_c),
        identity.diffie_hellman(&s_c),
    ];

    let mut transcript_data = Vec::new();
    transcript_data.extend_from_slice(&eph_pub);
    transcript_data.extend_from_slice(&e_c);
    transcript_data.extend_from_slice(&s_c);
    let transcript = Sha256::digest(&transcript_data);

    let mut ikm1 = Vec::new();
    ikm1.extend_from_slice(&dh_chain[0]);
    ikm1.extend_from_slice(&dh_chain[1]);

    let k = hkdf_sha256(&transcript, &ikm1, b"opendesk-auth-k", 32);
    let cipher = ChaCha20Poly1305::new_from_slice(&k)?;
    let zero_nonce = Nonce::from_slice(&[0u8; 12]);
    let ct = cipher
        .encrypt(zero_nonce, b"ok".as_ref())
        .map_err(|e| anyhow!("aead encrypt failed: {}", e))?;

    send_msg(
        conn,
        HandshakeMessage {
            v: HANDSHAKE_VERSION,
            kind: "auth_offer".to_string(),
            e: Some(ByteBuf::from(eph_pub)),
            ct: Some(ByteBuf::from(ct)),
            s: None,
        },
    )
    .await?;

    let (send_key, recv_key) = session_keys(true, &transcript, &dh_chain, None);

    Ok(Session {
        channel: EncryptedChannel::new(&send_key, &recv_key),
        peer_public: s_c,
        is_pairing: false,
    })
}

pub async fn auth_client<T: Transport + ?Sized>(
    conn: &mut T,
    identity: &Identity,
    expected_server_pubkey: &[u8; 32],
) -> Result<Session> {
    let (eph, eph_pub) = gen_ephemeral();

    // 1. Send auth_hello
    send_msg(
        conn,
        HandshakeMessage {
            v: HANDSHAKE_VERSION,
            kind: "auth_hello".to_string(),
            e: Some(ByteBuf::from(eph_pub)),
            ct: None,
            s: Some(ByteBuf::from(identity.public_bytes())),
        },
    )
    .await?;

    // 2. Receive auth_offer
    let msg2 = recv_msg(conn, "auth_offer").await?;
    let e_s = require_bytes_32(msg2.e, "e")?;
    let ct = msg2.ct.ok_or_else(|| anyhow!("missing ct in auth_offer"))?;

    let dh_chain = [
        dh(&eph, &e_s),
        dh(&eph, expected_server_pubkey),
        identity.diffie_hellman(&e_s),
        identity.diffie_hellman(expected_server_pubkey),
    ];

    let mut transcript_data = Vec::new();
    transcript_data.extend_from_slice(&e_s);
    transcript_data.extend_from_slice(&eph_pub);
    transcript_data.extend_from_slice(&identity.public_bytes());
    let transcript = Sha256::digest(&transcript_data);

    let mut ikm1 = Vec::new();
    ikm1.extend_from_slice(&dh_chain[0]);
    ikm1.extend_from_slice(&dh_chain[1]);

    let k = hkdf_sha256(&transcript, &ikm1, b"opendesk-auth-k", 32);
    let cipher = ChaCha20Poly1305::new_from_slice(&k)?;
    let zero_nonce = Nonce::from_slice(&[0u8; 12]);
    let ok = cipher
        .decrypt(zero_nonce, ct.as_ref())
        .map_err(|_| anyhow!("unexpected_peer: server does not hold expected static key"))?;

    if ok != b"ok" {
        return Err(anyhow!("protocol error: unexpected confirmation payload"));
    }

    let (send_key, recv_key) = session_keys(false, &transcript, &dh_chain, None);

    Ok(Session {
        channel: EncryptedChannel::new(&send_key, &recv_key),
        peer_public: *expected_server_pubkey,
        is_pairing: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handshake_message_wire_compat() {
        let msg = HandshakeMessage {
            v: 1,
            kind: "pair_hello".to_string(),
            e: Some(ByteBuf::from(vec![1u8; 32])),
            ct: None,
            s: None,
        };
        let packed = rmp_serde::to_vec_named(&msg).unwrap();
        let expected_hex = "83a17601a46b696e64aa706169725f68656c6c6fa165c4200101010101010101010101010101010101010101010101010101010101010101";
        assert_eq!(data_encoding::HEXLOWER.encode(&packed), expected_hex);
    }

    struct InMemoryTransport {
        tx: tokio::sync::mpsc::Sender<Vec<u8>>,
        rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    }

    impl Transport for InMemoryTransport {
        async fn send(&mut self, data: &[u8]) -> Result<()> {
            self.tx
                .send(data.to_vec())
                .await
                .map_err(|_| anyhow!("send failed; receiver dropped"))
        }

        async fn recv(&mut self) -> Result<Vec<u8>> {
            self.rx
                .recv()
                .await
                .ok_or_else(|| anyhow!("recv failed; sender dropped"))
        }
    }

    fn channel_pair() -> (InMemoryTransport, InMemoryTransport) {
        let (tx1, rx1) = tokio::sync::mpsc::channel(16);
        let (tx2, rx2) = tokio::sync::mpsc::channel(16);
        (
            InMemoryTransport { tx: tx1, rx: rx2 },
            InMemoryTransport { tx: tx2, rx: rx1 },
        )
    }

    #[tokio::test]
    async fn test_pairing_handshake_flow() -> Result<()> {
        let server_id = Identity::generate();
        let client_id = Identity::generate();
        let code = "123456";

        let (mut server_conn, mut client_conn) = channel_pair();

        let s_id = server_id.clone();
        let server_task =
            tokio::spawn(async move { pair_server(&mut server_conn, &s_id, code).await });

        let c_id = client_id.clone();
        let client_task =
            tokio::spawn(async move { pair_client(&mut client_conn, &c_id, code).await });

        let (server_sess, client_sess) = tokio::try_join!(
            async { server_task.await.map_err(|e| anyhow!(e))? },
            async { client_task.await.map_err(|e| anyhow!(e))? },
        )?;

        // Verify public keys exchanged correctly
        assert_eq!(server_sess.peer_public, client_id.public_bytes());
        assert_eq!(client_sess.peer_public, server_id.public_bytes());

        // Test encrypted channel bidirectional communication
        let mut s_chan = server_sess.channel;
        let mut c_chan = client_sess.channel;

        let ct = c_chan.encrypt(b"hello from client")?;
        let pt = s_chan.decrypt(&ct)?;
        assert_eq!(pt, b"hello from client");

        let ct2 = s_chan.encrypt(b"hello from server")?;
        let pt2 = c_chan.decrypt(&ct2)?;
        assert_eq!(pt2, b"hello from server");

        Ok(())
    }

    #[tokio::test]
    async fn test_reconnect_auth_flow() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let trusted = TrustedPeers::new(Some(temp_dir.path()));

        let server_id = Identity::generate();
        let client_id = Identity::generate();

        // Trust client
        trusted.add(&client_id.public_bytes(), "test-client", "")?;

        let (mut server_conn, mut client_conn) = channel_pair();

        let s_id = server_id.clone();
        let tr = trusted.clone();
        let server_task =
            tokio::spawn(async move { auth_server(&mut server_conn, &s_id, &tr).await });

        let c_id = client_id.clone();
        let s_pub = server_id.public_bytes();
        let client_task =
            tokio::spawn(async move { auth_client(&mut client_conn, &c_id, &s_pub).await });

        let (server_sess, client_sess) = tokio::try_join!(
            async { server_task.await.map_err(|e| anyhow!(e))? },
            async { client_task.await.map_err(|e| anyhow!(e))? },
        )?;

        assert_eq!(server_sess.peer_public, client_id.public_bytes());
        assert_eq!(client_sess.peer_public, server_id.public_bytes());

        // Test encrypted channel communication
        let mut s_chan = server_sess.channel;
        let mut c_chan = client_sess.channel;

        let ct = c_chan.encrypt(b"auth payload test")?;
        let pt = s_chan.decrypt(&ct)?;
        assert_eq!(pt, b"auth payload test");

        Ok(())
    }
}
