"""Install / uninstall ``opendesk serve`` as a user-scoped system service.

* Linux  — ``systemd --user`` unit at ``~/.config/systemd/user/opendesk.service``
* macOS  — launchd agent at ``~/Library/LaunchAgents/com.opendesk.serve.plist``
* Windows — Task Scheduler entry registered as the current user

All variants run *as the current user*, so opendesk inherits the user's
desktop session and the accessibility / screen-recording permissions
granted to that user.  No ``sudo``, no machine-wide install.

Each platform helper is split into:

* ``_render_*`` — pure function returning the file content; safe to unit-test.
* ``_install_*`` — writes the file and (optionally) starts the service.
* ``_uninstall_*`` — stops the service and removes the file.
"""

from __future__ import annotations

import platform
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Optional


SERVICE_NAME = "opendesk"
LAUNCHD_LABEL = "com.opendesk.serve"
DEFAULT_PORT = 8423


@dataclass
class ServiceInstallation:
    """Result of a service install — where the file landed and whether the
    system service manager was reachable to register / start it."""

    path: Path
    started: bool
    manager: str  # "systemd" | "launchd" | "schtasks"


# ---------------------------------------------------------------------------
# Public entry points
# ---------------------------------------------------------------------------


def install_service(
    *,
    home: Optional[Path] = None,
    port: int = DEFAULT_PORT,
    python: Optional[str] = None,
    autostart: bool = True,
) -> ServiceInstallation:
    """Install a platform-appropriate user service for ``opendesk serve``.

    ``python`` defaults to the current interpreter path; explicit override
    is useful when installing for a different venv.
    """
    py = python or sys.executable
    system = platform.system()
    if system == "Linux":
        return _install_systemd(home, port, py, autostart)
    if system == "Darwin":
        return _install_launchd(home, port, py, autostart)
    if system == "Windows":
        return _install_schtasks(home, port, py, autostart)
    raise RuntimeError(f"Service install not supported on platform: {system!r}")


def uninstall_service() -> bool:
    """Remove the installed service.  Returns ``True`` if anything was removed."""
    system = platform.system()
    if system == "Linux":
        return _uninstall_systemd()
    if system == "Darwin":
        return _uninstall_launchd()
    if system == "Windows":
        return _uninstall_schtasks()
    raise RuntimeError(f"Service uninstall not supported on platform: {system!r}")


# ---------------------------------------------------------------------------
# Linux — systemd user unit
# ---------------------------------------------------------------------------


def _systemd_unit_path() -> Path:
    return Path.home() / ".config" / "systemd" / "user" / f"{SERVICE_NAME}.service"


def _render_systemd_unit(python: str, port: int, home: Optional[Path]) -> str:
    home_arg = f" --home {home}" if home else ""
    return f"""[Unit]
Description=opendesk serve — control this machine from a paired controller
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart={python} -m opendesk.cli serve --port {port}{home_arg}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
"""


def _install_systemd(
    home: Optional[Path], port: int, python: str, autostart: bool,
) -> ServiceInstallation:
    path = _systemd_unit_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(_render_systemd_unit(python, port, home))

    started = False
    if autostart and shutil.which("systemctl"):
        _run(["systemctl", "--user", "daemon-reload"])
        r = _run(["systemctl", "--user", "enable", "--now", SERVICE_NAME])
        started = r.returncode == 0
    return ServiceInstallation(path=path, started=started, manager="systemd")


def _uninstall_systemd() -> bool:
    path = _systemd_unit_path()
    if not path.exists():
        return False
    if shutil.which("systemctl"):
        _run(["systemctl", "--user", "disable", "--now", SERVICE_NAME])
    path.unlink()
    return True


# ---------------------------------------------------------------------------
# macOS — launchd user agent
# ---------------------------------------------------------------------------


def _launchd_plist_path() -> Path:
    return Path.home() / "Library" / "LaunchAgents" / f"{LAUNCHD_LABEL}.plist"


def _render_launchd_plist(python: str, port: int, home: Optional[Path]) -> str:
    home_dir = Path(home) if home else Path.home() / ".opendesk"
    args = [python, "-m", "opendesk.cli", "serve", "--port", str(port)]
    if home is not None:
        args.extend(["--home", str(home)])
    args_xml = "\n        ".join(f"<string>{_xml(a)}</string>" for a in args)
    return f"""<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LAUNCHD_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        {args_xml}
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{_xml(str(home_dir / 'serve.log'))}</string>
    <key>StandardErrorPath</key>
    <string>{_xml(str(home_dir / 'serve.err'))}</string>
</dict>
</plist>
"""


def _install_launchd(
    home: Optional[Path], port: int, python: str, autostart: bool,
) -> ServiceInstallation:
    path = _launchd_plist_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(_render_launchd_plist(python, port, home))

    started = False
    if autostart and shutil.which("launchctl"):
        # Idempotent: try unload first so a stale registration doesn't block.
        _run(["launchctl", "unload", str(path)])
        r = _run(["launchctl", "load", "-w", str(path)])
        started = r.returncode == 0
    return ServiceInstallation(path=path, started=started, manager="launchd")


def _uninstall_launchd() -> bool:
    path = _launchd_plist_path()
    if not path.exists():
        return False
    if shutil.which("launchctl"):
        _run(["launchctl", "unload", "-w", str(path)])
    path.unlink()
    return True


# ---------------------------------------------------------------------------
# Windows — Task Scheduler
# ---------------------------------------------------------------------------


def _render_schtasks_command(python: str, port: int, home: Optional[Path]) -> str:
    """Return the ``/tr`` argument for schtasks (the command line to run)."""
    home_arg = f' --home "{home}"' if home else ""
    return f'"{python}" -m opendesk.cli serve --port {port}{home_arg}'


def _install_schtasks(
    home: Optional[Path], port: int, python: str, autostart: bool,
) -> ServiceInstallation:
    if not shutil.which("schtasks"):
        raise RuntimeError("schtasks.exe not found — Task Scheduler unavailable")
    tr = _render_schtasks_command(python, port, home)
    r = _run([
        "schtasks", "/create",
        "/tn", SERVICE_NAME,
        "/tr", tr,
        "/sc", "onlogon",
        "/rl", "limited",
        "/f",
    ])
    if r.returncode != 0:
        raise RuntimeError(f"schtasks /create failed: {r.stderr.strip() or r.stdout.strip()}")
    started = False
    if autostart:
        _run(["schtasks", "/run", "/tn", SERVICE_NAME])
        started = True
    return ServiceInstallation(
        path=Path(f"TaskScheduler:{SERVICE_NAME}"), started=started, manager="schtasks",
    )


def _uninstall_schtasks() -> bool:
    if not shutil.which("schtasks"):
        return False
    r = _run(["schtasks", "/delete", "/tn", SERVICE_NAME, "/f"])
    return r.returncode == 0


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _run(cmd: list[str]) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True, timeout=30)


def _xml(s: str) -> str:
    """Minimal XML escape for plist string content."""
    return (
        s.replace("&", "&amp;")
         .replace("<", "&lt;")
         .replace(">", "&gt;")
    )
