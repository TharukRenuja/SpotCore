# Installation

**SpotCore source version: 0.1.1. Requires Rust 1.89 or newer.** Build Rust consumers directly from source. Generate language bindings and native libraries for the target OS and architecture.

## Use a source checkout

Clone SpotCore beside your application:

```sh
git clone https://github.com/TharukRenuja/SpotCore.git
```

Add it to your application's `Cargo.toml`:

```toml
[dependencies]
spotcore = { path = "../SpotCore" }
```

Add `features = ["native"]` to compile optional Soloist support. See the [quickstart](quickstart.md) for a complete application example, or [binding setup](../bindings/README.md) for other languages.

## Pin a Git dependency

**Cargo can build a Git dependency without a crates.io publication.** Select a published release tag from the [releases page](https://github.com/TharukRenuja/SpotCore/releases), or pin a full commit with `rev`. For example, after the `v0.1.1` tag is published:

```toml
[dependencies]
spotcore = { git = "https://github.com/TharukRenuja/SpotCore.git", tag = "v0.1.1", features = ["native"] }
```

The version bump in a checkout does not create that tag. **Use a tag that exists and commit your application's `Cargo.lock`.** Cargo builds the tagged source, including its workspace crates. It does not install binaries attached to a GitHub release. Build or install the launcher and selected engine separately.

## Choose the runtime

- **HTTP services and remote Connect control:** no browser, launcher or Soloist installation is needed. Supply a cookie and select `BrowserRole::Services`.
- **Local Web audio:** install the common `spotcore-engine` launcher plus either the Qt helper and system QtWebEngine, or a compatible installed Chromium-family browser. Widevine and AAC must work in the selected profile.
- **Native audio:** opt in with a Soloist key for a freshly verified Premium account. SpotCore downloads Soloist separately. Web fallback still needs the Web runtime above.

Build language bindings and the launcher separately for the target platform. See [binding setup](../bindings/README.md) and [engine setup](engines.md) for the commands.

**Browser runtimes and DRM are separate dependencies.** The SDK does not bundle Qt, Chromium, Widevine or Soloist. Services-only consumers do not need the launcher. The OS system helper is a services adapter, not a replacement for a DRM-capable Web playback engine.

## Known limitations

- Managed native startup can fall back to Web despite standalone Soloist working. Inspect `native_fallback_reason()`. Successful native playback is not promised for every account/platform.
- Handle recommendation rate limits. `radio_blend()` combines separate radios rather than calling an equivalent multi-seed server endpoint.
- Catalog availability and controls depend on region, tier and device permission. Some advanced domain responses remain JSON.
- Windows/macOS CI builds and synthetic helper checks do not establish real-account DRM or audible playback. Linux observations are not a memory guarantee.
- Local files, offline downloads and EQ are outside the current SDK scope. Ten language bindings are implemented as source integrations. Richer operations use JSON, and live/platform coverage varies. See [bindings](bindings.md).
