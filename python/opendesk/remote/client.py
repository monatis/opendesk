"""Controller-side helpers — connect to a paired opendesk peer.

Resolves a peer reference (name, :class:`DiscoveredPeer`, or explicit URL)
into a :class:`RemoteComputer` ready to drive.  Optionally runs pairing if
the peer isn't trusted yet.

By default :func:`connect` enables **auto-reconnect** — the returned
RemoteComputer keeps a connector closure and rebuilds the session if the
underlying WebSocket drops mid-flow.  Pass ``auto_reconnect=False`` to opt
out.
"""

from __future__ import annotations

import asyncio
from dataclasses import dataclass
from pathlib import Path
from typing import Optional, Union

from opendesk.computer.remote import RemoteComputer
from opendesk.computer.types import CapabilityManifest
from opendesk.protocol import Peer
from opendesk.protocol.auth import (
    AuthFailure,
    Identity,
    Session,
    TrustedPeers,
    auth_client,
    pair_client,
)
from opendesk.protocol.transports.websocket import (
    WebSocketConnection,
    connect_websocket,
)
from opendesk.remote.discovery import DiscoveredPeer


Target = Union[str, DiscoveredPeer]


@dataclass
class ResolvedTarget:
    is_rendezvous: bool
    expected_pubkey: bytes
    host: str = ""
    port: int = 0
    rendezvous_url: str = ""

    def __iter__(self):
        return iter((self.host, self.port, self.expected_pubkey))


async def connect(
    target: Optional[Target] = None,
    *,
    home: Optional[Path] = None,
    rendezvous: Optional[str] = None,
    rendezvous_token: Optional[str] = None,
    enable_p2p: bool = True,
    timeout: float = 5.0,
    auto_reconnect: bool = True,
    reconnect_budget: float = 30.0,
) -> RemoteComputer:
    """Open a :class:`RemoteComputer` to a paired peer.

    *target* is one of:

    * a :class:`DiscoveredPeer` from :func:`discover` — host, port, and
      expected public key are taken from it.
    * a peer ``name`` previously stored via pairing — looked up in trusted
      peers; the LAN is browsed to find its current address or its cached
      rendezvous URL is used.
    * a URL like ``"ws://192.168.1.42:8423#<pubkey-hex>"`` or
      ``"relay://rendezvous.host:8424#<pubkey-hex>"``
    * ``None`` — falls back to the persistent default peer
      (``opendesk peers default <name>``).

    When ``rendezvous`` is provided or the peer has a stored rendezvous URL,
    the connection is negotiated across the Internet via Direct P2P or Relay
    fallback.

    When ``auto_reconnect`` is true (default), the returned RemoteComputer
    re-establishes its session on transient drops with exponential backoff.
    """
    if target is None:
        default = TrustedPeers(home).get_default()
        if default is None:
            raise ValueError(
                "no peer specified and no default-peer set "
                "(run `opendesk peers default <name>`)"
            )
        target = default
    identity = Identity.load_or_create(home)

    async def _open_session() -> tuple[Peer, CapabilityManifest]:
        resolved = await _resolve(target, home=home, timeout=timeout, rendezvous=rendezvous)
        if resolved.is_rendezvous:
            from opendesk.remote.rendezvous import RendezvousClient
            client = RendezvousClient(
                resolved.rendezvous_url,
                token=rendezvous_token,
                enable_p2p=enable_p2p,
            )
            raw = await client.connect_to_peer(resolved.expected_pubkey, timeout=timeout)
        else:
            raw = await connect_websocket(f"ws://{resolved.host}:{resolved.port}")

        try:
            session = await auth_client(raw, identity, resolved.expected_pubkey)
        except BaseException:
            await raw.aclose()
            raise
        peer = Peer(session.connection, role="client")
        try:
            hello = await peer.hello({})
        except BaseException:
            await peer.aclose()
            raise
        try:
            manifest = CapabilityManifest.model_validate(hello.capabilities)
        except Exception:
            manifest = CapabilityManifest()
        peer.start()

        # Cache description and endpoint / rendezvous
        store = TrustedPeers(home)
        if manifest.description:
            store.cache_description(resolved.expected_pubkey, manifest.description)
        if resolved.is_rendezvous:
            store.cache_rendezvous(resolved.expected_pubkey, resolved.rendezvous_url)
        else:
            store.cache_endpoint(resolved.expected_pubkey, resolved.host, int(resolved.port))
        return peer, manifest

    if auto_reconnect:
        return await RemoteComputer.connect_with_reconnect(
            _open_session, reconnect_budget=reconnect_budget,
        )
    peer, manifest = await _open_session()
    return RemoteComputer(peer, manifest)


async def pair_with(
    host: Optional[str] = None,
    port: Optional[int] = None,
    code: str = "",
    *,
    rendezvous: Optional[str] = None,
    target_pubkey: Optional[Union[str, bytes]] = None,
    rendezvous_token: Optional[str] = None,
    enable_p2p: bool = True,
    home: Optional[Path] = None,
    name: str = "",
) -> tuple[RemoteComputer, bytes]:
    """Pair with a peer at ``host:port`` or via ``rendezvous`` using ``code``.

    Returns the resulting :class:`RemoteComputer` plus the now-trusted server
    public key.
    """
    identity = Identity.load_or_create(home)
    trusted = TrustedPeers(home)

    if rendezvous is not None:
        if target_pubkey is None:
            raise ValueError("target_pubkey is required when pairing via rendezvous")
        pk_bytes = bytes.fromhex(target_pubkey) if isinstance(target_pubkey, str) else target_pubkey
        from opendesk.remote.rendezvous import RendezvousClient
        client = RendezvousClient(rendezvous, token=rendezvous_token, enable_p2p=enable_p2p)
        raw = await client.connect_to_peer(pk_bytes)
    else:
        if not host or not port:
            raise ValueError("host and port are required for direct pairing")
        raw = await connect_websocket(f"ws://{host}:{port}")

    try:
        session = await pair_client(raw, identity, code)
    except BaseException:
        await raw.aclose()
        raise

    server_pubkey = session.peer_public
    peer_name = name or _default_peer_name(server_pubkey)
    if rendezvous is not None:
        trusted.add(server_pubkey, name=peer_name, rendezvous_url=rendezvous)
    else:
        trusted.add(server_pubkey, name=peer_name)
        trusted.cache_endpoint(server_pubkey, host, int(port))

    remote = await RemoteComputer.connect(session.connection)
    return remote, server_pubkey


# ---------------------------------------------------------------------------
# Resolution
# ---------------------------------------------------------------------------


async def _resolve(
    target: Target,
    *,
    home: Optional[Path],
    timeout: float,
    rendezvous: Optional[str] = None,
) -> ResolvedTarget:
    """Translate ``target`` into a :class:`ResolvedTarget`."""
    if isinstance(target, DiscoveredPeer):
        if target.host.startswith("ws://") or target.host.startswith("wss://"):
            return ResolvedTarget(
                is_rendezvous=True,
                expected_pubkey=target.public_key,
                rendezvous_url=target.host,
            )
        return ResolvedTarget(
            is_rendezvous=False,
            expected_pubkey=target.public_key,
            host=target.host,
            port=target.port,
        )

    if not isinstance(target, str):
        raise TypeError(f"unsupported target type: {type(target).__name__}")

    # Explicit relay / rendezvous URL schemes
    if target.startswith("relay://") or target.startswith("rendezvous://"):
        _, _, rest = target.partition("://")
        endpoint, _, frag = rest.partition("#")
        if not frag:
            host_part, _, path_part = endpoint.partition("/")
            if path_part:
                endpoint = host_part
                frag = path_part
        if not frag:
            raise ValueError(f"relay URL target requires '#<pubkey-hex>': got {target!r}")
        try:
            pubkey = bytes.fromhex(frag)
        except ValueError as exc:
            raise ValueError("invalid pubkey hex in URL fragment") from exc
        r_url = f"ws://{endpoint}"
        return ResolvedTarget(
            is_rendezvous=True,
            expected_pubkey=pubkey,
            rendezvous_url=r_url,
        )

    # Explicit WebSocket URL
    if target.startswith("ws://") or target.startswith("wss://"):
        url, _, frag = target.partition("#")
        if not frag:
            raise ValueError(
                f"explicit URL target requires '#<pubkey-hex>': got {target!r}"
            )
        try:
            pubkey = bytes.fromhex(frag)
        except ValueError as exc:
            raise ValueError("invalid pubkey hex in URL fragment") from exc
        if rendezvous:
            return ResolvedTarget(
                is_rendezvous=True,
                expected_pubkey=pubkey,
                rendezvous_url=rendezvous,
            )
        scheme_split = url.partition("://")
        host_port = scheme_split[2]
        host, _, port_s = host_port.partition(":")
        port = int(port_s) if port_s else 80
        return ResolvedTarget(
            is_rendezvous=False,
            expected_pubkey=pubkey,
            host=host,
            port=port,
        )

    # 64-character hex public key directly with rendezvous
    if len(target) == 64 and all(c in "0123456789abcdefABCDEF" for c in target) and rendezvous:
        pubkey = bytes.fromhex(target)
        return ResolvedTarget(
            is_rendezvous=True,
            expected_pubkey=pubkey,
            rendezvous_url=rendezvous,
        )

    # Peer name — must be in trusted-peers.
    trusted = TrustedPeers(home)
    peer = trusted.find_by_name(target)
    if peer is None:
        raise ValueError(
            f"unknown peer {target!r}; run `opendesk pair {target}` first "
            "or pass an explicit URL"
        )
    pubkey = peer.public_bytes

    if rendezvous:
        return ResolvedTarget(
            is_rendezvous=True,
            expected_pubkey=pubkey,
            rendezvous_url=rendezvous,
        )

    if peer.rendezvous_url:
        return ResolvedTarget(
            is_rendezvous=True,
            expected_pubkey=pubkey,
            rendezvous_url=peer.rendezvous_url,
        )

    if peer.last_host and peer.last_port:
        return ResolvedTarget(
            is_rendezvous=False,
            expected_pubkey=pubkey,
            host=peer.last_host,
            port=peer.last_port,
        )

    # Fall back to mDNS browse.
    from opendesk.remote.discovery import discover
    peers = await discover(timeout=timeout)
    for p in peers:
        if p.public_key == pubkey:
            return ResolvedTarget(
                is_rendezvous=False,
                expected_pubkey=pubkey,
                host=p.host,
                port=p.port,
            )
    raise RuntimeError(
        f"peer {target!r} is paired but has no cached address and could "
        f"not be located on the LAN within {timeout:.1f}s.  Either run "
        f"`opendesk pair-with <host-ip> <code>` to give it an address, or "
        f"connect via rendezvous with `--rendezvous <url>`."
    )


def _default_peer_name(public_key: bytes) -> str:
    return f"peer-{public_key.hex()[:6]}"
