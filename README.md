<h1 align="center">SpotCore</h1>
<p align="center"><strong>One core for Spotify services and playback.</strong></p>
<p align="center"><a href="docs/quickstart.md">Get started</a> · <a href="docs/README.md">Documentation</a> · <a href="examples">Examples</a> · <a href="#capabilities">Capabilities</a></p>

<p align="center">
<a href="https://github.com/TharukRenuja/SpotCore/actions/workflows/ci.yml"><img src="https://github.com/TharukRenuja/SpotCore/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
<a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL_v3-blue" alt="GNU GPL v3"></a>
<img src="https://img.shields.io/badge/Rust-1.89%2B-orange" alt="Rust 1.89 or newer">
</p>

**SpotCore** provides Spotify services and playback for applications. Free and Premium accounts can use Spotify’s Web Player, and Premium accounts can also opt into Soloist for native playback. SpotCore does not decrypt protected content itself, remove advertising, or bypass account entitlements.

## Add to your app

**Requires Rust 1.89 or newer.** Cargo can build SpotCore directly from GitHub without a crates.io package. With the `v0.1.4` release tag published, add:

```toml
[dependencies]
spotcore = { git = "https://github.com/TharukRenuja/SpotCore.git", tag = "v0.1.4" }
```

**Use a published tag or pin a full commit with `rev`, then commit your application's `Cargo.lock`.** Version `0.1.4` is the current source version; the version bump alone does not create a tag. Add `features = ["native"]` if your app needs Soloist support. See [installation](docs/installation.md) for a local source dependency and runtime setup.

## Start playing

```rust
use spotcore::{Paging, SearchKind, SpotCore};

let mut spotify = SpotCore::builder()
    .session_cookie(sp_dc)
    .device_name("My Music App")
    .launch()
    .await?;

spotify.play("spotify:track:0aWMVrwxPNYkKmFthzmpRi").await?;

let results = spotify.search()
    .query(SearchKind::Tracks, "track:Ordinary artist:Alex Warren", Paging::default())
    .await?;

if let Some(track) = results.items.first() {
    spotify.play(&track.uri).await?;
}

// When your app is finished with the session:
spotify.shutdown().await?;
```

Your app supplies the **`sp_dc` session cookie** and an async runtime. These examples show how to play a track, search Spotify and close the session. **Keep the session alive while playing and call `shutdown().await?` when finished.** The [quickstart](docs/quickstart.md) includes a complete application.

---

## Disclaimer

**SpotCore is an unofficial project and is not affiliated with, endorsed by or supported by Spotify.** Spotify’s services, content and software remain subject to their own terms and account permissions.

**Use SpotCore at your own risk.** Uninterrupted access and freedom from account restrictions, suspension or termination are not guaranteed. Using Spotify’s own playback components does not eliminate these risks. You are responsible for your use of the SDK and compliance with Spotify’s applicable terms.

SpotCore is licensed under [GPL-3.0](LICENSE). See [third-party notices](THIRD-PARTY-NOTICES.md) for component licenses and attribution.

---

## Capabilities

### Choose how your app runs

| What your app needs | What to use | What it requires |
| --- | --- | --- |
| Spotify data or control of another device | **Services mode** | Session cookie. No browser or local player needed. Remote controls need an existing Connect device. |
| Play audio locally with Free or Premium | **Web playback** | Session cookie, launcher or embedded supervisor, compatible browser engine, Widevine and AAC. |
| Play audio locally through Soloist | **Native playback** | `native` Cargo feature, Premium account, Soloist API key and a supported platform. |

Use `BrowserRole::Services` for data and remote control without starting a browser. The default `launch()` prepares local playback immediately. **All modes use the same Spotify service APIs.**

### What you can do

| Feature | Answer |
| --- | --- |
| Search, catalog, Home and browse | **Yes.** Read Spotify data in any mode. |
| Saves, follows and library folders | **Yes.** Read and update the signed-in account's library. |
| Playlists and covers | **Yes.** Read playlists and edit those the account can modify. |
| Profiles, lyrics, Canvas and credits | **Yes, when Spotify provides them.** Availability depends on content and account. |
| Play, pause, seek, shuffle and repeat | **Yes.** The target device and current content must allow the control. |
| Add, clear, remove, insert and reorder queue rows | **Yes.** HTTP edits require an active receiver. Exact row edits use returned URI/UID pairs. |
| Jump directly to a queue row | **Web session required.** HTTP-only sessions do not support skip-to. |
| Playback events | **Web and native sessions.** Services-only apps poll state. |
| Audio quality | **Web: up to VeryHigh. Native: device/account-supported levels.** A selected preference does not prove delivered audio quality. |
| Host a Jam, share an invite and end it | **Yes, with Premium and an available Connect receiver.** Soloist can be the receiver. |
| Join a Jam as a guest | **Yes.** Free guests can join and queue; audio plays on the host's device. |
| Download tracks or listen offline | **No.** SpotCore supports streaming only. |

### Free or Premium?

| Feature | Free | Premium |
| --- | --- | --- |
| Spotify data and remote controls | **Yes**, with account/device restrictions | **Yes**, with account/device restrictions |
| Local Web playback | **Yes, with ads** | **Yes** |
| Local Soloist playback | **No** | **Yes**, with an API key and supported platform |
| Host a Jam | **No** | **Yes**, with an available receiver |
| Lossless / HiRes preference | **No** | **Native only**, when the account, device and content allow it |

**Playback uses Spotify's own components and account permissions.** See [playback and queues](docs/playback.md) for routing and controls, and [engine setup](docs/engines.md) for installation. Verify DRM, codecs and audio on each deployment target.

## Platforms

| Platform | Data and remote controls | Local Web audio | Local Soloist audio |
| --- | :---: | :---: | :---: |
| Linux x86_64 / ARM64 | Yes | Yes | Yes |
| Linux ARMv7 | Yes | Yes | Yes |
| Windows x86_64 | Yes | Yes | Yes |
| macOS Apple Silicon | Yes | Yes | Yes |
| macOS Intel | Yes | Yes | No |
| Android | Yes | Yes | No |
| iOS | Not tested | Not tested | No |

**Web audio needs a compatible engine, DRM and codecs on that platform.** Language bindings do not supply a mobile playback host. See [platform prerequisites](docs/engines.md#platform-prerequisites-and-packaging) before packaging your app.

## Developer guides

- [Quickstart](docs/quickstart.md)
- [Language bindings](docs/bindings.md)
- [Configuration](docs/configuration.md)
- [Engine setup](docs/engines.md)
- [Authentication](docs/authentication.md)
- [Playback and queue](docs/playback.md)
- [Discovery](docs/discovery.md)
- [Library](docs/library.md)
- [Playlists](docs/playlists.md)
- [Devices](docs/devices.md)
- [Social and Jam](docs/social.md)
- [Errors](docs/errors.md)

## Language bindings

**Rust, Python, JavaScript/TypeScript, C#, Java, Kotlin, Swift, C, C++ and Go** are available. **Dart is experimental.** Each binding offers typed starter methods and access to all 172 service/playback operations through **`invoke()`**. Those broader operations use JSON arguments and results.

**Build bindings for the target OS and architecture.** They share the SDK’s runtime requirements and do not bundle playback engines. See the [binding matrix and setup](bindings/README.md).

Build the Rust API reference with `cargo doc -p spotcore --all-features --no-deps`.

## Special thanks

Special thanks to **kota-zo and contributors** behind [**Orpheon**](https://github.com/kota-zo/Orpheon) (formerly **Myspoti**)! SpotCore builds on their browser bridge and Spotify API mappings. You’ll find the credits and license details in [third-party notices](THIRD-PARTY-NOTICES.md). Also an **honourable mention to [librespot](https://github.com/librespot-org/librespot)** and its contributors.