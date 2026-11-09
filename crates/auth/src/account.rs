use crate::{AccountSecrets, AccountTier, CredentialStore, OsCredentialStore};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs::{DirBuilder, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum PlaybackMode {
    Web,
    Native,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebCredentials {
    pub sp_dc: Option<String>,
    pub access_token: Option<String>,
    pub expires_at_ms: Option<u64>,
}

impl fmt::Debug for WebCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebCredentials")
            .field("sp_dc", &self.sp_dc.as_ref().map(|_| "[REDACTED]"))
            .field(
                "access_token",
                &self.access_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoloistCredentials {
    pub api_key: Option<String>,
}

impl fmt::Debug for SoloistCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SoloistCredentials")
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountProfile {
    pub canonical_username: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub tier: Option<AccountTier>,
    #[serde(default)]
    pub tier_checked_at_ms: Option<u64>,
    #[serde(default)]
    pub web: Option<WebCredentials>,
    #[serde(default)]
    pub soloist: Option<SoloistCredentials>,
}

pub type AccountRecord = AccountProfile;

impl AccountProfile {
    pub fn new(canonical_username: impl Into<String>) -> Self {
        Self {
            canonical_username: canonical_username.into(),
            display_name: None,
            tier: None,
            tier_checked_at_ms: None,
            web: None,
            soloist: None,
        }
    }

    pub fn has_web(&self) -> bool {
        self.web.is_some()
    }

    pub fn native_available(&self) -> bool {
        self.soloist.is_some() && self.tier == Some(AccountTier::Premium)
    }

    pub fn playback_mode(&self) -> PlaybackMode {
        if self.native_available() {
            PlaybackMode::Native
        } else {
            PlaybackMode::Web
        }
    }

    pub fn tier_is_stale(&self, max_age_ms: u64) -> bool {
        self.tier_checked_at_ms
            .map(|checked| now_ms().saturating_sub(checked) > max_age_ms)
            .unwrap_or(true)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedSession {
    pub canonical_username: String,
    pub display_name: Option<String>,
    pub tier: Option<AccountTier>,
    pub playback: PlaybackMode,
    pub web: WebCredentials,
    pub soloist: Option<SoloistCredentials>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountMetadata {
    pub canonical_username: String,
    pub display_name: Option<String>,
    pub tier: Option<AccountTier>,
    pub tier_checked_at_ms: Option<u64>,
    #[serde(default)]
    pub credential_ref: Option<String>,
}

#[derive(Clone)]
pub struct AccountStore {
    base_dir: PathBuf,
    credentials: Arc<dyn CredentialStore>,
}
impl fmt::Debug for AccountStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccountStore")
            .field("base_dir", &self.base_dir)
            .finish_non_exhaustive()
    }
}
impl Default for AccountStore {
    fn default() -> Self {
        Self::new(spotcore_core::paths::Paths::default().accounts())
    }
}

/// Collision-free filesystem component; case-sensitive identities stay distinct on Windows too.
pub fn account_key(id: &str) -> std::io::Result<String> {
    if id.is_empty() || id.len() > 120 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid account ID",
        ));
    }
    Ok(id.as_bytes().iter().map(|b| format!("{b:02x}")).collect())
}
impl AccountStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        let base_dir = base_dir.into();
        let scope = std::path::absolute(&base_dir).unwrap_or_else(|_| base_dir.clone());
        let credentials = Arc::new(OsCredentialStore::new(format!(
            "spotcore:{}",
            scope.display()
        )));
        Self {
            base_dir: scope,
            credentials,
        }
    }
    pub fn with_credential_store(
        base_dir: impl Into<PathBuf>,
        credentials: Arc<dyn CredentialStore>,
    ) -> Self {
        Self {
            base_dir: {
                let path = base_dir.into();
                std::path::absolute(&path).unwrap_or(path)
            },
            credentials,
        }
    }
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }
    pub fn account_path(&self, id: &str) -> PathBuf {
        let key: String = id.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
        self.base_dir.join(format!("{key}.json"))
    }
    fn legacy_path(&self, id: &str) -> PathBuf {
        let key: String = id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        self.base_dir.join(format!("{key}.json"))
    }
    fn read_record(
        &self,
        id: &str,
    ) -> std::io::Result<Option<(AccountMetadata, Option<AccountSecrets>)>> {
        let key = account_key(id)?;
        let path = self.account_path(id);
        let path = if path.exists() {
            path
        } else {
            self.legacy_path(id)
        };
        let data = match std::fs::read(path) {
            Ok(data) => data,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let value: serde_json::Value =
            serde_json::from_slice(&data).map_err(|_| invalid_record())?;
        let metadata: AccountMetadata =
            serde_json::from_value(value.clone()).map_err(|_| invalid_record())?;
        if metadata.canonical_username != id
            || metadata
                .credential_ref
                .as_ref()
                .is_some_and(|r| r != &format!("account-{key}"))
        {
            return Err(invalid_record());
        }
        let secrets = if !value["web"].is_null() || !value["soloist"].is_null() {
            if metadata.credential_ref.is_some() {
                return Err(invalid_record());
            }
            Some(serde_json::from_value(value).map_err(|_| invalid_record())?)
        } else {
            None
        };
        Ok(Some((metadata, secrets)))
    }
    pub fn load_metadata(&self, id: &str) -> std::io::Result<Option<AccountMetadata>> {
        Ok(self.read_record(id)?.map(|(m, _)| m))
    }
    fn lock(&self) -> std::io::Result<std::fs::File> {
        create_private_dir(&self.base_dir)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.base_dir.join("store.lock"))?;
        file.lock()?;
        Ok(file)
    }
    pub fn save_profile(&self, profile: &AccountProfile) -> std::io::Result<()> {
        let _lock = self.lock()?;
        self.save_unlocked(profile)
    }
    fn save_unlocked(&self, profile: &AccountProfile) -> std::io::Result<()> {
        let key = account_key(&profile.canonical_username)?;
        create_private_dir(&self.base_dir)?;
        let old = self
            .read_record(&profile.canonical_username)?
            .map(|(m, _)| m);
        let reference = old
            .as_ref()
            .and_then(|m| m.credential_ref.clone())
            .unwrap_or_else(|| format!("account-{key}"));
        let previous = if old.as_ref().is_some_and(|m| m.credential_ref.is_some()) {
            self.credentials.load(&reference)?
        } else {
            None
        };
        let secrets = AccountSecrets {
            web: profile.web.clone(),
            soloist: profile.soloist.clone(),
        };
        let has_secrets = secrets.web.is_some() || secrets.soloist.is_some();
        if has_secrets {
            self.credentials.save(&reference, &secrets)?;
        } else if previous.is_some() {
            self.credentials.delete(&reference)?;
        }
        let metadata = AccountMetadata {
            canonical_username: profile.canonical_username.clone(),
            display_name: profile.display_name.clone(),
            tier: profile.tier,
            tier_checked_at_ms: profile.tier_checked_at_ms,
            credential_ref: has_secrets.then_some(reference.clone()),
        };
        let data = serde_json::to_vec_pretty(&metadata).map_err(|_| invalid_record())?;
        if let Err(error) = write_private(&self.account_path(&profile.canonical_username), &data) {
            if let Some(previous) = previous {
                self.credentials.save(&reference, &previous)?;
            } else if has_secrets {
                self.credentials.delete(&reference)?;
            }
            return Err(error);
        }
        self.remove_legacy(&profile.canonical_username)?;
        Ok(())
    }
    fn remove_legacy(&self, id: &str) -> std::io::Result<()> {
        let legacy = self.legacy_path(id);
        if legacy != self.account_path(id) && legacy.exists() {
            let profile: AccountProfile =
                serde_json::from_slice(&std::fs::read(&legacy)?).map_err(|_| invalid_record())?;
            if profile.canonical_username != id {
                return Err(invalid_record());
            }
            std::fs::remove_file(legacy)?;
        }
        Ok(())
    }
    pub fn load_profile(&self, id: &str) -> std::io::Result<Option<AccountProfile>> {
        let _lock = self.lock()?;
        self.load_unlocked(id)
    }
    fn load_unlocked(&self, id: &str) -> std::io::Result<Option<AccountProfile>> {
        let Some((metadata, legacy)) = self.read_record(id)? else {
            return Ok(None);
        };
        let secrets = if let Some(secrets) = legacy {
            secrets
        } else if let Some(reference) = &metadata.credential_ref {
            self.credentials.load(reference)?.ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "account credentials missing; sign in again",
                )
            })?
        } else {
            AccountSecrets::default()
        };
        let profile = AccountProfile {
            canonical_username: metadata.canonical_username,
            display_name: metadata.display_name,
            tier: metadata.tier,
            tier_checked_at_ms: metadata.tier_checked_at_ms,
            web: secrets.web,
            soloist: secrets.soloist,
        };
        if metadata.credential_ref.is_none() && (profile.web.is_some() || profile.soloist.is_some())
        {
            self.save_unlocked(&profile)?;
        } else {
            self.remove_legacy(id)?;
        }
        Ok(Some(profile))
    }
    pub fn delete_account(&self, id: &str) -> std::io::Result<bool> {
        let _lock = self.lock()?;
        let Some((metadata, _)) = self.read_record(id)? else {
            return Ok(false);
        };
        if let Some(reference) = metadata.credential_ref {
            self.credentials.delete(&reference)?;
        }
        self.remove_legacy(id)?;
        match std::fs::remove_file(self.account_path(id)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        Ok(true)
    }
    pub fn list_accounts(&self) -> std::io::Result<Vec<String>> {
        if !self.base_dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut ids = Vec::new();
        for entry in std::fs::read_dir(&self.base_dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "json") {
                let metadata: AccountMetadata =
                    serde_json::from_slice(&std::fs::read(path)?).map_err(|_| invalid_record())?;
                account_key(&metadata.canonical_username)?;
                ids.push(metadata.canonical_username);
            }
        }
        ids.sort();
        ids.dedup();
        Ok(ids)
    }
    pub fn record_tier(&self, id: &str, tier: AccountTier) -> std::io::Result<()> {
        let _lock = self.lock()?;
        let Some((_, legacy)) = self.read_record(id)? else {
            return Ok(());
        };
        if legacy.is_some() {
            self.load_unlocked(id)?;
        }
        let Some(mut metadata) = self.load_metadata(id)? else {
            return Ok(());
        };
        metadata.tier = Some(tier);
        metadata.tier_checked_at_ms = Some(now_ms());
        write_private(
            &self.account_path(id),
            &serde_json::to_vec_pretty(&metadata).map_err(|_| invalid_record())?,
        )
    }
    pub fn resolve_session(&self, id: &str) -> std::io::Result<Option<ResolvedSession>> {
        let Some(profile) = self.load_profile(id)? else {
            return Ok(None);
        };
        let Some(web) = profile.web.clone() else {
            return Ok(None);
        };
        Ok(Some(ResolvedSession {
            canonical_username: profile.canonical_username.clone(),
            display_name: profile.display_name.clone(),
            tier: profile.tier,
            playback: profile.playback_mode(),
            web,
            soloist: profile.soloist,
        }))
    }
}
fn invalid_record() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "invalid or mismatched account record",
    )
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Legacy records may still contain secrets until migrated.
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    let mut builder = DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Writes through a temp file so a crash cannot truncate a live account, and
/// tightens the mode on every write, not only when the file is created.
fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    let result = (|| {
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct TestVault(std::sync::Mutex<std::collections::HashMap<String, AccountSecrets>>);
    impl CredentialStore for TestVault {
        fn load(&self, k: &str) -> std::io::Result<Option<AccountSecrets>> {
            Ok(self.0.lock().unwrap().get(k).cloned())
        }
        fn save(&self, k: &str, v: &AccountSecrets) -> std::io::Result<()> {
            self.0.lock().unwrap().insert(k.into(), v.clone());
            Ok(())
        }
        fn delete(&self, k: &str) -> std::io::Result<()> {
            self.0.lock().unwrap().remove(k);
            Ok(())
        }
    }
    fn temp_store(name: &str) -> AccountStore {
        let dir = std::env::temp_dir().join(format!("spotcore_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        AccountStore::with_credential_store(dir, Arc::new(TestVault::default()))
    }

    fn web(sp_dc: &str) -> WebCredentials {
        WebCredentials {
            sp_dc: Some(sp_dc.to_string()),
            access_token: Some("web-token".to_string()),
            expires_at_ms: Some(1_900_000_000_000),
        }
    }

    fn soloist(key: &str) -> SoloistCredentials {
        SoloistCredentials {
            api_key: Some(key.to_string()),
        }
    }

    #[cfg(unix)]
    #[test]
    fn account_file_is_owner_only_even_if_it_already_existed() {
        use std::os::unix::fs::PermissionsExt;
        let store = temp_store("perms");
        let profile = AccountProfile::new("31perms0000000000000000001");
        store.save_profile(&profile).unwrap();

        let path = store.account_path(&profile.canonical_username);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        store.save_profile(&profile).unwrap();

        let file = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let dir = std::fs::metadata(store.base_dir())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file, 0o600);
        assert_eq!(dir, 0o700);
        assert!(!path.with_extension("json.tmp").exists());

        let _ = std::fs::remove_dir_all(store.base_dir());
    }

    #[test]
    fn metadata_and_vault_round_trip_credentials() {
        let store = temp_store("one_file");
        let mut profile = AccountProfile::new("31premium000000000000000001");
        profile.display_name = Some("Premium Listener".into());
        profile.tier = Some(AccountTier::Premium);
        profile.tier_checked_at_ms = Some(now_ms());
        profile.web = Some(web("sp_dc_premium"));
        profile.soloist = Some(soloist("spak_premium"));
        store.save_profile(&profile).unwrap();

        assert!(store.account_path(&profile.canonical_username).is_file());
        let disk =
            std::fs::read_to_string(store.account_path(&profile.canonical_username)).unwrap();
        assert!(!disk.contains("sp_dc_premium"));
        assert!(!disk.contains("spak_premium"));
        assert_eq!(
            store.list_accounts().unwrap(),
            vec![profile.canonical_username.clone()]
        );

        let loaded = store
            .load_profile(&profile.canonical_username)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.web.unwrap().sp_dc.as_deref(), Some("sp_dc_premium"));
        assert_eq!(
            loaded.soloist.unwrap().api_key.as_deref(),
            Some("spak_premium")
        );

        let _ = std::fs::remove_dir_all(store.base_dir());
    }

    #[test]
    fn session_keeps_one_canonical_account_across_web_and_native() {
        let store = temp_store("session_same_account");
        let mut profile = AccountProfile::new("31dual00000000000000000001");
        profile.tier = Some(AccountTier::Premium);
        profile.web = Some(web("sp_dc_dual"));
        profile.soloist = Some(soloist("spak_dual"));
        store.save_profile(&profile).unwrap();

        let session = store
            .resolve_session(&profile.canonical_username)
            .unwrap()
            .unwrap();
        assert_eq!(session.canonical_username, profile.canonical_username);
        assert_eq!(session.playback, PlaybackMode::Native);
        assert_eq!(session.web.sp_dc.as_deref(), Some("sp_dc_dual"));
        assert_eq!(
            session.soloist.unwrap().api_key.as_deref(),
            Some("spak_dual")
        );

        let _ = std::fs::remove_dir_all(store.base_dir());
    }

    #[test]
    fn free_account_with_soloist_key_plays_over_web() {
        let store = temp_store("free_fallback");
        let mut profile = AccountProfile::new("31free00000000000000000001");
        profile.tier = Some(AccountTier::Free);
        profile.web = Some(web("sp_dc_free"));
        profile.soloist = Some(soloist("spak_stale"));
        store.save_profile(&profile).unwrap();

        let session = store
            .resolve_session(&profile.canonical_username)
            .unwrap()
            .unwrap();
        assert_eq!(session.playback, PlaybackMode::Web);
        assert!(
            session.soloist.is_some(),
            "credentials are kept for a later upgrade"
        );

        let _ = std::fs::remove_dir_all(store.base_dir());
    }

    #[test]
    fn profile_without_web_auth_has_no_session() {
        let store = temp_store("no_web");
        let mut profile = AccountProfile::new("31soloist00000000000000001");
        profile.tier = Some(AccountTier::Premium);
        profile.soloist = Some(soloist("spak_only"));
        store.save_profile(&profile).unwrap();

        assert!(
            store
                .resolve_session(&profile.canonical_username)
                .unwrap()
                .is_none()
        );

        let _ = std::fs::remove_dir_all(store.base_dir());
    }

    #[test]
    fn tier_refresh_updates_timestamp() {
        let store = temp_store("tier_refresh");
        let profile = AccountProfile::new("31tier0000000000000000001");
        store.save_profile(&profile).unwrap();
        assert!(profile.tier_is_stale(86_400_000));

        store
            .record_tier(&profile.canonical_username, AccountTier::Free)
            .unwrap();
        let loaded = store
            .load_profile(&profile.canonical_username)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.tier, Some(AccountTier::Free));
        assert!(!loaded.tier_is_stale(86_400_000));

        let _ = std::fs::remove_dir_all(store.base_dir());
    }

    #[test]
    fn delete_removes_the_account_file() {
        let store = temp_store("delete");
        let profile = AccountProfile::new("31delete00000000000000001");
        store.save_profile(&profile).unwrap();
        assert!(store.delete_account(&profile.canonical_username).unwrap());
        assert!(
            store
                .load_profile(&profile.canonical_username)
                .unwrap()
                .is_none()
        );
        assert!(store.list_accounts().unwrap().is_empty());

        let _ = std::fs::remove_dir_all(store.base_dir());
    }
    struct LockedVault;
    impl CredentialStore for LockedVault {
        fn load(&self, _: &str) -> std::io::Result<Option<AccountSecrets>> {
            Err(std::io::Error::other("vault locked"))
        }
        fn save(&self, _: &str, _: &AccountSecrets) -> std::io::Result<()> {
            Err(std::io::Error::other("vault locked"))
        }
        fn delete(&self, _: &str) -> std::io::Result<()> {
            Err(std::io::Error::other("vault locked"))
        }
    }
    #[test]
    fn legacy_migration_is_secure_and_failure_preserves_the_original() {
        let store = temp_store("legacy");
        create_private_dir(store.base_dir()).unwrap();
        let mut profile = AccountProfile::new("legacy-user");
        profile.web = Some(web("legacy-secret"));
        let old = store.legacy_path("legacy-user");
        let bytes = serde_json::to_vec(&profile).unwrap();
        fs_write(&old, &bytes);
        let locked = AccountStore::with_credential_store(store.base_dir(), Arc::new(LockedVault));
        assert!(locked.load_profile("legacy-user").is_err());
        assert_eq!(std::fs::read(&old).unwrap(), bytes);
        assert!(!store.account_path("legacy-user").exists());
        assert_eq!(
            store
                .load_profile("legacy-user")
                .unwrap()
                .unwrap()
                .web
                .unwrap()
                .sp_dc
                .as_deref(),
            Some("legacy-secret")
        );
        assert!(!old.exists());
        assert!(
            !std::fs::read_to_string(store.account_path("legacy-user"))
                .unwrap()
                .contains("legacy-secret")
        );
        std::fs::remove_dir_all(store.base_dir()).unwrap();
    }
    fn fs_write(path: &Path, bytes: &[u8]) {
        std::fs::write(path, bytes).unwrap();
    }
    #[test]
    fn identities_cannot_collide_or_reference_other_accounts() {
        let store = temp_store("identities");
        for id in ["../user", "user", "User", "用戶"] {
            store.save_profile(&AccountProfile::new(id)).unwrap();
        }
        assert_ne!(store.account_path("../user"), store.account_path("user"));
        assert_ne!(store.account_path("User"), store.account_path("user"));
        assert_eq!(store.list_accounts().unwrap().len(), 4);
        let mut metadata = store.load_metadata("user").unwrap().unwrap();
        metadata.credential_ref = Some("account-other".into());
        fs_write(
            &store.account_path("user"),
            &serde_json::to_vec(&metadata).unwrap(),
        );
        assert!(store.load_profile("user").is_err());
        std::fs::remove_dir_all(store.base_dir()).unwrap();
    }
    #[test]
    fn unavailable_vault_never_falls_back_to_plaintext() {
        let store = temp_store("locked");
        let locked = AccountStore::with_credential_store(store.base_dir(), Arc::new(LockedVault));
        let mut profile = AccountProfile::new("locked");
        profile.soloist = Some(soloist("must-not-be-written"));
        assert!(locked.save_profile(&profile).is_err());
        assert!(!locked.account_path("locked").exists());
        std::fs::remove_dir_all(store.base_dir()).unwrap();
    }
}
