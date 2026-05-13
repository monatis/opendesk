"""Wire-protocol frame schemas.

Every byte that crosses the protocol boundary is one of the five frames
defined here.  Frames are msgpack-encoded (see :mod:`opendesk.protocol.codec`)
and exchanged as single binary messages over whatever
:class:`~opendesk.protocol.connection.Connection` is in use.

Design rules
------------
* All frames carry ``v`` (protocol version) and a ``type`` discriminator.
* ``params`` / ``result`` / ``payload`` / ``capabilities`` / ``auth`` are
  opaque dicts at the frame layer.  Higher layers (RemoteComputer, server
  dispatcher) attach typed Pydantic models to them.
* ``bytes`` values inside those dicts round-trip through msgpack as native
  ``bin`` — never base64.  Frame fields themselves stay scalar / dict / list.
"""

from __future__ import annotations

import enum
from typing import Any, Literal, Optional, Union

from pydantic import BaseModel, ConfigDict, Field


PROTOCOL_VERSION = 1


class ErrorCode(str, enum.Enum):
    """The closed set of error codes the protocol carries.

    Codes are stable strings.  Callers map them to Python exceptions at the
    application layer (:class:`~opendesk.computer.remote.RemoteComputer`
    translates ``capability_unsupported`` to
    :class:`~opendesk.computer.CapabilityUnsupported`, etc.).
    """

    CAPABILITY_UNSUPPORTED = "capability_unsupported"
    PERMISSION_DENIED = "permission_denied"
    INVALID_ARGUMENT = "invalid_argument"
    NOT_FOUND = "not_found"
    TIMEOUT = "timeout"
    CANCELLED = "cancelled"
    INTERNAL = "internal"
    PROTOCOL = "protocol"
    BUSY = "busy"


class ErrorInfo(BaseModel):
    """An error attached to a :class:`ResFrame`."""

    model_config = ConfigDict(extra="forbid")

    code: str = ErrorCode.INTERNAL.value
    message: str = ""
    details: dict[str, Any] = Field(default_factory=dict)


class _FrameBase(BaseModel):
    """Common fields on every frame."""

    model_config = ConfigDict(extra="forbid")

    v: int = PROTOCOL_VERSION


class HelloFrame(_FrameBase):
    """First frame each peer sends after the byte stream opens.

    Carries the protocol version, the peer's role, its principal, the auth
    proof for the current handshake, and the full capability manifest.  The
    receiving peer stores the manifest and uses it to gate subsequent calls.

    A server can short-circuit a connection by sending HELLO with ``error``
    set — e.g. when another controller already holds the single allowed
    session (:attr:`ErrorCode.BUSY`).  The client's :meth:`Peer.hello` will
    raise :class:`~opendesk.protocol.peer.ProtocolError` carrying that code.
    """

    type: Literal["hello"] = "hello"
    role: Literal["client", "server"]
    principal: str = ""
    auth: dict[str, Any] = Field(default_factory=dict)
    capabilities: dict[str, Any] = Field(default_factory=dict)
    error: Optional[ErrorInfo] = None


class ReqFrame(_FrameBase):
    """A request — unary by default, stream-starting when ``stream=True``."""

    type: Literal["req"] = "req"
    id: int
    method: str
    stream: bool = False
    params: dict[str, Any] = Field(default_factory=dict)


class ResFrame(_FrameBase):
    """A response, or a single frame of a server-streaming response.

    Unary calls receive exactly one ``ResFrame`` with ``end=True``.
    Streaming calls receive many frames with ``end=False`` followed by a final
    frame with ``end=True`` (which may carry a last result or be empty).
    """

    type: Literal["res"] = "res"
    id: int
    seq: int = 0
    end: bool = True
    result: Optional[dict[str, Any]] = None
    error: Optional[ErrorInfo] = None


class CancelFrame(_FrameBase):
    """Caller signals that an in-flight call should be aborted.

    Valid for both unary and streaming calls.  The peer SHOULD respond with a
    final ``ResFrame`` carrying ``error.code = "cancelled"``; callers MUST
    tolerate the possibility that the final RES arrives before the cancel is
    processed.
    """

    type: Literal["cancel"] = "cancel"
    id: int
    reason: str = ""


class PushFrame(_FrameBase):
    """Server-originated event, not tied to any prior request.

    Used for session lifecycle, server-side warnings, and other side-channel
    notifications.  Receivers that don't care SHOULD ignore unknown topics.
    """

    type: Literal["push"] = "push"
    topic: str
    payload: dict[str, Any] = Field(default_factory=dict)


Frame = Union[HelloFrame, ReqFrame, ResFrame, CancelFrame, PushFrame]


_FRAME_TYPES: dict[str, type[_FrameBase]] = {
    "hello": HelloFrame,
    "req": ReqFrame,
    "res": ResFrame,
    "cancel": CancelFrame,
    "push": PushFrame,
}


def frame_class(type_name: str) -> type[_FrameBase]:
    """Return the Pydantic class for a wire ``type`` string, or raise."""
    cls = _FRAME_TYPES.get(type_name)
    if cls is None:
        raise ValueError(f"Unknown frame type: {type_name!r}")
    return cls
