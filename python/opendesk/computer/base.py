"""The :class:`Computer` abstract base class — the capability surface of a
computer that an opendesk agent can observe and act upon.

Two implementations exist in-tree:

* :class:`~opendesk.computer.local.LocalComputer` — the machine running this
  process.
* :class:`~opendesk.computer.remote.RemoteComputer` (forthcoming) — a peer
  reached over the remote-control protocol.

Tools and agents code against :class:`Computer`.  Where the computer lives is
a deployment concern, not a code concern.

Conceptual model
----------------
Three kinds of operations:

1. **Observe** — one-shot queries that return current state (``capture``,
   ``windows``, ``clipboard_read``, ``processes`` …).
2. **Act** — one-shot state changes (``pointer``, ``key``, ``open_app``,
   ``write_file`` …).
3. **Subscribe** — server-pushed event streams returned as async iterators
   (``subscribe_display``, ``subscribe_input`` …).  Cancel a subscription by
   leaving its ``async for`` loop or calling ``aclose()`` on the iterator.

Backends advertise the subset they support via :meth:`capabilities`.  Callers
that need a feature must check the manifest first; calling an unsupported
method raises :class:`CapabilityUnsupported`.
"""

from __future__ import annotations

from abc import ABC, abstractmethod
from typing import AsyncIterator, Optional, Union

from opendesk.computer.types import (
    Capability,
    CapabilityManifest,
    ClipboardContents,
    ClipboardEntry,
    CompletedCommand,
    Display,
    DisplayFrame,
    Environment,
    FileEntry,
    KeyAction,
    KeyEvent,
    Modifier,
    Notification,
    Pixmap,
    Point,
    PointerAction,
    PointerButton,
    PointerEvent,
    Process,
    Rect,
    TextInput,
    UIElement,
    Window,
)


class CapabilityUnsupported(RuntimeError):
    """Raised when a method is called on a backend that doesn't support it.

    Always describes the missing capability so callers can present a useful
    error.  Check :meth:`Computer.capabilities` to avoid this proactively.
    """

    def __init__(self, capability: Capability, backend: str = "") -> None:
        self.capability = capability
        self.backend = backend
        super().__init__(
            f"Capability {capability.value!r} is not supported by backend "
            f"{backend or '<unknown>'}."
        )


InputEvent = Union[PointerEvent, KeyEvent]


class Computer(ABC):
    """Abstract capability surface of a computer.

    Subclasses implement the abstract methods.  Convenience helpers
    (``click``, ``hotkey``, ``type_text``) are concrete and built atop the
    abstract primitives so every backend gets them for free.
    """

    # ------------------------------------------------------------------
    # Introspection
    # ------------------------------------------------------------------

    @abstractmethod
    def capabilities(self) -> CapabilityManifest:
        """Return the set of capabilities this Computer supports.

        Synchronous and cheap — callers may check this on the hot path before
        attempting an operation.
        """

    @abstractmethod
    async def environment(self) -> Environment:
        """Return static facts about the machine (OS, displays, locale …)."""

    # ------------------------------------------------------------------
    # Display
    # ------------------------------------------------------------------

    @abstractmethod
    async def displays(self) -> list[Display]:
        """List connected displays."""

    @abstractmethod
    async def capture(
        self,
        *,
        display_id: Optional[str] = None,
        region: Optional[Rect] = None,
        downscale: bool = True,
    ) -> Pixmap:
        """Capture a screenshot.

        Parameters
        ----------
        display_id:
            Display to capture, or ``None`` for the primary.
        region:
            Sub-region in logical pixels.  ``None`` captures the full display.
        downscale:
            If true the backend may downscale very large displays to keep the
            payload practical.  The returned :class:`Pixmap` always carries the
            true logical screen dimensions in ``logical_width`` /
            ``logical_height`` so coordinates can be translated back.
        """

    @abstractmethod
    async def cursor_position(self) -> Point:
        """Return the current pointer position in logical pixels."""

    # ------------------------------------------------------------------
    # Display subscription
    # ------------------------------------------------------------------

    @abstractmethod
    def subscribe_display(
        self,
        *,
        display_id: Optional[str] = None,
        fps: int = 30,
        region: Optional[Rect] = None,
    ) -> AsyncIterator[DisplayFrame]:
        """Stream display frames at approximately ``fps``.

        The returned async iterator is cancellable: leaving its ``async for``
        loop or calling ``aclose()`` stops the stream and releases resources.
        Backends may downsample to honour their own limits — check
        :meth:`capabilities` ``limits["display.stream"]``.
        """

    # ------------------------------------------------------------------
    # Input — low-level events
    # ------------------------------------------------------------------

    @abstractmethod
    async def pointer(self, event: PointerEvent) -> None:
        """Inject a pointer event (move / down / up / scroll)."""

    @abstractmethod
    async def key(self, event: KeyEvent) -> None:
        """Inject a single key-down or key-up event."""

    @abstractmethod
    async def text(self, text_input: TextInput) -> None:
        """Insert text high-level (clipboard-paste or IME, Unicode-safe)."""

    @abstractmethod
    def subscribe_input(self) -> AsyncIterator[InputEvent]:
        """Stream every pointer / key event observed on the computer.

        Used for recording, human takeover, and live agent observation.
        """

    # ------------------------------------------------------------------
    # Windows / apps
    # ------------------------------------------------------------------

    @abstractmethod
    async def windows(self) -> list[Window]:
        """List all open windows across applications."""

    @abstractmethod
    async def focused_window(self) -> Optional[Window]:
        """Return the currently focused window, if any."""

    @abstractmethod
    async def focus_window(self, window_id: str) -> None:
        """Bring the given window to the foreground."""

    @abstractmethod
    async def move_window(self, window_id: str, bounds: Rect) -> None:
        """Move and resize a window to ``bounds``."""

    @abstractmethod
    async def close_window(self, window_id: str) -> None:
        """Close a window (the app may show a confirmation dialog)."""

    @abstractmethod
    async def open_app(self, name: str) -> None:
        """Launch an application by name."""

    @abstractmethod
    async def close_app(self, name: str) -> None:
        """Quit / terminate an application."""

    @abstractmethod
    async def focus_app(self, name: str) -> None:
        """Bring an application to the foreground."""

    @abstractmethod
    async def list_apps(self) -> list[str]:
        """List visible running applications by display name."""

    # ------------------------------------------------------------------
    # UI tree
    # ------------------------------------------------------------------

    @abstractmethod
    async def ui_tree(
        self,
        *,
        window_id: Optional[str] = None,
        app: Optional[str] = None,
        max_depth: int = 8,
    ) -> UIElement:
        """Return the accessibility tree rooted at the given window or app.

        With no arguments returns the tree of the focused window.
        """

    @abstractmethod
    async def perform_ui_action(
        self,
        element: UIElement,
        action: str = "click",
        *,
        app: Optional[str] = None,
    ) -> None:
        """Invoke an accessibility action on a UI element.

        The backend locates a matching element by ``(role, name, bounds)``
        within ``app`` (or the currently focused app) and invokes the named
        action via the platform's native a11y API — more reliable than
        synthesising a pointer click at ``bounds.center`` because:

        * Elements with no visible bounds (collapsed menu items) can still
          be invoked.
        * The platform handles focus, scrolling, and event dispatch.
        * It works for elements obscured by other windows.

        ``element.metadata`` may carry backend-specific hints — e.g. macOS
        recognises ``{"menu_path": ["File", "Save"]}`` and routes through the
        AppleScript menu syntax.

        Raises :class:`CapabilityUnsupported` when the backend has no a11y
        action support — callers should fall back to a pointer click at
        ``element.bounds.center`` in that case.
        """

    # ------------------------------------------------------------------
    # Clipboard
    # ------------------------------------------------------------------

    @abstractmethod
    async def clipboard_read(self) -> ClipboardContents:
        """Read the system clipboard in all available MIME representations."""

    @abstractmethod
    async def clipboard_write(self, contents: ClipboardContents) -> None:
        """Write ``contents`` to the system clipboard."""

    # ------------------------------------------------------------------
    # Filesystem
    # ------------------------------------------------------------------

    @abstractmethod
    async def read_file(self, path: str) -> bytes: ...

    @abstractmethod
    async def write_file(self, path: str, data: bytes) -> None: ...

    @abstractmethod
    async def list_dir(self, path: str) -> list[FileEntry]: ...

    @abstractmethod
    async def stat(self, path: str) -> FileEntry: ...

    @abstractmethod
    async def delete(self, path: str) -> None: ...

    @abstractmethod
    async def move(self, src: str, dst: str) -> None: ...

    @abstractmethod
    async def mkdir(self, path: str, *, parents: bool = True) -> None: ...

    # ------------------------------------------------------------------
    # Processes
    # ------------------------------------------------------------------

    @abstractmethod
    async def processes(self) -> list[Process]: ...

    @abstractmethod
    async def shell(
        self,
        command: str,
        *,
        timeout: Optional[float] = None,
        cwd: Optional[str] = None,
        env: Optional[dict[str, str]] = None,
    ) -> CompletedCommand:
        """Run ``command`` through the platform shell and return its output."""

    @abstractmethod
    async def exec(
        self,
        argv: list[str],
        *,
        timeout: Optional[float] = None,
        cwd: Optional[str] = None,
        env: Optional[dict[str, str]] = None,
        stdin: Optional[bytes] = None,
    ) -> CompletedCommand:
        """Spawn a process from an argv vector (no shell interpolation)."""

    # ------------------------------------------------------------------
    # Power
    # ------------------------------------------------------------------

    @abstractmethod
    async def lock_screen(self) -> None: ...

    # ------------------------------------------------------------------
    # Notifications
    # ------------------------------------------------------------------

    @abstractmethod
    async def notifications(self) -> list[Notification]: ...

    # ==================================================================
    # Convenience helpers — built on the abstract primitives above
    # ==================================================================

    async def click(
        self,
        point: Point,
        *,
        button: PointerButton = PointerButton.LEFT,
        count: int = 1,
        modifiers: Optional[list[Modifier]] = None,
    ) -> None:
        """Emit ``count`` clicks at ``point`` with optional modifiers held."""
        mods = modifiers or []
        await self.pointer(
            PointerEvent(action=PointerAction.MOVE, point=point, modifiers=mods)
        )
        for _ in range(count):
            await self.pointer(
                PointerEvent(
                    action=PointerAction.DOWN, point=point,
                    button=button, modifiers=mods,
                )
            )
            await self.pointer(
                PointerEvent(
                    action=PointerAction.UP, point=point,
                    button=button, modifiers=mods,
                )
            )

    async def drag(
        self,
        start: Point,
        end: Point,
        *,
        button: PointerButton = PointerButton.LEFT,
        modifiers: Optional[list[Modifier]] = None,
    ) -> None:
        """Press at ``start``, move to ``end``, release."""
        mods = modifiers or []
        await self.pointer(PointerEvent(action=PointerAction.MOVE, point=start, modifiers=mods))
        await self.pointer(
            PointerEvent(action=PointerAction.DOWN, point=start, button=button, modifiers=mods)
        )
        await self.pointer(PointerEvent(action=PointerAction.MOVE, point=end, modifiers=mods))
        await self.pointer(
            PointerEvent(action=PointerAction.UP, point=end, button=button, modifiers=mods)
        )

    async def scroll(
        self,
        point: Point,
        *,
        dx: float = 0.0,
        dy: float = 0.0,
        modifiers: Optional[list[Modifier]] = None,
    ) -> None:
        await self.pointer(
            PointerEvent(
                action=PointerAction.SCROLL, point=point,
                dx=dx, dy=dy, modifiers=modifiers or [],
            )
        )

    async def press(
        self,
        keysym: str,
        *,
        modifiers: Optional[list[Modifier]] = None,
    ) -> None:
        """Press and release a single key."""
        mods = modifiers or []
        for mod in mods:
            await self.key(KeyEvent(action=KeyAction.DOWN, keysym=mod.value, modifiers=[]))
        await self.key(KeyEvent(action=KeyAction.DOWN, keysym=keysym, modifiers=mods))
        await self.key(KeyEvent(action=KeyAction.UP, keysym=keysym, modifiers=mods))
        for mod in reversed(mods):
            await self.key(KeyEvent(action=KeyAction.UP, keysym=mod.value, modifiers=[]))

    async def hotkey(self, keysyms: list[str]) -> None:
        """Press a chord — all keys down, then all up in reverse order."""
        for k in keysyms:
            await self.key(KeyEvent(action=KeyAction.DOWN, keysym=k))
        for k in reversed(keysyms):
            await self.key(KeyEvent(action=KeyAction.UP, keysym=k))

    async def type_text(self, text: str, *, interval_ms: int = 20) -> None:
        await self.text(TextInput(text=text, interval_ms=interval_ms))

    async def clipboard_text(self) -> Optional[str]:
        contents = await self.clipboard_read()
        return contents.text()

    async def clipboard_set_text(self, text: str) -> None:
        await self.clipboard_write(ClipboardContents(entries=[ClipboardEntry.from_text(text)]))

    # ------------------------------------------------------------------
    # Lifecycle
    # ------------------------------------------------------------------

    async def aclose(self) -> None:
        """Release any resources held by this Computer.

        Override in backends that hold sockets, threads, or open subscriptions.
        """
        return None

    async def __aenter__(self) -> "Computer":
        return self

    async def __aexit__(self, exc_type, exc, tb) -> None:
        await self.aclose()
