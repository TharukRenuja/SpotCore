# Playback and queues

[Controls](#facade-method-reference) · [Events](events.md) · [Models](models.md)

Use one `SpotCore` session for Web playback, optional native audio and remote controls. Before enabling controls in your UI, refresh the state and check its capabilities. Spotify’s account, context, ad and device restrictions still apply.

```rust
use spotcore::{AudioQuality, RepeatMode, ShuffleMode, SpotCore};
use std::time::Duration;

async fn controls(spotify: &mut SpotCore) -> Result<(), spotcore::Error> {
    spotify.refresh_state().await?;
    if spotify.capabilities().seek {
        spotify.seek(Duration::from_secs(45)).await?;
    }
    if spotify.capabilities().repeat {
        spotify.set_repeat(RepeatMode::Context).await?;
    }
    if spotify.capabilities().smart_shuffle {
        spotify.set_shuffle_mode(ShuffleMode::Smart).await?;
    }
    if spotify.capabilities().quality {
        let applied = spotify.set_quality(AudioQuality::VeryHigh).await?;
        println!("Quality preference: {applied:?}");
    }
    Ok(())
}
```

`play`, `pause`, `resume`, `toggle`, `stop`, `next`, `previous`, and `set_volume(0.0..=1.0)` provide transport controls. Web `stop` pauses. `play_context(uri, track, index, position)` supports an optional track, zero-based index and starting `Duration`.

## Queue rows

```rust
use spotcore::SpotCore;

async fn remove_queue_row(
    spotify: &SpotCore,
    index: usize,
    expected_uri: &str,
) -> Result<(), spotcore::Error> {
    let queues = spotify.api.queues();
    let snapshot = queues.snapshot().await?;
    let item = snapshot.item_at(index, expected_uri)?;
    queues.remove(&snapshot, &[item]).await?;
    Ok(())
}
```

`Queue` separates `current`, manually `queued`, and context `next_up` items. Preserve exact URI/UID pairs: repeated track URIs can represent different rows. `snapshot.item_at(index, expected_uri)` addresses the upcoming rows in manual-then-context order and rejects a mismatched URI.

Use `api.queues()` for edits against a `QueueSnapshot`. Its writes check the current receiver/Jam scope and permissions before submission. `insert_into_queue(uris, before)` and `reorder_queue(items, before, after)` also use HTTP Connect. Reordering requires exactly one exact `before` or `after` anchor; missing, ambiguous and stale anchors are rejected.

`skip_to_queue(item)` and `queues().skip_to(&snapshot, &item)` use the HTTP Connect skip command. They require an exact visible upcoming occurrence and skip permission; they do not rebuild the queue or repeatedly send Next. Queue commands do not transfer playback. `devices().replace_queued(device, uris)` replaces the manual section while preserving reusable row UIDs and context entries. See [scoped queue operations](advanced.md#queue-services) for the full API.

## Podcasts and audiobooks

```rust
use spotcore::SpotCore;
use std::time::Duration;

async fn spoken_audio(spotify: &mut SpotCore) -> Result<(), spotcore::Error> {
    spotify.set_speed(1.25).await?;
    spotify.rewind(Duration::from_secs(15)).await?;
    spotify.forward(Duration::from_secs(30)).await?;
    Ok(())
}
```

Use `MediaKind::Episode` and current seek/speed capabilities to show spoken-audio controls. Speed accepts 0.5–3.5. Spotify may further restrict it. Relative skips refresh state and clamp to item bounds without resuming a paused player. Catalog availability varies by region and entitlement.

## State and events

Use `PlayerSnapshot::playback_speed()` for the validated optional rate when interpolating progress. Native snapshots also retain Soloist's reported rate in `raw["speed"]`. Speed controls require the current capability and use `set_speed()` through Connect.

`refresh_state().await?` queries fresh state. `state()` is cached and does not run a clock. Web/HTTP refreshed positions account for elapsed time and speed, stop advancing while paused/buffering and clamp to duration. Optional device/volume fields may be absent.

`next_event().await?` supplies events for Web/native playback. HTTP-only sessions return `Unsupported`. Apps choose their own polling policy. On `EventsLagged`, refresh state. **Command completion confirms submission.** Observe state changes to confirm the result, and check the audio output separately. No ambiguous write is replayed on another backend.

## Native audio and quality

Enable the `native` Cargo feature, then opt in with `.native(api_key)` on the builder. Premium eligibility, platform support, authentication and same-account Connect-device matching are checked at launch. Free/unknown tiers do not trigger native downloads. `native_fallback_reason()` explains why Web was selected.

```rust
use spotcore::SpotCore;

async fn native(sp_dc: String, api_key: String) -> Result<(), spotcore::Error> {
    let spotify = SpotCore::builder()
        .session_cookie(sp_dc)
        .device_name("My Music App")
        .native(api_key)
        .launch().await?;
    if let Some(reason) = spotify.native_fallback_reason() {
        eprintln!("Using Web playback: {reason}");
    }
    spotify.shutdown().await?;
    Ok(())
}
```

Enable `features = ["native"]` on the pinned SpotCore dependency for this example.

With `AccountManager`, a saved native key is tried by default after fresh account verification. `manager.prefer_native(false)` selects Web for subsequent opens without deleting the key. New managers default to native preference.

Managed native startup first checks the daemon's authentication state. If it is not signed in, the SDK looks for an unambiguous matching device in the current account's Connect list and submits one paused transfer to activate it. A transfer response alone is not success: the daemon must confirm `logged_in`, followed by same-account device verification. Existing authenticated daemons skip this activation. A paused transfer can move the account away from its current receiver.

A device absent from the account list cannot be authenticated by guessing a transfer target. **Pairing requires explicit opt-in** through `native_pairing(true)` on `SpotCoreBuilder` or `AccountManager`; the default is false. With that option enabled, failed activation can restart the daemon once in Soloist's pairing mode. The official Spotify app provisions the device, and the session is retained in the data directory. Ordinary restores do not enter pairing mode. The pairing attempt is made at most once per process, so an unpaired device reports `NativeAuthenticationRequired` instead of stalling every switch. A disconnected daemon is reported as `Disconnected`, and a timed-out discovery or transfer request as `NativeStartupTimeout`. Transfers are never automatically replayed. If Web fallback also fails, `NativeStartupFailed` retains both errors instead of hiding the native cause behind DRM/engine errors.

The separately cached executable is `spotcore-native` (`.exe` on Windows), with install metadata beside it. Managed downloads refresh after 85 days from recorded download time, independently of SDK releases. This is a freshness policy, not proof of a vendor build's expiry. Stop managed daemons before updating. The Soloist API key is passed as an argument because its interface requires it, so it can be visible in process listings.

`set_quality()` returns an applied preference, not measured stream fidelity. Web clamps Lossless/HiRes to VeryHigh. Native quality depends on device/account eligibility. A setting alone does not prove delivered bitrate. `audio_info()` reports Web `bitrate`, `mime` and `level`. Native audio diagnostics return `Unsupported` rather than reporting an inactive Web stream.

### Native control boundaries

Spotify’s desktop application uses its local Core preferences for streaming quality and EQ, and its scrobble service for Private Session. These are separate from ordinary Connect playback commands. The presence of a preference key or quality enum does not establish that an external receiver exposes that control.

The current Soloist command interface used by this SDK exposes playback, volume, shuffle, repeat and queue operations. It does not expose the native preference or scrobble services needed to implement Spotify’s six-band EQ or Private Session. SpotCore does not provide a native EQ API. `users().set_private_session()` requires a compatible browser service; it is not a native receiver control. Do not represent a browser-side change as a confirmed native Private Session.

Native quality methods use Spotify’s device-settings HTTP service, not Soloist preference commands. A successful settings write and readback confirms a server-side preference only. The API currently has no complete verified list of selectable native quality levels. Device lossless flags do not establish every permitted quality, and account tier or country alone is insufficient. Applications should not advertise native levels as available solely from the `AudioQuality` enum. Changes must target the owned receiver and remain unconfirmed if the endpoint cannot read them back. There is no native delivered-stream bitrate/codec report in the current interface.

Call `shutdown().await?` when finished. It stops owned engines. Attached external browsers are left running.

## Facade method reference

Receiver arguments are omitted. This table summarizes behavior and selected return types. Use the generated Rust reference for exact signatures.

| Method and inputs | Behavior / result |
| --- | --- |
| `wait_ready(timeout)` | Wait for the owned receiver without activation or transfer |
| `play_spoken(uri, position, device)` | Select an episode/chapter and restore a nonzero position after confirming Native selection |
| `play_audiobook(value, chapter, position, device)` | Apply account policy and validate chapter membership/access; a resume offset requires an explicit chapter |
| `quality_report()` | Distinguish Web stream reporting from a native device preference |
| `play(uri)` | Load/start a Spotify URI. `Command::Load` |
| `pause()`, `resume()`, `toggle()`, `stop()` | Transport controls. Web Stop maps to pause |
| `next()`, `previous()` | Receiver decides skip eligibility |
| `seek(position)`, `rewind(amount)`, `forward(amount)` | `Duration`. Relative skips require current state |
| `set_volume(volume)` / `volume()` | Finite 0.0–1.0 input / reported `f64` result |
| `set_shuffle(bool)` | Ordinary shuffle. Does not explicitly request Smart Shuffle |
| `set_shuffle_mode(mode)` | `ShuffleMode::{Off, On, Smart}` |
| `set_repeat(mode)` | `RepeatMode::{Off, Context, Track}` |
| `set_speed(speed)` | Finite 0.5–3.5 input, with receiver restrictions |
| `play_context(uri, track, index, position)` | Optional track URI, zero-based index and `Duration` start. Use one track-selection policy |
| `add_to_queue(uris)` | `Vec<String>`. One or more URIs through queue services; partial completion is possible |
| `insert_into_queue(uris, before)` | `Vec<String>`, optional `QueueItem`. None inserts at manual-queue head |
| `reorder_queue(items, before, after)` | `Vec<QueueItem>`, exactly one reference row |
| `skip_to_queue(item)` | Exact visible `QueueItem`, checked through HTTP Connect |
| `remove_from_queue(items)` / `clear_queue()` | Exact rows / clear operation. Preserve context and restrictions where supported |
| `queue()` | `Result<Queue, Error>` |
| `set_quality(quality)` | `AudioQuality` → `Result<AppliedQuality, Error>` |
| `audio_info()` / `drm_support()` | Web stream metadata / DRM+AAC capability. Not native stream inspection |
| `refresh_state()` | `Result<Option<PlayerSnapshot>, Error>` |
| `state()` / `current_track()` | Cached optional borrowed snapshot/media |
| `is_playing()` / `position_ms()` | Cached helper values. Not continuously advancing clocks |
| `capabilities()` | Current combined `Capabilities` |
| `playback_mode()` | `PlaybackMode` selected by the coordinator |
| `native_fallback_reason()` | Optional borrowed error explaining native fallback |
| `next_event()` | `Result<PlaybackEvent, Error>`. See [events](events.md) |
| `execute(command)` | Unified `Command` dispatch |
| `execute_on(device, command)` / `play_on(device, uri)` | Explicit receiver routing. See [devices](devices.md) |
| `shutdown()` | Consume and stop owned session resources |

Native repeat uses the web-assisted Connect control so context and track repeat flags change together. Native audio remains on Soloist; changing repeat does not require browser playback.

Native snapshots read duration from Soloist's decorated playback metadata and preserve context and volume. Once startup verifies a receiver in the account, the session binds its Connect device ID for subsequent activation and snapshots. Initial registration still requires an unambiguous device name in the account.

Applications can set `application_name("My Music App")` on `AccountManager`, `SpotCoreBuilder` or `HostLauncher` to brand managed host processes and login windows. Rebuild the launcher and selected host to apply that branding. Executable filenames and profile storage identifiers remain stable.

## Queue metadata and recovery

`api.queues().snapshot()` returns authoritative order, occurrence identities, receiver/Jam scope and permissions without waiting for metadata enrichment. `devices().queue_snapshot()` returns queue data and fills only metadata already cached. `devices().queue()` additionally performs a best-effort recovery batch.

The shared per-API metadata cache holds at most 256 items for ten minutes. Negative cache entries expire after 30 seconds. A best-effort hydration pass requests at most four unique items with four concurrent reads and an eight-second timeout per item. Individual metadata failures preserve the queue rows. For complete enrichment, follow the explicit [metadata recovery](service-recovery.md) continuation API rather than repeatedly reading the queue.

`devices().queue_snapshot()` returns queue state and context identity without waiting for catalog enrichment. `devices().queue()` retains optional enrichment. Native `queue_changed`, `context_changed` and `options_changed` events invalidate queue state. Context and option changes also request a fresh playback snapshot.

## Local device readiness

`SpotCore::local_device()` resolves the session's owned player. Web sessions query the playback backend's browser device list, independently of the public HTTP services API. Native sessions use their bound device ID, with unambiguous configured-name discovery only until an ID is available. Missing registration returns None. Services-only sessions return Unsupported.

`activate_local()` transfers to that owned player with playback disabled and waits for activation. It does not start a track. Call it for an explicit local Play or when startup has confirmed no other device is active. Clients should retry read-only discovery while registration is pending, keep playback controls disabled on failure, and avoid equating authenticated services with an activated playback device.

Web player state reads return immediately with cached shuffle capabilities. Context-specific shuffle availability refreshes in the background and publishes a new snapshot when ready. It never delays core playback state.

`Api::local_playback_state()` reads the local Web receiver independently of a mutable playback command. HTTP-only sessions return Unsupported. Use `devices().playback_state()` for the account's active Connect receiver instead.

`SpotCore::play_items(context, uris, position)` starts an ordered list on this SDK's local receiver. `devices().play_items(target, context, uris, position)` targets an explicit Connect receiver. Items are sent in one inline context page, not as Add to queue commands, so another collection replaces this context's tail while genuine manual queue entries remain intact. Lists accept 1 to 1000 items.
