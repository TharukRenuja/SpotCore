//! JSON adapters call the public SDK methods, preserving their validation and routing.
use crate::BindingError;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

mod blends;
mod catalog;
mod devices;
mod extras;
mod home;
mod jam;
mod library;
mod playback;
mod playlists;
mod podcasts;
mod recents;
mod search;
mod users;

pub const INVENTORY: &str = include_str!("../../../../bindings/operations.json");

fn unknown() -> BindingError {
    BindingError::new("invalid_input", "Unknown SDK operation")
}
fn parse<T: DeserializeOwned>(arguments: Value) -> Result<T, BindingError> {
    serde_json::from_value(arguments)
        .map_err(|_| BindingError::new("invalid_input", "Invalid operation arguments"))
}
fn empty(arguments: Value) -> Result<(), BindingError> {
    if arguments
        .as_object()
        .is_some_and(|object| object.is_empty())
    {
        Ok(())
    } else {
        Err(BindingError::new(
            "invalid_input",
            "Expected an empty argument object",
        ))
    }
}
fn encode(value: impl Serialize) -> Result<Value, BindingError> {
    serde_json::to_value(value)
        .map_err(|_| BindingError::new("internal", "Cannot encode SDK response"))
}

pub async fn call(
    sdk: &mut spotcore::SpotCore,
    operation: &str,
    arguments: Value,
) -> Result<Value, BindingError> {
    let (domain, method) = operation.split_once('.').ok_or_else(unknown)?;
    match domain {
        "blends" => blends::call(sdk, method, arguments).await,
        "catalog" => catalog::call(sdk, method, arguments).await,
        "devices" => devices::call(sdk, method, arguments).await,
        "extras" => extras::call(sdk, method, arguments).await,
        "home" => home::call(sdk, method, arguments).await,
        "jam" => jam::call(sdk, method, arguments).await,
        "library" => library::call(sdk, method, arguments).await,
        "playback" => playback::call(sdk, method, arguments).await,
        "playlists" => playlists::call(sdk, method, arguments).await,
        "podcasts" => podcasts::call(sdk, method, arguments).await,
        "recents" => recents::call(sdk, method, arguments).await,
        "search" => search::call(sdk, method, arguments).await,
        "users" => users::call(sdk, method, arguments).await,
        _ => Err(unknown()),
    }
}

pub fn validate(operation: &str, arguments: &Value) -> Result<(), BindingError> {
    if !arguments.is_object() {
        return Err(BindingError::new(
            "invalid_input",
            "Arguments must be an object",
        ));
    }
    let inventory: Value = serde_json::from_str(INVENTORY).expect("generated operation inventory");
    if !inventory
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["operation"] == operation)
    {
        return Err(unknown());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arguments_and_results_keep_sdk_shapes() {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            values: Vec<String>,
        }
        let input = serde_json::json!({"values":["spotify:track:test"]});
        assert_eq!(parse::<Args>(input).unwrap().values, ["spotify:track:test"]);
        assert!(parse::<Args>(serde_json::json!({"values":1})).is_err());
        assert!(parse::<Args>(serde_json::json!({"values":[],"unknown":true})).is_err());
        assert!(empty(serde_json::json!({"unknown":true})).is_err());
        assert_eq!(
            encode(serde_json::json!({"raw_snake_key":true})).unwrap()["raw_snake_key"],
            true
        );
        let quality: spotcore::AudioQuality = parse(serde_json::json!("VeryHigh")).unwrap();
        assert_eq!(encode(quality).unwrap(), "VeryHigh");
    }
}
