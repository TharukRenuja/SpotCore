use crate::{AccountMetadata, AccountProfile, AccountStore, Error, Paths, SpotCore};
use std::fs::{self, File, OpenOptions};
use std::path::Path;

/// Owns one selected session. Each account has a separate browser and native data directory.
pub struct AccountManager {
    paths: Paths,
    store: AccountStore,
    active: Option<(String, SpotCore, File)>,
    web_options: crate::WebPlayerOptions,
    role: crate::BrowserRole,
    engine: crate::BrowserEngine,
    device_name: Option<String>,
    prefer_native: bool,
}

impl AccountManager {
    pub fn new(paths: Paths) -> Result<Self, Error> {
        let paths = paths.resolve()?;
        let store = AccountStore::new(paths.accounts());
        Self::with_store(paths, store)
    }

    /// Supplies application-managed credentials, including an external vault implementation.
    pub fn with_store(paths: Paths, store: AccountStore) -> Result<Self, Error> {
        Ok(Self {
            paths: paths.resolve()?,
            store,
            active: None,
            web_options: crate::WebPlayerOptions::default(),
            role: crate::BrowserRole::Playback,
            engine: crate::BrowserEngine::Auto,
            device_name: None,
            prefer_native: true,
        })
    }

    /// Native playback is preferred by default when credentials and Premium eligibility permit it.
    /// Select Web explicitly without deleting the saved native credentials.
    pub fn prefer_native(&mut self, enabled: bool) {
        self.prefer_native = enabled;
    }

    pub fn engine(mut self, engine: crate::BrowserEngine) -> Self {
        self.engine = engine;
        self
    }

    pub fn browser_role(mut self, role: crate::BrowserRole) -> Self {
        self.role = role;
        self
    }

    pub fn web_options(mut self, options: crate::WebPlayerOptions) -> Self {
        self.web_options = options;
        self
    }

    /// Sets the Spotify Connect name for both Web and native playback.
    pub fn device_name(mut self, name: impl Into<String>) -> Self {
        self.device_name = Some(name.into());
        self
    }

    /// Lists readable metadata without opening the credential store.
    pub async fn list(&self) -> Result<Vec<AccountMetadata>, Error> {
        let store = self.store.clone();
        blocking(move || {
            store
                .list_accounts()?
                .into_iter()
                .map(|id| {
                    store
                        .load_metadata(&id)?
                        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
                })
                .collect()
        })
        .await
    }

    /// Saves an account; its identity is checked against Spotify when selected.
    pub async fn add(&self, profile: AccountProfile) -> Result<(), Error> {
        let store = self.store.clone();
        blocking(move || store.save_profile(&profile)).await
    }

    pub fn active_account(&self) -> Option<&str> {
        self.active.as_ref().map(|(id, _, _)| id.as_str())
    }

    pub fn session(&self) -> Option<&SpotCore> {
        self.active.as_ref().map(|(_, session, _)| session)
    }

    pub fn session_mut(&mut self) -> Option<&mut SpotCore> {
        self.active.as_mut().map(|(_, session, _)| session)
    }

    pub async fn close(&mut self) -> Result<(), Error> {
        if let Some((_, session, _lease)) = &mut self.active {
            session.shutdown_owned().await?;
        }
        self.active = None;
        Ok(())
    }

    /// Selects a saved account without starting playback.
    /// A pending native login may activate an account-scoped receiver while paused.
    pub async fn switch_to(&mut self, id: &str) -> Result<(), Error> {
        let store = self.store.clone();
        let owned_id = id.to_owned();
        let profile = blocking(move || store.load_profile(&owned_id))
            .await?
            .ok_or(Error::NotAuthenticated)?;
        self.open(profile, true, false).await
    }

    /// Uses credentials supplied by the app without persisting them in AccountStore.
    /// The browser still maintains its own cookies in this account's profile directory.
    pub async fn switch_with(&mut self, profile: AccountProfile) -> Result<(), Error> {
        self.open(profile, false, false).await
    }

    /// Opens interactive login in this account's isolated profile, then verifies the account.
    /// Pass the canonical Spotify username, rather than a display name.
    pub async fn login(&mut self, id: &str) -> Result<(), Error> {
        let store = self.store.clone();
        let owned_id = id.to_owned();
        let profile = blocking(move || login_profile(&store, &owned_id)).await?;
        self.open(profile, true, true).await
    }

    async fn open(
        &mut self,
        mut profile: AccountProfile,
        persist: bool,
        login: bool,
    ) -> Result<(), Error> {
        spotcore_auth::account_key(&profile.canonical_username).map_err(storage_error)?;
        self.close().await?;
        let (paths, lease) = self.lease(&profile.canonical_username)?;
        if login {
            crate::web::HostLauncher::new()
                .data_dir(paths.root())
                .engine(self.engine)
                .profile_dir(paths.profile())
                .login()
                .await?;
            profile.web = None;
        }
        let mut builder = SpotCore::builder()
            .data_dir(paths.root())
            .profile_dir(paths.profile())
            .web_options(self.web_options.clone())
            .browser_role(self.role)
            .engine(self.engine);
        if let Some(name) = &self.device_name {
            builder = builder.device_name(name.clone());
        }
        if !login && let Some(cookie) = profile.web.as_ref().and_then(|web| web.sp_dc.as_ref()) {
            builder = builder.session_cookie(cookie);
        }
        #[cfg(feature = "native")]
        if self.prefer_native
            && let Some(credentials) = profile.soloist.clone()
        {
            builder = builder.with_native_credentials(credentials);
        }
        let (session, verified_account) = if let Some(cookie) = profile
            .web
            .as_ref()
            .and_then(|web| web.sp_dc.as_ref())
            .filter(|_| !login)
        {
            let api = crate::Api::from_cookie(cookie.clone(), &paths)?;
            let account = api.users().me().await?;
            verify_identity(&profile.canonical_username, &account.username)?;
            (
                builder.clone().launch_with_services(api).await?,
                Some(account),
            )
        } else {
            (
                SpotCore::from_web(builder.clone().launch_web().await?),
                None,
            )
        };
        let mut session = session;
        #[cfg(feature = "native")]
        let mut verify_before_native = verified_account.is_none();
        let result = async {
            let account = match verified_account {
                Some(account) => account,
                None => session.users().me().await?,
            };
            verify_identity(&profile.canonical_username, &account.username)?;
            if session.api.browser().is_ok()
                && let Ok((api, credentials)) = session.api.http_services(&paths).await
                && let Ok(http_account) = api.users().me().await
            {
                verify_identity(&profile.canonical_username, &http_account.username)?;
                profile.web = Some(credentials);
                let use_http = self.role == crate::BrowserRole::Services;
                #[cfg(feature = "native")]
                let use_http = use_http || (self.prefer_native && profile.soloist.is_some());
                if use_http {
                    // Retire the old Web receiver before registering Soloist with the same name.
                    session.shutdown_owned().await?;
                    let candidate = builder.clone().launch_http(api).await?;
                    if self.role == crate::BrowserRole::Services
                        || candidate.playback_mode() == crate::PlaybackMode::Native
                    {
                        session = candidate;
                    } else {
                        let fallback = candidate.native_fallback.clone();
                        candidate.shutdown().await?;
                        let web = builder.clone().launch_web().await.map_err(|error| {
                            crate::session::startup_error(fallback.clone(), error)
                        })?;
                        session = SpotCore::from_web(web);
                        session.native_fallback = fallback;
                    }
                    #[cfg(feature = "native")]
                    {
                        verify_before_native = false;
                    }
                }
            }
            profile.display_name = Some(account.name);
            profile.tier = Some(account.tier);
            profile.tier_checked_at_ms = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            );
            if persist {
                self.add(profile.clone()).await?;
            }
            #[cfg(feature = "native")]
            if self.prefer_native
                && verify_before_native
                && let Some(credentials) = profile.soloist.clone()
            {
                session
                    .try_native(credentials, &paths, self.device_name.as_deref())
                    .await;
            }
            if self.role == crate::BrowserRole::Playback
                && session.playback_mode() == crate::PlaybackMode::Web
            {
                session.require_web_playback().await?;
            }
            Ok::<_, Error>(())
        }
        .await;
        if let Err(error) = result {
            session.shutdown().await?;
            return Err(error);
        }
        self.active = Some((profile.canonical_username, session, lease));
        Ok(())
    }

    /// Removes saved credentials and this manager's browser/native state after stopping it.
    pub async fn forget(&mut self, id: &str) -> Result<(), Error> {
        if self.active_account() == Some(id) {
            self.close().await?;
        }
        let (paths, lease) = self.lease(id)?;
        crate::web::HostLauncher::forget_system_profile(self.paths.root(), &paths.profile())
            .await?;
        let store = self.store.clone();
        let id = id.to_owned();
        blocking(move || {
            let _lease = lease;
            fs::remove_dir_all(paths.root())?;
            store.delete_account(&id).map(|_| ())
        })
        .await
    }

    fn lease(&self, id: &str) -> Result<(Paths, File), Error> {
        let key = spotcore_auth::account_key(id).map_err(storage_error)?;
        self.paths.prepare().map_err(storage_error)?;
        let sessions = self.paths.root().join("sessions");
        reject_symlink(&sessions)?;
        Paths::new(&sessions).prepare().map_err(storage_error)?;
        let lock_path = sessions.join(format!("{key}.lock"));
        reject_symlink(&lock_path)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)
            .map_err(storage_error)?;
        lock.try_lock()
            .map_err(|_| Error::Backend("account session is already in use".into()))?;
        let paths = Paths::new(sessions.join(key));
        reject_symlink(paths.root())?;
        paths.prepare().map_err(storage_error)?;
        Ok((paths, lock))
    }
}

impl Drop for AccountManager {
    fn drop(&mut self) {
        if let Some((_, mut session, lease)) = self.active.take()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            runtime.spawn(async move {
                let _lease = lease;
                let _ = session.shutdown_owned().await;
            });
        }
    }
}

fn login_profile(store: &AccountStore, id: &str) -> std::io::Result<AccountProfile> {
    match store.load_profile(id) {
        Ok(Some(profile)) => Ok(profile),
        Ok(None) => Ok(AccountProfile::new(id)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AccountProfile::new(id)),
        Err(error) => Err(error),
    }
}

fn reject_symlink(path: &Path) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            Err(Error::InvalidInput("account session path is a symlink"))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(storage_error(error)),
    }
}

fn verify_identity(expected: &str, actual: &str) -> Result<(), Error> {
    if expected == actual {
        Ok(())
    } else {
        Err(Error::AccountRestricted(
            "credentials belong to a different account",
        ))
    }
}

fn storage_error(error: std::io::Error) -> Error {
    Error::Backend(format!("account storage: {error}"))
}

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> std::io::Result<T> + Send + 'static,
) -> Result<T, Error> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|_| Error::Backend("account storage task failed".into()))?
        .map_err(storage_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MissingVault;
    impl crate::CredentialStore for MissingVault {
        fn load(&self, _: &str) -> std::io::Result<Option<crate::AccountSecrets>> {
            Ok(None)
        }
        fn save(&self, _: &str, _: &crate::AccountSecrets) -> std::io::Result<()> {
            Ok(())
        }
        fn delete(&self, _: &str) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn explicit_web_selection_keeps_native_preferred_for_new_managers() {
        let paths = Paths::new(std::env::temp_dir().join("spotcore-native-preference"));
        let mut manager = AccountManager::new(paths.clone()).unwrap();
        assert!(manager.prefer_native);
        manager.prefer_native(false);
        assert!(!manager.prefer_native);
        assert!(AccountManager::new(paths).unwrap().prefer_native);
    }

    #[test]
    fn missing_vault_entry_can_be_recovered_with_interactive_login() {
        let root =
            std::env::temp_dir().join(format!("spotcore-missing-entry-{}", std::process::id()));
        let store = AccountStore::with_credential_store(&root, std::sync::Arc::new(MissingVault));
        let mut profile = AccountProfile::new("user");
        profile.soloist = Some(crate::SoloistCredentials {
            api_key: Some("dummy".into()),
        });
        store.save_profile(&profile).unwrap();
        assert!(store.load_profile("user").is_err());
        let recovered = login_profile(&store, "user").unwrap();
        assert_eq!(recovered.canonical_username, "user");
        assert!(recovered.soloist.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn accounts_have_distinct_exclusive_sessions_and_forget_only_the_selected_data() {
        let root =
            std::env::temp_dir().join(format!("spotcore-coordinator-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mut manager = AccountManager::new(Paths::new(&root)).unwrap();
        manager.add(AccountProfile::new("one")).await.unwrap();
        manager.add(AccountProfile::new("two")).await.unwrap();
        let (one, lease) = manager.lease("one").unwrap();
        let (two, lease2) = manager.lease("two").unwrap();
        assert_ne!(one.profile(), two.profile());
        assert!(manager.lease("one").is_err());
        assert!(manager.forget("one").await.is_err());
        drop(lease);
        manager.forget("one").await.unwrap();
        assert!(!one.root().exists());
        assert!(two.root().exists());
        assert_eq!(manager.list().await.unwrap().len(), 1);
        assert!(verify_identity("one", "two").is_err());
        assert!(verify_identity("one", "one").is_ok());
        drop(lease2);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_session_directories() {
        let root = std::env::temp_dir().join(format!("spotcore-symlink-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(std::env::temp_dir(), root.join("sessions")).unwrap();
        let manager = AccountManager::new(Paths::new(&root)).unwrap();
        assert!(manager.lease("one").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
