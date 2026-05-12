"""Stateful session backing the MCP integration.

Owns peer resolution and connection caching so the MCP server itself stays
mostly stateless.  One :class:`MCPSession` lives for the duration of an MCP
client session — typically the lifetime of the opendesk-mcp subprocess.

Resolution rules for each tool call::

    explicit `peer` argument   →  use that peer
    else default peer set      →  use the default
    else                       →  local

Special name ``"local"`` always means the in-process :class:`LocalComputer`,
even if a peer happens to be named ``"local"`` (which the CLI doesn't allow
but defence-in-depth is cheap).
"""

from __future__ import annotations

import asyncio
import contextlib
from pathlib import Path
from typing import Optional

from opendesk.computer import Computer, LocalComputer


LOCAL = "local"


class MCPSessionError(RuntimeError):
    """Surfaced through MCP as a clear error message for the agent."""


class MCPSession:
    """Per-MCP-session state: current peer, connection cache, local computer."""

    def __init__(
        self,
        *,
        home: Optional[Path] = None,
        local: Optional[Computer] = None,
    ) -> None:
        self._home = home
        self._local = local or LocalComputer()
        self._current_peer: Optional[str] = None
        self._connections: dict[str, Computer] = {}
        self._lock = asyncio.Lock()

    # ------------------------------------------------------------------
    # State
    # ------------------------------------------------------------------

    @property
    def home(self) -> Optional[Path]:
        return self._home

    @property
    def local(self) -> Computer:
        return self._local

    @property
    def current_peer(self) -> Optional[str]:
        """Explicit default peer name (set via :meth:`use_peer`), or ``None``.

        Note: when this is ``None`` and exactly one trusted peer exists,
        :meth:`effective_peer` reports that peer as the implicit default.
        """
        return self._current_peer

    def effective_peer(self) -> tuple[Optional[str], str]:
        """Return ``(name, source)`` for what :meth:`resolve` would pick now.

        ``name`` is ``None`` for local *or* ambiguous.  ``source`` is one of:

        * ``"explicit"`` — set via :meth:`use_peer`.
        * ``"implicit"`` — exactly one trusted peer exists, no explicit
          default set, so it's used automatically.
        * ``"ambiguous"`` — multiple trusted peers and no default set;
          :meth:`resolve` will raise unless a ``peer`` argument is supplied.
        * ``"local"`` — no peers paired; falls through to the local computer.
        """
        if self._current_peer is not None:
            return self._current_peer, "explicit"
        return self._default_state()

    def _default_state(self) -> tuple[Optional[str], str]:
        """Classify what implicit default applies given the trusted-peers file."""
        from opendesk.protocol.auth import TrustedPeers
        peers = TrustedPeers(self._home).list()
        if len(peers) == 1:
            return peers[0].name, "implicit"
        if len(peers) > 1:
            return None, "ambiguous"
        return None, "local"

    def active_peer_names(self) -> list[str]:
        """Names of peers with an open cached connection."""
        return sorted(self._connections.keys())

    # ------------------------------------------------------------------
    # Selection
    # ------------------------------------------------------------------

    def use_peer(self, name: Optional[str]) -> None:
        """Set the default peer for subsequent tool calls.

        Pass ``None`` or ``"local"`` to revert to the local computer.
        Raises :class:`MCPSessionError` if the peer is not in trusted-peers.
        """
        if name is None or name == LOCAL:
            self._current_peer = None
            return
        from opendesk.protocol.auth import TrustedPeers
        if TrustedPeers(self._home).find_by_name(name) is None:
            raise MCPSessionError(
                f"No trusted peer named {name!r}.  Pair from the controlled "
                f"machine first via `opendesk pair`, then `opendesk pair-with "
                f"<host> <code> --name {name}` here."
            )
        self._current_peer = name

    async def resolve(self, requested: Optional[str] = None) -> tuple[Computer, str]:
        """Get the :class:`Computer` to use for one tool call.

        Resolution order:

        1. ``requested`` (the call-site ``peer`` argument), if provided.
        2. The explicit default set via :meth:`use_peer`.
        3. The lone trusted peer, when exactly one exists ("implicit default").
        4. If multiple trusted peers exist with no default set, **raise**
           :class:`MCPSessionError` rather than silently picking local —
           agent intent is ambiguous and a wrong pick would land actions on
           the wrong machine.
        5. Otherwise (no peers paired), local.

        Returns ``(computer, name)``.  Lazy-connects to a remote peer the
        first time it's referenced.
        """
        chosen = requested
        if chosen is None:
            chosen = self._current_peer
        if chosen is None:
            implicit, source = self._default_state()
            if source == "ambiguous":
                from opendesk.protocol.auth import TrustedPeers
                names = ", ".join(p.name for p in TrustedPeers(self._home).list())
                raise MCPSessionError(
                    f"Multiple peers paired ({names}) and no default set. "
                    "Run `opendesk_use <name>` to choose one, or pass `peer:` "
                    "on this call (use 'local' to target this machine)."
                )
            chosen = implicit
        if chosen is None or chosen == LOCAL:
            return self._local, LOCAL

        async with self._lock:
            if chosen in self._connections:
                return self._connections[chosen], chosen
            try:
                from opendesk.remote.client import connect
                computer = await connect(chosen, home=self._home)
            except Exception as exc:
                raise MCPSessionError(
                    f"Could not connect to peer {chosen!r}: {exc}"
                ) from exc
            self._connections[chosen] = computer
            return computer, chosen

    async def disconnect(self, name: Optional[str] = None) -> int:
        """Close a cached connection.  ``None`` closes all.

        Returns the number of connections closed.
        """
        async with self._lock:
            if name == LOCAL:
                return 0
            if name is None:
                targets = list(self._connections.items())
                self._connections.clear()
            else:
                if name not in self._connections:
                    return 0
                targets = [(name, self._connections.pop(name))]
            if self._current_peer in {n for n, _ in targets}:
                self._current_peer = None

        for _, computer in targets:
            with contextlib.suppress(Exception):
                await computer.aclose()
        return len(targets)

    async def aclose(self) -> None:
        """Close every cached connection.  Idempotent."""
        await self.disconnect()
        with contextlib.suppress(Exception):
            await self._local.aclose()
