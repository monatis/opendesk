"""Tests for the service install file generators.

We test the pure ``_render_*`` functions to verify file content; we don't
actually call ``install_service`` because that would touch the user's real
systemd / launchd / Task Scheduler.  The wrapper logic that runs the system
command is exercised via a smoke test that verifies the CLI subcommand
exists and is wired up.
"""

from __future__ import annotations

from pathlib import Path

from opendesk.remote.service import (
    LAUNCHD_LABEL,
    SERVICE_NAME,
    _render_launchd_plist,
    _render_schtasks_command,
    _render_systemd_unit,
)


class TestSystemdUnit:
    def test_contains_required_fields(self):
        unit = _render_systemd_unit("/usr/bin/python3", port=8423, home=None)
        assert "[Unit]" in unit and "[Service]" in unit and "[Install]" in unit
        assert "Description=opendesk serve" in unit
        assert "/usr/bin/python3 -m opendesk.cli serve" in unit
        assert "--port 8423" in unit
        assert "Restart=on-failure" in unit
        assert "WantedBy=default.target" in unit

    def test_home_override_included(self):
        unit = _render_systemd_unit(
            "/usr/bin/python3", port=9000, home=Path("/var/lib/opendesk"),
        )
        assert "--home /var/lib/opendesk" in unit
        assert "--port 9000" in unit

    def test_home_omitted_when_default(self):
        unit = _render_systemd_unit("/usr/bin/python3", port=8423, home=None)
        assert "--home" not in unit


class TestLaunchdPlist:
    def test_well_formed_xml(self):
        import xml.etree.ElementTree as ET
        plist = _render_launchd_plist("/usr/bin/python3", port=8423, home=None)
        # Strip the DOCTYPE for ElementTree (it doesn't fetch the DTD).
        without_doctype = "\n".join(
            line for line in plist.splitlines()
            if not line.startswith("<!DOCTYPE")
        )
        root = ET.fromstring(without_doctype)
        assert root.tag == "plist"

    def test_contains_label_and_program_args(self):
        plist = _render_launchd_plist("/usr/bin/python3", port=8423, home=None)
        assert f"<string>{LAUNCHD_LABEL}</string>" in plist
        assert "<string>/usr/bin/python3</string>" in plist
        assert "<string>opendesk.cli</string>" in plist
        assert "<string>serve</string>" in plist
        assert "<string>8423</string>" in plist
        assert "<key>RunAtLoad</key>" in plist
        assert "<key>KeepAlive</key>" in plist

    def test_home_override_appears_as_two_strings(self):
        plist = _render_launchd_plist(
            "/usr/bin/python3", port=8423, home=Path("/var/lib/opendesk"),
        )
        assert "<string>--home</string>" in plist
        assert "<string>/var/lib/opendesk</string>" in plist


class TestSchtasksCommand:
    def test_basic_command(self):
        cmd = _render_schtasks_command("C:\\Python\\python.exe", port=8423, home=None)
        assert '"C:\\Python\\python.exe"' in cmd
        assert "-m opendesk.cli serve" in cmd
        assert "--port 8423" in cmd
        assert "--home" not in cmd

    def test_home_override(self):
        cmd = _render_schtasks_command(
            "python.exe", port=8500, home=Path("C:\\opendesk-home"),
        )
        assert '--home "C:\\opendesk-home"' in cmd
        assert "--port 8500" in cmd


class TestCLIWired:
    def test_install_service_subcommand_exists(self):
        import subprocess
        import sys
        result = subprocess.run(
            [sys.executable, "-m", "opendesk.cli", "install-service", "--help"],
            capture_output=True, text=True, timeout=10,
        )
        assert result.returncode == 0
        assert "--port" in result.stdout
        assert "--no-start" in result.stdout

    def test_uninstall_service_subcommand_exists(self):
        import subprocess
        import sys
        result = subprocess.run(
            [sys.executable, "-m", "opendesk.cli", "uninstall-service", "--help"],
            capture_output=True, text=True, timeout=10,
        )
        assert result.returncode == 0
        assert "uninstall-service" in result.stdout

    def test_top_level_help_lists_service_commands(self):
        import subprocess
        import sys
        result = subprocess.run(
            [sys.executable, "-m", "opendesk.cli", "--help"],
            capture_output=True, text=True, timeout=10,
        )
        assert result.returncode == 0
        assert "install-service" in result.stdout
        assert "uninstall-service" in result.stdout
