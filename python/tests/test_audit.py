"""Tests for the server-side audit log.

Two layers exercised:

* :class:`AuditLog` unit-level — JSONL file format, mode, day rotation.
* Integration through :class:`OpendeskServer` + :class:`RemoteComputer` —
  session lifecycle and per-method call entries land on disk with the right
  shape.
"""

from __future__ import annotations

import asyncio
import json
import os
from pathlib import Path

import pytest

from opendesk.computer import PointerAction, PointerEvent, Point
from opendesk.protocol.auth import Identity, TrustedPeers
from opendesk.remote.audit import AUDIT_DIR_NAME, AuditLog
from opendesk.remote.client import connect
from opendesk.remote.policy import AllowAllPolicy
from opendesk.remote.server import OpendeskServer

from tests._fakes import FakeComputer


# ---------------------------------------------------------------------------
# AuditLog unit tests
# ---------------------------------------------------------------------------


class TestAuditLogFile:
    @pytest.mark.asyncio
    async def test_creates_directory_with_secure_mode(self, tmp_path: Path):
        AuditLog(home=tmp_path)
        audit_dir = tmp_path / AUDIT_DIR_NAME
        assert audit_dir.exists()
        assert audit_dir.is_dir()
        # 0o700 perms — group/world bits must be zero.
        assert audit_dir.stat().st_mode & 0o077 == 0

    @pytest.mark.asyncio
    async def test_call_entry_format(self, tmp_path: Path):
        log = AuditLog(home=tmp_path)
        try:
            await log.record_call(
                peer_public=b"\x12" * 32, peer_name="laptop",
                session_id="abc123",
                method="input.pointer",
                params={"event": {"action": "down", "point": {"x": 100, "y": 200}}},
                outcome="ok",
            )
        finally:
            await log.aclose()

        entries = log.iter_entries()
        assert len(entries) == 1
        e = entries[0]
        assert e["type"] == "call"
        assert e["peer"]["name"] == "laptop"
        assert e["peer"]["fp"] == ("12" * 32)[:16]
        assert e["session_id"] == "abc123"
        assert e["method"] == "input.pointer"
        assert "send pointer down" in e["summary"]
        assert e["outcome"] == "ok"
        assert "error_code" not in e

    @pytest.mark.asyncio
    async def test_error_entry_records_code(self, tmp_path: Path):
        log = AuditLog(home=tmp_path)
        try:
            await log.record_call(
                peer_public=b"\x00" * 32, peer_name="laptop",
                session_id="x",
                method="fs.write",
                params={"path": "/etc/passwd", "data": b"hi"},
                outcome="error",
                error_code="permission_denied",
                error_message="not allowed",
            )
        finally:
            await log.aclose()
        e = log.iter_entries()[0]
        assert e["outcome"] == "error"
        assert e["error_code"] == "permission_denied"
        assert "not allowed" in e["error_message"]

    @pytest.mark.asyncio
    async def test_session_lifecycle_entries(self, tmp_path: Path):
        log = AuditLog(home=tmp_path)
        try:
            await log.record_session_opened(
                peer_public=b"\xaa" * 32, peer_name="laptop",
                session_id="s1", remote_addr="192.168.1.10:443", mode="serve",
            )
            await log.record_session_closed(
                peer_public=b"\xaa" * 32, peer_name="laptop",
                session_id="s1", duration=12.34, reason="",
            )
            await log.record_session_rejected(
                peer_public=b"\xbb" * 32, peer_name="other",
                remote_addr="192.168.1.20:9999", reason="busy: laptop",
            )
        finally:
            await log.aclose()
        kinds = [e["type"] for e in log.iter_entries()]
        assert kinds == ["session.opened", "session.closed", "session.rejected"]

    @pytest.mark.asyncio
    async def test_file_is_jsonl(self, tmp_path: Path):
        """Each line of the audit file must be a standalone JSON object."""
        log = AuditLog(home=tmp_path)
        try:
            for i in range(3):
                await log.record_call(
                    peer_public=b"\x00" * 32, peer_name=f"client{i}",
                    session_id=f"s{i}", method="display.capture",
                    params={}, outcome="ok",
                )
        finally:
            await log.aclose()
        # Read raw and parse line-by-line.
        files = list((tmp_path / AUDIT_DIR_NAME).glob("*.jsonl"))
        assert len(files) == 1
        for line in files[0].read_text().splitlines():
            obj = json.loads(line)
            assert obj["type"] == "call"

    @pytest.mark.asyncio
    async def test_file_mode_0600(self, tmp_path: Path):
        log = AuditLog(home=tmp_path)
        try:
            await log.record_call(
                peer_public=b"\x00" * 32, peer_name="x", session_id="s",
                method="display.capture", params={}, outcome="ok",
            )
        finally:
            await log.aclose()
        files = list((tmp_path / AUDIT_DIR_NAME).glob("*.jsonl"))
        assert len(files) == 1
        mode = files[0].stat().st_mode & 0o777
        # 0600 = owner read/write only.  Test process may be running as a
        # user whose umask differs; at minimum we control the group/world bits.
        assert mode & 0o077 == 0, oct(mode)


# ---------------------------------------------------------------------------
# Integration through real server
# ---------------------------------------------------------------------------


async def _server(tmp_path: Path):
    fake = FakeComputer()
    server_id = Identity.load_or_create(tmp_path / "server")
    server_trusted = TrustedPeers(tmp_path / "server")
    client_id = Identity.load_or_create(tmp_path / "client")
    server_trusted.add(client_id.public_bytes, name="laptop")
    TrustedPeers(tmp_path / "client").add(server_id.public_bytes, name="host")
    server = OpendeskServer(
        fake, server_id, server_trusted,
        host="127.0.0.1", port=0,
        advertise_mdns=False,
        home=tmp_path / "server",
    )
    await server.start()
    return server, server_id.public_bytes


def _read_audit(tmp_path: Path) -> list[dict]:
    log = AuditLog(home=tmp_path / "server")
    return log.iter_entries()


class TestAuditIntegration:
    @pytest.mark.asyncio
    async def test_session_open_and_close_recorded(self, tmp_path: Path):
        server, server_pub = await _server(tmp_path)
        try:
            remote = await connect(
                f"ws://127.0.0.1:{server.port}#{server_pub.hex()}",
                home=tmp_path / "client",
                auto_reconnect=False,
            )
            await remote.aclose()
            # Give the server's finally block a moment to write the close entry.
            await asyncio.sleep(0.1)
        finally:
            await server.aclose()

        entries = _read_audit(tmp_path)
        kinds = [e["type"] for e in entries]
        assert "session.opened" in kinds
        assert "session.closed" in kinds
        opened = next(e for e in entries if e["type"] == "session.opened")
        assert opened["peer"]["name"] == "laptop"

    @pytest.mark.asyncio
    async def test_call_outcomes_recorded(self, tmp_path: Path):
        server, server_pub = await _server(tmp_path)
        try:
            remote = await connect(
                f"ws://127.0.0.1:{server.port}#{server_pub.hex()}",
                home=tmp_path / "client",
                auto_reconnect=False,
            )
            try:
                await remote.cursor_position()
                await remote.pointer(PointerEvent(
                    action=PointerAction.MOVE, point=Point(x=10, y=20),
                ))
            finally:
                await remote.aclose()
            await asyncio.sleep(0.05)
        finally:
            await server.aclose()

        entries = _read_audit(tmp_path)
        call_entries = [e for e in entries if e["type"] == "call"]
        methods = {e["method"]: e for e in call_entries}
        assert "display.cursor_position" in methods
        assert methods["display.cursor_position"]["outcome"] == "ok"
        assert "input.pointer" in methods
        assert "send pointer move" in methods["input.pointer"]["summary"]

    @pytest.mark.asyncio
    async def test_policy_denial_recorded_as_error(self, tmp_path: Path):
        from opendesk.tools.base import PermissionDeniedError

        class DenyPointerPolicy:
            async def check(self, *, peer_public, peer_name, method, params):
                if method == "input.pointer":
                    raise PermissionDeniedError("no")

        fake = FakeComputer()
        server_id = Identity.load_or_create(tmp_path / "server")
        server_trusted = TrustedPeers(tmp_path / "server")
        client_id = Identity.load_or_create(tmp_path / "client")
        server_trusted.add(client_id.public_bytes, name="laptop")
        TrustedPeers(tmp_path / "client").add(server_id.public_bytes, name="host")
        server = OpendeskServer(
            fake, server_id, server_trusted,
            host="127.0.0.1", port=0,
            advertise_mdns=False,
            home=tmp_path / "server",
            policy=DenyPointerPolicy(),
        )
        await server.start()
        try:
            remote = await connect(
                f"ws://127.0.0.1:{server.port}#{server_id.public_bytes.hex()}",
                home=tmp_path / "client",
                auto_reconnect=False,
            )
            try:
                with pytest.raises(Exception):
                    await remote.pointer(PointerEvent(
                        action=PointerAction.MOVE, point=Point(x=1, y=2),
                    ))
            finally:
                await remote.aclose()
            await asyncio.sleep(0.05)
        finally:
            await server.aclose()

        entries = _read_audit(tmp_path)
        pointer_entries = [
            e for e in entries
            if e["type"] == "call" and e["method"] == "input.pointer"
        ]
        assert pointer_entries
        assert pointer_entries[0]["outcome"] == "error"
        assert pointer_entries[0]["error_code"] == "permission_denied"


class TestNoAudit:
    @pytest.mark.asyncio
    async def test_enable_audit_false_writes_nothing(self, tmp_path: Path):
        fake = FakeComputer()
        server_id = Identity.load_or_create(tmp_path / "server")
        server_trusted = TrustedPeers(tmp_path / "server")
        client_id = Identity.load_or_create(tmp_path / "client")
        server_trusted.add(client_id.public_bytes, name="laptop")
        TrustedPeers(tmp_path / "client").add(server_id.public_bytes, name="host")
        server = OpendeskServer(
            fake, server_id, server_trusted,
            host="127.0.0.1", port=0,
            advertise_mdns=False,
            home=tmp_path / "server",
            enable_audit=False,
        )
        await server.start()
        try:
            remote = await connect(
                f"ws://127.0.0.1:{server.port}#{server_id.public_bytes.hex()}",
                home=tmp_path / "client",
                auto_reconnect=False,
            )
            try:
                await remote.cursor_position()
            finally:
                await remote.aclose()
            await asyncio.sleep(0.05)
        finally:
            await server.aclose()

        # No audit directory should exist (or if it does, it's empty).
        audit_dir = tmp_path / "server" / AUDIT_DIR_NAME
        assert not audit_dir.exists() or not list(audit_dir.iterdir())
