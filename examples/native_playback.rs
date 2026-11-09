use spotcore::Client;
use spotcore::native::{NativeBackend, NativeManager};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== SpotCore Native Playback Integration ===\n");

    let paths = spotcore::Paths::default();
    let manager = NativeManager::new(paths.native());
    println!("Native data root       : {}", manager.root().display());
    println!("Daemon state directory : {}", paths.native_data().display());
    println!(
        "Audio cache directory  : {}",
        paths.native_cache().display()
    );
    println!(
        "Native binary path     : {}",
        manager.binary_path().display()
    );
    println!("Is binary cached & fresh: {}", manager.is_binary_fresh());

    let (url, format) = NativeManager::release_url()?;
    println!("Official Spotify CDN URL: {} ({})", url, format);

    let api_key = std::env::var("SPOTCORE_NATIVE_API_KEY")
        .or_else(|_| std::env::var("SOLOIST_API_KEY"))
        .ok();
    let ws_url = std::env::var("SPOTCORE_NATIVE_WS_URL")
        .or_else(|_| std::env::var("SOLOIST_WS_URL"))
        .unwrap_or_else(|_| "ws://127.0.0.1:9090".to_string());

    let backend = match api_key {
        Some(key) => {
            println!("\nLaunching SpotCore native daemon with provided API key...");
            NativeBackend::launch_with(key, &paths).await?
        }
        None => {
            println!(
                "\nNo API key set. Connecting to existing daemon on {}...",
                ws_url
            );
            println!(
                "(Tip: Run `spotcore-native --device-name SpotCore --api-key <YOUR_KEY> --ws 127.0.0.1:9090` or export SPOTCORE_NATIVE_API_KEY)"
            );
            match NativeBackend::connect(&ws_url).await {
                Ok(b) => b,
                Err(e) => {
                    println!("\n[Notice] Could not connect to local daemon: {}", e);
                    println!(
                        "This example verifies the architecture. When a native daemon is running, SpotCore controls it via Client<NativeBackend>."
                    );
                    return Ok(());
                }
            }
        }
    };

    println!("✓ Successfully connected to SpotCore native backend!");

    let mut client = Client::new(backend);
    println!("Backend capabilities: {:?}", client.capabilities());

    let track_uri = "spotify:track:4cOdK2wGLETKBW3PvgPWqT";
    println!("\n[Command] Loading and playing: {}", track_uri);
    client.play(track_uri).await?;

    println!("[Command] Setting volume to 80%...");
    client.set_volume(0.80).await?;

    println!("[Command] Seeking to 30s...");
    client.seek(Duration::from_secs(30)).await?;

    println!("\nStreaming live Soloist playback events. Press Ctrl+C to stop playback...");

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("\nReceived Ctrl+C, stopping playback.");
                let _ = client.pause().await;
                break;
            }
            event = client.next_event() => {
                match event {
                    Ok(ev) => println!(" -> [Soloist Event]: {:?}", ev),
                    Err(e) => {
                        println!(" -> Event stream ended: {:?}", e);
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}
