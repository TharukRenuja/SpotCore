use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use spotcore_core::Error;
use std::{collections::HashMap, net::IpAddr, time::Duration};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};
use url::Url;

const DEADLINE: Duration = Duration::from_secs(30);
type Reply = oneshot::Sender<Result<Value, Error>>;
struct Request {
    method: &'static str,
    params: Value,
    reply: Reply,
}

#[derive(Clone)]
pub(crate) struct Cdp {
    requests: mpsc::Sender<Request>,
    events: broadcast::Sender<Value>,
}

pub(crate) fn is_spotify_page(value: &str) -> bool {
    Url::parse(value).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str() == Some("open.spotify.com")
            && u.port_or_known_default() == Some(443)
            && u.username().is_empty()
            && u.password().is_none()
    })
}

fn local_url(value: &str, websocket: bool) -> Result<Url, Error> {
    let url =
        Url::parse(value).map_err(|_| Error::InvalidInput("invalid browser debugging URL"))?;
    let local = url.host_str().is_some_and(|h| {
        h == "localhost"
            || h.trim_matches(['[', ']'])
                .parse::<IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if !local
        || url.scheme() != if websocket { "ws" } else { "http" }
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::InvalidInput(
            "browser debugging must use a local loopback URL without credentials",
        ));
    }
    Ok(url)
}

pub(crate) async fn body(mut response: reqwest::Response, max: usize) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| Error::Backend("HTTP response read failed".into()))?
    {
        if bytes.len().saturating_add(chunk.len()) > max {
            return Err(Error::Backend("HTTP response is too large".into()));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

impl Cdp {
    pub(crate) async fn connect(
        endpoint: &str,
        target_id: Option<&str>,
        allow_blank: bool,
    ) -> Result<Self, Error> {
        let mut url = local_url(endpoint, false)?;
        if url.query().is_some() || !matches!(url.path(), "" | "/") {
            return Err(Error::InvalidInput("use the browser debugging base URL"));
        }
        url.set_path("/json/list");
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(DEADLINE)
            .build()
            .map_err(|_| Error::Backend("HTTP client initialization failed".into()))?;
        let response = http.get(url).send().await.map_err(|_| {
            Error::Backend(format!("cannot reach the debugging endpoint at {endpoint}"))
        })?;
        if !response.status().is_success() {
            return Err(Error::Backend("browser target discovery failed".into()));
        }
        let targets: Vec<Value> = serde_json::from_slice(&body(response, 1024 * 1024).await?)
            .map_err(|_| Error::Backend("invalid browser target list".into()))?;
        let mut pages = targets.iter().filter(|v| {
            v["type"] == "page"
                && v["url"].as_str().is_some_and(|url| {
                    is_spotify_page(url) || (allow_blank && url == "about:blank")
                })
                && target_id.is_none_or(|id| v["id"] == id)
        });
        let page = pages.next().ok_or(Error::InvalidInput(
            "open a logged-in Spotify Web Player tab in the debugging browser",
        ))?;
        if target_id.is_none() && pages.next().is_some() {
            return Err(Error::InvalidInput(
                "multiple Spotify tabs; use connect_tab to choose an account",
            ));
        }
        let socket_url = local_url(
            page["webSocketDebuggerUrl"]
                .as_str()
                .ok_or(Error::Disconnected)?,
            true,
        )?;
        let config = WebSocketConfig::default()
            .max_message_size(Some(8 * 1024 * 1024))
            .max_frame_size(Some(8 * 1024 * 1024));
        let (mut socket, _) = tokio::time::timeout(
            DEADLINE,
            connect_async_with_config(socket_url.as_str(), Some(config), true),
        )
        .await
        .map_err(|_| Error::Timeout)?
        .map_err(|_| Error::Backend("browser websocket connection failed".into()))?;
        let (requests, mut rx) = mpsc::channel::<Request>(64);
        let (events, _) = broadcast::channel(256);
        let output = events.clone();
        tokio::spawn(async move {
            let mut next_id = 0_u64;
            let mut pending = HashMap::<u64, Reply>::new();
            let mut cleanup = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    request = rx.recv() => {
                        let Some(request) = request else { break; };
                        if request.reply.is_closed() { continue; }
                        if request.method == "SpotCore.close" {
                            let _ = tokio::time::timeout(Duration::from_secs(1), socket.close(None)).await;
                            drop(socket);
                            let _ = request.reply.send(Ok(json!({})));
                            for (_, reply) in pending { let _ = reply.send(Err(Error::Disconnected)); }
                            return;
                        }
                        if pending.len() >= 128 { let _ = request.reply.send(Err(Error::Backend("too many pending browser requests".into()))); continue; }
                        next_id += 1;
                        let packet = json!({"id":next_id, "method":request.method, "params":request.params});
                        pending.insert(next_id, request.reply);
                        if !matches!(tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Text(packet.to_string().into()))).await, Ok(Ok(()))) { break; }
                    }
                    packet = socket.next() => {
                        match packet {
                            Some(Ok(Message::Text(text))) => {
                                let Ok(packet) = serde_json::from_str::<Value>(&text) else { break; };
                                if let Some(id) = packet["id"].as_u64() {
                                    if let Some(reply) = pending.remove(&id) {
                                        let result = if packet.get("error").is_some() { Err(Error::Backend("browser protocol command failed".into())) }
                                            else { Ok(packet.get("result").cloned().unwrap_or(Value::Null)) };
                                        let _ = reply.send(result);
                                    }
                                } else if !packet["method"].as_str().is_some_and(|m| m.starts_with("Network.")) {
                                    // Network body events swamp the control stream; consumers use Fetch, Page and Runtime.
                                    let _ = output.send(packet);
                                }
                            }
                            Some(Ok(Message::Ping(data))) => {
                                if !matches!(tokio::time::timeout(Duration::from_secs(5), socket.send(Message::Pong(data))).await, Ok(Ok(()))) { break; }
                            }
                            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                            _ => {}
                        }
                    }
                    _ = cleanup.tick() => pending.retain(|_, reply| !reply.is_closed()),
                }
            }
            rx.close();
            for (_, reply) in pending {
                let _ = reply.send(Err(Error::Disconnected));
            }
            let _ = output.send(json!({"method":"SpotCore.disconnected"}));
            let _ = tokio::time::timeout(Duration::from_secs(1), socket.close(None)).await;
        });
        Ok(Self { requests, events })
    }

    pub(crate) async fn close(&self) {
        let _ = self.request("SpotCore.close", json!({})).await;
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.events.subscribe()
    }

    pub(crate) async fn request(
        &self,
        method: &'static str,
        params: Value,
    ) -> Result<Value, Error> {
        tokio::time::timeout(DEADLINE, async {
            let (reply, receive) = oneshot::channel();
            self.requests
                .send(Request {
                    method,
                    params,
                    reply,
                })
                .await
                .map_err(|_| Error::Disconnected)?;
            receive.await.map_err(|_| Error::Disconnected)?
        })
        .await
        .map_err(|_| Error::Timeout)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_remote_debuggers_and_lookalike_origins() {
        for url in [
            "http://example.com:9222",
            "http://user:pass@localhost:9222",
            "https://localhost:9222",
            "http://127.0.0.1.evil:9222",
        ] {
            assert!(local_url(url, false).is_err());
        }
        assert!(local_url("http://127.0.0.1:9222", false).is_ok());
        assert!(local_url("http://[::1]:9222", false).is_ok());
        assert!(is_spotify_page("https://open.spotify.com/track/id"));
        for url in [
            "https://open.spotify.com.evil/",
            "http://open.spotify.com/",
            "https://open.spotify.com@evil/",
        ] {
            assert!(!is_spotify_page(url));
        }
    }
}
