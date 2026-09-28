"""opendesk.remote — the user-facing API for controlling another machine.

Stitches together everything below it:

* :mod:`opendesk.computer` (the Computer ABC + RemoteComputer)
* :mod:`opendesk.protocol` (frames, codec, peer, transports)
* :mod:`opendesk.protocol.auth` (identity, trusted peers, pairing, encryption)

Public API
----------
* :class:`OpendeskServer` / :func:`serve` — run on the controlled machine.
* :func:`connect` — open a :class:`RemoteComputer` to a known peer.
* :func:`discover` — list opendesk peers on the LAN via mDNS.
"""

from opendesk.remote.audit import AuditLog
from opendesk.remote.client import connect
from opendesk.remote.discovery import (
    DiscoveredPeer,
    advertise,
    discover,
)
from opendesk.remote.policy import (
    AllowAllPolicy,
    ConsolePolicy,
    OBSERVATION_METHODS,
    Policy,
)
from opendesk.remote.rendezvous import (
    DirectSocketConnection,
    RelayConnection,
    RendezvousAgent,
    RendezvousClient,
    RendezvousServer,
)
from opendesk.remote.server import (
    OpendeskServer,
    ServerMode,
    SessionInfo,
    serve,
)

__all__ = [
    "OpendeskServer",
    "ServerMode",
    "SessionInfo",
    "serve",
    "connect",
    "discover",
    "advertise",
    "DiscoveredPeer",
    "Policy",
    "AllowAllPolicy",
    "ConsolePolicy",
    "OBSERVATION_METHODS",
    "AuditLog",
    "RendezvousServer",
    "RendezvousAgent",
    "RendezvousClient",
    "RelayConnection",
    "DirectSocketConnection",
]
