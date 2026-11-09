use crate::{assets::PlayerAssets, cdp::body};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha1::Sha1;
use spotcore_auth::WebSession;
use spotcore_core::Error;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

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
#[derive(Default)]
struct State {
    session: Option<WebSession>,
    recipe: Option<(Recipe, String, String)>,
}
pub(crate) struct CookieSession {
    cookie: String,
    client: reqwest::Client,
    state: Mutex<State>,
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
        })
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
    pub(crate) async fn invalidate(&self) {
        self.state.lock().await.session = None;
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
        for attempt in 0..2 {
            let assets = PlayerAssets::load().await?;
            if state
                .recipe
                .as_ref()
                .is_none_or(|(_, _, build)| build != &assets.build_version)
            {
                state.recipe = Some((
                    Recipe::parse(&assets.main_script().await?)?,
                    assets.client_version,
                    assets.build_version,
                ));
            }
            let (recipe, version, _) = state
                .recipe
                .as_ref()
                .ok_or_else(|| Error::Backend("missing token recipe".into()))?;
            match self.issue(recipe, version).await {
                Ok(session) => {
                    state.session = Some(session.clone());
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
