# CLI Reference

Most commands accept `--home DIR` to override the identity / trusted-peers
location (default `~/.opendesk`). Commands that reach other machines also
accept `--rendezvous URL` and `--rendezvous-token TOKEN`; without them, the
saved configuration from `opendesk set-rendezvous` is used. Run
`opendesk <command> --help` for the authoritative list.

## Controlled machine

```
opendesk pair         [--host H] [--port N] [--code XXXXXX] [--timeout S] [--no-mdns]
                      [--rendezvous URL] [--rendezvous-token TOKEN]
opendesk serve        [--host H] [--port N] [--no-mdns] [--no-audit]
                      [--rendezvous URL]... [--rendezvous-token TOKEN]
opendesk rendezvous   [--host H] [--port N] [--token TOKEN]   # standalone relay server
opendesk sessions                    # show the active controller (0 or 1)
opendesk disconnect                  # ask the active controller to leave
opendesk unpair NAME                 # revoke trust + disconnect if active
opendesk describe     [TEXT] [--clear]   # this machine's broadcast description
opendesk check        [--open]       # macOS Accessibility / Screen Recording probe
opendesk audit        [--date Y-M-D] [--peer NAME] [--limit N] [--follow]
opendesk install-service [--port N] [--no-start] [--rendezvous URL] [--rendezvous-token TOKEN]
opendesk uninstall-service
opendesk wsl-setup    [--port N] [--apply | --undo]   # WSL only: Windows-side port forwarding
```

## Controller

```
opendesk discover     [--timeout S] [--rendezvous URL] [--rendezvous-token TOKEN]
opendesk pair-with    [HOST] CODE [--port N] [--name NAME] [--target-pubkey HEX]
                      [--rendezvous URL] [--rendezvous-token TOKEN]
opendesk connect      [PEER] [--screenshot PATH]   # PEER optional if a default is set
opendesk peers        [list | default [NAME|--clear] | rename NAME NEW | remove NAME
                       | describe NAME [TEXT|--clear] | rendezvous NAME [URL]]
opendesk privacy      PEER [--lock-input] [--blackout] [--unlock]
                      # Remote-only: locks physical input / blanks screen on the remote peer.
                      # Automatically unlocks when the controller disconnects. Local use prohibited for safety.
opendesk exec         (COMMAND | --file SCRIPT) [--peer PEER] [--shell powershell|cmd|bash|sh] [--timeout S]
                      # Run a command on this machine or a remote peer (default timeout 30 s).
                      # --file sends a local script's contents. On timeout the process tree is
                      # killed, partial output is printed, and the exit code is 124.
```

## Agent integration and settings

```
opendesk mcp          [--rendezvous URL] [--rendezvous-token TOKEN] [--home DIR]
                      # stdio MCP server (same as opendesk-mcp; also the default with no subcommand)
opendesk install      [--scope user|project|local] [--rendezvous URL] [--rendezvous-token TOKEN]
opendesk uninstall    # remove the Claude Code registration
opendesk app          [--host H] [--port N] [--no-browser]   # local web UI, default http://127.0.0.1:8424
opendesk set-rendezvous [URL] [--token TOKEN] [--clear]
opendesk config       [show | rendezvous [URL] [--token TOKEN] [--clear]]
opendesk scheduler    start|list [--dir DIR]   # schedules are stored only; `start` just logs them
opendesk test         # quick local diagnostic (lists apps, reads the clipboard)
```

---

Want `opendesk serve` to survive reboots? See [Service Install →](service.md)
