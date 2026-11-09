#![cfg(feature = "web")]
#[allow(dead_code)]
#[path = "../crates/web/tests/common/mod.rs"]
mod common;
use common::*;
use spotcore::{Error, PlaybackMode, ShuffleMode, SpotCore};

#[tokio::test]
async fn facade_routes_controls_and_domains_without_backend_construction() {
    let mock = MockBrowser::start().await;
    let mut sdk = SpotCore::connect(&mock.endpoint).await.unwrap();
    sdk.play(TRACK).await.unwrap();
    sdk.clear_queue().await.unwrap();
    sdk.set_shuffle_mode(ShuffleMode::On).await.unwrap();
    sdk.stop().await.unwrap();
    sdk.library()
        .follow_artist("spotify:artist:test")
        .await
        .unwrap();
    assert!(matches!(
        sdk.library().follow_artist(TRACK).await,
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(sdk.playback_mode(), PlaybackMode::Web);
    assert!(sdk.capabilities().queue_edit);
    assert!(sdk.capabilities().stop);
    assert!(
        sdk.refresh_state()
            .await
            .unwrap()
            .unwrap()
            .capabilities
            .stop
    );
    assert!(sdk.state().unwrap().capabilities.stop);
    let calls = mock.calls.lock().unwrap();
    for method in [
        "player.drmSupport",
        "player.play",
        "queue.clear",
        "player.setShuffleMode",
        "player.pause",
        "library.add",
    ] {
        assert!(
            calls.iter().any(|(name, _)| name == method),
            "missing {method}"
        );
    }
}

#[cfg(feature = "native")]
#[tokio::test]
async fn free_account_never_downloads_soloist_even_when_opted_in() {
    let mock = MockBrowser::start().await;
    let root = std::env::temp_dir().join(format!("spotcore-free-session-{}", std::process::id()));
    let builder = SpotCore::builder()
        .data_dir(&root)
        .connect_to(&mock.endpoint)
        .native("secret-api-key")
        .session_cookie("secret-cookie");
    assert!(!format!("{builder:?}").contains("secret-api-key"));
    assert!(!format!("{builder:?}").contains("secret-cookie"));
    let sdk = builder.launch().await.unwrap();
    assert_eq!(sdk.playback_mode(), PlaybackMode::Web);
    assert!(matches!(
        sdk.native_fallback_reason(),
        Some(Error::AccountRestricted(_))
    ));
    assert!(!root.join("native").exists());
    drop(sdk);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn missing_drm_rejects_play_before_any_playback_write() {
    let mock = MockBrowser::start().await;
    let mut sdk = SpotCore::connect(&mock.endpoint).await.unwrap();
    let mut restricted = state();
    restricted["drmSupport"] = serde_json::json!(false);
    mock.control
        .send(Control::Event(binding(
            serde_json::json!({"type":"state","state":restricted}),
            1,
        )))
        .await
        .unwrap();
    sdk.next_event().await.unwrap();
    assert!(matches!(
        sdk.play(TRACK).await,
        Err(Error::DrmUnavailable(_))
    ));
    assert!(
        !mock
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(method, _)| method == "player.play")
    );
}

#[tokio::test]
async fn service_browser_skips_drm_and_local_playback_but_keeps_domains() {
    let mock = MockBrowser::start().await;
    let mut sdk = SpotCore::builder()
        .connect_to(&mock.endpoint)
        .browser_role(spotcore::BrowserRole::Services)
        .launch()
        .await
        .unwrap();
    sdk.library()
        .follow_artist("spotify:artist:test")
        .await
        .unwrap();
    assert!(!sdk.capabilities().load);
    assert_eq!(sdk.play(TRACK).await, Err(Error::Unsupported));
    let calls = mock.calls.lock().unwrap();
    assert!(calls.iter().any(|(name, _)| name == "library.add"));
    assert!(!calls.iter().any(|(name, _)| matches!(
        name.as_str(),
        "player.drmSupport" | "player.play" | "player.state"
    )));
}

#[tokio::test]
async fn http_facade_needs_no_browser_and_has_no_synthetic_event_loop() {
    let paths = spotcore::Paths::new(
        std::env::temp_dir().join(format!("spotcore-http-facade-{}", std::process::id())),
    );
    let session=serde_json::from_value(serde_json::json!({"accessToken":"dummy","expiresAtMs":u64::MAX,"clientId":"client","appVersion":"version","isAnonymous":false})).unwrap();
    let api = spotcore::Api::from_session(session, &paths).unwrap();
    let mut sdk = SpotCore::builder()
        .data_dir(paths.root())
        .launch_http(api)
        .await
        .unwrap();
    assert!(sdk.host.is_none());
    assert!(sdk.api.browser().is_err());
    assert!(!sdk.capabilities().load);
    assert!(matches!(sdk.next_event().await, Err(Error::Unsupported)));
    sdk.shutdown().await.unwrap();
}

#[cfg(feature = "native")]
#[tokio::test]
#[ignore = "requires a private Free-account cookie file"]
async fn http_free_account_never_downloads_native_or_launches_a_browser() {
    let file = std::env::var("SPOTCORE_FREE_COOKIE_FILE").expect("private cookie file required");
    let cookie = std::fs::read_to_string(file).unwrap();
    let root = std::env::temp_dir().join(format!("spotcore-http-free-{}", std::process::id()));
    let paths = spotcore::Paths::new(&root);
    let api = spotcore::Api::from_cookie(cookie.trim(), &paths).unwrap();
    assert_eq!(
        api.users().me().await.unwrap().tier,
        spotcore::AccountTier::Free
    );
    let sdk = SpotCore::builder()
        .data_dir(&root)
        .native("unused-test-key")
        .launch_http(api)
        .await
        .unwrap();
    assert!(sdk.host.is_none() && sdk.api.browser().is_err());
    assert!(matches!(
        sdk.native_fallback_reason(),
        Some(Error::AccountRestricted(_))
    ));
    assert!(!root.join("native").exists());
    sdk.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(root);
}
