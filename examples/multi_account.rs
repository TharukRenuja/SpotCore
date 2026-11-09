use spotcore::{AccountManager, AccountProfile, BrowserRole, Paths, WebCredentials};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths = Paths::for_app("multi-account-example")?;
    let mut accounts = AccountManager::new(paths)?.browser_role(BrowserRole::Services);
    for account in accounts.list().await? {
        println!(
            "{} | {:?} | {:?}",
            account.canonical_username, account.display_name, account.tier
        );
    }
    if let Some(id) = std::env::args().nth(1) {
        if let Some(file) = std::env::var_os("SPOTCORE_COOKIE_FILE") {
            let cookie = std::fs::read_to_string(file)?;
            let mut profile = AccountProfile::new(&id);
            profile.web = Some(WebCredentials {
                sp_dc: Some(cookie.trim().into()),
                access_token: None,
                expires_at_ms: None,
            });
            accounts.switch_with(profile).await?;
        } else {
            accounts.login(&id).await?;
        }
        println!("Selected {}", accounts.active_account().unwrap());
        accounts.close().await?;
    }
    Ok(())
}
