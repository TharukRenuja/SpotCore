use crate::web::{Api, ManagedHost, WebBackend};
use crate::{
    AppliedQuality, AudioQuality, Backend, BackendFuture, Capabilities, Client, Command, Error,
    PlaybackEvent, PlaybackMode, PlayerSnapshot, QualityReport, QualitySource, Queue, QueueItem,
    RepeatMode, ShuffleMode, SpotCoreBuilder, Spotify,
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
    /// Finds this session's player through its backend, independently of HTTP services.
    pub async fn local_device(&self) -> Result<Option<crate::web::Device>, Error> {
        #[cfg(feature = "native")]
        if let Some(native) = &self.player.backend().native {
            let devices = self.api.devices().list().await?;
            if let Some(id) = native.device_id() {
                return Ok(devices.into_iter().find(|device| device.id == id));
            }
            return match native.device_name() {
                Some(name) => crate::spotify::named_device(devices, name),
                None => Ok(None),
            };
        }
        let web = &self.player.backend().web;
        if web.role() != crate::BrowserRole::Playback {
            return Err(Error::Unsupported);
        }
        let mut devices = web
            .api()
            .devices()
            .list()
            .await?
            .into_iter()
            .filter(|device| device.local);
        let device = devices.next();
        if devices.next().is_some() {
            return Err(Error::Backend(
                "More than one local playback device was reported".into(),
            ));
        }
        Ok(device)
    }

    pub async fn quality_report(&self) -> Result<QualityReport, Error> {
        if self.playback_mode() == PlaybackMode::Native {
            let device = self.local_device().await?.ok_or(Error::Disconnected)?;
            return Ok(QualityReport {
                quality: Some(self.api.devices().audio_quality(&device).await?),
                source: QualitySource::DevicePreference,
                lossless_available: device.lossless_supported && device.lossless_eligible,
            });
        }
        let info = self.audio_info().await?;
        let quality = [
            AudioQuality::Auto,
            AudioQuality::Low,
            AudioQuality::Normal,
            AudioQuality::High,
            AudioQuality::VeryHigh,
            AudioQuality::Lossless,
            AudioQuality::HiRes,
        ]
        .into_iter()
        .find(|quality| quality.as_str() == info.level);
        Ok(QualityReport {
            quality,
            source: QualitySource::WebStream,
            lossless_available: false,
        })
    }

    /// Wait for the owned receiver without activating or transferring playback.
    pub async fn wait_ready(&mut self, timeout: Duration) -> Result<crate::web::Device, Error> {
        tokio::time::timeout(timeout, async {
            loop {
                let delay = match self.local_device().await {
                    Ok(Some(device)) => return Ok(device),
                    Ok(None) | Err(Error::Disconnected | Error::Timeout) => Duration::from_secs(1),
                    Err(Error::Http {
                        status: 429,
                        retry_after,
                    }) => Duration::from_secs(retry_after.unwrap_or(30).max(1)),
                    Err(Error::Http {
                        status: 500..=599, ..
                    }) => Duration::from_secs(1),
                    Err(error) => return Err(error),
                };
                tokio::time::sleep(delay).await;
            }
        })
        .await
        .map_err(|_| Error::Timeout)?
    }

    /// Play a verified audiobook chapter. No chapter selection means chapter one, not server resume.
    pub async fn play_audiobook(
        &mut self,
        value: &str,
        chapter: Option<&str>,
        position: Duration,
        device: Option<&str>,
    ) -> Result<(), Error> {
        let book = self.api.podcasts().audiobook_info(value).await?;
        let uri = match chapter {
            Some(uri) => {
                let item = self.api.podcasts().episode(uri).await?;
                let parent = item
                    .raw
                    .pointer("/audiobookV2/data/uri")
                    .or_else(|| item.raw.pointer("/podcastV2/data/uri"))
                    .and_then(crate::web::Value::as_str);
                if parent != Some(book.uri.as_str()) {
                    return Err(Error::InvalidInput(
                        "could not verify chapter membership in this audiobook",
                    ));
                }
                item.require_playable()?;
                uri.to_owned()
            }
            None => {
                if !position.is_zero() {
                    return Err(Error::InvalidInput(
                        "an audiobook resume position requires its chapter URI",
                    ));
                }
                let page = self
                    .api
                    .podcasts()
                    .chapters(
                        &book.uri,
                        crate::Paging {
                            offset: 0,
                            limit: 50,
                        },
                    )
                    .await?;
                let item = page
                    .items
                    .first()
                    .ok_or_else(|| Error::FeatureUnavailable {
                        feature: "audiobook_playback".into(),
                        reason: crate::FeatureRestriction::TitleLocked,
                    })?;
                item.require_playable()?;
                item.uri.clone()
            }
        };
        self.play_spoken(&uri, position, device).await
    }

    /// Play one spoken item, restoring its position only after the receiver confirms the item.
    pub async fn play_spoken(
        &mut self,
        uri: &str,
        position: Duration,
        device: Option<&str>,
    ) -> Result<(), Error> {
        if !matches!(uri.split(':').nth(1), Some("episode" | "chapter")) {
            return Err(Error::InvalidInput(
                "spoken playback requires an episode or chapter URI",
            ));
        }
        let id = uri.rsplit(':').next().unwrap_or_default();
        if uri.split(':').count() != 3
            || id.is_empty()
            || !id.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err(Error::InvalidInput("invalid spoken item URI"));
        }
        if uri.starts_with("spotify:episode:") {
            self.api.podcasts().episode(uri).await?.require_playable()?;
        }
        let native_here = self.playback_mode() == PlaybackMode::Native
            && match device {
                None => true,
                Some(id) => self
                    .local_device()
                    .await?
                    .is_some_and(|local| local.id == id),
            };
        if native_here {
            self.play(uri).await?;
            if !position.is_zero() {
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        if self
                            .refresh_state()
                            .await?
                            .and_then(|state| state.track)
                            .is_some_and(|track| track.uri == uri)
                        {
                            break Ok::<(), Error>(());
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                })
                .await
                .map_err(|_| Error::Timeout)??;
                self.seek(position).await?;
            }
            return Ok(());
        }
        let command = Command::PlayContext {
            uri: uri.into(),
            track: None,
            index: None,
            position: Some(position),
        };
        match device {
            Some(id) => self.execute_on(id, command).await,
            None => self.execute(command).await,
        }
    }

    /// Transfers to the owned player without starting audio. Called for an explicit local Play.
    pub async fn activate_local(&self) -> Result<(), Error> {
        #[cfg(feature = "native")]
        if self.player.backend().native.is_some() {
            return self.player.backend().activate_native().await;
        }
        let web = &self.player.backend().web;
        let device = self.local_device().await?.ok_or(Error::Disconnected)?;
        if device.active {
            return Ok(());
        }
        web.api().devices().transfer(&device.id, false).await?;
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if self
                    .local_device()
                    .await?
                    .is_some_and(|current| current.id == device.id && current.active)
                {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .map_err(|_| Error::Backend("The local playback device did not become active".into()))?
    }

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

    pub(crate) async fn require_web_playback(&self) -> Result<(), Error> {
        let result = async { self.drm_support().await?.require_playback() }.await;
        result.map_err(|error| startup_error(self.native_fallback.clone(), error))
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
    /// Starts an ordered list on this SDK's local receiver as context rows, not manual queue entries.
    pub async fn play_items(
        &mut self,
        context: Option<&str>,
        uris: &[String],
        position: Duration,
    ) -> Result<(), Error> {
        let device = self.local_device().await?.ok_or(Error::Disconnected)?;
        self.api
            .devices()
            .play_items(Some(&device.id), context, uris, position)
            .await
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
        Ok(self.api.queues().snapshot().await?.queue)
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
    pub fn queues(&self) -> crate::web::Queues<'_> {
        self.api.queues()
    }
    pub fn jam(&self) -> crate::web::Jam<'_> {
        #[cfg(feature = "native")]
        if let Some(native) = &self.player.backend().native
            && let Some(device) = native.device_id()
        {
            let _ = self.api.bind_local_receiver(device);
        }
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
        allow_pairing: bool,
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
            let mut native = crate::native::NativeBackend::launch_as(
                key.clone(),
                paths,
                name.unwrap_or(crate::native::DEFAULT_DEVICE_NAME),
            )
            .await?;
            if let Err(error) =
                authenticate_native(&self.api, &native, Duration::from_secs(10)).await
            {
                // Pairing is interactive and must never run during an ordinary restore.
                if !matches!(error, Error::NativeAuthenticationRequired) || !allow_pairing {
                    return Err(error);
                }
                let device_name = name
                    .unwrap_or(crate::native::DEFAULT_DEVICE_NAME)
                    .to_owned();
                native.shutdown().await?;
                let pairing = crate::native::NativeBackend::pair_with(
                    key.clone(),
                    paths,
                    device_name,
                    Duration::from_secs(180),
                )
                .await;
                native = crate::native::NativeBackend::launch_as(
                    key,
                    paths,
                    name.unwrap_or(crate::native::DEFAULT_DEVICE_NAME),
                )
                .await?;
                if let Err(verification) =
                    authenticate_native(&self.api, &native, Duration::from_secs(60)).await
                {
                    return Err(pairing.err().unwrap_or(verification));
                }
            }
            // Registration in this Web account prevents controlling another account's daemon.
            let device = crate::spotify::native_device(&self.api, &native).await?;
            native.bind_device_id(device.id);
            if allow_pairing {
                native.execute(Command::Pause).await?;
            }
            Ok(native)
        }
        .await;
        match result {
            Ok(native) => self.player.backend_mut().set_native(native),
            Err(error) => self.native_fallback = Some(error),
        }
    }
}

#[cfg(feature = "native")]
async fn authenticate_native(
    api: &Api,
    native: &crate::native::NativeBackend,
    timeout: Duration,
) -> Result<(), Error> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut pending_operation = None;
    let verification = tokio::time::timeout_at(deadline, async {
        loop {
            if native.authentication_state()? {
                return Ok(());
            }
            let name = native
                .device_name()
                .ok_or(Error::NativeAuthenticationRequired)?;
            pending_operation =
                Some("discovering Spotify devices (no activation request was sent)");
            let devices = api.devices().list().await?;
            pending_operation = None;
            if native.authentication_state()? {
                return Ok(());
            }
            if let Some(device) = crate::spotify::named_device(
                devices.into_iter().filter(|device| !device.local).collect(),
                name,
            )? {
                // Only an unambiguous receiver in this account can be activated. Never replay a transfer.
                pending_operation =
                    Some("activating Soloist (the activation request may have completed)");
                api.devices().transfer(&device.id, false).await?;
                pending_operation = None;
                if native
                    .wait_authenticated(
                        deadline.saturating_duration_since(tokio::time::Instant::now()),
                    )
                    .await
                {
                    return Ok(());
                }
                native.authentication_state()?;
                return Err(Error::NativeAuthenticationRequired);
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    });
    let result = tokio::select! {
        biased;
        authenticated = native.wait_authenticated(timeout) => {
            if authenticated { Ok(()) } else { Err(Error::Timeout) }
        }
        result = verification => result.unwrap_or(Err(Error::Timeout)),
    };
    match result {
        Ok(()) => Ok(()),
        Err(error) => match (native.authentication_state(), pending_operation) {
            (Err(actual), _) => Err(actual),
            (Ok(true), _) => Ok(()),
            (Ok(false), Some(operation)) if error == Error::Timeout => {
                Err(Error::NativeStartupTimeout(operation))
            }
            (Ok(false), None) if error == Error::Timeout => {
                Err(Error::NativeAuthenticationRequired)
            }
            _ => Err(error),
        },
    }
}

pub(crate) fn startup_error(native: Option<Error>, fallback: Error) -> Error {
    match native {
        Some(native) => Error::NativeStartupFailed {
            native: Box::new(native),
            fallback: Box::new(fallback),
        },
        None => fallback,
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
        let device = crate::spotify::native_device(self.web.api(), native).await?;
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

fn queue_command(command: &Command) -> bool {
    matches!(
        command,
        Command::AddToQueue(_)
            | Command::ClearQueue
            | Command::RemoveFromQueue(_)
            | Command::InsertIntoQueue { .. }
            | Command::ReorderQueue { .. }
            | Command::SkipToQueue(_)
    )
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
            | Command::SetRepeat(_)
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
            if queue_command(&command) {
                return self.web.api().queues().execute(command).await;
            }
            #[cfg(feature = "native")]
            if matches!(command, Command::Load(_) | Command::PlayContext { .. })
                && let Some(native) = &self.native
            {
                // Context replacement belongs to Connect, even when a browser supplies services.
                let device = crate::spotify::native_device(self.web.api(), native).await?;
                return self.web.dispatch_on(&device.id, command).await;
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
                    #[cfg(feature = "native")]
                    if let Some(native) = &self.native {
                        snapshot.device_id = native.device_id().map(str::to_owned);
                        snapshot.device_name = native.device_name().map(str::to_owned);
                    }
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
            Some(native) => native.current_snapshot().await.map(|mut state| {
                state.device_id = native.device_id().map(str::to_owned);
                state.device_name = native.device_name().map(str::to_owned);
                state
            }),
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
                && state
                    .device_id
                    .as_ref()
                    .zip(web.device_id.as_ref())
                    .is_some_and(|(local, remote)| local == remote)
                && web.volume.is_some()
            {
                state.volume = web.volume;
            }
            if let Ok(Some(web)) = &web
                && state
                    .device_id
                    .as_ref()
                    .zip(web.device_id.as_ref())
                    .is_none_or(|(local, remote)| local == remote)
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
        Ok(self.backend.web.api().queues().snapshot().await?.queue)
    }
    pub async fn volume(&self) -> Result<f64, Error> {
        #[cfg(feature = "native")]
        if let Some(native) = &self.backend.native {
            let device = crate::spotify::native_device(self.backend.web.api(), native).await?;
            if let Some(volume) = device.volume {
                return Ok(volume);
            }
            if !device.supports_volume {
                return Err(Error::Unsupported);
            }
            return native
                .current_snapshot()
                .await
                .and_then(|state| state.volume)
                .ok_or(Error::Unsupported);
        }
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

    #[tokio::test]
    async fn local_identity_comes_from_the_player_and_activation_is_explicit_and_paused() {
        use serde_json::json;
        let browser = crate::browser_test::MockBrowser::start().await;
        browser.response.lock().unwrap()["devices"] = json!([
            {"id":"owned-web", "name":"Web Player", "isLocal":true, "isActive":false},
            {"id":"phone", "name":"Phone", "isLocal":false, "isActive":true}
        ]);
        let player_api = Api::connect(&browser.endpoint).await.unwrap();
        let backend = WebBackend::new(player_api.clone()).await.unwrap();
        let mut sdk = SpotCore::new(player_api, backend);
        sdk.api = Api::from_cookie(
            "test-cookie",
            &crate::Paths::new(std::env::temp_dir().join("spotcore-readiness-test")),
        )
        .unwrap();
        assert_eq!(sdk.local_device().await.unwrap().unwrap().id, "owned-web");
        assert!(
            !browser
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|(_, args)| args["service"] == "ConnectTransferAPI")
        );
        sdk.activate_local().await.unwrap();
        {
            let calls = browser.calls.lock().unwrap();
            let (_, args) = calls
                .iter()
                .find(|(_, args)| args["service"] == "ConnectTransferAPI")
                .unwrap();
            assert_eq!(args["args"], json!(["owned-web", {"playOnTransfer":false}]));
        }
        assert!(sdk.local_device().await.unwrap().unwrap().active);
        browser.response.lock().unwrap()["devices"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"ambiguous", "name":"Another", "isLocal":true, "isActive":false}));
        assert!(sdk.local_device().await.is_err());
    }

    #[test]
    fn native_failure_survives_a_browser_fallback_failure() {
        let error = startup_error(
            Some(Error::NativeAuthenticationRequired),
            Error::DrmUnavailable("missing DRM".into()),
        );
        assert!(
            matches!(&error, Error::NativeStartupFailed { native, fallback } if **native == Error::NativeAuthenticationRequired && matches!(**fallback, Error::DrmUnavailable(_)))
        );
        assert!(error.to_string().contains("Soloist"));
        assert!(error.to_string().contains("missing DRM"));
        assert_eq!(
            startup_error(None, Error::Disconnected),
            Error::Disconnected
        );
    }

    #[cfg(feature = "native")]
    #[tokio::test]
    async fn connect_activation_requires_daemon_confirmation_and_is_not_replayed() {
        use futures_util::{SinkExt, StreamExt};
        use serde_json::json;
        use tokio_tungstenite::tungstenite::Message;
        for (name, confirm, initially_authenticated, hold, expected) in [
            ("Test native", true, false, None, Ok(())),
            (
                "Test native",
                false,
                false,
                None,
                Err(Error::NativeAuthenticationRequired),
            ),
            (
                "Unregistered device",
                false,
                false,
                None,
                Err(Error::NativeAuthenticationRequired),
            ),
            ("Test native", false, true, None, Ok(())),
            (
                "Test native",
                true,
                false,
                Some("ConnectTransferAPI"),
                Ok(()),
            ),
            (
                "Test native",
                true,
                false,
                Some("ConnectDevicesAPI"),
                Ok(()),
            ),
            (
                "Test native",
                false,
                false,
                Some("ConnectDevicesAPI"),
                Err(Error::NativeStartupTimeout(
                    "discovering Spotify devices (no activation request was sent)",
                )),
            ),
            (
                "Test native",
                false,
                false,
                Some("ConnectTransferAPI"),
                Err(Error::NativeStartupTimeout(
                    "activating Soloist (the activation request may have completed)",
                )),
            ),
        ] {
            let browser = super::test_browser::MockBrowser::start().await;
            browser.response.lock().unwrap()["hold_service"] = serde_json::json!(hold);
            let api = Api::connect(&browser.endpoint).await.unwrap();
            let calls = browser.calls.clone();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("ws://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                socket
                    .send(Message::Text(
                        json!({"type":"auth_state","logged_in":initially_authenticated})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                let mut sent = false;
                loop {
                    tokio::select! {
                        packet = socket.next() => if packet.is_none() { break; },
                        _ = tokio::time::sleep(Duration::from_millis(5)) => {
                            let transferred = calls.lock().unwrap().iter().any(|(method, args)| method == "service.call" && args["service"] == "ConnectTransferAPI" && args["method"] == "transferPlayback");
                            let discovery_pending = calls.lock().unwrap().iter().any(|(method, args)| method == "service.call" && args["service"] == "ConnectDevicesAPI");
                            if confirm && (transferred || (hold == Some("ConnectDevicesAPI") && discovery_pending)) && !sent {
                                socket.send(Message::Text(json!({"type":"auth_state","logged_in":true}).to_string().into())).await.unwrap();
                                sent = true;
                            }
                        }
                    }
                }
            });
            let native = tokio::time::timeout(
                Duration::from_millis(250),
                crate::native::NativeBackend::connect_named(&endpoint, name),
            )
            .await
            .expect("socket connection must not wait for authentication")
            .unwrap();
            if initially_authenticated {
                assert!(native.wait_authenticated(Duration::from_secs(1)).await);
            }
            let started = tokio::time::Instant::now();
            let pending_but_confirmed = confirm && hold.is_some();
            let timeout = if pending_but_confirmed {
                Duration::from_secs(2)
            } else {
                Duration::from_millis(200)
            };
            assert_eq!(authenticate_native(&api, &native, timeout).await, expected);
            if pending_but_confirmed {
                assert!(
                    started.elapsed() < Duration::from_secs(1),
                    "local authentication must not wait for a stalled HTTP operation"
                );
            }
            let calls = browser.calls.lock().unwrap();
            let transfers: Vec<_> = calls
                .iter()
                .filter(|(method, args)| {
                    method == "service.call" && args["method"] == "transferPlayback"
                })
                .collect();
            assert_eq!(
                transfers.len(),
                usize::from(
                    name == "Test native"
                        && !initially_authenticated
                        && hold != Some("ConnectDevicesAPI")
                )
            );
            if let Some((_, args)) = transfers.first() {
                assert_eq!(
                    args["args"],
                    json!(["native-test", {"playOnTransfer":false}])
                );
            }
            drop(calls);
            drop(native);
            server.abort();
        }
    }
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
        // A failed HTTP authentication must not fall through to a successful native send
        // or start the hidden browser's player.
        browser.response.lock().unwrap()["token"] = json!({});
        assert!(matches!(
            sdk.play(super::test_browser::TRACK).await,
            Err(Error::NotAuthenticated)
        ));
        assert!(matches!(
            sdk.play_context("spotify:playlist:selection", None, None, None)
                .await,
            Err(Error::NotAuthenticated)
        ));
        browser
            .response
            .lock()
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("token");
        assert!(commands.try_recv().is_err());
        sdk.pause().await.unwrap();
        let command = tokio::time::timeout(Duration::from_secs(1), commands.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(command["command"], "pause");
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
        browser.response.lock().unwrap()["token"] = json!({});
        assert!(matches!(
            sdk.clear_queue().await,
            Err(Error::NotAuthenticated)
        ));
        browser
            .response
            .lock()
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("token");
        sdk.set_speed(1.25).await.unwrap();
        sdk.set_shuffle_mode(ShuffleMode::Smart).await.unwrap();
        for mode in [RepeatMode::Context, RepeatMode::Track, RepeatMode::Off] {
            sdk.set_repeat(mode).await.unwrap();
        }
        let calls = browser.calls.lock().unwrap();
        assert!(!calls.iter().any(|(name, _)| name == "queue.clear"));
        for method in ["player.setSpeed", "player.setShuffleMode"] {
            assert!(calls.iter().any(|(name, _)| name == method));
        }
        let repeats: Vec<_> = calls
            .iter()
            .filter(|(method, _)| method == "player.setRepeat")
            .map(|(_, args)| args["mode"].clone())
            .collect();
        assert_eq!(repeats, vec![json!(1), json!(2), json!(0)]);
        assert!(commands.try_recv().is_err());
        server.abort();
    }

    #[test]
    fn routing_is_chosen_before_writing() {
        for command in [
            Command::AddToQueue(vec![]),
            Command::ClearQueue,
            Command::RemoveFromQueue(vec![]),
            Command::ReorderQueue {
                items: vec![],
                before: None,
                after: None,
            },
            Command::InsertIntoQueue {
                uris: vec![],
                before: None,
            },
            Command::SkipToQueue(QueueItem {
                uri: "spotify:track:test".into(),
                uid: "one".into(),
            }),
        ] {
            assert!(queue_command(&command));
        }
        assert!(!queue_command(&Command::Resume));
        assert!(!queue_command(&Command::Pause));
        assert!(web_control(&Command::ClearQueue));
        assert!(web_control(&Command::SetShuffleMode(ShuffleMode::Smart)));
        assert!(web_control(&Command::SetSpeed(1.25)));
        assert!(web_control(&Command::SetRepeat(RepeatMode::Off)));
        assert!(web_control(&Command::SetRepeat(RepeatMode::Context)));
        assert!(web_control(&Command::SetRepeat(RepeatMode::Track)));
        assert!(!web_control(&Command::Load("spotify:track:test".into())));
        assert!(!web_control(&Command::Pause));
    }
}
