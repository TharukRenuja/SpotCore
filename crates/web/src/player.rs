use crate::{
    Api, BrowserEvent,
    mapping::{queue, snapshot},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use spotcore_core::{
    AudioQuality, Backend, BackendFuture, Capabilities, Command, Error, PlaybackEvent,
    PlayerSnapshot, Queue, QueueItem, ShuffleMode,
};
use tokio::sync::broadcast;

fn queue_item(item: &QueueItem) -> serde_json::Value {
    json!({"uri":item.uri,"uid":item.uid})
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DrmSupport {
    pub widevine: bool,
    pub aac: bool,
    pub error: Option<String>,
}

impl DrmSupport {
    pub fn require_playback(&self) -> Result<(), Error> {
        if self.widevine && self.aac {
            return Ok(());
        }
        Err(Error::DrmUnavailable(self.error.clone().unwrap_or_else(
            || {
                if self.widevine {
                    "AAC decoding is unavailable".into()
                } else {
                    "Widevine EME access is unavailable".into()
                }
            },
        )))
    }
}

/// What the Web Player reports it is streaming.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioInfo {
    /// Bits per second, 0 when unknown.
    pub bitrate: u64,
    /// MIME type and codecs.
    pub mime: String,
    /// The level the page holds, e.g. `very_high`.
    pub level: String,
}

/// Whether this browser may supply local audio or only Spotify services.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BrowserRole {
    #[default]
    Playback,
    Services,
}

pub struct WebBackend {
    api: Api,
    events: Option<broadcast::Receiver<BrowserEvent>>,
    state: Option<PlayerSnapshot>,
    epoch: u64,
    quality: AudioQuality,
    role: BrowserRole,
}
impl WebBackend {
    pub async fn new(api: Api) -> Result<Self, Error> {
        Self::with_role(api, BrowserRole::Playback).await
    }

    pub async fn with_role(api: Api, role: BrowserRole) -> Result<Self, Error> {
        let (events, epoch) = match api.browser() {
            Ok(browser) => (Some(browser.subscribe()), browser.session_epoch()),
            Err(_) if role == BrowserRole::Services => (None, 0),
            Err(error) => return Err(error),
        };
        let mut backend = Self {
            api,
            events,
            state: None,
            epoch,
            quality: AudioQuality::Auto,
            role,
        };
        if role == BrowserRole::Playback {
            backend.refresh().await?;
        }
        Ok(backend)
    }
    pub fn role(&self) -> BrowserRole {
        self.role
    }

    pub fn api(&self) -> &Api {
        &self.api
    }
    pub async fn drm_support(&self) -> Result<DrmSupport, Error> {
        serde_json::from_value(self.api.call("player.drmSupport", json!({})).await?)
            .map_err(|_| Error::Backend("invalid DRM support response".into()))
    }
    pub fn state(&self) -> Option<&PlayerSnapshot> {
        if self.api.browser().is_err() {
            return self.state.as_ref();
        }
        self.state.as_ref().filter(|_| {
            self.api
                .browser()
                .is_ok_and(|browser| self.epoch == browser.session_epoch() && browser.is_ready())
        })
    }
    pub async fn refresh(&mut self) -> Result<Option<PlayerSnapshot>, Error> {
        if self.api.browser().is_err() {
            let cluster = crate::direct::cluster(&self.api).await?;
            self.state = snapshot(&crate::connect::snapshot(&cluster))?;
            if let Some(state) = &mut self.state {
                state.capabilities = crate::connect::capabilities(&cluster);
            }
        } else {
            let value = self.api.call("player.state", json!({})).await?;
            self.state = snapshot(&value)?;
            self.epoch = self.api.browser()?.session_epoch();
            if let Some(state) = &mut self.state
                && let Ok(devices) = self.api.devices().list().await
                && let Some(device) = devices.iter().find(|device| device.active)
            {
                state.device_id = Some(device.id.clone());
                state.device_name = Some(device.name.clone());
                state.volume = device.volume;
            }
        }
        Ok(self.state.clone())
    }
    pub async fn queue(&self) -> Result<Queue, Error> {
        queue(&self.api.call("player.queue", json!({})).await?)
    }
    pub async fn volume(&self) -> Result<f64, Error> {
        self.api
            .call("volume.get", json!({}))
            .await?
            .as_f64()
            .ok_or_else(|| Error::Backend("invalid volume response".into()))
    }
    pub async fn set_speed(&self, speed: f64) -> Result<(), Error> {
        if !speed.is_finite() || !(0.5..=3.5).contains(&speed) {
            return Err(Error::InvalidInput("podcast speed must be 0.5..=3.5"));
        }
        self.api
            .call("player.setSpeed", json!({"speed":speed}))
            .await?;
        Ok(())
    }
    pub async fn clear_queue(&self) -> Result<(), Error> {
        self.api.call("queue.clear", json!({})).await?;
        Ok(())
    }
    /// Changes the active Connect context's shuffle mode through Spotify's ShuffleAPI.
    pub async fn set_shuffle_mode(&self, mode: ShuffleMode) -> Result<(), Error> {
        let mode = match mode {
            ShuffleMode::Off => 0,
            ShuffleMode::On => 1,
            ShuffleMode::Smart => 2,
        };
        self.api
            .call("player.setShuffleMode", json!({"mode":mode}))
            .await?;
        Ok(())
    }
    pub async fn remove_from_queue(&self, items: &[QueueItem]) -> Result<(), Error> {
        let values: Vec<_> = items
            .iter()
            .map(|item| json!({"uri":item.uri,"uid":item.uid}))
            .collect();
        self.api
            .call("queue.remove", json!({"items":values}))
            .await?;
        Ok(())
    }
    /// Kept so a page reload can restore the level; a fresh page starts at auto.
    /// The Web Player has no lossless level, so `Lossless` and `HiRes` clamp to `VeryHigh`.
    pub async fn set_quality(&mut self, quality: AudioQuality) -> Result<(), Error> {
        let quality = match quality {
            AudioQuality::Lossless | AudioQuality::HiRes => AudioQuality::VeryHigh,
            quality => quality,
        };
        self.api
            .call("player.setAudioQuality", json!({"level":quality.as_str()}))
            .await?;
        self.quality = quality;
        Ok(())
    }
    /// What the page is actually streaming now, after any account or tier clamp.
    pub async fn audio_info(&self) -> Result<AudioInfo, Error> {
        let value = self.api.call("player.audioInfo", json!({})).await?;
        serde_json::from_value(value).map_err(|_| Error::Backend("invalid audio info".into()))
    }
    pub async fn play_context(
        &mut self,
        uri: &str,
        track: Option<&str>,
        index: Option<u32>,
        position_ms: Option<u64>,
    ) -> Result<(), Error> {
        self.dispatch(
            Command::PlayContext {
                uri: uri.into(),
                track: track.map(str::to_owned),
                index,
                position: position_ms.map(std::time::Duration::from_millis),
            },
            true,
        )
        .await
    }
    pub fn remote_capabilities(&self) -> Capabilities {
        if self.api.browser().is_err() {
            return self
                .state
                .as_ref()
                .map(|state| state.capabilities)
                .unwrap_or_default();
        }
        if !self.api.browser().is_ok_and(|browser| browser.is_ready()) {
            return Capabilities::default();
        }
        self.state()
            .map(|state| state.capabilities)
            .unwrap_or(Capabilities {
                load: true,
                volume: true,
                queue: true,
                queue_edit: true,
                play_context: true,
                speed: true,
                quality: true,
                ..Default::default()
            })
    }

    /// Web-assisted operations for an already selected remote Connect device.
    /// The caller must verify/activate that device before issuing a command.
    pub async fn execute_remote(&mut self, command: Command) -> Result<(), Error> {
        if !matches!(
            command,
            Command::PlayContext { .. }
                | Command::ClearQueue
                | Command::RemoveFromQueue(_)
                | Command::InsertIntoQueue { .. }
                | Command::ReorderQueue { .. }
                | Command::SkipToQueue(_)
                | Command::SetSpeed(_)
                | Command::SetShuffleMode(_)
        ) {
            return Err(Error::Unsupported);
        }
        self.dispatch(command, false).await
    }

    /// Targets Connect directly; unavailable target queue state is never borrowed.
    pub async fn dispatch_on(&mut self, device: &str, command: Command) -> Result<(), Error> {
        if device.trim().is_empty() {
            return Err(Error::InvalidInput("missing Connect device"));
        }
        let (method, args) = match command {
            Command::Load(uri) => {
                super::domains::uri(&uri)?;
                ("player.play", json!({"contextUri":uri}))
            }
            Command::Resume => ("player.resume", json!({})),
            Command::Pause | Command::Stop => ("player.pause", json!({})),
            Command::Seek(position) => {
                let ms = position.as_millis();
                if ms > 9_007_199_254_740_991 {
                    return Err(Error::InvalidInput(
                        "position exceeds JavaScript integer range",
                    ));
                }
                ("player.seek", json!({"positionMs":ms as u64}))
            }
            Command::Next => ("player.next", json!({})),
            Command::Previous => ("player.previous", json!({})),
            Command::SetVolume(level) => {
                if !level.is_finite() || !(0.0..=1.0).contains(&level) {
                    return Err(Error::InvalidInput("volume must be 0..=1"));
                }
                ("volume.set", json!({"level":level}))
            }
            Command::SetShuffle(on) => ("player.setShuffle", json!({"on":on})),
            Command::SetShuffleMode(mode) => (
                "player.setShuffleMode",
                json!({"mode":match mode {
                    ShuffleMode::Off => 0,
                    ShuffleMode::On => 1,
                    ShuffleMode::Smart => 2,
                }}),
            ),
            Command::SetRepeat(mode) => ("player.setRepeat", json!({"mode":mode.as_u8()})),
            Command::AddToQueue(uris) => {
                if uris.len() != 1 {
                    return Err(Error::Unsupported);
                }
                super::domains::uri(&uris[0])?;
                ("queue.add", json!({"uris":uris}))
            }
            Command::ClearQueue => ("queue.clear", json!({})),
            Command::RemoveFromQueue(items) => {
                if items.is_empty() {
                    return Err(Error::InvalidInput("no queue items"));
                }
                for item in &items {
                    super::domains::uri(&item.uri)?;
                    if item.uid.is_empty() {
                        return Err(Error::InvalidInput("queue item has no uid"));
                    }
                }
                ("queue.remove", json!({"items":items}))
            }
            Command::PlayContext {
                uri,
                track,
                index,
                position,
            } => {
                super::domains::uri(&uri)?;
                let mut args = json!({"contextUri":uri});
                if let Some(track) = track {
                    super::domains::uri(&track)?;
                    args["trackUri"] = json!(track);
                } else if let Some(index) = index {
                    args["index"] = json!(index);
                }
                if let Some(position) = position {
                    let ms = position.as_millis();
                    if ms > 9_007_199_254_740_991 {
                        return Err(Error::InvalidInput(
                            "position exceeds JavaScript integer range",
                        ));
                    }
                    args["positionMs"] = json!(ms as u64);
                }
                ("player.play", args)
            }
            Command::SetSpeed(speed) => {
                if !speed.is_finite() || !(0.5..=3.5).contains(&speed) {
                    return Err(Error::InvalidInput("podcast speed must be 0.5..=3.5"));
                }
                ("player.setSpeed", json!({"speed":speed}))
            }
            _ => return Err(Error::Unsupported),
        };
        crate::connect::call_to(&self.api, Some(device), method, args).await?;
        Ok(())
    }

    async fn dispatch(&mut self, command: Command, local_playback: bool) -> Result<(), Error> {
        self.refresh().await?;
        if !self.remote_capabilities().supports(&command) {
            return Err(Error::Unsupported);
        }
        let (method, args) = match command {
            Command::Load(uri) => {
                super::domains::uri(&uri)?;
                ("player.play", json!({"contextUri":uri}))
            }
            Command::Resume => ("player.resume", json!({})),
            Command::Pause => ("player.pause", json!({})),
            Command::Seek(position) => {
                let ms = position.as_millis();
                if ms > 9_007_199_254_740_991 {
                    return Err(Error::InvalidInput(
                        "position exceeds JavaScript integer range",
                    ));
                }
                ("player.seek", json!({"positionMs":ms as u64}))
            }
            Command::Next => ("player.next", json!({})),
            Command::Previous => ("player.previous", json!({})),
            Command::SetVolume(level) => {
                if !level.is_finite() || !(0.0..=1.0).contains(&level) {
                    return Err(Error::InvalidInput("volume must be 0..=1"));
                }
                ("volume.set", json!({"level":level}))
            }
            Command::SetShuffle(on) => ("player.setShuffle", json!({"on":on})),
            Command::SetRepeat(mode) => ("player.setRepeat", json!({"mode":mode.as_u8()})),
            Command::AddToQueue(uris) => {
                for value in &uris {
                    super::domains::uri(value)?;
                }
                ("queue.add", json!({"uris":uris}))
            }
            Command::InsertIntoQueue { uris, before } => {
                if uris.is_empty() {
                    return Err(Error::InvalidInput("no queue URIs"));
                }
                for value in &uris {
                    super::domains::uri(value)?;
                }
                let mut args = json!({"uris":uris});
                if let Some(before) = before {
                    args["before"] = queue_item(&before);
                }
                ("queue.insert", args)
            }
            Command::ReorderQueue {
                items,
                before,
                after,
            } => {
                if items.is_empty() {
                    return Err(Error::InvalidInput("no queue items"));
                }
                let items: Vec<_> = items.iter().map(queue_item).collect();
                let args = match (before, after) {
                    (Some(before), None) => json!({"items":items,"before":queue_item(&before)}),
                    (None, Some(after)) => json!({"items":items,"after":queue_item(&after)}),
                    (Some(_), Some(_)) => {
                        return Err(Error::InvalidInput(
                            "reorder takes a before or an after row, not both",
                        ));
                    }
                    (None, None) => {
                        return Err(Error::InvalidInput("reorder needs a before or after row"));
                    }
                };
                ("queue.reorder", args)
            }
            Command::SkipToQueue(item) => {
                super::domains::uri(&item.uri)?;
                if item.uid.is_empty() {
                    return Err(Error::InvalidInput("queue item has no uid"));
                }
                ("queue.skipTo", json!({"uri":item.uri,"uid":item.uid}))
            }
            Command::ClearQueue => {
                self.clear_queue().await?;
                return Ok(());
            }
            Command::RemoveFromQueue(items) => {
                self.remove_from_queue(&items).await?;
                return Ok(());
            }
            Command::PlayContext {
                uri,
                track,
                index,
                position,
            } => {
                super::domains::uri(&uri)?;
                if let Some(track) = &track {
                    super::domains::uri(track)?;
                }
                let position = position.map(|v| v.as_millis());
                if position.is_some_and(|v| v > 9_007_199_254_740_991) {
                    return Err(Error::InvalidInput(
                        "position exceeds JavaScript integer range",
                    ));
                }
                (
                    "player.play",
                    json!({"contextUri":uri,"trackUri":track,"index":index,"positionMs":position.map(|v|v as u64)}),
                )
            }
            Command::SetSpeed(speed) => {
                self.set_speed(speed).await?;
                return Ok(());
            }
            Command::SetShuffleMode(mode) => {
                self.set_shuffle_mode(mode).await?;
                return Ok(());
            }
            Command::SetQuality(quality) => {
                self.set_quality(quality).await?;
                return Ok(());
            }
            Command::Stop => return Err(Error::Unsupported),
            _ => return Err(Error::Unsupported),
        };
        if local_playback
            && self.api.browser().is_ok()
            && matches!(method, "player.play" | "player.resume")
        {
            self.api.devices().pull_here().await?;
        }
        self.api.call(method, args).await?;
        Ok(())
    }
}
impl Backend for WebBackend {
    fn capabilities(&self) -> Capabilities {
        if self.role == BrowserRole::Services && self.api.browser().is_ok() {
            Capabilities::default()
        } else {
            self.remote_capabilities()
        }
    }
    fn execute(&mut self, command: Command) -> BackendFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            if let Command::On { device, command } = command {
                return self.dispatch_on(&device, *command).await;
            }
            if self.role == BrowserRole::Services && self.api.browser().is_ok() {
                return Err(Error::Unsupported);
            }
            self.dispatch(command, true).await
        })
    }
    fn next_event(&mut self) -> BackendFuture<'_, Result<PlaybackEvent, Error>> {
        Box::pin(async move {
            {
                let event = self
                    .events
                    .as_mut()
                    .ok_or(Error::Unsupported)?
                    .recv()
                    .await
                    .map_err(|error| match error {
                        broadcast::error::RecvError::Closed => Error::Disconnected,
                        broadcast::error::RecvError::Lagged(n) => Error::EventsLagged(n),
                    })?;
                match event {
                    BrowserEvent::State(value) => {
                        self.state = snapshot(&value)?;
                        self.epoch = self.api.browser()?.session_epoch();
                        Ok(match self.state.clone() {
                            Some(state) => PlaybackEvent::Snapshot(Box::new(state)),
                            None => PlaybackEvent::StateChanged(spotcore_core::PlaybackState::Idle),
                        })
                    }
                    BrowserEvent::Queue(value) => Ok(PlaybackEvent::QueueChanged(queue(&value)?)),
                    BrowserEvent::SessionChanged => {
                        self.state = None;
                        self.epoch = self.api.browser()?.session_epoch();
                        Ok(PlaybackEvent::SessionChanged)
                    }
                    BrowserEvent::Ready => {
                        if self.role == BrowserRole::Playback {
                            self.refresh().await?;
                        }
                        // A reloaded page starts at auto; restore the chosen level.
                        if self.quality != AudioQuality::Auto {
                            let quality = self.quality;
                            let _ = self.set_quality(quality).await;
                        }
                        Ok(PlaybackEvent::CapabilitiesChanged(self.capabilities()))
                    }
                    BrowserEvent::Error(message) => Err(Error::Backend(message)),
                }
            }
        })
    }
}
