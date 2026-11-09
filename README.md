<h1 align="center">SpotCore</h1>
<p align="center"><strong>One core for Spotify services and playback.</strong></p>
<p align="center"><a href="docs/quickstart.md">Get started</a> · <a href="docs/README.md">Documentation</a> · <a href="examples">Examples</a> · <a href="#capabilities">Capabilities</a></p>

<p align="center">
<a href="https://github.com/TharukRenuja/SpotCore/actions/workflows/ci.yml"><img src="https://github.com/TharukRenuja/SpotCore/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
<a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL_v3-blue" alt="GNU GPL v3"></a>
<img src="https://img.shields.io/badge/Rust-1.89%2B-orange" alt="Rust 1.89 or newer">
</p>

Building a Spotify app? SpotCore brings Spotify services and playback together in one SDK. Free and Premium accounts can use Spotify’s Web Player, and Premium accounts can also opt into Soloist for native playback. SpotCore does not decrypt protected content itself, remove advertising, or bypass account entitlements.

## Start playing

```rust
use spotcore::SpotCore;

let mut spotify = SpotCore::builder()
    .session_cookie(sp_dc)
    .device_name("My Music App")
    .launch()
    .await?;

spotify.play("spotify:track:0aWMVrwxPNYkKmFthzmpRi").await?;
spotify.pause().await?;
spotify.resume().await?;
spotify.next().await?;
```

Use this inside your app's async function; the app supplies `sp_dc`. These lines demonstrate individual controls, rather than a playback sequence to run immediately. Keep the session alive while listening and call `shutdown().await?` when finished. The [quickstart](docs/quickstart.md) covers dependencies, credentials and engine setup.

## A quick note

Unofficial project, not affiliated with or endorsed by Spotify. Playback uses Spotify's own components and respects account entitlements. Licensed under [GNU GPL v3](LICENSE); see [third-party notices](THIRD-PARTY-NOTICES.md).

## What you can build

- **A player for Free and Premium accounts:** Web playback, optional native audio, quality selection, podcast speed, queues and Spotify Connect.
- **A personalized music app:** Home shelves, nine search categories, detailed catalog metadata, playlist suggestions and radio blends.
- **Library and playlist tools:** Save music, follow artists and shows, organize folders, edit playlists and manage covers.
- **Social features:** Profiles, lyrics, canvas, credits, Blends, and Premium Jam hosting with shareable invites.
- **Multi-account applications:** Isolated profiles, application-specific paths and credential-store integration.

## Capabilities

Services are shared across playback backends. Cookie-backed service access does not need a running browser; HTTP controls require an existing Connect receiver.

| Capability | HTTP services / remote device | Web playback | Optional native playback |
| --- | --- | --- | --- |
| Search, catalog, Home and browse | Yes | Shared services | Shared services |
| Library saves, follows and folders | Yes | Shared services | Shared services |
| Playlist reads, edits and covers | Yes, subject to permissions | Shared services | Shared services |
| Profiles, lyrics, canvas and credits | Yes, where available | Shared services | Shared services |
| Local audio | No | Spotify Web Player | Spotify Soloist |
| Play, pause, seek, shuffle and repeat | Existing receiver and its restrictions | Yes, subject to current permissions | Yes, with service routing where needed |
| Queue add / clear / remove | Active receiver, with restrictions | Yes | Unified service controls |
| Queue insert / reorder / skip-to | Unsupported | Browser-assisted controls | Requires an attached Web session |
| Local playback events | No synthetic event stream | Yes | Yes |
| Quality selection | Device settings where supported | Preference up to VeryHigh | Device/account-supported levels |
| Jam host / invite / end | Verified with Premium host | Shared services | Shared services; Soloist is a receiver |
| Jam guest joining | Not supported as a verified feature | Not supported as a verified feature | Not supported as a verified feature |

### Accounts and runtime

| | Free | Premium |
| --- | --- | --- |
| Spotify service access | Yes, subject to region and permissions | Yes, subject to region and permissions |
| Local Web playback | Yes; Spotify ads remain | Yes |
| Optional Soloist playback | SDK does not select it | Opt-in with API key and supported platform |
| Jam hosting and invites | Premium required | Verified |
| Lossless / HiRes preference | No entitlement bypass | Native only where device, account and content permit |

[Playback](docs/playback.md) explains routing and quality. [Engines](docs/engines.md) covers the common launcher, system QtWebEngine, installed-browser fallback and Widevine. Linux runtime paths have local coverage; Windows/macOS builds do not establish real DRM/audio compatibility on every machine.

## Platforms

| Platform | Services and remote controls | Local Web playback | Optional Soloist playback |
| --- | :---: | :---: | :---: |
| Linux x86_64 / ARM64 | ✅ | ✅ | ✅ |
| Linux ARMv7 | Not verified | Not verified | ✅ |
| Windows x86_64 | ✅ | ✅ | ✅ |
| macOS Apple Silicon | ✅ | ✅ | ✅ |
| macOS Intel | ✅ | ✅ | ❌ |
| Android | In development | In development | In development |
| iOS | Not tested | ❌ | ❌ |

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

Use Rust directly, or pick your language: Python, JavaScript/TypeScript, C#, Java, Kotlin, Swift, Dart (experimental), C, C++ or Go. Each binding provides starter methods and access to all 171 current service/playback operations through `invoke()`. Broader operation arguments and results use JSON; they are not fully typed in every language.

We’ve checked all ten bindings locally on Linux, and Node also has Windows/macOS CI coverage. Sampled live-account checks cover Python and Node on Linux. Bindings share the SDK’s runtime requirements and do not bundle playback engines. See the [binding matrix and setup](bindings/README.md).

Build the Rust API reference with `cargo doc -p spotcore --all-features --no-deps`.

## Special thanks

Special thanks to kota-zo and everyone behind [Myspoti](https://github.com/kota-zo/myspoti)! SpotCore builds on their browser bridge and Spotify API mappings. You’ll find the credits and license details in [third-party notices](THIRD-PARTY-NOTICES.md).
