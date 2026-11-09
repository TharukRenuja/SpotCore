# Installation

The source version is **0.1.0**. SDK packages and release artifacts are not published. Use a local checkout; a source version is not an installable release or a promise that a matching tag exists.

## Install from source

For an app beside the SpotCore checkout:

```toml
[dependencies]
spotcore = { path = "../SpotCore" }
```

Add `features = ["native"]` to compile optional Soloist support. See the [quickstart](quickstart.md) for a complete application example, or the [binding setup](../bindings/README.md) for other languages. Generated bindings and native libraries must be built locally for the deployment target.

Once a version is published, you can pin its Git revision or tag. Do not depend on a moving development branch for reproducible builds; commit the consumer's Cargo.lock.

## Choose the runtime

- **HTTP services and remote Connect control:** no browser, launcher or Soloist installation is needed. Supply a cookie and select `BrowserRole::Services`.
- **Local Web audio:** install the common `spotcore-engine` launcher plus either the Qt helper and system QtWebEngine, or a compatible installed Chromium-family browser. Widevine and AAC must work in the selected profile.
- **Native audio:** opt in with a Soloist key for a freshly verified Premium account. SpotCore downloads Soloist separately. Web fallback still needs the Web runtime above.

SpotCore is distributed from source. A versioned GitHub release can provide a tag, release notes and GitHub’s source archives; an all-in-one binary bundle isn’t required. Language bindings and the launcher are built separately for the target platform. See [binding setup](../bindings/README.md) and [engine setup](engines.md) for the commands.

Qt, Chromium, Widevine and Soloist aren’t bundled with the SDK. Service-only consumers don’t need the launcher. The OS system helper is a services adapter, not a verified Web playback engine. Development artifact builds are separate packaging checks, not published SDK packages.

## Known limitations

- Managed native startup can fall back to Web despite standalone Soloist working. Inspect `native_fallback_reason()`; successful native playback is not promised for every account/platform.
- Jam hosting/invites were exercised locally with Premium; guest joining remains unsupported as a verified feature.
- Multi-seed Web API recommendations returned 429 in live checks. `radio_blend()` combines separate radios and is not an equivalent server-side endpoint.
- Catalog availability and controls depend on region, tier and device permission. Some advanced domain responses remain JSON.
- Windows/macOS CI builds and synthetic helper checks do not establish real-account DRM or audible playback. Linux observations are not a memory guarantee.
- Local files, offline downloads and EQ are outside the current SDK scope. Ten language bindings are implemented as source integrations; richer operations use JSON, and live/platform coverage varies. See [bindings](bindings.md).
