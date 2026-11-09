use std::fs;
use std::io::{self, Cursor};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const MAX_BINARY_AGE: Duration = Duration::from_secs(85 * 24 * 60 * 60);

#[derive(Debug, thiserror::Error)]
pub enum ManagerError {
    #[error("Unsupported platform: {os} {arch}")]
    UnsupportedPlatform { os: String, arch: String },
    #[error("Failed to download Soloist release: {0}")]
    Download(String),
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Archive extraction error: {0}")]
    Extraction(String),
    #[error("native executable is in use; stop its owned sessions before updating")]
    InUse,
}

#[derive(Debug, Clone)]
pub struct SoloistManager {
    root: PathBuf,
}

impl Default for SoloistManager {
    fn default() -> Self {
        Self::new(spotcore_core::paths::Paths::default().native())
    }
}

impl SoloistManager {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn binary_path(&self) -> PathBuf {
        let exe = if cfg!(windows) {
            "spotcore-native.exe"
        } else {
            "spotcore-native"
        };
        self.root.join(exe)
    }

    /// Resolves the official Spotify CDN archive URL for the current host architecture.
    pub fn release_url() -> Result<(&'static str, &'static str), ManagerError> {
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;

        match (os, arch) {
            ("linux", "x86_64") => Ok((
                "https://soloist-builds.spotifycdn.com/soloist_release_x86_64.tar.gz",
                "tar.gz",
            )),
            ("linux", "aarch64") => Ok((
                "https://soloist-builds.spotifycdn.com/soloist_release_arm64.tar.gz",
                "tar.gz",
            )),
            ("linux", "arm") => Ok((
                "https://soloist-builds.spotifycdn.com/soloist_release_arm32.tar.gz",
                "tar.gz",
            )),
            ("macos", "aarch64") => Ok((
                "https://soloist-builds.spotifycdn.com/soloist_release_osx_arm64.tar.gz",
                "tar.gz",
            )),
            ("windows", "x86_64") => Ok((
                "https://soloist-builds.spotifycdn.com/soloist_release_windows_x86_64.zip",
                "zip",
            )),
            _ => Err(ManagerError::UnsupportedPlatform {
                os: os.to_string(),
                arch: arch.to_string(),
            }),
        }
    }

    pub fn is_binary_fresh(&self) -> bool {
        if self.root.join("installed.pending").exists() {
            return false;
        }
        let Some(installed) = self.installed() else {
            return false;
        };
        let age = SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(installed.downloaded_at))
            .and_then(|time| SystemTime::now().duration_since(time).ok());
        installed.file == self.binary_path().file_name().unwrap().to_string_lossy()
            && age.is_some_and(|age| age < MAX_BINARY_AGE)
            && fs::metadata(self.root.join(installed.file))
                .is_ok_and(|meta| meta.is_file() && meta.len() >= 4 && meta.len() == installed.size)
    }

    fn installed(&self) -> Option<Installed> {
        let installed: Installed = serde_json::from_slice(
            &fs::read(self.root.join("installed.json"))
                .or_else(|_| fs::read(self.root.join("installed.previous")))
                .ok()?,
        )
        .ok()?;
        if Path::new(&installed.file).components().count() != 1
            || !(installed.file == self.binary_path().file_name().unwrap().to_string_lossy()
                || (installed.file.starts_with("spotcore-native-")
                    && installed
                        .file
                        .ends_with(if cfg!(windows) { ".exe" } else { ".bin" })))
        {
            return None;
        }
        Some(installed)
    }

    /// Refreshes after 85 days; download age is a conservative policy, not a build-expiry assertion.
    pub async fn ensure_binary(&self) -> Result<PathBuf, ManagerError> {
        Self::release_url()?;
        fs::create_dir_all(&self.root)?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("update.lock"))?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock)
                    if tokio::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(Duration::from_millis(100)).await
                }
                Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
                _ => {
                    return Err(ManagerError::Download(
                        "another Soloist update is still running".into(),
                    ));
                }
            }
        }
        if self.is_binary_fresh() {
            return Ok(self.binary_path());
        }
        let execution = self.execution_lock(false)?;
        self.recover_install()?;
        if let Some(old) = self.installed() {
            let age = SystemTime::UNIX_EPOCH
                .checked_add(Duration::from_secs(old.downloaded_at))
                .and_then(|time| SystemTime::now().duration_since(time).ok());
            let source = self.root.join(&old.file);
            if age.is_some_and(|age| age < MAX_BINARY_AGE)
                && fs::metadata(&source).is_ok_and(|meta| meta.len() == old.size && meta.len() >= 4)
            {
                let staged = Staged(self.root.join("native-migration.tmp"));
                fs::copy(&source, &staged.0)?;
                validate_binary(&staged.0)?;
                let notice = Staged(self.root.join("native-migration.licenses.tmp"));
                let source_notice = if source == self.binary_path() {
                    self.root.join("spotcore-native.licenses.txt")
                } else {
                    self.root.join(format!("{}.licenses.txt", old.file))
                };
                if source_notice.is_file() {
                    fs::copy(source_notice, &notice.0)?;
                }
                return self.publish(&staged.0, &notice.0, old.size, old.downloaded_at);
            }
        }
        drop(execution);
        let (url, format) = Self::release_url()?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                let url = attempt.url();
                if attempt.previous().len() < 5
                    && url.scheme() == "https"
                    && url.host_str().is_some_and(|host| {
                        host == "spotifycdn.com" || host.ends_with(".spotifycdn.com")
                    })
                {
                    attempt.follow()
                } else {
                    attempt.error("Soloist redirect outside Spotify CDN")
                }
            }))
            .build()
            .map_err(|e| ManagerError::Download(e.to_string()))?;
        let mut response = client
            .get(url)
            .send()
            .await
            .map_err(|e| ManagerError::Download(e.to_string()))?
            .error_for_status()
            .map_err(|e| ManagerError::Download(e.to_string()))?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| ManagerError::Download(e.to_string()))?
        {
            if bytes.len() + chunk.len() > MAX_BINARY_BYTES as usize {
                return Err(ManagerError::Download(
                    "Soloist archive exceeds size limit".into(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let manager = self.clone();
        tokio::task::spawn_blocking(move || {
            let _lock = lock;
            manager.install_archive(&bytes, format)
        })
        .await
        .map_err(|error| ManagerError::Download(format!("Soloist update task failed: {error}")))?
    }

    fn install_archive(&self, bytes: &[u8], format: &str) -> Result<PathBuf, ManagerError> {
        let _execution = self.execution_lock(false)?;
        self.recover_install()?;
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|_| ManagerError::Download("system clock is before Unix epoch".into()))?;
        let file = format!(
            "spotcore-native-{}-{}{}",
            stamp.as_nanos(),
            std::process::id(),
            if cfg!(windows) { ".exe" } else { ".bin" }
        );
        let staged = Staged(self.root.join(&file));
        let notice = Staged(self.root.join(format!("{file}.licenses.txt")));
        let mut out = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged.0)?;
        match format {
            "tar.gz" => {
                let mut archive =
                    tar::Archive::new(flate2::read::GzDecoder::new(Cursor::new(bytes)));
                let mut found = false;
                for entry in archive
                    .entries()
                    .map_err(|e| ManagerError::Extraction(e.to_string()))?
                {
                    let mut entry = entry.map_err(|e| ManagerError::Extraction(e.to_string()))?;
                    if !entry.header().entry_type().is_file() {
                        continue;
                    }
                    let path = entry.path()?.into_owned();
                    if path.file_name().is_some_and(|name| name == "soloist") {
                        if found {
                            return Err(ManagerError::Extraction(
                                "duplicate Soloist executable".into(),
                            ));
                        }
                        copy_binary(&mut entry, &mut out)?;
                        found = true;
                    } else if path
                        .file_name()
                        .is_some_and(|name| name == "THIRD_PARTY_LICENSES.txt")
                    {
                        copy_notice(&mut entry, &notice.0)?;
                    }
                }
                if !found {
                    return Err(ManagerError::Extraction(
                        "archive contains no Soloist executable".into(),
                    ));
                }
            }
            "zip" => extract_zip_binary(bytes, &mut out, Some(&notice.0))?,
            _ => {
                return Err(ManagerError::Extraction(
                    "unsupported archive format".into(),
                ));
            }
        }
        out.sync_all()?;
        validate_binary(&staged.0)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&staged.0, fs::Permissions::from_mode(0o755))?;
        }
        let size = out.metadata()?.len();
        drop(out);
        self.publish(&staged.0, &notice.0, size, stamp.as_secs())
    }

    /// Holds a shared execution lease until the caller has stopped the daemon.
    pub async fn acquire_binary(&self) -> Result<(PathBuf, fs::File), ManagerError> {
        for _ in 0..3 {
            let path = self.ensure_binary().await?;
            let lease = self.execution_lock(true)?;
            if self.is_binary_fresh() {
                return Ok((path, lease));
            }
        }
        Err(ManagerError::InUse)
    }

    fn execution_lock(&self, shared: bool) -> Result<fs::File, ManagerError> {
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("run.lock"))?;
        let result = if shared {
            file.try_lock_shared()
        } else {
            file.try_lock()
        };
        result.map_err(|error| match error {
            fs::TryLockError::WouldBlock => ManagerError::InUse,
            fs::TryLockError::Error(error) => ManagerError::Io(error),
        })?;
        Ok(file)
    }

    fn publish(
        &self,
        staged: &Path,
        notice: &Path,
        size: u64,
        downloaded_at: u64,
    ) -> Result<PathBuf, ManagerError> {
        let bin = self.binary_path();
        let licenses = self.root.join("spotcore-native.licenses.txt");
        let pending = Pending {
            binary: bin.exists(),
            notice: licenses.exists(),
            installed: self.installed(),
        };
        for backup in ["native.previous", "licenses.previous"] {
            let backup = self.root.join(backup);
            if backup.exists() {
                fs::remove_file(backup)?;
            }
        }
        let journal = Staged(self.root.join("installed.pending.tmp"));
        write_json(&journal.0, &pending)?;
        fs::rename(&journal.0, self.root.join("installed.pending"))?;
        let result = (|| -> Result<(), ManagerError> {
            if pending.binary {
                fs::rename(&bin, self.root.join("native.previous"))?;
            }
            if pending.notice {
                fs::rename(&licenses, self.root.join("licenses.previous"))?;
            }
            fs::rename(staged, &bin)?;
            if notice.exists() {
                fs::rename(notice, &licenses)?;
            }
            let installed = Installed {
                file: bin.file_name().unwrap().to_string_lossy().into_owned(),
                size,
                downloaded_at,
            };
            let metadata = Staged(self.root.join("installed.tmp"));
            write_json(&metadata.0, &installed)?;
            replace_metadata(&metadata.0, &self.root.join("installed.json"))?;
            fs::remove_file(self.root.join("installed.pending"))?;
            Ok(())
        })();
        if let Err(error) = result {
            self.recover_install()?;
            return Err(error);
        }
        let _ = fs::remove_file(self.root.join("native.previous"));
        let _ = fs::remove_file(self.root.join("licenses.previous"));
        // A generation from an earlier SDK may still be executing on Windows.
        if let Some(old) = pending.installed
            && old.file != bin.file_name().unwrap().to_string_lossy()
            && fs::remove_file(self.root.join(&old.file)).is_ok()
        {
            let _ = fs::remove_file(self.root.join(format!("{}.licenses.txt", old.file)));
        }
        Ok(bin)
    }

    fn recover_install(&self) -> Result<(), ManagerError> {
        let path = self.root.join("installed.pending");
        if !path.exists() {
            return Ok(());
        }
        let pending: Pending = serde_json::from_slice(&fs::read(&path)?).map_err(|_| {
            ManagerError::Extraction("invalid interrupted installation record".into())
        })?;
        for (target, backup, existed) in [
            (
                self.binary_path(),
                self.root.join("native.previous"),
                pending.binary,
            ),
            (
                self.root.join("spotcore-native.licenses.txt"),
                self.root.join("licenses.previous"),
                pending.notice,
            ),
        ] {
            if backup.exists() {
                if target.exists() {
                    fs::remove_file(&target)?;
                }
                fs::rename(backup, target)?;
            } else if !existed && target.exists() {
                fs::remove_file(target)?;
            }
        }
        if let Some(installed) = pending.installed {
            let staged = Staged(self.root.join("installed.recovery.tmp"));
            write_json(&staged.0, &installed)?;
            replace_metadata(&staged.0, &self.root.join("installed.json"))?;
        } else if self.root.join("installed.json").exists() {
            fs::remove_file(self.root.join("installed.json"))?;
        }
        fs::remove_file(path)?;
        Ok(())
    }
}

const MAX_BINARY_BYTES: u64 = 256 * 1024 * 1024;

#[derive(serde::Serialize, serde::Deserialize)]
struct Pending {
    binary: bool,
    notice: bool,
    installed: Option<Installed>,
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<(), ManagerError> {
    use io::Write;
    let mut file = fs::File::create(path)?;
    file.write_all(
        &serde_json::to_vec(value).map_err(|e| ManagerError::Extraction(e.to_string()))?,
    )?;
    file.sync_all()?;
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Installed {
    file: String,
    size: u64,
    downloaded_at: u64,
}

struct Staged(PathBuf);
impl Drop for Staged {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn copy_binary(reader: &mut impl io::Read, out: &mut fs::File) -> Result<(), ManagerError> {
    let size = io::copy(&mut io::Read::take(reader, MAX_BINARY_BYTES + 1), out)?;
    if size == 0 || size > MAX_BINARY_BYTES {
        return Err(ManagerError::Extraction(
            "invalid Soloist executable size".into(),
        ));
    }
    Ok(())
}

fn validate_binary(path: &Path) -> Result<(), ManagerError> {
    use io::Read;
    let mut magic = [0; 4];
    fs::File::open(path)?.read_exact(&mut magic)?;
    let valid = if cfg!(windows) {
        magic.starts_with(b"MZ")
    } else if cfg!(target_os = "macos") {
        matches!(
            magic,
            [0xcf, 0xfa, 0xed, 0xfe] | [0xca, 0xfe, 0xba, 0xbe] | [0xbe, 0xba, 0xfe, 0xca]
        )
    } else {
        magic == *b"\x7fELF"
    };
    if !valid {
        return Err(ManagerError::Extraction(
            "archive does not contain a platform executable".into(),
        ));
    }
    Ok(())
}

fn replace_metadata(staged: &Path, target: &Path) -> Result<(), io::Error> {
    #[cfg(windows)]
    if target.exists() {
        let backup = target.with_extension("previous");
        if backup.exists() {
            fs::remove_file(&backup)?;
        }
        fs::rename(target, &backup)?;
        if let Err(error) = fs::rename(staged, target) {
            fs::rename(&backup, target)?;
            return Err(error);
        }
        let _ = fs::remove_file(backup);
        return Ok(());
    }
    fs::rename(staged, target)
}

fn copy_notice(reader: &mut impl io::Read, path: &Path) -> Result<(), ManagerError> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    copy_binary(reader, &mut file)?;
    file.sync_all()?;
    Ok(())
}

fn extract_zip_binary(
    bytes: &[u8],
    out: &mut fs::File,
    notice: Option<&Path>,
) -> Result<(), ManagerError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| ManagerError::Extraction(e.to_string()))?;
    let mut found = false;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| ManagerError::Extraction(e.to_string()))?;
        if entry.is_dir()
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            continue;
        }
        let entry_name = entry
            .name()
            .map_err(|e| ManagerError::Extraction(e.to_string()))?
            .into_owned();
        let name = Path::new(&entry_name)
            .file_name()
            .and_then(|name| name.to_str());
        if matches!(name, Some("soloist" | "soloist.exe")) {
            if found {
                return Err(ManagerError::Extraction(
                    "duplicate Soloist executable".into(),
                ));
            }
            copy_binary(&mut entry, out)?;
            found = true;
        } else if name == Some("THIRD_PARTY_LICENSES.txt")
            && let Some(path) = notice
        {
            copy_notice(&mut entry, path)?;
        }
    }
    if found {
        Ok(())
    } else {
        Err(ManagerError::Extraction(
            "archive contains no Soloist executable".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive() -> Vec<u8> {
        use std::io::Write;
        let mut data = Cursor::new(Vec::new());
        let magic: &[u8] = if cfg!(windows) {
            b"MZ00"
        } else if cfg!(target_os = "macos") {
            &[0xcf, 0xfa, 0xed, 0xfe]
        } else {
            b"\x7fELF"
        };
        {
            let mut zip = zip::ZipWriter::new(&mut data);
            zip.start_file("soloist", zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(magic).unwrap();
            zip.start_file(
                "THIRD_PARTY_LICENSES.txt",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(b"vendor notices").unwrap();
            zip.finish().unwrap();
        }
        data.into_inner()
    }

    #[test]
    fn update_uses_fixed_name_and_preserves_existing_on_failure() {
        use std::io::Read;
        let root = std::env::temp_dir().join(format!("spotcore-install-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let manager = SoloistManager::new(&root);
        let first = manager.install_archive(&archive(), "zip").unwrap();
        let mut running = fs::File::open(&first).unwrap();
        assert_eq!(
            fs::read(manager.root.join("spotcore-native.licenses.txt")).unwrap(),
            b"vendor notices"
        );
        assert!(manager.is_binary_fresh());
        assert!(manager.install_archive(b"not an archive", "zip").is_err());
        assert_eq!(manager.binary_path(), first);
        let lease = manager.execution_lock(true).unwrap();
        assert!(matches!(
            manager.install_archive(&archive(), "zip"),
            Err(ManagerError::InUse)
        ));
        drop(lease);
        let second = manager.install_archive(&archive(), "zip").unwrap();
        assert_eq!(first, second);
        assert_eq!(manager.binary_path(), second);
        let mut bytes = Vec::new();
        running.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes.len(), 4);
        let mut installed = manager.installed().unwrap();
        installed.downloaded_at = u64::MAX;
        fs::write(
            root.join("installed.json"),
            serde_json::to_vec(&installed).unwrap(),
        )
        .unwrap();
        assert!(!manager.is_binary_fresh());
        drop(running);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn concurrent_update_waits_and_reuses_fresh_install() {
        if SoloistManager::release_url().is_err() {
            return;
        }
        let root = std::env::temp_dir().join(format!("spotcore-lock-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let manager = SoloistManager::new(&root);
        let expected = manager.install_archive(&archive(), "zip").unwrap();
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("update.lock"))
            .unwrap();
        lock.try_lock().unwrap();
        let task = tokio::spawn(async move { manager.ensure_binary().await });
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!task.is_finished());
        drop(lock);
        assert_eq!(task.await.unwrap().unwrap(), expected);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interrupted_replacement_restores_binary_metadata_and_notices() {
        let root = std::env::temp_dir().join(format!("spotcore-recovery-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let manager = SoloistManager::new(&root);
        let bin = manager.install_archive(&archive(), "zip").unwrap();
        let original = fs::read(&bin).unwrap();
        let pending = Pending {
            binary: true,
            notice: true,
            installed: manager.installed(),
        };
        write_json(&root.join("installed.pending"), &pending).unwrap();
        fs::rename(&bin, root.join("native.previous")).unwrap();
        fs::rename(
            root.join("spotcore-native.licenses.txt"),
            root.join("licenses.previous"),
        )
        .unwrap();
        fs::write(&bin, b"incomplete replacement").unwrap();
        fs::write(root.join("installed.json"), b"incomplete metadata").unwrap();
        assert!(!manager.is_binary_fresh());
        manager.recover_install().unwrap();
        assert_eq!(fs::read(bin).unwrap(), original);
        assert_eq!(
            fs::read(root.join("spotcore-native.licenses.txt")).unwrap(),
            b"vendor notices"
        );
        assert!(manager.is_binary_fresh());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn migrates_generations_without_resetting_download_age() {
        if SoloistManager::release_url().is_err() {
            return;
        }
        let root = std::env::temp_dir().join(format!("spotcore-migration-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let manager = SoloistManager::new(&root);
        let bin = manager.install_archive(&archive(), "zip").unwrap();
        let mut installed = manager.installed().unwrap();
        installed.file = format!(
            "spotcore-native-123-456{}",
            if cfg!(windows) { ".exe" } else { ".bin" }
        );
        installed.downloaded_at -= 60;
        let downloaded_at = installed.downloaded_at;
        fs::rename(&bin, root.join(&installed.file)).unwrap();
        fs::rename(
            root.join("spotcore-native.licenses.txt"),
            root.join(format!("{}.licenses.txt", installed.file)),
        )
        .unwrap();
        write_json(&root.join("installed.json"), &installed).unwrap();
        assert!(!manager.is_binary_fresh());
        assert_eq!(manager.ensure_binary().await.unwrap(), bin);
        assert_eq!(manager.installed().unwrap().downloaded_at, downloaded_at);
        assert!(manager.is_binary_fresh());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn test_release_url_resolution() {
        match SoloistManager::release_url() {
            Ok((url, format)) => {
                assert!(url.starts_with("https://soloist-builds.spotifycdn.com/soloist_release_"));
                assert!(format == "tar.gz" || format == "zip");
            }
            Err(ManagerError::UnsupportedPlatform { os, arch }) => {
                assert_eq!(os, std::env::consts::OS);
                assert_eq!(arch, std::env::consts::ARCH);
            }
            Err(error) => panic!("unexpected resolution error: {error}"),
        }
    }

    #[test]
    fn test_manager_cache_paths() {
        let base = std::env::temp_dir().join("test_soloist");
        let manager = SoloistManager::new(&base);
        assert_eq!(manager.root(), base.as_path());
        let bin_name = if cfg!(windows) {
            "spotcore-native.exe"
        } else {
            "spotcore-native"
        };
        assert_eq!(manager.binary_path(), base.join(bin_name));
    }

    #[test]
    fn zip_archive_yields_binary() {
        use std::io::Write;

        let mut buffer = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            writer
                .start_file("soloist.exe", zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"binary").unwrap();
            writer.finish().unwrap();
        }

        let dir = std::env::temp_dir().join(format!("spotcore_zip_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("spotcore-native.exe");
        extract_zip_binary(buffer.get_ref(), &mut fs::File::create(&bin).unwrap(), None).unwrap();
        assert_eq!(std::fs::read(&bin).unwrap(), b"binary");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
