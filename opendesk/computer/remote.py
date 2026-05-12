"""RemoteComputer — a :class:`Computer` whose methods are calls to a peer.

Wraps a :class:`~opendesk.protocol.Peer` to satisfy the
:class:`~opendesk.computer.Computer` ABC.  Every abstract method packages its
arguments into a params dict, awaits ``peer.call(method, params)``, and
deserialises the result into the right Pydantic model.  Subscriptions return
async iterators backed by ``peer.stream(...)``.

``capabilities()`` is synchronous (per the ABC) and is served from the cached
manifest exchanged during HELLO — no round-trip.

Reconnect resilience
--------------------
:meth:`connect_with_reconnect` takes a ``connector`` callable that produces a
fresh ``(Peer, manifest)`` on demand.  On a transient connection failure
(``ConnectionClosed`` or protocol-fault :class:`ProtocolError`), the
RemoteComputer:

1. Reconnects with exponential backoff (capped by ``reconnect_budget``).
2. If the call is in :data:`_IDEMPOTENT_METHODS`, replays it once.
3. Otherwise, raises the original error — but the connection is now healthy,
   so the next call succeeds immediately.
"""

from __future__ import annotations

import asyncio
import contextlib
from typing import Any, AsyncIterator, Awaitable, Callable, Optional

from opendesk.computer.base import (
    CapabilityUnsupported,
    Computer,
    InputEvent,
)
from opendesk.computer.types import (
    Capability,
    CapabilityManifest,
    ClipboardContents,
    CompletedCommand,
    Display,
    DisplayFrame,
    Environment,
    FileEntry,
    KeyEvent,
    Notification,
    Pixmap,
    Point,
    PointerEvent,
    Process,
    Rect,
    TextInput,
    UIElement,
    Window,
)
from opendesk.protocol import (
    Connection,
    ErrorCode,
    Peer,
    ProtocolError,
)
from opendesk.protocol.connection import ConnectionClosed


# Protocol methods that have no side effects and can safely be replayed
# after a transient disconnect.  Anything that injects input, mutates
# state, or runs a command is intentionally excluded.
_IDEMPOTENT_METHODS: frozenset[str] = frozenset({
    "system.environment",
    "display.capture", "display.cursor_position", "display.displays",
    "windows.list", "windows.focused",
    "ui.tree",
    "clipboard.read",
    "fs.read", "fs.list", "fs.stat",
    "process.list",
    "apps.list",
    "notifications.list",
})

# Exponential backoff between reconnect attempts.  Total wall-clock time is
# capped separately by ``reconnect_budget``.
_RECONNECT_DELAYS: tuple[float, ...] = (0.5, 1.0, 2.0, 4.0, 5.0, 5.0, 5.0, 5.0)


Connector = Callable[[], Awaitable[tuple[Peer, CapabilityManifest]]]
"""Async factory that opens a fresh authed session and returns
``(running_peer, capability_manifest)`` for it."""


class SessionEvicted(RuntimeError):
    """Raised when the server has explicitly evicted this controller.

    The wire signal is a ``session.evicted`` PUSH frame.  A cooperative
    :class:`RemoteComputer` (this one) honours it by suppressing the
    auto-reconnect: the controller is *meant* to stop driving the host
    until the operator explicitly opens a new connection.

    Distinct from :class:`~opendesk.protocol.connection.ConnectionClosed`
    so agents can tell the difference between a transient network drop
    (probably worth retrying) and an intentional eviction (do not retry —
    the operator wants you gone).
    """

    def __init__(self, reason: str = "") -> None:
        suffix = f": {reason}" if reason else ""
        super().__init__(f"session evicted by server{suffix}")
        self.reason = reason


def _translate(exc: BaseException) -> BaseException:
    """Map a :class:`ProtocolError` into the richest available Python exception.

    The :class:`Peer` layer already maps ``cancelled`` to
    :class:`asyncio.CancelledError` and ``permission_denied`` to
    :class:`~opendesk.tools.base.PermissionDeniedError`.  Here we add the one
    bit it can't: :class:`CapabilityUnsupported` — because that exception
    needs a :class:`Capability` enum value the protocol layer doesn't know
    about.
    """
    if isinstance(exc, ProtocolError) and exc.code == ErrorCode.CAPABILITY_UNSUPPORTED.value:
        cap_name = exc.details.get("capability")
        backend = exc.details.get("backend", "remote")
        try:
            cap = Capability(cap_name) if cap_name else Capability.DISPLAY_CAPTURE
        except ValueError:
            cap = Capability.DISPLAY_CAPTURE
        return CapabilityUnsupported(cap, backend=backend)
    return exc


class RemoteComputer(Computer):
    """A :class:`Computer` that lives on the other end of a :class:`Peer`."""

    BACKEND = "remote"

    def __init__(
        self,
        peer: Peer,
        manifest: CapabilityManifest,
        *,
        connector: Optional[Connector] = None,
        reconnect_budget: float = 30.0,
    ) -> None:
        self._peer = peer
        self._manifest = manifest
        self._connector = connector
        self._reconnect_budget = reconnect_budget
        self._reconnect_lock = asyncio.Lock()
        self._closed = False
        self._evicted = False
        self._eviction_reason = ""
        self._peer.on_push(self._on_push)

    # ------------------------------------------------------------------
    # Eviction (cooperative — server PUSH "session.evicted")
    # ------------------------------------------------------------------

    @property
    def evicted(self) -> bool:
        """True if the server has signalled an eviction on this connection."""
        return self._evicted

    @property
    def eviction_reason(self) -> str:
        return self._eviction_reason

    async def _on_push(self, topic: str, payload: dict) -> None:
        if topic == "session.evicted":
            self._evicted = True
            self._eviction_reason = (payload or {}).get("reason", "") or ""

    # ------------------------------------------------------------------
    # Construction / lifecycle
    # ------------------------------------------------------------------

    @classmethod
    async def connect(
        cls,
        connection: Connection,
        *,
        principal: str = "",
        auth: Optional[dict[str, Any]] = None,
        local_capabilities: Optional[dict[str, Any]] = None,
    ) -> "RemoteComputer":
        """Perform the HELLO handshake and return a ready-to-use RemoteComputer.

        ``connection`` must already be open.  After this returns the peer's
        recv loop is running and any method on the returned RemoteComputer
        translates to protocol traffic.

        This flavor has **no auto-reconnect**.  Use
        :meth:`connect_with_reconnect` to get a RemoteComputer that survives
        transient drops.
        """
        peer = Peer(connection, role="client")
        try:
            hello = await peer.hello(local_capabilities or {}, principal=principal, auth=auth)
        except BaseException:
            await peer.aclose()
            raise
        try:
            manifest = CapabilityManifest.model_validate(hello.capabilities)
        except Exception:
            manifest = CapabilityManifest()
        peer.start()
        return cls(peer, manifest)

    @classmethod
    async def connect_with_reconnect(
        cls,
        connector: Connector,
        *,
        reconnect_budget: float = 30.0,
    ) -> "RemoteComputer":
        """Create a RemoteComputer with auto-reconnect on transient drops.

        ``connector`` is invoked once now to open the initial session, and
        again on each reconnect attempt.  ``reconnect_budget`` is the total
        wall-clock seconds spent retrying before surfacing the disconnect.
        """
        peer, manifest = await connector()
        return cls(peer, manifest, connector=connector, reconnect_budget=reconnect_budget)

    async def aclose(self) -> None:
        self._closed = True
        await self._peer.aclose()

    # ------------------------------------------------------------------
    # call helpers — handle reconnect + replay
    # ------------------------------------------------------------------

    async def _call(self, method: str, params: Optional[dict[str, Any]] = None) -> Any:
        if self._evicted:
            raise SessionEvicted(self._eviction_reason)
        try:
            return await self._peer.call(method, params or {})
        except (ConnectionClosed, ProtocolError) as exc:
            if self._evicted:
                raise SessionEvicted(self._eviction_reason) from exc
            translated = _translate(exc) if isinstance(exc, ProtocolError) else exc
            if not self._is_transient(exc) or self._connector is None or self._closed:
                raise translated from exc
            reconnected = await self._reconnect()
            if not reconnected or method not in _IDEMPOTENT_METHODS:
                if self._evicted:
                    raise SessionEvicted(self._eviction_reason) from exc
                raise translated from exc
            try:
                return await self._peer.call(method, params or {})
            except ProtocolError as exc2:
                raise _translate(exc2) from exc2

    def _stream(self, method: str, params: Optional[dict[str, Any]] = None) -> AsyncIterator[Any]:
        async def _wrap():
            try:
                async for item in self._peer.stream(method, params or {}):
                    yield item
            except ProtocolError as exc:
                raise _translate(exc) from exc
        return _wrap()

    def _is_transient(self, exc: BaseException) -> bool:
        """Was this error caused by the connection breaking?"""
        if isinstance(exc, ConnectionClosed):
            return True
        if isinstance(exc, ProtocolError):
            return exc.code == ErrorCode.PROTOCOL.value
        return False

    async def _reconnect(self) -> bool:
        """Try to swap the dead peer with a fresh one.  Returns success.

        Refuses immediately when:

        * No connector is registered (caller used :meth:`connect`, not
          :meth:`connect_with_reconnect`).
        * :meth:`aclose` has been called.
        * The server sent a ``session.evicted`` PUSH — the eviction is a
          *cooperative* signal that the controller is meant to stop trying.
        """
        if self._connector is None or self._closed or self._evicted:
            return False
        async with self._reconnect_lock:
            # Another concurrent caller may have already reconnected for us.
            if not self._peer.closed and not getattr(self._peer, "_closed", False):
                return True
            with contextlib.suppress(Exception):
                await self._peer.aclose()

            loop = asyncio.get_event_loop()
            start = loop.time()
            for delay in _RECONNECT_DELAYS:
                if loop.time() - start > self._reconnect_budget:
                    break
                try:
                    new_peer, new_manifest = await self._connector()
                except Exception:
                    if self._closed or self._evicted:
                        return False
                    await asyncio.sleep(delay)
                    continue
                # Carry the eviction push handler onto the fresh peer too.
                new_peer.on_push(self._on_push)
                self._peer = new_peer
                self._manifest = new_manifest
                return True
            return False

    # ------------------------------------------------------------------
    # Introspection
    # ------------------------------------------------------------------

    def capabilities(self) -> CapabilityManifest:
        return self._manifest

    async def environment(self) -> Environment:
        return Environment.model_validate(await self._call("system.environment"))

    # ------------------------------------------------------------------
    # Display
    # ------------------------------------------------------------------

    async def displays(self) -> list[Display]:
        result = await self._call("display.displays")
        return [Display.model_validate(d) for d in result["items"]]

    async def capture(
        self,
        *,
        display_id: Optional[str] = None,
        region: Optional[Rect] = None,
        downscale: bool = True,
    ) -> Pixmap:
        result = await self._call("display.capture", {
            "display_id": display_id,
            "region": region.model_dump() if region else None,
            "downscale": downscale,
        })
        return Pixmap.model_validate(result)

    async def cursor_position(self) -> Point:
        return Point.model_validate(await self._call("display.cursor_position"))

    def subscribe_display(
        self,
        *,
        display_id: Optional[str] = None,
        fps: int = 30,
        region: Optional[Rect] = None,
    ) -> AsyncIterator[DisplayFrame]:
        params = {
            "display_id": display_id,
            "fps": fps,
            "region": region.model_dump() if region else None,
        }
        async def _gen():
            async for item in self._stream("display.subscribe", params):
                yield DisplayFrame.model_validate(item)
        return _gen()

    # ------------------------------------------------------------------
    # Input
    # ------------------------------------------------------------------

    async def pointer(self, event: PointerEvent) -> None:
        await self._call("input.pointer", {"event": event.model_dump()})

    async def key(self, event: KeyEvent) -> None:
        await self._call("input.key", {"event": event.model_dump()})

    async def text(self, text_input: TextInput) -> None:
        await self._call("input.text", {"text_input": text_input.model_dump()})

    def subscribe_input(self) -> AsyncIterator[InputEvent]:
        async def _gen():
            async for item in self._stream("input.subscribe", {}):
                # The server side decides which variant; we trust the dict shape.
                if "keysym" in item:
                    yield KeyEvent.model_validate(item)
                else:
                    yield PointerEvent.model_validate(item)
        return _gen()

    # ------------------------------------------------------------------
    # Windows & apps
    # ------------------------------------------------------------------

    async def windows(self) -> list[Window]:
        result = await self._call("windows.list")
        return [Window.model_validate(w) for w in result["items"]]

    async def focused_window(self) -> Optional[Window]:
        result = await self._call("windows.focused")
        w = result.get("window")
        return Window.model_validate(w) if w else None

    async def focus_window(self, window_id: str) -> None:
        await self._call("windows.focus", {"window_id": window_id})

    async def move_window(self, window_id: str, bounds: Rect) -> None:
        await self._call("windows.move", {
            "window_id": window_id, "bounds": bounds.model_dump(),
        })

    async def close_window(self, window_id: str) -> None:
        await self._call("windows.close", {"window_id": window_id})

    async def open_app(self, name: str) -> None:
        await self._call("apps.open", {"name": name})

    async def close_app(self, name: str) -> None:
        await self._call("apps.close", {"name": name})

    async def focus_app(self, name: str) -> None:
        await self._call("apps.focus", {"name": name})

    async def list_apps(self) -> list[str]:
        result = await self._call("apps.list")
        return list(result["items"])

    # ------------------------------------------------------------------
    # UI tree
    # ------------------------------------------------------------------

    async def ui_tree(
        self,
        *,
        window_id: Optional[str] = None,
        app: Optional[str] = None,
        max_depth: int = 8,
    ) -> UIElement:
        return UIElement.model_validate(await self._call("ui.tree", {
            "window_id": window_id, "app": app, "max_depth": max_depth,
        }))

    async def perform_ui_action(
        self,
        element: UIElement,
        action: str = "click",
        *,
        app: Optional[str] = None,
    ) -> None:
        await self._call("ui.action", {
            "element": element.model_dump(), "action": action, "app": app,
        })

    # ------------------------------------------------------------------
    # Clipboard
    # ------------------------------------------------------------------

    async def clipboard_read(self) -> ClipboardContents:
        return ClipboardContents.model_validate(await self._call("clipboard.read"))

    async def clipboard_write(self, contents: ClipboardContents) -> None:
        await self._call("clipboard.write", {"contents": contents.model_dump()})

    # ------------------------------------------------------------------
    # Filesystem
    # ------------------------------------------------------------------

    async def read_file(self, path: str) -> bytes:
        result = await self._call("fs.read", {"path": path})
        return result["data"]

    async def write_file(self, path: str, data: bytes) -> None:
        await self._call("fs.write", {"path": path, "data": data})

    async def list_dir(self, path: str) -> list[FileEntry]:
        result = await self._call("fs.list", {"path": path})
        return [FileEntry.model_validate(e) for e in result["items"]]

    async def stat(self, path: str) -> FileEntry:
        return FileEntry.model_validate(await self._call("fs.stat", {"path": path}))

    async def delete(self, path: str) -> None:
        await self._call("fs.delete", {"path": path})

    async def move(self, src: str, dst: str) -> None:
        await self._call("fs.move", {"src": src, "dst": dst})

    async def mkdir(self, path: str, *, parents: bool = True) -> None:
        await self._call("fs.mkdir", {"path": path, "parents": parents})

    # ------------------------------------------------------------------
    # Processes
    # ------------------------------------------------------------------

    async def processes(self) -> list[Process]:
        result = await self._call("process.list")
        return [Process.model_validate(p) for p in result["items"]]

    async def shell(
        self,
        command: str,
        *,
        timeout: Optional[float] = None,
        cwd: Optional[str] = None,
        env: Optional[dict[str, str]] = None,
    ) -> CompletedCommand:
        return CompletedCommand.model_validate(await self._call("process.shell", {
            "command": command, "timeout": timeout, "cwd": cwd, "env": env,
        }))

    async def exec(
        self,
        argv: list[str],
        *,
        timeout: Optional[float] = None,
        cwd: Optional[str] = None,
        env: Optional[dict[str, str]] = None,
        stdin: Optional[bytes] = None,
    ) -> CompletedCommand:
        return CompletedCommand.model_validate(await self._call("process.exec", {
            "argv": argv, "timeout": timeout, "cwd": cwd, "env": env, "stdin": stdin,
        }))

    # ------------------------------------------------------------------
    # Power
    # ------------------------------------------------------------------

    async def lock_screen(self) -> None:
        await self._call("power.lock")

    # ------------------------------------------------------------------
    # Notifications
    # ------------------------------------------------------------------

    async def notifications(self) -> list[Notification]:
        result = await self._call("notifications.list")
        return [Notification.model_validate(n) for n in result["items"]]
