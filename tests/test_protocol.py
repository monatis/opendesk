"""Tests for the opendesk wire protocol.

Covers:

* Frame round-trips through the msgpack codec, including binary fields
  passed through *as bytes* — no base64 anywhere.
* :class:`LoopbackConnection` piping correctness.
* :class:`Peer` semantics: unary call, stream call, cancellation in both
  directions, error code mapping, HELLO handshake.

All tests run in-process via :class:`LoopbackConnection`; no sockets, no
external services.
"""

from __future__ import annotations

import asyncio
from typing import Any, AsyncIterator, Optional

import pytest

from opendesk.protocol import (
    PROTOCOL_VERSION,
    CancelFrame,
    Connection,
    Dispatcher,
    ErrorCode,
    ErrorInfo,
    HelloFrame,
    LoopbackConnection,
    Peer,
    ProtocolError,
    PushFrame,
    ReqFrame,
    ResFrame,
    decode,
    encode,
)
from opendesk.protocol.connection import ConnectionClosed


# ---------------------------------------------------------------------------
# Codec — binary payloads round-trip as bytes, not base64
# ---------------------------------------------------------------------------


class TestCodec:
    def test_req_roundtrip(self):
        frame = ReqFrame(id=1, method="display.capture", params={"x": 1})
        data = encode(frame)
        assert isinstance(data, bytes)
        out = decode(data)
        assert isinstance(out, ReqFrame)
        assert out.id == 1
        assert out.method == "display.capture"
        assert out.params == {"x": 1}

    def test_res_roundtrip_with_bytes_payload_no_base64(self):
        """The whole point: bytes traverse the wire as msgpack bin, not base64."""
        raw_png = b"\x89PNG\r\n\x1a\n" + b"\x00" * 1000
        frame = ResFrame(
            id=42, end=True,
            result={"width": 100, "height": 100, "format": "png", "data": raw_png},
        )
        data = encode(frame)

        # Sanity: there is no base64-looking ASCII chunk of the PNG bytes.
        import base64
        b64 = base64.b64encode(raw_png)
        assert b64 not in data, "bytes were base64-encoded somewhere — protocol leak"
        # Sanity: the raw bytes ARE present verbatim.
        assert raw_png in data, "bytes did not pass through as raw msgpack bin"

        out = decode(data)
        assert isinstance(out, ResFrame)
        assert out.result is not None
        assert isinstance(out.result["data"], bytes)
        assert out.result["data"] == raw_png

    def test_error_frame_roundtrip(self):
        frame = ResFrame(
            id=7, end=True,
            error=ErrorInfo(code=ErrorCode.NOT_FOUND.value, message="missing",
                            details={"path": "/x"}),
        )
        out = decode(encode(frame))
        assert isinstance(out, ResFrame)
        assert out.error is not None
        assert out.error.code == "not_found"
        assert out.error.details == {"path": "/x"}

    def test_hello_roundtrip(self):
        frame = HelloFrame(
            role="server", principal="server-1",
            capabilities={"backend": "local/linux", "caps": ["display.capture"]},
        )
        out = decode(encode(frame))
        assert isinstance(out, HelloFrame)
        assert out.role == "server"
        assert out.capabilities["backend"] == "local/linux"

    def test_unknown_type_rejected(self):
        import msgpack
        data = msgpack.packb({"v": 1, "type": "bogus"}, use_bin_type=True)
        from opendesk.protocol.codec import CodecError
        with pytest.raises(CodecError):
            decode(data)


# ---------------------------------------------------------------------------
# LoopbackConnection
# ---------------------------------------------------------------------------


class TestLoopbackConnection:
    @pytest.mark.asyncio
    async def test_pipes_bytes_both_ways(self):
        a, b = LoopbackConnection.pair()
        await a.send(b"hello")
        assert await b.recv() == b"hello"
        await b.send(b"world")
        assert await a.recv() == b"world"

    @pytest.mark.asyncio
    async def test_close_propagates(self):
        a, b = LoopbackConnection.pair()
        await a.aclose()
        with pytest.raises(ConnectionClosed):
            await b.recv()

    @pytest.mark.asyncio
    async def test_send_on_closed_raises(self):
        a, _ = LoopbackConnection.pair()
        await a.aclose()
        with pytest.raises(ConnectionClosed):
            await a.send(b"x")


# ---------------------------------------------------------------------------
# Dispatcher used by Peer tests
# ---------------------------------------------------------------------------


class RecordingDispatcher:
    """Test Dispatcher with controllable behaviour."""

    def __init__(self) -> None:
        self.calls: list[tuple[str, dict[str, Any]]] = []
        # Override these to script responses.
        self.unary_result: Any = {"ok": True}
        self.stream_items: list[dict[str, Any]] = [{"i": 0}, {"i": 1}, {"i": 2}]
        self.raise_in_unary: Optional[BaseException] = None
        self.stream_delay: float = 0.0
        self.stream_started = asyncio.Event()

    async def call(self, method: str, params: dict[str, Any]) -> Optional[dict[str, Any]]:
        self.calls.append((method, params))
        if self.raise_in_unary is not None:
            raise self.raise_in_unary
        return self.unary_result

    async def stream(
        self, method: str, params: dict[str, Any],
    ) -> AsyncIterator[dict[str, Any]]:
        self.calls.append((method, params))
        self.stream_started.set()
        for item in self.stream_items:
            if self.stream_delay:
                await asyncio.sleep(self.stream_delay)
            yield item


async def _make_pair(dispatcher: Optional[RecordingDispatcher] = None) -> tuple[Peer, Peer, RecordingDispatcher]:
    """Build a connected client+server Peer pair with HELLO done and recv loops running."""
    if dispatcher is None:
        dispatcher = RecordingDispatcher()
    a, b = LoopbackConnection.pair()
    client = Peer(a, role="client")
    server = Peer(b, role="server", dispatcher=dispatcher)

    async def server_hello():
        return await server.hello({"backend": "test-server"})

    async def client_hello():
        return await client.hello({"backend": "test-client"})

    s_task = asyncio.create_task(server_hello())
    c_task = asyncio.create_task(client_hello())
    await asyncio.gather(s_task, c_task)

    client.start()
    server.start()
    return client, server, dispatcher


# ---------------------------------------------------------------------------
# Peer — HELLO handshake
# ---------------------------------------------------------------------------


class TestHandshake:
    @pytest.mark.asyncio
    async def test_hello_exchanges_manifests(self):
        client, server, _ = await _make_pair()
        try:
            assert client.peer_hello is not None
            assert client.peer_hello.role == "server"
            assert client.peer_hello.capabilities["backend"] == "test-server"
            assert server.peer_hello is not None
            assert server.peer_hello.role == "client"
        finally:
            await client.aclose()
            await server.aclose()


# ---------------------------------------------------------------------------
# Peer — unary calls
# ---------------------------------------------------------------------------


class TestUnary:
    @pytest.mark.asyncio
    async def test_unary_call_round_trip(self):
        client, server, disp = await _make_pair()
        try:
            disp.unary_result = {"hello": "world", "blob": b"\x00\x01\x02"}
            result = await client.call("echo", {"x": 1})
            assert result == {"hello": "world", "blob": b"\x00\x01\x02"}
            assert disp.calls == [("echo", {"x": 1})]
        finally:
            await client.aclose()
            await server.aclose()

    @pytest.mark.asyncio
    async def test_unary_call_propagates_error(self):
        client, server, disp = await _make_pair()
        try:
            disp.raise_in_unary = ValueError("bad arg")
            with pytest.raises(ProtocolError) as exc_info:
                await client.call("boom")
            assert exc_info.value.code == ErrorCode.INVALID_ARGUMENT.value
            assert "bad arg" in exc_info.value.message
        finally:
            await client.aclose()
            await server.aclose()

    @pytest.mark.asyncio
    async def test_unary_call_not_found_maps_to_code(self):
        client, server, disp = await _make_pair()
        try:
            disp.raise_in_unary = FileNotFoundError("nope.txt")
            with pytest.raises(ProtocolError) as exc_info:
                await client.call("read")
            assert exc_info.value.code == ErrorCode.NOT_FOUND.value
        finally:
            await client.aclose()
            await server.aclose()

    @pytest.mark.asyncio
    async def test_unary_carries_bytes_without_base64(self):
        client, server, disp = await _make_pair()
        try:
            raw = b"\x89PNG\r\n" + b"\x42" * 5000
            disp.unary_result = {"data": raw, "format": "png"}
            result = await client.call("display.capture")
            assert isinstance(result["data"], bytes)
            assert result["data"] == raw
        finally:
            await client.aclose()
            await server.aclose()


# ---------------------------------------------------------------------------
# Peer — streaming calls
# ---------------------------------------------------------------------------


class TestStreaming:
    @pytest.mark.asyncio
    async def test_stream_yields_all_items(self):
        client, server, disp = await _make_pair()
        try:
            disp.stream_items = [{"i": 0}, {"i": 1}, {"i": 2}]
            seen = []
            async for item in client.stream("subscribe", {"fps": 30}):
                seen.append(item)
            assert seen == [{"i": 0}, {"i": 1}, {"i": 2}]
        finally:
            await client.aclose()
            await server.aclose()

    @pytest.mark.asyncio
    async def test_stream_cancel_sends_cancel_frame(self):
        client, server, disp = await _make_pair()
        try:
            disp.stream_items = [{"i": i} for i in range(1000)]
            disp.stream_delay = 0.01

            seen = 0
            async for _ in client.stream("subscribe"):
                seen += 1
                if seen >= 3:
                    break  # caller-side cancel via break

            # Give the cancel frame time to flow and server task to be cancelled.
            await asyncio.sleep(0.05)
            # The server task should be gone.
            assert not server._inbound, "server should have cancelled its handler"
        finally:
            await client.aclose()
            await server.aclose()


# ---------------------------------------------------------------------------
# Peer — connection closure failure-mode
# ---------------------------------------------------------------------------


class TestClose:
    @pytest.mark.asyncio
    async def test_closing_peer_fails_pending_calls(self):
        """When the server closes mid-call, the client's pending call must terminate.

        The exact exception type depends on timing: if the server's handler task
        receives its cancellation in time to send a ``cancelled`` error RES,
        the client raises :class:`asyncio.CancelledError`; otherwise the recv
        loop sees a closed connection and fails the future with
        :class:`ConnectionClosed`.  Either is correct — what matters is the
        call doesn't hang.
        """
        client, server, disp = await _make_pair()
        try:
            async def hang(*a, **k):
                await asyncio.sleep(10)

            disp.call = hang  # type: ignore[assignment]
            call_task = asyncio.create_task(client.call("hangs"))
            await asyncio.sleep(0.05)
            await server.aclose()
            with pytest.raises((ConnectionClosed, ProtocolError, RuntimeError, asyncio.CancelledError)):
                await call_task
        finally:
            await client.aclose()
