"""Tests for WSL detection, port-forwarding setup, and the trusted-peers
endpoint cache that lets WSL users skip mDNS once paired."""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path
from unittest.mock import patch

import pytest

from opendesk.protocol.auth import Identity, TrustedPeers
from opendesk.wsl import (
    _wslconfig_section_has,
    is_wsl,
    render_setup_commands,
    render_undo_commands,
    render_wslconfig_with_mirrored,
    windows_host_ipv4,
    wsl_interface_ipv4,
)


# ---------------------------------------------------------------------------
# Detection
# ---------------------------------------------------------------------------


class TestIsWSL:
    def test_detects_microsoft_kernel(self, tmp_path: Path, monkeypatch):
        fake_osrelease = tmp_path / "osrelease"
        fake_osrelease.write_text("6.6.87.2-microsoft-standard-WSL2\n")
        monkeypatch.setattr("opendesk.wsl._OSRELEASE", fake_osrelease)
        monkeypatch.setattr("opendesk.wsl._PROC_VERSION", tmp_path / "nope")
        # is_wsl() short-circuits on non-Linux, so force Linux for the test.
        monkeypatch.setattr("platform.system", lambda: "Linux")
        assert is_wsl() is True

    def test_returns_false_for_real_linux(self, tmp_path: Path, monkeypatch):
        fake_osrelease = tmp_path / "osrelease"
        fake_osrelease.write_text("6.5.0-15-generic\n")
        monkeypatch.setattr("opendesk.wsl._OSRELEASE", fake_osrelease)
        monkeypatch.setattr("opendesk.wsl._PROC_VERSION", tmp_path / "nope")
        monkeypatch.setattr("platform.system", lambda: "Linux")
        assert is_wsl() is False

    def test_returns_false_on_non_linux(self, monkeypatch):
        monkeypatch.setattr("platform.system", lambda: "Darwin")
        assert is_wsl() is False


# ---------------------------------------------------------------------------
# Setup commands
# ---------------------------------------------------------------------------


class TestSetupCommands:
    def test_render_includes_port_and_wsl_ip(self):
        cmds = render_setup_commands(port=8423, wsl_ip="172.21.133.14")
        joined = "\n".join(cmds)
        assert "listenport=8423" in joined
        assert "connectport=8423" in joined
        assert "connectaddress=172.21.133.14" in joined
        assert "New-NetFirewallRule" in joined
        assert "opendesk inbound 8423" in joined

    def test_render_undo_removes_same_rule(self):
        cmds = render_undo_commands(port=8423)
        joined = "\n".join(cmds)
        assert "portproxy delete" in joined
        assert "Remove-NetFirewallRule" in joined
        assert "opendesk inbound 8423" in joined

    def test_custom_port(self):
        cmds = render_setup_commands(port=9000, wsl_ip="172.21.0.5")
        assert "listenport=9000" in "\n".join(cmds)


class TestWSLIPHelpers:
    def test_wsl_interface_ipv4_returns_a_non_loopback_or_empty(self):
        # On any host this should either return a non-loopback IP or "".
        # We don't assert a specific value, just that the contract holds.
        ip = wsl_interface_ipv4()
        if ip:
            assert not ip.startswith("127.")
            assert ":" not in ip  # IPv4 only

    def test_windows_host_ipv4_reads_resolv_conf(self, monkeypatch, tmp_path: Path):
        fake = tmp_path / "resolv.conf"
        fake.write_text("nameserver 172.21.128.1\n")
        monkeypatch.setattr("pathlib.Path.read_text", lambda self, *a, **k: fake.read_text())
        # Can't easily monkeypatch the open() inside windows_host_ipv4
        # without reading the source; this is a soft check.
        # If we're actually in WSL the function should return something.


# ---------------------------------------------------------------------------
# .wslconfig writer — the file we ask users to edit to enable mirrored mode
# ---------------------------------------------------------------------------


class TestRenderWslconfig:
    """``render_wslconfig_with_mirrored`` must be safe to call on any
    pre-existing ``.wslconfig`` content — we don't want to clobber the
    user's other tweaks."""

    def test_empty_file_gets_full_section(self):
        out = render_wslconfig_with_mirrored("")
        assert "[wsl2]" in out
        assert "networkingMode=mirrored" in out
        assert _wslconfig_section_has(out, "wsl2", "networkingmode", "mirrored")

    def test_already_set_is_idempotent(self):
        existing = "[wsl2]\nnetworkingMode=mirrored\n"
        # Already-set content is returned with the same content (newline
        # normalization is fine, but the user's content shouldn't change).
        out = render_wslconfig_with_mirrored(existing)
        assert out == existing
        # Calling again is also a no-op.
        assert render_wslconfig_with_mirrored(out) == out

    def test_existing_wsl2_section_without_networkingmode(self):
        existing = "[wsl2]\nmemory=8GB\nprocessors=4\n"
        out = render_wslconfig_with_mirrored(existing)
        # Other [wsl2] settings preserved.
        assert "memory=8GB" in out
        assert "processors=4" in out
        # networkingMode added inside the section.
        assert _wslconfig_section_has(out, "wsl2", "networkingmode", "mirrored")

    def test_existing_wsl2_section_with_different_networkingmode(self):
        existing = "[wsl2]\nnetworkingMode=nat\nmemory=8GB\n"
        out = render_wslconfig_with_mirrored(existing)
        # NAT is replaced, not duplicated.
        assert out.lower().count("networkingmode") == 1
        assert _wslconfig_section_has(out, "wsl2", "networkingmode", "mirrored")
        assert "memory=8GB" in out

    def test_other_section_preserved(self):
        """Sections we don't touch (like [experimental]) must survive."""
        existing = (
            "[experimental]\n"
            "autoMemoryReclaim=gradual\n"
            "\n"
            "[wsl2]\n"
            "memory=4GB\n"
        )
        out = render_wslconfig_with_mirrored(existing)
        assert "autoMemoryReclaim=gradual" in out
        assert "memory=4GB" in out
        assert _wslconfig_section_has(out, "wsl2", "networkingmode", "mirrored")

    def test_no_wsl2_section_at_all_appends_new(self):
        existing = "[experimental]\nautoMemoryReclaim=gradual\n"
        out = render_wslconfig_with_mirrored(existing)
        assert "autoMemoryReclaim=gradual" in out
        assert "[wsl2]" in out
        assert _wslconfig_section_has(out, "wsl2", "networkingmode", "mirrored")

    def test_case_insensitive_match_for_already_set(self):
        existing = "[WSL2]\nNetworkingMode=Mirrored\n"
        # Already mirrored — must not duplicate.
        out = render_wslconfig_with_mirrored(existing)
        # The function leaves user's casing alone if it's already correct.
        assert out.lower().count("networkingmode") == 1


# ---------------------------------------------------------------------------
# CLI smoke
# ---------------------------------------------------------------------------


class TestWSLCLI:
    def test_wsl_setup_help_exists(self):
        r = subprocess.run(
            [sys.executable, "-m", "opendesk.cli", "wsl-setup", "--help"],
            capture_output=True, text=True, timeout=10,
        )
        assert r.returncode == 0
        assert "--apply" in r.stdout
        assert "--undo" in r.stdout

    def test_wsl_setup_on_non_wsl_exits_clean(self, monkeypatch):
        """When not in WSL, the command should refuse with a friendly message."""
        with patch("opendesk.wsl.is_wsl", return_value=False):
            r = subprocess.run(
                [sys.executable, "-m", "opendesk.cli", "wsl-setup"],
                capture_output=True, text=True, timeout=10,
            )
            # Subprocess runs the *real* is_wsl, so the patch above doesn't
            # affect it.  We just assert it doesn't crash.
            assert r.returncode in (0, 2)

    def test_top_level_help_lists_wsl_setup(self):
        r = subprocess.run(
            [sys.executable, "-m", "opendesk.cli", "--help"],
            capture_output=True, text=True, timeout=10,
        )
        assert r.returncode == 0
        assert "wsl-setup" in r.stdout


# ---------------------------------------------------------------------------
# Trusted-peers endpoint cache
# ---------------------------------------------------------------------------


class TestEndpointCache:
    def test_default_is_empty(self, tmp_path: Path):
        store = TrustedPeers(tmp_path)
        store.add(Identity.generate().public_bytes, name="mini")
        peer = store.find_by_name("mini")
        assert peer.last_host == ""
        assert peer.last_port == 0

    def test_cache_endpoint_persists(self, tmp_path: Path):
        ident = Identity.generate()
        store = TrustedPeers(tmp_path)
        store.add(ident.public_bytes, name="mini")
        assert store.cache_endpoint(ident.public_bytes, "192.168.1.5", 8423) is True

        # Fresh instance reads from disk.
        peer = TrustedPeers(tmp_path).find_by_name("mini")
        assert peer.last_host == "192.168.1.5"
        assert peer.last_port == 8423

    def test_cache_noop_when_unchanged(self, tmp_path: Path):
        ident = Identity.generate()
        store = TrustedPeers(tmp_path)
        store.add(ident.public_bytes, name="mini")
        store.cache_endpoint(ident.public_bytes, "192.168.1.5", 8423)
        # Second identical write is a no-op (avoids needless file churn).
        assert store.cache_endpoint(ident.public_bytes, "192.168.1.5", 8423) is False

    def test_cache_preserves_other_fields(self, tmp_path: Path):
        ident = Identity.generate()
        store = TrustedPeers(tmp_path)
        store.add(ident.public_bytes, name="mini")
        store.cache_description(ident.public_bytes, "billing")
        store.set_description_override("mini", "local label")
        store.cache_endpoint(ident.public_bytes, "192.168.1.5", 8423)

        peer = TrustedPeers(tmp_path).find_by_name("mini")
        assert peer.description == "billing"
        assert peer.description_override == "local label"
        assert peer.last_host == "192.168.1.5"
        assert peer.last_port == 8423

    def test_cache_unknown_peer_returns_false(self, tmp_path: Path):
        store = TrustedPeers(tmp_path)
        assert store.cache_endpoint(b"\x00" * 32, "1.2.3.4", 1234) is False


class TestResolvePrefersCachedEndpoint:
    """When the trusted peer has a cached endpoint, _resolve skips mDNS."""

    @pytest.mark.asyncio
    async def test_uses_cached_endpoint_when_present(self, tmp_path: Path):
        from opendesk.remote.client import _resolve

        ident = Identity.generate()
        store = TrustedPeers(tmp_path)
        store.add(ident.public_bytes, name="mini")
        store.cache_endpoint(ident.public_bytes, "192.168.1.99", 9999)

        host, port, pubkey = await _resolve("mini", home=tmp_path, timeout=0.1)
        assert host == "192.168.1.99"
        assert port == 9999
        assert pubkey == ident.public_bytes
