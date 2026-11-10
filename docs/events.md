# Playback events and session lifetime

Web and native sessions expose `next_event()`. Cookie-only HTTP provides current-state queries but no synthetic playback-event stream. Do not loop on `Unsupported`; choose an app polling policy instead.

```rust
use spotcore::{Error, PlaybackEvent, SpotCore};

async fn events(spotify: &mut SpotCore) -> Result<(), Error> {
    loop {
        match spotify.next_event().await {
            Ok(PlaybackEvent::Snapshot(state)) => {
                println!("{:?}: {} ms", state.state, state.position_ms);
            }
            Ok(PlaybackEvent::TrackUnavailable(uri)) => eprintln!("Unavailable: {uri}"),
            Ok(_) => {},
            Err(Error::EventsLagged(_)) => { spotify.refresh_state().await?; },
            Err(error) => return Err(error),
        }
    }
}
```

An application normally combines event consumption with its command channel and shutdown signal on the async runtime. Do not block the UI thread on SDK futures. `BackendFuture` is not required to be `Send`; do not assume every backend future can be passed to `tokio::spawn`. Keep ownership of the mutable session with one task and communicate through app channels when needed.

| Event | Meaning |
| --- | --- |
| `StateChanged(PlaybackState)` | Idle/Playing/Paused transition |
| `EndOfTrack` | Backend reported the end of an item |
| `TrackUnavailable(String)` | Item could not be played |
| `CapabilitiesChanged(Capabilities)` | Refresh enabled controls |
| `Snapshot(Box<PlayerSnapshot>)` | Backend playback snapshot |
| `QueueChanged(Queue)` | Updated queue data |
| `SessionChanged` | Session change; refresh account/device state as appropriate |

Event order/frequency depends on the backend; not every service operation produces a playback event. `EventsLagged(count)` is an error reporting dropped events. Non-exhaustive event enums need a fallback arm.

Native events cover the owned daemon, not every receiver in the Spotify account.
`playback_state` produces a snapshot, playback changes produce `StateChanged`,
and track changes update and emit a snapshot. Position-sync messages update the
daemon's cached position without emitting a separate UI tick. Authentication
messages produce `SessionChanged`. Device-active messages update the native
activation state; they do not provide a complete account-wide device feed.
The current native adapter does not emit a typed queue-change feed. Keep targeted
queue/remote-device refreshes and refresh after reconnecting or missing events;
native playback alone is not a reason to disable all polling. Advance the display
clock locally between position updates.

## PlayerSnapshot

Snapshots contain state, optional track/context, position/duration/timestamp, buffering, shuffle, optional Smart Shuffle, repeat and capabilities. Optional device id/name/volume can be absent in push/native snapshots. Refreshing Web/HTTP state estimates elapsed position; cached `state()` does not advance on its own. Applications run their own display clock between updates.

## Capabilities

Flags are `load`, `stop`, `resume`, `pause`, `seek`, `next`, `previous`, `volume`, `shuffle`, `smart_shuffle`, `repeat`, `queue`, `queue_edit`, `play_context`, `speed` and `quality`. `supports(&Command)` maps a concrete command to its flag. Ordinary shuffle support does not imply Smart Shuffle support. A shared queue-edit flag does not remove backend-specific restrictions on insert/reorder/skip-to.

Capabilities are a recent snapshot, not authorization reserved for a future command. Refresh before deciding which controls to display and still handle rejection. Explicit targeting checks the receiver separately; local capability flags alone do not establish another receiver’s support.

## Ownership and shutdown

Keep the session alive for playback. `shutdown(self).await` consumes the facade and stops owned browser/native processes. Drop also stops owned runtime resources, but explicit shutdown is preferable for observable failures. Attached external browsers are not closed.

Account switching closes the old owned session before opening another. Engine fallback is initialization behavior, not a replay mechanism for failed commands. After a timeout the operation may already have happened; inspect state before retrying a mutation.
