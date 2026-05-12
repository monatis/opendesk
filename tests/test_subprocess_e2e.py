"""Cross-process end-to-end tests.

Spawns ``python -m opendesk.cli serve`` as a real subprocess on
``127.0.0.1`` with pre-populated identity + trusted-peers, then drives it
from the test process via :func:`opendesk.remote.client.connect`.  This
exercises the actual user-facing binary across a process boundary, over real
TCP, with real msgpack framing and AEAD encryption.

Only Computer operations that don't need a display are tested — capabilities,
environment, ``shell``, and filesystem.  Display / input / UI methods need an
X session or accessibility permissions that aren't guaranteed in CI.

These tests are slower than the loopback ones (~1 s of process startup per
test) and live in a dedicated file so fast-feedback runs can ``-x
--ignore=tests/test_subprocess_e2e.py``.
"""

from __future__ import annotations

import asyncio
import os
import re
import signal
import subprocess
import sys
import time
from contextlib import asynccontextmanager
from pathlib import Path
from typing import AsyncIterator

import pytest

from opendesk.protocol.auth import Identity, TrustedPeers
from opendesk.remote.client import connect


_OPENDESK_PORT_RE = re.compile(r"listening on [^:]+:(\d+)")
_STARTUP_TIMEOUT = 15.0


# ---------------------------------------------------------------------------
# Subprocess helpers
# ---------------------------------------------------------------------------


async def _spawn_serve(
    *,
    home: Path,
    host: str = "127.0.0.1",
) -> tuple[subprocess.Popen, int]:
    """Spawn ``opendesk.cli serve`` and wait for it to be listening.

    Returns the subprocess handle and the actual bound port.
    """
    env = os.environ.copy()
    env["PYTHONUNBUFFERED"] = "1"
    proc = subprocess.Popen(
        [
            sys.executable, "-m", "opendesk.cli", "serve",
            "--port", "0",
            "--host", host,
            "--home", str(home),
            "--no-mdns",
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
        env=env,
    )

    deadline = time.monotonic() + _STARTUP_TIMEOUT
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            # Drain stdout for the failure message.
            out = proc.stdout.read() if proc.stdout else ""
            raise RuntimeError(
                f"`opendesk serve` exited early (rc={proc.returncode}):\n{out}"
            )

        # Non-blocking poll for output via select would be portable; here we
        # just sleep briefly and try to read the next line.
        await asyncio.sleep(0.05)
        line = await asyncio.to_thread(_readline_nowait, proc)
        if not line:
            continue
        m = _OPENDESK_PORT_RE.search(line)
        if m:
            return proc, int(m.group(1))

    proc.terminate()
    raise TimeoutError(
        f"`opendesk serve` didn't print a listening line within {_STARTUP_TIMEOUT}s",
    )


def _readline_nowait(proc: subprocess.Popen) -> str:
    """Read one line of stdout with a short blocking timeout via the thread pool."""
    if proc.stdout is None:
        return ""
    return proc.stdout.readline()


async def _stop_proc(proc: subprocess.Popen) -> None:
    """Terminate the subprocess cleanly, falling back to SIGKILL."""
    if proc.poll() is not None:
        return
    proc.terminate()
    try:
        await asyncio.to_thread(proc.wait, 5.0)
    except subprocess.TimeoutExpired:
        proc.kill()
        await asyncio.to_thread(proc.wait, 5.0)


# ---------------------------------------------------------------------------
# Fixture-style: spawn server, pre-trust client, yield port + pubkey
# ---------------------------------------------------------------------------


@asynccontextmanager
async def _running_server(
    tmp_path: Path,
) -> AsyncIterator[tuple[int, bytes, Path]]:
    """Yield ``(port, server_pubkey, client_home)`` for a live server subprocess.

    Pre-populates trust on both sides so the test can connect immediately
    without going through pairing.
    """
    server_home = tmp_path / "server"
    client_home = tmp_path / "client"
    server_home.mkdir()
    client_home.mkdir()

    # Generate identities up-front so we know the public keys before the
    # subprocess starts.
    server_identity = Identity.load_or_create(server_home)
    client_identity = Identity.load_or_create(client_home)

    # Mutual trust.
    TrustedPeers(server_home).add(client_identity.public_bytes, name="controller")
    TrustedPeers(client_home).add(server_identity.public_bytes, name="host")

    proc, port = await _spawn_serve(home=server_home)
    try:
        yield port, server_identity.public_bytes, client_home
    finally:
        await _stop_proc(proc)


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


class TestSubprocessE2E:
    @pytest.mark.asyncio
    async def test_capabilities_round_trip(self, tmp_path: Path):
        async with _running_server(tmp_path) as (port, server_pub, client_home):
            remote = await connect(
                f"ws://127.0.0.1:{port}#{server_pub.hex()}",
                home=client_home,
            )
            try:
                caps = remote.capabilities()
                assert caps.backend.startswith("local/")
                # At minimum a real LocalComputer advertises these.
                assert any(c.value == "display.capture" for c in caps.capabilities)
                assert any(c.value == "process.shell" for c in caps.capabilities)
            finally:
                await remote.aclose()

    @pytest.mark.asyncio
    async def test_environment_round_trip(self, tmp_path: Path):
        async with _running_server(tmp_path) as (port, server_pub, client_home):
            remote = await connect(
                f"ws://127.0.0.1:{port}#{server_pub.hex()}",
                home=client_home,
            )
            try:
                env = await remote.environment()
                # The subprocess runs on the same machine we're testing from,
                # so OS / hostname match.
                import platform
                assert env.os == platform.system()
                assert env.hostname == platform.node()
            finally:
                await remote.aclose()

    @pytest.mark.asyncio
    async def test_shell_command_through_subprocess(self, tmp_path: Path):
        """Exercises bytes-in-result over a real TCP socket between processes."""
        async with _running_server(tmp_path) as (port, server_pub, client_home):
            remote = await connect(
                f"ws://127.0.0.1:{port}#{server_pub.hex()}",
                home=client_home,
            )
            try:
                result = await remote.shell("echo opendesk-cross-process")
                assert result.returncode == 0
                assert b"opendesk-cross-process" in result.stdout
            finally:
                await remote.aclose()

    @pytest.mark.asyncio
    async def test_binary_fs_round_trip_through_subprocess(self, tmp_path: Path):
        """Write random bytes through the wire to disk on the remote side, then
        read them back — the canonical 'no base64 ever' end-to-end check."""
        async with _running_server(tmp_path) as (port, server_pub, client_home):
            remote = await connect(
                f"ws://127.0.0.1:{port}#{server_pub.hex()}",
                home=client_home,
            )
            try:
                payload = os.urandom(4096) + b"\x00" * 100 + b"end"
                target = tmp_path / "round-trip.bin"
                await remote.write_file(str(target), payload)
                read_back = await remote.read_file(str(target))
                assert read_back == payload
            finally:
                await remote.aclose()
