"""Internet Remote Transport — Rendezvous & Relay service, Agent, and Client.

Architecture
------------
Enables OpenDesk controllers to reach controlled machines anywhere across the
Internet, including endpoints behind NATs and stateful firewalls, with NO inbound
port forwarding required on the controlled machine.

Components:
* :class:`RendezvousServer` — Lightweight signaling, device discovery, and opaque frame relay.
* :class:`RendezvousAgent`  — Runs on controlled machine; maintains outbound connection,
                              accepts sessions, attempts direct P2P, and falls back to relay.
* :class:`RendezvousClient` — Controller side; initiates session through rendezvous, attempts
                              direct P2P, and returns a ready :class:`Connection`.
* :class:`RelayConnection`  — Duplex byte-message :class:`Connection` over relay WebSocket.
* :class:`DirectSocketConnection` — Duplex byte-message :class:`Connection` over direct TCP stream.

Security:
OpenDesk's end-to-end cryptographic security is completely preserved:
Noise-like handshake (X25519 DH + ChaCha20-Poly1305 AEAD) is performed *over* the
resulting Connection between controller and controlled machine. The relay forwards
opaque binary frames and cannot read keystrokes, commands, or display captures.
"""

from __future__ import annotations

import asyncio
import contextlib
import json
import logging
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Awaitable, Callable, Optional, Union
import time
import uuid

try:
    import websockets
    from websockets.asyncio.client import connect as ws_connect
    from websockets.asyncio.server import Server, serve as ws_serve
    from websockets.exceptions import ConnectionClosed as _WSClosed
except ImportError as _exc:  # pragma: no cover
    raise ImportError(
        "websockets is required for OpenDesk internet remote transport. "
        "Install with: pip install 'opendesk[remote]'"
    ) from _exc

from opendesk.protocol.auth.identity import Identity
from opendesk.protocol.connection import Connection, ConnectionClosed
from opendesk.remote.discovery import DiscoveredPeer
from opendesk.wsl import local_ipv4s


log = logging.getLogger("opendesk.remote.rendezvous")

DEFAULT_RENDEZVOUS_PORT = 8424
P2P_HANDSHAKE_PREFIX = b"OPENDESK-P2P:"


# ---------------------------------------------------------------------------
# Connection Implementations
# ---------------------------------------------------------------------------


class RelayConnection(Connection):
    """Duplex byte-message :class:`Connection` tunneled through a relay WebSocket.

    Each call to :meth:`send` transmits one binary WebSocket frame.
    Each call to :meth:`recv` retrieves one binary WebSocket frame.
    Text frames are rejected or treated as signaling termination.
    """

    def __init__(self, ws: Any) -> None:
        self._ws = ws
        self._closed = False

    async def send(self, data: bytes) -> None:
        if self._closed:
            raise ConnectionClosed("send on closed relay connection")
        try:
            await self._ws.send(data)
        except _WSClosed as exc:
            self._closed = True
            raise ConnectionClosed(str(exc)) from exc
        except Exception as exc:
            self._closed = True
            raise ConnectionClosed(f"relay send failed: {exc}") from exc

    async def recv(self) -> bytes:
        if self._closed:
            raise ConnectionClosed("recv on closed relay connection")
        try:
            msg = await self._ws.recv()
        except _WSClosed as exc:
            self._closed = True
            raise ConnectionClosed(str(exc)) from exc
        except Exception as exc:
            self._closed = True
            raise ConnectionClosed(f"relay recv failed: {exc}") from exc

        if isinstance(msg, str):
            with contextlib.suppress(Exception):
                parsed = json.loads(msg)
                if parsed.get("action") == "close":
                    self._closed = True
                    raise ConnectionClosed(parsed.get("reason", "peer closed session"))
            self._closed = True
            raise ConnectionClosed("received unexpected text frame in relay data mode")
        return msg

    async def aclose(self) -> None:
        if self._closed:
            return
        self._closed = True
        with contextlib.suppress(Exception):
            await self._ws.send(json.dumps({"action": "close"}))
        with contextlib.suppress(Exception):
            await self._ws.close()


class DirectSocketConnection(Connection):
    """Duplex byte-message :class:`Connection` over direct TCP reader/writer streams.

    Uses a 4-byte big-endian length prefix for each frame.
    """

    def __init__(
        self,
        reader: asyncio.StreamReader,
        writer: asyncio.StreamWriter,
    ) -> None:
        self._reader = reader
        self._writer = writer
        self._closed = False

    async def send(self, data: bytes) -> None:
        if self._closed:
            raise ConnectionClosed("send on closed direct socket")
        try:
            length = len(data)
            self._writer.write(length.to_bytes(4, "big") + data)
            await self._writer.drain()
        except Exception as exc:
            self._closed = True
            raise ConnectionClosed(f"direct send failed: {exc}") from exc

    async def recv(self) -> bytes:
        if self._closed:
            raise ConnectionClosed("recv on closed direct socket")
        try:
            header = await self._reader.readexactly(4)
            length = int.from_bytes(header, "big")
            return await self._reader.readexactly(length)
        except asyncio.IncompleteReadError as exc:
            self._closed = True
            raise ConnectionClosed("direct peer closed socket") from exc
        except Exception as exc:
            self._closed = True
            raise ConnectionClosed(f"direct recv failed: {exc}") from exc

    async def aclose(self) -> None:
        if self._closed:
            return
        self._closed = True
        with contextlib.suppress(Exception):
            self._writer.close()
            await self._writer.wait_closed()


# ---------------------------------------------------------------------------
# Direct P2P Traversal Helpers
# ---------------------------------------------------------------------------


def gather_candidates(
    port: int,
    reflexive: Optional[tuple[str, int]] = None,
) -> list[dict[str, Any]]:
    """Gather host and reflexive address candidates for direct connection."""
    cands: list[dict[str, Any]] = []
    seen = set()

    # Host LAN candidates
    try:
        ips = local_ipv4s()
    except Exception:
        ips = []
    for ip in ips:
        if ip and ip != "0.0.0.0":
            key = (ip, port)
            if key not in seen:
                seen.add(key)
                cands.append({"type": "host", "ip": ip, "port": port})

    # Always include localhost for hermetic testing or same-machine cross-net
    if ("127.0.0.1", port) not in seen:
        cands.append({"type": "host", "ip": "127.0.0.1", "port": port})
        seen.add(("127.0.0.1", port))

    # Server-reflexive (public) candidate discovered by rendezvous
    if reflexive and reflexive[0] and reflexive[1]:
        key = (reflexive[0], reflexive[1])
        if key not in seen:
            cands.append({"type": "srflx", "ip": reflexive[0], "port": reflexive[1]})
            seen.add(key)

    return cands


class P2PAcceptor:
    """Ephemeral TCP listener for direct P2P incoming connection attempts."""

    def __init__(self, session_id: str) -> None:
        self.session_id = session_id
        self.server: Optional[asyncio.Server] = None
        self.port: int = 0
        self.conn_future: asyncio.Future[tuple[asyncio.StreamReader, asyncio.StreamWriter]] = (
            asyncio.get_running_loop().create_future()
        )

    async def start(self) -> int:
        async def _client_connected(
            reader: asyncio.StreamReader,
            writer: asyncio.StreamWriter,
        ) -> None:
            try:
                line = await asyncio.wait_for(reader.readline(), timeout=1.5)
                expected = P2P_HANDSHAKE_PREFIX + self.session_id.encode("utf-8") + b"\n"
                if line == expected:
                    if not self.conn_future.done():
                        writer.write(b"OK\n")
                        await writer.drain()
                        self.conn_future.set_result((reader, writer))
                        with contextlib.suppress(Exception):
                            await writer.wait_closed()
                        return
                    else:
                        writer.write(b"BUSY\n")
                        await writer.drain()
            except Exception:
                pass
            with contextlib.suppress(Exception):
                writer.close()

        self.server = await asyncio.start_server(_client_connected, "0.0.0.0", 0)
        self.port = self.server.sockets[0].getsockname()[1]
        return self.port

    async def close(self) -> None:
        if self.server is not None:
            self.server.close()
            self.server = None


async def try_connect_candidates(
    candidates: list[dict[str, Any]],
    session_id: str,
    timeout: float = 1.0,
) -> Optional[tuple[asyncio.StreamReader, asyncio.StreamWriter]]:
    """Attempt concurrent direct TCP connections to a list of candidate endpoints."""
    if not candidates:
        return None

    async def _try_one(cand: dict[str, Any]) -> Optional[tuple[asyncio.StreamReader, asyncio.StreamWriter]]:
        ip = cand.get("ip")
        port = cand.get("port")
        if not ip or not port:
            return None
        try:
            reader, writer = await asyncio.wait_for(
                asyncio.open_connection(ip, int(port)),
                timeout=timeout,
            )
            # Send handshake token
            writer.write(P2P_HANDSHAKE_PREFIX + session_id.encode("utf-8") + b"\n")
            await writer.drain()
            resp = await asyncio.wait_for(reader.readline(), timeout=timeout)
            if resp == b"OK\n":
                return reader, writer
            writer.close()
        except Exception:
            pass
        return None

    tasks = [asyncio.create_task(_try_one(c)) for c in candidates]
    try:
        for fut in asyncio.as_completed(tasks, timeout=timeout + 0.5):
            try:
                res = await fut
                if res is not None:
                    for t in tasks:
                        if not t.done():
                            t.cancel()
                    return res
            except Exception:
                pass
    except Exception:
        pass
    finally:
        for t in tasks:
            if not t.done():
                t.cancel()
    return None


# ---------------------------------------------------------------------------
# Rendezvous & Relay Server
# ---------------------------------------------------------------------------


@dataclass
class RegisteredAgent:
    public_key: str  # hex
    name: str
    description: str
    ws: Any
    remote_addr: tuple[str, int]
    registered_at: float = field(default_factory=time.time)


@dataclass
class PendingSession:
    session_id: str
    target_public_key: str
    controller_ws: Any
    controller_remote_addr: tuple[str, int]
    controller_candidates: list[dict[str, Any]]
    agent_ws: Optional[Any] = None
    agent_remote_addr: Optional[tuple[str, int]] = None
    agent_candidates: list[dict[str, Any]] = field(default_factory=list)
    joined_event: asyncio.Event = field(default_factory=asyncio.Event)
    ready_event: asyncio.Event = field(default_factory=asyncio.Event)
    direct_p2p_succeeded: bool = False


class RendezvousServer:
    """Self-hosted Internet Rendezvous and Relay Server.

    Maintains device identity registrations, coordinates session signaling /
    candidate exchange for direct P2P, and acts as an opaque binary relay when
    NAT/firewall traversal prevents direct connection.
    """

    def __init__(
        self,
        host: str = "0.0.0.0",
        port: int = DEFAULT_RENDEZVOUS_PORT,
        *,
        token: Optional[str] = None,
    ) -> None:
        self.host = host
        self.port = port
        self.token = token
        self._server: Optional[Server] = None
        self._agents: dict[str, RegisteredAgent] = {}
        self._sessions: dict[str, PendingSession] = {}
        self._lock = asyncio.Lock()
        self._closed = False

    async def start(self) -> None:
        if self._server is not None:
            return
        self._server = await ws_serve(
            self._handle_ws_connection,
            self.host,
            self.port,
            max_size=None,
            compression=None,
        )
        # Resolve actual port when port=0 was requested
        socks = getattr(self._server, "sockets", None) or []
        for s in socks:
            try:
                self.port = s.getsockname()[1]
                break
            except Exception:
                continue
        log.info("Rendezvous server listening on %s:%d", self.host, self.port)

    async def aclose(self) -> None:
        if self._closed:
            return
        self._closed = True
        if self._server is not None:
            self._server.close()
            with contextlib.suppress(Exception):
                await self._server.wait_closed()
            self._server = None

        # Terminate active agents and sessions
        async with self._lock:
            for agent in list(self._agents.values()):
                with contextlib.suppress(Exception):
                    await agent.ws.close()
            self._agents.clear()

            for sess in list(self._sessions.values()):
                if sess.controller_ws:
                    with contextlib.suppress(Exception):
                        await sess.controller_ws.close()
                if sess.agent_ws:
                    with contextlib.suppress(Exception):
                        await sess.agent_ws.close()
            self._sessions.clear()

    async def wait_closed(self) -> None:
        if self._server is not None:
            await self._server.wait_closed()

    async def serve_forever(self) -> None:
        if self._server is None:
            await self.start()
        await self.wait_closed()

    # ------------------------------------------------------------------
    # Dispatcher
    # ------------------------------------------------------------------

    async def _handle_ws_connection(self, ws: Any) -> None:
        # Determine client address
        remote_addr = ("?", 0)
        if getattr(ws, "remote_address", None):
            remote_addr = ws.remote_address

        try:
            # Read first message to determine connection intent
            first_msg = await ws.recv()
            if not isinstance(first_msg, str):
                await ws.close(1002, "expected JSON handshake message")
                return

            data = json.loads(first_msg)
            action = data.get("action")

            # Validate authentication token if configured
            if self.token and data.get("token") != self.token:
                await ws.send(json.dumps({"status": "error", "error": "unauthorized"}))
                await ws.close(1008, "unauthorized")
                return

            if action == "register":
                await self._handle_agent_register(ws, data, remote_addr)
            elif action == "connect":
                await self._handle_session_connect(ws, data, remote_addr)
            elif action == "join":
                await self._handle_session_join(ws, data, remote_addr)
            elif action == "list":
                await self._handle_list(ws)
            elif action == "lookup":
                await self._handle_lookup(ws, data)
            else:
                await ws.send(json.dumps({"status": "error", "error": f"unknown action: {action}"}))
                await ws.close(1002, "unknown action")
        except _WSClosed:
            pass
        except Exception as exc:
            log.warning("error in rendezvous connection handler: %s", exc)
            with contextlib.suppress(Exception):
                await ws.close(1011, str(exc))

    # ------------------------------------------------------------------
    # Agent Registration
    # ------------------------------------------------------------------

    async def _handle_agent_register(
        self,
        ws: Any,
        data: dict[str, Any],
        remote_addr: tuple[str, int],
    ) -> None:
        public_key = data.get("public_key")
        if not public_key:
            await ws.send(json.dumps({"status": "error", "error": "missing public_key"}))
            return

        name = data.get("name", "")
        description = data.get("description", "")
        agent = RegisteredAgent(
            public_key=public_key,
            name=name,
            description=description,
            ws=ws,
            remote_addr=remote_addr,
        )

        async with self._lock:
            # Kick previous connection for same key if any
            if public_key in self._agents:
                old = self._agents[public_key]
                with contextlib.suppress(Exception):
                    await old.ws.close(1000, "replaced by new registration")
            self._agents[public_key] = agent

        log.info(
            "Registered agent %s (%s) from %s:%d",
            name or public_key[:8],
            public_key[:8],
            remote_addr[0],
            remote_addr[1],
        )

        await ws.send(json.dumps({
            "status": "ok",
            "action": "registered",
            "public_key": public_key,
            "client_ip": remote_addr[0],
            "client_port": remote_addr[1],
        }))

        # Keep agent connection alive and listen for ping/heartbeat
        try:
            async for msg in ws:
                if isinstance(msg, str):
                    try:
                        cmd = json.loads(msg)
                        if cmd.get("action") == "ping":
                            await ws.send(json.dumps({"action": "pong"}))
                    except Exception:
                        pass
        finally:
            async with self._lock:
                if self._agents.get(public_key) and self._agents[public_key].ws is ws:
                    del self._agents[public_key]
                    log.info("Agent %s disconnected", public_key[:8])

    # ------------------------------------------------------------------
    # Controller Session Connection
    # ------------------------------------------------------------------

    async def _handle_session_connect(
        self,
        ws: Any,
        data: dict[str, Any],
        remote_addr: tuple[str, int],
    ) -> None:
        target = data.get("target")
        if not target:
            await ws.send(json.dumps({"status": "error", "error": "missing target"}))
            return

        session_id = data.get("session_id") or uuid.uuid4().hex
        candidates = data.get("candidates", [])

        async with self._lock:
            agent = self._agents.get(target)
            if not agent:
                await ws.send(json.dumps({
                    "status": "error",
                    "error": "target_offline",
                    "message": f"Target peer {target[:12]}... is not registered or offline",
                }))
                return

            sess = PendingSession(
                session_id=session_id,
                target_public_key=target,
                controller_ws=ws,
                controller_remote_addr=remote_addr,
                controller_candidates=candidates,
            )
            self._sessions[session_id] = sess

        # Notify the registered agent over its control channel
        try:
            await agent.ws.send(json.dumps({
                "action": "session_request",
                "session_id": session_id,
                "controller_remote_addr": list(remote_addr),
                "controller_candidates": candidates,
            }))
        except Exception as exc:
            async with self._lock:
                self._sessions.pop(session_id, None)
            await ws.send(json.dumps({
                "status": "error",
                "error": "agent_unreachable",
                "message": f"Failed to signal agent: {exc}",
            }))
            return

        # Wait for agent to join
        try:
            await asyncio.wait_for(sess.joined_event.wait(), timeout=15.0)
        except asyncio.TimeoutError:
            async with self._lock:
                self._sessions.pop(session_id, None)
            await ws.send(json.dumps({
                "status": "error",
                "error": "agent_timeout",
                "message": "Timed out waiting for agent to join session",
            }))
            return

        # Forward agent candidates to controller
        await ws.send(json.dumps({
            "status": "joined",
            "session_id": session_id,
            "agent_candidates": sess.agent_candidates,
            "agent_remote_addr": list(sess.agent_remote_addr) if sess.agent_remote_addr else [],
        }))

        # Wait for either direct P2P completion or relay ready
        await self._await_session_relay(sess)

    # ------------------------------------------------------------------
    # Agent Session Join
    # ------------------------------------------------------------------

    async def _handle_session_join(
        self,
        ws: Any,
        data: dict[str, Any],
        remote_addr: tuple[str, int],
    ) -> None:
        session_id = data.get("session_id")
        if not session_id:
            await ws.send(json.dumps({"status": "error", "error": "missing session_id"}))
            return

        async with self._lock:
            sess = self._sessions.get(session_id)
            if not sess:
                await ws.send(json.dumps({"status": "error", "error": "session_not_found"}))
                return
            sess.agent_ws = ws
            sess.agent_remote_addr = remote_addr
            sess.agent_candidates = data.get("candidates", [])
            sess.joined_event.set()

        await ws.send(json.dumps({
            "status": "joined",
            "session_id": session_id,
        }))

        # Keep alive while session is negotiating / active
        try:
            async for raw in ws:
                if isinstance(raw, str):
                    try:
                        msg = json.loads(raw)
                        act = msg.get("action")
                        if act == "ready":
                            sess.ready_event.set()
                        elif act == "direct_ok":
                            sess.direct_p2p_succeeded = True
                            sess.ready_event.set()
                            break
                        elif act == "close":
                            if sess.controller_ws is not None:
                                with contextlib.suppress(Exception):
                                    await sess.controller_ws.send(raw)
                            break
                    except Exception:
                        pass
                elif isinstance(raw, bytes):
                    # In relay mode, controller receives binary frames
                    if sess.controller_ws:
                        with contextlib.suppress(Exception):
                            await sess.controller_ws.send(raw)
        finally:
            async with self._lock:
                self._sessions.pop(session_id, None)
            if not sess.direct_p2p_succeeded and sess.controller_ws is not None:
                with contextlib.suppress(Exception):
                    await sess.controller_ws.close()

    # ------------------------------------------------------------------
    # Relay Pump
    # ------------------------------------------------------------------

    async def _await_session_relay(self, sess: PendingSession) -> None:
        ws = sess.controller_ws
        try:
            async for raw in ws:
                if isinstance(raw, str):
                    try:
                        msg = json.loads(raw)
                        act = msg.get("action")
                        if act == "ready":
                            sess.ready_event.set()
                            # Notify both sides that relay is active
                            await ws.send(json.dumps({
                                "status": "relay_active",
                                "session_id": sess.session_id,
                            }))
                            if sess.agent_ws:
                                await sess.agent_ws.send(json.dumps({
                                    "status": "relay_active",
                                    "session_id": sess.session_id,
                                }))
                        elif act == "direct_ok":
                            sess.direct_p2p_succeeded = True
                            return
                        elif act == "close":
                            if sess.agent_ws is not None:
                                with contextlib.suppress(Exception):
                                    await sess.agent_ws.send(raw)
                            break
                    except Exception:
                        pass
                elif isinstance(raw, bytes):
                    # Forward binary frame to agent
                    if sess.agent_ws:
                        with contextlib.suppress(Exception):
                            await sess.agent_ws.send(raw)
        finally:
            async with self._lock:
                self._sessions.pop(sess.session_id, None)
            if not sess.direct_p2p_succeeded and sess.agent_ws is not None:
                with contextlib.suppress(Exception):
                    await sess.agent_ws.close()

    # ------------------------------------------------------------------
    # Query / Discovery
    # ------------------------------------------------------------------

    async def _handle_list(self, ws: Any) -> None:
        async with self._lock:
            peers = [
                {
                    "public_key": p.public_key,
                    "name": p.name,
                    "description": p.description,
                    "remote_addr": f"{p.remote_addr[0]}:{p.remote_addr[1]}",
                    "online": True,
                }
                for p in self._agents.values()
            ]
        await ws.send(json.dumps({"status": "ok", "peers": peers}))

    async def _handle_lookup(self, ws: Any, data: dict[str, Any]) -> None:
        target = data.get("target")
        async with self._lock:
            agent = self._agents.get(target)
            if agent:
                res = {
                    "public_key": agent.public_key,
                    "name": agent.name,
                    "description": agent.description,
                    "remote_addr": f"{agent.remote_addr[0]}:{agent.remote_addr[1]}",
                    "online": True,
                }
            else:
                res = None
        await ws.send(json.dumps({"status": "ok", "peer": res, "online": agent is not None}))


# ---------------------------------------------------------------------------
# Rendezvous Agent (Controlled Machine)
# ---------------------------------------------------------------------------


class RendezvousAgent:
    """Outbound background agent on the controlled machine.

    Connects outbound to a Rendezvous server, registers its identity, keeps
    connection alive with automatic reconnect on failure, and dispatches incoming
    sessions to the server's connection handler.
    """

    def __init__(
        self,
        rendezvous_url: str,
        identity: Identity,
        connection_handler: Callable[[Connection], Awaitable[None]],
        *,
        token: Optional[str] = None,
        name: str = "",
        description_getter: Optional[Callable[[], str]] = None,
        enable_p2p: bool = True,
    ) -> None:
        self.rendezvous_url = rendezvous_url.rstrip("/")
        self.identity = identity
        self.connection_handler = connection_handler
        self.token = token
        self.name = name
        self.description_getter = description_getter
        self.enable_p2p = enable_p2p

        self._task: Optional[asyncio.Task] = None
        self._closed = False
        self._registered_event = asyncio.Event()
        self._reflexive_addr: Optional[tuple[str, int]] = None

    async def start(self) -> None:
        if self._task is not None:
            return
        self._closed = False
        self._task = asyncio.create_task(self._run_loop())

    async def wait_registered(self, timeout: float = 5.0) -> None:
        """Wait until registration with the rendezvous server completes."""
        await asyncio.wait_for(self._registered_event.wait(), timeout=timeout)

    async def aclose(self) -> None:
        self._closed = True
        if self._task is not None:
            self._task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await self._task
            self._task = None

    async def _run_loop(self) -> None:
        backoff = 1.0
        while not self._closed:
            try:
                await self._connect_and_serve()
                backoff = 1.0
            except asyncio.CancelledError:
                break
            except Exception as exc:
                if not self._closed:
                    log.warning(
                        "Rendezvous connection to %s failed: %s; reconnecting in %.1fs",
                        self.rendezvous_url,
                        exc,
                        backoff,
                    )
                    await asyncio.sleep(backoff)
                    backoff = min(backoff * 1.5, 30.0)

    async def _connect_and_serve(self) -> None:
        desc = self.description_getter() if self.description_getter else ""
        pubkey = self.identity.public_bytes.hex()

        ws = await ws_connect(
            self.rendezvous_url,
            max_size=None,
            compression=None,
        )
        try:
            # Send registration
            await ws.send(json.dumps({
                "action": "register",
                "public_key": pubkey,
                "name": self.name,
                "description": desc,
                "token": self.token,
            }))

            ack_raw = await ws.recv()
            if not isinstance(ack_raw, str):
                raise RuntimeError("unexpected response from rendezvous server")
            ack = json.loads(ack_raw)
            if ack.get("status") != "ok":
                raise RuntimeError(f"registration failed: {ack.get('error', 'unknown error')}")

            client_ip = ack.get("client_ip")
            client_port = ack.get("client_port")
            if client_ip and client_port:
                self._reflexive_addr = (client_ip, int(client_port))

            self._registered_event.set()
            log.info("Agent registered with rendezvous %s", self.rendezvous_url)

            # Listen for session requests and heartbeats
            async for raw in ws:
                if not isinstance(raw, str):
                    continue
                try:
                    msg = json.loads(raw)
                    action = msg.get("action")
                    if action == "session_request":
                        session_id = msg["session_id"]
                        controller_cands = msg.get("controller_candidates", [])
                        asyncio.create_task(
                            self._handle_incoming_session(session_id, controller_cands)
                        )
                except Exception as exc:
                    log.warning("error processing rendezvous message: %s", exc)
        finally:
            self._registered_event.clear()
            with contextlib.suppress(Exception):
                await ws.close()

    async def _handle_incoming_session(
        self,
        session_id: str,
        controller_candidates: list[dict[str, Any]],
    ) -> None:
        """Handle incoming session request: join session, negotiate P2P, dispatch Connection."""
        session_ws = None
        acceptor: Optional[P2PAcceptor] = None
        try:
            session_ws = await ws_connect(
                self.rendezvous_url,
                max_size=None,
                compression=None,
            )

            candidates: list[dict[str, Any]] = []
            if self.enable_p2p:
                acceptor = P2PAcceptor(session_id)
                port = await acceptor.start()
                candidates = gather_candidates(port, self._reflexive_addr)

            # Join session on rendezvous
            await session_ws.send(json.dumps({
                "action": "join",
                "session_id": session_id,
                "candidates": candidates,
                "token": self.token,
            }))

            resp_raw = await session_ws.recv()
            if not isinstance(resp_raw, str):
                raise RuntimeError("invalid join response from rendezvous")
            resp = json.loads(resp_raw)
            if resp.get("status") != "joined":
                raise RuntimeError(f"failed to join session: {resp.get('error')}")

            # Try direct P2P connection if enabled
            direct_conn: Optional[Connection] = None
            if self.enable_p2p and acceptor is not None:
                # 1. As the callee / agent, wait for controller to dial our acceptor
                try:
                    reader, writer = await asyncio.wait_for(
                        asyncio.shield(acceptor.conn_future),
                        timeout=1.2,
                    )
                    direct_conn = DirectSocketConnection(reader, writer)
                except asyncio.TimeoutError:
                    # 2. If controller could not connect (e.g. agent NAT blocks inbound), reverse dial
                    direct_sock = await try_connect_candidates(controller_candidates, session_id, timeout=1.2)
                    if direct_sock is not None:
                        direct_conn = DirectSocketConnection(direct_sock[0], direct_sock[1])

            if direct_conn is not None:
                log.info("Session %s established via Direct P2P", session_id[:8])
                # Close relay channel and acceptor
                if acceptor:
                    await acceptor.close()
                with contextlib.suppress(Exception):
                    await session_ws.send(json.dumps({"action": "direct_ok", "session_id": session_id}))
                    await session_ws.close()
                await self.connection_handler(direct_conn)
                return

            # Direct P2P failed or disabled — fall back to Relay
            if acceptor:
                await acceptor.close()

            await session_ws.send(json.dumps({
                "action": "ready",
                "mode": "relay",
                "session_id": session_id,
            }))

            # Await relay_active confirmation
            while True:
                active_raw = await session_ws.recv()
                if isinstance(active_raw, str):
                    active_msg = json.loads(active_raw)
                    if active_msg.get("status") == "relay_active":
                        break

            log.info("Session %s established via Relay", session_id[:8])
            relay_conn = RelayConnection(session_ws)
            await self.connection_handler(relay_conn)
        except Exception as exc:
            log.warning("Incoming session %s failed: %s", session_id[:8], exc)
            if acceptor:
                with contextlib.suppress(Exception):
                    await acceptor.close()
            if session_ws:
                with contextlib.suppress(Exception):
                    await session_ws.close()


# ---------------------------------------------------------------------------
# Rendezvous Client (Controller Side)
# ---------------------------------------------------------------------------


class RendezvousClient:
    """Controller-side client for initiating internet connections through Rendezvous."""

    def __init__(
        self,
        rendezvous_url: str,
        *,
        token: Optional[str] = None,
        enable_p2p: bool = True,
    ) -> None:
        self.rendezvous_url = rendezvous_url.rstrip("/")
        self.token = token
        self.enable_p2p = enable_p2p

    async def list_peers(self, timeout: float = 5.0) -> list[DiscoveredPeer]:
        """Query rendezvous server for registered online peers."""
        ws = await ws_connect(self.rendezvous_url, max_size=None, compression=None)
        try:
            await ws.send(json.dumps({
                "action": "list",
                "token": self.token,
            }))
            resp_raw = await asyncio.wait_for(ws.recv(), timeout=timeout)
            if not isinstance(resp_raw, str):
                return []
            data = json.loads(resp_raw)
            peers = []
            for p in data.get("peers", []):
                pub = bytes.fromhex(p["public_key"])
                fp = ":".join(p["public_key"][i : i + 4] for i in range(0, 16, 4))
                host = self.rendezvous_url
                port = 0
                peers.append(DiscoveredPeer(
                    name=p.get("name") or f"peer-{p['public_key'][:6]}",
                    host=host,
                    port=port,
                    public_key=pub,
                    fingerprint=fp,
                    description=p.get("description", ""),
                ))
            return peers
        finally:
            with contextlib.suppress(Exception):
                await ws.close()

    async def connect_to_peer(
        self,
        target_public_key: bytes,
        *,
        timeout: float = 10.0,
    ) -> Connection:
        """Establish a Connection to target_public_key via Direct P2P or Relay fallback."""
        target_hex = target_public_key.hex()
        session_id = uuid.uuid4().hex
        acceptor: Optional[P2PAcceptor] = None

        ws = await ws_connect(self.rendezvous_url, max_size=None, compression=None)
        try:
            candidates: list[dict[str, Any]] = []
            if self.enable_p2p:
                acceptor = P2PAcceptor(session_id)
                port = await acceptor.start()
                candidates = gather_candidates(port)

            # Send connect request
            await ws.send(json.dumps({
                "action": "connect",
                "target": target_hex,
                "session_id": session_id,
                "token": self.token,
                "candidates": candidates,
            }))

            # Await join response with agent candidates
            resp_raw = await asyncio.wait_for(ws.recv(), timeout=timeout)
            if not isinstance(resp_raw, str):
                raise RuntimeError("unexpected response from rendezvous server")
            resp = json.loads(resp_raw)
            if resp.get("status") != "joined":
                error_code = resp.get("error", "unknown_error")
                error_msg = resp.get("message", error_code)
                raise ConnectionError(f"Rendezvous connection failed: {error_msg}")

            agent_candidates = resp.get("agent_candidates", [])

            # Direct P2P Attempt
            direct_conn: Optional[Connection] = None
            if self.enable_p2p and acceptor is not None:
                # 1. As the caller / controller, first dial agent candidates
                direct_sock = await try_connect_candidates(agent_candidates, session_id, timeout=1.2)
                if direct_sock is not None:
                    direct_conn = DirectSocketConnection(direct_sock[0], direct_sock[1])
                elif not acceptor.conn_future.done():
                    # 2. If dial failed, wait briefly to see if agent reverse-dials our acceptor
                    try:
                        reader, writer = await asyncio.wait_for(
                            asyncio.shield(acceptor.conn_future),
                            timeout=1.5,
                        )
                        direct_conn = DirectSocketConnection(reader, writer)
                    except asyncio.TimeoutError:
                        pass

            if direct_conn is not None:
                log.info("Connected to %s via Direct P2P", target_hex[:8])
                if acceptor:
                    await acceptor.close()
                with contextlib.suppress(Exception):
                    await ws.send(json.dumps({"action": "direct_ok", "session_id": session_id}))
                    await ws.close()
                return direct_conn

            # Fallback to Relay
            if acceptor:
                await acceptor.close()

            await ws.send(json.dumps({
                "action": "ready",
                "mode": "relay",
                "session_id": session_id,
            }))

            # Await relay_active confirmation
            while True:
                active_raw = await asyncio.wait_for(ws.recv(), timeout=timeout)
                if isinstance(active_raw, str):
                    active_msg = json.loads(active_raw)
                    if active_msg.get("status") == "relay_active":
                        break

            log.info("Connected to %s via Relay", target_hex[:8])
            return RelayConnection(ws)
        except BaseException:
            if acceptor:
                with contextlib.suppress(Exception):
                    await acceptor.close()
            with contextlib.suppress(Exception):
                await ws.close()
            raise
