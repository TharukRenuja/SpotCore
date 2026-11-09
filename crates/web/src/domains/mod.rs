//! Domain methods ported from Myspoti's Spotify API mappings (GPL-3.0-only).
//! Common media lists are normalized; richer domain responses retain Spotify's JSON fields.
mod browsing;
mod content;
mod details;
mod library;
mod social;
use crate::Api;
pub use browsing::*;
pub use content::*;
pub use details::*;
pub use library::*;
use serde_json::Value;
pub use social::*;
use spotcore_core::Error;

pub(crate) fn uri(value: &str) -> Result<(), Error> {
    if !value.starts_with("spotify:")
        || value.len() > 4096
        || value.split(':').skip(1).any(str::is_empty)
    {
        return Err(Error::InvalidInput("expected a Spotify URI"));
    }
    Ok(())
}
pub(crate) fn ids(uris: &[String]) -> Result<Vec<String>, Error> {
    uris.iter()
        .map(|value| {
            uri(value)?;
            Ok(value.rsplit(':').next().unwrap_or_default().to_owned())
        })
        .collect()
}
pub(crate) fn nonempty(value: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        Err(Error::InvalidInput("value must not be empty"))
    } else {
        Ok(())
    }
}
pub(crate) async fn service(
    api: &Api,
    name: &str,
    method: &str,
    args: Vec<Value>,
) -> Result<Value, Error> {
    api.service(name, method, args).await
}
pub(crate) fn field(value: &Value, pointer: &str) -> Result<Value, Error> {
    value
        .pointer(pointer)
        .cloned()
        .ok_or_else(|| Error::Backend(format!("missing API field {pointer}")))
}
pub(crate) fn segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}
