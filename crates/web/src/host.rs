//! Host process discovery, launcher, and RAII lifecycle management for SpotCore.

use spotcore_core::Error;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::{Child, Command};

/// Auto prefers the system-Qt host, then the OS adapter and installed Chromium browsers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BrowserEngine {
    #[default]
    Auto,
    Qt,
    System,
    Chromium,
}

fn find_system_host(root: &Path) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SPOTCORE_SYSTEM_ENGINE") {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }
    let name = format!("spotcore-system-engine{EXE_SUFFIX}");
    let mut candidates = vec![
        root.join(&name),
        PathBuf::from("target/debug").join(&name),
        PathBuf::from("target/release").join(&name),
    ];
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        candidates.insert(0, parent.join(&name));
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|dir| dir.join(&name)));
    }
    candidates.into_iter().find(|path| path.is_file())
}

/// Executable suffix for the target platform.
const EXE_SUFFIX: &str = if cfg!(windows) { ".exe" } else { "" };

/// Platform filename of the Widevine CDM.
const WIDEVINE_LIB: &str = if cfg!(windows) {
    "widevinecdm.dll"
} else if cfg!(target_os = "macos") {
    "libwidevinecdm.dylib"
} else {
    "libwidevinecdm.so"
};

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Browser profile roots that can hold a `WidevineCdm` installation.
fn browser_data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if cfg!(windows) {
        let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else {
            return dirs;
        };
        for sub in [
            r"Google\Chrome\User Data",
            r"Chromium\User Data",
            r"BraveSoftware\Brave-Browser\User Data",
            r"Microsoft\Edge\User Data",
        ] {
            dirs.push(local.join(sub));
        }
        return dirs;
    }
    let Some(home) = home_dir() else {
        return dirs;
    };
    if cfg!(target_os = "macos") {
        let root = home.join("Library/Application Support");
        for sub in [
            "Google/Chrome",
            "Chromium",
            "BraveSoftware/Brave-Browser",
            "Microsoft Edge",
        ] {
            dirs.push(root.join(sub));
        }
    } else {
        let root = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        for sub in [
            "google-chrome",
            "chromium",
            "BraveSoftware/Brave-Browser",
            "microsoft-edge",
            "opera",
            "vivaldi",
        ] {
            dirs.push(root.join(sub));
        }
        dirs.push(home.join(".var/app/com.google.Chrome/config/google-chrome"));
        dirs.push(home.join(".var/app/org.chromium.Chromium/config/chromium"));
    }
    dirs
}

/// Executable names to look for, including the platform suffix.
fn host_bin_names() -> [String; 2] {
    [
        format!("spotcore-qt-engine{EXE_SUFFIX}"),
        format!("spotcore{EXE_SUFFIX}"),
    ]
}

/// Discovers the Qt backend executable on the system.
///
/// Search order:
/// 1. `SPOTCORE_ENGINE` or `SPOTCORE_HOST` environment variable
/// 2. Relative to current executable (`./spotcore-qt-engine`, `../host/qt/build/spotcore-qt-engine`)
/// 3. Current working directory (`./host/qt/build/spotcore-qt-engine`, `./spotcore-qt-engine`)
/// 4. User data directory (`<data>/spotcore/spotcore-qt-engine`)
/// 5. System `$PATH`
pub fn find_host() -> Option<PathBuf> {
    find_host_in(spotcore_core::paths::Paths::discover().ok()?.root())
}

/// Same search as [`find_host`], but reads the data directory from `root`.
fn find_host_in(root: &Path) -> Option<PathBuf> {
    for env in ["SPOTCORE_ENGINE", "SPOTCORE_HOST"] {
        if let Ok(path) = std::env::var(env) {
            let p = PathBuf::from(path);
            if p.is_file() {
                return Some(p);
            }
        }
    }

    let bin_names = host_bin_names();

    if let Ok(mut exe) = std::env::current_exe() {
        exe.pop();
        for name in &bin_names {
            let candidates = [
                exe.join(name),
                exe.join("host/qt/build").join(name),
                exe.join("../host/qt/build").join(name),
                exe.join("../../host/qt/build").join(name),
            ];
            for c in candidates {
                if c.is_file() {
                    return Some(c);
                }
            }
        }
    }

    for name in &bin_names {
        let cwd_candidates = [
            PathBuf::from("host/qt/build").join(name),
            PathBuf::from(".").join(name),
        ];
        for c in cwd_candidates {
            if c.is_file() {
                return Some(c.canonicalize().unwrap_or(c));
            }
        }
    }

    for name in &bin_names {
        let candidate = root.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for name in &bin_names {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }

    let legacy = PathBuf::from("host/qt/build").join(format!("spotcore-engine{EXE_SUFFIX}"));
    legacy
        .is_file()
        .then(|| legacy.canonicalize().unwrap_or(legacy))
}

fn find_launcher_in(root: &Path) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SPOTCORE_LAUNCHER") {
        return Some(PathBuf::from(path));
    }
    let name = format!("spotcore-engine{EXE_SUFFIX}");
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        candidates.push(parent.join(&name));
        candidates.push(parent.join("..").join(&name));
    }
    candidates.extend([
        root.join(&name),
        PathBuf::from("target/debug").join(&name),
        PathBuf::from("target/release").join(&name),
    ]);
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|dir| dir.join(&name)));
    }
    candidates.into_iter().find(|path| path.is_file())
}

fn engine_command(bin_path: &Path, root: &Path) -> Result<Command, Error> {
    let launcher = find_launcher_in(root).ok_or_else(|| Error::Backend(
            "spotcore-engine launcher was not found; install it beside the application or set SPOTCORE_LAUNCHER".into()))?;
    if std::fs::canonicalize(&launcher)
        .ok()
        .zip(std::fs::canonicalize(bin_path).ok())
        .is_some_and(|(launcher, backend)| launcher == backend)
    {
        return Err(Error::InvalidInput(
            "launcher and backend paths must be different",
        ));
    }
    let mut cmd = Command::new(launcher);
    cmd.arg("--parent-pid")
        .arg(std::process::id().to_string())
        .arg("--backend")
        .arg(bin_path)
        .arg("--");
    Ok(cmd)
}

/// Discovers an installed Chromium-based browser (Chrome, Brave, Edge, Chromium)
/// for zero-dependency universal fallback execution.
pub fn find_system_browser() -> Option<PathBuf> {
    find_system_browsers().into_iter().next()
}

fn find_system_browsers() -> Vec<PathBuf> {
    let mut found = Vec::new();
    if let Ok(path) = std::env::var("SPOTCORE_BROWSER") {
        let p = PathBuf::from(path);
        if p.is_file() {
            found.push(p);
        }
    }

    let names = [
        "google-chrome",
        "google-chrome-stable",
        "brave-browser",
        "brave",
        "microsoft-edge",
        "microsoft-edge-stable",
        "chromium",
        "chromium-browser",
        "chrome",
        "msedge",
    ];

    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for name in names {
                let candidate = dir.join(format!("{name}{EXE_SUFFIX}"));
                if candidate.is_file() {
                    found.push(candidate);
                }
            }
        }
    }

    let linux_paths = [
        "/opt/google/chrome/google-chrome",
        "/opt/brave.com/brave/brave-browser",
        "/opt/microsoft/msedge/msedge",
        "/usr/bin/google-chrome",
        "/usr/bin/brave-browser",
        "/usr/bin/microsoft-edge",
        "/usr/bin/chromium",
    ];
    for p in linux_paths {
        let path = PathBuf::from(p);
        if path.is_file() {
            found.push(path);
        }
    }

    let mac_paths = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
    ];
    for p in mac_paths {
        let path = PathBuf::from(p);
        if path.is_file() {
            found.push(path);
        }
    }

    let win_fixed = [
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    ];
    for p in win_fixed {
        let path = PathBuf::from(p);
        if path.is_file() {
            found.push(path);
        }
    }
    let win_rel = [
        r"Google\Chrome\Application\chrome.exe",
        r"BraveSoftware\Brave-Browser\Application\brave.exe",
        r"Microsoft\Edge\Application\msedge.exe",
    ];
    for base in ["LOCALAPPDATA", "PROGRAMFILES", "PROGRAMFILES(X86)"] {
        let Some(root) = std::env::var_os(base) else {
            continue;
        };
        for rel in win_rel {
            let path = PathBuf::from(&root).join(rel);
            if path.is_file() {
                found.push(path);
            }
        }
    }

    let mut seen = std::collections::HashSet::new();
    found.retain(|path| seen.insert(std::fs::canonicalize(path).unwrap_or_else(|_| path.clone())));
    found
}

/// Scans a directory tree for `WidevineCdm/*/_platform_specific/<arch>/<cdm>`.
fn widevine_platform() -> String {
    let os = match std::env::consts::OS {
        "windows" => "win",
        "macos" => "mac",
        _ => "linux",
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "x86" => "x86",
        "aarch64" => "arm64",
        value => value,
    };
    format!("{os}_{arch}")
}

fn find_widevine_in_profile(profile_dir: &Path) -> Option<PathBuf> {
    let mut versions: Vec<_> = std::fs::read_dir(profile_dir.join("WidevineCdm"))
        .ok()?
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let version = entry
                .file_name()
                .to_str()?
                .split('.')
                .map(str::parse::<u64>)
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            Some((version, entry.path()))
        })
        .collect();
    versions.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, dir) in versions {
        for candidate in [
            dir.join("_platform_specific")
                .join(widevine_platform())
                .join(WIDEVINE_LIB),
            dir.join(WIDEVINE_LIB),
        ] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    [
        profile_dir
            .join("WidevineCdm/_platform_specific")
            .join(widevine_platform())
            .join(WIDEVINE_LIB),
        profile_dir.join("WidevineCdm").join(WIDEVINE_LIB),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file())
}

/// Candidate `WidevineCdm` roots shipped beside a Chrome/Chromium installation.
fn widevine_app_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if cfg!(windows) {
        for base in ["LOCALAPPDATA", "PROGRAMFILES", "PROGRAMFILES(X86)"] {
            let Some(base) = std::env::var_os(base).map(PathBuf::from) else {
                continue;
            };
            for relative in [
                r"Google\Chrome\Application",
                r"Microsoft\Edge\Application",
                r"BraveSoftware\Brave-Browser\Application",
            ] {
                let application = base.join(relative);
                roots.push(application.clone());
                if let Ok(versions) = std::fs::read_dir(application) {
                    roots.extend(
                        versions
                            .flatten()
                            .filter(|entry| entry.path().is_dir())
                            .map(|entry| entry.path()),
                    );
                }
            }
        }
    } else if cfg!(target_os = "macos") {
        for app in [
            "/Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Versions/Current/Libraries",
            "/Applications/Brave Browser.app/Contents/Frameworks/Brave Browser Framework.framework/Versions/Current/Libraries",
            "/Applications/Microsoft Edge.app/Contents/Frameworks/Microsoft Edge Framework.framework/Versions/Current/Libraries",
        ] {
            roots.push(PathBuf::from(app));
        }
    } else if !cfg!(windows) {
        for dir in [
            "/opt/google/chrome",
            "/opt/brave.com/brave",
            "/opt/microsoft/msedge",
        ] {
            roots.push(PathBuf::from(dir));
        }
    }
    roots
}

/// Discovers the Widevine Content Decryption Module library on the system.
pub fn find_widevine() -> Option<PathBuf> {
    find_widevine_in(spotcore_core::paths::Paths::discover().ok()?.root())
}

/// Same search as [`find_widevine`], but reads the standalone copy from `root`.
fn find_widevine_in(root: &Path) -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SPOTCORE_WIDEVINE") {
        let p = PathBuf::from(path);
        if p.is_file() {
            return Some(p);
        }
    }

    // A copy the user placed beside the SDK data.
    let standalone = root.join(WIDEVINE_LIB);
    if standalone.is_file() {
        return Some(standalone);
    }

    let mut candidates: Vec<_> = browser_data_dirs()
        .into_iter()
        .chain(widevine_app_roots())
        .filter_map(|root| find_widevine_in_profile(&root))
        .collect();
    candidates.sort_by_key(|path| {
        path.ancestors()
            .find(|part| {
                part.parent()
                    .and_then(Path::file_name)
                    .is_some_and(|name| name == "WidevineCdm")
            })
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .and_then(|name| {
                name.split('.')
                    .map(str::parse::<u64>)
                    .collect::<Result<Vec<_>, _>>()
                    .ok()
            })
            .unwrap_or_default()
    });
    if let Some(candidate) = candidates.pop() {
        return Some(candidate);
    }

    // Flat libraries some distributions ship.
    if !cfg!(windows) {
        let system_candidates = [
            "/usr/lib/chromium/libwidevinecdm.so",
            "/usr/lib64/chromium/libwidevinecdm.so",
            "/usr/lib/chromium-browser/libwidevinecdm.so",
            "/usr/lib64/chromium-browser/libwidevinecdm.so",
        ];
        for c in system_candidates {
            let p = PathBuf::from(c);
            if p.is_file() {
                return Some(p);
            }
        }
    }

    None
}

/// Finds an ephemeral available TCP port on localhost.
pub fn find_free_port() -> Result<u16, Error> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| Error::Backend(format!("failed to bind ephemeral port: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| Error::Backend(format!("failed to get local addr: {e}")))?
        .port();
    drop(listener);
    Ok(port)
}

fn write_profile_json(path: &Path, value: &serde_json::Value) -> Result<(), Error> {
    use std::io::Write;
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(&serde_json::to_vec(value)?)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result.map_err(|_| Error::Backend("could not save Chromium DRM configuration".into()))
}

fn prepare_chromium_drm(
    profile: &Path,
    browser: &Path,
    library: Option<&Path>,
) -> Result<(), Error> {
    let Some(library) = library else {
        return Ok(());
    };
    std::fs::create_dir_all(profile)
        .map_err(|_| Error::Backend("could not prepare Chromium profile".into()))?;
    #[cfg(target_os = "linux")]
    if let Some(component) = library
        .parent()
        .into_iter()
        .flat_map(|p| p.ancestors())
        .take(4)
        .find(|p| p.join("manifest.json").is_file())
    {
        let component = std::fs::canonicalize(component)
            .map_err(|_| Error::Backend("could not resolve Widevine component".into()))?;
        let directory = profile.join("WidevineCdm");
        std::fs::create_dir_all(&directory)
            .map_err(|_| Error::Backend("could not prepare Widevine registration".into()))?;
        // Chromium reads this component-root hint before starting the sandboxed CDM process.
        write_profile_json(
            &directory.join("latest-component-updated-widevine-cdm"),
            &serde_json::json!({"Path": component}),
        )?;
    }
    if browser.file_name().is_some_and(|name| {
        name.to_string_lossy()
            .to_ascii_lowercase()
            .contains("brave")
    }) {
        let state = profile.join("Local State");
        let mut value = match std::fs::metadata(&state) {
            Ok(metadata) if metadata.len() <= 8 * 1024 * 1024 => {
                serde_json::from_slice::<serde_json::Value>(
                    &std::fs::read(&state)
                        .map_err(|_| Error::Backend("could not read Brave Local State".into()))?,
                )
                .map_err(|_| Error::Backend("invalid Brave Local State".into()))?
            }
            Ok(_) => return Err(Error::Backend("Brave Local State is too large".into())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
            Err(_) => return Err(Error::Backend("could not inspect Brave Local State".into())),
        };
        if !value.is_object() || value.get("brave").is_some_and(|v| !v.is_object()) {
            return Err(Error::Backend("invalid Brave Local State structure".into()));
        }
        value["brave"]["widevine_opted_in"] = true.into();
        write_profile_json(&state, &value)?;
    }
    Ok(())
}

/// Must match the standalone Qt host's platform default.
pub fn default_profile_dir() -> PathBuf {
    spotcore_core::paths::Paths::default().profile()
}

/// An actively running SpotCore host or browser process.
/// Automatically terminates the child process when dropped.
pub struct ManagedHost {
    child: Option<Child>,
    process_group: Option<u32>,
    port: u16,
    cdp_url: String,
    is_custom_engine: bool,
    engine: BrowserEngine,
}

impl ManagedHost {
    /// The CDP URL assigned to this host instance (e.g. `http://127.0.0.1:41235`).
    pub fn cdp_url(&self) -> &str {
        &self.cdp_url
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn is_custom_engine(&self) -> bool {
        self.is_custom_engine
    }

    pub fn engine(&self) -> BrowserEngine {
        self.engine
    }

    /// Manually terminates the host child process.
    pub async fn shutdown(&mut self) -> Result<(), Error> {
        if self.engine != BrowserEngine::System {
            kill_process_group(self.process_group.take());
        }
        if let Some(child) = &mut self.child
            && child
                .try_wait()
                .map_err(|e| Error::Backend(e.to_string()))?
                .is_none()
        {
            if self.engine == BrowserEngine::System {
                match tokio::time::timeout(Duration::from_secs(15), child.wait()).await {
                    Ok(Ok(status)) if status.success() => {}
                    Ok(Ok(_)) => {
                        return Err(Error::Backend(
                            "system engine failed during shutdown".into(),
                        ));
                    }
                    _ => {
                        child
                            .kill()
                            .await
                            .map_err(|e| Error::Backend(e.to_string()))?;
                        return Err(Error::Backend(
                            "system engine did not close gracefully".into(),
                        ));
                    }
                }
            } else {
                child
                    .kill()
                    .await
                    .map_err(|e| Error::Backend(e.to_string()))?;
            }
        }
        kill_process_group(self.process_group.take());
        self.child = None;
        Ok(())
    }
}

impl Drop for ManagedHost {
    fn drop(&mut self) {
        kill_process_group(self.process_group.take());
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
    }
}

fn kill_process_group(group: Option<u32>) {
    #[cfg(unix)]
    if let Some(group) = group {
        // The launcher creates a private group, including children of shell wrappers.
        unsafe {
            libc::kill(-(group as i32), libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    let _ = group;
}

/// Builder and launcher for the SpotCore host process.
#[derive(Clone)]
pub struct HostLauncher {
    host_path: Option<PathBuf>,
    data_root: Option<PathBuf>,
    profile_dir: Option<PathBuf>,
    widevine_path: Option<PathBuf>,
    cdp_port: Option<u16>,
    no_cuts: bool,
    helpers: bool,
    defer_navigation: bool,
    engine: BrowserEngine,
    role: crate::BrowserRole,
    headless: bool,
    session_cookie: Option<String>,
}

impl std::fmt::Debug for HostLauncher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostLauncher")
            .field("host_path", &self.host_path)
            .field("data_root", &self.data_root)
            .field("profile_dir", &self.profile_dir)
            .field("widevine_path", &self.widevine_path)
            .field("cdp_port", &self.cdp_port)
            .field("headless", &self.headless)
            .field(
                "session_cookie",
                &self.session_cookie.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

impl Default for HostLauncher {
    fn default() -> Self {
        Self {
            host_path: None,
            data_root: None,
            profile_dir: None,
            widevine_path: None,
            cdp_port: None,
            no_cuts: false,
            helpers: false,
            defer_navigation: false,
            engine: BrowserEngine::Auto,
            role: crate::BrowserRole::Playback,
            headless: true,
            session_cookie: None,
        }
    }
}

impl HostLauncher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn engine(mut self, engine: BrowserEngine) -> Self {
        self.engine = engine;
        self
    }

    pub fn browser_role(mut self, role: crate::BrowserRole) -> Self {
        self.role = role;
        self
    }

    pub fn host_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.host_path = Some(path.into());
        self
    }

    pub fn profile_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.profile_dir = Some(dir.into());
        self
    }

    /// Points the profile and the engine search at a custom data root.
    pub fn data_dir(mut self, root: impl Into<PathBuf>) -> Self {
        let paths = spotcore_core::paths::Paths::new(root);
        self.data_root = Some(paths.root().to_path_buf());
        self
    }

    fn root(&self) -> Result<PathBuf, Error> {
        let paths = match &self.data_root {
            Some(root) => spotcore_core::Paths::new(root),
            None => spotcore_core::Paths::discover()?,
        };
        paths.validate()?;
        Ok(paths.resolve()?.root().to_path_buf())
    }

    pub fn widevine_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.widevine_path = Some(path.into());
        self
    }

    pub fn cdp_port(mut self, port: u16) -> Self {
        self.cdp_port = Some(port);
        self
    }

    /// Waits on a blank page until the SDK has installed its bootstrap interceptor.
    pub fn defer_navigation(mut self, enabled: bool) -> Self {
        self.defer_navigation = enabled;
        self
    }

    pub fn no_cuts(mut self, no_cuts: bool) -> Self {
        self.no_cuts = no_cuts;
        self
    }

    /// Seeds the Qt profile before navigation. System-browser fallback requires its own signed-in profile.
    pub fn session_cookie(mut self, cookie: impl Into<String>) -> Self {
        self.session_cookie = Some(cookie.into());
        self
    }

    pub fn headless(mut self, headless: bool) -> Self {
        self.headless = headless;
        self
    }

    pub async fn login(self) -> Result<(), Error> {
        let (host, _) = self.resolve_host()?;
        let profile = self
            .profile_dir
            .clone()
            .unwrap_or(self.root()?.join("profile"));
        login_interactive(Some(&host), Some(&profile)).await
    }

    /// WKWebView stores account data outside the profile directory; remove its named store first.
    pub async fn forget_system_profile(root: &Path, profile: &Path) -> Result<(), Error> {
        #[cfg(target_os = "macos")]
        if profile.join("wk-store-id").is_file() {
            let host = find_system_host(root).ok_or_else(|| {
                Error::Backend(
                    "system engine is required to remove its WKWebView data store".into(),
                )
            })?;
            let mut process = engine_command(&host, root)?
                .arg("--forget")
                .arg("--profile-dir")
                .arg(profile)
                .kill_on_drop(true)
                .spawn()
                .map_err(|_| Error::Backend("cannot start system-profile cleanup".into()))?;
            let status = tokio::time::timeout(Duration::from_secs(35), process.wait())
                .await
                .map_err(|_| Error::Timeout)?
                .map_err(|_| Error::Backend("system-profile cleanup failed".into()))?;
            if !status.success() {
                return Err(Error::Backend(
                    "system data store is still in use or cleanup failed".into(),
                ));
            }
        }
        let _ = (root, profile);
        Ok(())
    }

    /// Spawns the host or fallback browser process, waits for CDP readiness, and returns a `ManagedHost`.
    pub async fn spawn(self) -> Result<ManagedHost, Error> {
        let mut failures = Vec::new();
        for candidate in self.candidates()? {
            match candidate.clone().spawn_configured().await {
                Ok(host) => return Ok(host),
                Err(error) => failures.push(candidate.failure(error)),
            }
        }
        Err(Self::unavailable(failures, self.role))
    }

    /// Seeds an SDK-owned Chromium profile over loopback CDP, never through argv.
    pub async fn seed_session(&self, host: &ManagedHost) -> Result<(), Error> {
        if host.engine() != BrowserEngine::Chromium {
            return Ok(());
        }
        let Some(cookie) = self.session_cookie.as_deref().filter(|c| !c.is_empty()) else {
            return Ok(());
        };
        if cookie.len() > 8192
            || !cookie
                .bytes()
                .all(|b| b.is_ascii_graphic() && b != b';' && b != b',')
        {
            return Err(Error::InvalidInput("invalid sp_dc cookie"));
        }
        let cdp = crate::cdp::Cdp::connect(host.cdp_url(), None, false).await?;
        let value = cdp
            .request(
                "Network.setCookie",
                serde_json::json!({
                    "name": "sp_dc", "value": cookie, "url": "https://open.spotify.com/",
                    "domain": ".spotify.com", "path": "/", "secure": true, "httpOnly": true
                }),
            )
            .await
            .map_err(|_| Error::Backend("could not seed Chromium session cookie".into()))?;
        if value["success"] != true {
            return Err(Error::Backend("Chromium rejected session cookie".into()));
        }
        cdp.request("Page.reload", serde_json::json!({})).await?;
        Ok(())
    }

    pub fn automatic(&self) -> bool {
        self.engine == BrowserEngine::Auto
            && self.host_path.is_none()
            && std::env::var_os("SPOTCORE_ENGINE").is_none()
            && std::env::var_os("SPOTCORE_HOST").is_none()
    }

    /// Explicit configuration stays pinned; Auto enumerates all known candidates.
    pub fn candidates(&self) -> Result<Vec<Self>, Error> {
        if !self.automatic() {
            return Ok(self.process_modes());
        }
        let root = self.root()?;
        let mut system = self.clone().engine(BrowserEngine::System);
        if system.profile_dir.is_none() {
            system.profile_dir = Some(root.join("profiles/system"));
        }
        let mut qt = self.clone().engine(BrowserEngine::Qt);
        qt.host_path = find_host_in(&root);
        let mut candidates = vec![qt, system];
        let browsers = find_system_browsers();
        if browsers.is_empty() {
            candidates.push(self.clone().engine(BrowserEngine::Chromium));
        } else {
            candidates.extend(browsers.into_iter().map(|path| {
                let mut candidate = self.clone().engine(BrowserEngine::Chromium);
                if candidate.profile_dir.is_none() {
                    let name = path.file_name().unwrap_or_default();
                    candidate.profile_dir = Some(root.join("profiles/chromium").join(name));
                }
                candidate.host_path(path)
            }));
        }
        Ok(candidates
            .into_iter()
            .flat_map(|candidate| candidate.process_modes())
            .collect())
    }

    fn process_modes(&self) -> Vec<Self> {
        if cfg!(target_os = "linux")
            && self.role == crate::BrowserRole::Playback
            && self.engine != BrowserEngine::System
            && !self.helpers
        {
            let mut fallback = self.clone();
            fallback.helpers = true;
            vec![self.clone(), fallback]
        } else {
            vec![self.clone()]
        }
    }

    pub fn failure(&self, cause: Error) -> spotcore_core::EngineFailure {
        spotcore_core::EngineFailure {
            engine: format!(
                "{:?} ({})",
                self.engine,
                if cfg!(target_os = "linux")
                    && self.engine != BrowserEngine::System
                    && self.role == crate::BrowserRole::Playback
                    && !self.helpers
                {
                    "prefer single process"
                } else {
                    "helpers allowed"
                }
            ),
            path: self.resolve_host().ok().map(|(path, _)| path),
            cause: Box::new(cause),
        }
    }

    pub fn unavailable(
        attempts: Vec<spotcore_core::EngineFailure>,
        role: crate::BrowserRole,
    ) -> Error {
        let mut requirements = vec![
            "Install a supported Chromium browser, or spotcore-engine plus the system Qt6 WebEngine runtime and resources".into(),
            "Install the spotcore-engine launcher; SPOTCORE_LAUNCHER can specify its path".into(),
            "Explicit backend and runtime paths can be supplied by the application".into(),
        ];
        if role == crate::BrowserRole::Playback {
            requirements.push("Playback requires working Widevine EME and AAC decoding; OS adapters currently support services only".into());
        }
        Error::NoCompatibleEngine {
            attempts,
            requirements,
        }
    }

    /// Starts exactly this candidate; the unified builder coordinates DRM-aware retries.
    pub async fn spawn_configured(self) -> Result<ManagedHost, Error> {
        if self.engine == BrowserEngine::System && self.role == crate::BrowserRole::Playback {
            return Err(Error::Unsupported);
        }
        let paths = spotcore_core::Paths::new(self.root()?);
        paths
            .prepare()
            .map_err(|e| Error::Backend(format!("cannot prepare data directory: {e}")))?;
        let (bin_path, is_custom_engine) = self.resolve_host()?;
        if !bin_path.is_file() {
            return Err(Error::Backend(
                "selected backend executable does not exist".into(),
            ));
        }

        // Retry ephemeral-port races; never substitute an explicitly requested port.
        let attempts = if self.cdp_port.is_some() { 1 } else { 3 };
        let mut last_error = Error::Timeout;

        for _ in 0..attempts {
            let port = match self.cdp_port {
                Some(port) => port,
                None => find_free_port()?,
            };

            let mut cmd = self.command(&bin_path, is_custom_engine, port)?;
            cmd.stdout(Stdio::null());
            cmd.stderr(Stdio::inherit());

            let mut child = cmd.spawn().map_err(|e| {
                Error::Backend(format!(
                    "failed to spawn playback engine ({}): {e}",
                    bin_path.display()
                ))
            })?;

            let process_group = if cfg!(unix) { child.id() } else { None };
            let cdp_url = format!("http://127.0.0.1:{port}");
            match wait_for_cdp_ready(
                &cdp_url,
                &mut child,
                is_custom_engine,
                is_custom_engine && self.defer_navigation,
                Duration::from_secs(14),
            )
            .await
            {
                Ok(()) => {
                    return Ok(ManagedHost {
                        child: Some(child),
                        process_group,
                        port,
                        cdp_url,
                        is_custom_engine,
                        engine: if self.engine == BrowserEngine::System {
                            BrowserEngine::System
                        } else if is_custom_engine {
                            BrowserEngine::Qt
                        } else {
                            BrowserEngine::Chromium
                        },
                    });
                }
                Err(e) => {
                    kill_process_group(process_group);
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    last_error = e;
                }
            }
        }

        Err(last_error)
    }

    fn resolve_host(&self) -> Result<(PathBuf, bool), Error> {
        if self.engine == BrowserEngine::System
            && !cfg!(any(windows, target_os = "macos", target_os = "linux"))
        {
            return Err(Error::Unsupported);
        }
        if let Some(path) = &self.host_path {
            return Ok((path.clone(), self.engine != BrowserEngine::Chromium));
        }
        let found = match self.engine {
            BrowserEngine::Auto => find_host_in(&self.root()?)
                .map(|path| (path, true))
                .or_else(|| find_system_browser().map(|path| (path, false))),
            BrowserEngine::Qt => find_host_in(&self.root()?).map(|path| (path, true)),
            BrowserEngine::Chromium => find_system_browser().map(|path| (path, false)),
            BrowserEngine::System => find_system_host(&self.root()?).map(|path| (path, true)),
        };
        found.ok_or_else(|| Error::Backend("selected browser engine was not found; configure its host path or install its runtime".into()))
    }

    /// Builds the child command for one launch attempt.
    fn command(
        &self,
        bin_path: &Path,
        is_custom_engine: bool,
        port: u16,
    ) -> Result<Command, Error> {
        let root = self.root()?;
        if self.role == crate::BrowserRole::Playback
            && self.widevine_path.is_none()
            && let Some(path) = std::env::var_os("SPOTCORE_WIDEVINE")
            && !Path::new(&path).is_file()
        {
            return Err(Error::InvalidInput(
                "SPOTCORE_WIDEVINE does not name a library file",
            ));
        }
        let profile = self
            .profile_dir
            .clone()
            .unwrap_or_else(|| root.join("profile"));
        if profile.as_os_str().is_empty() {
            return Err(Error::InvalidInput("profile directory is empty"));
        }
        let profile = std::path::absolute(profile)
            .map_err(|e| Error::Backend(format!("cannot resolve profile directory: {e}")))?;
        if self.role == crate::BrowserRole::Playback
            && let Some(path) = self.widevine_path.as_ref()
            && !path.is_file()
        {
            return Err(Error::InvalidInput(
                "configured Widevine library does not exist",
            ));
        }
        let mut cmd = engine_command(bin_path, &root)?;

        if is_custom_engine {
            if self.helpers && self.engine != BrowserEngine::System {
                cmd.arg("--multi-process");
            }
            if self.defer_navigation && self.engine != BrowserEngine::System {
                cmd.env("SPOTCORE_DEFER_PAGE", "1");
            }
            cmd.arg("--profile-dir").arg(&profile);
            cmd.arg("--cdp-port").arg(port.to_string());
            // The engine exits with its owning process.
            cmd.arg("--parent-pid").arg(std::process::id().to_string());

            if self.role == crate::BrowserRole::Playback
                && let Some(widevine) = self
                    .widevine_path
                    .clone()
                    .or_else(|| find_widevine_in(&root))
            {
                cmd.arg("--widevine").arg(widevine);
            }

            if self.no_cuts {
                cmd.arg("--no-cuts");
            }
            if self.engine == BrowserEngine::System && !self.headless {
                cmd.arg("--visible");
            }

            if self.role == crate::BrowserRole::Services && self.engine != BrowserEngine::System {
                cmd.arg("--services-only");
            }

            // Keep the session cookie out of argv.
            if let Some(cookie) = self.session_cookie.as_deref().filter(|c| !c.is_empty()) {
                cmd.env("SPOTCORE_SP_DC", cookie);
            }

            if self.headless && self.engine != BrowserEngine::System {
                cmd.env("QT_QPA_PLATFORM", "offscreen");
                cmd.env("QT_OPENGL", "software");
                cmd.env("QT_QPA_OFFSCREEN_NO_GLX", "1");
            }
        } else {
            if self.role == crate::BrowserRole::Playback {
                let library = self
                    .widevine_path
                    .clone()
                    .or_else(|| find_widevine_in(&root));
                prepare_chromium_drm(&profile, bin_path, library.as_deref())?;
            }
            if self.headless {
                cmd.arg("--headless=new");
            }
            cmd.arg(format!("--remote-debugging-port={port}"));
            cmd.arg(format!("--user-data-dir={}", profile.display()));
            if cfg!(target_os = "linux")
                && self.role == crate::BrowserRole::Playback
                && !self.helpers
            {
                cmd.arg("--single-process");
                cmd.arg("--no-sandbox");
            }
            cmd.arg("--disable-gpu");
            cmd.arg("--enable-low-end-device-mode");
            cmd.arg("--disable-extensions");
            cmd.arg("--disable-sync");
            cmd.arg("--disable-background-networking");
            cmd.arg("--disable-default-apps");
            cmd.arg("--no-default-browser-check");
            cmd.arg("--no-pings");
            cmd.arg("https://open.spotify.com");
        }

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.as_std_mut().process_group(0);
        }

        // On Linux, bind child lifetime to parent using PR_SET_PDEATHSIG
        #[cfg(target_os = "linux")]
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }

        Ok(cmd)
    }
}

// Verify process lifetime and engine identity; an open port alone proves neither.
async fn wait_for_cdp_ready(
    endpoint: &str,
    child: &mut Child,
    expect_engine: bool,
    allow_blank: bool,
    timeout: Duration,
) -> Result<(), Error> {
    let base = endpoint.trim_end_matches('/');
    let list_url = format!("{base}/json/list");
    let version_url = format!("{base}/json/version");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .map_err(|e| Error::Backend(format!("failed to build HTTP client: {e}")))?;

    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| Error::Backend(format!("failed to poll playback engine: {e}")))?
        {
            return Err(Error::Backend(format!(
                "playback engine exited before CDP was ready: {status}"
            )));
        }

        if let Ok(res) = client.get(&list_url).send().await
            && res.status().is_success()
            && let Ok(targets) = res.json::<Vec<serde_json::Value>>().await
        {
            let has_spotify = targets.iter().any(|t| {
                t["type"] == "page"
                    && t["url"].as_str().is_some_and(|u| {
                        crate::cdp::is_spotify_page(u) || (allow_blank && u == "about:blank")
                    })
            });
            if has_spotify {
                if expect_engine {
                    verify_engine_identity(&client, &version_url, endpoint).await?;
                }
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    Err(Error::Timeout)
}

/// Refuses a CDP endpoint that is not the `spotcore-engine` this process spawned.
async fn verify_engine_identity(
    client: &reqwest::Client,
    version_url: &str,
    endpoint: &str,
) -> Result<(), Error> {
    let browser = match client.get(version_url).send().await {
        Ok(res) if res.status().is_success() => res
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|value| value["Browser"].as_str().map(str::to_owned)),
        _ => None,
    }
    .unwrap_or_default();

    if browser.starts_with("spotcore-engine") {
        Ok(())
    } else {
        Err(Error::Backend(format!(
            "CDP endpoint {endpoint} answered as {browser:?}, not the spotcore-engine this process spawned"
        )))
    }
}

/// Spawns an interactive login window using the engine binary and awaits completion.
pub async fn login_interactive(
    host_path: Option<&Path>,
    profile_dir: Option<&Path>,
) -> Result<(), Error> {
    let host_bin = host_path
        .map(PathBuf::from)
        .or_else(find_host)
        .or_else(find_system_browser)
        .ok_or_else(|| {
            Error::Backend("no spotcore-engine or Chromium browser found for login.".into())
        })?;

    let profile = profile_dir
        .map(PathBuf::from)
        .unwrap_or_else(default_profile_dir);

    let is_engine = host_bin
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.contains("spotcore"));

    let mut cmd = engine_command(&host_bin, spotcore_core::Paths::discover()?.root())?;
    if is_engine {
        cmd.arg("--login");
        cmd.arg("--profile-dir").arg(&profile);
    } else {
        cmd.arg(format!("--user-data-dir={}", profile.display()));
        cmd.arg("https://accounts.spotify.com/login");
    }

    cmd.kill_on_drop(true);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());

    let mut child = cmd.spawn().map_err(|e| {
        Error::Backend(format!(
            "failed to spawn login window ({}): {e}",
            host_bin.display()
        ))
    })?;

    let status = child
        .wait()
        .await
        .map_err(|e| Error::Backend(format!("login process error: {e}")))?;

    if status.success() {
        Ok(())
    } else {
        Err(Error::Backend(format!(
            "login process exited with status: {:?}",
            status.code()
        )))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn services_mode_does_not_require_or_forward_widevine() {
        let root = scratch("service-role");
        let launcher = HostLauncher::new()
            .data_dir(&root)
            .widevine_path(root.join("missing-cdm"));
        assert!(
            launcher
                .command(Path::new("spotcore-engine"), true, 1234)
                .is_err()
        );
        let command = launcher
            .browser_role(crate::BrowserRole::Services)
            .command(Path::new("spotcore-engine"), true, 1234)
            .unwrap();
        let arguments: Vec<_> = command.as_std().get_args().collect();
        assert!(arguments.contains(&std::ffi::OsStr::new("--services-only")));
        assert!(!arguments.contains(&std::ffi::OsStr::new("--widevine")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn chromium_drm_preparation_preserves_preferences_and_registers_component() {
        let root = scratch("chromium-drm");
        let component = root.join("component");
        let binary = component
            .join("_platform_specific")
            .join(widevine_platform())
            .join(WIDEVINE_LIB);
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, b"fixture").unwrap();
        std::fs::write(
            component.join("manifest.json"),
            br#"{"name":"WidevineCdm"}"#,
        )
        .unwrap();
        let profile = root.join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::write(
            profile.join("Local State"),
            br#"{"keep":7,"brave":{"other":true,"widevine_opted_in":false}}"#,
        )
        .unwrap();
        prepare_chromium_drm(&profile, Path::new("brave-browser"), Some(&binary)).unwrap();
        let state: serde_json::Value =
            serde_json::from_slice(&std::fs::read(profile.join("Local State")).unwrap()).unwrap();
        assert_eq!(state["keep"], 7);
        assert_eq!(state["brave"]["other"], true);
        assert_eq!(state["brave"]["widevine_opted_in"], true);
        #[cfg(target_os = "linux")]
        {
            let hint: serde_json::Value = serde_json::from_slice(
                &std::fs::read(profile.join("WidevineCdm/latest-component-updated-widevine-cdm"))
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                hint["Path"],
                std::fs::canonicalize(component).unwrap().to_str().unwrap()
            );
        }
        std::fs::write(profile.join("Local State"), b"malformed").unwrap();
        assert!(prepare_chromium_drm(&profile, Path::new("brave-browser"), Some(&binary)).is_err());
        assert_eq!(
            std::fs::read(profile.join("Local State")).unwrap(),
            b"malformed"
        );
        let untouched = root.join("no-cdm");
        prepare_chromium_drm(&untouched, Path::new("brave-browser"), None).unwrap();
        assert!(!untouched.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn failed_wrapper_startup_kills_its_remaining_children() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch("wrapper-child");
        let script = root.join("wrapper");
        let pidfile = root.join("pid");
        std::fs::write(
            &script,
            format!("#!/bin/sh\nsleep 60 &\necho $! > '{}'\n", pidfile.display()),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let result = HostLauncher::new()
            .data_dir(&root)
            .engine(BrowserEngine::Chromium)
            .host_path(script)
            .cdp_port(find_free_port().unwrap())
            .spawn()
            .await;
        assert!(result.is_err());
        let pid = std::fs::read_to_string(pidfile).unwrap();
        for _ in 0..20 {
            let state = std::fs::read_to_string(format!("/proc/{}/status", pid.trim()));
            if state.as_ref().map_or(true, |value| {
                value.lines().any(|line| {
                    line.starts_with("State:")
                        && (line.contains("Z (zombie)") || line.contains("X (dead)"))
                })
            }) {
                std::fs::remove_dir_all(root).unwrap();
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("wrapper child survived failed startup");
    }

    #[test]
    fn automatic_order_keeps_engine_options_and_explicit_selection() {
        let root = scratch("candidates");
        let launcher = HostLauncher::new()
            .data_dir(&root)
            .session_cookie("private-cookie")
            .headless(false)
            .no_cuts(true);
        let candidates = launcher.candidates().unwrap();
        assert_eq!(candidates.first().unwrap().engine, BrowserEngine::Qt);
        let system_index = if cfg!(target_os = "linux") { 2 } else { 1 };
        if cfg!(target_os = "linux") {
            assert_eq!(candidates[1].engine, BrowserEngine::Qt);
            assert!(!candidates[0].helpers && candidates[1].helpers);
        }
        assert_eq!(candidates[system_index].engine, BrowserEngine::System);
        assert!(
            candidates[system_index + 1..]
                .iter()
                .all(|c| c.engine == BrowserEngine::Chromium)
        );
        assert!(candidates.iter().all(|c| !c.headless
            && c.no_cuts
            && c.session_cookie.as_deref() == Some("private-cookie")));
        let pinned = launcher.host_path("chosen-host");
        assert!(!pinned.automatic());
        let modes = pinned.candidates().unwrap();
        assert_eq!(modes.len(), if cfg!(target_os = "linux") { 2 } else { 1 });
        assert!(
            modes
                .iter()
                .all(|mode| mode.host_path == pinned.host_path && mode.engine == pinned.engine)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn chromium_keeps_memory_flags_and_cookie_out_of_arguments() {
        let root = scratch("chromium-flags");
        let launcher = HostLauncher::new()
            .data_dir(&root)
            .engine(BrowserEngine::Chromium)
            .session_cookie("private-cookie");
        let command = launcher
            .command(Path::new("chromium"), false, 9222)
            .unwrap();
        let args: Vec<_> = command.as_std().get_args().collect();
        for flag in [
            "--headless=new",
            "--enable-low-end-device-mode",
            "--disable-extensions",
            "--disable-background-networking",
        ] {
            assert!(args.contains(&std::ffi::OsStr::new(flag)));
        }
        assert!(
            !args
                .iter()
                .any(|arg| arg.to_string_lossy().starts_with("--mute-audio"))
        );
        assert!(!format!("{command:?}").contains("private-cookie"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_engine_preserves_executable_type() {
        let launcher = HostLauncher::new().host_path("chosen-host");
        assert!(
            launcher
                .clone()
                .engine(BrowserEngine::Qt)
                .resolve_host()
                .unwrap()
                .1
        );
        assert!(
            !launcher
                .engine(BrowserEngine::Chromium)
                .resolve_host()
                .unwrap()
                .1
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn system_shutdown_allows_the_host_to_finish() {
        let child = tokio::process::Command::new("sleep")
            .arg("0.1")
            .spawn()
            .unwrap();
        let mut host = ManagedHost {
            child: Some(child),
            process_group: None,
            port: 0,
            cdp_url: String::new(),
            is_custom_engine: true,
            engine: BrowserEngine::System,
        };
        host.shutdown().await.unwrap();
        assert!(host.child.is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shutdown_waits_for_the_owned_process_and_is_idempotent() {
        let child = tokio::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        let mut host = super::ManagedHost {
            child: Some(child),
            process_group: None,
            port: 0,
            cdp_url: String::new(),
            is_custom_engine: true,
            engine: BrowserEngine::Qt,
        };
        host.shutdown().await.unwrap();
        assert!(host.child.is_none());
        host.shutdown().await.unwrap();
    }

    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("spotcore-host-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn widevine_scan_finds_bundled_cdm_without_version_directory() {
        let root = scratch("bundled-cdm");
        let platform = root
            .join("WidevineCdm/_platform_specific")
            .join(widevine_platform());
        std::fs::create_dir_all(&platform).unwrap();
        std::fs::write(platform.join(WIDEVINE_LIB), b"fake").unwrap();
        assert_eq!(
            find_widevine_in_profile(&root).unwrap(),
            platform.join(WIDEVINE_LIB)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn custom_paths_preserve_spaces_and_unicode() {
        let root = scratch("app space 音楽");
        let profile = root.join("profile space 音楽");
        let launcher = HostLauncher::new().profile_dir(&profile).data_dir(&root);
        let command = launcher
            .command(Path::new("spotcore-engine"), true, 1234)
            .unwrap();
        let args: Vec<_> = command.as_std().get_args().collect();
        assert_eq!(args[0], "--parent-pid");
        assert_eq!(args[1], std::process::id().to_string().as_str());
        assert_eq!(args[2], "--backend");
        assert_eq!(args[3], "spotcore-engine");
        assert_eq!(args[4], "--");
        assert_eq!(args[5], "--profile-dir");
        assert_eq!(args[6], profile.as_os_str());
        assert_eq!(launcher.root().unwrap(), root);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn widevine_uses_newest_matching_architecture() {
        let root = scratch("versions");
        for (version, arch) in [
            ("4.9.9.0", widevine_platform()),
            ("4.10.1.0", widevine_platform()),
            ("99.0.0.0", "foreign_cpu".into()),
        ] {
            let dir = root
                .join("WidevineCdm")
                .join(version)
                .join("_platform_specific")
                .join(arch);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(WIDEVINE_LIB), b"fake").unwrap();
        }
        assert!(
            find_widevine_in_profile(&root)
                .unwrap()
                .components()
                .any(|part| part.as_os_str() == "4.10.1.0")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn profile_override_keeps_data_root_and_secrets_are_redacted() {
        let root = scratch("root");
        let launcher = HostLauncher::new()
            .data_dir(&root)
            .profile_dir(root.join("profiles/account"))
            .session_cookie("secret-cookie");
        assert_eq!(launcher.root().unwrap(), root);
        assert!(!format!("{launcher:?}").contains("secret-cookie"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn host_bin_names_carry_platform_suffix() {
        let names = host_bin_names();
        assert_eq!(names[0], format!("spotcore-qt-engine{EXE_SUFFIX}"));
        assert_eq!(names[1], format!("spotcore{EXE_SUFFIX}"));
        if cfg!(windows) {
            assert!(names.iter().all(|n| n.ends_with(".exe")));
        } else {
            assert!(names.iter().all(|n| !n.ends_with(".exe")));
        }
    }

    #[test]
    fn session_cookie_is_passed_in_the_environment_only() {
        let launcher = HostLauncher::new().session_cookie("AQDsecret");
        let cmd = launcher
            .command(Path::new("/tmp/spotcore-engine"), true, 1234)
            .unwrap();
        let cmd = cmd.as_std();
        assert_eq!(
            cmd.get_envs()
                .find(|(key, _)| *key == "SPOTCORE_SP_DC")
                .and_then(|(_, value)| value),
            Some(std::ffi::OsStr::new("AQDsecret"))
        );
        assert!(
            !cmd.get_args()
                .any(|arg| arg.to_string_lossy().contains("AQDsecret"))
        );

        let empty = HostLauncher::new().session_cookie("");
        let empty = empty
            .command(Path::new("/tmp/spotcore-engine"), true, 1234)
            .unwrap();
        assert!(
            empty
                .as_std()
                .get_envs()
                .all(|(key, _)| key != "SPOTCORE_SP_DC")
        );
    }

    #[test]
    fn default_profile_dir_is_under_spotcore() {
        let dir = default_profile_dir();
        assert!(
            dir.ends_with("spotcore/profile"),
            "unexpected path: {dir:?}"
        );
    }

    #[test]
    fn widevine_scan_walks_version_and_arch() {
        let root = scratch("widevine");
        let arch = root
            .join("WidevineCdm")
            .join("4.10.3112.0")
            .join("_platform_specific")
            .join(widevine_platform());
        std::fs::create_dir_all(&arch).unwrap();
        std::fs::write(arch.join(WIDEVINE_LIB), b"fake").unwrap();

        let found = find_widevine_in_profile(&root).expect("cdm should be found");
        assert_eq!(found.file_name().unwrap(), WIDEVINE_LIB);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn engine_identity_rejects_a_foreign_endpoint() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        for (browser, accepted) in [("spotcore-engine/", true), ("Chrome/140.0.0.0", false)] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let body = format!("{{\"Browser\":\"{browser}\"}}");
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });

            let url = format!("http://{addr}/json/version");
            let client = reqwest::Client::new();
            let result = verify_engine_identity(&client, &url, "http://127.0.0.1:1").await;
            assert_eq!(result.is_ok(), accepted, "browser {browser:?}");
            server.await.unwrap();
        }
    }

    #[test]
    fn widevine_scan_returns_none_without_cdm() {
        let root = scratch("empty");
        assert!(find_widevine_in_profile(&root).is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
