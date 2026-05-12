"""LAN discovery via mDNS / Zeroconf / Bonjour.

Service type: ``_opendesk._tcp.local.``

TXT-record properties:
* ``pk`` — the host's 32-byte X25519 long-lived static public key (binary)
* ``v``  — protocol version (currently ``"1"``)
* ``fp`` — short colon-separated fingerprint for human display

This module is optional: it raises :class:`ImportError` from ``advertise`` /
``discover`` if ``zeroconf`` is not installed.  The :class:`OpendeskServer`
treats that as a soft failure (logs a warning, keeps serving without mDNS).
"""

from __future__ import annotations

import asyncio
import contextlib
import socket
from dataclasses import dataclass
from typing import Optional


SERVICE_TYPE = "_opendesk._tcp.local."


def _require_zeroconf():
    try:
        import zeroconf
        from zeroconf.asyncio import AsyncServiceInfo, AsyncZeroconf
        return zeroconf, AsyncServiceInfo, AsyncZeroconf
    except ImportError as exc:
        raise ImportError(
            "zeroconf is required for opendesk LAN discovery. "
            "Install with: pip install 'opendesk[remote]' "
            "(or directly: pip install zeroconf)"
        ) from exc


@dataclass
class DiscoveredPeer:
    """One peer advertising itself on the LAN.

    ``description`` carries whatever short label the controlled machine
    broadcasts via the ``desc`` TXT record — typically a truncated copy of
    its full ``opendesk describe …`` text.  Empty if the peer hasn't set
    one.
    """

    name: str
    host: str
    port: int
    public_key: bytes
    fingerprint: str = ""
    description: str = ""

    @property
    def url(self) -> str:
        return f"ws://{self.host}:{self.port}"


# ---------------------------------------------------------------------------
# Advertise (controlled-machine side)
# ---------------------------------------------------------------------------


class Advertisement:
    """A live mDNS advertisement.  Call :meth:`aclose` to unregister."""

    def __init__(self, azc, info) -> None:
        self._azc = azc
        self._info = info
        self._closed = False

    async def aclose(self) -> None:
        if self._closed:
            return
        self._closed = True
        with contextlib.suppress(Exception):
            await self._azc.async_unregister_service(self._info)
        with contextlib.suppress(Exception):
            await self._azc.async_close()


_DESC_TXT_MAX = 120  # bytes; mDNS TXT records have a hard ~255-byte cap per entry


async def advertise(
    *,
    name: str,
    port: int,
    public_key: bytes,
    host_ips: Optional[list[str]] = None,
    description: str = "",
) -> Advertisement:
    """Register an opendesk service on the LAN via mDNS.

    Returns an :class:`Advertisement` whose ``aclose()`` retracts the record.

    The ``description`` is published as a ``desc`` TXT record, truncated to
    :data:`_DESC_TXT_MAX` bytes (UTF-8) so it stays within mDNS limits.  The
    full description still flows over the HELLO frame after pairing — the
    TXT record's job is to surface a short tag at discovery time.
    """
    _, AsyncServiceInfo, AsyncZeroconf = _require_zeroconf()

    if host_ips is None:
        host_ips = _local_ipv4s()
    addresses = [socket.inet_aton(ip) for ip in host_ips]

    fp = ":".join(public_key.hex()[i : i + 4] for i in range(0, 16, 4))
    properties = {b"v": b"1", b"pk": public_key, b"fp": fp.encode("ascii")}
    if description:
        encoded = description.encode("utf-8")[:_DESC_TXT_MAX]
        # Avoid leaving a half-character at the boundary.
        try:
            encoded = encoded.decode("utf-8").encode("utf-8")
        except UnicodeDecodeError:
            encoded = encoded[:-1].decode("utf-8", errors="ignore").encode("utf-8")
        properties[b"desc"] = encoded

    azc = AsyncZeroconf()
    info = AsyncServiceInfo(
        type_=SERVICE_TYPE,
        name=f"{name}.{SERVICE_TYPE}",
        addresses=addresses,
        port=port,
        properties=properties,
        server=f"{name}.local.",
    )
    await azc.async_register_service(info)
    return Advertisement(azc, info)


# ---------------------------------------------------------------------------
# Discover (controller-machine side)
# ---------------------------------------------------------------------------


async def discover(timeout: float = 2.0) -> list[DiscoveredPeer]:
    """Browse the LAN for opendesk peers and return what's found.

    ``timeout`` is how long to wait for responses.  2 s is enough on a quiet
    LAN; bump it on saturated networks.
    """
    _, AsyncServiceInfo, AsyncZeroconf = _require_zeroconf()
    from zeroconf import ServiceListener  # type: ignore

    found: dict[str, DiscoveredPeer] = {}

    class _Listener(ServiceListener):
        def __init__(self, zc):
            self._zc = zc

        def add_service(self, zc, type_, name):  # noqa: D401
            info = zc.get_service_info(type_, name, timeout=1000)
            if info is None:
                return
            try:
                peer = _info_to_peer(info)
            except Exception:
                return
            if peer is not None:
                found[name] = peer

        def update_service(self, zc, type_, name):
            self.add_service(zc, type_, name)

        def remove_service(self, zc, type_, name):
            found.pop(name, None)

    async with AsyncZeroconf() as azc:
        from zeroconf.asyncio import AsyncServiceBrowser
        listener = _Listener(azc.zeroconf)
        browser = AsyncServiceBrowser(azc.zeroconf, SERVICE_TYPE, listener)
        try:
            await asyncio.sleep(timeout)
        finally:
            await browser.async_cancel()

    return list(found.values())


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _info_to_peer(info) -> Optional[DiscoveredPeer]:
    props = info.properties or {}
    pk = props.get(b"pk")
    if not pk or len(pk) != 32:
        return None
    fp = (props.get(b"fp") or b"").decode("ascii", errors="replace")
    desc = (props.get(b"desc") or b"").decode("utf-8", errors="replace")
    # Prefer IPv4 for v1.
    host = None
    for addr in info.parsed_addresses() if hasattr(info, "parsed_addresses") else []:
        if ":" not in addr:
            host = addr
            break
    if host is None and info.addresses:
        host = socket.inet_ntoa(info.addresses[0])
    if host is None:
        return None
    name = info.name
    if name.endswith("." + SERVICE_TYPE):
        name = name[: -len("." + SERVICE_TYPE)]
    return DiscoveredPeer(
        name=name, host=host, port=info.port or 0,
        public_key=bytes(pk), fingerprint=fp, description=desc,
    )


def _local_ipv4s() -> list[str]:
    """Best-effort enumeration of this machine's non-loopback IPv4 addresses."""
    addrs: list[str] = []
    try:
        # Probing trick: open a UDP socket to a public address (no packets sent)
        # to learn which local interface would be used as the source.
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        try:
            s.connect(("8.8.8.8", 80))
            addrs.append(s.getsockname()[0])
        finally:
            s.close()
    except Exception:
        pass

    # Also enumerate via gethostbyname_ex as a fallback.
    try:
        for ip in socket.gethostbyname_ex(socket.gethostname())[2]:
            if not ip.startswith("127."):
                if ip not in addrs:
                    addrs.append(ip)
    except Exception:
        pass

    if not addrs:
        addrs.append("127.0.0.1")
    return addrs
