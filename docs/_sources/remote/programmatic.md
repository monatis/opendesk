# Programmatic Use

OpenDesk can be embedded directly into Rust applications via the `opendesk-rs` crate, or consumed over standard Model Context Protocol (MCP) JSON-RPC from any programming language. There is no Python package in the Rust version.

## Rust Library (`opendesk-rs`)

Add `opendesk-rs` to your `Cargo.toml`:

```toml
[dependencies]
opendesk-rs = { path = "../opendesk-rs" }
tokio = { version = "1", features = ["full"] }
anyhow = "1.0"
```

### Driving a Paired Remote Peer

`connect(peer, rendezvous_url, rendezvous_token, home)` loads the identity and
trusted peers from `home` (default `~/.opendesk`) and opens an authenticated,
encrypted session. With `peer = None` it uses the persistent default peer.

```rust
use anyhow::Result;
use opendesk_rs::remote::client::connect;

#[tokio::main]
async fn main() -> Result<()> {
    // Connect to a paired peer (e.g., 'work-pc')
    let remote = connect(Some("work-pc"), None, None, None).await?;

    // Capture a screenshot as PNG bytes
    let png_bytes = remote.screenshot_bytes(None).await?;
    println!("Captured screenshot: {} bytes", png_bytes.len());

    // Type text and trigger hotkeys
    remote.keyboard_type("Hello from Rust!").await?;
    remote.keyboard_hotkey(&["ctrl", "s"]).await?;

    // Run a command (command, shell, timeout in seconds)
    let res = remote.exec("hostname", None, Some(10)).await?;
    println!("{} (timed out: {})", res.stdout.trim(), res.timed_out);

    Ok(())
}
```

### Driving the Local Desktop

```rust
use anyhow::Result;
use opendesk_rs::computer::local::LocalComputer;

fn main() -> Result<()> {
    let computer = LocalComputer::new();

    // Inspect semantic accessibility tree
    let tree = computer.ui_tree(Some("Notepad"), Some(5))?;
    println!("Accessibility tree:\n{tree}");

    // Type and click
    computer.keyboard_type("Autonomous testing")?;
    computer.ui_click(Some("Notepad"), "Save")?;

    Ok(())
}
```

These are the crate's internal types, not a stable public API; check the
signatures in `opendesk-rs/src` for the version you build against.

---

## Language-Agnostic Use via MCP

For Python, Node.js, Go, or any other language, start the `opendesk-mcp` binary and speak MCP JSON-RPC over its stdin/stdout (`tools/list`, `tools/call`). See [Custom / Generic Integration](../integrations/custom.md).
