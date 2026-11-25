#[boltffi::error]
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct BindingError {
    pub code: String,
    pub message: String,
}

impl BindingError {
    pub(crate) fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
    pub(crate) fn closed() -> Self {
        Self::new("closed", "The SDK session has shut down")
    }
}
impl std::fmt::Display for BindingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for BindingError {}
impl From<spotcore::Error> for BindingError {
    fn from(error: spotcore::Error) -> Self {
        use spotcore::Error;
        let code = match &error {
            Error::NotAuthenticated => "not_authenticated",
            Error::NativeStartupTimeout(_) => "native_startup_timeout",
            Error::NativeAuthenticationRequired => "native_authentication_required",
            Error::NativeStartupFailed { .. } => "native_startup_failed",
            Error::Unsupported => "unsupported",
            Error::InvalidInput(_) => "invalid_input",
            Error::AccountRestricted(_) => "account_restricted",
            Error::FeatureUnavailable { reason, .. } => match reason {
                spotcore::FeatureRestriction::UnavailableInMarket { .. } => "unavailable_in_market",
                spotcore::FeatureRestriction::UnverifiedMarket { .. } => "unverified_market",
                spotcore::FeatureRestriction::RequiresPremium => "requires_premium",
                spotcore::FeatureRestriction::TitleLocked => "title_locked",
                spotcore::FeatureRestriction::UnsupportedByEngine => "unsupported_by_engine",
                spotcore::FeatureRestriction::AccountDenied => "account_denied",
                _ => "availability_unknown",
            },
            Error::Http { .. } => "http",
            _ => "backend",
        };
        let message = match &error {
            Error::Http { status, .. } => format!("Spotify returned HTTP {status}"),
            _ => error.to_string(),
        };
        Self::new(code, message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_policy_errors_are_not_transport_errors() {
        let error = BindingError::from(spotcore::Error::FeatureUnavailable {
            feature: "audiobooks".into(),
            reason: spotcore::FeatureRestriction::UnverifiedMarket {
                country: "LK".into(),
            },
        });
        assert_eq!(error.code, "unverified_market");
        assert!(error.message.contains("LK"));
        assert_eq!(
            BindingError::from(spotcore::Error::Http {
                status: 502,
                retry_after: None
            })
            .code,
            "http"
        );
    }
    #[test]
    fn native_authentication_and_fallback_have_distinct_binding_codes() {
        assert_eq!(
            BindingError::from(spotcore::Error::NativeAuthenticationRequired).code,
            "native_authentication_required"
        );
        assert_eq!(
            BindingError::from(spotcore::Error::NativeStartupTimeout("discovering devices")).code,
            "native_startup_timeout"
        );
        let error = spotcore::Error::NativeStartupFailed {
            native: Box::new(spotcore::Error::NativeAuthenticationRequired),
            fallback: Box::new(spotcore::Error::DrmUnavailable("missing DRM".into())),
        };
        let error = BindingError::from(error);
        assert_eq!(error.code, "native_startup_failed");
        assert!(error.message.contains("Soloist") && error.message.contains("missing DRM"));
    }
}
