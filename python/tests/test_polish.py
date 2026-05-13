"""Tests for the Phase-1 polish items: persistent default peer, daemon log
rotation, and the macOS-style permission self-check."""

from __future__ import annotations

import logging
import subprocess
import sys
from pathlib import Path
from unittest.mock import patch

import pytest

from opendesk.protocol.auth import Identity, TrustedPeers


# ---------------------------------------------------------------------------
# Persistent default peer
# ---------------------------------------------------------------------------


class TestPersistentDefaultPeer:
    def test_no_default_initially(self, tmp_path: Path):
        store = TrustedPeers(tmp_path)
        assert store.get_default() is None

    def test_set_default_requires_trusted_peer(self, tmp_path: Path):
        store = TrustedPeers(tmp_path)
        assert store.set_default("ghost") is False
        assert store.get_default() is None

    def test_set_default_persists_on_disk(self, tmp_path: Path):
        store = TrustedPeers(tmp_path)
        store.add(Identity.generate().public_bytes, name="mini")
        assert store.set_default("mini") is True
        # Fresh instance reads from disk.
        assert TrustedPeers(tmp_path).get_default() == "mini"

    def test_clear_default(self, tmp_path: Path):
        store = TrustedPeers(tmp_path)
        store.add(Identity.generate().public_bytes, name="mini")
        store.set_default("mini")
        assert store.clear_default() is True
        assert store.get_default() is None
        # Clearing twice is a no-op.
        assert store.clear_default() is False

    def test_default_invalidates_when_peer_removed(self, tmp_path: Path):
        """Removing the peer that's set as default also clears the default."""
        store = TrustedPeers(tmp_path)
        store.add(Identity.generate().public_bytes, name="mini")
        store.set_default("mini")
        assert store.remove("mini") is True
        assert store.get_default() is None

    def test_dangling_default_returns_none(self, tmp_path: Path):
        """If the file points at a name no longer in trusted-peers (manual
        edit, race), ``get_default`` returns None rather than the dangling
        name."""
        store = TrustedPeers(tmp_path)
        store.add(Identity.generate().public_bytes, name="mini")
        store.set_default("mini")
        # Manually corrupt: change the file to a non-existent peer.
        (tmp_path / "default-peer").write_text("nonexistent\n")
        assert store.get_default() is None


class TestMCPSessionDefaultPeer:
    def test_persistent_default_appears_in_effective_peer(self, tmp_path: Path):
        from opendesk.integrations.mcp_session import MCPSession
        from tests._fakes import FakeComputer

        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="mini")
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="desk")
        # Two peers — would normally be "ambiguous" — but a persistent
        # default disambiguates.
        TrustedPeers(tmp_path).set_default("desk")

        session = MCPSession(home=tmp_path, local=FakeComputer())
        name, source = session.effective_peer()
        assert name == "desk"
        assert source == "persistent"

    def test_explicit_use_peer_overrides_persistent(self, tmp_path: Path):
        from opendesk.integrations.mcp_session import MCPSession
        from tests._fakes import FakeComputer

        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="mini")
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="desk")
        TrustedPeers(tmp_path).set_default("desk")

        session = MCPSession(home=tmp_path, local=FakeComputer())
        session.use_peer("mini")
        name, source = session.effective_peer()
        assert name == "mini"
        assert source == "explicit"


class TestCLIDefaultPeer:
    """Smoke tests for the `opendesk peers default` CLI subcommand."""

    def _run(self, *args: str, home: Path) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, "-m", "opendesk.cli", "peers", *args,
             "--home", str(home)],
            capture_output=True, text=True, timeout=10,
        )

    def test_set_get_clear(self, tmp_path: Path):
        # Pre-seed a trusted peer so set-default can succeed.
        TrustedPeers(tmp_path).add(Identity.generate().public_bytes, name="mini")

        # Get with no default set.
        r = self._run("default", home=tmp_path)
        assert r.returncode == 0
        assert "No default peer set" in r.stdout

        # Set.
        r = self._run("default", "mini", home=tmp_path)
        assert r.returncode == 0
        assert "Default peer is now: mini" in r.stdout

        # Get again.
        r = self._run("default", home=tmp_path)
        assert r.returncode == 0
        assert "mini" in r.stdout

        # Clear.
        r = self._run("default", "--clear", home=tmp_path)
        assert r.returncode == 0
        assert "Default peer cleared" in r.stdout

    def test_set_unknown_peer_fails(self, tmp_path: Path):
        r = self._run("default", "nonexistent", home=tmp_path)
        assert r.returncode != 0
        assert "No trusted peer" in r.stderr


# ---------------------------------------------------------------------------
# Log rotation
# ---------------------------------------------------------------------------


class TestLogRotation:
    def test_configure_logging_writes_to_file(self, tmp_path: Path):
        from opendesk.cli import _configure_logging

        log_path = tmp_path / "serve.log"
        _configure_logging(str(log_path))
        logging.getLogger("opendesk.test").info("hello world")
        # Ensure flushed
        for h in logging.getLogger().handlers:
            h.flush()
        assert log_path.exists()
        content = log_path.read_text()
        assert "hello world" in content

        # Tidy: clear handlers so this doesn't leak into other tests.
        root = logging.getLogger()
        for h in list(root.handlers):
            root.removeHandler(h)

    def test_configure_logging_without_file_keeps_stderr_only(self):
        from opendesk.cli import _configure_logging

        _configure_logging(None)
        root = logging.getLogger()
        # Exactly one handler, writing to stderr.
        assert len(root.handlers) == 1
        h = root.handlers[0]
        assert isinstance(h, logging.StreamHandler)
        # Tidy.
        for h in list(root.handlers):
            root.removeHandler(h)


# ---------------------------------------------------------------------------
# Permission self-check
# ---------------------------------------------------------------------------


class TestPermissionsModule:
    def test_check_all_is_empty_on_non_macos(self):
        from opendesk.computer import permissions
        if sys.platform == "darwin":
            pytest.skip("macOS-specific behavior tested separately")
        assert permissions.check_all() == []

    def test_report_prints_summary(self, capsys):
        from opendesk.computer.permissions import PermissionStatus, report
        statuses = [
            PermissionStatus(name="Foo", granted=True),
            PermissionStatus(
                name="Bar", granted=False, reason="missing X",
                install_hint="run something",
            ),
        ]
        ok = report(statuses, file=sys.stdout)
        captured = capsys.readouterr()
        assert not ok
        assert "✓ Foo" in captured.out
        assert "✗ Bar" in captured.out
        assert "missing X" in captured.out
        assert "run something" in captured.out

    def test_permission_status_bool(self):
        from opendesk.computer.permissions import PermissionStatus
        assert bool(PermissionStatus(name="x", granted=True)) is True
        assert bool(PermissionStatus(name="x", granted=False)) is False


class TestCLICheck:
    def test_check_subcommand_runs(self):
        r = subprocess.run(
            [sys.executable, "-m", "opendesk.cli", "check"],
            capture_output=True, text=True, timeout=10,
        )
        # On non-macOS the command prints a no-op message and exits 0.
        # On macOS it might exit 1 if perms missing — either is fine here;
        # we just want to confirm the subcommand exists and is wired.
        assert r.returncode in (0, 1)
        assert "opendesk" in (r.stdout + r.stderr).lower() or r.returncode == 0
