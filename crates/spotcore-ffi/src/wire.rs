//! Shared request protocol for the C ABI and native Node bridge.
use crate::{BindingError, Session, SessionOptions};
use serde::{Deserialize, Serialize};

pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Login {
        cookie: String,
        options: SessionOptions,
        native_key: Option<String>,
    },
    Invoke {
        operation: String,
        arguments: serde_json::Value,
    },
    Operations,
    Account,
    SearchTracks {
        query: String,
        limit: u32,
        offset: u32,
    },
    Play {
        uri: String,
    },
    Pause,
    NextEvent,
    PollEvent,
    Shutdown,
}

pub(crate) fn response<T: Serialize>(result: Result<T, BindingError>) -> String {
    match result {
        Ok(result) => serde_json::json!({"ok":true,"result":result}).to_string(),
        Err(error) => serde_json::json!({"ok":false,"error":error}).to_string(),
    }
}

pub async fn call(session: &Session, request: &str) -> String {
    call_inner(session, request, true).await
}

pub(crate) async fn call_c(session: &Session, request: &str) -> String {
    call_inner(session, request, false).await
}

async fn call_inner(session: &Session, request: &str, allow_event_wait: bool) -> String {
    if request.len() > MAX_REQUEST_BYTES {
        return response::<()>(Err(BindingError::new(
            "input_too_large",
            "Request exceeds 1 MiB",
        )));
    }
    let request = match serde_json::from_str::<Request>(request) {
        Ok(request) => request,
        Err(_) => {
            return response::<()>(Err(BindingError::new(
                "invalid_input",
                "Invalid request JSON or unsupported operation",
            )));
        }
    };
    match request {
        Request::Login {
            cookie,
            options,
            native_key,
        } => response(session.login(cookie, options, native_key).await),
        Request::Operations => response(Ok::<_, BindingError>(
            serde_json::from_str::<serde_json::Value>(&session.operations())
                .expect("operation inventory"),
        )),
        Request::Invoke {
            operation,
            arguments,
        } => {
            let result = session
                .invoke(operation, arguments.to_string())
                .await
                .and_then(|json| {
                    serde_json::from_str::<serde_json::Value>(&json)
                        .map_err(|_| BindingError::new("internal", "Invalid operation result"))
                });
            response(result)
        }
        Request::Account => response(session.account().await),
        Request::SearchTracks {
            query,
            limit,
            offset,
        } => response(session.search_tracks(query, limit, offset).await),
        Request::Play { uri } => response(session.play(uri).await),
        Request::Pause => response(session.pause().await),
        Request::NextEvent if !allow_event_wait => response::<()>(Err(BindingError::new(
            "unsupported",
            "Use poll_event with the blocking C interface",
        ))),
        Request::NextEvent => response(session.next_event().await),
        Request::PollEvent => response(session.poll_event()),
        Request::Shutdown => response(session.shutdown().await),
    }
}
