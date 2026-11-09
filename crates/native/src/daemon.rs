use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::time::sleep;

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("IO error running Soloist daemon: {0}")]
    Io(#[from] std::io::Error),
    #[error("Daemon exited prematurely")]
    ExitedEarly,
    #[error("Timed out waiting for Soloist WebSocket server on {0}")]
    Timeout(String),
}

/// Device name shown in the Spotify Connect picker when the caller sets none.
pub const DEFAULT_DEVICE_NAME: &str = "SpotCore";

#[derive(Debug, Clone)]
pub struct SoloistConfig {
    pub binary_path: PathBuf,
    pub api_key: String,
    pub device_name: String,
    pub ws_port: u16,
    pub data_dir: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
}

impl SoloistConfig {
    /// Keeps the daemon's state next to the SDK's native directory for these paths.
    pub fn new(
        binary_path: impl Into<PathBuf>,
        api_key: impl Into<String>,
        paths: &spotcore_core::paths::Paths,
    ) -> Self {
        Self {
            binary_path: binary_path.into(),
            api_key: api_key.into(),
            device_name: DEFAULT_DEVICE_NAME.to_string(),
            ws_port: 0,
            data_dir: Some(paths.native_data()),
            cache_dir: Some(paths.native_cache()),
        }
    }
}

pub struct SoloistProcess {
    child: Child,
    ws_url: String,
}

impl SoloistProcess {
    pub fn ws_url(&self) -> &str {
        &self.ws_url
    }

    /// Spawns the official Spotify Soloist daemon and waits until the local WebSocket is ready.
    pub async fn spawn(config: SoloistConfig) -> Result<Self, DaemonError> {
        if let (Some(data_dir), Some(cache_dir)) = (&config.data_dir, &config.cache_dir) {
            import_legacy_state(data_dir, cache_dir);
        }

        let port = if config.ws_port == 0 {
            let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
            let port = listener.local_addr()?.port();
            drop(listener);
            port
        } else {
            config.ws_port
        };

        let ws_addr = format!("127.0.0.1:{}", port);
        let ws_url = format!("ws://{}", ws_addr);

        let mut cmd = Command::new(&config.binary_path);
        // `--flag=value` keeps a value starting with `-` from parsing as a flag.
        cmd.arg(format!("--device-name={}", config.device_name))
            .arg(format!("--api-key={}", config.api_key))
            .arg("--ws")
            .arg(&ws_addr)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(ref data_dir) = config.data_dir {
            private_dir(data_dir)?;
            cmd.arg("--data-dir").arg(data_dir);
        }
        if let Some(ref cache_dir) = config.cache_dir {
            private_dir(cache_dir)?;
            cmd.arg("--cache-dir").arg(cache_dir);
        }

        log::info!(
            "Spawning Soloist daemon with device '{}' on {}...",
            config.device_name,
            ws_addr
        );

        let mut child = cmd.spawn()?;

        // Undrained output pipes eventually block the daemon.
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(async move {
                use tokio::io::AsyncBufReadExt;
                let mut lines = tokio::io::BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    log::debug!("soloist: {line}");
                }
            });
        }
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                use tokio::io::AsyncBufReadExt;
                let mut lines = tokio::io::BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    log::warn!("soloist: {line}");
                }
            });
        }

        let mut connected = false;
        for _ in 0..50 {
            if let Ok(Some(status)) = child.try_wait() {
                log::error!("Soloist process exited unexpectedly with: {}", status);
                return Err(DaemonError::ExitedEarly);
            }

            if tokio::net::TcpStream::connect(&ws_addr).await.is_ok() {
                connected = true;
                break;
            }
            sleep(Duration::from_millis(200)).await;
        }

        if !connected {
            let _ = child.kill().await;
            return Err(DaemonError::Timeout(ws_url));
        }

        log::info!("Soloist daemon ready! WebSocket listening on {}", ws_url);

        Ok(Self { child, ws_url })
    }

    pub async fn kill(&mut self) -> std::io::Result<()> {
        if self.child.try_wait()?.is_none() {
            self.child.kill().await?;
        }
        Ok(())
    }
}

impl Drop for SoloistProcess {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

fn private_dir(path: &std::path::Path) -> std::io::Result<()> {
    spotcore_core::Paths::new(path).prepare()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

// Import legacy login state once; never overwrite an existing session.
fn import_legacy_state(data_dir: &std::path::Path, cache_dir: &std::path::Path) {
    if data_dir.join("settings").is_dir() && cache_dir.join("client_token").is_file() {
        return;
    }
    let Ok(platform) = spotcore_core::paths::Paths::platform() else {
        return;
    };
    if data_dir != platform.native_data() || cache_dir != platform.native_cache() {
        return;
    }
    let Some(legacy) = platform
        .root()
        .parent()
        .map(|parent| parent.join("soloist"))
    else {
        return;
    };
    import_from(&legacy, data_dir, cache_dir);
}

fn import_from(legacy: &std::path::Path, data_dir: &std::path::Path, cache_dir: &std::path::Path) {
    if !legacy.join("settings").is_dir() {
        return;
    }
    let items = [
        (legacy.join("settings"), data_dir.join("settings")),
        (legacy.join(".device_id"), data_dir.join(".device_id")),
        (legacy.join("cache"), cache_dir.to_path_buf()),
    ];
    for (src, dst) in items {
        let _ = copy_missing(&src, &dst);
    }
}

// Existing login files take precedence over imported state.
fn copy_missing(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    let meta = std::fs::metadata(src)?;
    if meta.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_missing(&entry.path(), &dst.join(entry.file_name()))?;
        }
    } else if !dst.exists() {
        std::fs::copy(src, dst)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_login_state_without_overwriting() {
        let base = std::env::temp_dir().join(format!("spotcore_import_{}", std::process::id()));
        let legacy = base.join("soloist");
        let data_dir = base.join("native/data");
        let cache_dir = base.join("native/cache");
        let _ = std::fs::remove_dir_all(&base);

        std::fs::create_dir_all(legacy.join("settings/Users/user")).unwrap();
        std::fs::write(legacy.join("settings/Users/user/cached"), b"session").unwrap();
        std::fs::write(legacy.join(".device_id"), b"legacy-device").unwrap();
        std::fs::create_dir_all(legacy.join("cache")).unwrap();
        std::fs::write(legacy.join("cache/client_token"), b"token").unwrap();

        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(data_dir.join(".device_id"), b"current-device").unwrap();

        import_from(&legacy, &data_dir, &cache_dir);

        assert_eq!(
            std::fs::read(data_dir.join(".device_id")).unwrap(),
            b"current-device"
        );
        assert_eq!(
            std::fs::read(data_dir.join("settings/Users/user/cached")).unwrap(),
            b"session"
        );
        assert_eq!(
            std::fs::read(cache_dir.join("client_token")).unwrap(),
            b"token"
        );

        import_from(&legacy, &data_dir, &cache_dir);
        assert!(cache_dir.join("client_token").is_file());

        let _ = std::fs::remove_dir_all(&base);
    }
}
