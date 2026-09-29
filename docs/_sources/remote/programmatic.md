# Programmatic Use

OpenDesk can be embedded directly into Rust applications via the `opendesk-rs` crate, or consumed over standard Model Context Protocol (MCP) JSON-RPC from any programming language.

## Rust Library (`opendesk-rs`)

Add `opendesk-rs` to your `Cargo.toml`:

```toml
[dependencies]
opendesk-rs = { path = "../opendesk-rs" }
tokio = { version = "1", features = ["full"] }
anyhow = "1.0"
```

### Driving a Paired Remote Peer

```rust
use anyhow::Result;
use opendesk_rs::protocol::identity::Identity;
use opendesk_rs::protocol::storage::TrustedPeers;
use opendesk_rs::remote::client::connect;

#[tokio::main]
async fn main() -> Result<()> {
    let home = dirs::home_dir().expect("home dir").join(".opendesk");
    let identity = Identity::load_or_create(&home)?;
    let trusted = TrustedPeers::load(&home)?;

    // Connect to a paired peer (e.g., 'work-pc')
    let remote = connect("work-pc", &home, &identity, &trusted, None).await?;

    // Capture screenshot as native binary PNG bytes
    let png_bytes = remote.screenshot_bytes().await?;
    println!("Captured screenshot: {} bytes", png_bytes.len());

    // Type text and trigger hotkeys
    remote.keyboard_type("Hello from Rust!").await?;
    remote.keyboard_hotkey(&["ctrl", "s"]).await?;

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

---

## Language-Agnostic Use via MCP

For Python, Node.js, Go, or any other language, connect to the standalone `opendesk-mcp` binary via standard stdio JSON-RPC.
