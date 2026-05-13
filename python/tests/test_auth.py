"""Tests for the auth layer — identity, trusted-peers, handshakes, encryption."""

from __future__ import annotations

import asyncio
from pathlib import Path

import pytest

from opendesk.protocol import LoopbackConnection
from opendesk.protocol.auth import (
    AuthFailure,
    EncryptedConnection,
    Identity,
    TrustedPeers,
    auth_client,
    auth_server,
    pair_client,
    pair_server,
)
from opendesk.protocol.auth.identity import generate_pairing_code


# ---------------------------------------------------------------------------
# Identity
# ---------------------------------------------------------------------------


class TestIdentity:
    def test_generate_produces_32_byte_pubkey(self):
        ident = Identity.generate()
        assert len(ident.public_bytes) == 32

    def test_roundtrip_through_disk(self, tmp_path: Path):
        a = Identity.load_or_create(tmp_path)
        b = Identity.load_or_create(tmp_path)
        # Second call must load the existing key, not generate a new one.
        assert a.public_bytes == b.public_bytes

    def test_file_is_owner_only(self, tmp_path: Path):
        Identity.load_or_create(tmp_path)
        path = tmp_path / "identity.key"
        # 0o077 mask: world / group bits must be clear.
        assert path.stat().st_mode & 0o077 == 0

    def test_dh_is_symmetric(self):
        a = Identity.generate()
        b = Identity.generate()
        assert a.exchange(b.public_bytes) == b.exchange(a.public_bytes)


class TestPairingCode:
    def test_six_digit_zero_padded(self):
        for _ in range(20):
            code = generate_pairing_code()
            assert len(code) == 6
            assert code.isdigit()


# ---------------------------------------------------------------------------
# TrustedPeers
# ---------------------------------------------------------------------------


class TestTrustedPeers:
    def test_add_then_contains(self, tmp_path: Path):
        store = TrustedPeers(tmp_path)
        ident = Identity.generate()
        store.add(ident.public_bytes, name="mini")
        assert store.contains(ident.public_bytes)

    def test_persists_across_instances(self, tmp_path: Path):
        ident = Identity.generate()
        TrustedPeers(tmp_path).add(ident.public_bytes, name="mini")
        # Fresh instance reads the file from scratch.
        assert TrustedPeers(tmp_path).contains(ident.public_bytes)

    def test_remove_by_name(self, tmp_path: Path):
        ident = Identity.generate()
        store = TrustedPeers(tmp_path)
        store.add(ident.public_bytes, name="mini")
        assert store.remove("mini")
        assert not store.contains(ident.public_bytes)

    def test_rename(self, tmp_path: Path):
        ident = Identity.generate()
        store = TrustedPeers(tmp_path)
        store.add(ident.public_bytes, name="mini")
        assert store.rename("mini", "laptop")
        assert store.find_by_name("laptop") is not None


# ---------------------------------------------------------------------------
# Handshake — pairing
# ---------------------------------------------------------------------------


async def _run_pair(code_server: str, code_client: str):
    """Run pair_server / pair_client on opposite ends of a loopback link.

    Returns the identities, connections, and tasks.  Tests are responsible
    for closing the connections — handshake functions don't own them.
    """
    s_ident = Identity.generate()
    c_ident = Identity.generate()
    a, b = LoopbackConnection.pair()

    server_task = asyncio.create_task(pair_server(b, s_ident, code_server))
    client_task = asyncio.create_task(pair_client(a, c_ident, code_client))
    return s_ident, c_ident, a, b, server_task, client_task


class TestPairing:
    @pytest.mark.asyncio
    async def test_pairing_succeeds_with_matching_code(self):
        s_ident, c_ident, a, b, st, ct = await _run_pair("123456", "123456")
        s_session, c_session = await asyncio.gather(st, ct)
        try:
            # Each side learned the other's long-lived static public key.
            assert s_session.peer_public == c_ident.public_bytes
            assert c_session.peer_public == s_ident.public_bytes
            assert s_session.is_pairing and c_session.is_pairing

            # The two encrypted connections can talk: client sends, server receives.
            await c_session.connection.send(b"hello from client")
            assert await s_session.connection.recv() == b"hello from client"
            await s_session.connection.send(b"hello back")
            assert await c_session.connection.recv() == b"hello back"
        finally:
            await c_session.connection.aclose()
            await s_session.connection.aclose()

    @pytest.mark.asyncio
    async def test_pairing_with_wrong_code_fails(self):
        s_ident, c_ident, a, b, st, ct = await _run_pair("123456", "000000")
        # Both peers will fail — the client at step 2 (can't decrypt server's
        # offer), the server at step 3 (can't decrypt the client's reply, OR
        # gets ConnectionClosed because the client gave up).  We need to
        # close the connections from the test side so the server's recv
        # unblocks if the client raised before sending msg3.
        async def close_after_failure():
            try:
                await ct
            finally:
                await a.aclose()
        client_task = asyncio.create_task(close_after_failure())
        results = await asyncio.gather(st, client_task, return_exceptions=True)
        server_exc, client_exc = results
        assert isinstance(client_exc, AuthFailure), client_exc
        assert client_exc.reason == "wrong_code"
        # Server side: either AuthFailure or the connection closing under it.
        assert isinstance(server_exc, BaseException), server_exc


# ---------------------------------------------------------------------------
# Handshake — reconnect (static-key auth)
# ---------------------------------------------------------------------------


async def _run_auth(
    server_ident: Identity, client_ident: Identity, trusted: TrustedPeers,
    *, expected_server_pubkey: bytes = None,
):
    a, b = LoopbackConnection.pair()
    server_task = asyncio.create_task(auth_server(b, server_ident, trusted))
    client_task = asyncio.create_task(auth_client(
        a, client_ident, expected_server_pubkey or server_ident.public_bytes,
    ))
    return a, b, server_task, client_task


class TestReconnect:
    @pytest.mark.asyncio
    async def test_static_key_auth_succeeds(self, tmp_path: Path):
        server_id = Identity.generate()
        client_id = Identity.generate()
        trusted = TrustedPeers(tmp_path)
        trusted.add(client_id.public_bytes, name="laptop")

        a, b, st, ct = await _run_auth(server_id, client_id, trusted)
        s_session, c_session = await asyncio.gather(st, ct)
        try:
            assert not s_session.is_pairing
            assert s_session.peer_public == client_id.public_bytes
            assert c_session.peer_public == server_id.public_bytes

            await c_session.connection.send(b"reconnect ok")
            assert await s_session.connection.recv() == b"reconnect ok"
        finally:
            await c_session.connection.aclose()
            await s_session.connection.aclose()

    @pytest.mark.asyncio
    async def test_untrusted_client_rejected(self, tmp_path: Path):
        server_id = Identity.generate()
        client_id = Identity.generate()
        trusted = TrustedPeers(tmp_path)  # empty — nobody trusted

        a, b, st, ct = await _run_auth(server_id, client_id, trusted)
        results = await asyncio.gather(st, ct, return_exceptions=True)
        server_exc, client_exc = results
        assert isinstance(server_exc, AuthFailure), server_exc
        assert server_exc.reason == "untrusted_peer"
        assert isinstance(client_exc, BaseException), client_exc

    @pytest.mark.asyncio
    async def test_wrong_server_static_rejected(self, tmp_path: Path):
        server_id = Identity.generate()
        wrong_server = Identity.generate()
        client_id = Identity.generate()
        trusted = TrustedPeers(tmp_path)
        trusted.add(client_id.public_bytes, name="laptop")

        a, b, st, ct = await _run_auth(
            server_id, client_id, trusted,
            expected_server_pubkey=wrong_server.public_bytes,
        )
        # Server side will complete (it doesn't know the client expected a
        # different static key); the resulting session just won't be usable
        # by the client, who fails.  We close the loopback to free the server.
        async def close_after_failure():
            try:
                await ct
            finally:
                await a.aclose()
        client_task = asyncio.create_task(close_after_failure())
        results = await asyncio.gather(st, client_task, return_exceptions=True)
        server_outcome, client_exc = results
        assert isinstance(client_exc, AuthFailure)
        assert client_exc.reason == "unexpected_peer"


# ---------------------------------------------------------------------------
# EncryptedConnection — direct unit tests
# ---------------------------------------------------------------------------


class TestEncryptedConnection:
    @pytest.mark.asyncio
    async def test_roundtrip_preserves_bytes(self):
        from os import urandom
        key_c2s = urandom(32)
        key_s2c = urandom(32)
        a, b = LoopbackConnection.pair()
        client = EncryptedConnection(a, send_key=key_c2s, recv_key=key_s2c)
        server = EncryptedConnection(b, send_key=key_s2c, recv_key=key_c2s)

        payload = b"\x00\x01\x02 binary \xff\xfe"
        await client.send(payload)
        assert await server.recv() == payload

        await server.send(b"a" * 100_000)
        assert await client.recv() == b"a" * 100_000

    @pytest.mark.asyncio
    async def test_tampered_frame_closes_connection(self):
        from os import urandom
        key = urandom(32)
        a, b = LoopbackConnection.pair()
        client = EncryptedConnection(a, send_key=key, recv_key=key)
        # Tamper directly through the underlying loopback.
        await b.send(b"\x00" * 32)  # not a valid AEAD ciphertext for any nonce

        from opendesk.protocol.connection import ConnectionClosed
        with pytest.raises(ConnectionClosed):
            await client.recv()


# ---------------------------------------------------------------------------
# End-to-end: pair, store, then reconnect — proves the full lifecycle.
# ---------------------------------------------------------------------------


class TestLifecycle:
    @pytest.mark.asyncio
    async def test_pair_then_reconnect(self, tmp_path: Path):
        server_id = Identity.generate()
        client_id = Identity.generate()
        code = "424242"

        # 1. Pair.
        a, b = LoopbackConnection.pair()
        st = asyncio.create_task(pair_server(b, server_id, code))
        ct = asyncio.create_task(pair_client(a, client_id, code))
        s_session, c_session = await asyncio.gather(st, ct)
        # Server records client as trusted.
        server_store = TrustedPeers(tmp_path / "server")
        server_store.add(s_session.peer_public, name="laptop")
        # Client records server's pubkey (in practice persisted similarly).
        server_pubkey = c_session.peer_public
        await c_session.connection.aclose()
        await s_session.connection.aclose()

        # 2. Reconnect (separate Loopback link, fresh ephemeral keys).
        a2, b2 = LoopbackConnection.pair()
        st2 = asyncio.create_task(auth_server(b2, server_id, server_store))
        ct2 = asyncio.create_task(auth_client(a2, client_id, server_pubkey))
        s2, c2 = await asyncio.gather(st2, ct2)
        try:
            await c2.connection.send(b"reconnect path works")
            assert await s2.connection.recv() == b"reconnect path works"
        finally:
            await c2.connection.aclose()
            await s2.connection.aclose()
