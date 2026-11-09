use crate::cdp::Cdp;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};

/// Browser-side playback behavior. Existing-tab attachment never opts into a reload implicitly.
#[derive(Debug, Clone)]
pub struct WebPlayerOptions {
    /// Skip the UI in known player builds; enabling this reloads the tab once.
    pub minimal_ui: bool,
    /// Connect name for managed Web playback; `None` keeps Spotify's own label.
    pub device_name: Option<String>,
}
impl Default for WebPlayerOptions {
    fn default() -> Self {
        Self {
            minimal_ui: true,
            device_name: Some("SpotCore".into()),
        }
    }
}

pub(crate) fn minimal_script(source: &str) -> Option<String> {
    // shortcut: unknown bootstrap structure keeps the full UI; tolerate only the factory's minified name.
    const PREFIX: &str = "let o=await ";
    const SUFFIX: &str = "(),l=o.getFeatureFlags(),u=o.getRegistry()";
    let mut matches = source.match_indices(SUFFIX);
    let (end, _) = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    let start = source[..end].rfind(PREFIX)?;
    let factory = &source[start + PREFIX.len()..end];
    if factory.is_empty()
        || factory.len() > 32
        || !factory.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphabetic()
                || byte == b'_'
                || byte == b'$'
                || (index > 0 && byte.is_ascii_digit())
        })
    {
        return None;
    }
    let marker = &source[start..end + SUFFIX.len()];
    Some(source.replacen(marker, &format!("let o=await {factory}();window.__spotcorePlatform=o;return;let l=o.getFeatureFlags(),u=o.getRegistry()"), 1))
}

pub(crate) async fn intercept(cdp: &Cdp, params: &Value, minimal: &AtomicBool) {
    let Some(id) = params["requestId"].as_str() else {
        return;
    };
    let mut large = false;
    let result = async {
        if !params["responseStatusCode"].as_u64().is_some_and(|n| (200..300).contains(&n)) { return None; }
        let response = cdp.request("Fetch.getResponseBody", json!({"requestId":id})).await.ok()?;
        let body = response["body"].as_str()?;
        let bytes = if response["base64Encoded"] == true { STANDARD.decode(body).ok()? } else { body.as_bytes().to_vec() };
        large = bytes.len() > 1024 * 1024;
        let source = String::from_utf8(bytes).ok()?;
        let url = url::Url::parse(params["request"]["url"].as_str()?).ok()?;
        if url.scheme() != "https"
            || !url.host_str().is_some_and(|h| h.ends_with(".spotifycdn.com"))
            || !url.path().starts_with("/cdn/build/web-player/web-player.")
            || !minimal.load(Ordering::Relaxed)
        {
            return None;
        }
        let output = minimal_script(&source)?;
        let headers: Vec<Value> = params["responseHeaders"].as_array()?.iter().filter(|h| {
            !h["name"].as_str().is_some_and(|n| ["content-length", "content-encoding", "etag"].iter().any(|k| n.eq_ignore_ascii_case(k)))
        }).cloned().collect();
        cdp.request("Fetch.fulfillRequest", json!({"requestId":id,"responseCode":params["responseStatusCode"],"responseHeaders":headers,"body":STANDARD.encode(output)})).await.ok()
    }.await;
    if result.is_none() {
        let _ = cdp
            .request("Fetch.continueRequest", json!({"requestId":id}))
            .await;
    }
    if large {
        release_bundle_buffers();
    }
}

pub(crate) fn release_bundle_buffers() {
    // Return the temporary bundle/base64 buffers instead of retaining them in the SDK process.
    #[cfg(target_os = "linux")]
    unsafe {
        libc::malloc_trim(0);
    }
}

pub(crate) async fn navigate(cdp: &Cdp) -> Result<(), spotcore_core::Error> {
    let tree = cdp.request("Page.getFrameTree", json!({})).await?;
    if tree["frameTree"]["frame"]["url"] == "about:blank" {
        cdp.request(
            "Page.navigate",
            json!({"url":"https://open.spotify.com/preferences"}),
        )
        .await?;
    } else {
        cdp.request("Page.reload", json!({"ignoreCache":true}))
            .await?;
    }
    Ok(())
}

pub(crate) async fn optimized_navigation(
    cdp: &Cdp,
    endpoint: &str,
    target_id: Option<&str>,
    minimal: &AtomicBool,
) -> Result<(), spotcore_core::Error> {
    if !minimal.load(Ordering::Relaxed) {
        return navigate(cdp).await;
    }
    // Isolate the large bundle buffers from the long-lived control connection.
    let patcher = Cdp::connect(endpoint, target_id, true).await?;
    let mut events = patcher.subscribe();
    let result = async {
        patcher.request("Page.enable", json!({})).await?;
        patcher.request("Fetch.enable", json!({"patterns":[{"urlPattern":"*cdn/build/web-player/web-player.*.js","requestStage":"Response"}]})).await?;
        navigate(cdp).await?;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            while let Ok(packet) = events.recv().await {
                if packet["method"] == "Fetch.requestPaused" {
                    intercept(&patcher, &packet["params"], minimal).await;
                    break;
                }
            }
        }).await;
        Ok(())
    }.await;
    let _ = patcher.request("Fetch.disable", json!({})).await;
    patcher.close().await;
    release_bundle_buffers();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_or_ambiguous_bootstrap_is_unchanged() {
        assert!(minimal_script("new Spotify build").is_none());
        for factory in ["TT", "TE", "$platform_1"] {
            let current = format!(
                "let o=await {factory}(),l=o.getFeatureFlags(),u=o.getRegistry(),v=o.getSession();"
            );
            assert!(minimal_script(&current).unwrap().contains(&format!(
                "await {factory}();window.__spotcorePlatform=o;return;"
            )));
        }
        assert!(
            minimal_script("let o=await malicious.call(),l=o.getFeatureFlags(),u=o.getRegistry()")
                .is_none()
        );
        let source = "let o=await TE(),l=o.getFeatureFlags(),u=o.getRegistry();";
        assert!(minimal_script(&source.repeat(2)).is_none());
        assert!(
            minimal_script(source)
                .unwrap()
                .contains("window.__spotcorePlatform=o;return;")
        );
    }
}
