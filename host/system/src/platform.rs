use crate::protocol::{self, MAX_MESSAGE};
use serde_json::{Value, json};
#[cfg(target_os = "macos")]
use std::io::Write;
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    net::TcpListener,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{SyncSender, sync_channel},
    },
    time::Duration,
};
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};
use wry::{PageLoadEvent, WebContext, WebViewBuilder};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
enum HostEvent {
    Command(Value),
    Loaded(PageLoadEvent, String),
    Ipc(String, String),
    Exit(bool),
    #[cfg(windows)]
    Stopped(bool),
}

fn emit(sender: &SyncSender<Value>, value: Value) {
    if sender.try_send(value).is_err() {
        eprintln!("system engine control queue overflowed or closed");
        std::process::exit(1);
    }
}
fn failure(id: u64) -> Value {
    json!({"id":id,"error":{"code":-32601,"message":"unsupported or stale system engine command"}})
}
fn context(epoch: u64) -> Value {
    json!({"method":"Runtime.executionContextCreated","params":{"context":{"id":epoch,"uniqueId":format!("system-{epoch}"),"origin":"https://open.spotify.com","auxData":{"isDefault":true,"frameId":"main"}}}})
}

pub fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let value = |name: &str| {
        args.windows(2)
            .find(|pair| pair[0] == name)
            .map(|pair| pair[1].clone())
    };
    let profile = PathBuf::from(value("--profile-dir").ok_or("profile directory required")?);
    if profile.as_os_str().is_empty() {
        return Err("profile directory is empty".into());
    }
    fs::create_dir_all(&profile)?;
    let profile = std::path::absolute(profile)?;
    let _lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(profile.join("system-engine.lock"))?;
    _lock.try_lock()?;
    let login = args.iter().any(|arg| arg == "--login");
    let smoke = args.iter().any(|arg| arg == "--smoke");
    let forget = args.iter().any(|arg| arg == "--forget");
    let smoke_cookie = value("--cookie-value");
    let expected_cookie = value("--expect-cookie");
    #[cfg(target_os = "macos")]
    let store_id = {
        let version = std::process::Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()?;
        let major: u32 = std::str::from_utf8(&version.stdout)?
            .trim()
            .split('.')
            .next()
            .ok_or("missing macOS version")?
            .parse()?;
        if major < 14 {
            return Err("isolated WKWebView profiles require macOS 14+".into());
        }
        let path = profile.join("wk-store-id");
        if forget && !path.exists() {
            return Ok(());
        }
        if !path.exists() {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)?;
            file.write_all(uuid::Uuid::new_v4().to_string().as_bytes())?;
            file.sync_all()?;
        }
        let id = uuid::Uuid::parse_str(fs::read_to_string(path)?.trim())?;
        *id.as_bytes()
    };
    #[cfg(not(target_os = "macos"))]
    if forget {
        return Ok(());
    }
    let event_loop = EventLoopBuilder::<HostEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_title("SpotCore login")
        .with_visible(login || args.iter().any(|arg| arg == "--visible"))
        .with_inner_size(tao::dpi::LogicalSize::new(1024.0, 768.0))
        .build(&event_loop)?;
    let mut web_context = WebContext::new(Some(profile.clone()));
    let ipc = proxy.clone();
    let loaded = proxy.clone();
    let mut builder = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_autoplay(false)
        .with_incognito(forget)
        .with_initialization_script("if(location.origin==='https://open.spotify.com'){window.__mspHostPost=payload=>window.ipc.postMessage(JSON.stringify({type:'binding',payload}));}")
        .with_ipc_handler(move |request| {
            if request.body().len() <= MAX_MESSAGE { let _ = ipc.send_event(HostEvent::Ipc(request.uri().to_string(), request.body().clone())); }
        })
        .with_on_page_load_handler(move |event, url| { let _ = loaded.send_event(HostEvent::Loaded(event, url)); });
    #[cfg(target_os = "macos")]
    {
        use wry::WebViewBuilderExtDarwin;
        if !forget {
            builder = builder.with_data_store_identifier(store_id);
        }
        builder = builder.with_background_throttling(wry::BackgroundThrottlingPolicy::Disabled);
    }
    if smoke {
        builder = builder.with_html("<!doctype html><title>SpotCore engine smoke</title><script>window.ipc.postMessage(JSON.stringify({type:'smoke',value:6*7}));</script>");
    }
    if forget {
        builder = builder.with_html("<!doctype html><title>Profile cleanup</title>");
    }
    #[cfg(target_os = "linux")]
    let view = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        builder.build_gtk(
            window
                .default_vbox()
                .ok_or("GTK content container unavailable")?,
        )?
    };
    #[cfg(not(target_os = "linux"))]
    let view = builder.build(&window)?;
    let mut webview = Some(view);
    if !forget && let Some(view) = &webview {
        if let Some(cookie) = std::env::var_os("SPOTCORE_SP_DC") {
            let cookie = cookie
                .into_string()
                .map_err(|_| "invalid cookie encoding")?;
            if cookie.is_empty() || cookie.contains(['\r', '\n', '\0']) {
                return Err("invalid session cookie".into());
            }
            #[cfg(target_os = "linux")]
            seed_linux_session_cookie(view, &cookie)?;
            #[cfg(not(target_os = "linux"))]
            view.set_cookie(
                &wry::cookie::Cookie::build(("sp_dc", cookie))
                    .domain(".spotify.com")
                    .path("/")
                    .secure(true)
                    .http_only(true)
                    .expires(
                        wry::cookie::time::OffsetDateTime::now_utc()
                            + wry::cookie::time::Duration::days(365),
                    )
                    .build(),
            )?;
        }
        if smoke
            && let Ok(seed) = std::env::var("SPOTCORE_SP_DC")
            && !view
                .cookies_for_url("https://open.spotify.com/")?
                .iter()
                .any(|cookie| cookie.name() == "sp_dc" && cookie.value() == seed)
        {
            return Err("session cookie did not cover the player origin".into());
        }
        if smoke && let Some(cookie) = smoke_cookie {
            view.set_cookie(
                &wry::cookie::Cookie::build(("spotcore_smoke", cookie))
                    .domain("open.spotify.com")
                    .path("/")
                    .secure(true)
                    .expires(
                        wry::cookie::time::OffsetDateTime::now_utc()
                            + wry::cookie::time::Duration::days(1),
                    )
                    .build(),
            )?;
        }
        if !smoke {
            view.load_url(if login {
                "https://accounts.spotify.com/login"
            } else {
                "https://open.spotify.com"
            })?;
        }
    }
    let current_url = Arc::new(Mutex::new("about:blank".to_owned()));
    let (output, incoming) = sync_channel(256);
    if !login && !smoke && !forget {
        let port = value("--cdp-port")
            .ok_or("debug port required")?
            .parse::<u16>()?;
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let requests = proxy.clone();
        let url = current_url.clone();
        std::thread::spawn(move || {
            protocol::serve(listener, url, incoming, move |packet| {
                let _ = requests.send_event(HostEvent::Command(packet));
            })
        });
    } else {
        drop(incoming);
    }
    if smoke || forget {
        let timeout = proxy.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(30));
            let _ = timeout.send_event(HostEvent::Exit(false));
        });
    }
    if let Some(parent) = value("--parent-pid") {
        let parent = parent.parse::<u32>()?;
        let watch = proxy.clone();
        std::thread::spawn(move || {
            loop {
                if !parent_alive(parent) {
                    let _ = watch.send_event(HostEvent::Exit(true));
                    break;
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        });
    }
    #[cfg(target_os = "macos")]
    let mut cleanup_started = false;
    #[cfg(windows)]
    let mut closing = false;
    let mut epoch = 1u64;
    let mut enabled = false;
    let mut pending = HashSet::new();
    event_loop.run(move |event, _, control| {
        *control = ControlFlow::Wait;
        match event {
            #[cfg(target_os = "macos")]
            Event::UserEvent(HostEvent::Loaded(PageLoadEvent::Finished, _)) if forget && !cleanup_started => {
                use wry::WebViewExtDarwin;
                cleanup_started = true;
                // WebKit's store IO requires initialization by a webview, not just AppKit.
                webview.take();
                let done = proxy.clone();
                wry::WebView::remove_data_store(&store_id, move |result| {
                    let _ = done.send_event(HostEvent::Exit(result.is_ok()));
                });
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } | Event::UserEvent(HostEvent::Exit(true)) => {
                #[cfg(windows)]
                {
                    use wry::WebViewExtWindows;
                    if closing { return; }
                    closing = true;
                    let mut pid = 0;
                    if let Some(view) = &webview { let _ = unsafe { view.webview().BrowserProcessId(&mut pid) }; }
                    webview.take();
                    let done = proxy.clone();
                    std::thread::spawn(move || {
                        let success = wait_browser_exit(pid);
                        let _ = done.send_event(HostEvent::Stopped(success));
                    });
                }
                #[cfg(not(windows))]
                { webview.take(); *control = ControlFlow::Exit; }
            }
            #[cfg(windows)]
            Event::UserEvent(HostEvent::Stopped(success)) => {
                if success { *control = ControlFlow::Exit; }
                else { eprintln!("WebView2 browser did not finish shutting down"); std::process::exit(1); }
            }
            Event::UserEvent(HostEvent::Exit(false)) => { eprintln!("system engine smoke or cleanup failed"); std::process::exit(1); }
            Event::UserEvent(HostEvent::Loaded(stage, url)) => {
                *current_url.lock().unwrap() = url.clone();
                if login || smoke || forget { return; }
                match stage {
                    PageLoadEvent::Started => {
                        epoch += 1;
                        for id in pending.drain() { emit(&output, failure(id)); }
                        emit(&output, json!({"method":"Runtime.executionContextsCleared"}));
                    }
                    PageLoadEvent::Finished => {
                        emit(&output, json!({"method":"Page.frameNavigated","params":{"frame":{"id":"main","url":url}}}));
                        if enabled && protocol::spotify_origin(&url) { emit(&output, context(epoch)); }
                    }
                }
            }
            Event::UserEvent(HostEvent::Ipc(origin, body)) => {
                let Ok(packet) = serde_json::from_str::<Value>(&body) else { return; };
                if smoke && packet["type"] == "eval" {
                    let valid = packet["id"] == 1 && packet["epoch"] == 0 && packet["result"]["result"]["value"] == 42;
                    let _ = proxy.send_event(HostEvent::Exit(valid));
                    return;
                }
                if smoke && packet["type"] == "smoke" && packet["value"] == 42 {
                    if let Some(view) = &webview {
                        if let Some(expected) = &expected_cookie {
                            let valid = view.cookies_for_url("https://open.spotify.com/").is_ok_and(|cookies| {
                                let value = cookies.iter().find(|cookie| cookie.name() == "spotcore_smoke").map(|cookie| cookie.value());
                                if expected == "absent" { value.is_none() } else { value == Some(expected.as_str()) }
                            });
                            if !valid {
                                eprintln!("synthetic cookie persistence/isolation check failed");
                                let _ = proxy.send_event(HostEvent::Exit(false)); return;
                            }
                        }
                        let script = protocol::evaluate(1, 0, "Promise.resolve(42)", true);
                        // The synthetic about:blank fixture supplies the trusted origin locally.
                        let script = format!("(()=>{{const location={{origin:'https://open.spotify.com'}};{script}}})();");
                        if view.evaluate_script(&script).is_err() { let _ = proxy.send_event(HostEvent::Exit(false)); }
                    }
                    return;
                }
                if !protocol::spotify_origin(&origin) { return; }
                match packet["type"].as_str() {
                    Some("eval") => {
                        if let Some(id) = packet["id"].as_u64() && pending.remove(&id) {
                            if packet["epoch"] == epoch { emit(&output, json!({"id":id,"result":packet["result"]})); }
                            else { emit(&output, failure(id)); }
                        }
                    }
                    Some("binding") => {
                        if enabled && let Some(payload) = packet["payload"].as_str() {
                            emit(&output, json!({"method":"Runtime.bindingCalled","params":{"name":"__mspHostPost","executionContextId":epoch,"payload":payload}}));
                        }
                    }
                    _ => {}
                }
            }
            Event::UserEvent(HostEvent::Command(packet)) => {
                let Some(view) = &webview else { return; };
                let id = packet["id"].as_u64().unwrap();
                let params = &packet["params"];
                let result = match packet["method"].as_str().unwrap() {
                    "Page.getFrameTree" => Some(json!({"frameTree":{"frame":{"id":"main","url":*current_url.lock().unwrap()}}})),
                    "Page.enable" | "Network.enable" => Some(json!({})),
                    "Network.getCookies" if params["urls"] == json!(["https://open.spotify.com/"])
                        && protocol::spotify_origin(&current_url.lock().unwrap()) => {
                        match view.cookies_for_url("https://open.spotify.com/") {
                            Ok(cookies) => Some(json!({"cookies": cookies.into_iter().filter(|cookie| cookie.name() == "sp_dc").map(|cookie| json!({"name":cookie.name(), "value":cookie.value(), "domain":cookie.domain().unwrap_or("open.spotify.com")})).collect::<Vec<_>>()})),
                            Err(_) => { emit(&output, failure(id)); return; }
                        }
                    }
                    "Browser.close" => { let _ = proxy.send_event(HostEvent::Exit(true)); Some(json!({})) }

                    "Runtime.addBinding" if params["name"] == "__mspHostPost" => Some(json!({})),
                    "Runtime.enable" => {
                        enabled = true;
                        if protocol::spotify_origin(&current_url.lock().unwrap()) { emit(&output, context(epoch)); }
                        Some(json!({}))
                    }
                    "Runtime.evaluate" => {
                        let valid = params["uniqueContextId"].as_str().is_none_or(|s| s == format!("system-{epoch}"));
                        if !valid || !protocol::spotify_origin(&current_url.lock().unwrap()) || pending.len() >= 128 { emit(&output, failure(id)); return; }
                        let Some(expression) = params["expression"].as_str() else { emit(&output, failure(id)); return; };
                        pending.insert(id);
                        let script = protocol::evaluate(id, epoch, expression, params["awaitPromise"] == true);
                        if view.evaluate_script(&script).is_err() { pending.remove(&id); emit(&output, failure(id)); }
                        None
                    }
                    "Page.reload" => { if view.reload().is_err() { emit(&output, failure(id)); return; } Some(json!({})) }
                    "Page.navigate" if params["url"].as_str().is_some_and(protocol::spotify_origin) => {
                        if view.load_url(params["url"].as_str().unwrap()).is_err() { emit(&output, failure(id)); return; } Some(json!({}))
                    }
                    _ => { emit(&output, failure(id)); return; }
                };
                if let Some(result) = result { emit(&output, json!({"id":id,"result":result})); }
            }
            _ => {}
        }
    });
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn parent_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}
#[cfg(windows)]
fn parent_alive(pid: u32) -> bool {
    unsafe {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, WAIT_TIMEOUT},
            System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        };
        let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if process.is_null() {
            return false;
        }
        let alive = WaitForSingleObject(process, 0) == WAIT_TIMEOUT;
        CloseHandle(process);
        alive
    }
}

#[cfg(windows)]
fn wait_browser_exit(pid: u32) -> bool {
    if pid == 0 {
        return true;
    }
    unsafe {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, WAIT_OBJECT_0},
            System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        };
        let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if process.is_null() {
            return true;
        }
        let exited = WaitForSingleObject(process, 10000) == WAIT_OBJECT_0;
        CloseHandle(process);
        exited
    }
}

#[cfg(target_os = "linux")]
fn seed_linux_session_cookie(view: &wry::WebView, value: &str) -> Result<()> {
    use webkit2gtk::{CookieManagerExt, WebViewExt, WebsiteDataManagerExt};
    use wry::WebViewExtUnix;
    let manager = view
        .webview()
        .website_data_manager()
        .and_then(|data| data.cookie_manager())
        .ok_or("cookie manager unavailable")?;
    // Wry's Cookie conversion strips the dot that libsoup needs for subdomain scope.
    let mut cookie = soup::Cookie::new("sp_dc", value, ".spotify.com", "/", 365 * 24 * 60 * 60);
    cookie.set_secure(true);
    cookie.set_http_only(true);
    let (sender, receiver) = std::sync::mpsc::channel();
    manager.add_cookie(
        &mut cookie,
        None::<&webkit2gtk::gio::Cancellable>,
        move |result| {
            let _ = sender.send(result);
        },
    );
    let context = webkit2gtk::glib::MainContext::default();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        context.iteration(false);
        if let Ok(result) = receiver.try_recv() {
            result?;
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err("session cookie import timed out".into());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
