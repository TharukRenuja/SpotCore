# Advanced integration

Start with the unified `SpotCore` facade. Use these APIs when you need to own transport setup or integrate an existing backend. Low-level calls retain upstream response shapes and are more sensitive to Spotify changes.

## Construct an HTTP Api

```rust
use spotcore::{Api, Paths, SpotCore};

async fn custom_http(cookie: String) -> Result<(), spotcore::Error> {
    let paths = Paths::for_app("my-app")?;
    let api = Api::from_cookie(cookie, &paths)?;
    let spotify = SpotCore::builder().data_dir(paths.root()).launch_http(api).await?;
    println!("{:?}", spotify.users().me().await?.tier);
    spotify.shutdown().await?;
    Ok(())
}
```

`Api::from_cookie(cookie, paths)` can acquire/renew authentication from the cookie. `from_session(WebSession, paths)` accepts supplied session data. It does not acquire a renewable cookie just because an access token exists. Respect expiry and provide renewed authentication yourself. `Api` clones share its session/operation/cooldown state. Use separate instances and paths for separate accounts.

Browser construction uses `Api::new(browser)` or `with_paths(browser, paths)`. `connect(endpoint)` / `connect_with_paths(endpoint, paths)` attach to a local debugger. `browser()` returns `Unsupported` for HTTP-only APIs. `session()` returns current session data. Never log its credentials. `is_signed_in()` checks authentication.

## GraphQL and services

| Api method | Use |
| --- | --- |
| `gql(name, variables)` | Named persisted GraphQL operation |
| `operation_schema(name)` | Optional schema metadata for an operation |
| `operations()` | Clone the current operation table |
| `has_cached_operations()` | Check for a cache file, not proof that every hash is current |
| `refresh_operations()` | Discover current public/player-script hashes, merge and cache. Returns count |
| `replace_operations(operations)` | Validate/replace the active table. Does not replay requests |
| `services()` | Inspect exposed page services. Not available as a general HTTP service index |
| `service(name, method, args)` | Call a page service or an implemented direct HTTP equivalent |
| `call(method, args)` | Browser bridge command, or supported HTTP Connect mapping |
| `request(method, url, json_body)` | Authenticated allowed-origin JSON request |
| `request_raw(method, url, bytes, content_type)` | Raw response with `status` and `body` |

`Operation`, `Operations`, `OPERATIONS_JSON` and `OPERATION_SCHEMA_JSON` are available through `spotcore::web`. `Operations::from_json()` validates a serialized table. Arbitrary authenticated URLs are rejected. This is not a general-purpose credential-forwarding HTTP client. HTTP methods accepted by this layer are GET, POST, PUT, PATCH and DELETE.

A named service method existing in the Web page does not imply an HTTP implementation exists. Unsupported direct service calls return `Unsupported`. Persisted-query recovery is limited to an explicit rejection that indicates the operation did not execute. Ordinary GraphQL failures do not cause writes to be replayed. Keep bundled operations in source. Runtime discovery is recovery, not a reason to omit the initial table.

## Direct backends and generic clients

`Client<B: Backend>` exposes shared control helpers and can be used without the Web facade. `Backend` requires `capabilities()` and `execute(command)`. Its optional `next_event()` defaults to `Unsupported`. State/queue helpers are provided by concrete backends and facade implementations, not by this trait. `BackendFuture` can hold non-Send futures. `Command` is non-exhaustive. Adapters must reject unsupported variants rather than pretending success.

`Spotify<B>` wraps a service `Api` and `Client<B>`, with `new` and `with_host` constructors. `SpotCore::new(api, WebBackend)` / `with_host` build a unified Web session from existing parts. `SessionBackend` coordinates Web/native routing. The builder is the normal way to establish its account/device invariants.

Through `spotcore::web`, use `WebBackend::new(api)`, `Browser`, `BrowserEvent`, `HostLauncher` and `ManagedHost` for explicit browser integration. `HostLauncher::spawn_configured()` is the one-candidate path. Normal facade startup applies selection/fallback policy. These types do not bypass DRM or authentication requirements.

With Cargo `native`, `spotcore::native` exposes `NativeBackend`/`SoloistBackend`, `NativeManager`/`SoloistManager`, process/config/protocol types and typed manager/daemon errors. Direct adapters can launch with an API key, connect to a running Soloist endpoint, wait for authentication and shut down the process. The unified builder additionally enforces verified tier and same-account device selection. Direct backend use makes the app responsible for those checks.

Generate the exact low-level reference with:

```sh
cargo doc --workspace --all-features --no-deps --open
```

The root API reference alone does not generate dependency-crate pages when `--no-deps` is used. Workspace docs include the individual auth/core/web/native crates. For normal app work, use the [category guides](README.md) instead of raw service calls.

## Raw query example

```rust
use spotcore::Api;
use spotcore::web::{Value, json};

async fn raw_track(api: &Api, track_uri: &str) -> Result<Value, spotcore::Error> {
    api.gql("getTrack", json!({
        "uri": track_uri,
        "includeVideoAssociationItems": false,
    })).await
}
```

Prefer `catalog().track_info()` when its typed fields are sufficient. Raw queries return the GraphQL envelope, not just the entity. Partial GraphQL errors fail the call. Do not interpret an error as an empty successful result.

## Native manager and process ownership

`SoloistManager::new(root)` selects an installation root. `release_url()` resolves the current target archive, `is_binary_fresh()` checks recorded age, `ensure_binary().await` downloads/refreshes when necessary, and `acquire_binary().await` also returns an execution lease. Keep that lease while running the managed binary so another process cannot replace it underneath you.

`SoloistConfig::new(binary_path, api_key, paths)` defaults to device name SpotCore, an automatically allocated WebSocket port and native data/cache paths. `SoloistProcess::spawn(config).await` owns a child and exposes its `ws_url`. Call `kill().await` on the process when finished. The API key must not appear in app logs even though the vendor process interface requires a command argument.

`SoloistBackend::launch`, `launch_with`, `launch_as`, `connect` and `connect_named` provide explicit daemon/backend paths. Use their generated signatures rather than guessing WebSocket parameters. Socket connection returns without waiting for authentication. `authentication_state()` distinguishes a live unauthenticated daemon from a disconnected socket. `wait_authenticated()` waits for daemon session readiness, not account-tier eligibility for the unified SDK policy. `NativeConfig`, `NativeProcess` and `NativeManager` are compatibility aliases for the Soloist types. The SDK’s native protocol types describe control messages. They do not expose decoded PCM or encrypted stream contents.

## Browser and host lifecycle reference

`Browser` exposes connection/options, bridge calls, event reception and session epoch information. Browser events are the lower-level bridge stream. They are not interchangeable with the facade’s `PlaybackEvent`. Use `Api` for account services and `WebBackend` for playback mapping instead of parsing bridge traffic unless you are implementing an adapter.

`HostLauncher` offers the same engine/path/profile/DRM/port/cookie/headless/cuts setters as managed startup, plus `defer_navigation` for coordinating cookie seeding. `spawn()` applies startup policy. `candidates()`, `automatic()`, `failure(error)` and `unavailable(attempts)` support candidate diagnostics. `seed_session(host)` seeds a launched session. `login()` opens login. `forget_system_profile(root, profile)` removes a system-adapter profile through the helper rather than treating it as a generic browser directory.

`ManagedHost` exposes `cdp_url()`, `port()`, `is_custom_engine()` and `engine()`. Call `shutdown().await` to await owned-process termination. A host handle is not proof that Spotify is authenticated, media is ready or playback is audible.

## Raw response formats

Raw requests using `application/x-protobuf` negotiate protobuf responses. Other raw requests, including image uploads, retain JSON response negotiation.

## Jam membership

Join as a member following the host with `api.jam().join_on(invite, ListenOn::Host)`. Reading membership never creates a Jam or launches a player. Only an explicit Start requests a new Jam.

| Method | Behavior |
| --- | --- |
| `join_on(invite, ListenOn::Host)` | Join and follow the host's playback |
| `join_on(invite, ListenOn::ThisDevice)` | Join using this instance's ready local receiver; requires Premium |
| `state()` | Read membership through the five-second account cache |
| `refresh()` | Fetch membership again |
| `leave_session(id)` | Leave the selected Jam |
| `end_session(id)` | End the selected Jam as its host |

```rust,ignore
let session = api.jam()
    .join_on(invite, spotcore::ListenOn::Host)
    .await?
    .ok_or(spotcore::Error::Disconnected)?;

api.queues().add(&[track_uri]).await?;
api.jam().leave_session(&session.id).await?;
```

### Receiver selection

Managed native sessions bind their receiver automatically. External player integrations can supply their actual receiver through `bind_local_receiver` or `join_with_receiver`. Joining does not launch a player or transfer playback to guess a receiver.

### Confirmation and stale state

- Join succeeds only after membership is confirmed. Invite information is accepted as a fallback only when it lists the exact account in the exact session.
- An unconfirmed join returns an error without repeating the write.
- Successful Leave and End acknowledgements retire that session ID, preventing delayed reads from restoring it. Repeating either operation accepts an already-missing session.
- An explicit join can revisit a Jam previously left. Passive reads cannot rejoin it.
- Member and permission changes invalidate the membership cache.

Membership reads and lifecycle writes use HTTP Social Connect endpoints even for browser-backed `Api` instances.

## Queue services

Use `api.queues()` for the active Connect receiver or shared Jam. Reads and writes use HTTP services, including when native mode is selected and another device is playing. Queue commands do not activate Soloist or transfer playback.

| Method | Use |
| --- | --- |
| `snapshot()` | Read order, occurrence identities, receiver/Jam scope and edit permissions without waiting for artwork |
| `add(uris)` | Append songs to Playing next |
| `move_items(snapshot, items, before, after, manual)` | Move existing occurrences, optionally changing their manual/context section |
| `remove(snapshot, items)` | Remove exact occurrences |
| `clear_upcoming(snapshot)` | Remove the upcoming occurrences shown in that snapshot |
| `insert(snapshot, uris, before, after, manual)` | Insert new occurrences at a visible slot |
| `replace_manual(uris)` | Replace Playing next while retaining context rows and reusable occurrence identities |

### Safe edits

- Use exact URI/UID pairs from the snapshot. Two occurrences of the same song remain distinct.
- Writes serialize per SDK session and recheck queue scope and permissions. A changed receiver or Jam rejects the edit. Snapshots from another SDK instance are rejected.
- Clear and Remove preserve later additions and other copies of the same song. Already-consumed requested occurrences are ignored.
- Edits preserve ads, hidden continuation rows and previous tracks. Ads and hidden rows are not exposed as editable songs.
- A complete upcoming-track list is required for queue rewrites. An omitted empty previous-track list is accepted; malformed or incomplete state is rejected.
- Uncertain writes are not automatically retried.

Spotify rows with `provider: queue` count as Playing next even when `metadata.is_queued` is absent.

### Existing low-level methods

Prefer `queues()` for new integrations. These existing methods remain available:

| Method | Behavior |
| --- | --- |
| `devices().edit_queue_occurrence(device, item, before, after, manual)` | Pin the active receiver. Omit anchors and membership to remove, or provide an anchor to move |
| `devices().insert_queue_items(device, uris, before, after, manual)` | Pin the active receiver and insert at one anchor, or at the manual-queue head without an anchor. Set `manual` to false for Next up |
| `SocialConnectAPI.reorderQueue` | Accept `sessionId`, `device`, exact `items`, one `before` or `after` anchor, and optional `queued` section membership |

The Jam service shim verifies membership, host receiver and edit permission before sending a bare `set_queue` command through the session. A guest's cluster receiver ID may differ from the host's, so shared edits validate the Jam's host identity and exact queue occurrences instead of requiring those receiver IDs to match.

## Verification coverage

Direct services-only checks with saved accounts verified:

- Guest join, repeated leave and deliberate rejoin.
- Guest reorder without duplication, followed by restoration of the original order.
- Manual addition and removal of the exact new occurrence.
- Host End, with independent membership reads remaining empty immediately and at delayed checks.
- Unchanged playback state and current track throughout those checks.

Native routing is covered by mocks; no native player was launched. Positional insertion, multi-item insertion and clearing are covered by offline checks, but were not exercised by that live probe.
