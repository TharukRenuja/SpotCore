# Build with SpotCore

New here? Start with the [quickstart](quickstart.md). Already building? Pick the guide you need below.

| Guide | What it covers |
| --- | --- |
| [Language bindings](bindings.md) | Ten language integrations, operation access, checks and limits |
| [Installation](installation.md) | Source setup, runtime requirements and known limitations |
| [Configuration](configuration.md) | Cargo features, builder defaults and environment variables |
| [Models and pagination](models.md) | Identifiers, typed results, raw data and continuation |
| [Events and lifetime](events.md) | Event handling, capabilities and shutdown |
| [Podcasts and audiobooks](podcasts.md) | Spoken-audio catalog, saved episodes and restrictions |
| [Advanced integration](advanced.md) | Raw API access, operations, custom backends and generated reference |
| [Authentication](authentication.md) | Cookies, renewal, login, account switching and storage |
| [Playback](playback.md) | Controls, queues, events, quality, native audio and podcasts |
| [Engines](engines.md) | Packaging, discovery, Qt, Chromium and Widevine |
| [Discovery](discovery.md) | Search, Home, browse, catalog, podcasts and radio |
| [Library](library.md) | Saves, follows, membership, pins and folders |
| [Playlists](playlists.md) | Metadata, edits, covers, collaborators and hidden songs |
| [Devices](devices.md) | Discovery, transfers and explicit remote commands |
| [Social](social.md) | Profiles, extras, Blends and Jam hosting |
| [Errors](errors.md) | Restrictions, cooldowns and recovery |

## Choose a session

`launch()` means “initialize the SDK session.” The default prepares local playback immediately. For data access and remote controls without a browser, supply a cookie and select `BrowserRole::Services`. See the [services-only example](quickstart.md#services-only-skip-local-playback-setup).

| Mode | Local audio | Setup |
| --- | --- | --- |
| Cookie-backed HTTP services | None; controls existing Connect devices | Cookie + `BrowserRole::Services` |
| Web playback | Spotify Web Player | Common launcher, compatible browser engine, Widevine and AAC |
| Optional native playback | Spotify Soloist | `native` feature, Premium account, API key and downloaded engine |

The unified session routes commands before sending them. It does not replay an ambiguous failed write on another backend. Service domains stay available with either playback backend.

Each snippet is an independent async function unless noted. Pass in the URIs and credentials from your app. Examples that mutate libraries, playlists or playback are intended to run in response to the corresponding user action.

SpotCore is not published on crates.io. Use a local path as shown in the quickstart; see [installation status](installation.md). The default Cargo feature is `web`; add `native` to enable Soloist integration. Enabling that feature alone does not download or launch Soloist.

For individual method signatures, run `cargo doc -p spotcore --all-features --no-deps --open`. Older experiment reports are local, ignored files and are not required by these guides.

For bug reports, development checks and pull requests, see [Contributing](../CONTRIBUTING.md).

## How to use the reference

Each domain guide starts with a workflow and continues with method tables. Inputs use Rust types from the public API; `value` means the entity URI for that method, not a URL or display name. `Value` denotes an upstream JSON response. See [models](models.md) for shared identifiers, units and pagination rules. A method being listed means it is implemented, not that every region/account/backend has passed a live test. Feature-specific restrictions remain beside the methods.

Use the generated Rust reference for the complete field and low-level type signatures. The written guides explain configuration and behavior, while the reference follows the source API. No report directory is required to use them.
