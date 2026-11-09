use crate::{Account, BindingError, PlaybackUpdate, SearchPage, SessionInfo, SessionOptions};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};

type Reply<T> = oneshot::Sender<Result<T, BindingError>>;
enum Request {
    Login(String, SessionOptions, Option<String>, Reply<SessionInfo>),
    Account(Reply<Account>),
    Search(String, u32, u32, Reply<SearchPage>),
    Play(String, Reply<()>),
    Pause(Reply<()>),
    Invoke(String, serde_json::Value, Reply<String>),
    Shutdown(Option<Reply<()>>),
}

#[boltffi::export]
pub struct Session {
    commands: mpsc::UnboundedSender<Request>,
    events: Mutex<broadcast::Receiver<Result<PlaybackUpdate, BindingError>>>,
    state: Arc<AtomicU8>,
}

#[boltffi::export]
impl Session {
    pub fn new() -> Result<Self, BindingError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| BindingError::new("runtime", error.to_string()))?;
        let (commands, receiver) = mpsc::unbounded_channel();
        let (events, event_receiver) = broadcast::channel(64);
        let state = Arc::new(AtomicU8::new(0));
        let worker_state = state.clone();
        std::thread::Builder::new()
            .name("spotcore-ffi".into())
            .spawn(move || {
                tokio::task::LocalSet::new()
                    .block_on(&runtime, run(receiver, events, worker_state));
            })
            .map_err(|error| BindingError::new("runtime", error.to_string()))?;
        Ok(Self {
            commands,
            events: Mutex::new(event_receiver),
            state,
        })
    }

    pub async fn login(
        &self,
        cookie: String,
        options: SessionOptions,
        native_key: Option<String>,
    ) -> Result<SessionInfo, BindingError> {
        request(&self.commands, |reply| {
            Request::Login(cookie, options, native_key, reply)
        })
        .await
    }
    pub async fn account(&self) -> Result<Account, BindingError> {
        request(&self.commands, Request::Account).await
    }
    pub async fn search_tracks(
        &self,
        query: String,
        limit: u32,
        offset: u32,
    ) -> Result<SearchPage, BindingError> {
        request(&self.commands, |reply| {
            Request::Search(query, limit, offset, reply)
        })
        .await
    }
    pub async fn play(&self, uri: String) -> Result<(), BindingError> {
        request(&self.commands, |reply| Request::Play(uri, reply)).await
    }
    pub async fn pause(&self) -> Result<(), BindingError> {
        request(&self.commands, Request::Pause).await
    }
    pub async fn next_event(&self) -> Result<PlaybackUpdate, BindingError> {
        match self.state.load(Ordering::Acquire) {
            0 => return Err(spotcore::Error::NotAuthenticated.into()),
            1 => return Err(spotcore::Error::Unsupported.into()),
            3 => return Err(BindingError::closed()),
            _ => {}
        }
        self.events
            .lock()
            .await
            .recv()
            .await
            .map_err(|error| match error {
                broadcast::error::RecvError::Closed => BindingError::closed(),
                broadcast::error::RecvError::Lagged(count) => {
                    BindingError::new("events_lagged", format!("Missed {count} playback events"))
                }
            })?
    }
    /// Calls a documented domain operation; returns JSON without renaming SDK fields.
    pub async fn invoke(
        &self,
        operation: String,
        arguments_json: String,
    ) -> Result<String, BindingError> {
        if operation.len() > 128 || arguments_json.len() > crate::wire::MAX_REQUEST_BYTES {
            return Err(BindingError::new(
                "input_too_large",
                "Operation input exceeds its size limit",
            ));
        }
        let arguments = serde_json::from_str(&arguments_json)
            .map_err(|_| BindingError::new("invalid_input", "Invalid argument JSON"))?;
        crate::operations::validate(&operation, &arguments)?;
        request(&self.commands, |reply| {
            Request::Invoke(operation, arguments, reply)
        })
        .await
    }
    /// Lists available operation names and argument types as JSON; no login required.
    pub fn operations(&self) -> String {
        crate::operations::INVENTORY.to_owned()
    }
    pub async fn shutdown(&self) -> Result<(), BindingError> {
        request(&self.commands, |reply| Request::Shutdown(Some(reply))).await
    }
}

impl Session {
    pub(crate) fn poll_event(&self) -> Result<Option<PlaybackUpdate>, BindingError> {
        match self.state.load(Ordering::Acquire) {
            0 => return Err(spotcore::Error::NotAuthenticated.into()),
            1 => return Err(spotcore::Error::Unsupported.into()),
            3 => return Err(BindingError::closed()),
            _ => {}
        }
        let mut events = self
            .events
            .try_lock()
            .map_err(|_| BindingError::new("busy", "Another event consumer is active"))?;
        match events.try_recv() {
            Ok(event) => event.map(Some),
            Err(broadcast::error::TryRecvError::Empty) => Ok(None),
            Err(broadcast::error::TryRecvError::Closed) => Err(BindingError::closed()),
            Err(broadcast::error::TryRecvError::Lagged(count)) => Err(BindingError::new(
                "events_lagged",
                format!("Missed {count} playback events"),
            )),
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.commands.send(Request::Shutdown(None));
    }
}

async fn request<T>(
    commands: &mpsc::UnboundedSender<Request>,
    build: impl FnOnce(Reply<T>) -> Request,
) -> Result<T, BindingError> {
    let (reply, response) = oneshot::channel();
    commands
        .send(build(reply))
        .map_err(|_| BindingError::closed())?;
    response.await.map_err(|_| BindingError::closed())?
}

async fn open(
    cookie: String,
    options: SessionOptions,
    native_key: Option<String>,
) -> Result<(spotcore::SpotCore, SessionInfo), BindingError> {
    if cookie.trim().is_empty() {
        return Err(BindingError::new(
            "invalid_input",
            "A session cookie is required",
        ));
    }
    if options.device_name.trim().is_empty() {
        return Err(BindingError::new(
            "invalid_input",
            "A device name is required",
        ));
    }
    let mut builder = spotcore::SpotCore::builder()
        .app_id(&options.app_id)?
        .session_cookie(cookie.trim())
        .device_name(options.device_name);
    if let Some(path) = options.data_dir {
        builder = builder.data_dir(path);
    }
    if !options.playback {
        builder = builder.browser_role(spotcore::BrowserRole::Services);
    }
    if options.playback
        && let Some(key) = native_key
    {
        builder = builder.native(key);
    }
    let sdk = builder.launch().await?;
    let account = match sdk.users().me().await {
        Ok(account) => account,
        Err(error) => {
            let _ = sdk.shutdown().await;
            return Err(error.into());
        }
    };
    let info = SessionInfo {
        account: account.into(),
        engine: if !options.playback {
            "services"
        } else if sdk.playback_mode() == spotcore::PlaybackMode::Native {
            "native"
        } else {
            "web"
        }
        .into(),
        fallback_reason: sdk.native_fallback_reason().map(ToString::to_string),
    };
    Ok((sdk, info))
}

async fn run(
    mut commands: mpsc::UnboundedReceiver<Request>,
    events: broadcast::Sender<Result<PlaybackUpdate, BindingError>>,
    state: Arc<AtomicU8>,
) {
    let mut sdk: Option<spotcore::SpotCore> = None;
    loop {
        let command = if state.load(Ordering::Acquire) == 2 {
            let session = sdk.as_mut().expect("playback state has a session");
            tokio::select! {
                command = commands.recv() => command,
                event = session.next_event() => {
                    match event {
                        Ok(event) => { let _ = events.send(Ok(event.into())); }
                        Err(error) => {
                            let _ = events.send(Err(error.into()));
                            state.store(1, Ordering::Release);
                        }
                    }
                    continue;
                }
            }
        } else {
            commands.recv().await
        };
        let Some(command) = command else { break };
        match command {
            Request::Login(cookie, options, native_key, reply) => {
                let result = if sdk.is_some() {
                    Err(BindingError::new(
                        "already_connected",
                        "Create a new Session to use another account",
                    ))
                } else {
                    open(cookie, options, native_key)
                        .await
                        .map(|(session, info)| {
                            state.store(
                                if info.engine == "services" { 1 } else { 2 },
                                Ordering::Release,
                            );
                            sdk = Some(session);
                            info
                        })
                };
                let _ = reply.send(result);
            }
            Request::Account(reply) => {
                let result = match sdk.as_ref() {
                    Some(sdk) => sdk.users().me().await.map(Into::into).map_err(Into::into),
                    None => Err(spotcore::Error::NotAuthenticated.into()),
                };
                let _ = reply.send(result);
            }
            Request::Search(query, limit, offset, reply) => {
                let result = match sdk.as_ref() {
                    Some(sdk) => sdk
                        .search()
                        .query(
                            spotcore::SearchKind::Tracks,
                            &query,
                            spotcore::Paging { limit, offset },
                        )
                        .await
                        .map(|page| SearchPage {
                            items: page.items.into_iter().map(Into::into).collect(),
                            total: page.total,
                            next_offset: page.next_offset,
                        })
                        .map_err(Into::into),
                    None => Err(spotcore::Error::NotAuthenticated.into()),
                };
                let _ = reply.send(result);
            }
            Request::Play(uri, reply) => {
                let result = match sdk.as_mut() {
                    Some(sdk) => sdk.play(uri).await.map_err(Into::into),
                    None => Err(spotcore::Error::NotAuthenticated.into()),
                };
                let _ = reply.send(result);
            }
            Request::Pause(reply) => {
                let result = match sdk.as_mut() {
                    Some(sdk) => sdk.pause().await.map_err(Into::into),
                    None => Err(spotcore::Error::NotAuthenticated.into()),
                };
                let _ = reply.send(result);
            }
            Request::Invoke(operation, arguments, reply) => {
                let result = match sdk.as_mut() {
                    Some(sdk) => crate::operations::call(sdk, &operation, arguments)
                        .await
                        .map(|value| value.to_string()),
                    None => Err(spotcore::Error::NotAuthenticated.into()),
                };
                let _ = reply.send(result);
            }
            Request::Shutdown(reply) => {
                let result = match sdk.take() {
                    Some(sdk) => sdk.shutdown().await.map_err(Into::into),
                    None => Ok(()),
                };
                state.store(3, Ordering::Release);
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
                break;
            }
        }
    }
    if let Some(sdk) = sdk {
        let _ = sdk.shutdown().await;
    }
    state.store(3, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn every_operation_reaches_the_worker_and_keeps_lifecycle_errors() {
        let session = Session::new().unwrap();
        let inventory: serde_json::Value = serde_json::from_str(&session.operations()).unwrap();
        let entries = inventory.as_array().unwrap();
        assert!(entries.len() >= 170);
        for entry in entries {
            let operation = entry["operation"].as_str().unwrap();
            assert_eq!(
                session
                    .invoke(operation.into(), "{}".into())
                    .await
                    .unwrap_err()
                    .code,
                "not_authenticated",
                "{operation}"
            );
        }
        for (operation, arguments, expected) in [
            ("library.unknown", "{}", "invalid_input"),
            ("library.add", "[]", "invalid_input"),
            ("library.add", "secret-cookie", "invalid_input"),
        ] {
            let error = session
                .invoke(operation.into(), arguments.into())
                .await
                .unwrap_err();
            assert_eq!(error.code, expected);
            assert!(!error.message.contains("secret-cookie"));
        }
        assert_eq!(
            session
                .invoke(
                    "library.add".into(),
                    "x".repeat(crate::wire::MAX_REQUEST_BYTES + 1)
                )
                .await
                .unwrap_err()
                .code,
            "input_too_large"
        );
        session.shutdown().await.unwrap();
        assert_eq!(
            session
                .invoke("library.add".into(), "{}".into())
                .await
                .unwrap_err()
                .code,
            "closed"
        );
    }

    #[tokio::test]
    async fn dropping_the_handle_stops_the_worker() {
        let session = Session::new().unwrap();
        let state = session.state.clone();
        drop(session);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while state.load(Ordering::Acquire) != 3 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn signed_out_calls_and_shutdown_do_not_need_an_engine() {
        let session = Session::new().unwrap();
        assert_eq!(
            session.account().await.unwrap_err().code,
            "not_authenticated"
        );
        assert_eq!(
            session.next_event().await.unwrap_err().code,
            "not_authenticated"
        );
        let error = session
            .login(
                String::new(),
                SessionOptions {
                    app_id: "ffi-test".into(),
                    device_name: "Test".into(),
                    playback: false,
                    data_dir: None,
                },
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "invalid_input");
        session.shutdown().await.unwrap();
        assert_eq!(session.account().await.unwrap_err().code, "closed");
    }
}
