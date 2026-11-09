use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};
use tokio_tungstenite::{accept_async, tungstenite::Message};

pub const TRACK: &str = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";
#[derive(Clone)]
pub enum Control {
    Event(Value),
    Close,
    Reload,
}
pub struct MockBrowser {
    pub endpoint: String,
    pub calls: Arc<Mutex<Vec<(String, Value)>>>,
    pub control: mpsc::Sender<Control>,
    pub protocol: Arc<Mutex<Vec<(String, Value)>>>,
    pub response: Arc<Mutex<Value>>,
    http: tokio::task::JoinHandle<()>,
    socket: tokio::task::JoinHandle<()>,
}
impl Drop for MockBrowser {
    fn drop(&mut self) {
        self.http.abort();
        self.socket.abort();
    }
}

pub fn state() -> Value {
    json!({"track":{"uri":TRACK,"name":"Song","durationMs":1000},"isPaused":true,"isBuffering":false,"positionMs":0,"timestamp":1,"durationMs":1000,"restrictions":{"canToggleSmartShuffle":true},"repeat":0,"shuffle":false})
}
pub fn binding(message: Value, context: u64) -> Value {
    json!({"method":"Runtime.bindingCalled","params":{"name":"__mspHostPost","executionContextId":context,"payload":message.to_string()}})
}
fn context(id: u64) -> Value {
    json!({"method":"Runtime.executionContextCreated","params":{"context":{"id":id,"uniqueId":format!("context-{id}"),"origin":"https://open.spotify.com","auxData":{"isDefault":true,"frameId":"main"}}}})
}
impl MockBrowser {
    pub async fn start() -> Self {
        Self::start_with_drm(true).await
    }
    pub async fn start_with_drm(drm: bool) -> Self {
        Self::start_config(0, drm).await
    }
    pub async fn fail_injections(failures: usize) -> Self {
        Self::start_config(failures, true).await
    }
    async fn start_config(mut failures: usize, drm: bool) -> Self {
        let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ws = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", http.local_addr().unwrap());
        let ws_url = format!("ws://{}/devtools/page/spotify", ws.local_addr().unwrap());
        let http_task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = http.accept().await {
                let mut buffer = [0; 4096];
                let _ = stream.read(&mut buffer).await;
                let body=json!([{"id":"spotify","type":"page","url":"https://open.spotify.com/","webSocketDebuggerUrl":ws_url}]).to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });
        let (tx, mut rx) = mpsc::channel(16);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let protocol = Arc::new(Mutex::new(Vec::new()));
        let protocol_log = protocol.clone();
        let response = Arc::new(Mutex::new(json!({"body":"{}","base64Encoded":false})));
        let body_response = response.clone();
        let log = calls.clone();
        let socket = tokio::spawn(async move {
            let listener = ws;
            let (stream, _) = listener.accept().await.unwrap();
            let mut patchers = tokio::task::JoinSet::new();
            let mut ws = accept_async(stream).await.unwrap();
            let mut current_context = 1_u64;
            let mut current_state = state();
            current_state["drmSupport"] = json!(drm);
            let mut held = Vec::new();
            let mut reorder = None;
            loop {
                tokio::select! {
                    accepted=listener.accept()=>{
                        let (stream,_)=accepted.unwrap();
                        let body=body_response.clone();let protocol=protocol_log.clone();
                        patchers.spawn(async move {
                            let mut socket=accept_async(stream).await.unwrap();
                            while let Some(Ok(Message::Text(text)))=socket.next().await {
                                let req:Value=serde_json::from_str(&text).unwrap();
                                let method=req["method"].as_str().unwrap();
                                protocol.lock().unwrap().push((method.into(),req["params"].clone()));
                                let result=if method=="Fetch.getResponseBody"{body.lock().unwrap().clone()}else{json!({})};
                                if socket.send(Message::Text(json!({"id":req["id"],"result":result}).to_string().into())).await.is_err(){break;}
                                if method=="Fetch.enable" {
                                    let _=socket.send(Message::Text(json!({"method":"Fetch.requestPaused","params":{"requestId":"bundle","responseStatusCode":200,"responseHeaders":[],"request":{"url":"https://open.spotifycdn.com/cdn/build/web-player/web-player.test.js"}}}).to_string().into())).await;
                                }
                            }
                        });
                    },
                    command=rx.recv()=>match command {
                        Some(Control::Close)|None=>{let _=ws.close(None).await;break;},
                        Some(Control::Event(event))=>{if event["method"]=="Runtime.bindingCalled" && let Some(payload)=event["params"]["payload"].as_str() && let Ok(message)=serde_json::from_str::<Value>(payload) && message["type"]=="state" {current_state=message["state"].clone();}ws.send(Message::Text(event.to_string().into())).await.unwrap();},
                        Some(Control::Reload)=>{
                            ws.send(Message::Text(json!({"method":"Runtime.executionContextsCleared"}).to_string().into())).await.unwrap();
                            current_context+=1;
                            ws.send(Message::Text(context(current_context).to_string().into())).await.unwrap();
                            for id in held.drain(..){ws.send(Message::Text(json!({"id":id,"result":{"result":{"value":"old page result"}}}).to_string().into())).await.unwrap();}
                        }
                    },
                    packet=ws.next()=>{
                        let Some(Ok(Message::Text(text)))=packet else {break;};
                        let request:Value=serde_json::from_str(&text).unwrap();let id=request["id"].clone();
                        let method=request["method"].as_str().unwrap();
                        protocol_log.lock().unwrap().push((method.to_owned(),request["params"].clone()));
                        let mut result=json!({});
                        match method {
                            "Page.getFrameTree"=>result=json!({"frameTree":{"frame":{"id":"main","url":"https://open.spotify.com/"}}}),
                            "Page.reload"=>{
                                ws.send(Message::Text(json!({"method":"Runtime.executionContextsCleared"}).to_string().into())).await.unwrap();
                                current_context+=1;
                                ws.send(Message::Text(context(current_context).to_string().into())).await.unwrap();
                            },
                            "Fetch.getResponseBody"=>result=body_response.lock().unwrap().clone(),
                            "Network.setCookie"=>result=json!({"success":true}),
                            "Runtime.enable"=>{ws.send(Message::Text(context(current_context).to_string().into())).await.unwrap();},
                            "Runtime.evaluate"=>{
                                assert_eq!(request["params"]["uniqueContextId"],format!("context-{current_context}"));
                                let expression=request["params"]["expression"].as_str().unwrap();
                                if let Some(arguments)=expression.strip_prefix("window.Myspoti.call(null,").and_then(|s|s.strip_suffix(')')) {
                                    let args:Vec<Value>=serde_json::from_str(&format!("[{arguments}]")).unwrap();
                                    let name=args[0].as_str().unwrap();let params=args[1].clone();
                                    log.lock().unwrap().push((name.into(),params.clone()));
                                    if name=="test.pending" {held.push(id);continue;}
                                    let value=match name {
                                        "test.echo"=>params,
                                        "test.reorder"=>{let packet=json!({"id":id,"result":{"result":{"value":params}}});if let Some(first)=reorder.take(){ws.send(Message::Text(packet.to_string().into())).await.unwrap();ws.send(Message::Text(first)).await.unwrap();}else{reorder=Some(packet.to_string().into());}continue;},
                                        "token.get"=>json!({"accessToken":"test-secret","expiresAtMs":u64::MAX,"clientId":"client","appVersion":"version","isAnonymous":false}),
                                        "player.state"=>current_state.clone(),
                                        "player.drmSupport"=>json!({"widevine":current_state["drmSupport"].as_bool().unwrap_or(true),"aac":true,"error":null}),
                                        "player.audioInfo"=>json!({"bitrate":256000,"mime":"audio/aac","level":"very_high"}),
                                        "player.queue"=>json!({"current":null,"queued":[],"nextUp":[]}),
                                        "library.contains"=>json!(params["uris"].as_array().unwrap().iter().map(|_|true).collect::<Vec<_>>()),
                                        "service.call"=>match (params["service"].as_str().unwrap(),params["method"].as_str().unwrap()) {
                                            ("ConnectDevicesAPI","getDevices")=>json!([{ "id":"native-test", "name":"Test native", "isActive":true, "isLocal":false }]),
                                            ("RootlistAPI","contains")=>json!(params["args"][0].as_array().unwrap().iter().map(|_|false).collect::<Vec<_>>()),
                                            ("SocialConnectAPI","getEnabled")=>json!("RESTRICTED"),
                                            ("PlaylistPermissionsAPI","getPermissionGrant")=>json!({"token":"invite-token"}),
                                            _=>json!({"mock":true}),
                                        },
                                        "gql.query"=>{
                                            let schema:Value=serde_json::from_str(spotcore_web::OPERATION_SCHEMA_JSON).unwrap();
                                            if let Some(required)=schema[params["name"].as_str().unwrap()]["required"].as_object() {
                                                for name in required.keys() {assert!(!params["variables"][name].is_null(),"missing variable {name} for {}",params["name"]);}
                                            }
                                            match params["name"].as_str().unwrap() {
                                                "profileAttributes"=>json!({"data":{"me":{"profile":{"uri":"spotify:user:test","username":"test","name":"Test"}}}}),
                                                "accountAttributes"=>json!({"data":{"me":{"account":{"product":"FREE","country":"LK"}}}}),
                                                "searchTracks"=>json!({"data":{"searchV2":{"tracksV2":{"items":[{"item":{"data":{"uri":TRACK,"name":"Song"}}}],"totalCount":1}}}}),
                                                "home"=>json!({"data":{"home":{"greeting":{"text":"Hello"},"homeChips":[],"sectionContainer":{"sections":{"items":[],"totalCount":0}}}}}),
                                                "browseAll"=>json!({"data":{"browseStart":{"sections":{"items":[]}}}}),
                                                "browsePage"=>json!({"data":{"browse":{"header":{"title":{"text":"Browse"}},"sections":{"items":[],"totalCount":0}}}}),
                                                "libraryV3"=>json!({"data":{"me":{"libraryV3":{"items":[{"pinned":true,"item":{"data":{"uri":TRACK,"name":"Saved"}}}],"totalCount":1}}}}),
                                                "searchSuggestions"=>json!({"data":{"searchV2":{"topResultsV2":{"itemsV2":[{"item":{"__typename":"SearchAutoCompleteEntity","text":"Jazz"}}]}}}}),
                                                "recentSearches"=>json!({"data":{"recentSearches":{"recentSearchesItems":{"items":[{"uri":TRACK,"name":"Recent"}]}}}}),
                                                name if matches!(name,"searchArtists"|"searchAlbums"|"searchPlaylists"|"searchFullEpisodes"|"searchPodcasts"|"searchAudiobooks"|"searchUsers"|"searchGenres")=>{
                                                    let collection=match name {"searchArtists"=>"artists","searchAlbums"=>"albumsV2","searchPlaylists"=>"playlists","searchFullEpisodes"=>"episodes","searchPodcasts"=>"podcasts","searchAudiobooks"=>"audiobooks","searchUsers"=>"users",_=>"genres"};
                                                    json!({"data":{"searchV2":{collection:{"items":[{"uri":TRACK,"name":name}],"totalCount":1}}}})
                                                },
                                                "getTrack"=>json!({"data":{"trackUnion":{"uri":TRACK,"name":"Song"}}}),
                                                "removeFromPlaylist" if params["variables"]["uids"][0]=="fail"=>json!({"data":{"changed":false},"errors":[{"message":"write refused"}]}),
                                                _=>json!({"data":{}}),
                                            }
                                        },
                                        _=>Value::Null,
                                    };
                                    result=json!({"result":{"value":value}});
                                } else if let Some(play)=expression.strip_prefix("window.Myspoti.recover(").and_then(|s|s.strip_suffix(')')) {
                                    log.lock().unwrap().push(("test.recovery".into(),serde_json::from_str(play).unwrap()));
                                } else if failures > 0 {
                                    failures-=1;
                                    result=json!({"exceptionDetails":{"text":"injection failed"}});
                                } else {
                                    ws.send(Message::Text(binding(json!({"type":"ready","version":1}),current_context).to_string().into())).await.unwrap();
                                    result=json!({"result":{"value":null}});
                                }
                            }
                            _=>{}
                        }
                        if ws.send(Message::Text(json!({"id":id,"result":result}).to_string().into())).await.is_err(){break;}
                    }
                }
            }
        });
        Self {
            endpoint,
            calls,
            control: tx,
            protocol,
            response,
            http: http_task,
            socket,
        }
    }
}
