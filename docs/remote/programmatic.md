# Programmatic Use

## Python

```python
import asyncio
from opendesk.remote import connect

async def main():
    remote = await connect("mini")        # peer name from `opendesk peers list`
    try:
        pixmap = await remote.capture()
        print(pixmap.width, pixmap.height, len(pixmap.data))
    finally:
        await remote.aclose()

asyncio.run(main())
```

`remote` is a full `Computer` — drop it into any existing opendesk
`ToolContext` and every tool transparently targets the remote machine.

## Over the Internet (Rendezvous & Direct P2P)

```python
import asyncio
from opendesk.remote import connect

async def main():
    # Connects over the Internet using stored rendezvous URL or explicit URL
    remote = await connect(
        "cloud-vm",
        rendezvous="ws://rendezvous.example.com:8424",
        rendezvous_token="secret",
        enable_p2p=True,  # tries direct P2P first, falls back to relay
    )
    try:
        pix = await remote.capture()
        print(f"Captured screen: {pix.width}x{pix.height}")
    finally:
        await remote.aclose()

asyncio.run(main())
```

## Running an Outbound Server (Python)

```python
import asyncio
from pathlib import Path
from opendesk.computer.local import LocalComputer
from opendesk.protocol.auth import Identity, TrustedPeers
from opendesk.remote.server import OpendeskServer

async def run_server():
    computer = LocalComputer()
    home = Path.home() / ".opendesk"
    identity = Identity.load_or_create(home)
    trusted = TrustedPeers(home)

    server = OpendeskServer(
        computer,
        identity,
        trusted,
        home=home,
        listen=False,  # Zero inbound open ports
        rendezvous="ws://rendezvous.example.com:8424",
        rendezvous_token="secret",
    )
    await server.start()
    await server.serve_forever()

asyncio.run(run_server())
```

---

Running into issues? See [Troubleshooting →](troubleshooting.md)
