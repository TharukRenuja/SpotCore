//! Account metadata and credentials. Browser refresh remains owned by Spotify's page.
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    time::{SystemTime, UNIX_EPOCH},
};

pub mod account;
mod credentials;
pub use account::*;
pub use credentials::{AccountSecrets, CredentialStore, OsCredentialStore};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AccountTier {
    Free,
    Premium,
    Unknown,
}

/// Secret values are deliberately excluded from Debug output.
#[derive(Clone, Deserialize)]
pub struct AccessToken(String);
impl AccessToken {
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AccessToken([REDACTED])")
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSession {
    pub access_token: AccessToken,
    pub expires_at_ms: u64,
    pub client_id: String,
    pub app_version: String,
    pub is_anonymous: bool,
}
impl WebSession {
    pub fn is_authenticated(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        !self.is_anonymous
            && !self.access_token.expose().trim().is_empty()
            && u128::from(self.expires_at_ms) > now
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn debug_redacts_tokens() {
        let value = WebSession {
            access_token: AccessToken("secret-value".into()),
            expires_at_ms: u64::MAX,
            client_id: "client".into(),
            app_version: "version".into(),
            is_anonymous: false,
        };
        assert!(!format!("{value:?}").contains("secret-value"));
        assert!(value.is_authenticated());
        assert!(
            !WebSession {
                is_anonymous: true,
                ..value.clone()
            }
            .is_authenticated()
        );
        assert!(
            !WebSession {
                expires_at_ms: 0,
                ..value
            }
            .is_authenticated()
        );
    }
}
