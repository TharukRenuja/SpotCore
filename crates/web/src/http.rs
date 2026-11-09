use crate::{Api, cdp::body};
use serde_json::{Value, json};
use spotcore_auth::WebSession;
use spotcore_core::Error;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

pub struct RawResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}
use tokio::sync::Mutex;
use url::Url;

struct ClientToken {
    value: String,
    expires: Instant,
    client_id: String,
    app_version: String,
}
pub(crate) struct Http {
    client: reqwest::Client,
    token: Mutex<Option<ClientToken>>,
    device_id: String,
    cooldowns: Mutex<BTreeMap<String, Instant>>,
}

fn account_url(source: &str) -> Result<Url, Error> {
    let url = Url::parse(source).map_err(|_| Error::InvalidInput("invalid Spotify URL"))?;
    let allowed = matches!(
        url.host_str(),
        Some(
            "image-upload.spotify.com"
                | "spclient.wg.spotify.com"
                | "guc-spclient.spotify.com"
                | "gew1-spclient.spotify.com"
                | "api-partner.spotify.com"
                | "api.spotify.com"
        )
    );
    if url.scheme() != "https"
        || !allowed
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::InvalidInput(
            "authenticated HTTP requires an allowed Spotify HTTPS host",
        ));
    }
    Ok(url)
}

impl Http {
    pub(crate) fn new() -> Result<Self, Error> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| Error::Backend("HTTP client initialization failed".into()))?;
        Ok(Self {
            client,
            token: Mutex::new(None),
            device_id: uuid::Uuid::new_v4().to_string(),
            cooldowns: Mutex::new(BTreeMap::new()),
        })
    }
    pub(crate) fn device_id(&self) -> &str {
        &self.device_id
    }
    async fn client_token(&self, session: &WebSession) -> Result<String, Error> {
        let mut cached = self.token.lock().await;
        if let Some(token) = cached.as_ref().filter(|t| {
            t.expires > Instant::now()
                && t.client_id == session.client_id
                && t.app_version == session.app_version
        }) {
            return Ok(token.value.clone());
        }
        let payload = json!({"client_data":{"client_version":session.app_version,"client_id":session.client_id,"js_sdk_data":{
            "device_brand":"unknown","device_model":"unknown","os":std::env::consts::OS,"os_version":"unknown","device_id":self.device_id,"device_type":"computer"}}});
        let response = self
            .client
            .post("https://clienttoken.spotify.com/v1/clienttoken")
            .header("accept", "application/json")
            .header("content-type", "application/json")
            .header("origin", "https://open.spotify.com")
            .header("referer", "https://open.spotify.com/")
            .header(
                "user-agent",
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
            )
            .json(&payload)
            .send()
            .await
            .map_err(|_| Error::Backend("Spotify client token request failed".into()))?;
        check_status(&response)?;
        let value: Value = serde_json::from_slice(&body(response, 1024 * 1024).await?)
            .map_err(|_| Error::Backend("invalid client token response".into()))?;
        let granted = &value["granted_token"];
        let token = granted["token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Backend("Spotify did not grant a client token".into()))?
            .to_owned();
        let seconds = granted["expires_after_seconds"]
            .as_u64()
            .unwrap_or(3600)
            .saturating_sub(60)
            .min(14 * 86400);
        *cached = Some(ClientToken {
            value: token.clone(),
            expires: Instant::now() + Duration::from_secs(seconds),
            client_id: session.client_id.clone(),
            app_version: session.app_version.clone(),
        });
        Ok(token)
    }
    pub(crate) async fn request(
        &self,
        api: &Api,
        method: &str,
        source: &str,
        payload: Option<Value>,
    ) -> Result<Value, Error> {
        let bytes = payload
            .map(|value| serde_json::to_vec(&value))
            .transpose()
            .map_err(|_| Error::InvalidInput("invalid JSON body"))?;
        let response = self
            .raw(api, method, source, bytes, "application/json")
            .await?;
        if response.body.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&response.body)
            .map_err(|_| Error::Backend("Spotify response is not JSON".into()))
    }
    pub(crate) async fn graphql(&self, api: &Api, payload: Value) -> Result<Value, Error> {
        let bytes = serde_json::to_vec(&payload)
            .map_err(|_| Error::InvalidInput("invalid GraphQL body"))?;
        let response = self
            .raw_inner(
                api,
                "POST",
                "https://api-partner.spotify.com/pathfinder/v2/query",
                Some(bytes),
                "application/json",
                true,
            )
            .await?;
        graphql_result(response)
    }
    pub(crate) async fn raw(
        &self,
        api: &Api,
        method: &str,
        source: &str,
        payload: Option<Vec<u8>>,
        content_type: &str,
    ) -> Result<RawResponse, Error> {
        self.raw_inner(api, method, source, payload, content_type, false)
            .await
    }
    async fn raw_inner(
        &self,
        api: &Api,
        method: &str,
        source: &str,
        payload: Option<Vec<u8>>,
        content_type: &str,
        graphql_rejection: bool,
    ) -> Result<RawResponse, Error> {
        let url = account_url(source)?;
        let method = match method {
            "GET" => reqwest::Method::GET,
            "POST" => reqwest::Method::POST,
            "PUT" => reqwest::Method::PUT,
            "PATCH" => reqwest::Method::PATCH,
            "DELETE" => reqwest::Method::DELETE,
            _ => return Err(Error::InvalidInput("unsupported HTTP method")),
        };
        let host = url.host_str().unwrap_or_default().to_owned();
        if let Some(error) = cooldown_error(&*self.cooldowns.lock().await, &host, Instant::now()) {
            return Err(error);
        }
        let epoch = api.session_epoch();
        let session = api.session().await?;
        let token = self.client_token(&session).await?;
        if api.session_epoch() != epoch {
            return Err(Error::Disconnected);
        }
        let mut request = self
            .client
            .request(method, url)
            .bearer_auth(session.access_token.expose())
            .header("client-token", token)
            .header("app-platform", "WebPlayer")
            .header("spotify-app-version", session.app_version)
            .header("origin", "https://open.spotify.com")
            .header("referer", "https://open.spotify.com/")
            .header("accept", "application/json");
        if let Some(payload) = payload {
            request = request.header("content-type", content_type).body(payload);
        }
        let response = request.send().await.map_err(|_| {
            Error::Backend("Spotify HTTP request failed; write outcome may be unknown".into())
        })?;
        if api.session_epoch() != epoch {
            return Err(Error::Disconnected);
        }
        if response.status().as_u16() == 429
            && let Some(wait) = retry_after(&response)
        {
            remember_cooldown(
                &mut *self.cooldowns.lock().await,
                host,
                Instant::now(),
                wait,
            );
        }
        if response.status().as_u16() == 401 {
            api.invalidate_session().await;
            *self.token.lock().await = None;
        }
        if !(graphql_rejection && response.status().as_u16() == 412) {
            check_status(&response)?;
        }
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.to_string(), value.to_owned()))
            })
            .collect();
        let bytes = body(response, 32 * 1024 * 1024).await?;
        if api.session_epoch() != epoch {
            return Err(Error::Disconnected);
        }
        Ok(RawResponse {
            status,
            headers,
            body: bytes,
        })
    }
}
fn graphql_result(response: RawResponse) -> Result<Value, Error> {
    let value: Value = serde_json::from_slice(&response.body)
        .map_err(|_| Error::Backend("Spotify GraphQL response is not JSON".into()))?;
    if response.status == 412 && value.as_str() == Some("Invalid query hash") {
        return Ok(json!({"errors": [{"message": "PersistedQueryNotFound"}]}));
    }
    if response.status == 412 && !crate::api::is_persisted_query_error(&value) {
        return Err(Error::Http {
            status: 412,
            retry_after: None,
        });
    }
    Ok(value)
}
fn cooldown_error(
    cooldowns: &BTreeMap<String, Instant>,
    host: &str,
    now: Instant,
) -> Option<Error> {
    let remaining = cooldowns.get(host)?.checked_duration_since(now)?;
    if remaining.is_zero() {
        return None;
    }
    Some(Error::Http {
        status: 429,
        retry_after: Some(
            remaining
                .as_secs()
                .saturating_add(u64::from(remaining.subsec_nanos() > 0)),
        ),
    })
}

fn remember_cooldown(
    cooldowns: &mut BTreeMap<String, Instant>,
    host: String,
    now: Instant,
    seconds: u64,
) {
    if let Some(until) = now.checked_add(Duration::from_secs(seconds)) {
        cooldowns
            .entry(host)
            .and_modify(|current| *current = (*current).max(until))
            .or_insert(until);
    }
}

fn retry_after(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get("retry-after")?
        .to_str()
        .ok()?
        .parse()
        .ok()
}

fn check_status(response: &reqwest::Response) -> Result<(), Error> {
    if !response.status().is_success() {
        return Err(Error::Http {
            status: response.status().as_u16(),
            retry_after: retry_after(response),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cooldowns_are_host_scoped_monotonic_and_return_remaining_wait() {
        let now = Instant::now();
        let mut cooldowns = BTreeMap::new();
        remember_cooldown(&mut cooldowns, "api.spotify.com".into(), now, 30);
        assert_eq!(
            cooldown_error(
                &cooldowns,
                "api.spotify.com",
                now + Duration::from_millis(500)
            ),
            Some(Error::Http {
                status: 429,
                retry_after: Some(30)
            })
        );
        assert_eq!(
            cooldown_error(&cooldowns, "api-partner.spotify.com", now),
            None
        );
        remember_cooldown(&mut cooldowns, "api.spotify.com".into(), now, 1);
        assert!(
            cooldown_error(&cooldowns, "api.spotify.com", now + Duration::from_secs(2)).is_some()
        );
        assert_eq!(
            cooldown_error(&cooldowns, "api.spotify.com", now + Duration::from_secs(30)),
            None
        );
    }

    #[test]
    fn only_explicit_unexecuted_persisted_rejections_are_recoverable() {
        let response = |value: Value| RawResponse {
            status: 412,
            headers: BTreeMap::new(),
            body: serde_json::to_vec(&value).unwrap(),
        };
        assert!(crate::api::is_persisted_query_error(
            &graphql_result(response(json!("Invalid query hash"))).unwrap()
        ));
        assert!(
            graphql_result(response(
                json!({"errors":[{"message":"PersistedQueryNotFound"}]})
            ))
            .is_ok()
        );
        assert!(matches!(
            graphql_result(response(
                json!({"errors":[{"message":"precondition failed"}]})
            )),
            Err(Error::Http { status: 412, .. })
        ));
        assert!(matches!(
            graphql_result(response(
                json!({"data":{"changed":true},"errors":[{"message":"PersistedQueryNotFound"}]})
            )),
            Err(Error::Http { status: 412, .. })
        ));
    }
    #[test]
    fn credentials_cannot_be_sent_to_arbitrary_urls() {
        assert!(account_url("https://spclient.wg.spotify.com/path").is_ok());
        for url in [
            "https://spclient.wg.spotify.com.evil/path",
            "http://api.spotify.com/",
            "https://api.spotify.com:444/",
            "https://api.spotify.com@evil/",
            "https://open.spotify.com/",
        ] {
            assert!(account_url(url).is_err());
        }
    }
}
