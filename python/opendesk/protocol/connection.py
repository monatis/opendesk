"""The transport-shaped interface :class:`~opendesk.protocol.peer.Peer` runs on.

A :class:`Connection` is a duplex byte-message channel.  The Peer pushes
encoded frames into ``send`` and pulls them from ``recv``.  Concrete
implementations live elsewhere:

* :class:`LoopbackConnection` — two halves connected by in-process queues,
  for hermetic testing.
* ``WebSocketConnection`` (forthcoming) — a single WebSocket binary channel.
* ``QuicConnection`` (later) — one QUIC stream per direction.

The Peer never assumes ordering across different ``send`` calls beyond
in-order delivery on a single Connection.
"""

from __future__ import annotations

import asyncio
from abc import ABC, abstractmethod
from typing import Optional


class ConnectionClosed(ConnectionError):
    """Raised when ``recv`` or ``send`` is called on a closed Connection."""


class Connection(ABC):
    """Duplex byte-message channel.  Each ``send``/``recv`` carries one frame."""

    @abstractmethod
    async def send(self, data: bytes) -> None:
        """Send a single binary message.  Raises :class:`ConnectionClosed`."""

    @abstractmethod
    async def recv(self) -> bytes:
        """Receive a single binary message.

        Blocks until a message arrives.  Raises :class:`ConnectionClosed` when
        the peer half-closes the channel.
        """

    @abstractmethod
    async def aclose(self) -> None:
        """Close the connection.  Idempotent."""

    async def __aenter__(self) -> "Connection":
        return self

    async def __aexit__(self, exc_type, exc, tb) -> None:
        await self.aclose()


class LoopbackConnection(Connection):
    """An in-process Connection — useful for testing :class:`Peer` without sockets.

    Use :meth:`pair` to get the two halves of a single duplex link::

        a, b = LoopbackConnection.pair()
        # Anything ``a.send`` sends, ``b.recv`` will receive, and vice versa.
    """

    def __init__(self, outbox: asyncio.Queue, inbox: asyncio.Queue) -> None:
        self._outbox = outbox
        self._inbox = inbox
        self._closed = False

    @classmethod
    def pair(cls) -> tuple["LoopbackConnection", "LoopbackConnection"]:
        a_to_b: asyncio.Queue = asyncio.Queue()
        b_to_a: asyncio.Queue = asyncio.Queue()
        return cls(a_to_b, b_to_a), cls(b_to_a, a_to_b)

    async def send(self, data: bytes) -> None:
        if self._closed:
            raise ConnectionClosed("send on closed connection")
        await self._outbox.put(data)

    async def recv(self) -> bytes:
        if self._closed:
            raise ConnectionClosed("recv on closed connection")
        data: Optional[bytes] = await self._inbox.get()
        if data is None:
            self._closed = True
            raise ConnectionClosed("peer closed connection")
        return data

    async def aclose(self) -> None:
        if self._closed:
            return
        self._closed = True
        try:
            self._outbox.put_nowait(None)
        except asyncio.QueueFull:  # pragma: no cover
            pass
