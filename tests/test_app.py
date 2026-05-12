"""Tests for the local web UI.

Uses FastAPI's ``TestClient`` against ``create_app(state)`` with a fake
:class:`OpendeskServer` so we don't bind real ports.  The endpoints are
thin facades over primitives covered by other test files (pairing,
trusted-peers, audit, RemoteComputer); we mostly verify wiring + JSON
shape here.
"""

from __future__ import annotations

import asyncio
import contextlib
from pathlib import Path
from typing import Any

import pytest

try:
    from fastapi.testclient import TestClient
except ImportError:
    pytest.skip("fastapi not installed", allow_module_level=True)

from opendesk.protocol.auth import Identity, TrustedPeers
from opendesk.remote.audit import AuditLog
from opendesk.remote.server import write_description
from opendesk.app.app import AppState, create_app


# ---------------------------------------------------------------------------
# Fakes
# ---------------------------------------------------------------------------


class FakeSessionRegistry:
    def __init__(self) -> None:
        self._sessions: list = []
        self.killed_all = 0

    async def list(self) -> list:
        return list(self._sessions)

    async def kill_all(self) -> int:
        n = len(self._sessions)
        self._sessions = []
        self.killed_all += 1
        return n

    async def kill(self, sid: str) -> bool:
        before = len(self._sessions)
        self._sessions = [s for s in self._sessions if s.id != sid]
        return len(self._sessions) < before


class FakeServer:
    def __init__(self) -> None:
        self.sessions = FakeSessionRegistry()
        self.enable_pairing_calls: list[str] = []

    async def enable_pairing(self, code: str, *, timeout=None) -> bytes:
        """Block until cancelled — so tests can observe the in-flight state.

        Real `enable_pairing` waits up to ``timeout`` seconds for a peer to
        connect; using a long sleep here mirrors that without pulling in a
        real OpendeskServer.
        """
        self.enable_pairing_calls.append(code)
        try:
            await asyncio.sleep(30)
        except asyncio.CancelledError:
            raise
        return b"\x12" * 32


def _state(tmp_path: Path) -> AppState:
    # Pre-seed an identity so /api/state has something to show.
    Identity.load_or_create(tmp_path)
    return AppState(home=tmp_path, server=FakeServer())


# ---------------------------------------------------------------------------
# /api/state
# ---------------------------------------------------------------------------


class TestState:
    def test_initial_state_is_clean(self, tmp_path: Path):
        state = _state(tmp_path)
        app = create_app(state)
        with TestClient(app) as client:
            r = client.get("/api/state")
            assert r.status_code == 200
            body = r.json()
            assert body["identity"]["fingerprint"]
            assert body["trusted_peers"] == []
            assert body["active_session"] is None
            assert body["pairing_active"] is False
            assert body["default_peer"] is None

    def test_trusted_peers_show_up(self, tmp_path: Path):
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="mini")
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="desk")
        state = _state(tmp_path)
        app = create_app(state)
        with TestClient(app) as client:
            body = client.get("/api/state").json()
            names = [p["name"] for p in body["trusted_peers"]]
            assert "mini" in names
            assert "desk" in names

    def test_default_peer_surfaces(self, tmp_path: Path):
        store = TrustedPeers(tmp_path)
        store.add(Identity.generate().public_bytes, name="mini")
        store.set_default("mini")
        state = _state(tmp_path)
        app = create_app(state)
        with TestClient(app) as client:
            body = client.get("/api/state").json()
            assert body["default_peer"] == "mini"
            entry = next(p for p in body["trusted_peers"] if p["name"] == "mini")
            assert entry["is_default"] is True

    def test_description_round_trip(self, tmp_path: Path):
        state = _state(tmp_path)
        app = create_app(state)
        with TestClient(app) as client:
            r = client.post("/api/describe", json={"text": "ERP terminal"})
            assert r.status_code == 200
            r = client.get("/api/state")
            assert r.json()["identity"]["description"] == "ERP terminal"
            r = client.post("/api/describe", json={"clear": True})
            assert r.status_code == 200
            assert client.get("/api/state").json()["identity"]["description"] == ""


# ---------------------------------------------------------------------------
# /api/pair/*
# ---------------------------------------------------------------------------


class TestPairing:
    def test_pair_begin_returns_code(self, tmp_path: Path):
        state = _state(tmp_path)
        app = create_app(state)
        with TestClient(app) as client:
            r = client.post("/api/pair/begin")
            assert r.status_code == 200
            body = r.json()
            assert body["code"].isdigit()
            assert len(body["code"]) == 6
            assert body["already_active"] is False
            # Second call sees the in-flight pairing.
            r2 = client.post("/api/pair/begin")
            assert r2.json()["code"] == body["code"]
            assert r2.json()["already_active"] is True

    def test_pair_cancel(self, tmp_path: Path):
        state = _state(tmp_path)
        app = create_app(state)
        with TestClient(app) as client:
            client.post("/api/pair/begin")
            r = client.post("/api/pair/cancel")
            assert r.status_code == 200
            assert r.json()["cancelled"] is True


# ---------------------------------------------------------------------------
# unpair / disconnect / default
# ---------------------------------------------------------------------------


class TestUnpair:
    def test_unpair_unknown_404(self, tmp_path: Path):
        state = _state(tmp_path)
        app = create_app(state)
        with TestClient(app) as client:
            r = client.post("/api/unpair", json={"name": "ghost"})
            assert r.status_code == 404

    def test_unpair_known(self, tmp_path: Path):
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="mini")
        state = _state(tmp_path)
        app = create_app(state)
        with TestClient(app) as client:
            r = client.post("/api/unpair", json={"name": "mini"})
            assert r.status_code == 200
            assert r.json()["ok"] is True
            assert TrustedPeers(tmp_path).find_by_name("mini") is None

    def test_unpair_all(self, tmp_path: Path):
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="mini")
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="desk")
        state = _state(tmp_path)
        app = create_app(state)
        with TestClient(app) as client:
            r = client.post("/api/unpair-all")
            assert r.status_code == 200
            assert r.json()["unpaired"] == 2
            assert TrustedPeers(tmp_path).list() == []

    def test_disconnect_calls_kill_all(self, tmp_path: Path):
        state = _state(tmp_path)
        app = create_app(state)
        with TestClient(app) as client:
            r = client.post("/api/disconnect")
            assert r.status_code == 200
            assert state.server.sessions.killed_all == 1


class TestDefaultPeer:
    def test_set_and_clear(self, tmp_path: Path):
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="mini")
        state = _state(tmp_path)
        app = create_app(state)
        with TestClient(app) as client:
            r = client.post("/api/peers/default", json={"name": "mini"})
            assert r.status_code == 200
            assert r.json()["default"] == "mini"
            r = client.post("/api/peers/default", json={"clear": True})
            assert r.status_code == 200
            assert r.json()["default"] is None

    def test_set_unknown_404(self, tmp_path: Path):
        state = _state(tmp_path)
        app = create_app(state)
        with TestClient(app) as client:
            r = client.post("/api/peers/default", json={"name": "ghost"})
            assert r.status_code == 404


# ---------------------------------------------------------------------------
# Description override per peer
# ---------------------------------------------------------------------------


class TestPeerDescription:
    def test_set_and_clear_override(self, tmp_path: Path):
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="mini")
        state = _state(tmp_path)
        app = create_app(state)
        with TestClient(app) as client:
            r = client.post("/api/peers/mini/description", json={"text": "billing"})
            assert r.status_code == 200
            assert TrustedPeers(tmp_path).effective_description("mini") == "billing"
            r = client.post("/api/peers/mini/description", json={"clear": True})
            assert r.status_code == 200
            assert TrustedPeers(tmp_path).effective_description("mini") == ""

    def test_set_unknown_404(self, tmp_path: Path):
        state = _state(tmp_path)
        app = create_app(state)
        with TestClient(app) as client:
            r = client.post("/api/peers/ghost/description", json={"text": "x"})
            assert r.status_code == 404


# ---------------------------------------------------------------------------
# Audit
# ---------------------------------------------------------------------------


class TestAudit:
    @pytest.mark.asyncio
    async def test_audit_endpoint_returns_recent(self, tmp_path: Path):
        log = AuditLog(home=tmp_path)
        try:
            await log.record_session_opened(
                peer_public=b"\x00" * 32, peer_name="laptop",
                session_id="s1", remote_addr="x:1", mode="serve",
            )
            await log.record_call(
                peer_public=b"\x00" * 32, peer_name="laptop",
                session_id="s1", method="display.capture", params={},
                outcome="ok",
            )
        finally:
            await log.aclose()

        state = _state(tmp_path)
        app = create_app(state)
        with TestClient(app) as client:
            r = client.get("/api/audit?limit=10")
            assert r.status_code == 200
            entries = r.json()["entries"]
            assert len(entries) == 2
            kinds = {e["type"] for e in entries}
            assert kinds == {"session.opened", "call"}


# ---------------------------------------------------------------------------
# Static
# ---------------------------------------------------------------------------


class TestStatic:
    def test_index_serves_html(self, tmp_path: Path):
        state = _state(tmp_path)
        app = create_app(state)
        with TestClient(app) as client:
            r = client.get("/")
            assert r.status_code == 200
            assert "opendesk" in r.text.lower()

    def test_styles_served(self, tmp_path: Path):
        state = _state(tmp_path)
        app = create_app(state)
        with TestClient(app) as client:
            r = client.get("/static/styles.css")
            assert r.status_code == 200
            assert ":root" in r.text


# ---------------------------------------------------------------------------
# CLI wiring smoke
# ---------------------------------------------------------------------------


class TestCLI:
    def test_app_subcommand_exists(self):
        import subprocess
        import sys
        r = subprocess.run(
            [sys.executable, "-m", "opendesk.cli", "app", "--help"],
            capture_output=True, text=True, timeout=10,
        )
        assert r.returncode == 0
        assert "--port" in r.stdout
        assert "--no-browser" in r.stdout
