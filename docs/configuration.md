# Features and configuration

## Cargo features

| Configuration | Public surface | Runtime behavior |
| --- | --- | --- |
| Default (`web`) | `SpotCore`, builder, account manager, Web/HTTP domains, core and auth types | Default launch prepares local Web playback immediately. Cookie-backed Services skips that setup |
| `web` + `native` | Above plus native adapters and builder `.native(key)` | Native is opt-in per session, not selected just by compiling it |
| `default-features = false` | Core/auth types and generic `Client<B>` | No unified Web facade or browser runtime |
| `default-features = false, features = ["native"]` | Core/auth, generic client and native adapters | Direct native integration. No Web-dependent unified facade |

```toml
[dependencies]
spotcore = { path = "../SpotCore", features = ["native"] }
```

`web` enables the optional Web crate and Tokio. `native` enables the optional native crate and Tokio. Neither bundles Qt, Widevine or a Soloist executable. Application runtime features such as Tokio macros and signals are selected by your application. See the [quickstart](quickstart.md).

## What does launch mean?

`launch()` initializes the SDK session. It does not always launch a browser. With a cookie and `BrowserRole::Services`, it uses HTTP without a browser. Default `BrowserRole::Playback` prepares local audio immediately, even if the first operation is a search. Services do not require that player. The default startup policy requests it. Explicit `.native(key)` may start native audio, including when using an HTTP service session.

`BrowserRole` selects the startup mode. A Services session backed only by an existing browser profile can still use a browser. Use cookie-backed Services when you want no browser startup.

## SpotCoreBuilder options

Builder methods return the builder unless noted. Unset options use the defaults below. Configure the session before `launch()`.

| Option | Default | Meaning and interaction |
| --- | --- | --- |
| `new()` / `SpotCore::builder()` | Default configuration | Construct a builder. Does not launch anything |
| `session_cookie(cookie)` | None | App-owned `sp_dc` for HTTP and managed Web. Does not read an environment file itself |
| `engine(BrowserEngine)` | `Auto` | Select automatic, Qt, System or Chromium candidates |
| `browser_role(BrowserRole)` | `Playback` | `Services` plus a cookie avoids browser startup and local audio |
| `device_name(name)` | `SpotCore` | Set managed Web and native Connect names. Use distinct app names to disambiguate receivers |
| `web_options(WebPlayerOptions)` | See below | Configure page startup and Web device naming |
| `native(api_key)` | Disabled | Requires Cargo `native`. Attempt eligible Premium native playback and expose fallback reason |
| `app_id(id)` | No app suffix | Return `Result<Self, Error>`. Use letters, digits, `-` or `_`. Selects a default-root subdirectory |
| `data_dir(root)` | Discovered persistent root | Set profile, caches, accounts and native root together |
| `profile_dir(dir)` | `<root>/profile` | Override only the browser profile, not other SDK storage |
| `host_path(path)` | Discovery | Explicit backend executable. This is not the common launcher |
| `widevine_path(path)` | Discovery | Explicit CDM library. Invalid explicit paths fail |
| `cdp_port(port)` | Ephemeral port | Fixed debugging port when needed. Conflicts fail rather than attaching arbitrarily |
| `no_cuts(true)` | `false` | Debugging: disable resource cuts and set `minimal_ui` false. Configure this before final Web options if overriding both |
| `headless(bool)` | `true` | Hide supported browser windows. Does not guarantee no display server is needed |
| `connect_to(endpoint)` | None | Attach instead of spawning. Keeps external browser ownership/credentials |
| `launch().await` | Not applicable | Start the SDK session. Default Playback prepares a local player, while cookie-backed Services skips Web startup |
| `launch_http(api).await` | Not applicable | Build from an existing browser-free Api. Does not create a local Web receiver |
| `login().await` | Not applicable | Open optional interactive login using the configured launcher/profile |

**Builder order matters for overlapping setters:** `app_id` and `data_dir` both set the root. The later one wins. `device_name` updates both the native name and Web options. A later `web_options` replaces Web options only. Prefer one naming policy and set it last.

`WebPlayerOptions` has two fields: `minimal_ui: true` and `device_name: Some("SpotCore")` by default. `minimal_ui` attempts the UI bypass only on recognized player builds. `device_name: None` keeps Spotify’s own Web label. Existing-tab attachment does not implicitly reload/rename the tab like managed startup.

```rust
use spotcore::{BrowserRole, SpotCore, WebPlayerOptions};

async fn configure(cookie: String) -> Result<(), spotcore::Error> {
    let spotify = SpotCore::builder()
        .app_id("my-dashboard")?
        .session_cookie(cookie)
        .browser_role(BrowserRole::Services)
        .web_options(WebPlayerOptions {
            minimal_ui: true,
            device_name: Some("My Dashboard".into()),
        })
        .launch().await?;
    spotify.shutdown().await?;
    Ok(())
}
```

## Environment overrides

These are SDK configuration overrides, not credentials to publish. Prefer explicit builder configuration for app-controlled paths.

| Variable | Purpose |
| --- | --- |
| `SPOTCORE_DATA_DIR` | Override the discovered persistent root when no explicit data root is supplied |
| `SPOTCORE_LAUNCHER` | Common `spotcore-engine` supervisor executable |
| `SPOTCORE_ENGINE`, `SPOTCORE_HOST` | Backend executable override. Engine name checked before legacy host name |
| `SPOTCORE_BROWSER` | Installed Chromium-family browser executable override |
| `SPOTCORE_SYSTEM_ENGINE` | OS-adapter helper executable override |
| `SPOTCORE_WIDEVINE` | CDM library override after an explicit builder path |

`SPOTCORE_COOKIE_FILE` is read by repository examples, not automatically by the SDK. `SPOTCORE_SP_DC` and `SPOTCORE_DEFER_PAGE` are internal host handoff variables, not the public application authentication interface. Supply credentials through `session_cookie()`.

Explicit backend selection narrows fallback. It is not the same as automatic engine selection. See [engines](engines.md) for search locations, CDM precedence, process behavior and setup.

## Paths reference

`Paths::discover()` uses `SPOTCORE_DATA_DIR` or a persistent OS location and returns an error instead of silently choosing `/tmp`. `Paths::platform()` ignores that override. `Paths::new(root)` stores an explicit root. `resolve()` validates and anchors relative paths. `validate()` rejects an empty root. `prepare()` creates it with owner-only creation permissions on Unix. `Paths::default()` and `platform_root()` can panic if no persistent location exists. Prefer fallible discovery in apps.

| Accessor | Layout |
| --- | --- |
| `root()` | Application data root |
| `accounts()` | `<root>/accounts` |
| `profile()` | `<root>/profile` |
| `cache()` | `<root>/cache` |
| `native()` | `<root>/native` |
| `native_data()` | `<root>/native/data` |
| `native_cache()` | `<root>/native/data/cache` |

`for_app(id)` appends a validated app ID under the discovered root. AccountManager creates additional isolated account roots. Paths describe layout. Constructing a `Paths` value does not authenticate, install an engine or create every directory.
