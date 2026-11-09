use spotcore::{BrowserEngine, BrowserRole, Paths, SpotCore};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let name = args.first().map(String::as_str).unwrap_or("qt");
    let engine = match name {
        "qt" => BrowserEngine::Qt,
        "system" => BrowserEngine::System,
        "chromium" => BrowserEngine::Chromium,
        _ => return Err("choose qt, system, or chromium, optionally followed by --login".into()),
    };
    let paths = Paths::for_app(&format!("services-test-{name}"))?;
    let builder = SpotCore::builder()
        .data_dir(paths.root())
        .engine(engine)
        .browser_role(BrowserRole::Services);
    if args.iter().any(|arg| arg == "--login") {
        builder.clone().login().await?;
    }
    let sdk = builder.launch().await?;
    let result = async {
        let account = sdk.users().me().await?;
        println!(
            "{engine:?}: {} ({:?}); local playback: {}",
            account.name,
            account.tier,
            sdk.capabilities().load
        );
        println!("Services ready. Press Ctrl+C to stop.");
        tokio::signal::ctrl_c().await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    }
    .await;
    sdk.shutdown().await?;
    result
}
