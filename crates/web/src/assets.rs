use crate::{Operation, Operations, cdp::body};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{StreamExt, stream};
use serde_json::Value;
use spotcore_core::Error;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
pub(crate) struct PlayerAssets {
    client: reqwest::Client,
    pub(crate) build_version: String,
    pub(crate) client_version: String,
    urls: BTreeSet<String>,
}
fn allowed_asset(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("open.spotifycdn.com")
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none()
            && url.path().starts_with("/cdn/build/web-player/")
            && url.path().ends_with(".js")
            && url.query().is_none()
            && url.fragment().is_none()
    })
}
fn script_json(html: &str, id: &str) -> Result<Value, Error> {
    for part in html.split("<script").skip(1) {
        let Some((tag, rest)) = part.split_once('>') else {
            continue;
        };
        if !tag.contains(&format!("id=\"{id}\"")) && !tag.contains(&format!("id='{id}'")) {
            continue;
        }
        let source = rest
            .split_once("</script>")
            .ok_or_else(|| Error::Backend("unterminated player configuration".into()))?
            .0
            .trim();
        let decoded = STANDARD
            .decode(source)
            .unwrap_or_else(|_| source.as_bytes().to_vec());
        return serde_json::from_slice(&decoded)
            .map_err(|_| Error::Backend("invalid public player configuration".into()));
    }
    Err(Error::Backend(format!(
        "missing public player configuration {id}"
    )))
}
fn collect_urls(value: &Value, urls: &mut BTreeSet<String>) {
    match value {
        Value::String(value) if allowed_asset(value) => {
            urls.insert(value.clone());
        }
        Value::Array(values) => {
            for value in values {
                collect_urls(value, urls);
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                collect_urls(value, urls);
            }
        }
        _ => {}
    }
}
fn operations(source: &str) -> BTreeMap<String, Operation> {
    let mut result = BTreeMap::new();
    let fields: Vec<_> = source.split('"').collect();
    for window in fields.windows(5) {
        let [name, comma, kind, comma2, hash] = window else {
            continue;
        };
        if !name.is_empty()
            && name.len() <= 128
            && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
            && comma.trim() == ","
            && comma2.trim() == ","
            && matches!(*kind, "query" | "mutation")
            && hash.len() == 64
            && hash.bytes().all(|c| c.is_ascii_hexdigit())
        {
            result.insert(
                (*name).to_owned(),
                Operation {
                    kind: (*kind).to_owned(),
                    hash: (*hash).to_owned(),
                },
            );
        }
    }
    result
}
impl PlayerAssets {
    pub(crate) async fn load() -> Result<Self, Error> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .user_agent(UA)
            .build()
            .map_err(|_| Error::Backend("public asset client initialization failed".into()))?;
        let response = client
            .get("https://open.spotify.com/")
            .send()
            .await
            .map_err(|_| Error::Backend("player page request failed".into()))?;
        if !response.status().is_success() {
            return Err(Error::Http {
                status: response.status().as_u16(),
                retry_after: None,
            });
        }
        let html = String::from_utf8(body(response, 8 * 1024 * 1024).await?)
            .map_err(|_| Error::Backend("invalid player page encoding".into()))?;
        let config = script_json(&html, "appServerConfig")?;
        let mut urls = BTreeSet::new();
        collect_urls(&script_json(&html, "__CDN_FILE_URLS__")?, &mut urls);
        // Include bootstrap scripts even when the lazy-asset map omits them.
        for part in html.split("src=\"").skip(1) {
            if let Some((url, _)) = part.split_once('"')
                && allowed_asset(url)
            {
                urls.insert(url.to_owned());
            }
        }
        if urls.is_empty() || urls.len() > 512 {
            return Err(Error::Backend("unexpected player asset count".into()));
        }
        let field = |name: &str| {
            config[name]
                .as_str()
                .filter(|v| !v.trim().is_empty())
                .map(str::to_owned)
                .ok_or_else(|| Error::Backend(format!("missing player {name}")))
        };
        Ok(Self {
            client,
            build_version: field("buildVersion")?,
            client_version: field("clientVersion")?,
            urls,
        })
    }
    async fn fetch(&self, url: &str) -> Result<String, Error> {
        if !allowed_asset(url) {
            return Err(Error::InvalidInput("untrusted player asset URL"));
        }
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|_| Error::Backend("player asset request failed".into()))?;
        if !response.status().is_success() {
            return Err(Error::Http {
                status: response.status().as_u16(),
                retry_after: None,
            });
        }
        String::from_utf8(body(response, 16 * 1024 * 1024).await?)
            .map_err(|_| Error::Backend("invalid player script encoding".into()))
    }
    pub(crate) async fn main_script(&self) -> Result<String, Error> {
        let url = self
            .urls
            .iter()
            .find(|url| {
                url.rsplit('/')
                    .next()
                    .is_some_and(|name| name.starts_with("web-player."))
            })
            .ok_or_else(|| Error::Backend("missing player bootstrap script".into()))?;
        self.fetch(url).await
    }
    pub(crate) async fn discover(&self) -> Result<Operations, Error> {
        let mut pending = stream::iter(self.urls.iter().map(|url| async move {
            let source = self.fetch(url).await?;
            let size = source.len();
            Ok::<_, Error>((size, operations(&source)))
        }))
        .buffered(4);
        let mut total = 0usize;
        let mut found: BTreeMap<String, Vec<Operation>> = BTreeMap::new();
        let mut hashes: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        while let Some(result) = pending.next().await {
            let (size, ops) = result?;
            total += size;
            if total > 256 * 1024 * 1024 {
                return Err(Error::Backend(
                    "player asset discovery size limit exceeded".into(),
                ));
            }
            for (name, op) in ops {
                hashes
                    .entry(op.hash.clone())
                    .or_default()
                    .insert(name.clone());
                found.entry(name).or_default().push(op);
            }
        }
        // Shared placeholder hashes are not usable persisted operations.
        let found = found
            .into_iter()
            .filter_map(|(name, candidates)| {
                candidates
                    .into_iter()
                    .rev()
                    .find(|op| hashes[&op.hash].len() == 1)
                    .map(|op| (name, op))
            })
            .collect();
        let result = Operations {
            build_version: self.build_version.clone(),
            operations: found,
        };
        Operations::from_json(
            &serde_json::to_string(&result)
                .map_err(|_| Error::Backend("operation serialization failed".into()))?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "live public Spotify CDN; no credentials"]
    async fn current_public_operations_can_be_discovered() {
        let assets = PlayerAssets::load().await.unwrap();
        let result = assets.discover().await.unwrap();
        assert!(result.operations.len() > 100);
        assert!(result.operations.contains_key("profileAttributes"));
        assert!(result.operations.contains_key("areEntitiesInLibrary"));
        println!(
            "Discovered {} operations from public assets",
            result.operations.len()
        );
    }
    #[test]
    fn public_manifest_and_definitions_are_parsed_without_execution() {
        let payload = STANDARD.encode(
            br#"{"main":"https://open.spotifycdn.com/cdn/build/web-player/web-player.test.js"}"#,
        );
        let html = format!("<script type='text/plain' id='__CDN_FILE_URLS__'>{payload}</script>");
        let mut urls = BTreeSet::new();
        collect_urls(&script_json(&html, "__CDN_FILE_URLS__").unwrap(), &mut urls);
        assert_eq!(urls.len(), 1);
        let hash = "a".repeat(64);
        let ops = operations(&format!(
            "new x(\"testQuery\", \"query\", \"{hash}\");new x(\"testWrite\",\"mutation\",\"{hash}\")"
        ));
        assert_eq!(ops.len(), 2);
        assert_eq!(ops["testWrite"].kind, "mutation");
        assert!(operations("\"name\",\"query\",\"bad hash\"").is_empty());
        for url in [
            "http://open.spotifycdn.com/cdn/build/web-player/a.js",
            "https://evil.test/cdn/build/web-player/a.js",
            "https://open.spotifycdn.com:444/cdn/build/web-player/a.js",
            "https://secret@open.spotifycdn.com/cdn/build/web-player/a.js",
        ] {
            assert!(!allowed_asset(url));
        }
    }
}
