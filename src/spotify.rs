use crate::AudioQuality;
use crate::web::host::{HostLauncher, ManagedHost};
use crate::web::{Api, WebBackend};
use crate::{Backend, Client, Error, Paths};
use std::path::{Path, PathBuf};

/// One data API, with the selected playback backend and optional managed host process.
pub struct Spotify<B: Backend> {
    pub api: Api,
    pub player: Client<B>,
    pub host: Option<ManagedHost>,
}

impl<B: Backend> Spotify<B> {
    /// Sets shuffle on the active Connect context, including native playback.
    pub async fn set_shuffle_mode(&self, mode: crate::ShuffleMode) -> Result<(), Error> {
        WebBackend::new(self.api.clone())
            .await?
            .set_shuffle_mode(mode)
            .await
    }

    pub fn new(api: Api, backend: B) -> Self {
        Self {
            api,
            player: Client::new(backend),
            host: None,
        }
    }

    pub fn with_host(api: Api, backend: B, host: ManagedHost) -> Self {
        Self {
            api,
            player: Client::new(backend),
            host: Some(host),
        }
    }
}

use crate::session::SpotCore;

impl Spotify<WebBackend> {
    /// Connects to an existing, already running SpotCore host instance over CDP (e.g. `http://127.0.0.1:41235`).
    pub async fn connect_web(endpoint: &str) -> Result<Self, Error> {
        Self::connect_web_with(endpoint, None).await
    }

    async fn connect_web_with(endpoint: &str, paths: Option<&Paths>) -> Result<Self, Error> {
        let api = connect_api(
            endpoint,
            paths,
            std::time::Duration::from_secs(10),
            crate::web::WebPlayerOptions {
                minimal_ui: false,
                device_name: None,
            },
            crate::web::BrowserRole::Playback,
        )
        .await?;
        let backend = WebBackend::new(api.clone()).await?;
        Ok(Self::new(api, backend))
    }

    /// Alias for `connect_web`: connects to an existing CDP endpoint.
    pub async fn connect(endpoint: &str) -> Result<Self, Error> {
        Self::connect_web(endpoint).await
    }

    pub fn builder() -> SpotCoreBuilder {
        SpotCoreBuilder::new()
    }

    /// Spawns a managed host with default configuration and connects to it.
    pub async fn launch() -> Result<Self, Error> {
        Self::builder().launch_web().await
    }

    /// Opens an interactive login window to sign into Spotify and store the profile.
    pub async fn login(profile_dir: Option<&Path>) -> Result<(), Error> {
        crate::web::host::login_interactive(None, profile_dir).await
    }
}

/// Accepted quality preference; this is not a measurement of the audio stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppliedQuality {
    pub requested: AudioQuality,
    /// Native readback, or the accepted Web preference after the lossless clamp.
    pub effective: AudioQuality,
    /// False when Spotify reports the device or account cannot stream lossless.
    pub lossless_available: bool,
}

/// Legacy native controls require a matching signed-in Web account.
#[cfg(feature = "native")]
impl Spotify<crate::native::NativeBackend> {
    /// Clears the active Spotify Connect queue through the Web session.
    /// Soloist must be the active Connect device.
    pub async fn clear_queue(&self) -> Result<(), Error> {
        WebBackend::new(self.api.clone()).await?.clear_queue().await
    }

    /// Removes rows from the active Spotify Connect queue through the Web session.
    /// Use the URI and UID from that session's queue; Soloist must be active.
    pub async fn remove_from_queue(&self, items: &[crate::QueueItem]) -> Result<(), Error> {
        WebBackend::new(self.api.clone())
            .await?
            .remove_from_queue(items)
            .await
    }

    /// Writes and reads back native quality; ineligible lossless requests clamp to `VeryHigh`.
    pub async fn set_quality(&mut self, quality: AudioQuality) -> Result<AppliedQuality, Error> {
        apply_native_quality(&self.api, self.player.backend(), quality).await
    }
}

#[cfg(feature = "native")]
pub(crate) async fn apply_native_quality(
    api: &Api,
    backend: &crate::native::NativeBackend,
    quality: AudioQuality,
) -> Result<AppliedQuality, Error> {
    let device = native_device(api, backend).await?;
    let lossless_available = device.lossless_supported && device.lossless_eligible;
    let requested = if lossless_available {
        quality
    } else {
        match quality {
            AudioQuality::Lossless | AudioQuality::HiRes => AudioQuality::VeryHigh,
            quality => quality,
        }
    };
    let effective = api.devices().set_audio_quality(&device, requested).await?;
    Ok(AppliedQuality {
        requested: quality,
        effective,
        lossless_available,
    })
}

#[cfg(feature = "native")]
pub(crate) async fn native_device(
    api: &Api,
    backend: &crate::native::NativeBackend,
) -> Result<crate::web::Device, Error> {
    let name = backend
        .device_name()
        .ok_or_else(|| Error::Backend("native quality needs a launched device name".into()))?;
    // The daemon registers with Connect shortly after its session opens.
    let mut last_error = None;
    for _ in 0..20 {
        let devices = api.devices().list().await?;
        match named_device(devices, name) {
            Ok(Some(device)) => return Ok(device),
            Ok(None) => {}
            Err(error) => last_error = Some(error),
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    Err(last_error.unwrap_or_else(|| {
        Error::Backend(format!(
            "Connect device {name:?} did not appear; is the daemon signed in?"
        ))
    }))
}

#[cfg(feature = "native")]
pub(crate) fn named_device(
    devices: Vec<crate::web::Device>,
    name: &str,
) -> Result<Option<crate::web::Device>, Error> {
    let mut matches: Vec<_> = devices
        .into_iter()
        .filter(|device| device.name == name)
        .collect();
    if matches.len() > 1 {
        // The local Web page shares the native name; the daemon is the remote entry.
        let remote: Vec<_> = matches
            .iter()
            .filter(|device| !device.local)
            .map(|device| device.id.clone())
            .collect();
        if let [id] = remote.as_slice() {
            matches.retain(|device| &device.id == id);
        }
    }
    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.pop()),
        _ => Err(Error::Backend(format!(
            "multiple Connect devices named {name:?}; set a distinct device_name"
        ))),
    }
}

#[cfg(all(test, feature = "native"))]
mod quality_tests {
    use super::*;

    fn device(id: &str) -> crate::web::Device {
        crate::web::Device {
            id: id.into(),
            name: "Soloist".into(),
            kind: "computer".into(),
            active: false,
            local: false,
            volume: None,
            lossless_supported: false,
            lossless_eligible: false,
            restricted: false,
            supports_volume: true,
            raw: Default::default(),
        }
    }

    #[test]
    fn quality_targets_only_a_unique_device() {
        assert!(
            named_device(vec![device("one")], "other")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            named_device(vec![device("one")], "Soloist")
                .unwrap()
                .unwrap()
                .id,
            "one"
        );
        assert!(named_device(vec![device("one"), device("two")], "Soloist").is_err());
    }

    #[test]
    fn shared_name_prefers_the_remote_native_device() {
        let mut web = device("web");
        web.local = true;
        let devices = vec![web, device("native")];
        assert_eq!(
            named_device(devices, "Soloist").unwrap().unwrap().id,
            "native"
        );
    }
}

#[cfg(test)]
mod builder_tests {
    use super::*;

    #[tokio::test]
    async fn service_startup_needs_no_browser_or_native_runtime() {
        let paths = Paths::new(
            std::env::temp_dir().join(format!("spotcore-builder-http-{}", std::process::id())),
        );
        let session = serde_json::from_value(serde_json::json!({"accessToken":"test", "expiresAtMs":u64::MAX, "clientId":"client", "appVersion":"version", "isAnonymous":false})).unwrap();
        let api = Api::from_session(session, &paths).unwrap();
        let mut sdk = SpotCoreBuilder::new()
            .data_dir(paths.root())
            .browser_role(crate::BrowserRole::Services)
            .host_path("missing-browser-backend")
            .launch_with_services(api)
            .await
            .unwrap();
        assert!(sdk.host.is_none() && sdk.api.browser().is_err());
        assert!(matches!(sdk.next_event().await, Err(Error::Unsupported)));
        sdk.shutdown().await.unwrap();
    }

    #[test]
    fn managed_playback_defaults_to_spotcore_name() {
        for builder in [SpotCoreBuilder::new(), SpotCoreBuilder::default()] {
            assert_eq!(builder.web_options.device_name.as_deref(), Some("SpotCore"));
        }
        #[cfg(feature = "native")]
        assert_eq!(crate::native::DEFAULT_DEVICE_NAME, "SpotCore");
    }

    #[test]
    fn device_name_applies_to_web_and_native() {
        let builder = SpotCoreBuilder::new().device_name("My App");
        assert_eq!(builder.web_options.device_name.as_deref(), Some("My App"));
        assert_eq!(builder.device_name.as_deref(), Some("My App"));
        let builder = builder.session_cookie("private-cookie");
        assert!(!format!("{builder:?}").contains("private-cookie"));
    }
}

#[derive(Debug, Clone, Default)]
pub struct SpotCoreBuilder {
    launcher: HostLauncher,
    web_options: crate::web::WebPlayerOptions,
    endpoint: Option<String>,
    role: crate::web::BrowserRole,
    engine: crate::web::BrowserEngine,
    paths: Option<Paths>,
    #[cfg(feature = "native")]
    native_credentials: Option<crate::SoloistCredentials>,
    device_name: Option<String>,
    web_credentials: Option<crate::WebCredentials>,
}

impl SpotCoreBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn engine(mut self, engine: crate::web::BrowserEngine) -> Self {
        self.engine = engine;
        self.launcher = self.launcher.engine(engine);
        self
    }

    pub fn browser_role(mut self, role: crate::web::BrowserRole) -> Self {
        self.role = role;
        self.launcher = self.launcher.browser_role(role);
        self
    }

    pub fn web_options(mut self, options: crate::web::WebPlayerOptions) -> Self {
        self.web_options = options;
        self
    }

    /// Enables native playback for a freshly verified Premium session. No binary is bundled.
    #[cfg(feature = "native")]
    pub fn native(mut self, api_key: impl Into<String>) -> Self {
        self.native_credentials = Some(crate::SoloistCredentials {
            api_key: Some(api_key.into()),
        });
        self
    }

    #[cfg(feature = "native")]
    pub(crate) fn with_native_credentials(
        mut self,
        credentials: crate::SoloistCredentials,
    ) -> Self {
        self.native_credentials = Some(credentials);
        self
    }

    /// Sets the Spotify Connect name for managed Web and native playback.
    pub fn device_name(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        self.web_options.device_name = Some(name.clone());
        self.device_name = Some(name);
        self
    }

    pub async fn login(self) -> Result<(), Error> {
        self.launcher.login().await
    }

    /// Uses HTTP services and optional native playback without launching a browser.
    pub async fn launch_http(self, api: Api) -> Result<SpotCore, Error> {
        if api.browser().is_ok() {
            return Err(Error::InvalidInput("launch_http requires an HTTP-only API"));
        }
        api.session().await?;
        let backend = WebBackend::with_role(api.clone(), crate::BrowserRole::Services).await?;
        let sdk = SpotCore::new(api, backend);
        #[cfg(feature = "native")]
        let mut sdk = sdk;
        #[cfg(feature = "native")]
        if let Some(credentials) = self.native_credentials {
            let paths = self
                .paths
                .map(Ok)
                .unwrap_or_else(Paths::discover)?
                .resolve()?;
            paths.prepare().map_err(|error| {
                Error::Backend(format!("cannot prepare data directory: {error}"))
            })?;
            sdk.try_native(credentials, &paths, self.device_name.as_deref())
                .await;
        }
        Ok(sdk)
    }

    /// Starts HTTP services for cookie-backed Services mode, or selects local playback.
    /// An opted-in native backend is tried before Web when a cookie is supplied.
    pub async fn launch(mut self) -> Result<SpotCore, Error> {
        #[cfg(feature = "native")]
        let native = self.native_credentials.clone();
        #[cfg(feature = "native")]
        let name = self.device_name.clone();
        let paths = self
            .paths
            .clone()
            .map(Ok)
            .unwrap_or_else(Paths::discover)?
            .resolve()?;
        self.launcher = self.launcher.data_dir(paths.root());
        self.paths = Some(paths.clone());
        paths
            .prepare()
            .map_err(|e| Error::Backend(format!("cannot prepare data directory: {e}")))?;
        if self.endpoint.is_none()
            && let Some(cookie) = self
                .web_credentials
                .as_ref()
                .and_then(|credentials| credentials.sp_dc.as_ref())
        {
            let api = Api::from_cookie(cookie.clone(), &paths)?;
            return self.launch_with_services(api).await;
        }
        let role = self.role;
        let web = self.launch_web().await?;
        let sdk = SpotCore::from_web(web);
        #[cfg(feature = "native")]
        let mut sdk = sdk;
        #[cfg(feature = "native")]
        if let Some(credentials) = native {
            sdk.try_native(credentials, &paths, name.as_deref()).await;
        }
        if role == crate::web::BrowserRole::Playback
            && sdk.playback_mode() == crate::PlaybackMode::Web
        {
            sdk.require_web_playback().await?;
        }
        Ok(sdk)
    }

    pub(crate) async fn launch_with_services(self, api: Api) -> Result<SpotCore, Error> {
        if self.role == crate::BrowserRole::Services {
            return self.launch_http(api).await;
        }
        #[cfg(feature = "native")]
        let mut self_builder = self;
        #[cfg(not(feature = "native"))]
        let self_builder = self;
        #[cfg(feature = "native")]
        let mut fallback = None;
        #[cfg(not(feature = "native"))]
        let fallback = None;
        #[cfg(feature = "native")]
        if self_builder.native_credentials.is_some() {
            let session = self_builder.clone().launch_http(api).await?;
            if session.playback_mode() == crate::PlaybackMode::Native {
                return Ok(session);
            }
            fallback = session.native_fallback_reason().cloned();
            self_builder.native_credentials = None;
        }
        let web = self_builder
            .launch_web()
            .await
            .map_err(|error| crate::session::startup_error(fallback.clone(), error))?;
        let session = SpotCore::from_web(web);
        #[cfg(feature = "native")]
        let mut session = session;
        #[cfg(feature = "native")]
        {
            session.native_fallback = fallback;
        }
        session.require_web_playback().await?;
        Ok(session)
    }

    /// Explicit backend executable, not the common `spotcore-engine` launcher.
    pub fn host_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.launcher = self.launcher.host_path(path);
        self
    }

    /// Overrides only the browser profile; the data root remains unchanged.
    pub fn profile_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.launcher = self.launcher.profile_dir(dir);
        self
    }

    /// Data root for the profile, the engine search and the operation cache.
    pub fn app_id(self, app: &str) -> Result<Self, Error> {
        Ok(self.data_dir(Paths::for_app(app)?.root()))
    }

    pub fn data_dir(mut self, root: impl Into<PathBuf>) -> Self {
        let paths = Paths::new(root);
        self.launcher = self.launcher.data_dir(paths.root());
        self.paths = Some(paths);
        self
    }

    /// Explicit path to Widevine CDM library (auto-discovered if omitted).
    pub fn widevine_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.launcher = self.launcher.widevine_path(path);
        self
    }

    /// Explicit CDP port to bind (an ephemeral port is allocated if omitted).
    pub fn cdp_port(mut self, port: u16) -> Self {
        self.launcher = self.launcher.cdp_port(port);
        self
    }

    /// Disables resource blocking cuts (debugging only).
    pub fn no_cuts(mut self, no_cuts: bool) -> Self {
        self.launcher = self.launcher.no_cuts(no_cuts);
        self.web_options.minimal_ui = !no_cuts;
        self
    }

    /// Supplies an app-owned `sp_dc` cookie for HTTP services and managed Web playback.
    /// Interactive login is optional; Services mode does not start a browser.
    pub fn session_cookie(mut self, cookie: impl Into<String>) -> Self {
        let cookie = cookie.into();
        self.launcher = self.launcher.session_cookie(cookie.clone());
        self.web_credentials = Some(crate::WebCredentials {
            sp_dc: Some(cookie),
            access_token: None,
            expires_at_ms: None,
        });
        self
    }

    /// Hides supported browser windows (default: true); OS adapters may still need a display.
    pub fn headless(mut self, headless: bool) -> Self {
        self.launcher = self.launcher.headless(headless);
        self
    }

    /// Instead of spawning a new host, connects to an already running CDP endpoint.
    pub fn connect_to(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into());
        self
    }

    /// Spawns the host (or connects to `connect_to`) and initializes the SpotCore client.
    pub(crate) async fn launch_web(mut self) -> Result<Spotify<WebBackend>, Error> {
        if let Some(name) = self.device_name.clone() {
            self.web_options.device_name = Some(name);
        }
        if self.engine == crate::web::BrowserEngine::System {
            if self.role != crate::web::BrowserRole::Services {
                return Err(Error::InvalidInput(
                    "system engines currently require service-only mode",
                ));
            }
            self.web_options.minimal_ui = false;
        }

        let paths = self
            .paths
            .clone()
            .map(Ok)
            .unwrap_or_else(Paths::discover)?
            .resolve()?;
        paths.validate()?;
        self.launcher = self.launcher.data_dir(paths.root());
        self.paths = Some(paths);
        if let Some(ref endpoint) = self.endpoint {
            let api = connect_api(
                endpoint,
                self.paths.as_ref(),
                std::time::Duration::from_secs(10),
                crate::web::WebPlayerOptions {
                    minimal_ui: false,
                    device_name: None,
                },
                self.role,
            )
            .await?;
            let backend = WebBackend::with_role(api.clone(), self.role).await?;
            return Ok(Spotify::new(api, backend));
        }

        self.launch_candidates(self.launcher.candidates()?).await
    }

    async fn launch_candidates(
        &self,
        candidates: Vec<HostLauncher>,
    ) -> Result<Spotify<WebBackend>, Error> {
        let mut failures = Vec::new();
        for candidate in candidates {
            match self.launch_candidate(candidate.clone()).await {
                Ok(client) => return Ok(client),
                // Changing engines cannot repair credentials or invalid app configuration.
                Err(error @ (Error::NotAuthenticated | Error::InvalidInput(_))) => {
                    return Err(error);
                }
                Err(error) => failures.push(candidate.failure(error)),
            }
        }
        Err(HostLauncher::unavailable(failures, self.role))
    }

    async fn launch_candidate(&self, launcher: HostLauncher) -> Result<Spotify<WebBackend>, Error> {
        let managed_host = launcher
            .clone()
            .defer_navigation(self.web_options.minimal_ui)
            .spawn_configured()
            .await?;
        let options = if managed_host.engine() == crate::web::BrowserEngine::System {
            crate::web::WebPlayerOptions {
                minimal_ui: false,
                device_name: None,
            }
        } else {
            self.web_options.clone()
        };
        let result = async {
            launcher.seed_session(&managed_host).await?;
            let api = connect_api(
                managed_host.cdp_url(),
                self.paths.as_ref(),
                std::time::Duration::from_secs(30),
                options,
                self.role,
            )
            .await?;
            let backend = WebBackend::with_role(api.clone(), self.role).await?;
            #[cfg(feature = "native")]
            let native_requested = self.native_credentials.is_some();
            #[cfg(not(feature = "native"))]
            let native_requested = false;
            if self.role == crate::web::BrowserRole::Playback && !native_requested {
                backend.drm_support().await?.require_playback()?;
            }
            Ok((api, backend))
        }
        .await;
        match result {
            Ok((api, backend)) => Ok(Spotify::with_host(api, backend, managed_host)),
            Err(error) => {
                let mut host = managed_host;
                host.shutdown().await?;
                Err(error)
            }
        }
    }
}

async fn connect_api(
    endpoint: &str,
    paths: Option<&Paths>,
    timeout: std::time::Duration,
    options: crate::web::WebPlayerOptions,
    role: crate::web::BrowserRole,
) -> Result<Api, Error> {
    let start = std::time::Instant::now();
    let mut last_error = Error::Disconnected;

    let api = loop {
        if start.elapsed() > timeout {
            return Err(last_error);
        }
        let connected =
            match crate::web::Browser::connect_with_role(endpoint, options.clone(), role).await {
                Ok(browser) => match paths {
                    Some(paths) => Api::with_paths(browser, paths),
                    None => Api::new(browser),
                },
                Err(error) => Err(error),
            };
        match connected {
            Ok(api) => match api.is_signed_in().await {
                Ok(true) => break api,
                Ok(false) => {
                    last_error = Error::NotAuthenticated;
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
                Err(e) => {
                    last_error = e;
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
            },
            Err(e) => {
                last_error = e;
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
        }
    };

    if !api.has_cached_operations() {
        let _ = api.refresh_operations().await;
    }
    Ok(api)
}

#[cfg(all(test, unix))]
mod engine_tests {
    use super::*;
    use crate::browser_test::MockBrowser;
    use std::os::unix::fs::PermissionsExt;

    fn fixture(root: &Path, mock: &MockBrowser) -> HostLauncher {
        let script = root.join("engine");
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(&script, "#!/bin/sh\nexec sleep 60\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        HostLauncher::new()
            .data_dir(root)
            .engine(crate::BrowserEngine::Chromium)
            .host_path(script)
            .cdp_port(mock.endpoint.rsplit(':').next().unwrap().parse().unwrap())
    }

    #[tokio::test]
    async fn fallback_rejects_drm_incapable_engine_and_initializes_next() {
        let bad = MockBrowser::start_with_drm(false).await;
        let good = MockBrowser::start().await;
        let root = std::env::temp_dir().join(format!(
            "spotcore-fallback-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let builder = SpotCoreBuilder::new().data_dir(&root);
        let mut sdk = builder
            .launch_candidates(vec![
                fixture(&root.join("bad"), &bad),
                fixture(&root.join("good"), &good),
            ])
            .await
            .unwrap();
        assert!(
            bad.calls
                .lock()
                .unwrap()
                .iter()
                .any(|(method, _)| method == "player.drmSupport")
        );
        assert!(
            good.calls
                .lock()
                .unwrap()
                .iter()
                .any(|(method, _)| method == "player.drmSupport")
        );
        sdk.host.as_mut().unwrap().shutdown().await.unwrap();
        drop(sdk);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn preferred_single_process_retries_with_helpers_for_the_same_backend() {
        let mock = MockBrowser::start().await;
        let root =
            std::env::temp_dir().join(format!("spotcore-helper-retry-{}", std::process::id()));
        let candidate = fixture(&root, &mock);
        std::fs::write(root.join("engine"), "#!/bin/sh\nfor arg in \"$@\"; do [ \"$arg\" = --single-process ] && exit 1; done\nexec sleep 60\n").unwrap();
        let modes = candidate.candidates().unwrap();
        assert_eq!(modes.len(), 2);
        let mut sdk = SpotCoreBuilder::new()
            .data_dir(&root)
            .launch_candidates(modes)
            .await
            .unwrap();
        assert_eq!(
            sdk.host.as_ref().unwrap().engine(),
            crate::BrowserEngine::Chromium
        );
        sdk.host.as_mut().unwrap().shutdown().await.unwrap();
        drop(sdk);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn chromium_cookie_is_seeded_over_cdp_without_login() {
        let mock = MockBrowser::start().await;
        let root = std::env::temp_dir().join(format!("spotcore-cookie-{}", std::process::id()));
        let candidate = fixture(&root, &mock).session_cookie("private-cookie");
        let mut host = candidate.clone().spawn().await.unwrap();
        candidate.seed_session(&host).await.unwrap();
        let protocol = mock.protocol.lock().unwrap().clone();
        let cookie = &protocol
            .iter()
            .find(|(method, _)| method == "Network.setCookie")
            .unwrap()
            .1;
        assert_eq!(cookie["value"], "private-cookie");
        assert_eq!(cookie["url"], "https://open.spotify.com/");
        assert_eq!(cookie["httpOnly"], true);
        assert!(protocol.iter().any(|(method, _)| method == "Page.reload"));
        host.shutdown().await.unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn exhaustion_reports_typed_causes_and_requirements() {
        let mock = MockBrowser::start_with_drm(false).await;
        let root = std::env::temp_dir().join(format!(
            "spotcore-exhaustion-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let candidate = fixture(&root, &mock);
        let builder = SpotCoreBuilder::new().data_dir(&root);
        let error = builder
            .launch_candidates(vec![
                candidate.clone().host_path(root.join("missing")),
                candidate,
            ])
            .await
            .err()
            .unwrap();
        let Error::NoCompatibleEngine {
            attempts,
            requirements,
        } = error
        else {
            panic!("expected engine diagnostics")
        };
        assert_eq!(attempts.len(), 2);
        assert!(matches!(*attempts[0].cause, Error::Backend(_)));
        assert!(matches!(*attempts[1].cause, Error::DrmUnavailable(_)));
        assert!(requirements.iter().any(|item| item.contains("Qt6")));
        assert!(requirements.iter().any(|item| item.contains("Widevine")));
        std::fs::remove_dir_all(root).unwrap();
    }
}
