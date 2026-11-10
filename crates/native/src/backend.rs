use crate::daemon::{DEFAULT_DEVICE_NAME, SoloistConfig, SoloistProcess};
use crate::manager::SoloistManager;
use crate::protocol::{CommandPayload, PlaybackStateEvent, SoloistServerEvent};
use futures_util::{SinkExt, StreamExt};
use spotcore_core::{
    Backend, BackendFuture, Capabilities, Command, Error, Image, MediaItem, MediaKind,
    PlaybackEvent, PlaybackState, PlayerSnapshot, RepeatMode,
};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc, watch};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

pub struct SoloistBackend {
    cmd_tx: mpsc::Sender<CommandPayload>,
    event_rx: mpsc::Receiver<PlaybackEvent>,
    snapshot: Arc<RwLock<Option<PlayerSnapshot>>>,
    /// Mirrors the daemon's `logged_in` state; commands require authentication.
    logged_in: watch::Receiver<bool>,
    // Last active-device state reported by Soloist.
    active: watch::Receiver<bool>,
    /// Connect device name passed to `launch_as`; `None` for a direct `connect`.
    device_name: Option<String>,
    _process: Option<SoloistProcess>,
    binary_lease: Option<std::fs::File>,
    reader_task: tokio::task::JoinHandle<()>,
    writer_task: tokio::task::JoinHandle<()>,
}

pub type NativeBackend = SoloistBackend;

impl SoloistBackend {
    /// Downloads or reuses Soloist, then connects to the local daemon.
    pub async fn launch(api_key: impl Into<String>) -> Result<Self, Error> {
        let paths = spotcore_core::paths::Paths::discover()?;
        Self::launch_with(api_key, &paths).await
    }

    /// Launches the daemon under a caller-chosen data root instead of the platform default.
    pub async fn launch_with(
        api_key: impl Into<String>,
        paths: &spotcore_core::paths::Paths,
    ) -> Result<Self, Error> {
        Self::launch_as(api_key, paths, DEFAULT_DEVICE_NAME).await
    }

    /// Uses the supplied Connect device name without appending a suffix.
    pub async fn launch_as(
        api_key: impl Into<String>,
        paths: &spotcore_core::paths::Paths,
        device_name: impl Into<String>,
    ) -> Result<Self, Error> {
        let paths = paths.resolve()?;
        let device_name = device_name.into().trim().to_string();
        if device_name.is_empty() {
            return Err(Error::InvalidInput("connect device name must not be empty"));
        }
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(Error::InvalidInput("native API key is empty"));
        }
        paths
            .prepare()
            .map_err(|error| Error::Backend(format!("cannot prepare native data root: {error}")))?;

        let manager = SoloistManager::new(paths.native());
        let (binary_path, lease) = manager
            .acquire_binary()
            .await
            .map_err(|e| Error::Backend(format!("Failed to prepare Soloist binary: {}", e)))?;

        let mut config = SoloistConfig::new(binary_path, api_key, &paths);
        config.device_name = device_name.clone();
        let process = SoloistProcess::spawn(config)
            .await
            .map_err(|e| Error::Backend(format!("Failed to start Soloist process: {}", e)))?;

        let mut backend = Self::connect(process.ws_url()).await?;
        backend.device_name = Some(device_name);
        backend._process = Some(process);
        backend.binary_lease = Some(lease);
        Ok(backend)
    }

    /// Waits for the owned daemon to exit before releasing its executable lease.
    pub async fn shutdown(&mut self) -> Result<(), Error> {
        if let Some(process) = &mut self._process {
            process
                .kill()
                .await
                .map_err(|e| Error::Backend(e.to_string()))?;
        }
        self._process = None;
        self.binary_lease = None;
        self.reader_task.abort();
        self.writer_task.abort();
        Ok(())
    }

    /// The Connect device name the daemon was launched with, when known.
    pub fn device_name(&self) -> Option<&str> {
        self.device_name.as_deref()
    }

    /// Attaches with the existing Connect name for Web-assisted controls; does not rename the daemon.
    pub async fn connect_named(ws_url: &str, device_name: &str) -> Result<Self, Error> {
        let name = device_name.trim();
        if name.is_empty() {
            return Err(Error::InvalidInput("connect device name must not be empty"));
        }
        let mut backend = Self::connect(ws_url).await?;
        backend.device_name = Some(name.to_owned());
        Ok(backend)
    }

    /// Connects directly to an already-running Soloist WebSocket server (e.g. `ws://127.0.0.1:9090`).
    pub async fn connect(ws_url: &str) -> Result<Self, Error> {
        let (ws_stream, _) = connect_async(ws_url)
            .await
            .map_err(|e| Error::Backend(format!("WebSocket connect error: {}", e)))?;

        let (mut write, mut read) = ws_stream.split();

        let (cmd_tx, mut cmd_rx) = mpsc::channel::<CommandPayload>(64);
        let (event_tx, event_rx) = mpsc::channel::<PlaybackEvent>(128);
        let snapshot = Arc::new(RwLock::new(None));
        let snapshot_clone = Arc::clone(&snapshot);
        let (active_tx, active_rx) = watch::channel(false);
        let active_tx = Arc::new(active_tx);
        let (logged_in_tx, logged_in_rx) = watch::channel(false);
        let logged_in_tx = Arc::new(logged_in_tx);
        // Refresh state after authentication, which may arrive after the socket opens.
        let activate_tx = cmd_tx.clone();

        let writer_task = tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                if let Ok(json) = serde_json::to_string(&cmd)
                    && write.send(Message::Text(json.into())).await.is_err()
                {
                    break;
                }
            }
        });

        let reader_task = tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                let Ok(msg) = msg else { break };
                if let Message::Text(text) = msg
                    && let Ok(event) = serde_json::from_str::<SoloistServerEvent>(&text)
                {
                    match event {
                        SoloistServerEvent::PlaybackState(state) => {
                            if let Some(v) = state.is_active {
                                let _ = active_tx.send(v);
                            }
                            let snap = Self::map_snapshot(&state);
                            {
                                let mut lock = snapshot_clone.write().await;
                                *lock = Some(snap.clone());
                            }
                            let _ = event_tx.send(PlaybackEvent::Snapshot(Box::new(snap))).await;
                        }
                        SoloistServerEvent::PlaybackChanged(p) => {
                            let new_state = match p.status.as_str() {
                                "playing" => PlaybackState::Playing,
                                "paused" => PlaybackState::Paused,
                                _ => PlaybackState::Idle,
                            };
                            {
                                let mut lock = snapshot_clone.write().await;
                                if let Some(ref mut snap) = *lock {
                                    snap.state = new_state;
                                }
                            }
                            let _ = event_tx.send(PlaybackEvent::StateChanged(new_state)).await;
                        }
                        SoloistServerEvent::TrackChanged(t) => {
                            if let Some(ref item) = t.item
                                && let Some(media_item) = Self::parse_media_item(item)
                            {
                                let snap = {
                                    let mut lock = snapshot_clone.write().await;
                                    if let Some(ref mut current) = *lock {
                                        current.duration_ms = media_item.duration_ms;
                                        current.track = Some(media_item);
                                        Some(current.clone())
                                    } else {
                                        let snap = PlayerSnapshot {
                                            state: PlaybackState::Idle,
                                            track: Some(media_item),
                                            context_uri: None,
                                            position_ms: 0,
                                            duration_ms: None,
                                            timestamp_ms: None,
                                            device_id: None,
                                            device_name: None,
                                            volume: None,
                                            buffering: false,
                                            shuffle: false,
                                            smart_shuffle: None,
                                            repeat: RepeatMode::Off,
                                            capabilities: Self::default_capabilities(),
                                            raw: serde_json::Value::Null,
                                        };
                                        *lock = Some(snap.clone());
                                        Some(snap)
                                    }
                                };
                                if let Some(snap) = snap {
                                    let _ = event_tx
                                        .send(PlaybackEvent::Snapshot(Box::new(snap)))
                                        .await;
                                }
                            }
                        }
                        SoloistServerEvent::PositionSync(p) => {
                            if let Some(pos) = p.position {
                                let mut lock = snapshot_clone.write().await;
                                if let Some(ref mut snap) = *lock {
                                    snap.position_ms = pos.position_ms;
                                    snap.timestamp_ms = Some(pos.timestamp_ms);
                                }
                            }
                        }
                        SoloistServerEvent::AuthState(e) => {
                            if let Some(v) = e.get("is_active").and_then(serde_json::Value::as_bool)
                            {
                                let _ = active_tx.send(v);
                            }
                            if update_login_state(&logged_in_tx, &e) == Some(true) {
                                let _ = activate_tx.send(CommandPayload::get_state()).await;
                            }
                            let _ = event_tx.send(PlaybackEvent::SessionChanged).await;
                        }
                        SoloistServerEvent::DeviceChanged(e) => {
                            if let Some(v) = e.is_active {
                                let _ = active_tx.send(v);
                            }
                        }
                        _ => {}
                    }
                }
            }
        });

        // Request the initial snapshot; Connect activation belongs to the session coordinator.
        let _ = cmd_tx.send(CommandPayload::get_state()).await;

        let backend = Self {
            cmd_tx,
            event_rx,
            snapshot,
            logged_in: logged_in_rx,
            active: active_rx,
            device_name: None,
            _process: None,
            binary_lease: None,
            reader_task,
            writer_task,
        };

        Ok(backend)
    }

    /// A live control socket is not proof that the daemon has authenticated.
    pub fn authentication_state(&self) -> Result<bool, Error> {
        self.logged_in
            .has_changed()
            .map_err(|_| Error::Disconnected)?;
        Ok(*self.logged_in.borrow())
    }

    /// Waits until the daemon confirms login authentication, up to `dur`.
    pub async fn wait_authenticated(&self, dur: std::time::Duration) -> bool {
        let mut rx = self.logged_in.clone();
        if *rx.borrow_and_update() {
            return true;
        }
        let deadline = tokio::time::Instant::now() + dur;
        while tokio::time::Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(remaining, rx.changed()).await {
                Ok(Ok(())) => {
                    if *rx.borrow() {
                        return true;
                    }
                }
                _ => return false,
            }
        }
        false
    }

    /// Waits until the daemon reports active-device status, up to `dur`.
    pub async fn wait_active(&self, dur: std::time::Duration) -> bool {
        let mut rx = self.active.clone();
        if *rx.borrow_and_update() {
            return true;
        }
        let deadline = tokio::time::Instant::now() + dur;
        while tokio::time::Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            match tokio::time::timeout(remaining, rx.changed()).await {
                Ok(Ok(())) => {
                    if *rx.borrow() {
                        return true;
                    }
                }
                _ => return false,
            }
        }
        false
    }

    pub async fn current_snapshot(&self) -> Option<PlayerSnapshot> {
        self.snapshot.read().await.clone()
    }

    fn default_capabilities() -> Capabilities {
        Capabilities {
            load: true,
            stop: true,
            resume: true,
            pause: true,
            seek: true,
            next: true,
            previous: true,
            volume: true,
            shuffle: true,
            repeat: true,
            queue: true,
            ..Default::default()
        }
    }

    fn parse_media_item(item: &serde_json::Value) -> Option<MediaItem> {
        let uri = item
            .get("uri")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let name = item
            .pointer("/decorations/identity/name")
            .or_else(|| item.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();

        let images = item
            .pointer("/decorations/visual_identity/cover")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|img| {
                        let url = img.get("url")?.as_str()?.to_string();
                        Some(Image {
                            url,
                            width: None,
                            height: None,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let duration_ms = item
            .pointer("/playback/duration_ms")
            .or_else(|| item.get("duration_ms"))
            .and_then(|v| v.as_u64());

        Some(MediaItem {
            uri,
            name,
            kind: MediaKind::Track,
            images,
            duration_ms,
            uid: None,
            raw: item.clone(),
        })
    }

    fn map_snapshot(ev: &PlaybackStateEvent) -> PlayerSnapshot {
        let state = match ev.status.as_str() {
            "playing" => PlaybackState::Playing,
            "paused" => PlaybackState::Paused,
            _ => PlaybackState::Idle,
        };

        let track = ev.item.as_ref().and_then(Self::parse_media_item);

        let duration_ms = track.as_ref().and_then(|t| t.duration_ms);
        let position_ms = ev.position.as_ref().map(|p| p.position_ms).unwrap_or(0);
        let timestamp_ms = ev.position.as_ref().map(|p| p.timestamp_ms);
        let buffering = ev.status == "buffering";
        let shuffle = ev.options.as_ref().map(|o| o.shuffle).unwrap_or(false);
        let repeat = match ev.options.as_ref().map(|o| o.repeat.as_str()) {
            Some("track") => RepeatMode::Track,
            Some("context") => RepeatMode::Context,
            _ => RepeatMode::Off,
        };

        let capabilities = Self::default_capabilities();

        PlayerSnapshot {
            state,
            track,
            context_uri: None,
            position_ms,
            duration_ms,
            timestamp_ms,
            device_id: None,
            device_name: None,
            volume: None,
            buffering,
            shuffle,
            smart_shuffle: None,
            repeat,
            capabilities,
            raw: serde_json::Value::Null,
        }
    }
}

impl Drop for SoloistBackend {
    fn drop(&mut self) {
        self.reader_task.abort();
        self.writer_task.abort();
        if let Some(mut process) = self._process.take() {
            let lease = self.binary_lease.take();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _lease = lease;
                    let _ = process.kill().await;
                });
            }
        }
    }
}

impl Backend for SoloistBackend {
    fn capabilities(&self) -> Capabilities {
        Self::default_capabilities()
    }

    fn execute(&mut self, command: Command) -> BackendFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            if !self
                .wait_authenticated(std::time::Duration::from_secs(10))
                .await
            {
                return Err(Error::NotAuthenticated);
            }

            let payload = match command {
                Command::Load(uri) => CommandPayload::play_uri(uri),
                Command::Resume => CommandPayload::resume(),
                Command::Pause | Command::Stop => CommandPayload::pause(),
                Command::Seek(pos) => CommandPayload::seek(pos.as_millis() as u64),
                Command::Next => CommandPayload::skip_next(),
                Command::Previous => CommandPayload::skip_prev(),
                Command::SetVolume(vol) => {
                    if !vol.is_finite() || !(0.0..=1.0).contains(&vol) {
                        return Err(Error::InvalidInput("volume must be 0..=1"));
                    }
                    CommandPayload::set_volume((vol * 100.0).round() as u32)
                }
                Command::SetShuffle(s) => CommandPayload::set_shuffle(s),
                Command::SetShuffleMode(spotcore_core::ShuffleMode::Off) => {
                    CommandPayload::set_shuffle(false)
                }
                Command::SetShuffleMode(spotcore_core::ShuffleMode::On) => {
                    CommandPayload::set_shuffle(true)
                }
                Command::SetRepeat(r) => match r {
                    RepeatMode::Track => CommandPayload::set_repeat_track(true),
                    RepeatMode::Context => CommandPayload::set_repeat_context(true),
                    RepeatMode::Off => CommandPayload::set_repeat_context(false),
                    _ => return Err(Error::Unsupported),
                },
                Command::AddToQueue(uris) => {
                    for uri in uris {
                        self.cmd_tx
                            .send(CommandPayload::add_to_queue(uri))
                            .await
                            .map_err(|_| Error::Disconnected)?;
                    }
                    return Ok(());
                }
                _ => return Err(Error::Unsupported),
            };

            self.cmd_tx
                .send(payload)
                .await
                .map_err(|_| Error::Disconnected)?;
            Ok(())
        })
    }

    fn next_event(&mut self) -> BackendFuture<'_, Result<PlaybackEvent, Error>> {
        Box::pin(async move { self.event_rx.recv().await.ok_or(Error::Disconnected) })
    }
}

// Auth events can be partial; only an explicit login field changes authentication.
fn update_login_state(sender: &watch::Sender<bool>, event: &serde_json::Value) -> Option<bool> {
    let logged_in = event.get("logged_in")?.as_bool()?;
    sender.send_replace(logged_in);
    Some(logged_in)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_auth_events_preserve_login_but_explicit_logout_clears_it() {
        let (sender, state) = watch::channel(false);
        assert_eq!(
            update_login_state(&sender, &serde_json::json!({"logged_in":true})),
            Some(true)
        );
        assert!(*state.borrow());
        assert_eq!(
            update_login_state(&sender, &serde_json::json!({"is_active":false})),
            None
        );
        assert!(*state.borrow());
        assert_eq!(
            update_login_state(&sender, &serde_json::json!({"logged_in":false})),
            Some(false)
        );
        assert!(!*state.borrow());
    }

    #[test]
    fn queue_add_does_not_advertise_queue_editing() {
        let capabilities = SoloistBackend::default_capabilities();
        assert!(capabilities.supports(&Command::AddToQueue(vec![])));
        assert!(!capabilities.supports(&Command::ClearQueue));
        assert!(!capabilities.supports(&Command::RemoveFromQueue(vec![])));
        assert!(capabilities.supports(&Command::SetShuffleMode(spotcore_core::ShuffleMode::On)));
        assert!(
            !capabilities.supports(&Command::SetShuffleMode(spotcore_core::ShuffleMode::Smart))
        );
    }

    #[tokio::test]
    async fn launch_as_rejects_blank_device_name() {
        let paths =
            spotcore_core::paths::Paths::new(std::env::temp_dir().join("spotcore_blank_name"));
        let result = SoloistBackend::launch_as("key", &paths, "   ").await;
        assert!(matches!(result, Err(Error::InvalidInput(_))));
    }

    #[tokio::test]
    async fn closed_socket_is_not_reported_as_missing_authentication() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            socket.close(None).await.unwrap();
        });
        let backend = SoloistBackend::connect(&endpoint).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while backend.authentication_state() != Err(Error::Disconnected) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        server.await.unwrap();
    }
}
