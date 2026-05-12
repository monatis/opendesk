"""opendesk.computer — the :class:`Computer` capability surface.

A :class:`Computer` is the unified interface every tool and integration codes
against.  In-tree implementations:

* :class:`LocalComputer` — the machine this process runs on.
* (forthcoming) :class:`RemoteComputer` — a peer reached over the network.

Public surface
--------------
* :class:`Computer`, :class:`LocalComputer`, :class:`CapabilityUnsupported`
* All value types from :mod:`opendesk.computer.types`
"""

from opendesk.computer.base import CapabilityUnsupported, Computer, InputEvent
from opendesk.computer.dispatcher import ComputerDispatcher
from opendesk.computer.local import LocalComputer
from opendesk.computer.remote import RemoteComputer
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
    PixmapFormat,
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

__all__ = [
    # Interface
    "Computer",
    "LocalComputer",
    "RemoteComputer",
    "ComputerDispatcher",
    "InputEvent",
    "CapabilityUnsupported",
    # Types — geometry
    "Point",
    "Rect",
    # Types — display
    "Display",
    "Pixmap",
    "PixmapFormat",
    "DisplayFrame",
    # Types — input
    "PointerAction",
    "PointerButton",
    "PointerEvent",
    "KeyAction",
    "KeyEvent",
    "Modifier",
    "TextInput",
    # Types — windows / processes / ui
    "Window",
    "Process",
    "UIElement",
    # Types — clipboard
    "ClipboardEntry",
    "ClipboardContents",
    # Types — filesystem / processes
    "FileEntry",
    "CompletedCommand",
    # Types — environment / notifications
    "Environment",
    "Notification",
    # Types — capability
    "Capability",
    "CapabilityManifest",
]
