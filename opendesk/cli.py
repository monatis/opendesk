"""opendesk CLI — setup and utility commands."""

from __future__ import annotations

import asyncio
import logging
import shutil
import subprocess
import sys
from pathlib import Path


def _find_mcp_binary() -> str:
    """Return the absolute path to opendesk-mcp in the current Python environment."""
    # Prefer the binary next to the current Python executable (same venv/conda env)
    import pathlib
    bin_dir = pathlib.Path(sys.executable).parent
    for name in ("opendesk-mcp", "opendesk-mcp.exe"):
        candidate = bin_dir / name
        if candidate.exists():
            return str(candidate)

    # Fall back to PATH
    found = shutil.which("opendesk-mcp")
    if found:
        return found

    raise RuntimeError(
        "opendesk-mcp not found. Make sure you installed with:\n"
        "  pip install 'opendesk[core,mcp]'"
    )


def cmd_install(scope: str = "user") -> None:
    """Register opendesk-mcp with Claude Code."""
    claude_bin = shutil.which("claude")
    if not claude_bin:
        print(
            "ERROR: 'claude' command not found.\n"
            "Install Claude Code first: https://claude.ai/code",
            file=sys.stderr,
        )
        sys.exit(1)

    mcp_path = _find_mcp_binary()

    # Remove existing registration if any (ignore errors)
    subprocess.run(
        [claude_bin, "mcp", "remove", "opendesk"],
        capture_output=True,
    )

    result = subprocess.run(
        [claude_bin, "mcp", "add", "opendesk", f"--scope={scope}", "--", mcp_path],
        capture_output=True,
        text=True,
    )

    if result.returncode != 0:
        print(f"ERROR: {result.stderr.strip()}", file=sys.stderr)
        sys.exit(1)

    print(f"opendesk MCP server registered ({scope}).")
    print(f"  Binary: {mcp_path}")
    print("Start a Claude Code conversation and say 'take a screenshot' to verify.")


def cmd_uninstall() -> None:
    """Remove opendesk MCP registration from Claude Code."""
    claude_bin = shutil.which("claude")
    if not claude_bin:
        print("ERROR: 'claude' command not found.", file=sys.stderr)
        sys.exit(1)

    result = subprocess.run(
        [claude_bin, "mcp", "remove", "opendesk"],
        capture_output=True,
        text=True,
    )

    if result.returncode != 0:
        print(f"ERROR: {result.stderr.strip()}", file=sys.stderr)
        sys.exit(1)

    print("opendesk MCP server removed from Claude Code.")
    print("To fully uninstall the package: pip uninstall opendesk")


# ---------------------------------------------------------------------------
# Remote — pair / serve / discover / connect / peers
# ---------------------------------------------------------------------------


def _remote_imports():
    """Lazy-import the remote stack so plain ``opendesk install`` doesn't need it."""
    try:
        from opendesk.protocol.auth import Identity, TrustedPeers
        from opendesk.protocol.auth.identity import generate_pairing_code
        from opendesk.remote.server import OpendeskServer, DEFAULT_PORT
    except ImportError as exc:
        print(
            f"ERROR: opendesk remote features require 'opendesk[remote]': {exc}",
            file=sys.stderr,
        )
        sys.exit(2)
    return Identity, TrustedPeers, generate_pairing_code, OpendeskServer, DEFAULT_PORT


def cmd_pair(args) -> None:
    """Run a one-shot pairing session and persist the new peer."""
    Identity, TrustedPeers, generate_pairing_code, OpendeskServer, DEFAULT_PORT = _remote_imports()
    from opendesk.computer import LocalComputer

    logging.basicConfig(level=logging.INFO, format="%(message)s")
    home = Path(args.home).expanduser() if args.home else None
    code = args.code or generate_pairing_code()

    identity = Identity.load_or_create(home)
    trusted = TrustedPeers(home)
    computer = LocalComputer()

    async def _run() -> None:
        server = OpendeskServer(
            computer, identity, trusted,
            host=args.host, port=args.port,
            advertise_mdns=not args.no_mdns,
        )
        await server.start()
        print()
        print("┌──────────────────────────────────────────────┐")
        print("│  opendesk pairing                            │")
        print(f"│  port:        {args.port:<31}│")
        print(f"│  fingerprint: {':'.join(identity.public_bytes.hex()[i:i+4] for i in range(0, 16, 4)):<31}│")
        print("│                                              │")
        print(f"│   pairing code:   {code:<27}│")
        print("│                                              │")
        print("│  Run on the controller:                      │")
        print(f"│    opendesk pair-with <host> {code}            │")
        print("└──────────────────────────────────────────────┘")
        print()

        new_pub = await server.enable_pairing(code, timeout=args.timeout)
        if new_pub is None:
            print(f"ERROR: no peer paired within {args.timeout}s.", file=sys.stderr)
            await server.aclose()
            sys.exit(1)

        peer = trusted.find(new_pub)
        name = peer.name if peer else "?"
        print(f"✓ Paired with {name} ({':'.join(new_pub.hex()[i:i+4] for i in range(0, 16, 4))})")
        await server.aclose()

    try:
        asyncio.run(_run())
    except KeyboardInterrupt:
        print("\nPairing cancelled.")
        sys.exit(130)


def cmd_pair_with(args) -> None:
    """Master-side counterpart: connect to a peer running ``opendesk pair`` and exchange keys."""
    Identity, TrustedPeers, _, _, _ = _remote_imports()
    from opendesk.remote.client import pair_with

    home = Path(args.home).expanduser() if args.home else None

    async def _run() -> None:
        remote, server_pub = await pair_with(
            args.host, args.port, args.code, home=home, name=args.name,
        )
        await remote.aclose()
        fp = ":".join(server_pub.hex()[i : i + 4] for i in range(0, 16, 4))
        name = args.name or f"peer-{server_pub.hex()[:6]}"
        print(f"✓ Paired with {name} ({fp})")
        print(f"  Now reachable as: opendesk connect {name}")

    try:
        asyncio.run(_run())
    except KeyboardInterrupt:
        sys.exit(130)
    except Exception as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        sys.exit(1)


def cmd_serve(args) -> None:
    """Long-running serve loop.  Accepts paired peers only."""
    _, TrustedPeers, _, OpendeskServer, DEFAULT_PORT = _remote_imports()
    from opendesk.protocol.auth import Identity
    from opendesk.computer import LocalComputer

    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
    home = Path(args.home).expanduser() if args.home else None
    identity = Identity.load_or_create(home)
    trusted = TrustedPeers(home)

    if not trusted.list():
        print(
            "ERROR: no trusted peers yet.  Run `opendesk pair` first.",
            file=sys.stderr,
        )
        sys.exit(2)

    async def _run() -> None:
        server = OpendeskServer(
            LocalComputer(), identity, trusted,
            host=args.host, port=args.port,
            advertise_mdns=not args.no_mdns,
        )
        await server.start()
        fp = ":".join(identity.public_bytes.hex()[i : i + 4] for i in range(0, 16, 4))
        print(f"opendesk serve listening on {args.host}:{server.port}  fp={fp}")
        try:
            await server.serve_forever()
        finally:
            await server.aclose()

    try:
        asyncio.run(_run())
    except KeyboardInterrupt:
        print("\nShutting down.")
        sys.exit(0)


def cmd_discover(args) -> None:
    """List opendesk peers visible on the LAN."""
    try:
        from opendesk.remote.discovery import discover
    except ImportError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        sys.exit(2)

    async def _run():
        peers = await discover(timeout=args.timeout)
        if not peers:
            print("No opendesk peers found on the LAN.")
            return
        print(f"{'NAME':<32}  {'ADDR':<22}  FINGERPRINT")
        for p in peers:
            print(f"{p.name:<32}  {p.host + ':' + str(p.port):<22}  {p.fingerprint}")

    asyncio.run(_run())


def cmd_connect(args) -> None:
    """Smoke-test connection — pull capabilities + a screenshot."""
    try:
        from opendesk.remote.client import connect
    except ImportError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        sys.exit(2)

    async def _run():
        home = Path(args.home).expanduser() if args.home else None
        remote = await connect(args.peer, home=home)
        try:
            caps = remote.capabilities()
            print(f"Connected to {args.peer}  backend={caps.backend}")
            print(f"Capabilities: {sorted(c.value for c in caps.capabilities)}")
            if args.screenshot:
                pixmap = await remote.capture()
                out = Path(args.screenshot).expanduser()
                out.write_bytes(pixmap.data)
                print(f"Saved screenshot ({pixmap.width}x{pixmap.height}) → {out}")
        finally:
            await remote.aclose()

    try:
        asyncio.run(_run())
    except Exception as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        sys.exit(1)


def cmd_sessions(args) -> None:
    """Inspect / kill active opendesk-serve sessions via local IPC."""
    try:
        from opendesk.remote.admin import AdminClient, AdminError
    except ImportError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        sys.exit(2)

    home = Path(args.home).expanduser() if args.home else None

    async def _run() -> int:
        try:
            client = await AdminClient.connect(home=home)
        except AdminError as exc:
            print(f"ERROR: {exc}", file=sys.stderr)
            return 1
        try:
            action = args.sessions_cmd or "list"
            if action == "list":
                sessions = await client.list_sessions()
                if not sessions:
                    print("No active sessions.")
                    return 0
                print(f"{'ID':<10}  {'PEER':<22}  {'FROM':<22}  {'AGE':<8}  MODE")
                for s in sessions:
                    age = _format_age(s.get("age_seconds", 0))
                    print(
                        f"{s['id']:<10}  {s['peer_name']:<22}  "
                        f"{s['remote_addr']:<22}  {age:<8}  {s['mode']}"
                    )
                return 0
            if action == "kill":
                if args.all:
                    n = await client.kill_all()
                    print(f"Killed {n} session(s).")
                    return 0
                if not args.id:
                    print("ERROR: provide an ID or --all", file=sys.stderr)
                    return 2
                ok = await client.kill(args.id)
                if ok:
                    print(f"Killed session {args.id}.")
                    return 0
                print(f"No session matched {args.id!r}.", file=sys.stderr)
                return 1
            print(f"ERROR: unknown sessions command {action!r}", file=sys.stderr)
            return 2
        finally:
            await client.aclose()

    sys.exit(asyncio.run(_run()))


def _format_age(seconds: float) -> str:
    s = int(seconds)
    if s < 60:
        return f"{s}s"
    if s < 3600:
        return f"{s // 60}m"
    if s < 86400:
        return f"{s // 3600}h"
    return f"{s // 86400}d"


def cmd_install_service(args) -> None:
    """Install opendesk serve as a systemd / launchd / Task Scheduler service."""
    try:
        from opendesk.remote.service import install_service
    except ImportError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        sys.exit(2)

    home = Path(args.home).expanduser() if args.home else None
    try:
        result = install_service(
            home=home, port=args.port, autostart=not args.no_start,
        )
    except Exception as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        sys.exit(1)

    print(f"✓ Service installed ({result.manager}): {result.path}")
    if result.started:
        print("  Started.  It will also run automatically on next login.")
    elif args.no_start:
        print("  Not started (--no-start).  Activate manually or rerun without the flag.")
    else:
        print(
            "  WARNING: file written but service manager could not start it. "
            "Activate manually.",
        )


def cmd_uninstall_service(args) -> None:
    try:
        from opendesk.remote.service import uninstall_service
    except ImportError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        sys.exit(2)
    try:
        removed = uninstall_service()
    except Exception as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        sys.exit(1)
    print("✓ Service uninstalled." if removed else "No opendesk service was installed.")


def cmd_peers(args) -> None:
    """List, remove, or rename trusted peers."""
    _, TrustedPeers, _, _, _ = _remote_imports()
    home = Path(args.home).expanduser() if args.home else None
    store = TrustedPeers(home)
    action = args.peers_cmd or "list"

    if action == "list":
        peers = store.list()
        if not peers:
            print("No trusted peers.")
            return
        print(f"{'NAME':<32}  FINGERPRINT")
        for p in peers:
            print(f"{p.name:<32}  {p.fingerprint}")
    elif action == "remove":
        if store.remove(args.target):
            print(f"Removed {args.target}.")
        else:
            print(f"No peer matched {args.target!r}.", file=sys.stderr)
            sys.exit(1)
    elif action == "rename":
        if store.rename(args.target, args.new_name):
            print(f"Renamed {args.target} → {args.new_name}.")
        else:
            print(f"No peer matched {args.target!r}.", file=sys.stderr)
            sys.exit(1)


def cmd_scheduler(args) -> None:
    """Run the scheduler daemon."""
    from pathlib import Path
    project_dir = Path(args.dir).resolve() if args.dir else Path.cwd()

    if args.scheduler_cmd == "start":
        from opendesk.automation.daemon import start_daemon
        start_daemon(project_dir)
    elif args.scheduler_cmd == "list":
        from opendesk.automation.schedule_store import ScheduleStore
        store = ScheduleStore(project_dir)
        entries = store.all()
        if not entries:
            print("No schedules.")
        for e in entries:
            status = "on" if e.enabled else "off"
            print(f"  [{status}] {e.name}  ({e.timing})  →  {e.task}")
    else:
        print("Usage: opendesk scheduler start|list")


def main() -> None:
    import argparse

    parser = argparse.ArgumentParser(
        prog="opendesk",
        description="opendesk — Open Desktop Agent",
    )
    sub = parser.add_subparsers(dest="command")

    install_p = sub.add_parser("install", help="Register opendesk with Claude Code")
    install_p.add_argument(
        "--scope",
        choices=["user", "project"],
        default="user",
        help="user = all projects (default), project = current project only",
    )

    sub.add_parser("uninstall", help="Remove opendesk from Claude Code")

    sched_p = sub.add_parser("scheduler", help="Manage the background task scheduler")
    sched_p.add_argument(
        "scheduler_cmd",
        choices=["start", "list"],
        help="start: run the scheduler daemon  |  list: show scheduled tasks",
    )
    sched_p.add_argument(
        "--dir",
        default=None,
        help="Project directory (default: current directory)",
    )

    # --- Remote (controlled machine) ----------------------------------

    pair_p = sub.add_parser(
        "pair",
        help="Accept one new controller (prints a code; controller types it on `pair-with`)",
    )
    pair_p.add_argument("--port", type=int, default=8423, help="WebSocket port (default 8423)")
    pair_p.add_argument("--host", default="0.0.0.0", help="Interface to bind (default 0.0.0.0)")
    pair_p.add_argument("--code", default=None, help="Use this code instead of a random one")
    pair_p.add_argument("--timeout", type=float, default=300.0, help="Seconds to wait for a peer")
    pair_p.add_argument("--home", default=None, help="Identity / trusted-peers directory")
    pair_p.add_argument("--no-mdns", action="store_true", help="Disable mDNS advertisement")

    serve_p = sub.add_parser(
        "serve", help="Run the long-lived opendesk server (paired peers only)",
    )
    serve_p.add_argument("--port", type=int, default=8423)
    serve_p.add_argument("--host", default="0.0.0.0")
    serve_p.add_argument("--home", default=None)
    serve_p.add_argument("--no-mdns", action="store_true")

    # --- Remote (controller machine) ----------------------------------

    pw_p = sub.add_parser(
        "pair-with",
        help="Pair this machine with a peer running `opendesk pair`",
    )
    pw_p.add_argument("host", help="Hostname or IP of the controlled machine")
    pw_p.add_argument("code", help="6-digit code shown on the peer")
    pw_p.add_argument("--port", type=int, default=8423)
    pw_p.add_argument("--name", default="", help="Friendly name for the new peer")
    pw_p.add_argument("--home", default=None)

    disc_p = sub.add_parser("discover", help="List opendesk peers on the LAN")
    disc_p.add_argument("--timeout", type=float, default=2.0)

    conn_p = sub.add_parser("connect", help="Open a paired peer and confirm it works")
    conn_p.add_argument("peer", help="Friendly name from `opendesk peers list`")
    conn_p.add_argument("--screenshot", default=None, help="Path to save a captured screenshot")
    conn_p.add_argument("--home", default=None)

    sess_p = sub.add_parser(
        "sessions",
        help="Inspect or kill active opendesk-serve sessions (controlled machine)",
    )
    sess_sub = sess_p.add_subparsers(dest="sessions_cmd")
    sess_sub.add_parser("list", help="List active sessions (default action)")
    kill_p = sess_sub.add_parser("kill", help="Disconnect one or all sessions")
    kill_p.add_argument("id", nargs="?", help="Session ID from `sessions list`")
    kill_p.add_argument("--all", action="store_true", help="Disconnect every session")
    sess_p.add_argument("--home", default=None)

    inst_p = sub.add_parser(
        "install-service",
        help="Install opendesk serve as a user-scoped system service",
    )
    inst_p.add_argument("--port", type=int, default=8423)
    inst_p.add_argument("--home", default=None)
    inst_p.add_argument(
        "--no-start", action="store_true",
        help="Only write the service file; don't activate it now",
    )

    sub.add_parser(
        "uninstall-service",
        help="Remove the opendesk serve system service",
    )

    peers_p = sub.add_parser("peers", help="List, remove, or rename trusted peers")
    peers_sub = peers_p.add_subparsers(dest="peers_cmd")
    peers_sub.add_parser("list", help="List trusted peers (default action)")
    rm_p = peers_sub.add_parser("remove", help="Forget a trusted peer")
    rm_p.add_argument("target", help="Peer name or fingerprint")
    ren_p = peers_sub.add_parser("rename", help="Rename a trusted peer")
    ren_p.add_argument("target")
    ren_p.add_argument("new_name")
    peers_p.add_argument("--home", default=None)

    args = parser.parse_args()

    if args.command == "install":
        cmd_install(scope=args.scope)
    elif args.command == "uninstall":
        cmd_uninstall()
    elif args.command == "scheduler":
        cmd_scheduler(args)
    elif args.command == "pair":
        cmd_pair(args)
    elif args.command == "pair-with":
        cmd_pair_with(args)
    elif args.command == "serve":
        cmd_serve(args)
    elif args.command == "discover":
        cmd_discover(args)
    elif args.command == "connect":
        cmd_connect(args)
    elif args.command == "peers":
        cmd_peers(args)
    elif args.command == "sessions":
        cmd_sessions(args)
    elif args.command == "install-service":
        cmd_install_service(args)
    elif args.command == "uninstall-service":
        cmd_uninstall_service(args)
    else:
        parser.print_help()


if __name__ == "__main__":
    main()
