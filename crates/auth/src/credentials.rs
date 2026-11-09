use crate::{SoloistCredentials, WebCredentials};
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSecrets {
    pub web: Option<WebCredentials>,
    pub soloist: Option<SoloistCredentials>,
}

/// Implementations must not include secret values in errors. Methods may block on an OS prompt.
pub trait CredentialStore: Send + Sync {
    fn load(&self, reference: &str) -> io::Result<Option<AccountSecrets>>;
    fn save(&self, reference: &str, secrets: &AccountSecrets) -> io::Result<()>;
    fn delete(&self, reference: &str) -> io::Result<()>;
}

#[derive(Debug)]
pub struct OsCredentialStore {
    service: String,
}
impl OsCredentialStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    fn entry(&self, reference: &str) -> io::Result<keyring::Entry> {
        keyring::Entry::new(&self.service, reference).map_err(|_| vault_error())
    }
}
fn vault_error() -> io::Error {
    io::Error::other(
        "OS credential store unavailable or locked; unlock it or supply a CredentialStore",
    )
}
impl CredentialStore for OsCredentialStore {
    fn load(&self, reference: &str) -> io::Result<Option<AccountSecrets>> {
        #[cfg(any(target_os = "linux", target_os = "macos", windows))]
        {
            let value = match self.entry(reference)?.get_secret() {
                Ok(value) => value,
                Err(keyring::Error::NoEntry) => return Ok(None),
                Err(_) => return Err(vault_error()),
            };
            serde_json::from_slice(&value).map(Some).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid credential-store entry")
            })
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        {
            let _ = reference;
            Err(vault_error())
        }
    }
    fn save(&self, reference: &str, secrets: &AccountSecrets) -> io::Result<()> {
        #[cfg(any(target_os = "linux", target_os = "macos", windows))]
        {
            let value = serde_json::to_vec(secrets)
                .map_err(|_| io::Error::other("cannot encode credentials"))?;
            self.entry(reference)?
                .set_secret(&value)
                .map_err(|_| vault_error())
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        {
            let _ = (reference, secrets);
            Err(vault_error())
        }
    }
    fn delete(&self, reference: &str) -> io::Result<()> {
        #[cfg(any(target_os = "linux", target_os = "macos", windows))]
        {
            match self.entry(reference)?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(_) => Err(vault_error()),
            }
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        {
            let _ = reference;
            Err(vault_error())
        }
    }
}
