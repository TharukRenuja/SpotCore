# Quickstart

Use Rust 1.89 or newer. Packages and release tags are not published for installation yet. Build from a local checkout; the path below assumes your app and SpotCore are sibling directories:

```toml
[dependencies]
spotcore = { path = "../SpotCore" }
tokio = { version = "1", features = ["macros", "rt-multi-thread", "signal"] }
```

## Authenticate, search and play

Your app supplies the user’s `sp_dc` cookie. Here’s a complete example: it reads a private cookie file, starts Web playback, and keeps playing until you press Ctrl+C.

```rust
use spotcore::{Paging, SearchKind, SpotCore};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cookie = std::fs::read_to_string(std::env::var("SPOTCORE_COOKIE_FILE")?)?;
    let mut spotify = SpotCore::builder()
        .app_id("my-music-app")?
        .session_cookie(cookie.trim())
        .device_name("My Music App")
        .launch()
        .await?;

    let results = spotify.search()
        .query(SearchKind::Tracks, "Miles Davis", Paging::default())
        .await?;
    if let Some(track) = results.items.first() {
        spotify.play(&track.uri).await?;
    }

    tokio::signal::ctrl_c().await?;
    spotify.shutdown().await?;
    Ok(())
}
```

Build or install the [launcher and Web engine](engines.md) before local Web playback. A cookie does not remove the Widevine/AAC requirement.

```sh
SPOTCORE_COOKIE_FILE=/absolute/path/to/private-cookie cargo run
```

This variable is read by the example above; the builder does not automatically read a cookie file. Never commit cookies or print them in logs.

## Services only: skip local playback setup

Use this when your app needs Spotify data or controls an existing Connect device. In this example, `launch()` starts an authenticated SDK session; **it does not open a browser**.

```rust
use spotcore::{BrowserRole, SpotCore};

async fn services(sp_dc: String) -> Result<(), spotcore::Error> {
    let spotify = SpotCore::builder()
        .session_cookie(sp_dc)
        .browser_role(BrowserRole::Services)
        .launch().await?;
    let account = spotify.users().me().await?;
    println!("{}: {:?}", account.name, account.tier);
    spotify.shutdown().await?;
    Ok(())
}
```

Here, `session_cookie` supplies authentication, `BrowserRole::Services` skips local Web-player setup, and `launch()` initializes HTTP access. `users().me()` then makes an HTTP request. `shutdown()` closes the SDK session.

Without `.browser_role(BrowserRole::Services)`, the default is `Playback`: `launch()` prepares local playback immediately, using an opted-in eligible native engine or a Web browser. Browser startup is not deferred until `play()`.

The services-only example produces no local audio or playback event stream. Remote controls need an existing Connect receiver. To add local audio, choose a playback session as shown above or explicitly opt into native playback; see [playback](playback.md). Profile-only browser sessions are a separate path under [authentication](authentication.md).
