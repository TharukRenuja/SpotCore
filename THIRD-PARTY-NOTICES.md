# Third-party notices

SpotCore is licensed under **GPL v3** (see `LICENSE`) and builds on the components below. Its Rust crates are pinned with checksums in `Cargo.lock` and each keeps its own license.

## Orpheon (formerly Myspoti)

The browser bridge, GraphQL operation and schema tables, and Spotify API request mappings are derived from **Orpheon (formerly Myspoti)** by **kota-zo and contributors**, and have been modified for SpotCore.

- **Upstream project:** https://github.com/kota-zo/Orpheon
- **Upstream license:** GNU GPL v3, https://github.com/kota-zo/Orpheon/blob/main/LICENSE

Retain the upstream attribution and applicable license notices when redistributing these components. Distribution of the code or modified versions must comply with GPL v3, including its corresponding-source requirements. The browser bridge retains its attribution header in [player-bridge.js](crates/web/resources/player-bridge.js).

## Qt and QtWebEngine

The optional host in `host/qt` uses **Qt 6** by **The Qt Company and contributors**. It links Core, Gui, Widgets, Network, WebEngineCore and WebEngineWidgets.

- **Upstream project:** [Qt](https://www.qt.io/)
- **Licenses:** Qt module terms include LGPL v3, GPL and commercial options. See [Qt licensing](https://doc.qt.io/qt-6/licensing.html) for the installed version.
- **Chromium components:** QtWebEngine includes Chromium and other dependencies under their own licenses. See [QtWebEngine licensing](https://doc.qt.io/qt-6/qtwebengine-licensing.html).
- **Distribution:** Qt and QtWebEngine runtimes are not included in this repository. The host links to the installed libraries.

**Retain the notices for the exact Qt runtime you distribute.** Use dynamic linking for the host and review the applicable module and third-party terms when packaging Qt libraries, resources or plugins.

## Widevine

Local Spotify Web Player playback uses **Widevine**, Google's proprietary content decryption module, when available in the selected engine.

- **Vendor:** Google
- **Product:** [Widevine](https://www.widevine.com/)
- **License:** Proprietary. SpotCore's GPL license does not grant rights to the CDM.
- **Distribution:** No Widevine binary is included in this repository. The host can use a separately installed CDM or an explicitly configured path.

**Use a CDM you are authorized to use or distribute.** Both Free and Premium Web playback need a compatible CDM and audio codecs.

## Spotify

SpotCore uses **Spotify services and the Spotify Web Player**. Spotify, its logo and its player belong to **Spotify AB**.

- **Service provider:** [Spotify](https://www.spotify.com/)
- **Terms:** Spotify's services, content and proprietary components remain subject to their own terms and account permissions.
- **Affiliation:** SpotCore is not affiliated with, endorsed by or supported by Spotify.
- **Integration:** Some service calls use undocumented endpoints that may change without notice.

**SpotCore's license does not grant rights to Spotify's content, trademarks or proprietary software.**

## Spotify Soloist

Optional native playback uses **Spotify Soloist**, downloaded separately from Spotify's CDN.

- **Vendor:** Spotify
- **License:** Proprietary vendor terms. Soloist is not covered by SpotCore's GPL license.
- **Integration:** The optional native backend launches and controls the separately obtained executable.
- **Distribution:** Soloist source and binaries are not included in this repository.

**Retain the vendor notices supplied with any Soloist distribution you are authorized to redistribute.** A separate download does not change the vendor's terms or account requirements.

## Optional system web engine

The helper in `host/system` uses **Wry and Tao** by **the Tauri Programme contributors** to access platform webviews and windows.

- **Wry:** [Source](https://github.com/tauri-apps/wry). Version 0.57.1 declares **Apache-2.0 OR MIT**.
- **Tao:** [Source](https://github.com/tauri-apps/tao). Version 0.37.2 declares **Apache-2.0**.
- **Platform runtimes:** WebView2 on Windows, WKWebView on macOS, and WebKitGTK on Linux. Those runtimes retain their own licenses and notices.
- **Distribution:** The helper does not bundle WebView2, WebKit or Widevine runtimes.

**Preserve the applicable Wry, Tao and dependency license notices when distributing the helper.** Packaging a platform runtime also requires its own notices and terms.

## BoltFFI

The optional `spotcore-ffi` crate and generated language interfaces use **BoltFFI** by **its contributors**.

- **Upstream project:** [BoltFFI](https://github.com/boltffi/boltffi)
- **License:** **MIT**, as declared by the pinned BoltFFI 0.32.1 crate.
- **Integration:** Generates language interfaces for the shared Rust session API.
- **SpotCore code:** Remains **GPL-3.0-only**. The generator's license does not relicense the SDK.

**Retain the applicable MIT license and copyright notices for BoltFFI components included in your distribution.**

## NAPI-RS

The optional `spotcore-node` crate uses **napi-rs** by **the NAPI-RS contributors** for the native Node.js interface.

- **Upstream project:** [NAPI-RS](https://github.com/napi-rs/napi-rs)
- **License:** **MIT**, as declared by the pinned `napi` crate and its derive/build helpers.
- **Integration:** Provides the native Node.js addon and asynchronous calls into SpotCore.
- **Distribution:** Build the native addon for the target platform and package it with the JavaScript wrapper.

**Retain the applicable MIT license and copyright notices for NAPI-RS components included in your distribution.** SpotCore's own code remains **GPL-3.0-only**.

## Other dependencies

Rust dependencies and their versions are recorded in [Cargo.lock](Cargo.lock). Each dependency retains its own license, copyright notices and any bundled third-party notices.

**Use the dependency sources and metadata for the exact build you distribute.** This overview does not replace the license texts or notices that must accompany individual components.
