//! Account-scoped Jam lifecycle. Reads never create a session or launch a player.
use crate::{Api, direct};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use spotcore_core::Error;
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

const SOCIAL: &str = "https://spclient.wg.spotify.com/social-connect";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListenOn {
    Host,
    ThisDevice,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JamSession {
    pub id: String,
    pub is_owner: bool,
    pub host_device: Option<String>,
    pub can_control: bool,
    pub raw: Value,
}

impl JamSession {
    pub fn parse(raw: Value) -> Result<Option<Self>, Error> {
        let raw = normalize(raw);
        if raw.is_null() {
            return Ok(None);
        }
        let id = session_id(&raw)
            .ok_or_else(|| Error::Backend("Jam response has no session identity".into()))?
            .to_owned();
        let is_owner = raw["is_session_owner"] == true;
        let can_control = is_owner || (raw["active"] != false && raw["queue_only_mode"] != true);
        Ok(Some(Self {
            id,
            is_owner,
            host_device: direct::jam_host_device(&raw),
            can_control,
            raw,
        }))
    }
}

#[derive(Default)]
pub(crate) struct State {
    joined: Option<Joined>,
    retired: HashSet<String>,
    cached: Option<(Instant, Value)>,
}

struct Joined {
    id: String,
    token: String,
    username: String,
    listen_here: bool,
    receiver: String,
}

fn member_session(raw: Value, expected: &str, username: &str) -> Option<Value> {
    let mut raw = normalize(raw);
    let username = username.strip_prefix("spotify:user:").unwrap_or(username);
    if username.is_empty() || session_id(&raw) != Some(expected) {
        return None;
    }
    let members = raw["session_members"].as_array()?;
    if !members.iter().any(|member| {
        ["username", "id"].iter().any(|key| {
            member[*key]
                .as_str()
                .is_some_and(|id| id.strip_prefix("spotify:user:").unwrap_or(id) == username)
        })
    }) {
        return None;
    }
    if raw.get("is_session_owner").is_none() {
        let owner = raw["session_owner_id"].as_str()?;
        raw["is_session_owner"] =
            json!(owner.strip_prefix("spotify:user:").unwrap_or(owner) == username);
    }
    Some(raw)
}

impl State {
    fn accept(&self, value: Value) -> Value {
        let value = normalize(value);
        if session_id(&value).is_some_and(|id| self.retired.contains(id)) {
            Value::Null
        } else {
            value
        }
    }
    fn retire(&mut self, id: &str) {
        self.retired.insert(id.to_owned());
        if self.joined.as_ref().is_some_and(|joined| joined.id == id) {
            self.joined = None;
        }
        self.cached = Some((Instant::now(), Value::Null));
    }
}

pub(crate) fn session_id(value: &Value) -> Option<&str> {
    let value = value.get("session").unwrap_or(value);
    value
        .get("session_id")
        .or_else(|| value.get("sessionId"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
}

pub(crate) fn normalize(value: Value) -> Value {
    let mut value = value.get("session").cloned().unwrap_or(value);
    if !value.is_object() {
        return value;
    }
    for (snake, camel) in [
        ("session_id", "sessionId"),
        ("is_session_owner", "isSessionOwner"),
        ("session_members", "sessionMembers"),
        ("session_owner_id", "sessionOwnerId"),
        ("queue_only_mode", "queueOnlyMode"),
        ("host_active_device_id", "hostActiveDeviceId"),
        ("join_session_token", "joinSessionToken"),
        ("join_session_uri", "joinSessionUri"),
        ("is_paused", "isPaused"),
        ("is_jam_paused", "isJamPaused"),
    ] {
        if value.get(snake).is_none()
            && let Some(v) = value.get(camel).cloned()
        {
            value[snake] = v;
        }
    }
    if value.get("host_device_info").is_none()
        && let Some(v) = value.get("hostDeviceInfo").cloned()
    {
        value["host_device_info"] = v;
    }
    value
}

async fn read(api: &Api, state: &mut State) -> Result<Value, Error> {
    let mut value = match api
        .request("GET", &format!("{SOCIAL}/v2/sessions/current"), None)
        .await
    {
        Ok(value) => state.accept(value),
        Err(Error::Http {
            status: 404 | 410, ..
        }) => Value::Null,
        Err(error) => return Err(error),
    };
    if let Some(joined) = &state.joined {
        if value.is_null() && !state.retired.contains(&joined.id) {
            let info = match api
                .request(
                    "GET",
                    &format!(
                        "{SOCIAL}/v2/sessions/info/{}",
                        crate::domains::segment(&joined.token)
                    ),
                    None,
                )
                .await
            {
                Ok(info) => info,
                Err(Error::Http {
                    status: 404 | 410, ..
                }) => Value::Null,
                Err(error) => return Err(error),
            };
            value = member_session(info, &joined.id, &joined.username).unwrap_or(Value::Null);
        }
        if session_id(&value) == Some(joined.id.as_str()) {
            value["listen_on_device"] = json!(joined.listen_here);
            value["join_session_token"] = json!(joined.token);
        }
    }
    state.cached = Some((Instant::now(), value.clone()));
    Ok(value)
}

pub(crate) async fn invalidate(api: &Api) {
    api.jam_state().lock().await.cached = None;
}

pub(crate) async fn current(api: &Api, fresh: bool) -> Result<Value, Error> {
    let mut state = api.jam_state().lock().await;
    if !fresh
        && let Some((at, value)) = &state.cached
        && at.elapsed() < Duration::from_secs(5)
    {
        return Ok(value.clone());
    }
    read(api, &mut state).await
}

pub(crate) async fn finish(api: &Api, expected: Option<&str>, end: bool) -> Result<Value, Error> {
    let mut state = api.jam_state().lock().await;
    // A caller with an identity must not depend on another membership read.
    let current;
    let id = if let Some(id) = expected.filter(|id| !id.is_empty()) {
        id
    } else {
        current = read(api, &mut state).await?;
        let Some(id) = session_id(&current) else {
            return Ok(json!(true));
        };
        id
    };
    let path = format!(
        "{SOCIAL}/v3/sessions/{}{}",
        crate::domains::segment(id),
        if end { "" } else { "/leave" }
    );
    match api
        .request(if end { "DELETE" } else { "POST" }, &path, None)
        .await
    {
        Ok(_)
        | Err(Error::Http {
            status: 404 | 410, ..
        }) => {}
        Err(error) => return Err(error),
    }
    state.retire(id);
    api.set_jam_command_device(None);
    Ok(json!(true))
}

pub(crate) async fn start(api: &Api) -> Result<Value, Error> {
    let mut state = api.jam_state().lock().await;
    let current = read(api, &mut state).await?;
    if !current.is_null() {
        return Ok(current);
    }
    let created = api
        .request(
            "GET",
            &format!("{SOCIAL}/v2/sessions/current_or_new?activate=true"),
            None,
        )
        .await?;
    let created = state.accept(created);
    if session_id(&created).is_none() {
        return Err(Error::Backend(
            "Spotify did not identify the created Jam".into(),
        ));
    }
    state.cached = Some((Instant::now(), created.clone()));
    Ok(created)
}

pub(crate) async fn join(
    api: &Api,
    token: &str,
    receiver: &str,
    listen_here: bool,
) -> Result<Value, Error> {
    if receiver.is_empty() {
        return Err(Error::InvalidInput("missing Jam receiver"));
    }
    let mut state = api.jam_state().lock().await;
    let info = api
        .request(
            "GET",
            &format!(
                "{SOCIAL}/v2/sessions/info/{}",
                crate::domains::segment(token)
            ),
            None,
        )
        .await?;
    let expected = session_id(&info)
        .ok_or_else(|| Error::Backend("Jam invitation has ended or has no identity".into()))?
        .to_owned();
    let current = read(api, &mut state).await?;
    if session_id(&current) == Some(expected.as_str())
        && state
            .joined
            .as_ref()
            .is_some_and(|joined| joined.listen_here == listen_here && joined.receiver == receiver)
    {
        return Ok(current);
    }
    if !current.is_null() && session_id(&current) != Some(expected.as_str()) {
        return Err(Error::InvalidInput(
            "leave the current Jam before joining another",
        ));
    }
    let account = api.users().me().await?;
    if listen_here && account.tier != spotcore_auth::AccountTier::Premium {
        return Err(Error::InvalidInput(
            "listening on this device requires Premium",
        ));
    }
    api.request(
        "POST",
        &direct::jam_join_url(token, Some(receiver), listen_here),
        None,
    )
    .await?;
    // A deliberate join can revisit an existing session; passive reads cannot.
    state.retired.remove(&expected);
    state.joined = Some(Joined {
        id: expected.clone(),
        token: token.to_owned(),
        username: account.username,
        listen_here,
        receiver: receiver.to_owned(),
    });
    state.cached = None;
    api.set_jam_command_device(Some(receiver.to_owned()));
    for attempt in 0..8 {
        let mut current = read(api, &mut state).await?;
        if session_id(&current) == Some(expected.as_str()) {
            current["join_session_token"] = json!(token);
            state.cached = Some((Instant::now(), current.clone()));
            return Ok(current);
        }
        if attempt < 7 {
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }
    Err(Error::Backend(
        "Spotify has not confirmed Jam membership; refresh before retrying".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retired_membership_cannot_reappear_but_a_different_jam_can() {
        let mut state = State::default();
        state.retire("old");
        state.retire("second");
        for id in ["old", "second"] {
            assert!(state.accept(json!({"session":{"sessionId":id}})).is_null());
        }
        assert_eq!(
            session_id(&state.accept(json!({"sessionId":"new"}))),
            Some("new")
        );
        assert_eq!(
            session_id(&state.accept(json!({"session_id":"new","active":false}))),
            Some("new")
        );
    }
    #[test]
    fn invite_membership_is_scoped_to_the_expected_session_and_account() {
        let raw = json!({"session":{"sessionId":"one","sessionOwnerId":"spotify:user:host","sessionMembers":[{"id":"spotify:user:guest"}]}});
        assert!(member_session(raw.clone(), "one", "guest").is_some());
        assert!(member_session(raw.clone(), "other", "guest").is_none());
        assert!(member_session(raw.clone(), "one", "stranger").is_none());
        assert!(member_session(raw, "one", "spotify:user:").is_none());
    }

    #[test]
    fn typed_membership_normalizes_fields_and_rejects_missing_identity() {
        let s = JamSession::parse(json!({"session":{"sessionId":"one","isSessionOwner":false,"queueOnlyMode":true,"hostActiveDeviceId":"host"}})).unwrap().unwrap();
        assert_eq!(s.id, "one");
        assert!(!s.can_control);
        assert_eq!(s.host_device.as_deref(), Some("host"));
        assert!(JamSession::parse(json!({"active":true})).is_err());
    }
}
