use crate::cdp::{Cdp, is_spotify_page};
use crate::{WebPlayerOptions, page};
use serde_json::{Value, json};
use spotcore_core::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{sync::Arc, time::Duration};
use tokio::sync::{broadcast, watch};

pub const BRIDGE_SOURCE: &str = include_str!("../resources/player-bridge.js");

#[derive(Debug, Clone)]
pub enum BrowserEvent {
    Ready,
    State(Value),
    Queue(Value),
    SessionChanged,
    Error(String),
}

#[derive(Clone, Default)]
struct Status {
    epoch: u64,
    context: Option<(u64, String)>,
    ready: bool,
    error: Option<String>,
}
struct Inner {
    cdp: Cdp,
    status: watch::Receiver<Status>,
    events: broadcast::Sender<BrowserEvent>,
    task: tokio::task::JoinHandle<()>,
    minimal: Arc<AtomicBool>,
    endpoint: String,
    target_id: Option<String>,
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// A shared connection to one Spotify tab. Dropping the last handle detaches the SDK.
#[derive(Clone)]
pub struct Browser {
    inner: Arc<Inner>,
}

impl Browser {
    pub async fn connect(endpoint: &str) -> Result<Self, Error> {
        Self::connect_with_options(
            endpoint,
            WebPlayerOptions {
                minimal_ui: false,
                device_name: None,
            },
        )
        .await
    }
    pub async fn connect_tab(endpoint: &str, target_id: &str) -> Result<Self, Error> {
        Self::attach(
            endpoint,
            Some(target_id),
            WebPlayerOptions {
                minimal_ui: false,
                device_name: None,
            },
        )
        .await
    }

    pub async fn connect_tab_with_options(
        endpoint: &str,
        target_id: &str,
        options: WebPlayerOptions,
    ) -> Result<Self, Error> {
        Self::attach(endpoint, Some(target_id), options).await
    }

    pub async fn connect_with_options(
        endpoint: &str,
        options: WebPlayerOptions,
    ) -> Result<Self, Error> {
        Self::attach(endpoint, None, options).await
    }

    pub async fn connect_with_role(
        endpoint: &str,
        options: WebPlayerOptions,
        role: crate::BrowserRole,
    ) -> Result<Self, Error> {
        Self::attach_role(endpoint, None, options, role).await
    }

    async fn attach(
        endpoint: &str,
        target_id: Option<&str>,
        options: WebPlayerOptions,
    ) -> Result<Self, Error> {
        Self::attach_role(endpoint, target_id, options, crate::BrowserRole::Playback).await
    }

    async fn attach_role(
        endpoint: &str,
        target_id: Option<&str>,
        options: WebPlayerOptions,
        role: crate::BrowserRole,
    ) -> Result<Self, Error> {
        let cdp = Cdp::connect(endpoint, target_id, options.minimal_ui).await?;
        let mut incoming = cdp.subscribe();
        let tree = cdp.request("Page.getFrameTree", json!({})).await?;
        let mut main_frame = tree["frameTree"]["frame"]["id"]
            .as_str()
            .ok_or(Error::Disconnected)?
            .to_owned();
        if !tree["frameTree"]["frame"]["url"]
            .as_str()
            .is_some_and(|url| is_spotify_page(url) || (options.minimal_ui && url == "about:blank"))
        {
            return Err(Error::Disconnected);
        }
        let (status_tx, status) = watch::channel(Status::default());
        let (events, _) = broadcast::channel(128);
        let output = events.clone();
        let transport = cdp.clone();
        let minimal = Arc::new(AtomicBool::new(options.minimal_ui));
        let minimal_task = minimal.clone();
        let task = tokio::spawn(async move {
            let mut current = Status::default();
            while let Ok(packet) = incoming.recv().await {
                let params = &packet["params"];
                match packet["method"].as_str() {
                    Some("Fetch.requestPaused") => {
                        page::intercept(&transport, params, &minimal_task).await;
                    }

                    Some("Runtime.executionContextCreated") => {
                        let context = &params["context"];
                        if context["origin"] != "https://open.spotify.com"
                            || context["auxData"]["isDefault"] != true
                            || context["auxData"]["frameId"] != main_frame
                        {
                            continue;
                        }
                        let (Some(id), Some(unique)) =
                            (context["id"].as_u64(), context["uniqueId"].as_str())
                        else {
                            continue;
                        };
                        current = Status {
                            epoch: current.epoch + 1,
                            context: Some((id, unique.to_owned())),
                            ..Default::default()
                        };
                        status_tx.send_replace(current.clone());
                        let _ = output.send(BrowserEvent::SessionChanged);
                        let result = transport
                            .request(
                                "Runtime.evaluate",
                                json!({"expression": format!("window.__spotcoreServiceOnly={};{}", role == crate::BrowserRole::Services, BRIDGE_SOURCE),
                            "uniqueContextId": unique, "returnByValue": true}),
                            )
                            .await;
                        if !matches!(result, Ok(ref value) if value.get("exceptionDetails").is_none())
                        {
                            current.error = Some("browser bridge injection failed".into());
                            status_tx.send_replace(current.clone());
                        }
                    }
                    Some("Page.frameNavigated") if params["frame"].get("parentId").is_none() => {
                        if let Some(id) = params["frame"]["id"].as_str() {
                            main_frame = id.into();
                        }
                        if !params["frame"]["url"].as_str().is_some_and(is_spotify_page) {
                            current = Status {
                                epoch: current.epoch + 1,
                                ..Default::default()
                            };
                            status_tx.send_replace(current.clone());
                            let _ = output.send(BrowserEvent::SessionChanged);
                        }
                    }
                    Some("Runtime.executionContextsCleared") => {
                        current = Status {
                            epoch: current.epoch + 1,
                            ..Default::default()
                        };
                        status_tx.send_replace(current.clone());
                        let _ = output.send(BrowserEvent::SessionChanged);
                    }
                    Some("Runtime.executionContextDestroyed")
                        if current
                            .context
                            .as_ref()
                            .is_some_and(|(id, _)| params["executionContextId"] == *id) =>
                    {
                        current = Status {
                            epoch: current.epoch + 1,
                            ..Default::default()
                        };
                        status_tx.send_replace(current.clone());
                        let _ = output.send(BrowserEvent::SessionChanged);
                    }
                    Some("Runtime.bindingCalled") if params["name"] == "__mspHostPost" => {
                        if !current
                            .context
                            .as_ref()
                            .is_some_and(|(id, _)| params["executionContextId"] == *id)
                        {
                            continue;
                        }
                        let Some(payload) = params["payload"]
                            .as_str()
                            .filter(|s| s.len() <= 1024 * 1024)
                        else {
                            continue;
                        };
                        let Ok(message) = serde_json::from_str::<Value>(payload) else {
                            continue;
                        };
                        match message["type"].as_str() {
                            Some("ready") if message["version"] == 1 => {
                                current.ready = true;
                                current.error = None;
                                status_tx.send_replace(current.clone());
                                let _ = output.send(BrowserEvent::Ready);
                            }
                            Some("state") if current.ready => {
                                let _ = output.send(BrowserEvent::State(message["state"].clone()));
                            }
                            Some("queue") if current.ready => {
                                let _ = output.send(BrowserEvent::Queue(message["queue"].clone()));
                            }
                            Some("error") => {
                                let message = message["error"]
                                    .as_str()
                                    .unwrap_or("browser bridge failed")
                                    .to_owned();
                                current.ready = false;
                                current.error = Some(message.clone());
                                status_tx.send_replace(current.clone());
                                let _ = output.send(BrowserEvent::Error(message));
                            }
                            _ => {}
                        }
                    }
                    Some("SpotCore.disconnected") => break,
                    _ => {}
                }
            }
            current.ready = false;
            current.error = Some("browser disconnected or event stream overflowed".into());
            status_tx.send_replace(current);
            let _ = output.send(BrowserEvent::SessionChanged);
        });
        let browser = Self {
            inner: Arc::new(Inner {
                cdp,
                status,
                events,
                task,
                minimal: minimal.clone(),
                endpoint: endpoint.to_owned(),
                target_id: target_id.map(str::to_owned),
            }),
        };
        browser.inner.cdp.request("Page.enable", json!({})).await?;
        browser
            .inner
            .cdp
            .request("Runtime.addBinding", json!({"name":"__mspHostPost"}))
            .await?;
        browser
            .inner
            .cdp
            .request("Runtime.enable", json!({}))
            .await?;
        let _ = browser.inner.cdp.request("Network.enable", json!({})).await;
        let blocked: Vec<String> = [
            "*.png",
            "*.jpg",
            "*.jpeg",
            "*.webp",
            "*.gif",
            "*.svg",
            "*.woff",
            "*.woff2",
            "*.ttf",
            "*google-analytics.com*",
            "*branch.io*",
            "*recaptcha*",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let _ = browser
            .inner
            .cdp
            .request("Network.setBlockedURLs", json!({"urls":blocked}))
            .await;
        if options.minimal_ui {
            browser
                .inner
                .cdp
                .request("Network.setBypassServiceWorker", json!({"bypass":true}))
                .await?;
        }
        let startup = async {
            if options.minimal_ui {
                browser.reload().await?;
            }
            browser
                .wait_ready(if options.minimal_ui { 25 } else { 65 })
                .await
        }
        .await;
        if let Err(error) = startup {
            if !options.minimal_ui {
                return Err(error);
            }
            minimal.store(false, Ordering::Relaxed);
            browser.reload().await?;
            browser.await_ready().await?;
        }
        if let Some(name) = options
            .device_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            // Cosmetic: a player that refuses the label still plays.
            let _ = browser
                .call("player.setDeviceName", json!({"name": name}))
                .await;
        }
        let _ = browser
            .inner
            .cdp
            .request("Network.setCacheDisabled", json!({"cacheDisabled":false}))
            .await;
        if options.minimal_ui
            && browser
                .call("player.runtime", json!({}))
                .await
                .is_ok_and(|v| v["minimalUi"] == true)
        {
            let _ = browser
                .inner
                .cdp
                .request("HeapProfiler.collectGarbage", json!({}))
                .await;
        }
        if options.minimal_ui {
            page::release_bundle_buffers();
        }
        Ok(browser)
    }

    /// Requests a graceful exit of an owned host. Never call this on an external browser.
    pub async fn shutdown_host(&self) -> Result<(), Error> {
        self.inner
            .cdp
            .request("Browser.close", json!({}))
            .await
            .map(|_| ())
    }

    pub fn is_ready(&self) -> bool {
        self.inner.status.borrow().ready
    }
    pub fn session_epoch(&self) -> u64 {
        self.inner.status.borrow().epoch
    }
    pub fn subscribe(&self) -> broadcast::Receiver<BrowserEvent> {
        self.inner.events.subscribe()
    }

    async fn reload(&self) -> Result<(), Error> {
        let epoch = self.session_epoch();
        let mut status = self.inner.status.clone();
        if self.inner.minimal.load(Ordering::Relaxed) {
            self.inner
                .cdp
                .request("Network.setCacheDisabled", json!({"cacheDisabled":true}))
                .await?;
        }
        page::optimized_navigation(
            &self.inner.cdp,
            &self.inner.endpoint,
            self.inner.target_id.as_deref(),
            &self.inner.minimal,
        )
        .await?;
        tokio::time::timeout(Duration::from_secs(10), async {
            while status.borrow_and_update().epoch == epoch {
                status.changed().await.map_err(|_| Error::Disconnected)?;
            }
            Ok(())
        })
        .await
        .map_err(|_| Error::Timeout)?
    }

    pub async fn await_ready(&self) -> Result<(), Error> {
        self.wait_ready(65).await
    }

    async fn wait_ready(&self, seconds: u64) -> Result<(), Error> {
        let mut status = self.inner.status.clone();
        tokio::time::timeout(Duration::from_secs(seconds), async {
            loop {
                {
                    let current = status.borrow_and_update();
                    if current.ready {
                        return Ok(());
                    }
                    if let Some(error) = &current.error {
                        return Err(Error::Backend(error.clone()));
                    }
                }
                status.changed().await.map_err(|_| Error::Disconnected)?;
            }
        })
        .await
        .map_err(|_| Error::Timeout)?
    }

    /// Calls are bound to one execution context; ambiguous writes are never replayed.
    pub async fn call(&self, method: &str, args: Value) -> Result<Value, Error> {
        let status = self.inner.status.borrow().clone();
        if !status.ready {
            return Err(Error::Disconnected);
        }
        let (_, unique) = status.context.ok_or(Error::Disconnected)?;
        let expression = format!("window.Myspoti.call(null,{},{})", json!(method), args);
        let result = self.inner.cdp.request("Runtime.evaluate", json!({"expression":expression,
            "uniqueContextId":unique, "awaitPromise":true, "returnByValue":true, "userGesture":true})).await?;
        if self.session_epoch() != status.epoch || !self.is_ready() {
            return Err(Error::Disconnected);
        }
        if let Some(details) = result.get("exceptionDetails") {
            let message = details["exception"]["description"]
                .as_str()
                .or_else(|| details["text"].as_str())
                .unwrap_or("browser API call failed");
            return Err(Error::Backend(message.chars().take(500).collect()));
        }
        Ok(result["result"]
            .get("value")
            .cloned()
            .unwrap_or(Value::Null))
    }
}
