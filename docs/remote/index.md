# Remote Computer Use — CLI

Use an opendesk agent on one machine to control another over the LAN (or the
Internet via a rendezvous relay). The computer-use tools (screenshot, mouse,
keyboard, ui, app, clipboard, opendesk_exec) work the same: on the controller
they go through a `RemoteComputer`, which sends each call over an encrypted
WebSocket to `opendesk serve`, where a `LocalComputer` performs it.

```mermaid
flowchart LR
    A["<b>Controller</b><br/>Claude Code, etc<br/>RemoteComputer"]
    B["<b>Controlled Machine</b><br/>opendesk serve<br/>LocalComputer"]
    A <-->|"ws + msgpack + AEAD · Wi-Fi / Ethernet"| B
```

## Mental model

| Term | Means |
|---|---|
| **Controlled machine** | The one being controlled. Runs `opendesk pair` once, then `opendesk serve` long-running. In protocol terms it's the **server** — it listens. |
| **Controller** | The machine running the agent (Claude Code, Claude Desktop, etc.). Runs `opendesk pair-with` once, then talks to the controlled machine over MCP, the CLI, or the Rust crate. In protocol terms it's the **client** — it initiates. |
| **Pairing** | One-time exchange that establishes mutual trust via a 6-digit code shown on the controlled machine. After pairing, both ends know each other's long-lived public keys and can reconnect without the code. |
| **Trusted-peers store** | `~/.opendesk/trusted-peers.json` on each side — the list of peers that machine has paired with. |

## Guides

- [Setup](setup.md) — one-time pairing
- [Running](running.md) — `opendesk serve` and `opendesk connect`
- [Internet Remote Transport](internet-transport.md) — zero-inbound rendezvous & relay across NATs/firewalls
- [MCP Integration](mcp.md) — peer resolution, admin tools, agent example
- [Security](security.md) — threat model, files on disk
- [CLI Reference](cli.md)
- [Service Install](service.md) — survive reboots
- [Concurrency](concurrency.md) — single-controller policy, disconnect vs unpair
- [Programmatic Use](programmatic.md) — Rust crate, or MCP from any language
- [Troubleshooting](troubleshooting.md)
