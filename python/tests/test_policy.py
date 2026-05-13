"""Tests for the server-side authorisation policy.

Unit tests for :class:`AllowAllPolicy` / :class:`ConsolePolicy`, plus a
full-stack integration test that verifies a denying policy propagates as
:class:`PermissionDeniedError` to the controller through the wire.
"""

from __future__ import annotations

import asyncio
import io
from pathlib import Path
from typing import Any

import pytest

from opendesk.computer import PointerAction, PointerEvent, Point
from opendesk.protocol.auth import Identity, TrustedPeers
from opendesk.remote.client import connect
from opendesk.remote.policy import (
    AllowAllPolicy,
    ConsolePolicy,
    OBSERVATION_METHODS,
    Policy,
)
from opendesk.remote.server import OpendeskServer
from opendesk.tools.base import PermissionDeniedError

from tests._fakes import FakeComputer


# ---------------------------------------------------------------------------
# AllowAllPolicy
# ---------------------------------------------------------------------------


class TestAllowAllPolicy:
    @pytest.mark.asyncio
    async def test_allows_every_method(self):
        p = AllowAllPolicy()
        # Try a representative sample including destructive ones.
        for method in (
            "display.capture", "input.pointer", "fs.write",
            "process.shell", "apps.close", "power.lock",
        ):
            await p.check(
                peer_public=b"\x00" * 32, peer_name="x", method=method, params={},
            )


# ---------------------------------------------------------------------------
# ConsolePolicy
# ---------------------------------------------------------------------------


class _FakeStream:
    """Minimal stand-in for sys.stdin used to inject TTY-ness in tests."""

    def __init__(self, *, isatty: bool) -> None:
        self._tty = isatty

    def isatty(self) -> bool:
        return self._tty


class TestConsolePolicy:
    @pytest.mark.asyncio
    async def test_observation_methods_always_allowed(self):
        """Even without a TTY, observation methods pass through."""
        policy = ConsolePolicy(stream=_FakeStream(isatty=False))
        for method in OBSERVATION_METHODS:
            await policy.check(
                peer_public=b"\x00" * 32, peer_name="laptop",
                method=method, params={},
            )

    @pytest.mark.asyncio
    async def test_no_tty_denies_gated_methods(self):
        policy = ConsolePolicy(stream=_FakeStream(isatty=False))
        with pytest.raises(PermissionDeniedError) as exc_info:
            await policy.check(
                peer_public=b"\x00" * 32, peer_name="laptop",
                method="input.pointer", params={"event": {"action": "down"}},
            )
        assert "no TTY" in str(exc_info.value)

    @pytest.mark.asyncio
    async def test_tty_yes_allows(self, monkeypatch):
        policy = ConsolePolicy(stream=_FakeStream(isatty=True))
        # Stub the input() helper to answer "y".
        from opendesk.remote import policy as policy_mod
        monkeypatch.setattr(policy_mod, "_read_line", lambda prompt: "y")
        await policy.check(
            peer_public=b"\x00" * 32, peer_name="laptop",
            method="input.pointer",
            params={"event": {"action": "down", "point": {"x": 1, "y": 2}}},
        )

    @pytest.mark.asyncio
    async def test_tty_no_denies(self, monkeypatch):
        policy = ConsolePolicy(stream=_FakeStream(isatty=True))
        from opendesk.remote import policy as policy_mod
        monkeypatch.setattr(policy_mod, "_read_line", lambda prompt: "n")
        with pytest.raises(PermissionDeniedError):
            await policy.check(
                peer_public=b"\x00" * 32, peer_name="laptop",
                method="fs.write", params={"path": "/etc/passwd", "data": b""},
            )

    @pytest.mark.asyncio
    async def test_prompt_includes_peer_name_and_summary(self, monkeypatch):
        captured: dict[str, str] = {}
        policy = ConsolePolicy(stream=_FakeStream(isatty=True))

        from opendesk.remote import policy as policy_mod

        def fake_input(prompt: str) -> str:
            captured["prompt"] = prompt
            return "y"
        monkeypatch.setattr(policy_mod, "_read_line", fake_input)

        await policy.check(
            peer_public=b"\x12" * 32, peer_name="laptop",
            method="process.shell", params={"command": "ls -la /tmp"},
        )
        assert "laptop" in captured["prompt"]
        assert "run shell" in captured["prompt"]
        assert "ls -la /tmp" in captured["prompt"]


# ---------------------------------------------------------------------------
# Custom policy hooks for the integration tests
# ---------------------------------------------------------------------------


class DenyPointerPolicy:
    """Approves everything except input.pointer."""

    async def check(self, *, peer_public, peer_name, method, params):
        if method == "input.pointer":
            raise PermissionDeniedError(
                f"{peer_name} cannot move the pointer"
            )


class RecordingPolicy:
    """Records every check; useful for verifying that peer context is correct."""

    def __init__(self) -> None:
        self.calls: list[tuple[str, str, str]] = []

    async def check(self, *, peer_public, peer_name, method, params):
        self.calls.append((peer_public.hex()[:6], peer_name, method))


# ---------------------------------------------------------------------------
# End-to-end through the wire
# ---------------------------------------------------------------------------


async def _server_with_policy(tmp_path: Path, policy):
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
        policy=policy,
    )
    await server.start()
    return server, fake, server_id.public_bytes


class TestPolicyEndToEnd:
    @pytest.mark.asyncio
    async def test_denying_policy_surfaces_permission_denied(self, tmp_path: Path):
        server, _, server_pub = await _server_with_policy(
            tmp_path, DenyPointerPolicy(),
        )
        try:
            remote = await connect(
                f"ws://127.0.0.1:{server.port}#{server_pub.hex()}",
                home=tmp_path / "client",
                auto_reconnect=False,
            )
            try:
                # Allowed: an observation.
                pos = await remote.cursor_position()
                assert pos.x == 50

                # Denied: pointer event.
                with pytest.raises(PermissionDeniedError) as exc_info:
                    await remote.pointer(PointerEvent(
                        action=PointerAction.MOVE, point=Point(x=10, y=20),
                    ))
                assert "laptop" in str(exc_info.value)

                # Observations still work after the denial.
                pos2 = await remote.cursor_position()
                assert pos2.x == 50
            finally:
                await remote.aclose()
        finally:
            await server.aclose()

    @pytest.mark.asyncio
    async def test_policy_receives_correct_peer_context(self, tmp_path: Path):
        recorder = RecordingPolicy()
        server, _, server_pub = await _server_with_policy(tmp_path, recorder)
        try:
            remote = await connect(
                f"ws://127.0.0.1:{server.port}#{server_pub.hex()}",
                home=tmp_path / "client",
                auto_reconnect=False,
            )
            try:
                await remote.cursor_position()
                await remote.environment()
            finally:
                await remote.aclose()
        finally:
            await server.aclose()

        assert recorder.calls, "policy.check was never invoked"
        methods = {m for _, _, m in recorder.calls}
        assert "display.cursor_position" in methods
        assert "system.environment" in methods
        # Every call carries the same client public key + friendly name.
        client_pub_hex = Identity.load_or_create(tmp_path / "client").public_bytes.hex()[:6]
        for pub_hex, name, _ in recorder.calls:
            assert pub_hex == client_pub_hex
            assert name == "laptop"

    @pytest.mark.asyncio
    async def test_default_policy_is_allow_all(self, tmp_path: Path):
        """If you don't pass a policy, nothing's gated."""
        server, _, server_pub = await _server_with_policy(tmp_path, None)
        try:
            remote = await connect(
                f"ws://127.0.0.1:{server.port}#{server_pub.hex()}",
                home=tmp_path / "client",
                auto_reconnect=False,
            )
            try:
                # The "destructive" call works because default is allow-all.
                await remote.pointer(PointerEvent(
                    action=PointerAction.MOVE, point=Point(x=10, y=20),
                ))
            finally:
                await remote.aclose()
        finally:
            await server.aclose()
