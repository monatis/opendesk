"""opendesk.protocol — the remoting wire protocol.

Three layers, deliberately decoupled:

* :mod:`~opendesk.protocol.frames` — Pydantic models for the five frame types
  the protocol carries (HELLO, REQ, RES, CANCEL, PUSH).
* :mod:`~opendesk.protocol.codec` — msgpack encode/decode of frames.  Bytes
  fields pass through natively as msgpack ``bin``; no base64.
* :mod:`~opendesk.protocol.connection` — :class:`Connection` ABC plus a
  :class:`LoopbackConnection` for hermetic in-process testing.
* :mod:`~opendesk.protocol.peer` — :class:`Peer` correlates call ids, owns
  in-flight tables, routes cancellation, exposes ``call`` / ``stream`` /
  ``serve`` APIs on top of an arbitrary :class:`Connection`.

Computer remoting (:class:`~opendesk.computer.remote.RemoteComputer`) and the
serve daemon both build on :class:`Peer`.
"""

from opendesk.protocol.codec import decode, encode
from opendesk.protocol.connection import Connection, LoopbackConnection
from opendesk.protocol.frames import (
    PROTOCOL_VERSION,
    CancelFrame,
    ErrorCode,
    ErrorInfo,
    Frame,
    HelloFrame,
    PushFrame,
    ReqFrame,
    ResFrame,
)
from opendesk.protocol.peer import (
    Dispatcher,
    Peer,
    ProtocolError,
    error_to_exception,
    exception_to_error,
)

__all__ = [
    "PROTOCOL_VERSION",
    "Frame",
    "HelloFrame",
    "ReqFrame",
    "ResFrame",
    "CancelFrame",
    "PushFrame",
    "ErrorInfo",
    "ErrorCode",
    "encode",
    "decode",
    "Connection",
    "LoopbackConnection",
    "Peer",
    "Dispatcher",
    "ProtocolError",
    "error_to_exception",
    "exception_to_error",
]
