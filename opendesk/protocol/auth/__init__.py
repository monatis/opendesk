"""Auth, pairing, and encryption for the opendesk wire protocol.

This layer sits between a raw :class:`~opendesk.protocol.connection.Connection`
(typically a :class:`~opendesk.protocol.transports.websocket.WebSocketConnection`)
and a :class:`~opendesk.protocol.peer.Peer`.  It is responsible for:

* Long-lived peer identity (X25519 keypair on disk).
* The trusted-peers store (which keys we recognise on incoming connections).
* The handshake — PSK-authenticated for pairing, static-key for reconnect.
* AEAD encryption of every protocol frame after the handshake.

Flow::

    # Server, paired once before, then ``opendesk serve``
    identity = Identity.load_or_create(home_dir)
    trusted = TrustedPeers.load(home_dir)
    raw_conn = await accept_websocket()
    session = await auth_server(raw_conn, identity, trusted)
    peer = Peer(session.connection, role="server", dispatcher=...)
    ...

    # Client
    identity = Identity.load_or_create(home_dir)
    raw_conn = await connect_websocket(url)
    session = await auth_client(raw_conn, identity, expected_server_pubkey=...)
    remote = await RemoteComputer.connect(session.connection)

The handshake design and threat model are documented in :mod:`.handshake`.
"""

from opendesk.protocol.auth.encrypted import EncryptedConnection
from opendesk.protocol.auth.handshake import (
    AuthFailure,
    Session,
    auth_client,
    auth_server,
    pair_client,
    pair_server,
)
from opendesk.protocol.auth.identity import Identity
from opendesk.protocol.auth.storage import TrustedPeer, TrustedPeers

__all__ = [
    "Identity",
    "TrustedPeer",
    "TrustedPeers",
    "Session",
    "AuthFailure",
    "EncryptedConnection",
    "pair_server",
    "pair_client",
    "auth_server",
    "auth_client",
]
