# MCP Integration

Run `opendesk install` on the controller as before. The MCP server now exposes:

**Computer-use tools** — `screenshot`, `mouse`, `keyboard`, `ui`, `app`,
`clipboard`, `opendesk_exec`. Each accepts an optional `peer:` argument.
(`audit`, `learn` and `schedule` always act on the controller itself.)

**Admin tools** — for the agent to self-administer:

| Tool | Purpose |
|---|---|
| `opendesk_view [peer]` | Open the interactive MCP App remote desktop viewport inline in supported hosts (Goose, Claude Desktop, web frontends). |
| `opendesk_peers` | List `local` + every trusted peer; the current default is marked `[current]`, open connections `[active]`. |
| `opendesk_discover [timeout]` | Browse the LAN (mDNS) for opendesk peers (paired and unpaired). |
| `opendesk_use <peer>` | Set the default peer for subsequent calls. `peer="local"` reverts. |
| `opendesk_status` | Show the effective default peer and open connections. |
| `opendesk_describe <peer>` | Full description of a peer (set with `opendesk describe` / `opendesk peers describe`). |
| `opendesk_capabilities [peer]` | Capability manifest of a peer. |
| `opendesk_disconnect [peer]` | Close cached connection(s). |
| `opendesk_guard` | Set / clear / show the session's foreground-window guard for `mouse` and `keyboard` (see [Input guard](../tools/index.md#input-guard)). |
| `opendesk_privacy <peer>` | Lock physical input and/or show a blackout curtain on a remote peer (`action`: `set`, `status`, `unlock`). Refused for `local`. |

## Ask Claude

> "What machines are paired?"

> "Switch to the machine called mini"

> "Take a screenshot of mini"

> "Open Terminal on the remote machine and run ls"

> "Use the local machine"

> "What are the capabilities of desk?"

> "Disconnect from mini"

Claude manages peer selection automatically — you just describe which machine and what to do.

---

## Default peer resolution

When a tool call omits `peer:`, opendesk picks one in this order:

1. The session default set via `opendesk_use`.
2. The persistent default peer (`opendesk peers default NAME`).
3. Otherwise, the local machine.

Pairing a single peer does not make it the default, and having several
peers with no default is not an error: calls simply go to `local`. If you
mean a remote machine, say so (`opendesk_use`, a `peer:` argument, or a
persistent default).

## Agent example

```
> opendesk_use mini
> screenshot
[on mini] Captured 1920x1200 screenshot.
> opendesk_use local
```

## Why pairing isn't an MCP tool

The 6-digit code authenticates the pairing handshake. Codes should be
typed by a human looking at the controlled machine's screen, not flow
through the LLM channel where they might be logged, leaked, or replayed.
`opendesk pair` / `opendesk pair-with` are CLI-only by design.

---

Next: [Trust & Security →](security.md)
