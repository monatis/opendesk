"""Server-side authorisation policy.

Every protocol method on an active session passes through a :class:`Policy`
before the :class:`~opendesk.computer.Computer` is touched.  The policy can:

* return — call proceeds normally;
* raise :class:`~opendesk.tools.base.PermissionDeniedError` — call fails
  with ``permission_denied``, surfacing as the same exception on the
  controller side.

Two implementations ship in-tree:

* :class:`AllowAllPolicy` — the default.  Sane for paired single-user
  setups where mutual trust is established at pairing time.
* :class:`ConsolePolicy` — interactive approval via stdin.  Auto-allows
  observation methods (screen capture, window list, etc.) so the agent
  isn't prompting for every screenshot; gates anything that injects input,
  mutates state, or runs a command.  When there's no TTY (running under
  systemd / launchd), gated methods are denied by default — operators get
  a clear log line and can switch to :class:`AllowAllPolicy` or write a
  custom :class:`Policy` (e.g. backed by desktop notifications).

Custom policies are just classes implementing :class:`Policy` — pass them
to :class:`~opendesk.remote.server.OpendeskServer` or :func:`serve`.
"""

from __future__ import annotations

import asyncio
import logging
import sys
from typing import Any, Protocol

from opendesk.tools.base import PermissionDeniedError


log = logging.getLogger("opendesk.remote.policy")


# ---------------------------------------------------------------------------
# Method classification
# ---------------------------------------------------------------------------

# Methods with no observable side effect.  ConsolePolicy auto-approves these
# so the agent isn't prompting for every screenshot.  Same set used by
# RemoteComputer for safe-retry-after-reconnect.
OBSERVATION_METHODS: frozenset[str] = frozenset({
    "system.capabilities",
    "system.environment",
    "display.capture",
    "display.cursor_position",
    "display.displays",
    "display.subscribe",
    "windows.list",
    "windows.focused",
    "ui.tree",
    "clipboard.read",
    "fs.read",
    "fs.list",
    "fs.stat",
    "process.list",
    "apps.list",
    "notifications.list",
    "input.subscribe",
})


# ---------------------------------------------------------------------------
# Protocol
# ---------------------------------------------------------------------------


class Policy(Protocol):
    """Async gate evaluated before every method dispatch.

    Implementations should ``return`` to allow the call and raise
    :class:`PermissionDeniedError` to deny.  Any other exception is treated
    as an internal error and surfaces as ``internal`` on the wire.
    """

    async def check(
        self,
        *,
        peer_public: bytes,
        peer_name: str,
        method: str,
        params: dict[str, Any],
    ) -> None: ...


# ---------------------------------------------------------------------------
# AllowAllPolicy
# ---------------------------------------------------------------------------


class AllowAllPolicy:
    """Approves every method.  Default; safe for paired single-user setups."""

    async def check(self, *, peer_public, peer_name, method, params) -> None:
        return None


# ---------------------------------------------------------------------------
# ConsolePolicy
# ---------------------------------------------------------------------------


class ConsolePolicy:
    """Stdin-prompt interactive approval.

    Auto-allows :data:`OBSERVATION_METHODS`.  Gates everything else with a
    one-line prompt::

        [opendesk] laptop wants to send pointer down at (812, 410). Allow? [y/N]

    With no TTY (daemon contexts), gated methods are denied — the server
    logs a single warning at startup so the operator sees the situation
    clearly instead of every call mysteriously failing.
    """

    def __init__(self, *, stream=None) -> None:
        self._stream = stream or sys.stdin
        self._has_tty = bool(getattr(self._stream, "isatty", lambda: False)())
        if not self._has_tty:
            log.warning(
                "ConsolePolicy: no TTY available — non-observation methods "
                "will be denied. Use AllowAllPolicy or a custom Policy for "
                "headless setups."
            )

    async def check(
        self,
        *,
        peer_public: bytes,
        peer_name: str,
        method: str,
        params: dict[str, Any],
    ) -> None:
        if method in OBSERVATION_METHODS:
            return
        label = peer_name or f"peer-{peer_public.hex()[:6]}"
        summary = _summarise(method, params)
        if not self._has_tty:
            raise PermissionDeniedError(
                f"{label} → {summary}: no TTY for interactive approval"
            )
        prompt = f"\n[opendesk] {label} wants to {summary}. Allow? [y/N] "
        loop = asyncio.get_event_loop()
        answer = await loop.run_in_executor(None, _read_line, prompt)
        if answer.strip().lower() not in ("y", "yes"):
            raise PermissionDeniedError(f"{label} denied: {summary}")


def _read_line(prompt: str) -> str:
    """Sync read of one line.  Run via ``run_in_executor`` so we don't block
    the event loop while a human decides."""
    return input(prompt)


# ---------------------------------------------------------------------------
# Human-friendly summaries for prompts / logs
# ---------------------------------------------------------------------------


def _summarise(method: str, params: dict[str, Any]) -> str:
    """One-line natural description of an inbound method call."""
    if method == "input.pointer":
        evt = params.get("event") or {}
        pt = evt.get("point") or {}
        action = evt.get("action", "move")
        return f"send pointer {action} at ({pt.get('x', '?')}, {pt.get('y', '?')})"
    if method == "input.key":
        evt = params.get("event") or {}
        return f"send key {evt.get('action', 'press')} {evt.get('keysym', '?')!r}"
    if method == "input.text":
        text = (params.get("text_input") or {}).get("text", "")
        preview = text[:40] + ("…" if len(text) > 40 else "")
        return f"type {len(text)} chars: {preview!r}"
    if method == "process.shell":
        cmd = params.get("command", "")
        return f"run shell: {cmd[:80]!r}"
    if method == "process.exec":
        argv = params.get("argv", [])
        return f"exec: {' '.join(str(a) for a in argv)[:80]!r}"
    if method in ("fs.write", "fs.delete", "fs.mkdir", "fs.stat", "fs.read"):
        verb = method.split(".", 1)[1]
        return f"{verb} file: {params.get('path', '?')}"
    if method == "fs.move":
        return f"move {params.get('src', '?')} → {params.get('dst', '?')}"
    if method == "clipboard.write":
        return "write to the clipboard"
    if method in ("apps.open", "apps.close", "apps.focus"):
        verb = method.split(".", 1)[1]
        return f"{verb} app: {params.get('name', '?')!r}"
    if method.startswith("windows."):
        return f"{method} ({', '.join(f'{k}={v}' for k, v in params.items())})"
    if method == "ui.action":
        elem = params.get("element") or {}
        return f"ui action {params.get('action', 'click')!r} on {elem.get('name') or elem.get('role') or '?'}"
    if method == "power.lock":
        return "lock the screen"
    return method
