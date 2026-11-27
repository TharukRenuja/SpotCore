# Engines and DRM

Just fetching data or controlling another device? With a cookie, you don’t need a browser. For local Web audio, Spotify’s Web Player needs Widevine EME and AAC decoding. The SDK does not bundle or rebuild QtWebEngine, and it does not distribute Widevine.

## Package the launcher and choose a backend

| Selection | Backend | Use |
| --- | --- | --- |
| `BrowserEngine::Auto` | Automatic candidates | Prefers Qt for playback, then compatible fallback candidates |
| `BrowserEngine::Qt` | `spotcore-qt-engine` | SDK host using installed system QtWebEngine |
| `BrowserEngine::Chromium` | Installed Chromium-family browser | Playback where DRM and codecs work |
| `BrowserEngine::System` | `spotcore-system-engine` | Experimental browser services adapter. Not a Web playback backend |

Every managed browser backend runs through the common process supervisor, either the standalone `spotcore-engine` launcher or an application that embeds it. Browser helper processes retain their own executable names. Native playback and externally attached CDP sessions are not wrapped.

```sh
cargo build -p spotcore-engine --release --locked
cmake -S host/qt -B host/qt/build -DCMAKE_BUILD_TYPE=Release
cmake --build host/qt/build -j
```

Install QtWebEngine development packages before building the Qt host. Package `target/release/spotcore-engine` and, when choosing Qt, `host/qt/build/spotcore-qt-engine` beside your app or on PATH. Qt's runtime libraries must also be available on the target system. These small host executables do not contain a complete browser runtime.

## Embed the launcher

Applications can compile the existing supervisor into their own executable. Run these hooks at the beginning of `main`, before argument parsing, a runtime, single-instance checks or GUI initialization:

```rust
if let Some(code) = spotcore::host::run_embedded_launcher() {
    std::process::exit(i32::from(code));
}
spotcore::host::enable_embedded_launcher()?;
```

The opt-in applies to managed browser sessions and interactive login throughout the process. Each helper launch executes the application with the internal `--spotcore-engine` argument. The same supervisor handles parent lifetime, process groups/jobs, literal arguments and exit status. Native playback bypasses it. `SPOTCORE_LAUNCHER` overrides the embedded choice when explicitly supplied.

Embedding requires no extraction, runtime file writes or separate launcher build. The browser backend, Qt runtime and DRM requirements are unchanged. Applications that do not opt in retain standalone-launcher discovery.

The optional system helper builds with `cargo build -p spotcore-system-engine --release --locked`. Linux needs WebKitGTK 4.1/GTK development libraries. Windows uses WebView2 and macOS uses WKWebView. Cookie-backed HTTP is the simpler path for services and avoids loading these adapters.

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

Precedence is `.widevine_path(path)`, `SPOTCORE_WIDEVINE`, a standalone data-root copy, browser installations/profiles, then supported system locations. Invalid explicit paths fail. Discovery considers platform, architecture and numeric version ordering. Finding a file is only finding a candidate.

Managed Web startup probes Widevine EME access with AAC. `drm_support()` returns `widevine`, `aac` and an optional `error`. This establishes capability, not successful license acquisition or audible output. Attached browser playback is also checked. A browser accepting DRM in ordinary use is not proof every isolated SDK profile will work.

## Fallback and process lifetime

Initialization records candidate failures and returns `Error::NoCompatibleEngine` if none works. Authentication and invalid configuration are not retried as engine failures. On Linux, Qt/Chromium prefer experimental single-process mode and may retry with helpers. Other platforms use supported process models. Single-process Chromium disables sandbox isolation. One PID and lower RAM are not guaranteed.

The launcher supervises the SDK → launcher → backend lifetime chain. Shutdown stops owned process groups/jobs. Attached browsers remain open. `WebPlayerOptions::minimal_ui` defaults to optimized startup on recognized player builds. Unknown builds retain the normal UI. This changes the UI load, not advertising or audio decryption.

**Measure memory and verify DRM/audio on each target platform.** Browser memory depends on its build, account state and workload. Cookie-backed HTTP services avoid browser startup. A successful build does not establish DRM or audible playback compatibility.

## Platform prerequisites and packaging

| Target | Qt backend | Optional OS adapter | Deployment check |
| --- | --- | --- | --- |
| Linux | Qt 6 Core/Gui/Widgets/Network/WebEngineCore/WebEngineWidgets, C++17 compiler and CMake 3.16+ | WebKitGTK 4.1 and GTK 3 development/runtime libraries | Verify distribution libraries, display access, DRM and audio output |
| Windows | Matching Qt 6 WebEngine distribution, compiler toolchain and deployed Qt runtime/resources | WebView2 runtime plus the system helper | Verify deployed runtime resources, DRM and audio output |
| macOS | Matching Qt 6 WebEngine distribution/toolchain and deployed Qt runtime/resources | WKWebView via the system helper | Verify deployed runtime resources, DRM and audio output |

A Chromium fallback uses an installed browser. Shipping the common launcher does not ship Chromium. A system adapter is not a replacement for Widevine Web playback. On Linux, the CI system-helper build installs `libwebkit2gtk-4.1-dev` and `libgtk-3-dev`. Package names differ across distributions. Qt packages must include WebEngine, not just base Qt widgets.

A typical app package places the common launcher beside its executable and provides either a Qt backend with its runtime or a supported installed-browser path. Set `SPOTCORE_LAUNCHER` when storing the launcher elsewhere and `.host_path()` when storing a backend elsewhere. Use separate writable app data. Do not put mutable profiles in the installation directory. Do not assume copied browser DRM files work across architectures.

The native release resolver currently covers Linux x86_64/aarch64/arm, Windows x86_64 and macOS aarch64. Unsupported targets return a native setup error/fallback. MacOS Intel has no configured Soloist release. Native binaries are downloaded separately, not built from `host/qt`.

## Troubleshooting order

1. Check whether the app needs local Web audio or only HTTP services. HTTP services do not need Qt/DRM.
2. Check that the common launcher and the selected backend are separate executables and discoverable.
3. Check browser/Qt runtime libraries, resources, platform plugins and display requirements.
4. Inspect `NoCompatibleEngine { attempts, requirements }`: each `EngineFailure` includes engine, optional path and cause.
5. Probe `drm_support()`. Both Widevine and AAC are required. A capability pass still does not prove audible output.
6. Check authentication, active receiver, volume/output and account restrictions before retrying playback.

For diagnosis, `headless(false)`, `no_cuts(true)` and `WebPlayerOptions { minimal_ui: false, ..Default::default() }` can expose normal browser behavior. They are diagnostic settings, not a requirement for production. Use [configuration](configuration.md) for defaults and setter interactions.

### Saved HTTP startup

`Api::from_credentials` can reuse an unexpired protected access token when the saved credentials also include its client ID and app version. Cookie renewal remains owned by the SDK. Export `Api::web_credentials()` only to protected credential storage. Older cookie-only records remain readable.

Cookie renewal caches the public token-bootstrap recipe, client version and build version in `cache/cookie-bootstrap.json` under the API data root. Records are bounded to 8 KiB, expire after 24 hours, and are rediscovered if Spotify rejects the recipe. The file contains no user cookies or access tokens. Writes are atomic and run on the blocking pool. Missing or invalid caches use discovery.

`AccountManager::switch_to_with_services` accepts an early HTTP API clone, reuses its renewal/client-token state, and still verifies account identity before selecting it. Read-only GraphQL queries can retry once after a rejected token; mutations are not automatically replayed. Late 401 responses do not invalidate a newer token obtained by another request.

## First Native pairing

Explicit Native setup allows up to three minutes to pair in Spotify’s mobile or desktop app, then up to one minute to authenticate the restarted receiver. A pairing-process error is followed by verification of the saved session rather than treated as conclusive on its own. Local authentication events can complete verification while an HTTP discovery or activation request is pending. Activation is never replayed, and the receiver must still be found unambiguously in the signed-in account before Native playback is accepted. Ordinary account restoration never starts interactive pairing.
