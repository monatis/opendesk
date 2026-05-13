/**
 * opendesk-js CLI — mirrors the Python opendesk CLI.
 *
 * Controlled-machine commands: serve, pair, sessions, disconnect, describe
 * Controller commands:         pair-with, discover, connect, peers, unpair
 * Shared:                      install, uninstall, mcp
 */

import fs from "fs";
import path from "path";
import os from "os";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function fatal(msg: string, code = 1): never {
  process.stderr.write(`ERROR: ${msg}\n`);
  process.exit(code);
}

function formatAge(seconds: number): string {
  const s = Math.floor(seconds);
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

/** Resolve --home to an absolute path. Prevents path traversal. */
function resolveHome(raw: string | undefined): string | undefined {
  if (!raw) return undefined;
  return path.resolve(String(raw));
}

function validatePort(raw: unknown): number {
  const n = Number(raw);
  if (!Number.isInteger(n) || n < 1 || n > 65535) fatal(`invalid port: ${raw}`);
  return n;
}

function validateCode(raw: unknown): string {
  const s = String(raw ?? "");
  if (!/^\d{6}$/.test(s)) fatal("pairing code must be exactly 6 digits");
  return s;
}

/** Minimal flag parser — handles --key=val, --key val, --bool */
function parseFlags(argv: string[]): { pos: string[]; flags: Record<string, string | boolean> } {
  const pos: string[] = [];
  const flags: Record<string, string | boolean> = {};
  let i = 0;
  while (i < argv.length) {
    const arg = argv[i];
    if (arg === "--") { pos.push(...argv.slice(i + 1)); break; }
    if (arg.startsWith("--")) {
      const eqIdx = arg.indexOf("=");
      if (eqIdx > 2) {
        flags[arg.slice(2, eqIdx)] = arg.slice(eqIdx + 1);
      } else {
        const key = arg.slice(2);
        const nxt = argv[i + 1];
        if (nxt !== undefined && !nxt.startsWith("-")) { flags[key] = nxt; i++; }
        else { flags[key] = true; }
      }
    } else {
      pos.push(arg);
    }
    i++;
  }
  return { pos, flags };
}

function str(v: unknown): string  { return v != null ? String(v) : ""; }
function bool(v: unknown): boolean { return v === true || v === "true" || v === "1"; }
function num(v: unknown, def: number): number {
  const n = Number(v);
  return Number.isFinite(n) ? n : def;
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

async function cmdInstall(argv: string[]): Promise<void> {
  const { flags } = parseFlags(argv);
  const scope = str(flags["scope"]) || "user";
  if (scope !== "user" && scope !== "project") fatal("--scope must be 'user' or 'project'");
  const { install } = await import("./install.js");
  install(scope);
}

async function cmdUninstall(): Promise<void> {
  const { uninstall } = await import("./install.js");
  uninstall();
}

async function cmdMcp(): Promise<void> {
  const { runMcpStdio } = await import("./mcp.js");
  await runMcpStdio();
}

// ---------------------------------------------------------------------------
// serve
// ---------------------------------------------------------------------------

async function cmdServe(argv: string[]): Promise<void> {
  const { flags } = parseFlags(argv);
  const home    = resolveHome(str(flags["home"]) || undefined);
  const port    = validatePort(flags["port"] ?? 8423);
  const host    = str(flags["host"]) || "0.0.0.0";
  const noMdns  = bool(flags["no-mdns"]);
  const approve = str(flags["approve"]) || "auto";

  if (approve !== "auto" && approve !== "console") {
    fatal("--approve must be 'auto' or 'console'");
  }

  const { Identity }        = await import("./protocol/auth/identity.js");
  const { fingerprint }     = await import("./protocol/auth/identity.js");
  const { TrustedPeers }   = await import("./protocol/auth/storage.js");
  const { OpendeskServer } = await import("./remote/server.js");
  const { ToolDispatcher } = await import("./computer/dispatcher.js");
  const { createRegistry } = await import("./registry.js");
  const { allowAllContext, PermissionDeniedError } = await import("./tools/base.js");

  const identity = Identity.loadOrCreate(home);
  const trusted  = new TrustedPeers(home);

  if (!trusted.list().length) {
    fatal("no trusted peers yet — run 'opendesk-js pair' first to pair a controller", 2);
  }

  let ctx;
  if (approve === "console") {
    if (!process.stdin.isTTY) {
      process.stderr.write("WARNING: --approve console requires a TTY; falling back to auto-approve\n");
      ctx = allowAllContext();
    } else {
      const readline = await import("readline");
      ctx = {
        sessionId: "server",
        permissionHandler: async (tool: string, argument: string, description: string) => {
          const rl = readline.createInterface({ input: process.stdin, output: process.stderr });
          const answer = await new Promise<string>((resolve) =>
            rl.question(`Allow ${tool}(${argument}) — ${description}? [y/N] `, resolve),
          );
          rl.close();
          if (!answer.trim().toLowerCase().startsWith("y")) {
            throw new PermissionDeniedError(`denied: ${tool}(${argument})`);
          }
        },
      };
    }
  } else {
    ctx = allowAllContext();
  }

  const dispatcher = new ToolDispatcher({ registry: createRegistry(), ctx });
  const server = new OpendeskServer(identity, trusted, {
    host,
    port,
    home,
    advertise: !noMdns,
    dispatcherFactory: () => dispatcher,
  });

  await server.start();
  const fp = fingerprint(identity.publicBytes);
  console.log(`opendesk serve  ${host}:${server.port}  fp=${fp}  approve=${approve}`);
  if (host === "0.0.0.0") {
    console.log("Listening on all interfaces. Ensure your firewall allows port " + server.port + " on trusted networks only.");
  }

  const shutdown = async () => {
    console.log("\nShutting down.");
    await server.close();
    process.exit(0);
  };
  process.on("SIGINT",  () => void shutdown());
  process.on("SIGTERM", () => void shutdown());

  // Block forever until signal
  await new Promise<void>(() => {});
}

// ---------------------------------------------------------------------------
// pair  (controlled machine — generates a code, waits for controller)
// ---------------------------------------------------------------------------

async function cmdPair(argv: string[]): Promise<void> {
  const { flags } = parseFlags(argv);
  const home      = resolveHome(str(flags["home"]) || undefined);
  const port      = validatePort(flags["port"] ?? 8423);
  const host      = str(flags["host"]) || "0.0.0.0";
  const noMdns    = bool(flags["no-mdns"]);
  const timeout   = num(flags["timeout"], 300) * 1000; // convert s → ms
  const rawCode   = str(flags["code"]) || undefined;
  const code      = rawCode ? validateCode(rawCode) : undefined;

  const { Identity }         = await import("./protocol/auth/identity.js");
  const { fingerprint, generatePairingCode } = await import("./protocol/auth/identity.js");
  const { TrustedPeers }    = await import("./protocol/auth/storage.js");
  const { OpendeskServer }  = await import("./remote/server.js");

  const identity  = Identity.loadOrCreate(home);
  const trusted   = new TrustedPeers(home);
  const pairingCode = code ?? generatePairingCode();

  const server = new OpendeskServer(identity, trusted, {
    host, port, home, advertise: !noMdns,
  });
  await server.start();

  const fp = fingerprint(identity.publicBytes);
  console.log();
  console.log("┌──────────────────────────────────────────────────────┐");
  console.log("│  opendesk pairing                                    │");
  console.log(`│  port:        ${String(server.port).padEnd(39)}│`);
  console.log(`│  fingerprint: ${fp.padEnd(39)}│`);
  console.log("│                                                      │");
  console.log(`│  pairing code:  ${pairingCode.padEnd(37)}│`);
  console.log("│                                                      │");
  console.log("│  On the controller machine run:                      │");
  console.log(`│    opendesk-js pair-with <host> ${pairingCode.padEnd(21)}│`);
  console.log("└──────────────────────────────────────────────────────┘");
  console.log();

  let newPubkey: Buffer | null;
  try {
    newPubkey = await server.enablePairing(pairingCode, timeout);
  } catch {
    newPubkey = null;
  }

  await server.close();

  if (!newPubkey) {
    fatal(`no peer paired within ${Math.floor(timeout / 1000)}s`);
  }

  const fp2  = fingerprint(newPubkey);
  const peer = trusted.find(newPubkey);
  const name = peer?.name ?? `peer-${newPubkey.toString("hex").slice(0, 6)}`;
  console.log(`Paired with ${name} (${fp2})`);
  console.log(`Use 'opendesk-js peers rename ${name} <friendly-name>' to give it a better name.`);
}

// ---------------------------------------------------------------------------
// pair-with  (controller machine — connects to a running opendesk pair)
// ---------------------------------------------------------------------------

async function cmdPairWith(argv: string[]): Promise<void> {
  const { pos, flags } = parseFlags(argv);
  if (pos.length < 2) fatal("usage: opendesk-js pair-with <host> <code> [--port=8423] [--name=<name>]");
  const [host, rawCode] = pos;
  const code = validateCode(rawCode);
  const port = validatePort(flags["port"] ?? 8423);
  const name = str(flags["name"]) || undefined;
  const home = resolveHome(str(flags["home"]) || undefined);

  const { fingerprint } = await import("./protocol/auth/identity.js");
  const { pairWith }    = await import("./remote/client.js");

  let remote, serverPubkey;
  try {
    ({ remote, serverPubkey } = await pairWith(host, port, code, { home, name }));
  } catch (e) {
    fatal(e instanceof Error ? e.message : String(e));
  }
  await remote.close();

  const fp       = fingerprint(serverPubkey);
  const peerName = name || `peer-${serverPubkey.toString("hex").slice(0, 6)}`;
  console.log(`Paired with ${peerName} (${fp})`);
  console.log(`Now reachable as: opendesk-js connect ${peerName}`);
}

// ---------------------------------------------------------------------------
// discover
// ---------------------------------------------------------------------------

async function cmdDiscover(argv: string[]): Promise<void> {
  const { flags } = parseFlags(argv);
  const timeout   = num(flags["timeout"], 2000);
  const { discover } = await import("./remote/discovery.js");
  const peers = await discover(timeout);
  if (!peers.length) { console.log("No opendesk peers found on the LAN."); return; }
  console.log(`${"NAME".padEnd(24)}  ${"ADDR".padEnd(22)}  ${"FINGERPRINT".padEnd(22)}  DESCRIPTION`);
  for (const p of peers) {
    const addr = `${p.host}:${p.port}`;
    const desc = (p.description ?? "").slice(0, 60);
    console.log(`${p.name.padEnd(24)}  ${addr.padEnd(22)}  ${p.fingerprint.padEnd(22)}  ${desc}`);
  }
}

// ---------------------------------------------------------------------------
// connect  (smoke test)
// ---------------------------------------------------------------------------

async function cmdConnect(argv: string[]): Promise<void> {
  const { pos, flags } = parseFlags(argv);
  const peer = pos[0] || undefined;
  const home = resolveHome(str(flags["home"]) || undefined);

  const { connect } = await import("./remote/client.js");

  let remote;
  try {
    remote = await connect(peer, { home });
  } catch (e) {
    fatal(e instanceof Error ? e.message : String(e));
  }

  try {
    const caps  = remote.capabilities();
    const label = peer ?? "default";
    console.log(`Connected to ${label}  backend=${caps.backend ?? "remote"}`);
    if (caps.description) console.log(`Description: ${caps.description}`);

    // Take a screenshot to exercise the full ToolDispatcher path
    const result = await remote.call("tool.screenshot", {});
    const atts   = (result?.["attachments"] as Array<{ mediaType: string; content: string }>) ?? [];
    const img    = atts.find((a) => a.mediaType.startsWith("image/"));
    if (img) {
      console.log("Screenshot: OK (received image data)");
    } else if (result?.["output"]) {
      console.log(`Screenshot response: ${String(result["output"]).slice(0, 80)}`);
    } else {
      console.log("Screenshot call returned (no image attached)");
    }
  } finally {
    await remote.close();
  }
}

// ---------------------------------------------------------------------------
// peers  (list | remove | rename | default | describe)
// ---------------------------------------------------------------------------

async function cmdPeers(argv: string[]): Promise<void> {
  const [sub, ...rest] = argv;
  const action = sub && !sub.startsWith("-") ? sub : "list";
  const subArgv = action === sub ? rest : argv;

  const { flags, pos } = parseFlags(subArgv);
  const home = resolveHome(str(flags["home"]) || undefined);
  const { TrustedPeers, effectiveDescription } = await import("./protocol/auth/storage.js");
  const store = new TrustedPeers(home);

  if (action === "list") {
    const peers = store.list();
    if (!peers.length) { console.log("No trusted peers."); return; }
    const def = store.getDefault();
    console.log(`${"NAME".padEnd(22)}  ${"FINGERPRINT".padEnd(22)}  ${"LAST ENDPOINT".padEnd(22)}  DESCRIPTION`);
    for (const p of peers) {
      const { fingerprint } = await import("./protocol/auth/identity.js");
      const fp       = fingerprint(Buffer.from(p.publicKey, "hex"));
      const endpoint = p.lastHost ? `${p.lastHost}:${p.lastPort}` : "(unknown)";
      let desc       = effectiveDescription(p);
      if (desc.length > 60) desc = desc.slice(0, 59) + "…";
      const marker   = p.name === def ? "  [default]" : "";
      console.log(`${p.name.padEnd(22)}  ${fp.padEnd(22)}  ${endpoint.padEnd(22)}  ${desc}${marker}`);
    }

  } else if (action === "remove") {
    const target = pos[0] || str(flags["target"]);
    if (!target) fatal("usage: opendesk-js peers remove <name-or-key>");
    if (!store.remove(target)) fatal(`no peer matched '${target}'`);
    console.log(`Removed ${target}.`);

  } else if (action === "rename") {
    const [target, newName] = pos;
    if (!target || !newName) fatal("usage: opendesk-js peers rename <target> <new-name>");
    if (!store.rename(target, newName)) fatal(`no peer matched '${target}'`);
    console.log(`Renamed ${target} → ${newName}.`);

  } else if (action === "default") {
    if (bool(flags["clear"])) {
      const cleared = store.clearDefault();
      console.log(cleared ? "Default peer cleared." : "No default peer was set.");
      return;
    }
    const name = pos[0] || str(flags["name"]) || undefined;
    if (!name) {
      const cur = store.getDefault();
      console.log(cur ? cur : "No default peer set.");
      return;
    }
    if (!store.setDefault(name)) fatal(`no trusted peer named '${name}'`);
    console.log(`Default peer is now: ${name}`);

  } else if (action === "describe") {
    const name = pos[0] || str(flags["name"]);
    if (!name) fatal("usage: opendesk-js peers describe <name> [text] [--clear]");

    if (bool(flags["clear"])) {
      if (!store.clearDescriptionOverride(name)) fatal(`no peer named '${name}'`);
      console.log(`Description override for ${name} cleared.`);
      return;
    }
    const text = pos[1] || str(flags["text"]) || undefined;
    if (!text) {
      const p = store.findByName(name);
      if (!p) fatal(`no peer named '${name}'`);
      if (p.descriptionOverride) console.log(`override:   ${p.descriptionOverride}`);
      if (p.description)         console.log(`broadcast:  ${p.description}`);
      if (!p.descriptionOverride && !p.description) {
        console.log("(no description set)");
      }
      return;
    }
    if (!store.setDescriptionOverride(name, text)) fatal(`no peer named '${name}'`);
    console.log(`Description override saved for ${name}.`);

  } else {
    fatal(`unknown peers subcommand: ${action}`);
  }
}

// ---------------------------------------------------------------------------
// unpair
// ---------------------------------------------------------------------------

async function cmdUnpair(argv: string[]): Promise<void> {
  const { pos, flags } = parseFlags(argv);
  const name = pos[0] || str(flags["name"]);
  if (!name) fatal("usage: opendesk-js unpair <name>");
  const home = resolveHome(str(flags["home"]) || undefined);
  const { TrustedPeers } = await import("./protocol/auth/storage.js");
  const store = new TrustedPeers(home);
  if (!store.remove(name)) fatal(`no peer matched '${name}'`);
  console.log(`Unpaired ${name}.`);
}

// ---------------------------------------------------------------------------
// describe  (broadcast description of THIS machine)
// ---------------------------------------------------------------------------

async function cmdDescribe(argv: string[]): Promise<void> {
  const { pos, flags } = parseFlags(argv);
  const home = resolveHome(str(flags["home"]) || undefined);
  const { readDescription, writeDescription, clearDescription } = await import("./remote/server.js");

  if (bool(flags["clear"])) {
    const cleared = clearDescription(home);
    console.log(cleared ? "Description cleared." : "No description was set.");
    return;
  }
  const text = pos[0] || str(flags["text"]) || undefined;
  if (!text) {
    const cur = readDescription(home);
    console.log(cur || "(no description set)");
    return;
  }
  writeDescription(home, text);
  console.log("Description saved. Next session will broadcast it.");
}

// ---------------------------------------------------------------------------
// sessions
// ---------------------------------------------------------------------------

async function cmdSessions(argv: string[]): Promise<void> {
  const { flags } = parseFlags(argv);
  const home = resolveHome(str(flags["home"]) || undefined);
  const { AdminClient, AdminError } = await import("./remote/admin.js");

  let client;
  try {
    client = await AdminClient.connect(home);
  } catch (e) {
    fatal(e instanceof AdminError ? e.message : String(e));
  }
  try {
    const sessions = await client.listSessions();
    if (!sessions.length) { console.log("No active session."); return; }
    console.log(`${"PEER".padEnd(22)}  ${"FROM".padEnd(22)}  ${"AGE".padEnd(8)}  ID`);
    for (const s of sessions) {
      console.log(
        `${s.peer_name.padEnd(22)}  ${s.remote_addr.padEnd(22)}  ${formatAge(s.age_seconds).padEnd(8)}  ${s.id}`,
      );
    }
  } finally {
    client.close();
  }
}

// ---------------------------------------------------------------------------
// disconnect  (cooperative eviction of the active controller)
// ---------------------------------------------------------------------------

async function cmdDisconnect(argv: string[]): Promise<void> {
  const { flags } = parseFlags(argv);
  const home = resolveHome(str(flags["home"]) || undefined);
  const { AdminClient, AdminError } = await import("./remote/admin.js");

  let client;
  try {
    client = await AdminClient.connect(home);
  } catch (e) {
    fatal(e instanceof AdminError ? e.message : String(e));
  }
  try {
    const n = await client.killAll();
    if (n === 0) { console.log("No active session to disconnect."); return; }
    console.log("Disconnected the active controller.");
  } finally {
    client.close();
  }
}

// ---------------------------------------------------------------------------
// Help
// ---------------------------------------------------------------------------

function printHelp(): void {
  console.log(`opendesk-js — JavaScript SDK for opendesk

Controlled-machine commands (run on the machine to be controlled):
  serve         Long-running server — accepts paired controllers only
  pair          Accept one new controller (prints a pairing code)
  sessions      Show active controller session
  disconnect    Evict the active controller (peer stays paired)
  describe      Read / set / clear this machine's broadcast description

Controller commands (run on the machine that drives others):
  pair-with     Complete pairing with a machine running 'opendesk-js pair'
  discover      Find opendesk peers on the LAN
  connect       Smoke-test connection to a paired peer
  peers         Manage trusted peers (list | remove | rename | default | describe)
  unpair        Revoke a paired peer

Shared:
  install       Register the native MCP server with Claude Code
  uninstall     Remove the MCP server registration from Claude Code
  mcp           Run the MCP server over stdio

Flags (common):
  --home=<dir>  Identity / trusted-peers directory (default: ~/.opendesk)

Examples:
  opendesk-js pair                          # start pairing on port 8423
  opendesk-js pair-with 192.168.1.42 123456 --name=laptop
  opendesk-js serve
  opendesk-js discover
  opendesk-js connect laptop
  opendesk-js peers list
  opendesk-js peers default laptop
`);
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

export async function main(): Promise<void> {
  const [command, ...rest] = process.argv.slice(2);

  try {
    switch (command) {
      case "install":     return await cmdInstall(rest);
      case "uninstall":   return await cmdUninstall();
      case "mcp":         return await cmdMcp();
      case "serve":       return await cmdServe(rest);
      case "pair":        return await cmdPair(rest);
      case "pair-with":   return await cmdPairWith(rest);
      case "discover":    return await cmdDiscover(rest);
      case "connect":     return await cmdConnect(rest);
      case "peers":       return await cmdPeers(rest);
      case "unpair":      return await cmdUnpair(rest);
      case "describe":    return await cmdDescribe(rest);
      case "sessions":    return await cmdSessions(rest);
      case "disconnect":  return await cmdDisconnect(rest);
      default:
        printHelp();
        if (command && command !== "--help" && command !== "-h") {
          process.stderr.write(`Unknown command: ${command}\n`);
          process.exit(1);
        }
    }
  } catch (e) {
    if ((e as NodeJS.ErrnoException).code === "ERR_USE_AFTER_CLOSE" ||
        (e instanceof Error && e.message.includes("closed"))) {
      // Ignore clean shutdown errors
      return;
    }
    process.stderr.write(`ERROR: ${e instanceof Error ? e.message : String(e)}\n`);
    process.exit(1);
  }
}
