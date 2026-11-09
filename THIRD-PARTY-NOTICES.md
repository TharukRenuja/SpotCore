# Third-party notices

SpotCore is licensed under GPL-3.0-only (see `LICENSE`) and builds on the components below. Its Rust crates are pinned with checksums in `Cargo.lock` and each keeps its own license.

## Myspoti

The browser bridge, GraphQL operation and schema tables, and Spotify API request mappings are derived from Myspoti by kota-zo and contributors, GPL-3.0, modified for SpotCore: https://github.com/kota-zo/myspoti

## Qt and QtWebEngine

The optional host in `host/qt` links the installed Qt 6 Core, Gui, Widgets, Network, WebEngineCore and WebEngineWidgets modules, by The Qt Company and contributors, under LGPL-3.0 or GPL-3.0: https://www.qt.io. QtWebEngine embeds Chromium under BSD-3-Clause and its own third-party licenses. Neither Qt nor QtWebEngine is distributed here; link Qt dynamically if you ship a host binary.

## Widevine

Proprietary and not distributed here. The host accepts a path to a CDM you are licensed to use. Local Spotify Web Player playback requires a compatible CDM and codecs, for both Free and Premium accounts.

## Spotify

Spotify, the Spotify logo and the Spotify Web Player belong to Spotify AB. SpotCore is not affiliated with, endorsed by or supported by Spotify, and uses undocumented endpoints that may change at any time.

## Spotify Soloist

Optional native playback uses Spotify’s Soloist executable, downloaded separately from Spotify’s CDN. Its source and executable are not included in this repository. SpotCore’s license does not grant rights to Spotify’s components; retain the vendor notices supplied with any separately obtained distribution.

## Optional system web engine

`host/system` uses Wry (MIT OR Apache-2.0) and Tao (Apache-2.0) to access
WebView2 and WKWebView. Their source and dependency licenses are available at
https://github.com/tauri-apps/wry and https://github.com/tauri-apps/tao.
The helper does not redistribute WebView2, WebKit, or Widevine runtimes.

## BoltFFI

The optional `spotcore-ffi` crate and generated language bridge use BoltFFI
(MIT), https://github.com/boltffi/boltffi. SpotCore's own code remains GPL-3.0-only.

## NAPI-RS

The optional native Node.js bridge uses napi-rs (MIT),
https://github.com/napi-rs/napi-rs. No npm package is published by this repository.
