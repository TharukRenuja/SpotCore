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
            Error::Unsupported => "unsupported",
            Error::InvalidInput(_) => "invalid_input",
            Error::AccountRestricted(_) => "account_restricted",
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
