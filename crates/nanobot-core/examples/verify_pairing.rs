#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use nanobot_core::pairing;
    
    // Initialize DB (uses flowbot.db by default)
    println!("Initializing database...");
    pairing::init_database().await?;
    
    // Test data
    let channel = "telegram";
    let user_id = "123456789";
    let username = Some("test_user".to_string());
    
    // 1. Clean state
    println!("Cleaning up previous state...");
    // We can't easily delete generic requests via public API, but we can ignore previous state for this test logic
    // or we could add a cleanup function if this was a real integration test.
    // For now, let's just proceed.
    
    // 2. Check initial authorization (should be false)
    println!("Checking initial auth status...");
    let is_auth = pairing::is_authorized(channel, user_id).await?;
    println!("Is authorized? {}", is_auth);
    
    if !is_auth {
        // 3. Create pairing request
        println!("Creating pairing request...");
        let code = pairing::create_pairing_request(channel, user_id.to_string(), username.clone()).await?;
        println!("Generated code: {}", code);
        
        // 4. Verify code is pending
        let pending_code = pairing::get_user_code(channel, user_id).await?;
        assert_eq!(pending_code, Some(code.clone()), "Pending code should match generated code");
        println!("✅ Code is pending verification");
        
        // 5. Approve usage
        println!("Approving code...");
        let approved_user = pairing::approve(channel, &code).await?;
        assert_eq!(approved_user, user_id, "Approved user ID should match");
        println!("✅ Pairing approved");
        
        // 6. Verify authorization
        let is_auth_now = pairing::is_authorized(channel, user_id).await?;
        assert!(is_auth_now, "User should be authorized now");
        println!("✅ User is now authorized");
        
    } else {
        println!("⚠️ User was already authorized. Skipping full flow.");
    }

    Ok(())
}
