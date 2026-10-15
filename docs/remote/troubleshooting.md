# Troubleshooting

**`opendesk discover` shows nothing.**

The controlled machine must be running `opendesk pair` or `opendesk serve` with `--no-mdns` *not* set.
mDNS is multicast UDP — it doesn't cross routers, only the same LAN segment. Some Wi-Fi access
points isolate clients from each other ("AP isolation") — check the AP settings.

---

**`opendesk pair-with` says "wrong_code".**

Make sure the code on the controlled machine matches what you typed. Codes change every `opendesk pair` run.

---

**`opendesk serve` says "no trusted peers yet".**

You haven't paired anything. Run `opendesk pair` first; once one peer pairs, `serve` will start accepting connections.

---

**Tool calls land on the local machine instead of the remote one.**

Without a `peer:` argument, calls go to the peer chosen with `opendesk_use`,
then the persistent default (`opendesk peers default NAME`), and otherwise to
`local`. Run `opendesk_use <name>` from the agent, pass `peer: <name>` on each
call, or set a persistent default.

---

**Permissions on macOS.**

`opendesk serve` needs Accessibility + Screen Recording permission to drive mouse/keyboard and capture the screen — same as the local CLI. Grant them to the terminal (or the `opendesk` binary) running `opendesk serve`; `opendesk check` reports what is missing.
