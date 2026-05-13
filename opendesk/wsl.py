"""WSL (Windows Subsystem for Linux) detection and helpers.

WSL2 runs each distro inside a NAT'd virtual network — multicast traffic
(mDNS) doesn't cross either direction by default, and inbound TCP from the
host LAN doesn't reach WSL listeners unless a port-proxy is configured on
the Windows side.

This module surfaces those facts to users at the points where they'd
otherwise hit a wall: pairing, serving, discovering, and the local app.
"""

from __future__ import annotations

import platform
import socket
import subprocess
from pathlib import Path
from typing import Optional


_OSRELEASE = Path("/proc/sys/kernel/osrelease")
_PROC_VERSION = Path("/proc/version")


def is_wsl() -> bool:
    """Return ``True`` if this process is running inside WSL.

    Detection: look for ``microsoft`` (case-insensitive) in
    ``/proc/sys/kernel/osrelease``.  Set on every WSL kernel build (e.g.
    ``6.6.87.2-microsoft-standard-WSL2``).  Falls back to ``/proc/version``
    on older builds.
    """
    if platform.system() != "Linux":
        return False
    for path in (_OSRELEASE, _PROC_VERSION):
        try:
            text = path.read_text().lower()
        except OSError:
            continue
        if "microsoft" in text or "wsl" in text:
            return True
    return False


def wsl_interface_ipv4() -> str:
    """Best-effort guess at the WSL interface's IPv4 address.

    Returns ``""`` when we can't determine one — callers should print a
    helpful error rather than guess.
    """
    ips = local_ipv4s()
    return ips[0] if ips else ""


def local_ipv4s() -> list[str]:
    """Return non-loopback IPv4s bound on this host.

    Uses ``hostname -I`` (Linux/WSL) and falls back to
    ``socket.gethostbyname_ex``.  Loopback and link-local are filtered
    out.  Order matches the OS's preference where possible.
    """
    out: list[str] = []
    seen: set[str] = set()
    try:
        r = subprocess.run(
            ["hostname", "-I"], capture_output=True, text=True, timeout=5,
        )
        if r.returncode == 0:
            for ip in r.stdout.split():
                if ip and not ip.startswith("127.") and ":" not in ip and ip not in seen:
                    seen.add(ip)
                    out.append(ip)
    except (FileNotFoundError, subprocess.TimeoutExpired):
        pass
    if not out:
        try:
            _, _, addrs = socket.gethostbyname_ex(socket.gethostname())
            for ip in addrs:
                if ip and not ip.startswith("127.") and ip not in seen:
                    seen.add(ip)
                    out.append(ip)
        except OSError:
            pass
    return out


def windows_host_ipv4() -> Optional[str]:
    """Return the Windows host's IPv4 *as seen from inside WSL*.

    Read from ``/etc/resolv.conf`` (default WSL2 config sets the nameserver
    to the host).  This is the host's WSL-facing adapter, not the host's
    LAN IP — for that, see :func:`windows_lan_ipv4s`.
    """
    try:
        text = Path("/etc/resolv.conf").read_text()
    except OSError:
        return None
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("nameserver"):
            parts = line.split()
            if len(parts) == 2 and parts[1].count(".") == 3:
                return parts[1]
    return None


def windows_lan_ipv4s() -> list[str]:
    """The Windows host's *real* LAN IPv4s — what other devices need.

    Calls ``ipconfig.exe`` via WSL interop and pulls out IPv4 addresses,
    skipping link-local (``169.254.x.x``), loopback, and the WSL-internal
    range so what's left is what controllers on the LAN actually type.

    Returns an empty list when not in WSL or when interop isn't available.
    """
    if not is_wsl():
        return []
    try:
        r = subprocess.run(
            ["ipconfig.exe"],
            capture_output=True, text=True, timeout=10,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return []
    if r.returncode != 0:
        return []

    ips: list[str] = []
    seen: set[str] = set()
    for line in r.stdout.splitlines():
        if "IPv4" not in line or ":" not in line:
            continue
        # Format: "   IPv4 Address. . . . . . . . . . . : 192.168.1.5"
        ip = line.rsplit(":", 1)[-1].strip().rstrip("(Preferred)").strip()
        # Sometimes there's a trailing "(Preferred)".  Normalise.
        ip = ip.split("(", 1)[0].strip()
        if not _is_plausible_lan_ipv4(ip):
            continue
        if ip in seen:
            continue
        seen.add(ip)
        ips.append(ip)
    return ips


def _is_plausible_lan_ipv4(ip: str) -> bool:
    """True for an IPv4 a controller on the same LAN could conceivably use."""
    if ip.count(".") != 3:
        return False
    parts = ip.split(".")
    try:
        octets = [int(p) for p in parts]
    except ValueError:
        return False
    if any(not (0 <= o <= 255) for o in octets):
        return False
    # Skip loopback, link-local, multicast, WSL-internal NAT range.
    if octets[0] == 127:
        return False
    if octets[0] == 169 and octets[1] == 254:
        return False
    if octets[0] >= 224:
        return False
    # 172.16.0.0/12 — covers the WSL2 NAT range we want to *exclude*.
    if octets[0] == 172 and 16 <= octets[1] <= 31:
        return False
    return True


ADVISORY = (
    "NOTE: this process is running inside WSL.  WSL2's virtual network "
    "blocks mDNS in both directions, so peer discovery won't see hosts on "
    "the real LAN, and other devices can't see this WSL instance.\n"
    "  Quick fix (Windows 11 + WSL >= 2.0.0):  add the line\n"
    "        [wsl2]\n"
    "        networkingMode=mirrored\n"
    "    to %UserProfile%\\.wslconfig, then run `wsl --shutdown` from PowerShell.\n"
    "  Alternative (older WSL):  run `opendesk wsl-setup` from this shell\n"
    "    to set up Windows-side port forwarding.  Or pair by IP with\n"
    "    `opendesk pair-with <host-ip> <code>` — that path doesn't need mDNS."
)


def print_advisory_if_wsl(file=None) -> bool:
    """If running in WSL, print :data:`ADVISORY` and return ``True``."""
    import sys
    if not is_wsl():
        return False
    stream = file or sys.stderr
    print(ADVISORY, file=stream)
    return True


# ---------------------------------------------------------------------------
# Setup helper — Windows-side port forwarding for WSL listeners
# ---------------------------------------------------------------------------


def render_setup_commands(port: int, wsl_ip: str) -> list[str]:
    """Return the PowerShell commands to make a WSL listener LAN-reachable."""
    return [
        f"netsh interface portproxy add v4tov4 "
        f"listenport={port} listenaddress=0.0.0.0 "
        f"connectport={port} connectaddress={wsl_ip}",
        f"New-NetFirewallRule -DisplayName 'opendesk inbound {port}' "
        f"-Direction Inbound -LocalPort {port} -Protocol TCP -Action Allow "
        f"-Profile Private",
    ]


def render_undo_commands(port: int) -> list[str]:
    return [
        f"netsh interface portproxy delete v4tov4 "
        f"listenport={port} listenaddress=0.0.0.0",
        f"Remove-NetFirewallRule -DisplayName 'opendesk inbound {port}'",
    ]


def run_via_uac(commands: list[str]) -> int:
    """Try to run ``commands`` on the Windows host with a UAC prompt.

    Joins them with ``;`` and invokes through ``powershell.exe`` available
    via WSL's Windows interop.  Returns the exit code; non-zero means the
    elevation didn't happen (UAC declined or powershell.exe unreachable).
    """
    if not is_wsl():
        raise RuntimeError("not running inside WSL")
    script = "; ".join(commands)
    # -Verb RunAs triggers UAC.  -NoProfile keeps it fast.
    outer = (
        "Start-Process powershell -Verb RunAs -ArgumentList "
        f"'-NoProfile', '-Command', \"{script}\""
    )
    r = subprocess.run(
        ["powershell.exe", "-NoProfile", "-Command", outer],
        capture_output=True, text=True, timeout=30,
    )
    return r.returncode
