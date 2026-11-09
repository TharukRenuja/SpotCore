//! UI-independent entry point for SpotCore.
use std::time::Duration;

pub use spotcore_auth::{
    AccessToken, AccountMetadata, AccountProfile, AccountRecord, AccountSecrets, AccountStore,
    AccountTier, CredentialStore, OsCredentialStore, PlaybackMode, ResolvedSession,
    SoloistCredentials, WebCredentials, WebSession,
};
pub use spotcore_core::{
    AudioQuality, Backend, BackendFuture, Capabilities, Command, CursorPage, EngineFailure, Error,
    Image, MediaItem, MediaKind, Page, Paging, Paths, PlaybackEvent, PlaybackState, PlayerSnapshot,
    Queue, QueueItem, RepeatMode, ShuffleMode,
};

#[cfg(feature = "web")]
pub use spotcore_web as web;
#[cfg(feature = "web")]
pub use spotcore_web::host;
#[cfg(feature = "web")]
mod accounts;
#[cfg(feature = "web")]
pub use accounts::AccountManager;
#[cfg(feature = "web")]
mod session;
#[cfg(feature = "web")]
mod spotify;
#[cfg(feature = "web")]
pub use session::{SessionBackend, SpotCore};
#[cfg(feature = "web")]
pub use spotify::{AppliedQuality, SpotCoreBuilder, Spotify};
#[cfg(feature = "web")]
pub use web::{
    AlbumInfo, Api, ArtistInfo, ArtistRef, AudioInfo, BrowseCategory, BrowsePage, BrowserEngine,
    BrowserRole, Copyright, DrmSupport, EpisodeInfo, HiddenSong, HomeFeed, HomeFilter, LibraryItem,
    MediaRef, OwnerRef, PlaylistAccess, PlaylistAdder, PlaylistInfo, SearchKind, Shelf, ShowInfo,
    TimeRange, TrackInfo, WebPlayerOptions,
};

#[cfg(feature = "native")]
pub use spotcore_native as native;
#[cfg(feature = "native")]
pub use spotcore_native as soloist;

/// Owns a playback backend; commands retain the backend's asynchronous errors.
pub struct Client<B> {
    backend: B,
}

impl<B: Backend> Client<B> {
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    pub fn capabilities(&self) -> Capabilities {
        self.backend.capabilities()
    }

    pub async fn execute(&mut self, command: Command) -> Result<(), Error> {
        self.backend.execute(command).await
    }

    /// Targets a Connect device without pulling playback into the local player.
    pub async fn execute_on(
        &mut self,
        device: impl Into<String>,
        command: Command,
    ) -> Result<(), Error> {
        self.execute(Command::On {
            device: device.into(),
            command: Box::new(command),
        })
        .await
    }

    /// Starts a URI on a Connect device; that receiver may become active.
    pub async fn play_on(
        &mut self,
        device: impl Into<String>,
        uri: impl Into<String>,
    ) -> Result<(), Error> {
        self.execute_on(device, Command::Load(uri.into())).await
    }

    pub async fn next_event(&mut self) -> Result<PlaybackEvent, Error> {
        self.backend.next_event().await
    }

    pub fn into_backend(self) -> B {
        self.backend
    }

    /// Loads and plays a track, episode, or playlist URI.
    pub async fn play(&mut self, uri: impl Into<String>) -> Result<(), Error> {
        self.execute(Command::Load(uri.into())).await
    }

    pub async fn pause(&mut self) -> Result<(), Error> {
        self.execute(Command::Pause).await
    }

    pub async fn resume(&mut self) -> Result<(), Error> {
        self.execute(Command::Resume).await
    }

    pub async fn stop(&mut self) -> Result<(), Error> {
        self.execute(Command::Stop).await
    }

    pub async fn seek(&mut self, position: Duration) -> Result<(), Error> {
        self.execute(Command::Seek(position)).await
    }

    pub async fn next(&mut self) -> Result<(), Error> {
        self.execute(Command::Next).await
    }

    pub async fn previous(&mut self) -> Result<(), Error> {
        self.execute(Command::Previous).await
    }

    /// Sets playback volume (0.0 to 1.0).
    pub async fn set_volume(&mut self, volume: f64) -> Result<(), Error> {
        if !volume.is_finite() || !(0.0..=1.0).contains(&volume) {
            return Err(Error::InvalidInput("volume must be 0..=1"));
        }
        self.execute(Command::SetVolume(volume)).await
    }

    pub async fn set_shuffle(&mut self, enabled: bool) -> Result<(), Error> {
        self.execute(Command::SetShuffle(enabled)).await
    }

    pub async fn set_repeat(&mut self, mode: RepeatMode) -> Result<(), Error> {
        self.execute(Command::SetRepeat(mode)).await
    }

    /// Sets off, normal shuffle, or Spotify Smart Shuffle.
    pub async fn set_shuffle_mode(&mut self, mode: ShuffleMode) -> Result<(), Error> {
        self.execute(Command::SetShuffleMode(mode)).await
    }

    pub async fn add_to_queue(&mut self, uris: Vec<String>) -> Result<(), Error> {
        self.execute(Command::AddToQueue(uris)).await
    }

    /// Inserts URIs at the head of the manual queue, or before a queued row.
    pub async fn insert_into_queue(
        &mut self,
        uris: Vec<String>,
        before: Option<QueueItem>,
    ) -> Result<(), Error> {
        self.execute(Command::InsertIntoQueue { uris, before })
            .await
    }

    /// Moves queued rows before or after another queued row.
    pub async fn reorder_queue(
        &mut self,
        items: Vec<QueueItem>,
        before: Option<QueueItem>,
        after: Option<QueueItem>,
    ) -> Result<(), Error> {
        self.execute(Command::ReorderQueue {
            items,
            before,
            after,
        })
        .await
    }

    /// Jumps to a queued or upcoming row.
    pub async fn skip_to_queue(&mut self, item: QueueItem) -> Result<(), Error> {
        self.execute(Command::SkipToQueue(item)).await
    }

    /// Plays a context, optionally starting at a track, index, or position.
    pub async fn play_context(
        &mut self,
        uri: impl Into<String>,
        track: Option<String>,
        index: Option<u32>,
        position: Option<Duration>,
    ) -> Result<(), Error> {
        self.execute(Command::PlayContext {
            uri: uri.into(),
            track,
            index,
            position,
        })
        .await
    }

    /// Clears everything queued after the current item.
    pub async fn clear_queue(&mut self) -> Result<(), Error> {
        self.execute(Command::ClearQueue).await
    }

    pub async fn remove_from_queue(&mut self, items: Vec<QueueItem>) -> Result<(), Error> {
        self.execute(Command::RemoveFromQueue(items)).await
    }

    /// Sets podcast playback speed, 0.5 to 3.5.
    pub async fn set_speed(&mut self, speed: f64) -> Result<(), Error> {
        self.execute(Command::SetSpeed(speed)).await
    }

    /// Sets streaming quality where the backend allows it.
    pub async fn set_quality(&mut self, quality: AudioQuality) -> Result<(), Error> {
        self.execute(Command::SetQuality(quality)).await
    }
}

#[cfg(feature = "web")]
impl Client<crate::web::WebBackend> {
    /// Returns the most recently cached player snapshot, if any.
    pub fn state(&self) -> Option<&PlayerSnapshot> {
        self.backend.state()
    }

    /// Queries the Web player directly for a fresh snapshot.
    pub async fn refresh_state(&mut self) -> Result<Option<PlayerSnapshot>, Error> {
        self.backend.refresh().await
    }

    pub fn is_playing(&self) -> bool {
        self.state()
            .map(|s| s.state == PlaybackState::Playing)
            .unwrap_or(false)
    }

    pub fn current_track(&self) -> Option<&MediaItem> {
        self.state().and_then(|s| s.track.as_ref())
    }

    pub fn position_ms(&self) -> u64 {
        self.state().map(|s| s.position_ms).unwrap_or(0)
    }

    pub async fn queue(&self) -> Result<Queue, Error> {
        self.backend.queue().await
    }

    /// Queries the current volume level (0.0 to 1.0).
    pub async fn volume(&self) -> Result<f64, Error> {
        self.backend.volume().await
    }

    /// Reports what the Web player is actually streaming, after any tier clamp.
    pub async fn audio_info(&self) -> Result<crate::web::AudioInfo, Error> {
        self.backend.audio_info().await
    }

    pub async fn toggle(&mut self) -> Result<(), Error> {
        if self.is_playing() {
            self.pause().await
        } else {
            self.resume().await
        }
    }

    /// Moves playback back by `amount`, clamped to the start.
    pub async fn rewind(&mut self, amount: Duration) -> Result<(), Error> {
        let (position, duration) = self.skip_bounds().await?;
        self.seek(Duration::from_millis(skip_target(
            position,
            -skip_millis(amount),
            duration,
        )))
        .await
    }

    /// Moves playback forward by `amount`, clamped to the item duration.
    pub async fn forward(&mut self, amount: Duration) -> Result<(), Error> {
        let (position, duration) = self.skip_bounds().await?;
        self.seek(Duration::from_millis(skip_target(
            position,
            skip_millis(amount),
            duration,
        )))
        .await
    }

    async fn skip_bounds(&mut self) -> Result<(u64, Option<u64>), Error> {
        self.refresh_state()
            .await?
            .map(|state| (state.position_ms, state.duration_ms))
            .ok_or_else(|| Error::Backend("playback state unavailable".into()))
    }
}

#[cfg(feature = "web")]
fn skip_millis(amount: Duration) -> i64 {
    i64::try_from(amount.as_millis()).unwrap_or(i64::MAX)
}

#[cfg(feature = "web")]
fn skip_target(position_ms: u64, delta_ms: i64, duration_ms: Option<u64>) -> u64 {
    let target = if delta_ms >= 0 {
        position_ms.saturating_add(delta_ms as u64)
    } else {
        position_ms.saturating_sub(delta_ms.unsigned_abs())
    };
    duration_ms
        .filter(|duration| *duration > 0)
        .map_or(target, |duration| target.min(duration))
}

/// Common application-facing types; backend implementations remain under `web` and `native`.
pub mod prelude {
    #[cfg(feature = "web")]
    pub use crate::{AccountManager, SearchKind, SpotCore};
    pub use crate::{
        AudioQuality, Command, Error, Paging, PlaybackEvent, PlaybackMode, RepeatMode, ShuffleMode,
    };
}

#[cfg(all(test, feature = "web"))]
#[allow(dead_code)]
#[path = "../crates/web/tests/common/mod.rs"]
mod browser_test;

#[cfg(all(test, feature = "web"))]
mod skip_tests {
    use super::skip_target;

    #[test]
    fn skips_clamp_to_the_item() {
        assert_eq!(skip_target(30_000, -15_000, Some(120_000)), 15_000);
        assert_eq!(skip_target(5_000, -15_000, Some(120_000)), 0);
        assert_eq!(skip_target(110_000, 30_000, Some(120_000)), 120_000);
        assert_eq!(skip_target(30_000, 30_000, None), 60_000);
        assert_eq!(skip_target(30_000, 30_000, Some(0)), 60_000);
        assert_eq!(skip_target(u64::MAX, 30_000, None), u64::MAX);
        assert_eq!(skip_target(0, i64::MIN, None), 0);
        assert_eq!(skip_target(u64::MAX, -15_000, None), u64::MAX - 15_000);
    }
}
