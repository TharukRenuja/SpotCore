mod common;
use common::*;
use serde_json::json;
use spotcore_core::{Backend, Command, Error, Paging, PlaybackEvent};
use spotcore_web::{Api, Browser, SearchKind, WebBackend};
use std::time::Duration;

#[tokio::test]
async fn concurrent_calls_are_correlated_even_when_replies_arrive_out_of_order() {
    let mock = MockBrowser::start().await;
    let browser = Browser::connect(&mock.endpoint).await.unwrap();
    let (a, b) = tokio::join!(
        browser.call("test.reorder", json!({"value":"first"})),
        browser.call("test.reorder", json!({"value":"second"}))
    );
    assert_eq!(a.unwrap(), json!({"value":"first"}));
    assert_eq!(b.unwrap(), json!({"value":"second"}));
}

#[tokio::test]
async fn sdk_uses_the_browser_for_authenticated_search_and_domain_calls() {
    let mock = MockBrowser::start().await;
    let api = Api::connect(&mock.endpoint).await.unwrap();
    assert!(api.is_signed_in().await.unwrap());
    assert!(!format!("{:?}", api.session().await.unwrap()).contains("test-secret"));
    assert_eq!(api.users().me().await.unwrap().username, "test");
    let result = api
        .search()
        .query(SearchKind::Tracks, "A \"quoted\" query", Paging::default())
        .await
        .unwrap();
    assert_eq!(result.items[0].uri, TRACK);
    assert_eq!(result.next_offset, None);
    assert_eq!(api.catalog().track(TRACK).await.unwrap().name, "Song");
    let flags = api
        .library()
        .contains(&[TRACK.into(), "spotify:playlist:list".into(), TRACK.into()])
        .await
        .unwrap();
    assert_eq!(flags, vec![true, false, true]);
    let tracks = vec![TRACK.to_owned(); 201];
    api.playlists()
        .add_tracks("spotify:playlist:list", &tracks)
        .await
        .unwrap();
    let calls = mock.calls.lock().unwrap();
    let chunks: Vec<_> = calls
        .iter()
        .filter(|(name, args)| name == "gql.query" && args["name"] == "addToPlaylist")
        .map(|(_, args)| {
            args["variables"]["playlistItemUris"]
                .as_array()
                .unwrap()
                .len()
        })
        .collect();
    assert_eq!(chunks, vec![100, 100, 1]);
}

#[tokio::test]
async fn mutation_failures_are_returned_without_replaying_the_write() {
    let mock = MockBrowser::start().await;
    let api = Api::connect(&mock.endpoint).await.unwrap();
    assert!(
        api.playlists()
            .remove_tracks("spotify:playlist:list", &["fail".into()])
            .await
            .is_err()
    );
    assert!(matches!(api.jam().start().await, Err(Error::Unsupported)));
    assert!(matches!(
        api.request("GET", "https://example.com/", None).await,
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(
        mock.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, args)| name == "gql.query" && args["name"] == "removeFromPlaylist")
            .count(),
        1
    );
}

#[tokio::test]
async fn web_player_refreshes_restrictions_before_dispatch_and_delivers_events() {
    let mock = MockBrowser::start().await;
    let api = Api::connect(&mock.endpoint).await.unwrap();
    let mut player = WebBackend::new(api).await.unwrap();
    let mut restricted = state();
    restricted["restrictions"] = json!({"canSeek":false,"canSkipNext":false});
    mock.control
        .send(Control::Event(binding(
            json!({"type":"state","state":restricted}),
            1,
        )))
        .await
        .unwrap();
    assert!(matches!(
        player.next_event().await.unwrap(),
        PlaybackEvent::Snapshot(_)
    ));
    assert!(!player.capabilities().seek);
    assert_eq!(
        player.execute(Command::Seek(Duration::ZERO)).await,
        Err(Error::Unsupported)
    );
    assert!(
        mock.calls
            .lock()
            .unwrap()
            .iter()
            .all(|(name, _)| name != "player.seek")
    );
    player.execute(Command::Pause).await.unwrap();
    assert_eq!(
        player.execute(Command::SetVolume(f64::NAN)).await,
        Err(Error::InvalidInput("volume must be 0..=1"))
    );
}

#[tokio::test]
async fn web_only_commands_reach_the_page() {
    let mock = MockBrowser::start().await;
    let api = Api::connect(&mock.endpoint).await.unwrap();
    let mut player = WebBackend::new(api).await.unwrap();
    player
        .execute(Command::SetQuality(spotcore_core::AudioQuality::Lossless))
        .await
        .unwrap();
    assert_eq!(
        player.audio_info().await.unwrap(),
        spotcore_web::AudioInfo {
            bitrate: 256_000,
            mime: "audio/aac".into(),
            level: "very_high".into(),
        }
    );
    player
        .execute(Command::PlayContext {
            uri: TRACK.into(),
            track: None,
            index: Some(2),
            position: None,
        })
        .await
        .unwrap();
    player.execute(Command::SetSpeed(1.25)).await.unwrap();
    player
        .execute(Command::RemoveFromQueue(vec![spotcore_core::QueueItem {
            uri: TRACK.into(),
            uid: "uid".into(),
        }]))
        .await
        .unwrap();
    player.execute(Command::ClearQueue).await.unwrap();
    player
        .execute(Command::SetShuffleMode(spotcore_core::ShuffleMode::On))
        .await
        .unwrap();
    player
        .set_shuffle_mode(spotcore_core::ShuffleMode::Smart)
        .await
        .unwrap();
    let calls = mock.calls.lock().unwrap();
    let call = |name: &str| {
        calls
            .iter()
            .find(|(method, _)| method == name)
            .map(|(_, args)| args.clone())
            .unwrap_or_else(|| panic!("missing {name}"))
    };
    assert_eq!(call("player.setAudioQuality"), json!({"level":"very_high"}));
    assert_eq!(call("player.play")["index"], 2);
    assert_eq!(call("player.setSpeed"), json!({"speed":1.25}));
    assert_eq!(call("player.setShuffleMode"), json!({"mode":1}));
    assert!(
        calls
            .iter()
            .any(|(method, args)| method == "player.setShuffleMode" && args == &json!({"mode":2}))
    );
    assert_eq!(call("queue.remove")["items"][0]["uid"], "uid");
    assert!(calls.iter().any(|(method, _)| method == "queue.clear"));
}

#[tokio::test]
async fn reload_invalidates_pending_calls_and_reattaches_without_replaying() {
    let mock = MockBrowser::start().await;
    let browser = Browser::connect(&mock.endpoint).await.unwrap();
    let first_epoch = browser.session_epoch();
    let pending = browser.call("test.pending", json!({"write":true}));
    tokio::pin!(pending);
    tokio::select! {_=&mut pending=>panic!("request should be pending"),_=tokio::time::sleep(Duration::from_millis(30))=>{}}
    mock.control.send(Control::Reload).await.unwrap();
    assert_eq!(pending.await, Err(Error::Disconnected));
    browser.await_ready().await.unwrap();
    assert!(browser.session_epoch() > first_epoch);
    assert_eq!(
        browser
            .call("test.echo", json!({"after":"reload"}))
            .await
            .unwrap(),
        json!({"after":"reload"})
    );
    assert_eq!(
        mock.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(method, _)| method == "test.pending")
            .count(),
        1
    );
}

#[tokio::test]
async fn socket_close_fails_pending_calls() {
    let mock = MockBrowser::start().await;
    let browser = Browser::connect(&mock.endpoint).await.unwrap();
    let pending = browser.call("test.pending", json!({}));
    tokio::pin!(pending);
    tokio::select! {_=&mut pending=>panic!("request should be pending"),_=tokio::time::sleep(Duration::from_millis(20))=>{}}
    mock.control.send(Control::Close).await.unwrap();
    assert_eq!(pending.await, Err(Error::Disconnected));
}

#[tokio::test]
async fn minimal_startup_waits_for_the_reloaded_execution_context() {
    let mock = MockBrowser::start().await;
    let browser =
        Browser::connect_with_options(&mock.endpoint, spotcore_web::WebPlayerOptions::default())
            .await
            .unwrap();
    assert!(browser.is_ready());
    assert!(browser.session_epoch() >= 2);
    assert_eq!(
        browser
            .call("test.echo", json!({"newContext":true}))
            .await
            .unwrap(),
        json!({"newContext":true})
    );
}

#[tokio::test]
async fn failed_minimal_bridge_retries_normal_startup_once() {
    let mock = MockBrowser::fail_injections(2).await;
    let browser =
        Browser::connect_with_options(&mock.endpoint, spotcore_web::WebPlayerOptions::default())
            .await
            .unwrap();
    assert!(browser.is_ready());
    assert_eq!(
        mock.protocol
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == "Page.reload")
            .count(),
        2
    );
}

async fn protocol_call(mock: &MockBrowser, name: &str) -> serde_json::Value {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some((_, args)) = mock
                .protocol
                .lock()
                .unwrap()
                .iter()
                .find(|(m, _)| m == name)
                .cloned()
            {
                return args;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn minimal_bootstrap_bypasses_service_worker_and_keeps_control_socket_alive() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let mock = MockBrowser::start().await;
    *mock.response.lock().unwrap() = json!({"body":format!("{}let o=await TE(),l=o.getFeatureFlags(),u=o.getRegistry();"," ".repeat(1024*1024)),"base64Encoded":false});
    let browser =
        Browser::connect_with_options(&mock.endpoint, spotcore_web::WebPlayerOptions::default())
            .await
            .unwrap();
    let request = protocol_call(&mock, "Fetch.fulfillRequest").await;
    let source =
        String::from_utf8(STANDARD.decode(request["body"].as_str().unwrap()).unwrap()).unwrap();
    assert!(source.contains("window.__spotcorePlatform=o;return;"));
    {
        let protocol = mock.protocol.lock().unwrap();
        let bypass = protocol
            .iter()
            .position(|(name, args)| {
                name == "Network.setBypassServiceWorker" && args["bypass"] == true
            })
            .unwrap();
        let reload = protocol
            .iter()
            .position(|(name, _)| name == "Page.reload")
            .unwrap();
        assert!(bypass < reload);
    }
    assert_eq!(
        browser
            .call("test.echo", json!({"alive":true}))
            .await
            .unwrap(),
        json!({"alive":true})
    );
}

#[tokio::test]
async fn local_play_selects_web_device_but_remote_play_does_not() {
    let mock = MockBrowser::start().await;
    let api = Api::connect(&mock.endpoint).await.unwrap();
    let mut player = WebBackend::new(api).await.unwrap();
    player.execute(Command::Load(TRACK.into())).await.unwrap();
    player.execute(Command::Resume).await.unwrap();
    player.play_context(TRACK, None, None, None).await.unwrap();
    player
        .execute_remote(Command::PlayContext {
            uri: TRACK.into(),
            track: None,
            index: None,
            position: None,
        })
        .await
        .unwrap();
    let calls = mock.calls.lock().unwrap();
    let relevant: Vec<_> = calls
        .iter()
        .filter_map(|(method, args)| {
            if method == "service.call" && args["method"] == "pullToLocalDevice" {
                Some("transfer")
            } else if method == "player.play" || method == "player.resume" {
                Some(method.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        relevant,
        [
            "transfer",
            "player.play",
            "transfer",
            "player.resume",
            "transfer",
            "player.play",
            "player.play"
        ]
    );
}

#[tokio::test]
async fn queue_edit_commands_reach_the_page() {
    let mock = MockBrowser::start().await;
    let api = Api::connect(&mock.endpoint).await.unwrap();
    let mut player = WebBackend::new(api).await.unwrap();
    let item = |uid: &str| spotcore_core::QueueItem {
        uri: TRACK.into(),
        uid: uid.into(),
    };
    player
        .execute(Command::InsertIntoQueue {
            uris: vec![TRACK.into()],
            before: Some(item("next")),
        })
        .await
        .unwrap();
    player
        .execute(Command::ReorderQueue {
            items: vec![item("moved")],
            before: None,
            after: Some(item("anchor")),
        })
        .await
        .unwrap();
    player
        .execute(Command::SkipToQueue(item("target")))
        .await
        .unwrap();
    assert!(matches!(
        player
            .execute(Command::ReorderQueue {
                items: vec![item("moved")],
                before: None,
                after: None,
            })
            .await,
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        player
            .execute(Command::InsertIntoQueue {
                uris: vec![],
                before: None,
            })
            .await,
        Err(Error::InvalidInput(_))
    ));
    let calls = mock.calls.lock().unwrap();
    let call = |name: &str| {
        calls
            .iter()
            .find(|(method, _)| method == name)
            .map(|(_, args)| args.clone())
            .unwrap_or_else(|| panic!("missing {name}"))
    };
    assert_eq!(
        call("queue.insert"),
        json!({"uris":[TRACK],"before":{"uri":TRACK,"uid":"next"}})
    );
    assert_eq!(
        call("queue.reorder"),
        json!({"items":[{"uri":TRACK,"uid":"moved"}],"after":{"uri":TRACK,"uid":"anchor"}})
    );
    assert_eq!(call("queue.skipTo"), json!({"uri":TRACK,"uid":"target"}));
}

#[tokio::test]
async fn typed_browsing_uses_existing_queries_and_preserves_request_parameters() {
    let mock = MockBrowser::start().await;
    let api = Api::connect(&mock.endpoint).await.unwrap();
    let p = spotcore_core::Paging {
        offset: 3,
        limit: 2,
    };
    assert_eq!(
        api.home()
            .feed_info("UTC", Some("MUSIC"), 2)
            .await
            .unwrap()
            .greeting
            .as_deref(),
        Some("Hello")
    );
    assert!(api.home().browse_categories().await.unwrap().is_empty());
    assert_eq!(
        api.home()
            .browse_page("spotify:genre:jazz", p, p)
            .await
            .unwrap()
            .title
            .as_deref(),
        Some("Browse")
    );
    let rows = api
        .library()
        .items_detailed(None, "Recently Added", None, "Jazz", p)
        .await
        .unwrap();
    assert_eq!(rows.items[0].pinned, Some(true));
    assert_eq!(
        api.search().suggestion_texts("Ja", 2).await.unwrap(),
        ["Jazz"]
    );
    assert_eq!(api.search().recent_items().await.unwrap()[0].name, "Recent");
    let all = api.search().all("Jazz", 2).await.unwrap();
    assert_eq!(all.tracks.items.len(), 1);
    assert_eq!(all.artists.items[0].name, "searchArtists");
    assert_eq!(all.genres.items[0].name, "searchGenres");
    let calls = mock.calls.lock().unwrap();
    let vars = |name: &str| {
        calls
            .iter()
            .find(|(method, args)| method == "gql.query" && args["name"] == name)
            .unwrap()
            .1["variables"]
            .clone()
    };
    assert_eq!(vars("home")["facet"], "MUSIC");
    assert_eq!(vars("libraryV3")["textFilter"], "Jazz");
    assert_eq!(
        vars("browsePage")["pagePagination"],
        serde_json::json!({"offset":3,"limit":2})
    );
    assert_eq!(
        vars("browsePage")["sectionPagination"],
        serde_json::json!({"offset":3,"limit":2})
    );
}
