"""On-disk store of peers we trust to connect to (or be connected to by).

Single JSON file at ``<home>/trusted-peers.json``::

    [
      {
        "public_key": "<hex 64 chars>",
        "name": "fariz-laptop",
        "paired_at": "2026-05-12T10:30:00Z",
        "fingerprint": "abcd:ef01:..."
      },
      ...
    ]

The file is loaded fresh per query so that pairing from another terminal mid-
session takes effect without restarting :command:`opendesk serve`.
"""

from __future__ import annotations

import json
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Optional

from opendesk.protocol.auth.identity import DEFAULT_HOME


TRUSTED_PEERS_FILE = "trusted-peers.json"


def fingerprint(public_key: bytes) -> str:
    """A short human-readable fingerprint for a public key.

    Eight colon-separated groups of two hex digits — enough to compare
    visually without having to read the full 32-byte key.
    """
    h = public_key.hex()
    return ":".join(h[i : i + 4] for i in range(0, 16, 4))


@dataclass
class TrustedPeer:
    """One trusted peer entry."""

    public_key: str  # hex
    name: str = ""
    paired_at: float = field(default_factory=time.time)

    @property
    def public_bytes(self) -> bytes:
        return bytes.fromhex(self.public_key)

    @property
    def fingerprint(self) -> str:
        return fingerprint(self.public_bytes)


class TrustedPeers:
    """Read/write access to the trusted-peers JSON file."""

    def __init__(self, home: Optional[Path] = None) -> None:
        self._home = Path(home) if home else DEFAULT_HOME
        self._path = self._home / TRUSTED_PEERS_FILE

    @property
    def path(self) -> Path:
        return self._path

    def _load(self) -> list[TrustedPeer]:
        if not self._path.exists():
            return []
        try:
            raw = json.loads(self._path.read_text())
        except (OSError, json.JSONDecodeError):
            return []
        out: list[TrustedPeer] = []
        for item in raw:
            try:
                out.append(TrustedPeer(
                    public_key=item["public_key"],
                    name=item.get("name", ""),
                    paired_at=float(item.get("paired_at") or 0.0),
                ))
            except (KeyError, ValueError, TypeError):
                continue
        return out

    def _save(self, peers: list[TrustedPeer]) -> None:
        self._home.mkdir(parents=True, exist_ok=True, mode=0o700)
        self._path.write_text(json.dumps(
            [asdict(p) for p in peers],
            indent=2, sort_keys=True,
        ))

    def list(self) -> list[TrustedPeer]:
        return self._load()

    def contains(self, public_key: bytes) -> bool:
        hex_key = public_key.hex()
        return any(p.public_key == hex_key for p in self._load())

    def find(self, public_key: bytes) -> Optional[TrustedPeer]:
        hex_key = public_key.hex()
        for p in self._load():
            if p.public_key == hex_key:
                return p
        return None

    def find_by_name(self, name: str) -> Optional[TrustedPeer]:
        for p in self._load():
            if p.name == name:
                return p
        return None

    def add(self, public_key: bytes, *, name: str = "") -> TrustedPeer:
        """Add or update a trusted peer.  Returns the stored entry."""
        peers = self._load()
        hex_key = public_key.hex()
        for i, p in enumerate(peers):
            if p.public_key == hex_key:
                if name and p.name != name:
                    peers[i] = TrustedPeer(public_key=hex_key, name=name, paired_at=p.paired_at)
                    self._save(peers)
                return peers[i]
        peer = TrustedPeer(public_key=hex_key, name=name)
        peers.append(peer)
        self._save(peers)
        return peer

    def remove(self, public_key_or_name: str) -> bool:
        """Remove by hex key or by friendly name.  Returns True on hit."""
        peers = self._load()
        before = len(peers)
        peers = [
            p for p in peers
            if p.public_key != public_key_or_name and p.name != public_key_or_name
        ]
        if len(peers) == before:
            return False
        self._save(peers)
        return True

    def rename(self, public_key_or_name: str, new_name: str) -> bool:
        peers = self._load()
        for i, p in enumerate(peers):
            if p.public_key == public_key_or_name or p.name == public_key_or_name:
                peers[i] = TrustedPeer(public_key=p.public_key, name=new_name, paired_at=p.paired_at)
                self._save(peers)
                return True
        return False
