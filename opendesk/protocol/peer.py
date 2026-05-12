""":class:`Peer` — call-id correlator on top of a :class:`Connection`.

One Peer drives one Connection.  Both sides (client and server) of an opendesk
session are Peers — symmetric except for the ``role`` they declare in HELLO
and whether they have a :class:`Dispatcher` to handle inbound requests.

What it owns
------------
* The next call-id (monotonic per Peer).
* Two tables of in-flight outbound calls: ``_pending`` for unary, ``_streams``
  for subscriptions.
* A table of in-flight inbound handler tasks (so they can be cancelled).
* The peer's exchanged HELLO manifest.

What it does
------------
* :meth:`hello` — synchronous HELLO exchange (one frame each way) before
  the recv loop starts.
* :meth:`start` — spawns the background recv loop.  After this, all frames
  arrive via dispatch.
* :meth:`call` / :meth:`stream` — outbound call APIs used by
  :class:`~opendesk.computer.remote.RemoteComputer`.
* Dispatches inbound REQ frames to the registered :class:`Dispatcher`.

Backpressure
------------
Stream queues are bounded.  A slow consumer of one stream applies backpressure
through TCP/WebSocket — which on a single connection unfortunately also
delays other calls (no per-stream credit windows in v1).  This isn't a v1
correctness issue; QUIC v2 removes the limitation by giving each stream its
own QUIC stream.
"""

from __future__ import annotations

import asyncio
import contextlib
import itertools
from typing import Any, AsyncIterator, Optional, Protocol

from opendesk.protocol.codec import CodecError, decode, encode
from opendesk.protocol.connection import Connection, ConnectionClosed
from opendesk.protocol.frames import (
    CancelFrame,
    ErrorCode,
    ErrorInfo,
    Frame,
    HelloFrame,
    PushFrame,
    ReqFrame,
    ResFrame,
)


# ---------------------------------------------------------------------------
# Exceptions and error mapping
# ---------------------------------------------------------------------------


class ProtocolError(RuntimeError):
    """An error received from the peer (or raised locally for protocol faults).

    Carries the stable :class:`~opendesk.protocol.frames.ErrorCode` from the
    wire so callers can branch on it without parsing messages.
    """

    def __init__(self, code: str, message: str = "", details: Optional[dict[str, Any]] = None) -> None:
        super().__init__(f"{code}: {message}" if message else code)
        self.code = code
        self.message = message
        self.details = details or {}


def exception_to_error(exc: BaseException) -> ErrorInfo:
    """Map a handler-side exception to a wire :class:`ErrorInfo`."""
    if isinstance(exc, asyncio.CancelledError):
        return ErrorInfo(code=ErrorCode.CANCELLED.value, message="cancelled")
    if isinstance(exc, ProtocolError):
        return ErrorInfo(code=exc.code, message=exc.message, details=exc.details)

    code = ErrorCode.INTERNAL.value
    try:
        from opendesk.computer.base import CapabilityUnsupported
        if isinstance(exc, CapabilityUnsupported):
            code = ErrorCode.CAPABILITY_UNSUPPORTED.value
            return ErrorInfo(
                code=code, message=str(exc),
                details={"capability": exc.capability.value, "backend": exc.backend},
            )
    except ImportError:
        pass
    try:
        from opendesk.tools.base import PermissionDeniedError
        if isinstance(exc, PermissionDeniedError):
            return ErrorInfo(code=ErrorCode.PERMISSION_DENIED.value, message=str(exc))
    except ImportError:
        pass

    if isinstance(exc, FileNotFoundError):
        return ErrorInfo(code=ErrorCode.NOT_FOUND.value, message=str(exc))
    if isinstance(exc, ValueError):
        return ErrorInfo(code=ErrorCode.INVALID_ARGUMENT.value, message=str(exc))
    if isinstance(exc, asyncio.TimeoutError):
        return ErrorInfo(code=ErrorCode.TIMEOUT.value, message=str(exc))
    return ErrorInfo(code=code, message=str(exc) or type(exc).__name__)


def error_to_exception(err: ErrorInfo) -> BaseException:
    """Map a wire :class:`ErrorInfo` to a Python exception.

    Stable mappings:
    * ``cancelled`` → :class:`asyncio.CancelledError`
    * ``permission_denied`` → :class:`opendesk.tools.base.PermissionDeniedError`
    * everything else → :class:`ProtocolError` carrying the wire code.

    Higher layers (:class:`~opendesk.computer.remote.RemoteComputer`) further
    translate ``capability_unsupported`` to
    :class:`~opendesk.computer.CapabilityUnsupported` where the missing
    :class:`Capability` enum is available.
    """
    code = err.code
    if code == ErrorCode.CANCELLED.value:
        return asyncio.CancelledError(err.message or "cancelled")
    if code == ErrorCode.PERMISSION_DENIED.value:
        try:
            from opendesk.tools.base import PermissionDeniedError
            return PermissionDeniedError(err.message)
        except ImportError:
            pass
    return ProtocolError(code, err.message, err.details)


# ---------------------------------------------------------------------------
# Dispatcher protocol — what a server-side handler looks like to Peer
# ---------------------------------------------------------------------------


class Dispatcher(Protocol):
    """The application object that fulfils inbound requests on the server side.

    The server attaches a Dispatcher (wrapping a
    :class:`~opendesk.computer.LocalComputer` plus policy) to its Peer; the
    Peer routes each inbound :class:`ReqFrame` to either :meth:`call` (unary)
    or :meth:`stream` based on the request's ``stream`` flag.
    """

    async def call(self, method: str, params: dict[str, Any]) -> Optional[dict[str, Any]]:
        ...

    def stream(
        self, method: str, params: dict[str, Any],
    ) -> AsyncIterator[dict[str, Any]]:
        ...


# ---------------------------------------------------------------------------
# Sentinels
# ---------------------------------------------------------------------------


class _StreamEnd:
    """Internal sentinel pushed onto stream queues to signal end-of-stream."""

    __slots__ = ()


_STREAM_END = _StreamEnd()


# ---------------------------------------------------------------------------
# Peer
# ---------------------------------------------------------------------------


class Peer:
    """One end of an opendesk protocol session.

    Lifecycle::

        peer = Peer(connection, role="client")
        peer_hello = await peer.hello(my_manifest)
        peer.start()
        try:
            result = await peer.call("display.capture", {})
            async for frame in peer.stream("display.subscribe", {"fps": 30}):
                ...
        finally:
            await peer.aclose()

    On the server side: pass a :class:`Dispatcher` to handle inbound calls.
    """

    DEFAULT_STREAM_BUFFER = 64

    def __init__(
        self,
        connection: Connection,
        *,
        role: str = "client",
        dispatcher: Optional[Dispatcher] = None,
        stream_buffer: int = DEFAULT_STREAM_BUFFER,
    ) -> None:
        self._conn = connection
        self._role = role
        self._dispatcher = dispatcher
        self._stream_buffer = stream_buffer

        self._id_counter = itertools.count(1)
        self._pending: dict[int, asyncio.Future] = {}
        self._streams: dict[int, asyncio.Queue] = {}
        self._inbound: dict[int, asyncio.Task] = {}
        self._send_lock = asyncio.Lock()
        self._push_handlers: list = []

        self._peer_hello: Optional[HelloFrame] = None
        self._closed = False
        self._run_task: Optional[asyncio.Task] = None
        self._close_reason: Optional[BaseException] = None

    # ------------------------------------------------------------------
    # Properties
    # ------------------------------------------------------------------

    @property
    def role(self) -> str:
        return self._role

    @property
    def peer_hello(self) -> Optional[HelloFrame]:
        return self._peer_hello

    @property
    def closed(self) -> bool:
        return self._closed

    # ------------------------------------------------------------------
    # Lifecycle
    # ------------------------------------------------------------------

    async def hello(
        self,
        capabilities: Optional[dict[str, Any]] = None,
        *,
        principal: str = "",
        auth: Optional[dict[str, Any]] = None,
    ) -> HelloFrame:
        """Exchange HELLO frames.  Must be called before :meth:`start`."""
        await self._send_frame(HelloFrame(
            role=self._role,  # type: ignore[arg-type]
            principal=principal,
            auth=auth or {},
            capabilities=capabilities or {},
        ))
        try:
            data = await self._conn.recv()
        except ConnectionClosed as exc:
            raise ProtocolError(ErrorCode.PROTOCOL.value, "peer closed before HELLO") from exc
        frame = decode(data)
        if not isinstance(frame, HelloFrame):
            raise ProtocolError(
                ErrorCode.PROTOCOL.value,
                f"expected HELLO, got {frame.type!r}",
            )
        self._peer_hello = frame
        return frame

    def start(self) -> None:
        """Spawn the background recv loop.  Idempotent."""
        if self._run_task is None and not self._closed:
            self._run_task = asyncio.create_task(self._run(), name=f"opendesk-peer-{self._role}")

    async def wait_closed(self) -> None:
        """Block until the recv loop exits (i.e. the peer disconnects).

        Useful from a server handler that should keep the WebSocket alive
        for the lifetime of the session::

            await peer.hello(manifest)
            peer.start()
            await peer.wait_closed()
        """
        if self._run_task is None:
            return
        with contextlib.suppress(BaseException):
            await self._run_task

    async def aclose(self, reason: Optional[BaseException] = None) -> None:
        """Close the connection and fail all in-flight calls.  Idempotent."""
        if self._closed:
            return
        self._closed = True
        self._close_reason = reason or ConnectionClosed("peer closed")
        self._fail_all(self._close_reason)
        if self._run_task is not None and self._run_task is not asyncio.current_task():
            self._run_task.cancel()
            with contextlib.suppress(BaseException):
                await self._run_task
        with contextlib.suppress(BaseException):
            await self._conn.aclose()

    async def __aenter__(self) -> "Peer":
        return self

    async def __aexit__(self, exc_type, exc, tb) -> None:
        await self.aclose()

    # ------------------------------------------------------------------
    # Outbound calls
    # ------------------------------------------------------------------

    async def call(
        self, method: str, params: Optional[dict[str, Any]] = None,
    ) -> Optional[dict[str, Any]]:
        """Issue a unary request and await the single response.

        Translates wire errors via :func:`error_to_exception`.  A cancellation
        of this coroutine (e.g. by ``asyncio.wait_for`` timeout) sends a
        CANCEL frame to the peer.
        """
        if self._closed:
            raise ConnectionClosed("peer is closed")
        call_id = next(self._id_counter)
        fut: asyncio.Future = asyncio.get_event_loop().create_future()
        self._pending[call_id] = fut
        try:
            await self._send_frame(ReqFrame(id=call_id, method=method, params=params or {}))
            return await fut
        except asyncio.CancelledError:
            with contextlib.suppress(Exception):
                await self._send_frame(CancelFrame(id=call_id, reason="caller cancelled"))
            raise
        finally:
            self._pending.pop(call_id, None)

    async def stream(
        self, method: str, params: Optional[dict[str, Any]] = None,
    ) -> AsyncIterator[dict[str, Any]]:
        """Issue a streaming request and yield each response frame.

        Backpressure: the iterator's internal queue is bounded; a slow
        consumer slows the recv loop.  Cancelling the iterator (closing the
        generator or breaking out of ``async for``) sends a CANCEL frame.
        """
        if self._closed:
            raise ConnectionClosed("peer is closed")
        call_id = next(self._id_counter)
        queue: asyncio.Queue = asyncio.Queue(maxsize=self._stream_buffer)
        self._streams[call_id] = queue
        cancelled = False
        try:
            await self._send_frame(
                ReqFrame(id=call_id, method=method, stream=True, params=params or {})
            )
            while True:
                item = await queue.get()
                if isinstance(item, _StreamEnd):
                    return
                if isinstance(item, BaseException):
                    raise item
                yield item
        except (asyncio.CancelledError, GeneratorExit):
            cancelled = True
            raise
        finally:
            self._streams.pop(call_id, None)
            if cancelled and not self._closed:
                with contextlib.suppress(Exception):
                    await self._send_frame(CancelFrame(id=call_id, reason="caller cancelled"))

    # ------------------------------------------------------------------
    # Push frames
    # ------------------------------------------------------------------

    def on_push(self, handler) -> None:
        """Register a callback (``async (topic, payload) -> None``) for PUSH frames."""
        self._push_handlers.append(handler)

    async def push(self, topic: str, payload: Optional[dict[str, Any]] = None) -> None:
        """Send a server-originated PUSH frame to the peer."""
        await self._send_frame(PushFrame(topic=topic, payload=payload or {}))

    # ==================================================================
    # Internals — recv loop and dispatch
    # ==================================================================

    async def _run(self) -> None:
        try:
            while True:
                try:
                    data = await self._conn.recv()
                except ConnectionClosed as exc:
                    self._close_reason = exc
                    break
                try:
                    frame = decode(data)
                except CodecError as exc:
                    self._close_reason = ProtocolError(
                        ErrorCode.PROTOCOL.value, f"bad frame: {exc}",
                    )
                    break
                await self._dispatch(frame)
        except asyncio.CancelledError:
            pass
        finally:
            self._closed = True
            self._fail_all(self._close_reason or ConnectionClosed("peer disconnected"))
            for task in list(self._inbound.values()):
                task.cancel()

    async def _dispatch(self, frame: Frame) -> None:
        if isinstance(frame, ResFrame):
            self._dispatch_res(frame)
            return
        if isinstance(frame, ReqFrame):
            await self._dispatch_req(frame)
            return
        if isinstance(frame, CancelFrame):
            self._dispatch_cancel(frame)
            return
        if isinstance(frame, PushFrame):
            await self._dispatch_push(frame)
            return
        if isinstance(frame, HelloFrame):
            self._peer_hello = frame
            return

    def _dispatch_res(self, frame: ResFrame) -> None:
        if frame.id in self._pending:
            fut = self._pending.pop(frame.id)
            if fut.done():
                return
            if frame.error is not None:
                fut.set_exception(error_to_exception(frame.error))
            else:
                fut.set_result(frame.result)
            return
        if frame.id in self._streams:
            queue = self._streams[frame.id]
            if frame.error is not None:
                queue.put_nowait(error_to_exception(frame.error))
                queue.put_nowait(_STREAM_END)
                return
            if frame.end:
                if frame.result is not None:
                    queue.put_nowait(frame.result)
                queue.put_nowait(_STREAM_END)
                return
            if frame.result is not None:
                queue.put_nowait(frame.result)

    async def _dispatch_req(self, frame: ReqFrame) -> None:
        if self._dispatcher is None:
            await self._send_frame(ResFrame(
                id=frame.id, end=True,
                error=ErrorInfo(
                    code=ErrorCode.INTERNAL.value,
                    message="no dispatcher registered",
                ),
            ))
            return
        task = asyncio.create_task(
            self._handle_req(frame), name=f"opendesk-req-{frame.id}",
        )
        self._inbound[frame.id] = task

    def _dispatch_cancel(self, frame: CancelFrame) -> None:
        task = self._inbound.get(frame.id)
        if task is not None and not task.done():
            task.cancel()

    async def _dispatch_push(self, frame: PushFrame) -> None:
        for handler in list(self._push_handlers):
            try:
                await handler(frame.topic, frame.payload)
            except Exception:
                pass

    async def _handle_req(self, frame: ReqFrame) -> None:
        try:
            if frame.stream:
                await self._handle_req_stream(frame)
            else:
                await self._handle_req_unary(frame)
        finally:
            self._inbound.pop(frame.id, None)

    async def _handle_req_unary(self, frame: ReqFrame) -> None:
        assert self._dispatcher is not None
        try:
            result = await self._dispatcher.call(frame.method, frame.params)
        except asyncio.CancelledError:
            with contextlib.suppress(Exception):
                await self._send_frame(ResFrame(
                    id=frame.id, end=True,
                    error=ErrorInfo(code=ErrorCode.CANCELLED.value, message="cancelled"),
                ))
            raise
        except Exception as exc:
            with contextlib.suppress(Exception):
                await self._send_frame(ResFrame(
                    id=frame.id, end=True, error=exception_to_error(exc),
                ))
            return
        await self._send_frame(ResFrame(id=frame.id, end=True, result=result))

    async def _handle_req_stream(self, frame: ReqFrame) -> None:
        assert self._dispatcher is not None
        seq = 0
        try:
            async for item in self._dispatcher.stream(frame.method, frame.params):
                await self._send_frame(
                    ResFrame(id=frame.id, seq=seq, end=False, result=item),
                )
                seq += 1
        except asyncio.CancelledError:
            with contextlib.suppress(Exception):
                await self._send_frame(ResFrame(
                    id=frame.id, seq=seq, end=True,
                    error=ErrorInfo(code=ErrorCode.CANCELLED.value, message="cancelled"),
                ))
            raise
        except Exception as exc:
            with contextlib.suppress(Exception):
                await self._send_frame(ResFrame(
                    id=frame.id, seq=seq, end=True, error=exception_to_error(exc),
                ))
            return
        await self._send_frame(ResFrame(id=frame.id, seq=seq, end=True, result=None))

    # ------------------------------------------------------------------
    # Send / cleanup
    # ------------------------------------------------------------------

    async def _send_frame(self, frame: Frame) -> None:
        data = encode(frame)
        async with self._send_lock:
            await self._conn.send(data)

    def _fail_all(self, exc: BaseException) -> None:
        for fut in self._pending.values():
            if not fut.done():
                fut.set_exception(exc)
        self._pending.clear()
        for queue in self._streams.values():
            with contextlib.suppress(asyncio.QueueFull):
                queue.put_nowait(exc)
                queue.put_nowait(_STREAM_END)
        self._streams.clear()
