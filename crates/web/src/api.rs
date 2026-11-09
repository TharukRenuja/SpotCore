use crate::{Browser, domains::*, http::Http};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use spotcore_auth::WebSession;
use spotcore_core::{Error, paths::Paths};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

pub const OPERATIONS_JSON: &str = include_str!("../resources/gql-ops.json");
pub const OPERATION_SCHEMA_JSON: &str = include_str!("../resources/gql-schema.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub kind: String,
    pub hash: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Operations {
    pub build_version: String,
    pub operations: BTreeMap<String, Operation>,
}
impl Operations {
    pub fn from_json(source: &str) -> Result<Self, Error> {
        let table: Self = serde_json::from_str(source)
            .map_err(|_| Error::InvalidInput("invalid operation table"))?;
        if table.operations.is_empty()
            || table.operations.iter().any(|(name, op)| {
                name.is_empty()
                    || !matches!(op.kind.as_str(), "query" | "mutation")
                    || op.hash.len() != 64
                    || !op.hash.bytes().all(|c| c.is_ascii_hexdigit())
            })
        {
            return Err(Error::InvalidInput("invalid persisted GraphQL operation"));
        }
        Ok(table)
    }
}

fn load_cached_operations(path: &Path) -> Option<BTreeMap<String, Operation>> {
    let data = std::fs::read_to_string(path).ok()?;
    let ops = Operations::from_json(&data).ok()?;
    Some(ops.operations)
}

fn save_cached_operations(path: &Path, ops: &Operations) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(ops) {
        let _ = std::fs::write(path, json);
    }
}

struct Inner {
    browser: Option<Browser>,
    session: Option<WebSession>,
    cookie: Option<crate::cookie::CookieSession>,
    operations: RwLock<Operations>,
    schema: Value,
    http: Http,
    /// Per-root cache file, so a custom `Paths` keeps its hashes with it.
    cache: PathBuf,
}

/// Spotify data methods over a browser session or direct authenticated HTTP.
#[derive(Clone)]
pub struct Api {
    inner: Arc<Inner>,
}
impl Api {
    pub fn new(browser: Browser) -> Result<Self, Error> {
        Self::with_paths(browser, &Paths::discover()?)
    }

    /// Keeps the operation cache under a caller-chosen data root.
    pub fn with_paths(browser: Browser, paths: &Paths) -> Result<Self, Error> {
        Self::build(Some(browser), None, paths)
    }

    /// HTTP-only services. The caller supplies a fresh session when this one expires.
    pub fn from_session(session: WebSession, paths: &Paths) -> Result<Self, Error> {
        if !session.is_authenticated() {
            return Err(Error::NotAuthenticated);
        }
        if session.client_id.trim().is_empty() || session.app_version.trim().is_empty() {
            return Err(Error::InvalidInput(
                "session requires client ID and app version",
            ));
        }
        Self::build(None, Some(session), paths)
    }

    /// HTTP-only services with an in-memory sp_dc cookie and lazy automatic token renewal.
    pub fn from_cookie(cookie: impl Into<String>, paths: &Paths) -> Result<Self, Error> {
        let cookie = crate::cookie::CookieSession::new(cookie.into())?;
        let mut api = Self::build(None, None, paths)?;
        Arc::get_mut(&mut api.inner)
            .ok_or_else(|| Error::Backend("shared client during initialization".into()))?
            .cookie = Some(cookie);
        Ok(api)
    }

    fn build(
        browser: Option<Browser>,
        session: Option<WebSession>,
        paths: &Paths,
    ) -> Result<Self, Error> {
        let paths = paths.resolve()?;
        let mut ops = Operations::from_json(OPERATIONS_JSON)?;
        let cache = paths.cache().join("gql-hashes.json");
        if let Some(cached) = load_cached_operations(&cache) {
            for (k, v) in cached {
                ops.operations.insert(k, v);
            }
        }
        Ok(Self {
            inner: Arc::new(Inner {
                browser,
                session,
                cookie: None,
                operations: RwLock::new(ops),
                schema: serde_json::from_str(OPERATION_SCHEMA_JSON)
                    .map_err(|_| Error::Backend("invalid bundled operation schema".into()))?,
                http: Http::new()?,
                cache,
            }),
        })
    }
    pub async fn connect(endpoint: &str) -> Result<Self, Error> {
        Self::new(Browser::connect(endpoint).await?)
    }
    pub async fn connect_with_paths(endpoint: &str, paths: &Paths) -> Result<Self, Error> {
        Self::with_paths(Browser::connect(endpoint).await?, paths)
    }
    /// Checks whether GraphQL operation hashes are cached locally on disk.
    pub fn has_cached_operations(&self) -> bool {
        self.inner.cache.is_file()
    }
    /// Returns Unsupported for an HTTP-only client.
    pub fn browser(&self) -> Result<&Browser, Error> {
        self.inner.browser.as_ref().ok_or(Error::Unsupported)
    }

    pub(crate) async fn invalidate_session(&self) {
        if let Some(cookie) = &self.inner.cookie {
            cookie.invalidate().await;
        }
    }
    pub(crate) fn device_id(&self) -> &str {
        self.inner.http.device_id()
    }
    pub(crate) fn session_epoch(&self) -> Option<u64> {
        self.inner.browser.as_ref().map(Browser::session_epoch)
    }
    pub fn operations(&self) -> Result<Operations, Error> {
        self.inner
            .operations
            .read()
            .map(|v| v.clone())
            .map_err(|_| Error::Backend("operation table lock poisoned".into()))
    }
    /// Swap hashes after a Web Player update. No request is automatically replayed.
    pub fn replace_operations(&self, operations: Operations) -> Result<(), Error> {
        let checked = Operations::from_json(
            &serde_json::to_string(&operations)
                .map_err(|_| Error::InvalidInput("invalid operation table"))?,
        )?;
        *self
            .inner
            .operations
            .write()
            .map_err(|_| Error::Backend("operation table lock poisoned".into()))? = checked;
        Ok(())
    }
    /// Discovers hashes from public player assets, or running scripts for browser sessions.
    /// Merges recovered definitions into the active table and its cache.
    pub async fn refresh_operations(&self) -> Result<usize, Error> {
        if self.inner.browser.is_none() {
            let discovered = crate::assets::PlayerAssets::load()
                .await?
                .discover()
                .await?;
            let count = discovered.operations.len();
            let mut active = self
                .inner
                .operations
                .write()
                .map_err(|_| Error::Backend("operation table lock poisoned".into()))?;
            active.build_version = discovered.build_version;
            active.operations.extend(discovered.operations);
            save_cached_operations(&self.inner.cache, &active);
            return Ok(count);
        }
        let discovered = self.call("gql.discover", json!({})).await?;
        let Some(map) = discovered.as_object() else {
            return Ok(0);
        };
        let mut count = 0;
        let mut ops_guard = self
            .inner
            .operations
            .write()
            .map_err(|_| Error::Backend("operation table lock poisoned".into()))?;

        for (name, val) in map {
            let Some(hash) = val.get("hash").and_then(Value::as_str) else {
                continue;
            };
            let Some(kind) = val.get("kind").and_then(Value::as_str) else {
                continue;
            };
            if hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit()) {
                ops_guard.operations.insert(
                    name.clone(),
                    Operation {
                        kind: kind.to_string(),
                        hash: hash.to_string(),
                    },
                );
                count += 1;
            }
        }
        if count > 0 {
            save_cached_operations(&self.inner.cache, &ops_guard);
        }
        Ok(count)
    }
    pub fn operation_schema(&self, name: &str) -> Option<&Value> {
        self.inner.schema.get(name)
    }

    pub async fn session(&self) -> Result<WebSession, Error> {
        if let Some(cookie) = &self.inner.cookie {
            return cookie.session().await;
        }
        if let Some(session) = &self.inner.session {
            return if session.is_authenticated() {
                Ok(session.clone())
            } else {
                Err(Error::NotAuthenticated)
            };
        }
        let value = self.browser()?.call("token.get", json!({})).await?;
        let session: WebSession =
            serde_json::from_value(value).map_err(|_| Error::NotAuthenticated)?;
        if !session.is_authenticated() {
            return Err(Error::NotAuthenticated);
        }
        Ok(session)
    }
    pub async fn is_signed_in(&self) -> Result<bool, Error> {
        match self.session().await {
            Ok(_) => Ok(true),
            Err(Error::NotAuthenticated) => Ok(false),
            Err(error) => Err(error),
        }
    }
    pub async fn call(&self, method: &str, args: Value) -> Result<Value, Error> {
        if self.inner.browser.is_none() {
            return crate::connect::call(self, method, args).await;
        }
        self.browser()?.call(method, args).await
    }
    pub async fn services(&self) -> Result<Value, Error> {
        self.call("service.list", json!({})).await
    }
    pub async fn service(
        &self,
        service: &str,
        method: &str,
        args: Vec<Value>,
    ) -> Result<Value, Error> {
        if service.is_empty() || method.is_empty() || method.starts_with('_') {
            return Err(Error::InvalidInput("invalid browser service method"));
        }
        if self.inner.browser.is_none() {
            return crate::direct::service(self, service, method, &args).await;
        }
        self.call(
            "service.call",
            json!({"service":service,"method":method,"args":args}),
        )
        .await
    }
    /// Full persisted GraphQL response; any GraphQL error (including partial data) fails the call.
    /// Missing or rejected persisted hashes are discovered once before a retry.
    pub async fn gql(&self, name: &str, variables: Value) -> Result<Value, Error> {
        if !variables.is_object() {
            return Err(Error::InvalidInput("GraphQL variables must be an object"));
        }
        let op_opt = self
            .inner
            .operations
            .read()
            .map_err(|_| Error::Backend("operation table lock poisoned".into()))?
            .operations
            .get(name)
            .cloned();

        let operation = match op_opt {
            Some(op) => op,
            None => {
                self.refresh_operations().await?;
                self.inner
                    .operations
                    .read()
                    .map_err(|_| Error::Backend("operation table lock poisoned".into()))?
                    .operations
                    .get(name)
                    .cloned()
                    .ok_or(Error::InvalidInput("unknown persisted GraphQL operation"))?
            }
        };

        let result = self
            .query_operation(name, &operation, variables.clone())
            .await?;

        if is_persisted_query_error(&result) {
            self.refresh_operations().await?;
            let fresh_op = self
                .inner
                .operations
                .read()
                .map_err(|_| Error::Backend("operation table lock poisoned".into()))?
                .operations
                .get(name)
                .cloned();
            if let Some(fresh_op) = fresh_op
                && fresh_op.hash != operation.hash
            {
                let retry_result = self.query_operation(name, &fresh_op, variables).await?;
                check_graphql(&retry_result)?;
                return Ok(retry_result);
            }
        }

        check_graphql(&result)?;
        Ok(result)
    }
    async fn query_operation(
        &self,
        name: &str,
        operation: &Operation,
        variables: Value,
    ) -> Result<Value, Error> {
        if self.inner.browser.is_some() {
            self.call("gql.query", json!({"name":name,"hash":operation.hash,"operation":operation.kind,"variables":variables})).await
        } else {
            self.inner
                .http
                .graphql(
                    self,
                    json!({
                        "operationName":name,"variables":variables,
                        "extensions":{"persistedQuery":{"version":1,"sha256Hash":operation.hash}}
                    }),
                )
                .await
        }
    }

    /// Authenticated requests are HTTPS-only, Spotify-host-only, with redirects disabled.
    pub async fn request(
        &self,
        method: &str,
        url: &str,
        body: Option<Value>,
    ) -> Result<Value, Error> {
        self.inner.http.request(self, method, url, body).await
    }

    pub async fn request_raw(
        &self,
        method: &str,
        url: &str,
        body: Option<Vec<u8>>,
        content_type: &str,
    ) -> Result<crate::RawResponse, Error> {
        self.inner
            .http
            .raw(self, method, url, body, content_type)
            .await
    }

    pub fn search(&self) -> Search<'_> {
        Search(self)
    }
    pub fn catalog(&self) -> Catalog<'_> {
        Catalog(self)
    }
    pub fn library(&self) -> Library<'_> {
        Library(self)
    }
    pub fn playlists(&self) -> Playlists<'_> {
        Playlists(self)
    }
    pub fn home(&self) -> Home<'_> {
        Home(self)
    }
    pub fn podcasts(&self) -> Podcasts<'_> {
        Podcasts(self)
    }
    pub fn users(&self) -> Users<'_> {
        Users(self)
    }
    pub fn extras(&self) -> Extras<'_> {
        Extras(self)
    }
    pub fn devices(&self) -> Devices<'_> {
        Devices(self)
    }
    pub fn jam(&self) -> Jam<'_> {
        Jam(self)
    }
    pub fn blends(&self) -> Blends<'_> {
        Blends(self)
    }
    pub fn recents(&self) -> Recents<'_> {
        Recents(self)
    }
}

pub(crate) fn is_persisted_query_error(result: &Value) -> bool {
    if result.get("data").is_some_and(|value| !value.is_null()) {
        return false;
    }
    result["errors"].as_array().is_some_and(|errors| {
        errors.iter().any(|e| {
            let msg = e["message"].as_str().unwrap_or("").to_lowercase();
            msg.contains("persistedquerynotfound")
                || msg.contains("persisted query not found")
                || e["extensions"]["code"] == "PERSISTED_QUERY_NOT_FOUND"
        })
    })
}

fn check_graphql(result: &Value) -> Result<(), Error> {
    if result["errors"].as_array().is_some_and(|e| !e.is_empty()) {
        let message = result["errors"][0]["message"]
            .as_str()
            .unwrap_or("GraphQL operation failed");
        return Err(Error::Backend(message.chars().take(500).collect()));
    }
    if result.get("data").is_none_or(Value::is_null) {
        return Err(Error::Backend("GraphQL returned no data".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn test_session() -> WebSession {
        serde_json::from_value(json!({
            "accessToken":"test-secret", "expiresAtMs":u64::MAX,
            "clientId":"client", "appVersion":"version", "isAnonymous":false
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn http_only_sessions_reject_browser_calls_and_expiry() {
        let paths = Paths::new(
            std::env::temp_dir().join(format!("spotcore-http-test-{}", uuid::Uuid::new_v4())),
        );
        let mut api = Api::from_session(test_session(), &paths).unwrap();
        assert!(matches!(api.browser(), Err(Error::Unsupported)));
        assert!(matches!(api.services().await, Err(Error::Unsupported)));

        assert!(matches!(
            crate::WebBackend::new(api.clone()).await,
            Err(Error::Unsupported)
        ));
        assert!(api.session().await.unwrap().is_authenticated());

        assert!(matches!(
            api.gql("profileAttributes", json!([])).await,
            Err(Error::InvalidInput(_))
        ));
        let mut incomplete = test_session();
        incomplete.client_id.clear();
        assert!(matches!(
            Api::from_session(incomplete, &paths),
            Err(Error::InvalidInput(_))
        ));
        Arc::get_mut(&mut api.inner)
            .unwrap()
            .session
            .as_mut()
            .unwrap()
            .expires_at_ms = 0;
        assert!(matches!(api.session().await, Err(Error::NotAuthenticated)));
        assert!(matches!(
            api.users().me().await,
            Err(Error::NotAuthenticated)
        ));
        let mut expired = test_session();
        expired.expires_at_ms = 0;
        assert!(matches!(
            Api::from_session(expired, &paths),
            Err(Error::NotAuthenticated)
        ));
    }

    #[tokio::test]
    #[ignore = "requires a private Free-account cookie file and public CDN access"]
    async fn http_only_recovers_a_rejected_hash_from_public_assets() {
        let file =
            std::env::var("SPOTCORE_FREE_COOKIE_FILE").expect("private cookie file required");
        let cookie = std::fs::read_to_string(file).unwrap();
        let paths = Paths::new(
            std::env::temp_dir().join(format!("spotcore-hash-test-{}", uuid::Uuid::new_v4())),
        );
        let api = Api::from_cookie(cookie.trim(), &paths).unwrap();
        assert_eq!(
            api.users().me().await.unwrap().tier,
            spotcore_auth::AccountTier::Free
        );
        let mut ops = api.operations().unwrap();
        ops.operations.get_mut("profileAttributes").unwrap().hash = "0".repeat(64);
        api.replace_operations(ops).unwrap();
        assert_eq!(
            api.users().me().await.unwrap().tier,
            spotcore_auth::AccountTier::Free
        );
        assert_ne!(
            api.operations().unwrap().operations["profileAttributes"].hash,
            "0".repeat(64)
        );
        assert!(api.browser().is_err());
        let _ = std::fs::remove_dir_all(paths.root());
    }

    #[test]
    fn bundled_operations_are_valid_and_mutations_are_explicit() {
        let table = Operations::from_json(OPERATIONS_JSON).unwrap();
        assert!(table.operations.len() >= 170);
        assert_eq!(table.operations["addToPlaylist"].kind, "mutation");
        let schema: Value = serde_json::from_str(OPERATION_SCHEMA_JSON).unwrap();
        for name in table.operations.keys() {
            assert!(schema.get(name).is_some(), "missing schema for {name}");
        }
    }
    #[test]
    fn partial_graphql_errors_do_not_hide_failed_writes() {
        assert!(
            check_graphql(&json!({"data":{"changed":true},"errors":[{"message":"failed"}]}))
                .is_err()
        );
        assert!(check_graphql(&json!({"data":null})).is_err());
        assert!(check_graphql(&json!({"data":{}})).is_ok());
    }
    #[test]
    fn persisted_query_errors_are_detected() {
        assert!(!is_persisted_query_error(
            &json!({"errors":[{"message":"Track not found"}]})
        ));
        assert!(is_persisted_query_error(
            &json!({"errors":[{"extensions":{"code":"PERSISTED_QUERY_NOT_FOUND"}}]})
        ));
        assert!(is_persisted_query_error(&json!({
            "errors": [{"message": "PersistedQueryNotFound"}]
        })));
        assert!(is_persisted_query_error(&json!({
            "errors": [{"message": "Persisted query not found"}]
        })));
        assert!(!is_persisted_query_error(&json!({
            "errors": [{"message": "Validation error"}]
        })));
        assert!(!is_persisted_query_error(&json!({
            "data": {}
        })));
    }
}
