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
    spotify.set_repeat(RepeatMode::Context).await?;
    spotify.set_shuffle_mode(ShuffleMode::Smart).await?;
    let applied = spotify.set_quality(AudioQuality::VeryHigh).await?;
    println!("Quality preference: {applied:?}");
    Ok(())
}
```

`play`, `pause`, `resume`, `toggle`, `stop`, `next`, `previous`, and `set_volume(0.0..=1.0)` provide transport controls. Web `stop` pauses. `play_context(uri, track, index, position)` supports an optional track, zero-based index and starting `Duration`.

## Queue rows

```rust
use spotcore::{QueueItem, SpotCore};

async fn queue(spotify: &mut SpotCore, track_uri: String) -> Result<(), spotcore::Error> {
    spotify.add_to_queue(vec![track_uri]).await?;
    let queue = spotify.queue().await?;
    if let Some(row) = queue.queued.first()
        && let Some(uid) = &row.uid
    {
        spotify.remove_from_queue(vec![QueueItem {
            uri: row.uri.clone(), uid: uid.clone(),
        }]).await?;
    }
    Ok(())
}
```

`Queue` separates `current`, manually `queued`, and context `next_up` items. Preserve exact URI/UID pairs for edits; duplicate track URIs can represent different rows. `clear_queue()` clears user-queued items according to backend semantics.

`insert_into_queue(uris, before)`, `reorder_queue(items, before, after)` and `skip_to_queue(item)` require browser-assisted queue controls. Reordering takes exactly one of `before` or `after`. Cookie-only HTTP returns `Unsupported` for these operations; native audio alone does not supply the page. HTTP supports a single item per queue-add command and restricts state-dependent edits to the active receiver.

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

Use `MediaKind::Episode` and current seek/speed capabilities to show spoken-audio controls. Speed accepts 0.5–3.5; Spotify may further restrict it. Relative skips refresh state and clamp to item bounds without resuming a paused player. Catalog availability varies by region and entitlement.

## State and events

`refresh_state().await?` queries fresh state; `state()` is cached and does not run a clock. Web/HTTP refreshed positions account for elapsed time and speed, stop advancing while paused/buffering and clamp to duration. Optional device/volume fields may be absent.

`next_event().await?` supplies events for Web/native playback. HTTP-only sessions return `Unsupported`; apps choose their own polling policy. On `EventsLagged`, refresh state. A successful command means submitted, not confirmed audible output. No ambiguous write is replayed on another backend.

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

This example requires `spotcore = { path = "../SpotCore", features = ["native"] }`.

With `AccountManager`, a saved native key is tried by default after fresh account verification. `manager.prefer_native(false)` selects Web for subsequent opens without deleting the key; new managers default to native preference.

Preview limitation: standalone native playback has worked locally, but managed account startup has also fallen back to Web after authentication. Partial-event handling and startup waiting were patched; those patches have not resolved every reported case. Inspect `native_fallback_reason()` and retain Web fallback in apps.

The separately cached executable is `spotcore-native` (`.exe` on Windows), with install metadata beside it. Managed downloads refresh after 85 days from recorded download time, independently of SDK releases. This is a freshness policy, not proof of a vendor build's expiry. Stop managed daemons before updating. The Soloist API key is passed as an argument because its interface requires it, so it can be visible in process listings.

`set_quality()` returns an applied preference, not measured stream fidelity. Web clamps Lossless/HiRes to VeryHigh. Native quality depends on device/account eligibility; a setting alone does not prove delivered bitrate. `audio_info()` reports Web `bitrate`, `mime` and `level`; native audio diagnostics return `Unsupported` rather than reporting an inactive Web stream.

Call `shutdown().await?` when finished. It stops owned engines; attached external browsers are left running.

## Facade method reference

Receiver arguments are omitted. This table summarizes behavior and selected return types; use the generated Rust reference for exact signatures.

| Method and inputs | Behavior / result |
| --- | --- |
| `play(uri)` | Load/start a Spotify URI; `Command::Load` |
| `pause()`, `resume()`, `toggle()`, `stop()` | Transport controls; Web Stop maps to pause |
| `next()`, `previous()` | Receiver decides skip eligibility |
| `seek(position)`, `rewind(amount)`, `forward(amount)` | `Duration`; relative skips require current state |
| `set_volume(volume)` / `volume()` | Finite 0.0–1.0 input / reported `f64` result |
| `set_shuffle(bool)` | Ordinary shuffle; does not explicitly request Smart Shuffle |
| `set_shuffle_mode(mode)` | `ShuffleMode::{Off, On, Smart}` |
| `set_repeat(mode)` | `RepeatMode::{Off, Context, Track}` |
| `set_speed(speed)` | Finite 0.5–3.5 input, with receiver restrictions |
| `play_context(uri, track, index, position)` | Optional track URI, zero-based index and `Duration` start; use one track-selection policy |
| `add_to_queue(uris)` | `Vec<String>`; HTTP accepts one URI per command |
| `insert_into_queue(uris, before)` | `Vec<String>`, optional `QueueItem`; None inserts at manual-queue head |
| `reorder_queue(items, before, after)` | `Vec<QueueItem>`, exactly one reference row |
| `skip_to_queue(item)` | Exact `QueueItem`; browser-assisted only |
| `remove_from_queue(items)` / `clear_queue()` | Exact rows / clear operation; preserve context and restrictions where supported |
| `queue()` | `Result<Queue, Error>` |
| `set_quality(quality)` | `AudioQuality` → `Result<AppliedQuality, Error>` |
| `audio_info()` / `drm_support()` | Web stream metadata / DRM+AAC capability; not native stream inspection |
| `refresh_state()` | `Result<Option<PlayerSnapshot>, Error>` |
| `state()` / `current_track()` | Cached optional borrowed snapshot/media |
| `is_playing()` / `position_ms()` | Cached helper values; not continuously advancing clocks |
| `capabilities()` | Current combined `Capabilities` |
| `playback_mode()` | `PlaybackMode` selected by the coordinator |
| `native_fallback_reason()` | Optional borrowed error explaining native fallback |
| `next_event()` | `Result<PlaybackEvent, Error>`; see [events](events.md) |
| `execute(command)` | Unified `Command` dispatch |
| `execute_on(device, command)` / `play_on(device, uri)` | Explicit receiver routing; see [devices](devices.md) |
| `shutdown()` | Consume and stop owned session resources |
