use crate::web::{Api, ManagedHost, WebBackend};
use crate::{
    AppliedQuality, AudioQuality, Backend, BackendFuture, Capabilities, Client, Command, Error,
    PlaybackEvent, PlaybackMode, PlayerSnapshot, Queue, QueueItem, RepeatMode, ShuffleMode,
    SpotCoreBuilder, Spotify,
};
use std::time::Duration;

/// A Web session with optional, separately installed Soloist playback.
pub struct SpotCore {
    pub api: Api,
    pub player: Client<SessionBackend>,
    pub host: Option<ManagedHost>,
    pub(crate) native_fallback: Option<Error>,
}

impl SpotCore {
    pub fn new(api: Api, backend: WebBackend) -> Self {
        Self::from_web(Spotify::new(api, backend))
    }
    pub fn with_host(api: Api, backend: WebBackend, host: ManagedHost) -> Self {
        Self::from_web(Spotify::with_host(api, backend, host))
    }

    pub fn builder() -> SpotCoreBuilder {
        SpotCoreBuilder::new()
    }
    pub async fn launch() -> Result<Self, Error> {
        Self::builder().launch().await
    }
    pub async fn connect(endpoint: &str) -> Result<Self, Error> {
        Ok(Self::from_web(Spotify::connect_web(endpoint).await?))
    }
    pub async fn connect_web(endpoint: &str) -> Result<Self, Error> {
        Self::connect(endpoint).await
    }
    pub async fn login(profile: Option<&std::path::Path>) -> Result<(), Error> {
        Spotify::login(profile).await
    }

    pub(crate) fn from_web(spotify: Spotify<WebBackend>) -> Self {
        let web = spotify.player.into_backend();
        let snapshot = web.state().cloned();
        let mut backend = SessionBackend {
            web,
            snapshot,
            #[cfg(feature = "native")]
            native: None,
        };
        if let Some(mut state) = backend.snapshot.take() {
            state.capabilities = backend.capabilities();
            backend.snapshot = Some(state);
        }
        Self {
            api: spotify.api,
            player: Client::new(backend),
            host: spotify.host,
            native_fallback: None,
        }
    }

    /// Disconnects attached sessions and stops owned processes without closing an external browser.
    pub async fn shutdown(mut self) -> Result<(), Error> {
        self.shutdown_owned().await
    }

    pub(crate) async fn shutdown_owned(&mut self) -> Result<(), Error> {
        #[cfg(feature = "native")]
        let native_result = if let Some(native) = &mut self.player.backend_mut().native {
            native.shutdown().await
        } else {
            Ok(())
        };
        let host_result = if let Some(host) = &mut self.host {
            if host.engine() == crate::BrowserEngine::System
                && let Ok(browser) = self.api.browser()
            {
                let _ = browser.shutdown_host().await;
            }
            host.shutdown().await
        } else {
            Ok(())
        };
        #[cfg(feature = "native")]
        native_result?;
        host_result
    }

    /// Why an explicitly requested native engine was unavailable; playback remains Web.
    pub fn native_fallback_reason(&self) -> Option<&Error> {
        self.native_fallback.as_ref()
    }
    pub fn playback_mode(&self) -> PlaybackMode {
        self.player.backend().playback_mode()
    }
    pub fn capabilities(&self) -> Capabilities {
        self.player.capabilities()
    }
    pub async fn execute(&mut self, command: Command) -> Result<(), Error> {
        self.player.execute(command).await
    }
    /// Targets a Connect device without pulling playback into the local player.
    pub async fn execute_on(
        &mut self,
        device: impl Into<String>,
        command: Command,
    ) -> Result<(), Error> {
        self.player.execute_on(device, command).await
    }
    /// Starts a URI on a Connect device; that receiver may become active.
    pub async fn play_on(
        &mut self,
        device: impl Into<String>,
        uri: impl Into<String>,
    ) -> Result<(), Error> {
        self.player.play_on(device, uri).await
    }
    pub async fn next_event(&mut self) -> Result<PlaybackEvent, Error> {
        self.player.next_event().await
    }
    pub async fn play(&mut self, uri: impl Into<String>) -> Result<(), Error> {
        self.player.play(uri).await
    }
    pub async fn pause(&mut self) -> Result<(), Error> {
        self.player.pause().await
    }
    pub async fn resume(&mut self) -> Result<(), Error> {
        self.player.resume().await
    }
    pub async fn stop(&mut self) -> Result<(), Error> {
        self.player.stop().await
    }
    pub async fn seek(&mut self, position: Duration) -> Result<(), Error> {
        self.player.seek(position).await
    }
    pub async fn next(&mut self) -> Result<(), Error> {
        self.player.next().await
    }
    pub async fn previous(&mut self) -> Result<(), Error> {
        self.player.previous().await
    }
    pub async fn set_volume(&mut self, volume: f64) -> Result<(), Error> {
        self.player.set_volume(volume).await
    }
    pub async fn set_shuffle(&mut self, on: bool) -> Result<(), Error> {
        self.player.set_shuffle(on).await
    }
    pub async fn set_shuffle_mode(&mut self, mode: ShuffleMode) -> Result<(), Error> {
        self.execute(Command::SetShuffleMode(mode)).await
    }
    pub async fn set_repeat(&mut self, mode: RepeatMode) -> Result<(), Error> {
        self.player.set_repeat(mode).await
    }
    pub async fn set_speed(&mut self, speed: f64) -> Result<(), Error> {
        self.player.set_speed(speed).await
    }
    pub async fn rewind(&mut self, amount: Duration) -> Result<(), Error> {
        self.player.rewind(amount).await
    }
    pub async fn forward(&mut self, amount: Duration) -> Result<(), Error> {
        self.player.forward(amount).await
    }
    pub async fn add_to_queue(&mut self, uris: Vec<String>) -> Result<(), Error> {
        self.player.add_to_queue(uris).await
    }
    pub async fn insert_into_queue(
        &mut self,
        uris: Vec<String>,
        before: Option<QueueItem>,
    ) -> Result<(), Error> {
        self.player.insert_into_queue(uris, before).await
    }
    pub async fn reorder_queue(
        &mut self,
        items: Vec<QueueItem>,
        before: Option<QueueItem>,
        after: Option<QueueItem>,
    ) -> Result<(), Error> {
        self.player.reorder_queue(items, before, after).await
    }
    pub async fn skip_to_queue(&mut self, item: QueueItem) -> Result<(), Error> {
        self.player.skip_to_queue(item).await
    }
    pub async fn clear_queue(&mut self) -> Result<(), Error> {
        self.player.clear_queue().await
    }
    pub async fn remove_from_queue(&mut self, items: Vec<QueueItem>) -> Result<(), Error> {
        self.player.remove_from_queue(items).await
    }
    pub async fn play_context(
        &mut self,
        uri: impl Into<String>,
        track: Option<String>,
        index: Option<u32>,
        position: Option<Duration>,
    ) -> Result<(), Error> {
        self.player.play_context(uri, track, index, position).await
    }
    pub async fn drm_support(&self) -> Result<crate::web::DrmSupport, Error> {
        self.player.backend().web.drm_support().await
    }

    pub async fn queue(&self) -> Result<Queue, Error> {
        self.player.backend().web.queue().await
    }
    pub async fn refresh_state(&mut self) -> Result<Option<PlayerSnapshot>, Error> {
        self.player.refresh_state().await
    }
    pub async fn set_quality(&mut self, quality: AudioQuality) -> Result<AppliedQuality, Error> {
        self.player.backend_mut().apply_quality(quality).await
    }

    pub fn search(&self) -> crate::web::Search<'_> {
        self.api.search()
    }
    pub fn catalog(&self) -> crate::web::Catalog<'_> {
        self.api.catalog()
    }
    pub fn library(&self) -> crate::web::Library<'_> {
        self.api.library()
    }
    pub fn playlists(&self) -> crate::web::Playlists<'_> {
        self.api.playlists()
    }
    pub fn home(&self) -> crate::web::Home<'_> {
        self.api.home()
    }
    pub fn podcasts(&self) -> crate::web::Podcasts<'_> {
        self.api.podcasts()
    }
    pub fn users(&self) -> crate::web::Users<'_> {
        self.api.users()
    }
    pub fn extras(&self) -> crate::web::Extras<'_> {
        self.api.extras()
    }
    pub fn devices(&self) -> crate::web::Devices<'_> {
        self.api.devices()
    }
    pub fn jam(&self) -> crate::web::Jam<'_> {
        self.api.jam()
    }
    pub fn blends(&self) -> crate::web::Blends<'_> {
        self.api.blends()
    }
    pub fn recents(&self) -> crate::web::Recents<'_> {
        self.api.recents()
    }

    pub fn state(&self) -> Option<&PlayerSnapshot> {
        self.player.state()
    }
    pub fn is_playing(&self) -> bool {
        self.player.is_playing()
    }
    pub fn current_track(&self) -> Option<&crate::MediaItem> {
        self.player.current_track()
    }
    pub fn position_ms(&self) -> u64 {
        self.player.position_ms()
    }
    pub async fn volume(&self) -> Result<f64, Error> {
        self.player.volume().await
    }
    pub async fn audio_info(&self) -> Result<crate::AudioInfo, Error> {
        self.player.audio_info().await
    }
    pub async fn toggle(&mut self) -> Result<(), Error> {
        self.player.toggle().await
    }

    #[cfg(feature = "native")]
    pub(crate) async fn try_native(
        &mut self,
        credentials: crate::SoloistCredentials,
        paths: &crate::Paths,
        name: Option<&str>,
    ) {
        let result = async {
            let account = self.api.users().me().await?;
            if account.tier != crate::AccountTier::Premium {
                return Err(Error::AccountRestricted(
                    "native playback requires a verified Premium account",
                ));
            }
            let key = credentials
                .api_key
                .filter(|key| !key.trim().is_empty())
                .ok_or(Error::InvalidInput("native API key is empty"))?;
            let native = crate::native::NativeBackend::launch_as(
                key,
                paths,
                name.unwrap_or(crate::native::DEFAULT_DEVICE_NAME),
            )
            .await?;
            if !native.wait_authenticated(Duration::from_secs(10)).await {
                return Err(Error::NotAuthenticated);
            }
            // Registration in this Web account prevents controlling another account's daemon.
            crate::spotify::native_device(&self.api, &native).await?;
            Ok(native)
        }
        .await;
        match result {
            Ok(native) => self.player.backend_mut().set_native(native),
            Err(error) => self.native_fallback = Some(error),
        }
    }
}

/// Coordinates commands before dispatch; failed writes are never retried on another engine.
pub struct SessionBackend {
    web: WebBackend,
    snapshot: Option<PlayerSnapshot>,
    #[cfg(feature = "native")]
    native: Option<crate::native::NativeBackend>,
}

impl SessionBackend {
    pub fn playback_mode(&self) -> PlaybackMode {
        #[cfg(feature = "native")]
        if self.native.is_some() {
            return PlaybackMode::Native;
        }
        PlaybackMode::Web
    }

    #[cfg(feature = "native")]
    fn set_native(&mut self, native: crate::native::NativeBackend) {
        self.native = Some(native);
        if let Some(mut state) = self.snapshot.take() {
            state.capabilities = self.capabilities();
            self.snapshot = Some(state);
        }
    }

    #[cfg(feature = "native")]
    async fn activate_native(&self) -> Result<(), Error> {
        let native = self.native.as_ref().ok_or(Error::Unsupported)?;
        let name = native.device_name().ok_or(Error::Unsupported)?;
        let device = crate::spotify::named_device(self.web.api().devices().list().await?, name)?
            .ok_or(Error::Disconnected)?;
        if !device.active {
            self.web.api().devices().transfer(&device.id, false).await?;
            if !native.wait_active(Duration::from_secs(10)).await {
                return Err(Error::Backend(
                    "native Connect device did not become active".into(),
                ));
            }
        }
        Ok(())
    }

    async fn apply_quality(&mut self, quality: AudioQuality) -> Result<AppliedQuality, Error> {
        #[cfg(feature = "native")]
        if let Some(native) = &self.native {
            return crate::spotify::apply_native_quality(self.web.api(), native, quality).await;
        }
        if self.web.role() == crate::BrowserRole::Services {
            return Err(Error::Unsupported);
        }
        self.web.set_quality(quality).await?;
        Ok(AppliedQuality {
            requested: quality,
            effective: match quality {
                AudioQuality::Lossless | AudioQuality::HiRes => AudioQuality::VeryHigh,
                value => value,
            },
            lossless_available: false,
        })
    }
}

#[cfg(any(feature = "native", test))]
fn web_control(command: &Command) -> bool {
    matches!(
        command,
        Command::ClearQueue
            | Command::RemoveFromQueue(_)
            | Command::InsertIntoQueue { .. }
            | Command::ReorderQueue { .. }
            | Command::SkipToQueue(_)
            | Command::PlayContext { .. }
            | Command::SetSpeed(_)
            | Command::SetShuffleMode(_)
    )
}

impl Backend for SessionBackend {
    fn capabilities(&self) -> Capabilities {
        #[cfg(feature = "native")]
        if let Some(native) = &self.native {
            let mut capabilities = native.capabilities();
            let web = self.web.remote_capabilities();
            capabilities.queue_edit = web.queue_edit;
            capabilities.play_context = web.play_context;
            capabilities.speed = web.speed;
            capabilities.smart_shuffle = web.smart_shuffle;
            capabilities.quality = (web.quality || self.web.api().browser().is_err())
                && native.device_name().is_some();
            return capabilities;
        }
        let mut capabilities = self.web.capabilities();
        capabilities.stop = capabilities.pause;
        capabilities
    }
    fn execute(&mut self, command: Command) -> BackendFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            if let Command::On { device, command } = command {
                return self.web.dispatch_on(&device, *command).await;
            }
            if let Command::SetQuality(quality) = command {
                self.apply_quality(quality).await?;
                return Ok(());
            }
            #[cfg(feature = "native")]
            {
                if self.native.is_some()
                    && (web_control(&command)
                        || matches!(command, Command::Load(_) | Command::Resume))
                {
                    self.activate_native().await?;
                }
                if !web_control(&command)
                    && let Some(native) = &mut self.native
                {
                    return native.execute(command).await;
                }
            }
            if self.playback_mode() == PlaybackMode::Web
                && self.web.role() == crate::BrowserRole::Services
                && self.web.api().browser().is_ok()
            {
                return Err(Error::Unsupported);
            }
            if self.playback_mode() == PlaybackMode::Web
                && matches!(
                    command,
                    Command::Load(_) | Command::Resume | Command::PlayContext { .. }
                )
                && self.web.api().browser().is_ok()
            {
                self.web.drm_support().await?.require_playback()?;
            }
            if matches!(command, Command::Stop) {
                return self.web.execute(Command::Pause).await;
            }
            #[cfg(feature = "native")]
            if self.native.is_some() && web_control(&command) {
                return self.web.execute_remote(command).await;
            }
            self.web.execute(command).await
        })
    }
    fn next_event(&mut self) -> BackendFuture<'_, Result<PlaybackEvent, Error>> {
        Box::pin(async move {
            #[cfg(feature = "native")]
            let event = if let Some(native) = &mut self.native {
                if self.web.api().browser().is_err() {
                    native.next_event().await
                } else {
                    tokio::select! { event = native.next_event() => event, event = self.web.next_event() => event }
                }
            } else {
                self.web.next_event().await
            };
            #[cfg(not(feature = "native"))]
            let event = self.web.next_event().await;
            let mut event = event?;
            let capabilities = self.capabilities();
            match &mut event {
                PlaybackEvent::Snapshot(snapshot) => {
                    snapshot.capabilities = capabilities;
                    self.snapshot = Some((**snapshot).clone());
                }
                PlaybackEvent::CapabilitiesChanged(current) => {
                    *current = capabilities;
                    if let Some(state) = &mut self.snapshot {
                        state.capabilities = capabilities;
                    }
                }
                PlaybackEvent::StateChanged(state) => {
                    if let Some(snapshot) = &mut self.snapshot {
                        snapshot.state = *state;
                    }
                }
                _ => {}
            }
            Ok(event)
        })
    }
}

impl Client<SessionBackend> {
    pub fn state(&self) -> Option<&PlayerSnapshot> {
        if self.backend.playback_mode() == PlaybackMode::Web
            && self.backend.web.api().browser().is_ok()
            && !self
                .backend
                .web
                .api()
                .browser()
                .is_ok_and(|browser| browser.is_ready())
        {
            return None;
        }
        self.backend.snapshot.as_ref()
    }
    pub async fn refresh_state(&mut self) -> Result<Option<PlayerSnapshot>, Error> {
        let web = self.backend.web.refresh().await;
        #[cfg(feature = "native")]
        let native = match &self.backend.native {
            Some(native) => native.current_snapshot().await,
            None => None,
        };
        #[cfg(not(feature = "native"))]
        let native: Option<PlayerSnapshot> = None;
        let mut state = match native {
            Some(state) => Some(state),
            None => web.clone()?,
        };
        if let Some(state) = &mut state {
            state.capabilities = self.backend.capabilities();
            if let Ok(Some(web)) = &web
                && state.track.as_ref().map(|track| &track.uri)
                    == web.track.as_ref().map(|track| &track.uri)
            {
                state.context_uri = web.context_uri.clone();
                state.smart_shuffle = web.smart_shuffle;
            }
        }
        self.backend.snapshot = state.clone();
        Ok(state)
    }
    pub fn is_playing(&self) -> bool {
        self.state()
            .is_some_and(|state| state.state == crate::PlaybackState::Playing)
    }
    pub fn current_track(&self) -> Option<&crate::MediaItem> {
        self.state().and_then(|state| state.track.as_ref())
    }
    pub fn position_ms(&self) -> u64 {
        self.state().map_or(0, |state| state.position_ms)
    }
    pub async fn queue(&self) -> Result<Queue, Error> {
        self.backend.web.queue().await
    }
    pub async fn volume(&self) -> Result<f64, Error> {
        self.backend.web.volume().await
    }
    pub async fn audio_info(&self) -> Result<crate::AudioInfo, Error> {
        if self.backend.playback_mode() == PlaybackMode::Native {
            return Err(Error::Unsupported);
        }
        self.backend.web.audio_info().await
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
        self.seek(Duration::from_millis(crate::skip_target(
            position,
            -crate::skip_millis(amount),
            duration,
        )))
        .await
    }

    /// Moves playback forward by `amount`, clamped to the item duration.
    pub async fn forward(&mut self, amount: Duration) -> Result<(), Error> {
        let (position, duration) = self.skip_bounds().await?;
        self.seek(Duration::from_millis(crate::skip_target(
            position,
            crate::skip_millis(amount),
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

#[cfg(all(test, feature = "native"))]
use crate::browser_test as test_browser;

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "native")]
    #[tokio::test]
    async fn native_playback_and_web_controls_share_one_session() {
        use futures_util::{SinkExt, StreamExt};
        use serde_json::{Value, json};
        use tokio_tungstenite::tungstenite::Message;
        let browser = super::test_browser::MockBrowser::start().await;
        let api = Api::connect(&browser.endpoint).await.unwrap();
        let backend = WebBackend::with_role(api.clone(), crate::BrowserRole::Services)
            .await
            .unwrap();
        let web = Spotify::new(api, backend);
        let mut sdk = SpotCore::from_web(web);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("ws://{}", listener.local_addr().unwrap());
        let (tx, mut commands) = tokio::sync::mpsc::unbounded_channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            socket
                .send(Message::Text(
                    json!({"type":"auth_state","logged_in":true,"is_active":true})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value["command"] != "get_state" {
                    let _ = tx.send(value);
                }
            }
        });
        let native = crate::native::NativeBackend::connect_named(&address, "Test native")
            .await
            .unwrap();
        sdk.player.backend_mut().set_native(native);
        sdk.refresh_state().await.unwrap();
        assert_eq!(sdk.state().unwrap().capabilities, sdk.capabilities());
        sdk.play(super::test_browser::TRACK).await.unwrap();
        sdk.pause().await.unwrap();
        for expected in ["play", "pause"] {
            let command = tokio::time::timeout(Duration::from_secs(1), commands.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(command["command"], expected);
        }
        assert!(
            !browser
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|(name, _)| name == "player.play" || name == "player.pause")
        );
        assert_eq!(sdk.playback_mode(), PlaybackMode::Native);
        assert!(sdk.capabilities().queue_edit);
        assert!(sdk.capabilities().smart_shuffle);
        sdk.clear_queue().await.unwrap();
        sdk.set_speed(1.25).await.unwrap();
        sdk.set_shuffle_mode(ShuffleMode::Smart).await.unwrap();
        let calls = browser.calls.lock().unwrap();
        for method in ["queue.clear", "player.setSpeed", "player.setShuffleMode"] {
            assert!(calls.iter().any(|(name, _)| name == method));
        }
        assert!(commands.try_recv().is_err());
        server.abort();
    }

    #[test]
    fn routing_is_chosen_before_writing() {
        assert!(web_control(&Command::ClearQueue));
        assert!(web_control(&Command::SetShuffleMode(ShuffleMode::Smart)));
        assert!(web_control(&Command::SetSpeed(1.25)));
        assert!(!web_control(&Command::Load("spotify:track:test".into())));
        assert!(!web_control(&Command::Pause));
    }
}
