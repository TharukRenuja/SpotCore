use crate::{assets::PlayerAssets, cdp::body};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha1::Sha1;
use spotcore_auth::WebSession;
use spotcore_core::Error;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Recipe {
    key: Vec<u8>,
    version: u32,
}
fn literal(source: &str) -> Option<(String, &str)> {
    let source = source.trim_start();
    let quote = source.chars().next()?;
    if !matches!(quote, '\'' | '"') {
        return None;
    }
    let mut chars = source.char_indices().skip(1);
    let mut output = String::new();
    while let Some((index, ch)) = chars.next() {
        if ch == quote {
            return Some((output, &source[index + ch.len_utf8()..]));
        }
        if ch == '\\' {
            let (_, escaped) = chars.next()?;
            output.push(match escaped {
                '\\' => '\\',
                '\'' => '\'',
                '"' => '"',
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                _ => return None,
            });
        } else if ch.is_ascii() && !ch.is_control() {
            output.push(ch);
        } else {
            return None;
        }
        if output.len() > 256 {
            return None;
        }
    }
    None
}
impl Recipe {
    fn parse(source: &str) -> Result<Self, Error> {
        // shortcut: supports the current string/XOR recipe; fail closed on a changed encoding.
        if !source.contains("%33+9") {
            return Err(Error::Backend(
                "unsupported player token recipe encoding".into(),
            ));
        }
        let mut recipes = Vec::new();
        for (_, tail) in source
            .match_indices("secret:")
            .map(|(i, _)| (i, &source[i + 7..]))
        {
            let Some((secret, tail)) = literal(tail) else {
                continue;
            };
            let Some(tail) = tail
                .trim_start()
                .strip_prefix(',')
                .and_then(|s| s.trim_start().strip_prefix("version:"))
            else {
                continue;
            };
            let number: String = tail
                .trim_start()
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            let Ok(version) = number.parse::<u32>() else {
                continue;
            };
            if secret.is_empty() || version == 0 {
                continue;
            }
            let transformed = secret
                .bytes()
                .enumerate()
                .map(|(i, c)| (c ^ ((i % 33 + 9) as u8)).to_string())
                .collect::<String>();
            recipes.push(Self {
                key: transformed.into_bytes(),
                version,
            });
        }
        recipes
            .into_iter()
            .max_by_key(|recipe| recipe.version)
            .ok_or_else(|| Error::Backend("player token recipe not found".into()))
    }
    fn token(&self, seconds: u64) -> Result<String, Error> {
        let mut hmac = Hmac::<Sha1>::new_from_slice(&self.key)
            .map_err(|_| Error::Backend("invalid token recipe".into()))?;
        hmac.update(&(seconds / 30).to_be_bytes());
        let digest = hmac.finalize().into_bytes();
        let offset = (digest[19] & 15) as usize;
        let value = u32::from_be_bytes(
            digest[offset..offset + 4]
                .try_into()
                .map_err(|_| Error::Backend("invalid token digest".into()))?,
        ) & 0x7fffffff;
        Ok(format!("{:06}", value % 1_000_000))
    }
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Bootstrap {
    recipe: Recipe,
    app_version: String,
    build_version: String,
    checked_at_ms: u64,
}
impl Bootstrap {
    fn valid(&self, now: u64) -> bool {
        now.checked_sub(self.checked_at_ms)
            .is_some_and(|age| age < 24 * 60 * 60 * 1000)
            && self.recipe.version > 0
            && !self.recipe.key.is_empty()
            && self.recipe.key.len() <= 1024
            && self.recipe.key.iter().all(u8::is_ascii_digit)
            && !self.app_version.is_empty()
            && self.app_version.len() <= 128
            && !self.build_version.is_empty()
            && self.build_version.len() <= 256
    }
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
async fn read_bootstrap(path: std::path::PathBuf) -> Option<Bootstrap> {
    tokio::task::spawn_blocking(move || {
        if std::fs::metadata(&path).ok()?.len() > 8192 {
            return None;
        }
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .ok()?
            .take(8193)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() > 8192 {
            return None;
        }
        let value: Bootstrap = serde_json::from_slice(&bytes).ok()?;
        value.valid(now_ms()).then_some(value)
    })
    .await
    .ok()
    .flatten()
}
async fn write_bootstrap(path: std::path::PathBuf, value: Bootstrap) {
    let _ = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec(&value)?;
        let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let result =
            std::fs::write(&temporary, bytes).and_then(|()| std::fs::rename(&temporary, &path));
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    })
    .await;
}
#[derive(Default)]
struct State {
    session: Option<WebSession>,
    recipe: Option<Bootstrap>,
}
pub(crate) struct CookieSession {
    cookie: String,
    client: reqwest::Client,
    state: Mutex<State>,
    cache: Option<std::path::PathBuf>,
}
impl CookieSession {
    pub(crate) fn new(cookie: String) -> Result<Self, Error> {
        if cookie.is_empty()
            || cookie.len() > 8192
            || cookie
                .bytes()
                .any(|c| !c.is_ascii_graphic() || matches!(c, b';' | b','))
        {
            return Err(Error::InvalidInput("invalid sp_dc cookie"));
        }
        let client=reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(30))
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
            .build().map_err(|_|Error::Backend("cookie client initialization failed".into()))?;
        Ok(Self {
            cookie,
            client,
            state: Mutex::new(State::default()),
            cache: None,
        })
    }
    pub(crate) fn with_credentials(
        credentials: spotcore_auth::WebCredentials,
        cache: std::path::PathBuf,
    ) -> Result<Self, Error> {
        let mut auth = Self::new(credentials.sp_dc.clone().ok_or(Error::NotAuthenticated)?)?;
        let session = match (&credentials.access_token, credentials.expires_at_ms, &credentials.client_id, &credentials.app_version) {
            (Some(token), Some(expires), Some(client), Some(version)) if !client.is_empty() && !version.is_empty() =>
                serde_json::from_value(json!({"accessToken":token,"expiresAtMs":expires,"clientId":client,"appVersion":version,"isAnonymous":false})).ok(),
            _ => None,
        };
        auth.state = Mutex::new(State {
            session,
            recipe: None,
        });
        auth.cache = Some(cache);
        Ok(auth)
    }
    pub(crate) async fn credentials(&self) -> spotcore_auth::WebCredentials {
        let state = self.state.lock().await;
        let session = state.session.as_ref();
        spotcore_auth::WebCredentials {
            sp_dc: Some(self.cookie.clone()),
            access_token: session.map(|s| s.access_token.expose().to_owned()),
            expires_at_ms: session.map(|s| s.expires_at_ms),
            client_id: session.map(|s| s.client_id.clone()),
            app_version: session.map(|s| s.app_version.clone()),
        }
    }
    async fn issue(&self, recipe: &Recipe, app_version: &str) -> Result<WebSession, Error> {
        let response = self
            .client
            .get("https://open.spotify.com/api/server-time")
            .send()
            .await
            .map_err(|_| Error::Backend("server-time request failed".into()))?;
        if !response.status().is_success() {
            return Err(Error::Http {
                status: response.status().as_u16(),
                retry_after: None,
            });
        }
        let value: Value = serde_json::from_slice(&body(response, 1024 * 1024).await?)
            .map_err(|_| Error::Backend("invalid server time".into()))?;
        let server = value["serverTime"]
            .as_u64()
            .ok_or_else(|| Error::Backend("missing server time".into()))?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let response = self
            .client
            .get("https://open.spotify.com/api/token")
            .query(&[
                ("reason", "transport".to_owned()),
                ("productType", "web_player".to_owned()),
                ("totp", recipe.token(now)?),
                ("totpServer", recipe.token(server)?),
                ("totpVer", recipe.version.to_string()),
            ])
            .header("cookie", format!("sp_dc={}", self.cookie))
            .header("referer", "https://open.spotify.com/")
            .send()
            .await
            .map_err(|_| Error::Backend("cookie token request failed".into()))?;
        if matches!(response.status().as_u16(), 401 | 403) {
            return Err(Error::NotAuthenticated);
        }
        if !response.status().is_success() {
            return Err(Error::Http {
                status: response.status().as_u16(),
                retry_after: None,
            });
        }
        let value: Value = serde_json::from_slice(&body(response, 1024 * 1024).await?)
            .map_err(|_| Error::Backend("invalid token response".into()))?;
        let session:WebSession=serde_json::from_value(json!({"accessToken":value["accessToken"],"expiresAtMs":value["accessTokenExpirationTimestampMs"],"clientId":value["clientId"],"appVersion":app_version,"isAnonymous":value["isAnonymous"]}))
            .map_err(|_|Error::NotAuthenticated)?;
        if !session.is_authenticated() || session.client_id.is_empty() {
            return Err(Error::NotAuthenticated);
        }
        Ok(session)
    }
    pub(crate) async fn invalidate(&self, rejected: &str) {
        let mut state = self.state.lock().await;
        // A late rejection must not discard a token another shared request already renewed.
        if state
            .session
            .as_ref()
            .is_some_and(|session| session.access_token.expose() == rejected)
        {
            state.session = None;
        }
    }
    pub(crate) async fn session(&self) -> Result<WebSession, Error> {
        let mut state = self.state.lock().await;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        if let Some(session) = state
            .session
            .as_ref()
            .filter(|s| s.is_authenticated() && u128::from(s.expires_at_ms) > now + 60_000)
        {
            return Ok(session.clone());
        }
        if state
            .recipe
            .as_ref()
            .is_none_or(|value| !value.valid(now_ms()))
        {
            state.recipe = match &self.cache {
                Some(path) => read_bootstrap(path.clone()).await,
                None => None,
            };
        }
        for attempt in 0..2 {
            if state.recipe.is_none() {
                let assets = PlayerAssets::load().await?;
                let value = Bootstrap {
                    recipe: Recipe::parse(&assets.main_script().await?)?,
                    app_version: assets.client_version,
                    build_version: assets.build_version,
                    checked_at_ms: now_ms(),
                };
                state.recipe = Some(value);
            }
            let bootstrap = state
                .recipe
                .as_ref()
                .ok_or_else(|| Error::Backend("missing token recipe".into()))?;
            match self.issue(&bootstrap.recipe, &bootstrap.app_version).await {
                Ok(session) => {
                    state.session = Some(session.clone());
                    if let (Some(path), Some(bootstrap)) = (&self.cache, state.recipe.as_ref()) {
                        write_bootstrap(path.clone(), bootstrap.clone()).await;
                    }
                    return Ok(session);
                }
                Err(Error::Http { status: 400, .. }) if attempt == 0 => {
                    state.recipe = None;
                }
                Err(error) => return Err(error),
            }
        }
        Err(Error::NotAuthenticated)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn saved_credentials() -> spotcore_auth::WebCredentials {
        spotcore_auth::WebCredentials {
            sp_dc: Some("fixture-cookie".into()),
            access_token: Some("fixture-token".into()),
            expires_at_ms: Some(now_ms() + 3_600_000),
            client_id: Some("fixture-client".into()),
            app_version: Some("fixture-version".into()),
        }
    }

    #[tokio::test]
    async fn valid_saved_token_needs_no_bootstrap_or_network() {
        let auth = CookieSession::with_credentials(
            saved_credentials(),
            std::path::PathBuf::from("unused-bootstrap.json"),
        )
        .unwrap();
        let session = auth.session().await.unwrap();
        assert_eq!(session.access_token.expose(), "fixture-token");
        let credentials = auth.credentials().await;
        assert_eq!(credentials.client_id.as_deref(), Some("fixture-client"));
        auth.invalidate("older-token").await;
        assert!(auth.credentials().await.access_token.is_some());
        auth.invalidate("fixture-token").await;
        assert!(auth.credentials().await.access_token.is_none());
    }

    #[tokio::test]
    async fn bootstrap_cache_rejects_stale_corrupt_and_future_records() {
        let directory =
            std::env::temp_dir().join(format!("spotcore-bootstrap-{}", uuid::Uuid::new_v4()));
        let path = directory.join("bootstrap.json");
        let mut value = Bootstrap {
            recipe: Recipe {
                key: b"1234".to_vec(),
                version: 1,
            },
            app_version: "version".into(),
            build_version: "build".into(),
            checked_at_ms: now_ms(),
        };
        write_bootstrap(path.clone(), value.clone()).await;
        assert!(read_bootstrap(path.clone()).await.is_some());
        value.checked_at_ms = now_ms() - 86_400_001;
        write_bootstrap(path.clone(), value.clone()).await;
        assert!(read_bootstrap(path.clone()).await.is_none());
        value.checked_at_ms = now_ms() + 60_000;
        write_bootstrap(path.clone(), value).await;
        assert!(read_bootstrap(path.clone()).await.is_none());
        std::fs::write(&path, b"broken").unwrap();
        assert!(read_bootstrap(path).await.is_none());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    #[ignore = "requires a private Free-account cookie file"]
    async fn free_cookie_acquires_and_renews_without_browser() {
        let file =
            std::env::var("SPOTCORE_FREE_COOKIE_FILE").expect("private cookie file required");
        let cookie = std::fs::read_to_string(file).expect("read private cookie file");
        let auth = CookieSession::new(cookie.trim().to_owned()).unwrap();
        let first = auth.session().await.unwrap();
        let paths = spotcore_core::Paths::new(
            std::env::temp_dir().join(format!("spotcore-cookie-test-{}", uuid::Uuid::new_v4())),
        );
        let api = crate::Api::from_session(first.clone(), &paths).unwrap();
        assert_eq!(
            api.users().me().await.unwrap().tier,
            spotcore_auth::AccountTier::Free
        );
        assert!(api.browser().is_err());
        auth.state
            .lock()
            .await
            .session
            .as_mut()
            .unwrap()
            .expires_at_ms = 0;
        let renewed = auth.session().await.unwrap();
        assert!(renewed.is_authenticated());
        assert_eq!(first.client_id, renewed.client_id);
        let api = crate::Api::from_session(renewed, &paths).unwrap();
        assert_eq!(
            api.users().me().await.unwrap().tier,
            spotcore_auth::AccountTier::Free
        );
        println!("Free cookie acquisition and forced-expiry renewal passed without a browser");
    }
    #[test]
    fn recipe_literals_and_time_counter_are_correct() {
        let recipe = Recipe {
            key: b"12345678901234567890".to_vec(),
            version: 1,
        };
        assert_eq!(recipe.token(59).unwrap(), "287082");
        assert_eq!(recipe.token(1_111_111_109).unwrap(), "081804");
        assert_eq!(literal(r#"'a\\b\'c',version:1"#).unwrap().0, "a\\b'c");
        assert!(Recipe::parse("secret:'abc',version:1").is_err());
        assert_eq!(
            Recipe::parse("%33+9;secret:'abc',version:1;secret:'def',version:2")
                .unwrap()
                .version,
            2
        );
        for cookie in ["", "a;b", "a\r\nb", "a b"] {
            assert!(CookieSession::new(cookie.into()).is_err());
        }
    }
}
