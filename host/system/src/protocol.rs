use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex, mpsc::Receiver},
    time::{Duration, Instant},
};
use tungstenite::{Message, protocol::WebSocketConfig};

pub const MAX_MESSAGE: usize = 8 * 1024 * 1024;

pub fn spotify_origin(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str() == Some("open.spotify.com")
            && u.port_or_known_default() == Some(443)
            && u.username().is_empty()
            && u.password().is_none()
    })
}

pub fn evaluate(id: u64, epoch: u64, expression: &str, await_promise: bool) -> String {
    let expression = json!(expression);
    let result = if await_promise {
        "await value"
    } else {
        "value"
    };
    let mut script = String::from(
        "(async()=>{let result;try{if(location.origin!=='https://open.spotify.com')throw Error('origin changed');let value=(0,eval)(",
    );
    script.push_str(&expression.to_string());
    script.push_str(");result={result:{value:");
    script.push_str(result);
    script.push_str("}};}catch(_){result={exceptionDetails:{text:'system engine evaluation failed'}};}window.ipc.postMessage(JSON.stringify({type:'eval',id:");
    script.push_str(&id.to_string());
    script.push_str(",epoch:");
    script.push_str(&epoch.to_string());
    script.push_str(",result}));})();");
    script
}

fn consume_header(stream: &mut TcpStream, header: &str) -> std::io::Result<()> {
    let end = header
        .find("\r\n\r\n")
        .ok_or(std::io::ErrorKind::InvalidData)?
        + 4;
    let mut bytes = vec![0; end];
    stream.read_exact(&mut bytes)
}

fn response(stream: &mut TcpStream, status: &str, body: &str) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// A loopback-only compatibility endpoint for the SDK's Page/Runtime protocol subset.
pub fn serve(
    listener: TcpListener,
    current_url: Arc<Mutex<String>>,
    output: Receiver<Value>,
    command: impl Fn(Value) + Send + 'static,
) {
    let port = listener.local_addr().unwrap().port();
    let socket_path = format!("/{}", uuid::Uuid::new_v4());
    for mut stream in listener.incoming().flatten() {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
        let mut header = [0; 8192];
        let deadline = Instant::now() + Duration::from_secs(3);
        let size = loop {
            match stream.peek(&mut header) {
                Ok(0) | Err(_) => break 0,
                Ok(size) if header[..size].windows(4).any(|v| v == b"\r\n\r\n") => break size,
                _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
                _ => break 0,
            }
        };
        let Ok(header) = std::str::from_utf8(&header[..size]) else {
            continue;
        };
        if header.lines().skip(1).any(|line| {
            line.split_once(':')
                .is_some_and(|(key, _)| key.eq_ignore_ascii_case("origin"))
        }) {
            let _ = consume_header(&mut stream, header);
            let _ = response(&mut stream, "403 Forbidden", "{}");
            continue;
        }
        let path = header
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("GET "))
            .and_then(|line| line.strip_suffix(" HTTP/1.1"));
        if path == Some(socket_path.as_str()) {
            // A new owner must receive a fresh Runtime.enable announcement, not old replies.
            while output.try_recv().is_ok() {}
            let config = WebSocketConfig::default()
                .max_message_size(Some(MAX_MESSAGE))
                .max_frame_size(Some(MAX_MESSAGE));
            let Ok(mut socket) = tungstenite::accept_with_config(stream, Some(config)) else {
                continue;
            };
            let _ = socket
                .get_mut()
                .set_read_timeout(Some(Duration::from_millis(30)));
            loop {
                while let Ok(packet) = output.try_recv() {
                    if socket
                        .send(Message::Text(packet.to_string().into()))
                        .is_err()
                    {
                        break;
                    }
                }
                match socket.read() {
                    Ok(Message::Text(text)) => {
                        let Ok(packet) = serde_json::from_str::<Value>(&text) else {
                            break;
                        };
                        if packet["id"].as_u64().is_none() || packet["method"].as_str().is_none() {
                            break;
                        }
                        command(packet);
                    }
                    Ok(Message::Ping(data)) => {
                        if socket.send(Message::Pong(data)).is_err() {
                            break;
                        }
                    }
                    Ok(Message::Close(_)) | Err(tungstenite::Error::ConnectionClosed) => break,
                    Err(tungstenite::Error::Io(error))
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    Err(_) => break,
                    _ => {}
                }
            }
        } else {
            let body = match path {
                Some("/json/list") => {
                    json!([{"id":"spotify","type":"page","url":*current_url.lock().unwrap(),"webSocketDebuggerUrl":format!("ws://127.0.0.1:{port}{socket_path}")}])
                }
                Some("/json/version") => {
                    json!({"Browser":"spotcore-engine-system/1","SpotCore":{"responseInterception":false,"webPlayback":false}})
                }
                _ => {
                    let _ = consume_header(&mut stream, header);
                    let _ = response(&mut stream, "404 Not Found", "{}");
                    continue;
                }
            };
            if consume_header(&mut stream, header).is_ok() {
                let _ = response(&mut stream, "200 OK", &body.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn origins_and_expressions_are_not_interpolated_as_code() {
        assert!(spotify_origin("https://open.spotify.com/search"));
        for value in [
            "https://open.spotify.com.evil/",
            "https://user@open.spotify.com/",
            "http://open.spotify.com/",
            "https://open.spotify.com:444/",
        ] {
            assert!(!spotify_origin(value));
        }
        let expression = "'\\\"\n);process.exit()";
        assert!(evaluate(1, 2, expression, true).contains(&json!(expression).to_string()));
        assert!(evaluate(1, 2, "1+1", true).contains("await value"));
    }

    #[test]
    fn loopback_endpoint_exposes_capabilities_and_rejects_browser_origins() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (responses, output) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            serve(
                listener,
                Arc::new(Mutex::new("https://open.spotify.com/".into())),
                output,
                move |packet| {
                    let _ = responses.try_send(json!({"id":packet["id"],"result":{"value":42}}));
                },
            )
        });
        for (path, extra, expected) in [
            ("/json/version", "", "200 OK"),
            ("/json/list", "", "200 OK"),
            (
                "/json/list",
                "Origin: https://evil.example\r\n",
                "403 Forbidden",
            ),
        ] {
            let mut stream = TcpStream::connect(address).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            write!(
                stream,
                "GET {path} HTTP/1.1\r\nHost: {address}\r\n{extra}\r\n"
            )
            .unwrap();
            let mut response = String::new();
            let _ = stream.read_to_string(&mut response);
            assert!(
                response.contains(expected),
                "{path}: expected {expected}, received {response:?}"
            );
            if path == "/json/list" && extra.is_empty() {
                let targets: Value =
                    serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
                let (mut socket, _) =
                    tungstenite::connect(targets[0]["webSocketDebuggerUrl"].as_str().unwrap())
                        .unwrap();
                socket
                    .send(Message::Text(
                        json!({"id":1,"method":"test.echo","params":{}})
                            .to_string()
                            .into(),
                    ))
                    .unwrap();
                let message = socket.read().unwrap().into_text().unwrap();
                let reply: Value = serde_json::from_str(&message).unwrap();
                assert_eq!(reply["id"], 1);
                assert_eq!(reply["result"]["value"], 42);
                socket.close(None).unwrap();
            }
        }
    }
}
