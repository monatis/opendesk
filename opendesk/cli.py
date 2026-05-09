"""opendesk CLI — setup and utility commands."""

from __future__ import annotations

import shutil
import subprocess
import sys


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
    if not shutil.which("claude"):
        print(
            "ERROR: 'claude' command not found.\n"
            "Install Claude Code first: https://claude.ai/code",
            file=sys.stderr,
        )
        sys.exit(1)

    mcp_path = _find_mcp_binary()

    # Remove existing registration if any (ignore errors)
    subprocess.run(
        ["claude", "mcp", "remove", "opendesk"],
        capture_output=True,
    )

    result = subprocess.run(
        ["claude", "mcp", "add", "opendesk", f"--scope={scope}", "--", mcp_path],
        capture_output=True,
        text=True,
    )

    if result.returncode != 0:
        print(f"ERROR: {result.stderr.strip()}", file=sys.stderr)
        sys.exit(1)

    print(f"opendesk MCP server registered ({scope}).")
    print(f"  Binary: {mcp_path}")
    print("Start a Claude Code conversation and say 'take a screenshot' to verify.")


def cmd_scheduler(args) -> None:
    """Run the scheduler daemon."""
    from pathlib import Path
    project_dir = Path(args.dir).resolve() if args.dir else Path.cwd()

    if args.scheduler_cmd == "start":
        from opendesk.schedule.daemon import start_daemon
        start_daemon(project_dir)
    elif args.scheduler_cmd == "list":
        from opendesk.schedule.store import ScheduleStore
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

    args = parser.parse_args()

    if args.command == "install":
        cmd_install(scope=args.scope)
    elif args.command == "scheduler":
        cmd_scheduler(args)
    else:
        parser.print_help()


if __name__ == "__main__":
    main()
