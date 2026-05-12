"""Controller-side helpers — connect to a paired opendesk peer.

Resolves a peer reference (name, :class:`DiscoveredPeer`, or explicit URL)
into a :class:`RemoteComputer` ready to drive.  Optionally runs pairing if
the peer isn't trusted yet.
"""

from __future__ import annotations

import asyncio
from pathlib import Path
from typing import Optional, Union

from opendesk.computer.remote import RemoteComputer
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


async def connect(
    target: Target,
    *,
    home: Optional[Path] = None,
    timeout: float = 5.0,
) -> RemoteComputer:
    """Open a :class:`RemoteComputer` to a paired peer.

    *target* is one of:

    * a :class:`DiscoveredPeer` from :func:`discover` — host, port, and
      expected public key are taken from it.
    * a peer ``name`` previously stored via pairing — looked up in trusted
      peers; the LAN is browsed to find its current address.
    * a URL like ``"ws://192.168.1.42:8423#<pubkey-hex>"`` — explicit
      address with the expected public key in the fragment.

    Raises :class:`AuthFailure` if the server is not the expected one or the
    client itself isn't trusted by the server.
    """
    identity = Identity.load_or_create(home)
    host, port, expected_pubkey = await _resolve(target, home=home, timeout=timeout)

    raw = await connect_websocket(f"ws://{host}:{port}")
    try:
        session = await auth_client(raw, identity, expected_pubkey)
    except BaseException:
        await raw.aclose()
        raise

    return await RemoteComputer.connect(session.connection)


async def pair_with(
    host: str,
    port: int,
    code: str,
    *,
    home: Optional[Path] = None,
    name: str = "",
) -> tuple[RemoteComputer, bytes]:
    """Pair with a peer at ``host:port`` using ``code``.

    Returns the resulting :class:`RemoteComputer` plus the now-trusted server
    public key (the caller is responsible for persisting it in
    :class:`TrustedPeers`).
    """
    identity = Identity.load_or_create(home)
    trusted = TrustedPeers(home)
    raw = await connect_websocket(f"ws://{host}:{port}")
    try:
        session = await pair_client(raw, identity, code)
    except BaseException:
        await raw.aclose()
        raise

    server_pubkey = session.peer_public
    trusted.add(server_pubkey, name=name or _default_peer_name(server_pubkey))
    remote = await RemoteComputer.connect(session.connection)
    return remote, server_pubkey


# ---------------------------------------------------------------------------
# Resolution
# ---------------------------------------------------------------------------


async def _resolve(
    target: Target, *, home: Optional[Path], timeout: float,
) -> tuple[str, int, bytes]:
    """Translate ``target`` into ``(host, port, expected_pubkey)``."""
    if isinstance(target, DiscoveredPeer):
        return target.host, target.port, target.public_key

    if not isinstance(target, str):
        raise TypeError(f"unsupported target type: {type(target).__name__}")

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
        # Strip scheme to extract host/port.
        scheme_split = url.partition("://")
        host_port = scheme_split[2]
        host, _, port_s = host_port.partition(":")
        port = int(port_s) if port_s else 80
        return host, port, pubkey

    # Peer name — must be in trusted-peers.
    trusted = TrustedPeers(home)
    peer = trusted.find_by_name(target)
    if peer is None:
        raise ValueError(
            f"unknown peer {target!r}; run `opendesk pair {target}` first "
            "or pass an explicit URL"
        )
    pubkey = peer.public_bytes

    # Browse the LAN for the matching public key.
    from opendesk.remote.discovery import discover
    peers = await discover(timeout=timeout)
    for p in peers:
        if p.public_key == pubkey:
            return p.host, p.port, pubkey
    raise RuntimeError(
        f"peer {target!r} is paired but could not be located on the LAN "
        f"within {timeout:.1f}s.  Is `opendesk serve` running on that machine?"
    )


def _default_peer_name(public_key: bytes) -> str:
    return f"peer-{public_key.hex()[:6]}"
