"""A :class:`Connection` wrapper that AEAD-encrypts every frame.

After a successful handshake (:func:`pair_*` / :func:`auth_*`), the two peers
agree on a pair of session keys — one for each direction.  Every protocol
frame thereafter goes through :class:`EncryptedConnection`, which:

* on ``send`` — increments a per-direction counter, uses it as the AEAD
  nonce, and encrypts the plaintext with ChaCha20-Poly1305;
* on ``recv`` — does the inverse with its own counter.

The counter is **not** transmitted on the wire — both sides know what number
to use next.  A mismatch (replay, drop, reorder) fails AEAD verification and
the connection is closed.  Because the underlying transport is reliable +
ordered (TCP under WebSocket), this is safe.
"""

from __future__ import annotations

import contextlib
from typing import Optional

try:
    from cryptography.exceptions import InvalidTag
    from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305
except ImportError as _exc:  # pragma: no cover
    raise ImportError(
        "cryptography is required: pip install 'opendesk[remote]'"
    ) from _exc

from opendesk.protocol.connection import Connection, ConnectionClosed


_NONCE_BYTES = 12  # ChaCha20-Poly1305


def _nonce(counter: int) -> bytes:
    if counter < 0 or counter >= (1 << 96):  # pragma: no cover
        raise OverflowError("AEAD nonce counter exhausted")
    return counter.to_bytes(_NONCE_BYTES, "big")


class EncryptedConnection(Connection):
    """Wraps another :class:`Connection` with AEAD per-frame encryption.

    The wrapped Connection is owned: closing the wrapper closes the inner.
    """

    KEY_BYTES = 32

    def __init__(
        self,
        inner: Connection,
        *,
        send_key: bytes,
        recv_key: bytes,
        peer_public: Optional[bytes] = None,
    ) -> None:
        if len(send_key) != self.KEY_BYTES or len(recv_key) != self.KEY_BYTES:
            raise ValueError("session keys must be 32 bytes")
        self._inner = inner
        self._send = ChaCha20Poly1305(send_key)
        self._recv = ChaCha20Poly1305(recv_key)
        self._send_counter = 0
        self._recv_counter = 0
        self._peer_public = peer_public
        self._closed = False

    @property
    def peer_public(self) -> Optional[bytes]:
        return self._peer_public

    async def send(self, data: bytes) -> None:
        if self._closed:
            raise ConnectionClosed("send on closed connection")
        ct = self._send.encrypt(_nonce(self._send_counter), data, None)
        self._send_counter += 1
        await self._inner.send(ct)

    async def recv(self) -> bytes:
        if self._closed:
            raise ConnectionClosed("recv on closed connection")
        ct = await self._inner.recv()
        try:
            pt = self._recv.decrypt(_nonce(self._recv_counter), ct, None)
        except InvalidTag as exc:
            await self.aclose()
            raise ConnectionClosed("frame integrity check failed") from exc
        self._recv_counter += 1
        return pt

    async def aclose(self) -> None:
        if self._closed:
            return
        self._closed = True
        with contextlib.suppress(Exception):
            await self._inner.aclose()
