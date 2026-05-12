"""Shared test doubles used across the opendesk test suite."""

from __future__ import annotations

from typing import Any, AsyncIterator, Optional

from opendesk.computer import (
    Capability,
    CapabilityManifest,
    ClipboardContents,
    ClipboardEntry,
    CompletedCommand,
    Computer,
    Display,
    DisplayFrame,
    Environment,
    FileEntry,
    InputEvent,
    KeyEvent,
    Notification,
    Pixmap,
    PixmapFormat,
    Point,
    PointerEvent,
    Process,
    Rect,
    TextInput,
    UIElement,
    Window,
)


class FakeComputer(Computer):
    """In-memory :class:`Computer` that records every method invocation.

    Two roles in the test suite:

    * Verifying tools route through ``ctx.computer`` (tests in
      ``test_computer.py``).
    * Standing in as the *remote* computer for protocol round-trip tests
      (``test_remote.py``).  Because it's a real Computer, the
      :class:`~opendesk.computer.ComputerDispatcher` can dispatch all 35-ish
      protocol methods against it.
    """

    BACKEND = "fake"

    def __init__(self) -> None:
        self.calls: list[tuple[str, dict]] = []
        self._clipboard: str = ""

    def _record(self, method: str, **kwargs: Any) -> None:
        self.calls.append((method, kwargs))

    def capabilities(self) -> CapabilityManifest:
        return CapabilityManifest(
            capabilities=set(Capability),
            backend="fake",
        )

    async def environment(self) -> Environment:
        return Environment(os="fake", displays=[
            Display(id="1", bounds=Rect(x=0, y=0, width=1920, height=1080), primary=True),
        ])

    async def displays(self) -> list[Display]:
        return [Display(id="1", bounds=Rect(x=0, y=0, width=1920, height=1080), primary=True)]

    async def capture(self, *, display_id=None, region=None, downscale=True) -> Pixmap:
        self._record("capture", region=region)
        return Pixmap(
            data=b"\x89PNGfake\x00" * 4,
            format=PixmapFormat.PNG,
            width=100, height=100, logical_width=100, logical_height=100,
        )

    async def cursor_position(self) -> Point:
        return Point(x=50, y=60)

    def subscribe_display(self, *, display_id=None, fps=30, region=None) -> AsyncIterator[DisplayFrame]:
        async def _gen():
            for i in range(3):
                yield DisplayFrame(
                    pixmap=Pixmap(
                        data=b"frame" + bytes([i]),
                        format=PixmapFormat.PNG,
                        width=10, height=10, logical_width=10, logical_height=10,
                    ),
                    frame_number=i,
                    keyframe=(i == 0),
                )
        return _gen()

    async def pointer(self, event: PointerEvent) -> None:
        self._record("pointer", event=event)

    async def key(self, event: KeyEvent) -> None:
        self._record("key", event=event)

    async def text(self, text_input: TextInput) -> None:
        self._record("text", text=text_input.text)

    def subscribe_input(self) -> AsyncIterator[InputEvent]:
        async def _gen():
            return
            yield  # pragma: no cover
        return _gen()

    async def windows(self) -> list[Window]:
        return [Window(id="w1", title="Fake Window", app_name="Fake.app", focused=True)]

    async def focused_window(self) -> Optional[Window]:
        return Window(id="w1", title="Fake Window", app_name="Fake.app", focused=True)

    async def focus_window(self, window_id: str) -> None:
        self._record("focus_window", id=window_id)

    async def move_window(self, window_id: str, bounds: Rect) -> None:
        self._record("move_window", id=window_id, bounds=bounds)

    async def close_window(self, window_id: str) -> None:
        self._record("close_window", id=window_id)

    async def open_app(self, name: str) -> None:
        self._record("open_app", name=name)

    async def close_app(self, name: str) -> None:
        self._record("close_app", name=name)

    async def focus_app(self, name: str) -> None:
        self._record("focus_app", name=name)

    async def list_apps(self) -> list[str]:
        self._record("list_apps")
        return ["Fake.app"]

    async def ui_tree(self, *, window_id=None, app=None, max_depth=8) -> UIElement:
        self._record("ui_tree", app=app)
        return UIElement(
            role="window", name=app or "fake",
            children=[
                UIElement(
                    role="button", name="Save",
                    bounds=Rect(x=10, y=20, width=80, height=30),
                    actions=["click"],
                ),
            ],
        )

    async def perform_ui_action(
        self, element: UIElement, action: str = "click", *, app: Optional[str] = None,
    ) -> None:
        self._record("perform_ui_action", element=element, action=action, app=app)

    async def clipboard_read(self) -> ClipboardContents:
        return ClipboardContents(entries=[ClipboardEntry.from_text(self._clipboard)])

    async def clipboard_write(self, contents: ClipboardContents) -> None:
        self._clipboard = contents.text() or ""
        self._record("clipboard_write", text=self._clipboard)

    async def read_file(self, path: str) -> bytes:
        self._record("read_file", path=path)
        return b"file-bytes-\x00\x01\x02"

    async def write_file(self, path: str, data: bytes) -> None:
        self._record("write_file", path=path, data_len=len(data))

    async def list_dir(self, path: str) -> list[FileEntry]:
        return []

    async def stat(self, path: str) -> FileEntry:
        raise FileNotFoundError(path)

    async def delete(self, path: str) -> None:
        self._record("delete", path=path)

    async def move(self, src: str, dst: str) -> None:
        self._record("move", src=src, dst=dst)

    async def mkdir(self, path: str, *, parents=True) -> None:
        self._record("mkdir", path=path)

    async def processes(self) -> list[Process]:
        return []

    async def shell(self, command, *, timeout=None, cwd=None, env=None) -> CompletedCommand:
        self._record("shell", command=command)
        return CompletedCommand(returncode=0, stdout=b"shell-out", stderr=b"")

    async def exec(self, argv, *, timeout=None, cwd=None, env=None, stdin=None) -> CompletedCommand:
        self._record("exec", argv=argv)
        return CompletedCommand(returncode=0, stdout=b"exec-out", stderr=b"")

    async def lock_screen(self) -> None:
        self._record("lock_screen")

    async def notifications(self) -> list[Notification]:
        return []
