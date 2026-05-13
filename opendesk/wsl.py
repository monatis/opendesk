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


def windows_user_profile() -> Optional[Path]:
    """Return the WSL path (``/mnt/c/Users/<user>``) for the Windows user.

    Resolved via ``cmd.exe /c echo %USERPROFILE%`` + ``wslpath``.  Returns
    ``None`` outside WSL, or when interop / wslpath aren't available.
    """
    if not is_wsl():
        return None
    try:
        r = subprocess.run(
            ["cmd.exe", "/c", "echo %USERPROFILE%"],
            capture_output=True, text=True, timeout=5,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return None
    if r.returncode != 0:
        return None
    win_path = r.stdout.strip().split("\r")[0].strip()
    if not win_path or "%" in win_path:
        return None
    try:
        r2 = subprocess.run(
            ["wslpath", win_path], capture_output=True, text=True, timeout=5,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return None
    if r2.returncode != 0:
        return None
    p = Path(r2.stdout.strip())
    return p if p.exists() else None


def wslconfig_path() -> Optional[Path]:
    """The full path to the user's ``.wslconfig`` (whether it exists or not)."""
    profile = windows_user_profile()
    return (profile / ".wslconfig") if profile else None


def is_mirrored_mode() -> bool:
    """True when WSL is *currently running* in mirrored networking mode.

    Heuristic: in mirrored mode WSL binds to the Windows network adapters
    directly, so ``hostname -I`` returns one of the Windows LAN IPs (not
    the 172.16/12 NAT range).  We declare mirrored when the local IP set
    intersects the Windows LAN IP set.

    This is the *runtime* state, not the config state — a host can have
    ``networkingMode=mirrored`` set in ``.wslconfig`` but still be running
    NAT'd because it hasn't been restarted yet (``wsl --shutdown``).
    Use :func:`wslconfig_has_mirrored` for the config side.
    """
    if not is_wsl():
        return False
    return bool(set(local_ipv4s()) & set(windows_lan_ipv4s()))


def wslconfig_has_mirrored() -> bool:
    """True iff the user's ``.wslconfig`` has ``networkingMode=mirrored``.

    Doesn't say whether it's *active* — just whether the file is set up
    so a ``wsl --shutdown`` would activate it.
    """
    path = wslconfig_path()
    if path is None or not path.exists():
        return False
    try:
        text = path.read_text()
    except OSError:
        return False
    return _wslconfig_section_has(text, "wsl2", "networkingmode", "mirrored")


def _wslconfig_section_has(
    text: str, section: str, key: str, value: str,
) -> bool:
    """Tiny INI scan: does ``[section]`` contain ``key=value`` (case-insens.)?"""
    section_l = section.lower()
    key_l = key.lower()
    value_l = value.lower()
    current: Optional[str] = None
    for line in text.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith((";", "#")):
            continue
        if stripped.startswith("[") and stripped.endswith("]"):
            current = stripped[1:-1].strip().lower()
            continue
        if current != section_l or "=" not in stripped:
            continue
        k, _, v = stripped.partition("=")
        if k.strip().lower() == key_l and v.strip().lower() == value_l:
            return True
    return False


def render_wslconfig_with_mirrored(existing: str = "") -> str:
    """Return ``.wslconfig`` text guaranteed to contain ``[wsl2] /
    networkingMode=mirrored`` while preserving any other lines.

    Idempotent — calling on already-mirrored content returns it unchanged
    apart from a guaranteed trailing newline.
    """
    if _wslconfig_section_has(existing or "", "wsl2", "networkingmode", "mirrored"):
        if existing and not existing.endswith("\n"):
            existing += "\n"
        return existing

    lines = existing.splitlines() if existing else []
    # Find or create the [wsl2] section.
    wsl2_header_idx: Optional[int] = None
    for i, line in enumerate(lines):
        s = line.strip()
        if s.startswith("[") and s.endswith("]") and s[1:-1].strip().lower() == "wsl2":
            wsl2_header_idx = i
            break

    if wsl2_header_idx is None:
        # Append a new section.
        if lines and lines[-1].strip():
            lines.append("")
        lines.append("[wsl2]")
        lines.append("networkingMode=mirrored")
        return "\n".join(lines) + "\n"

    # Within [wsl2], look for an existing networkingMode line (any value).
    end = len(lines)
    for j in range(wsl2_header_idx + 1, len(lines)):
        s = lines[j].strip()
        if s.startswith("[") and s.endswith("]"):
            end = j
            break

    for j in range(wsl2_header_idx + 1, end):
        s = lines[j].strip()
        if "=" not in s:
            continue
        k, _, _ = s.partition("=")
        if k.strip().lower() == "networkingmode":
            lines[j] = "networkingMode=mirrored"
            return "\n".join(lines) + "\n"

    # No networkingMode line — insert right after the header.
    lines.insert(wsl2_header_idx + 1, "networkingMode=mirrored")
    return "\n".join(lines) + "\n"


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
    """Run ``commands`` on the Windows host with a UAC prompt, *and wait*.

    Writes the commands to a one-shot ``.ps1`` in the Windows user's TEMP
    directory, then launches it via ``Start-Process -Verb RunAs -Wait
    -PassThru`` so we can propagate the inner process's exit code.

    Returns:
        0 on success; non-zero if the elevated commands themselves failed,
        if the UAC prompt was declined, or if interop / wslpath / temp
        directory weren't available.
    """
    if not is_wsl():
        raise RuntimeError("not running inside WSL")

    profile = windows_user_profile()
    if profile is None:
        return 127
    temp_dir = profile / "AppData" / "Local" / "Temp"
    if not temp_dir.exists():
        return 127

    import time as _time
    script_path = temp_dir / f"opendesk-uac-{int(_time.time() * 1000)}.ps1"

    # The script: stop on any error, run the commands in order, surface the
    # right exit code.  $ErrorActionPreference = 'Stop' ensures the script
    # exits non-zero when (e.g.) netsh complains about a duplicate rule.
    body = "\r\n".join([
        "$ErrorActionPreference = 'Stop'",
        "try {",
        *(f"    {c}" for c in commands),
        "    exit 0",
        "} catch {",
        "    Write-Error $_",
        "    exit 1",
        "}",
    ]) + "\r\n"

    try:
        # utf-8-sig (BOM) so PowerShell handles non-ASCII reliably.
        script_path.write_text(body, encoding="utf-8-sig")
    except OSError:
        return 127

    try:
        win_path_r = subprocess.run(
            ["wslpath", "-w", str(script_path)],
            capture_output=True, text=True, timeout=5,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        script_path.unlink(missing_ok=True)
        return 127
    if win_path_r.returncode != 0:
        script_path.unlink(missing_ok=True)
        return 127
    win_path = win_path_r.stdout.strip()

    # -Wait stalls until the elevated process finishes; -PassThru returns
    # the process object so we can read ExitCode.  Without these, the outer
    # powershell returns immediately and we never know if the commands ran.
    outer = (
        "$p = Start-Process powershell -Verb RunAs -Wait -PassThru "
        "-ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File',"
        f"'{win_path}'; exit $p.ExitCode"
    )
    try:
        r = subprocess.run(
            ["powershell.exe", "-NoProfile", "-Command", outer],
            capture_output=True, text=True, timeout=180,
        )
        return r.returncode
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return 127
    finally:
        script_path.unlink(missing_ok=True)
