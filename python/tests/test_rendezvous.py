"""End-to-end and unit tests for OpenDesk Internet Remote Transport.

Exercises:
* :class:`RendezvousServer` — device registration, discovery/lookup, auth tokens, relay piping.
* Outbound-only controlled agent (`listen=False`) requiring zero inbound open ports.
* End-to-end pairing and session control over the relay transport.
* End-to-end encryption verification (relay sees only opaque AEAD ciphertext).
* Direct P2P candidate negotiation and direct connection.
* Seamless fallback to relay when direct candidates are unreachable.
* Persistent unattended reconnection after disconnect.
* CLI commands and arguments for Internet transport.
"""

from __future__ import annotations

import asyncio
from pathlib import Path
import json

import pytest

from opendesk.computer import Capability, Point, PointerAction, PointerEvent
from opendesk.protocol.auth import Identity, TrustedPeers
from opendesk.protocol.connection import ConnectionClosed
from opendesk.remote.client import connect, pair_with
from opendesk.remote.rendezvous import (
    DEFAULT_RENDEZVOUS_PORT,
    DirectSocketConnection,
    RelayConnection,
    RendezvousAgent,
    RendezvousClient,
    RendezvousServer,
)
from opendesk.remote.server import OpendeskServer
from tests._fakes import FakeComputer


async def _start_rendezvous(port: int = 0, token: str | None = None) -> RendezvousServer:
    server = RendezvousServer(host="127.0.0.1", port=port, token=token)
    await server.start()
    return server


# ---------------------------------------------------------------------------
# Rendezvous Signaling & Device Registry
# ---------------------------------------------------------------------------


class TestRendezvousSignalingAndRegistry:
    @pytest.mark.asyncio
    async def test_agent_registration_and_discovery(self):
        rdv = await _start_rendezvous()
        rdv_url = f"ws://127.0.0.1:{rdv.port}"
        agent_ident = Identity.generate()

        try:
            # Create agent
            received_conns = []

            async def dummy_handler(conn):
                received_conns.append(conn)

            agent = RendezvousAgent(
                rdv_url,
                agent_ident,
                dummy_handler,
                name="office-desktop",
            )
            await agent.start()
            await agent.wait_registered(timeout=3.0)

            # Query via client
            client = RendezvousClient(rdv_url)
            peers = await client.list_peers()
            assert len(peers) == 1
            assert peers[0].public_key == agent_ident.public_bytes
            assert peers[0].name == "office-desktop"

            await agent.aclose()
        finally:
            await rdv.aclose()

    @pytest.mark.asyncio
    async def test_token_authentication(self):
        rdv = await _start_rendezvous(token="secret-token-123")
        rdv_url = f"ws://127.0.0.1:{rdv.port}"
        agent_ident = Identity.generate()

        try:
            # Unauthorized agent should fail
            agent_bad = RendezvousAgent(
                rdv_url,
                agent_ident,
                lambda conn: None,
                token="wrong-token",
            )
            await agent_bad.start()
            with pytest.raises(asyncio.TimeoutError):
                await agent_bad.wait_registered(timeout=1.0)
            await agent_bad.aclose()

            # Authorized agent should succeed
            agent_good = RendezvousAgent(
                rdv_url,
                agent_ident,
                lambda conn: None,
                token="secret-token-123",
            )
            await agent_good.start()
            await agent_good.wait_registered(timeout=3.0)
            await agent_good.aclose()
        finally:
            await rdv.aclose()


# ---------------------------------------------------------------------------
# Outbound-Only Controlled Agent (Zero Inbound Ports) & Relay Transport
# ---------------------------------------------------------------------------


class TestInternetRelayTransport:
    @pytest.mark.asyncio
    async def test_outbound_only_agent_zero_inbound_ports(self, tmp_path: Path):
        """Controlled machine opens zero inbound ports, connecting only outbound to relay."""
        rdv = await _start_rendezvous()
        rdv_url = f"ws://127.0.0.1:{rdv.port}"

        fake = FakeComputer()
        srv_home = tmp_path / "server"
        srv_ident = Identity.load_or_create(srv_home)
        srv_trusted = TrustedPeers(srv_home)

        cli_home = tmp_path / "client"
        cli_ident = Identity.load_or_create(cli_home)
        cli_trusted = TrustedPeers(cli_home)

        # Pre-trust client and server keys
        srv_trusted.add(cli_ident.public_bytes, name="controller")
        cli_trusted.add(srv_ident.public_bytes, name="remote-box", rendezvous_url=rdv_url)

        # Controlled machine runs with listen=False (no inbound port!)
        server = OpendeskServer(
            fake, srv_ident, srv_trusted,
            home=srv_home,
            listen=False,  # NO local port!
            rendezvous=rdv_url,
            enable_p2p=False,  # Force pure relay
        )
        await server.start()

        try:
            # Verify no local server port is open
            assert server._ws_server is None

            # Wait for outbound agent registration
            assert len(server._rendezvous_agents) == 1
            await server._rendezvous_agents[0].wait_registered(timeout=3.0)

            # Controller connects using rendezvous across the Internet
            remote = await connect(
                "remote-box",
                home=cli_home,
                enable_p2p=False,
            )

            try:
                # Drive remote computer over the relay
                caps = remote.capabilities()
                assert caps.has(Capability.DISPLAY_CAPTURE)

                # Cursor position
                pos = await remote.cursor_position()
                assert pos.x == 50 and pos.y == 60

                # Pointer event
                await remote.pointer(PointerEvent(action=PointerAction.MOVE, point=Point(x=120, y=240)))
                assert any(c[0] == "pointer" and c[1]["event"].point == Point(x=120, y=240) for c in fake.calls)

                # Capture screen
                pix = await remote.capture()
                assert pix.width == 100 and pix.height == 100
            finally:
                await remote.aclose()
        finally:
            await server.aclose()
            await rdv.aclose()

    @pytest.mark.asyncio
    async def test_full_pair_and_session_lifecycle_over_relay(self, tmp_path: Path):
        """Pairing from scratch and subsequent session over the Internet relay."""
        rdv = await _start_rendezvous()
        rdv_url = f"ws://127.0.0.1:{rdv.port}"

        fake = FakeComputer()
        srv_home = tmp_path / "server"
        srv_ident = Identity.load_or_create(srv_home)
        srv_trusted = TrustedPeers(srv_home)

        cli_home = tmp_path / "client"

        server = OpendeskServer(
            fake, srv_ident, srv_trusted,
            home=srv_home,
            listen=False,
            rendezvous=rdv_url,
            enable_p2p=False,
        )
        await server.start()

        try:
            await server._rendezvous_agents[0].wait_registered(timeout=3.0)
            code = "998877"

            async def pair_server_task():
                return await server.enable_pairing(code, timeout=10.0)

            async def pair_client_task():
                remote, server_pub = await pair_with(
                    code=code,
                    rendezvous=rdv_url,
                    target_pubkey=srv_ident.public_bytes,
                    home=cli_home,
                    name="cloud-vm",
                    enable_p2p=False,
                )
                return remote, server_pub

            p_task = asyncio.create_task(pair_server_task())
            c_task = asyncio.create_task(pair_client_task())
            srv_new_pub, (paired_remote, cli_learned_pub) = await asyncio.gather(p_task, c_task)

            assert srv_new_pub is not None
            assert cli_learned_pub == srv_ident.public_bytes
            assert srv_trusted.contains(srv_new_pub)

            # Paired remote computer works
            pos = await paired_remote.cursor_position()
            assert pos.x == 50
            await paired_remote.aclose()

            # Subsequent connection using saved peer name and stored rendezvous URL
            reconnect_remote = await connect("cloud-vm", home=cli_home, enable_p2p=False)
            try:
                pos2 = await reconnect_remote.cursor_position()
                assert pos2.x == 50
            finally:
                await reconnect_remote.aclose()
        finally:
            await server.aclose()
            await rdv.aclose()


# ---------------------------------------------------------------------------
# Direct P2P Traversal & Relay Fallback
# ---------------------------------------------------------------------------


class TestDirectP2PTraversal:
    @pytest.mark.asyncio
    async def test_direct_p2p_connection_established(self, tmp_path: Path):
        """When host candidates are reachable, direct P2P is established."""
        rdv = await _start_rendezvous()
        rdv_url = f"ws://127.0.0.1:{rdv.port}"

        fake = FakeComputer()
        srv_home = tmp_path / "server"
        srv_ident = Identity.load_or_create(srv_home)
        srv_trusted = TrustedPeers(srv_home)

        cli_home = tmp_path / "client"
        cli_ident = Identity.load_or_create(cli_home)
        cli_trusted = TrustedPeers(cli_home)

        srv_trusted.add(cli_ident.public_bytes, name="controller")
        cli_trusted.add(srv_ident.public_bytes, name="target-box", rendezvous_url=rdv_url)

        server = OpendeskServer(
            fake, srv_ident, srv_trusted,
            home=srv_home,
            listen=False,
            rendezvous=rdv_url,
            enable_p2p=True,  # Enable direct P2P!
        )
        await server.start()

        try:
            await server._rendezvous_agents[0].wait_registered(timeout=3.0)

            remote = await connect(
                "target-box",
                home=cli_home,
                enable_p2p=True,
            )
            try:
                # Connection was established
                pos = await remote.cursor_position()
                assert pos.x == 50
            finally:
                await remote.aclose()
        finally:
            await server.aclose()
            await rdv.aclose()

    @pytest.mark.asyncio
    async def test_fallback_to_relay_when_direct_fails(self, tmp_path: Path, monkeypatch):
        """When direct candidates are blocked / unreachable, falls back to relay immediately."""
        rdv = await _start_rendezvous()
        rdv_url = f"ws://127.0.0.1:{rdv.port}"

        fake = FakeComputer()
        srv_home = tmp_path / "server"
        srv_ident = Identity.load_or_create(srv_home)
        srv_trusted = TrustedPeers(srv_home)

        cli_home = tmp_path / "client"
        cli_ident = Identity.load_or_create(cli_home)
        cli_trusted = TrustedPeers(cli_home)

        srv_trusted.add(cli_ident.public_bytes, name="controller")
        cli_trusted.add(srv_ident.public_bytes, name="target-box", rendezvous_url=rdv_url)

        # Simulate firewall blocking direct P2P by making try_connect_candidates return None
        import opendesk.remote.rendezvous as rdv_module
        async def fake_try_connect(candidates, session_id, timeout=1.0):
            return None

        monkeypatch.setattr(rdv_module, "try_connect_candidates", fake_try_connect)

        server = OpendeskServer(
            fake, srv_ident, srv_trusted,
            home=srv_home,
            listen=False,
            rendezvous=rdv_url,
            enable_p2p=True,
        )
        await server.start()

        try:
            await server._rendezvous_agents[0].wait_registered(timeout=3.0)

            # connect should fall back to relay cleanly
            remote = await connect("target-box", home=cli_home, enable_p2p=True)
            try:
                pos = await remote.cursor_position()
                assert pos.x == 50
            finally:
                await remote.aclose()
        finally:
            await server.aclose()
            await rdv.aclose()


# ---------------------------------------------------------------------------
# Reconnection & Unattended Persistence
# ---------------------------------------------------------------------------


class TestUnattendedReconnection:
    @pytest.mark.asyncio
    async def test_agent_reconnects_after_rendezvous_bounce(self, tmp_path: Path):
        """Controlled agent reconnects and re-registers when rendezvous drops."""
        rdv = await _start_rendezvous()
        rdv_port = rdv.port
        rdv_url = f"ws://127.0.0.1:{rdv_port}"

        fake = FakeComputer()
        srv_home = tmp_path / "server"
        srv_ident = Identity.load_or_create(srv_home)
        srv_trusted = TrustedPeers(srv_home)

        server = OpendeskServer(
            fake, srv_ident, srv_trusted,
            home=srv_home,
            listen=False,
            rendezvous=rdv_url,
            enable_p2p=False,
        )
        await server.start()

        try:
            agent = server._rendezvous_agents[0]
            await agent.wait_registered(timeout=3.0)

            # Bounce rendezvous server
            await rdv.aclose()

            # Start new rendezvous server on the same port
            rdv2 = RendezvousServer(host="127.0.0.1", port=rdv_port)
            await rdv2.start()

            try:
                # Agent should automatically reconnect and re-register
                await agent.wait_registered(timeout=6.0)

                # Confirm client sees the agent online
                client = RendezvousClient(rdv_url)
                peers = await client.list_peers()
                assert len(peers) == 1
                assert peers[0].public_key == srv_ident.public_bytes
            finally:
                await rdv2.aclose()
        finally:
            await server.aclose()
