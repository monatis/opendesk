# One-time Setup

## On the controlled machine

Install the binaries on both machines (`cargo install --path opendesk-rs`),
then:

```bash
opendesk pair
```

You'll see something like:

```
┌──────────────────────────────────────────────┐
│  opendesk pairing                            │
│  port:        8423                           │
│  fingerprint: 9c2f:1abc:b3d4:8870            │
│                                              │
│   pairing code:   428901                     │
│                                              │
│  Run on the controller:                      │
│    opendesk pair-with <host> 428901          │
└──────────────────────────────────────────────┘
```

Leave this terminal open while pairing. Once a controller successfully
pairs, the process exits.

## On the controller

```bash
opendesk discover
```

This lists `_opendesk._tcp.local` services on the LAN:

```
NAME                              ADDR                    FINGERPRINT
mac-mini-9c2f1a                   192.168.1.42:8423       9c2f:1abc:b3d4:8870
```

Now pair (using the code shown on the controlled machine):

```bash
opendesk pair-with mac-mini.local 428901 --name mini
```

Output:

```
✓ Paired with mini (9c2f:1abc:b3d4:8870)
  Now reachable as: opendesk connect mini
```

Both machines now have each other's static public keys stored in
`~/.opendesk/trusted-peers.json`.

---

Pairing done. Next: [start the daemon →](running.md)
