# Engines and DRM

Just fetching data or controlling another device? With a cookie, you don’t need a browser. For local Web audio, Spotify’s Web Player needs Widevine EME and AAC decoding. The SDK does not bundle or rebuild QtWebEngine, and it does not distribute Widevine.

## Package the launcher and choose a backend

| Selection | Backend | Use |
| --- | --- | --- |
| `BrowserEngine::Auto` | Automatic candidates | Prefers Qt for playback, then compatible fallback candidates |
| `BrowserEngine::Qt` | `spotcore-qt-engine` | SDK host using installed system QtWebEngine |
| `BrowserEngine::Chromium` | Installed Chromium-family browser | Playback where DRM and codecs work |
| `BrowserEngine::System` | `spotcore-system-engine` | Experimental browser services adapter; not a Web playback backend |

Every managed browser backend runs through the common `spotcore-engine` launcher. Browser helper processes retain their own executable names. Native playback and externally attached CDP sessions are not wrapped.

```sh
cargo build -p spotcore-engine --release --locked
cmake -S host/qt -B host/qt/build -DCMAKE_BUILD_TYPE=Release
cmake --build host/qt/build -j
```

Install QtWebEngine development packages before building the Qt host. Package `target/release/spotcore-engine` and, when choosing Qt, `host/qt/build/spotcore-qt-engine` beside your app or on PATH. Qt's runtime libraries must also be available on the target system. These small host executables do not contain a complete browser runtime.

The optional system helper builds with `cargo build -p spotcore-system-engine --release --locked`. Linux needs WebKitGTK 4.1/GTK development libraries; Windows uses WebView2 and macOS uses WKWebView. Cookie-backed HTTP is the simpler path for services and avoids loading these adapters.

## Explicit configuration

```rust
use spotcore::{BrowserEngine, SpotCore};

async fn web(sp_dc: String) -> Result<(), spotcore::Error> {
    let spotify = SpotCore::builder()
        .session_cookie(sp_dc)
        .engine(BrowserEngine::Qt)
        .launch().await?;
    let drm = spotify.drm_support().await?;
    println!("Widevine: {}, AAC: {}", drm.widevine, drm.aac);
    spotify.shutdown().await?;
    Ok(())
}
```

`.host_path(path)`, `SPOTCORE_ENGINE` or `SPOTCORE_HOST` selects the backend executable. `SPOTCORE_LAUNCHER` overrides the common launcher. Discovery also searches beside the app, under the data root, on PATH and in development build locations. A legacy Qt host named `spotcore-engine` should be rebuilt/renamed to avoid confusing it with the launcher.

## Widevine

Precedence is `.widevine_path(path)`, `SPOTCORE_WIDEVINE`, a standalone data-root copy, browser installations/profiles, then supported system locations. Invalid explicit paths fail. Discovery considers platform, architecture and numeric version ordering; finding a file is only finding a candidate.

Managed Web startup probes Widevine EME access with AAC. `drm_support()` returns `widevine`, `aac` and an optional `error`. This establishes capability, not successful license acquisition or audible output. Attached browser playback is also checked. A browser accepting DRM in ordinary use is not proof every isolated SDK profile will work.

## Fallback and process lifetime

Initialization records candidate failures and returns `Error::NoCompatibleEngine` if none works. Authentication and invalid configuration are not retried as engine failures. On Linux, Qt/Chromium prefer experimental single-process mode and may retry with helpers; other platforms use supported process models. Single-process Chromium disables sandbox isolation. One PID and lower RAM are not guaranteed.

The launcher supervises the SDK → launcher → backend lifetime chain. Shutdown stops owned process groups/jobs; attached browsers remain open. `WebPlayerOptions::minimal_ui` defaults to optimized startup on recognized player builds; unknown builds retain the normal UI. This changes the UI load, not advertising or audio decryption.

Memory depends on browser build, account state and workload. Browser-hosted Services measurements around 200 MiB and Web playback around 350–400 MiB are observations, not limits; cookie-backed HTTP services do not launch that browser. Linux has local runtime coverage. Windows/macOS build coverage does not replace DRM/audio checks on target machines.

## Platform prerequisites and packaging

| Target | Qt backend | Optional OS adapter | Validation limit |
| --- | --- | --- | --- |
| Linux | Qt 6 Core/Gui/Widgets/Network/WebEngineCore/WebEngineWidgets, C++17 compiler and CMake 3.16+ | WebKitGTK 4.1 and GTK 3 development/runtime libraries | Local engine/DRM/audio tests exist; distribution/browser versions still differ |
| Windows | Matching Qt 6 WebEngine distribution, compiler toolchain and deployed Qt runtime/resources | WebView2 runtime plus the system helper | Build/smoke checks do not establish real account audio/DRM behavior |
| macOS | Matching Qt 6 WebEngine distribution/toolchain and deployed Qt runtime/resources | WKWebView via the system helper | Build/smoke checks do not establish real account audio/DRM behavior |

A Chromium fallback uses an installed browser; shipping the common launcher does not ship Chromium. A system adapter is not a replacement for Widevine Web playback. On Linux, the CI system-helper build installs `libwebkit2gtk-4.1-dev` and `libgtk-3-dev`; package names differ across distributions. Qt packages must include WebEngine, not just base Qt widgets.

A typical app package places the common launcher beside its executable and provides either a Qt backend with its runtime or a supported installed-browser path. Set `SPOTCORE_LAUNCHER` when storing the launcher elsewhere and `.host_path()` when storing a backend elsewhere. Use separate writable app data; do not put mutable profiles in the installation directory. Do not assume copied browser DRM files work across architectures.

The native release resolver currently covers Linux x86_64/aarch64/arm, Windows x86_64 and macOS aarch64. Unsupported targets return a native setup error/fallback; macOS Intel has no configured Soloist release. Native binaries are downloaded separately, not built from `host/qt`.

## Troubleshooting order

1. Check whether the app needs local Web audio or only HTTP services. HTTP services do not need Qt/DRM.
2. Check that the common launcher and the selected backend are separate executables and discoverable.
3. Check browser/Qt runtime libraries, resources, platform plugins and display requirements.
4. Inspect `NoCompatibleEngine { attempts, requirements }`: each `EngineFailure` includes engine, optional path and cause.
5. Probe `drm_support()`; both Widevine and AAC are required. A capability pass still does not prove audible output.
6. Check authentication, active receiver, volume/output and account restrictions before retrying playback.

For diagnosis, `headless(false)`, `no_cuts(true)` and `WebPlayerOptions { minimal_ui: false, ..Default::default() }` can expose normal browser behavior. They are diagnostic settings, not a requirement for production. Use [configuration](configuration.md) for defaults and setter interactions.
