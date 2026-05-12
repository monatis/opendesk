"""Value types exchanged through the :class:`~opendesk.computer.Computer` interface.

These types form the wire-level vocabulary of the protocol.  Every value that
crosses the Computer boundary — input events, observations, frames, capability
manifests — is one of the models defined here.

Design rules
------------
* All types are pydantic models so they serialize cleanly to JSON / protobuf
  and round-trip across the eventual remote protocol.
* Coordinates use *logical* pixels (the OS-reported screen size, not the raw
  framebuffer).  :class:`Pixmap` carries both its own pixel dimensions and the
  logical screen size at capture time, so consumers translate coordinates via
  :meth:`Pixmap.to_logical` instead of guessing.
* Enums are string-valued — they round-trip through JSON without integer
  remapping and stay readable in logs.
"""

from __future__ import annotations

import enum
import time
from typing import Any, Optional

from pydantic import BaseModel, ConfigDict, Field


# ---------------------------------------------------------------------------
# Geometry
# ---------------------------------------------------------------------------


class Point(BaseModel):
    """A 2-D coordinate in logical pixels."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    x: float
    y: float

    def as_int(self) -> "Point":
        return Point(x=int(self.x), y=int(self.y))


class Rect(BaseModel):
    """An axis-aligned rectangle in logical pixels."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    x: float
    y: float
    width: float
    height: float

    @property
    def right(self) -> float:
        return self.x + self.width

    @property
    def bottom(self) -> float:
        return self.y + self.height

    @property
    def center(self) -> Point:
        return Point(x=self.x + self.width / 2, y=self.y + self.height / 2)

    def contains(self, point: Point) -> bool:
        return self.x <= point.x <= self.right and self.y <= point.y <= self.bottom


# ---------------------------------------------------------------------------
# Display / Pixmap
# ---------------------------------------------------------------------------


class PixmapFormat(str, enum.Enum):
    PNG = "png"
    JPEG = "jpeg"
    RAW_RGBA = "raw_rgba"
    RAW_RGB = "raw_rgb"


class Pixmap(BaseModel):
    """An encoded screen / region capture and the metadata to interpret it.

    ``data`` holds the encoded image bytes in :attr:`format`.  ``width`` and
    ``height`` are the *pixel* dimensions of ``data`` (may be downscaled from
    the source). ``logical_width`` / ``logical_height`` are the *logical* screen
    dimensions the OS reports — independent of any downscaling applied.

    Coordinates in any input event addressed at this capture should be
    translated via :meth:`to_logical` before being sent back to the Computer.
    """

    model_config = ConfigDict(extra="forbid")

    data: bytes
    format: PixmapFormat = PixmapFormat.PNG
    width: int
    height: int
    logical_width: int
    logical_height: int
    display_id: Optional[str] = None
    captured_at: float = Field(default_factory=time.time)

    @property
    def scale_x(self) -> float:
        """``pixel_width / logical_width`` — usually 1.0 unless downscaled."""
        return self.width / self.logical_width if self.logical_width else 1.0

    @property
    def scale_y(self) -> float:
        return self.height / self.logical_height if self.logical_height else 1.0

    def to_logical(self, point: Point) -> Point:
        """Translate a pixel-space coordinate (as seen in ``data``) to logical."""
        return Point(x=point.x / self.scale_x, y=point.y / self.scale_y)

    def to_pixel(self, point: Point) -> Point:
        """Translate a logical coordinate to pixel-space within ``data``."""
        return Point(x=point.x * self.scale_x, y=point.y * self.scale_y)


class Display(BaseModel):
    """A connected display."""

    model_config = ConfigDict(extra="forbid")

    id: str
    bounds: Rect
    scale_factor: float = 1.0
    primary: bool = False
    name: Optional[str] = None


# ---------------------------------------------------------------------------
# Windows, processes, UI tree
# ---------------------------------------------------------------------------


class Window(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str
    title: str
    app_name: str
    pid: Optional[int] = None
    bounds: Optional[Rect] = None
    focused: bool = False
    minimized: bool = False
    workspace: Optional[str] = None


class Process(BaseModel):
    model_config = ConfigDict(extra="forbid")

    pid: int
    name: str
    ppid: Optional[int] = None
    cmdline: list[str] = Field(default_factory=list)


class UIElement(BaseModel):
    """A node in the platform accessibility tree."""

    model_config = ConfigDict(extra="forbid")

    role: str
    name: str = ""
    value: Optional[str] = None
    bounds: Optional[Rect] = None
    focused: bool = False
    enabled: bool = True
    actions: list[str] = Field(default_factory=list)
    children: list["UIElement"] = Field(default_factory=list)
    metadata: dict[str, Any] = Field(default_factory=dict)


UIElement.model_rebuild()


# ---------------------------------------------------------------------------
# Input events
# ---------------------------------------------------------------------------


class PointerButton(str, enum.Enum):
    LEFT = "left"
    RIGHT = "right"
    MIDDLE = "middle"
    X1 = "x1"
    X2 = "x2"


class Modifier(str, enum.Enum):
    """Keyboard modifiers, normalised across platforms.

    ``META`` is the platform's "command-like" modifier (Cmd on macOS, Win on
    Windows, Super on Linux).  Backends translate to the platform-native key.
    """

    SHIFT = "shift"
    CTRL = "ctrl"
    ALT = "alt"
    META = "meta"
    FN = "fn"
    CAPS_LOCK = "caps_lock"


class PointerAction(str, enum.Enum):
    MOVE = "move"
    DOWN = "down"
    UP = "up"
    SCROLL = "scroll"


class PointerEvent(BaseModel):
    """A pointer event in logical pixels.

    For ``SCROLL`` events ``dx`` / ``dy`` carry the scroll delta (clicks for
    discrete wheels, pixels for high-resolution trackpads).  For ``DOWN`` /
    ``UP`` ``button`` is required.  Future input devices (pen, touch) attach
    ``pressure`` / ``tilt``; backends that don't support them ignore the
    fields.
    """

    model_config = ConfigDict(extra="forbid")

    action: PointerAction
    point: Point
    button: Optional[PointerButton] = None
    modifiers: list[Modifier] = Field(default_factory=list)
    dx: float = 0.0
    dy: float = 0.0
    pressure: Optional[float] = None
    tilt_x: Optional[float] = None
    tilt_y: Optional[float] = None
    timestamp: float = Field(default_factory=time.time)


class KeyAction(str, enum.Enum):
    DOWN = "down"
    UP = "up"


class KeyEvent(BaseModel):
    """A single key-down or key-up event.

    ``keysym`` is a normalised name (``"return"``, ``"escape"``, ``"a"``,
    ``"f5"``, ...).  ``char`` is the Unicode character produced by the key in
    combination with the held modifiers, when applicable.  ``scancode`` is the
    raw platform scancode when available.
    """

    model_config = ConfigDict(extra="forbid")

    action: KeyAction
    keysym: str
    char: Optional[str] = None
    scancode: Optional[int] = None
    modifiers: list[Modifier] = Field(default_factory=list)
    is_repeat: bool = False
    timestamp: float = Field(default_factory=time.time)


class TextInput(BaseModel):
    """High-level text insertion (IME-aware, paste-friendly).

    The backend is free to deliver this via clipboard-paste, a synthesised key
    stream, or platform IME — whichever preserves Unicode correctness.
    """

    model_config = ConfigDict(extra="forbid")

    text: str
    interval_ms: int = 20


# ---------------------------------------------------------------------------
# Clipboard
# ---------------------------------------------------------------------------


class ClipboardEntry(BaseModel):
    """One MIME representation of a clipboard payload."""

    model_config = ConfigDict(extra="forbid")

    mime_type: str
    data: bytes

    @classmethod
    def from_text(cls, text: str) -> "ClipboardEntry":
        return cls(mime_type="text/plain;charset=utf-8", data=text.encode("utf-8"))

    def as_text(self) -> Optional[str]:
        if self.mime_type.startswith("text/"):
            try:
                return self.data.decode("utf-8")
            except UnicodeDecodeError:
                return self.data.decode("utf-8", errors="replace")
        return None


class ClipboardContents(BaseModel):
    """Clipboard contents — may carry multiple MIME representations."""

    model_config = ConfigDict(extra="forbid")

    entries: list[ClipboardEntry] = Field(default_factory=list)

    def text(self) -> Optional[str]:
        for entry in self.entries:
            t = entry.as_text()
            if t is not None:
                return t
        return None


# ---------------------------------------------------------------------------
# Filesystem
# ---------------------------------------------------------------------------


class FileEntry(BaseModel):
    model_config = ConfigDict(extra="forbid")

    path: str
    name: str
    is_dir: bool
    size: int = 0
    mtime: Optional[float] = None
    mode: Optional[int] = None


# ---------------------------------------------------------------------------
# Processes
# ---------------------------------------------------------------------------


class CompletedCommand(BaseModel):
    """Result of a one-shot shell or argv exec."""

    model_config = ConfigDict(extra="forbid")

    returncode: int
    stdout: bytes
    stderr: bytes
    duration: float = 0.0

    def stdout_text(self) -> str:
        return self.stdout.decode("utf-8", errors="replace")

    def stderr_text(self) -> str:
        return self.stderr.decode("utf-8", errors="replace")


# ---------------------------------------------------------------------------
# Environment
# ---------------------------------------------------------------------------


class Environment(BaseModel):
    """Static-ish facts about the remote machine."""

    model_config = ConfigDict(extra="forbid")

    os: str
    os_version: str = ""
    hostname: str = ""
    locale: str = ""
    timezone: str = ""
    displays: list[Display] = Field(default_factory=list)


# ---------------------------------------------------------------------------
# Notifications
# ---------------------------------------------------------------------------


class Notification(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str
    title: str
    body: str = ""
    app: Optional[str] = None
    timestamp: float = Field(default_factory=time.time)


# ---------------------------------------------------------------------------
# Capability manifest
# ---------------------------------------------------------------------------


class Capability(str, enum.Enum):
    """Named capabilities a :class:`Computer` may advertise.

    Names are dotted and stable.  Adding a capability never reuses an existing
    name with different semantics — callers can rely on a name's meaning being
    fixed for the lifetime of the protocol.
    """

    # Display
    DISPLAY_CAPTURE = "display.capture"
    DISPLAY_MULTI = "display.multi_monitor"
    DISPLAY_STREAM = "display.stream"

    # Input
    INPUT_POINTER = "input.pointer"
    INPUT_KEYBOARD = "input.keyboard"
    INPUT_TEXT = "input.text"
    INPUT_PEN = "input.pen"
    INPUT_TOUCH = "input.touch"
    INPUT_IME = "input.ime"

    # UI tree
    UI_TREE = "ui.tree"
    UI_ACTIONS = "ui.actions"
    UI_SUBSCRIBE = "ui.subscribe"

    # Windows
    WINDOWS_LIST = "windows.list"
    WINDOWS_MANIPULATE = "windows.manipulate"
    WINDOWS_SUBSCRIBE = "windows.subscribe"

    # Apps
    APP_LIFECYCLE = "app.lifecycle"

    # Clipboard
    CLIPBOARD_READ = "clipboard.read"
    CLIPBOARD_WRITE = "clipboard.write"
    CLIPBOARD_MULTI_MIME = "clipboard.multi_mime"

    # Filesystem
    FS_READ = "fs.read"
    FS_WRITE = "fs.write"
    FS_WATCH = "fs.watch"

    # Process
    PROCESS_LIST = "process.list"
    PROCESS_SHELL = "process.shell"
    PROCESS_SPAWN = "process.spawn"

    # OCR (derived — usually run client-side, but a remote may offer it)
    OCR = "ocr"

    # Audio
    AUDIO_VOLUME = "audio.volume"
    AUDIO_PLAY = "audio.play"
    AUDIO_STREAM = "audio.stream"

    # Power
    POWER_LOCK = "power.lock"
    POWER_SLEEP = "power.sleep"

    # Notifications
    NOTIFICATIONS_LIST = "notifications.list"
    NOTIFICATIONS_SUBSCRIBE = "notifications.subscribe"


class CapabilityManifest(BaseModel):
    """What a Computer can do, plus any quantitative limits.

    ``limits`` is an open-ended dict keyed by capability name — backends use it
    to advertise things like ``{"display.stream": {"max_fps": 30}}``.
    """

    model_config = ConfigDict(extra="forbid")

    capabilities: set[Capability] = Field(default_factory=set)
    limits: dict[str, dict[str, Any]] = Field(default_factory=dict)
    protocol_version: str = "0.1"
    backend: str = "unknown"

    def has(self, cap: Capability) -> bool:
        return cap in self.capabilities


# ---------------------------------------------------------------------------
# Subscription envelopes
# ---------------------------------------------------------------------------


class DisplayFrame(BaseModel):
    """One frame of a display subscription stream."""

    model_config = ConfigDict(extra="forbid")

    pixmap: Pixmap
    frame_number: int = 0
    keyframe: bool = True
