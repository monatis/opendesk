use anyhow::Result;
use opendesk_rs::protocol::storage::resolve_rendezvous_config;
use opendesk_rs::remote::client::connect as remote_connect;

#[tokio::test]
#[ignore]
async fn test_live_oldpc_privacy() -> Result<()> {
    let peer = "old-pc";
    println!("\n=== Testing Live Privacy on '{}' ===", peer);

    let r_cfg = resolve_rendezvous_config(None, None, None);
    let r_url = if !r_cfg.url.is_empty() {
        Some(r_cfg.url.as_str())
    } else {
        None
    };

    println!("Connecting to {} via relay {:?}...", peer, r_url);
    let remote = remote_connect(Some(peer), r_url, r_cfg.token.as_deref(), None).await?;
    println!("✓ Connected!");

    // 1. Check initial privacy status
    let initial_status = remote.get_privacy().await?;
    println!("Initial privacy status: {:?}", initial_status);

    // 2. Enable Input Lock and Blackout Screen
    println!("\n[1] Enabling input locking and blackout curtain on {}...", peer);
    let st = remote.set_privacy(true, true).await?;
    println!("✓ Privacy state applied: {:?}", st);
    assert!(st.lock_input, "Expected lock_input to be true");
    assert!(st.blackout, "Expected blackout to be true");

    // Give the window manager 500ms to settle
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // 3. Verify screen capture is still visible and capturing desktop
    println!("\n[2] Testing screen capture while blackout curtain is active...");
    let shot_bytes = remote.screenshot_bytes(None).await?;
    println!("✓ Received screenshot: {} bytes", shot_bytes.len());
    assert!(!shot_bytes.is_empty(), "Screenshot should not be empty");
    std::fs::write("test_shot_during_privacy.png", &shot_bytes)?;
    println!("✓ Saved screenshot to test_shot_during_privacy.png");

    // 4. Test input simulation while physical input is locked
    println!("\n[3] Testing remote synthetic input simulation while physical input is locked...");
    remote.mouse_move(400, 400).await?;
    println!("✓ Synthetic mouse_move succeeded");
    remote.mouse_click(400, 400, Some("left")).await?;
    println!("✓ Synthetic mouse_click succeeded");
    remote.clipboard_write("PRIVACY_TEST_PASSED").await?;
    let text = remote.clipboard_read().await?;
    println!("✓ Synthetic clipboard read/write returned: {}", text);
    assert_eq!(text, "PRIVACY_TEST_PASSED");

    // 5. Test unlocking
    println!("\n[4] Unlocking {}...", peer);
    let st_unlocked = remote.set_privacy(false, false).await?;
    println!("✓ Unlocked: {:?}", st_unlocked);
    assert!(!st_unlocked.lock_input);
    assert!(!st_unlocked.blackout);

    println!("\n=== ALL TESTS PASSED! ===");
    Ok(())
}
