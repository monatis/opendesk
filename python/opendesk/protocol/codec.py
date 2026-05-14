"""msgpack codec for :mod:`~opendesk.protocol.frames`.

Why msgpack instead of JSON
---------------------------
* ``bytes`` flow as native ``bin`` — no base64, no sideband correlation, no
  inflation.  Pixmap data and process output go on the wire as raw bytes.
* Compact binary representation.  Comparable to protobuf for our payload
  shapes, with no codegen step.
* Polyglot: msgpack libraries exist for every language an :class:`~opendesk
  .computer.Computer` peer might be implemented in.

Encoding rules
--------------
* Each :class:`~opendesk.protocol.frames.Frame` becomes one msgpack object
  (one WebSocket binary message, one length-prefixed QUIC chunk, etc.).
* ``bytes`` values nested in ``params`` / ``result`` / ``payload`` pass
  through transparently — Pydantic's ``model_dump()`` keeps them as ``bytes``,
  and ``msgpack.packb(..., use_bin_type=True)`` emits ``bin`` for them.
* ``str`` is emitted as msgpack ``str`` (UTF-8), preserving the distinction
  from ``bytes`` end-to-end.
"""

from __future__ import annotations

import enum
from typing import Any

import msgpack

from opendesk.protocol.frames import Frame, frame_class


class CodecError(ValueError):
    """Raised when a frame cannot be encoded or decoded."""


def _msgpack_default(obj: Any) -> Any:
    """msgpack ``default`` hook for types it doesn't know natively.

    * ``set`` / ``frozenset`` → ``list`` (Pydantic validates lists back into
      sets on the receive side, so the set semantic is preserved end-to-end).
    * :class:`enum.Enum` → its ``.value``.

    Anything else falls through to a clear :class:`TypeError`, surfaced as a
    :class:`CodecError` by :func:`encode`.
    """
    if isinstance(obj, (set, frozenset)):
        return list(obj)
    if isinstance(obj, enum.Enum):
        return obj.value
    raise TypeError(f"msgpack cannot serialise {type(obj).__name__!r}")


def encode(frame: Frame) -> bytes:
    """Serialise *frame* to a single msgpack-encoded message.

    ``bytes`` fields anywhere inside the frame round-trip as msgpack ``bin``.
    """
    try:
        payload = frame.model_dump(mode="python")
    except Exception as exc:
        raise CodecError(f"failed to dump frame {type(frame).__name__}: {exc}") from exc
    try:
        return msgpack.packb(payload, use_bin_type=True, default=_msgpack_default)
    except Exception as exc:
        raise CodecError(f"msgpack pack failed for {type(frame).__name__}: {exc}") from exc


def decode(data: bytes) -> Frame:
    """Parse *data* into a typed :class:`Frame`.

    Raises :class:`CodecError` on malformed input or unknown frame type.
    Bytes values inside the frame stay as Python ``bytes`` (msgpack ``raw=False``
    semantics with the default unpacker means ``bin`` → ``bytes`` and
    ``str`` → ``str``).
    """
    try:
        obj = msgpack.unpackb(data, raw=False, use_list=True)
    except Exception as exc:
        raise CodecError(f"msgpack unpack failed: {exc}") from exc
    if not isinstance(obj, dict):
        raise CodecError(f"frame must decode to a dict, got {type(obj).__name__}")
    type_name = obj.get("type")
    if not isinstance(type_name, str):
        raise CodecError(f"missing or invalid 'type' field: {type_name!r}")
    try:
        cls = frame_class(type_name)
    except ValueError as exc:
        raise CodecError(str(exc)) from exc
    try:
        return cls.model_validate(obj)
    except Exception as exc:
        raise CodecError(f"frame validation failed for {type_name!r}: {exc}") from exc
