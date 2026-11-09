use serde_json::Value;
use spotcore_core::{
    Capabilities, Error, Image, MediaItem, MediaKind, Page, Paging, PlaybackState, PlayerSnapshot,
    Queue, RepeatMode,
};

fn text<'a>(value: &'a Value, paths: &[&str]) -> Option<&'a str> {
    paths
        .iter()
        .find_map(|p| value.pointer(p).and_then(Value::as_str))
}

pub(crate) fn item(row: &Value) -> Option<MediaItem> {
    let mut value = row;
    for _ in 0..5 {
        if let Some(data) = value.get("data").filter(|data| data.is_object()) {
            value = data;
            continue;
        }
        if let Some(release) = value
            .pointer("/releases/items/0")
            .filter(|release| release.is_object())
        {
            value = release;
            continue;
        }
        if value.get("uri").is_some()
            || (value.get("_uri").is_some() && value.get("data").is_none())
        {
            break;
        }
        let Some(inner) = [
            "/releases/items/0",
            "/itemV2/data",
            "/item/data",
            "/track/data",
            "/entity/data",
            "/content/data",
            "/data",
            "/track",
            "/item",
        ]
        .iter()
        .find_map(|p| value.pointer(p).filter(|v| v.is_object())) else {
            break;
        };
        value = inner;
    }
    let uri =
        text(value, &["/uri", "/_uri"]).or_else(|| text(row, &["/track/_uri", "/_uri", "/uri"]))?;
    let kind = match uri.split(':').nth(1) {
        Some("track") => MediaKind::Track,
        Some("episode") => MediaKind::Episode,
        Some("album") => MediaKind::Album,
        Some("artist") => MediaKind::Artist,
        Some("playlist") => MediaKind::Playlist,
        Some("show") => MediaKind::Show,
        Some("ad") => MediaKind::Ad,
        Some("user") if uri.contains(":folder:") => MediaKind::Folder,
        Some("user") => MediaKind::User,
        _ => MediaKind::Unknown,
    };
    let mut images = Vec::new();
    for path in [
        "/images",
        "/coverArt/sources",
        "/visuals/avatarImage/sources",
        "/avatar/sources",
        "/images/items/0/sources",
        "/album/images",
        "/albumOfTrack/coverArt/sources",
        "/image/sources",
    ] {
        if let Some(values) = value.pointer(path).and_then(Value::as_array) {
            for image in values {
                if let Some(url) = image["url"].as_str() {
                    images.push(Image {
                        url: url.into(),
                        width: image["width"].as_u64().and_then(|n| n.try_into().ok()),
                        height: image["height"].as_u64().and_then(|n| n.try_into().ok()),
                    });
                }
            }
        }
    }
    if images.is_empty()
        && let Some(url) = text(value, &["/image", "/image_url"])
    {
        images.push(Image {
            url: url.into(),
            width: None,
            height: None,
        });
    }
    Some(MediaItem {
        uri: uri.into(),
        name: text(
            value,
            &[
                "/name",
                "/title/transformedLabel",
                "/title",
                "/profile/name",
                "/displayName",
            ],
        )
        .unwrap_or("")
        .into(),
        kind: if value["kind"] == "ad" {
            MediaKind::Ad
        } else {
            kind
        },
        images,
        duration_ms: [
            "/durationMs",
            "/duration/totalMilliseconds",
            "/duration/milliseconds",
            "/duration_ms",
        ]
        .iter()
        .find_map(|p| value.pointer(p).and_then(Value::as_u64)),
        uid: text(row, &["/uid"])
            .or_else(|| text(value, &["/uid"]))
            .map(str::to_owned),
        raw: value.clone(),
    })
}

pub(crate) fn entity(value: &Value, pointer: &str) -> Result<MediaItem, Error> {
    let node = value
        .pointer(pointer)
        .ok_or_else(|| Error::Backend(format!("missing API entity {pointer}")))?;
    if restricted(node) {
        return Err(unavailable());
    }
    item(node).ok_or_else(|| Error::Backend("entity is unavailable or not found".into()))
}

fn restricted(node: &Value) -> bool {
    node["__typename"] == "RestrictedContent"
}

fn unavailable() -> Error {
    Error::Backend("content is unavailable in this market or region".into())
}

// Spotify returns RestrictedContent for shows and episodes blocked by market, so a
// missing page under one is a regional restriction rather than a protocol failure.
fn restricted_ancestor(value: &Value, pointer: &str) -> bool {
    let mut prefix = String::new();
    for part in pointer.split('/').filter(|part| !part.is_empty()) {
        prefix.push('/');
        prefix.push_str(part);
        if value.pointer(&prefix).is_some_and(restricted) {
            return true;
        }
    }
    false
}

pub(crate) fn page(body: &Value, pointer: &str, paging: Paging) -> Result<Page<MediaItem>, Error> {
    page_map(body, pointer, paging, item)
}

pub(crate) fn page_map<T>(
    body: &Value,
    pointer: &str,
    paging: Paging,
    map: impl Fn(&Value) -> Option<T>,
) -> Result<Page<T>, Error> {
    let value = body.pointer(pointer).ok_or_else(|| {
        if restricted_ancestor(body, pointer) {
            unavailable()
        } else {
            Error::Backend(format!("missing API page {pointer}"))
        }
    })?;
    let rows = value["items"]
        .as_array()
        .ok_or_else(|| Error::Backend("API page has no items array".into()))?;
    let total = value["totalCount"].as_u64();
    let end = paging
        .offset
        .checked_add(
            u32::try_from(rows.len())
                .map_err(|_| Error::Backend("API page exceeds offset range".into()))?,
        )
        .ok_or(Error::InvalidInput("page offset overflow"))?;
    let more = !rows.is_empty()
        && total.map_or(rows.len() >= paging.limit as usize, |total| {
            u64::from(end) < total
        });
    Ok(Page {
        items: rows.iter().filter_map(map).collect(),
        offset: paging.offset,
        total,
        next_offset: more.then_some(end),
    })
}

/// Merges the sections of a Home section page, keeping Spotify's order and
/// removing duplicate URIs across sections. Paging counts raw rows so skipped
/// card types still advance the offset.
pub(crate) fn home_section(body: &Value, paging: Paging) -> Result<Page<MediaItem>, Error> {
    let sections = body
        .pointer("/data/homeSections/sections")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            if restricted_ancestor(body, "/data/homeSections/sections") {
                unavailable()
            } else {
                Error::Backend("missing home sections".into())
            }
        })?;
    if sections.is_empty() {
        return Err(Error::Backend("Spotify returned no home sections".into()));
    }
    let mut items = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut rows = 0u32;
    let mut total = Some(0u64);
    for section in sections {
        let rows_value = section["sectionItems"]["items"]
            .as_array()
            .ok_or_else(|| Error::Backend("home section has no items array".into()))?;
        rows = rows
            .checked_add(
                u32::try_from(rows_value.len())
                    .map_err(|_| Error::Backend("home section exceeds offset range".into()))?,
            )
            .ok_or(Error::InvalidInput("page offset overflow"))?;
        total = total.and_then(|sum| {
            section["sectionItems"]["totalCount"]
                .as_u64()
                .map(|count| sum + count)
        });
        for row in rows_value {
            if let Some(item) = item(row)
                && seen.insert(item.uri.clone())
            {
                items.push(item);
            }
        }
    }
    let end = paging
        .offset
        .checked_add(rows)
        .ok_or(Error::InvalidInput("page offset overflow"))?;
    let more = rows > 0 && total.map_or(rows >= paging.limit, |total| u64::from(end) < total);
    Ok(Page {
        items,
        offset: paging.offset,
        total,
        next_offset: more.then_some(end),
    })
}

pub(crate) fn snapshot(value: &Value) -> Result<Option<PlayerSnapshot>, Error> {
    if value.is_null() {
        return Ok(None);
    }
    if !value.is_object() {
        return Err(Error::Backend("invalid player state".into()));
    }
    let restrictions = &value["restrictions"];
    let permitted = |key: &str| restrictions[key].as_bool().unwrap_or(true);
    let capabilities = Capabilities {
        load: true,
        pause: permitted("canPause"),
        resume: permitted("canResume"),
        seek: permitted("canSeek"),
        next: permitted("canSkipNext"),
        previous: permitted("canSkipPrevious"),
        volume: true,
        shuffle: permitted("canToggleShuffle"),
        smart_shuffle: value["smartShuffle"].as_bool().unwrap_or(false)
            || restrictions["canToggleSmartShuffle"]
                .as_bool()
                .unwrap_or(false),
        repeat: permitted("canRepeatContext") && permitted("canRepeatTrack"),
        queue: true,
        queue_edit: true,
        play_context: true,
        speed: match restrictions.get("disallowSettingPlaybackSpeedReasons") {
            None | Some(Value::Null) => true,
            Some(Value::Array(reasons)) => reasons.is_empty(),
            _ => false,
        },
        quality: true,
        ..Default::default()
    };
    Ok(Some(PlayerSnapshot {
        state: if value["isPaused"].as_bool().unwrap_or(true) {
            PlaybackState::Paused
        } else {
            PlaybackState::Playing
        },
        track: item(&value["track"]),
        context_uri: value["contextUri"].as_str().map(str::to_owned),
        position_ms: value["positionMs"].as_u64().unwrap_or(0),
        duration_ms: value["durationMs"].as_u64(),
        timestamp_ms: value["timestamp"].as_u64(),
        device_id: value["deviceId"].as_str().map(str::to_owned),
        device_name: value["deviceName"].as_str().map(str::to_owned),
        volume: value["volume"]
            .as_f64()
            .filter(|v| v.is_finite() && (0.0..=1.0).contains(v)),
        buffering: value["isBuffering"].as_bool().unwrap_or(false),
        shuffle: value["shuffle"].as_bool().unwrap_or(false),
        smart_shuffle: value["smartShuffle"].as_bool(),
        repeat: match value["repeat"].as_u64() {
            Some(1) => RepeatMode::Context,
            Some(2) => RepeatMode::Track,
            _ => RepeatMode::Off,
        },
        capabilities,
        raw: value.clone(),
    }))
}

pub(crate) fn queue(value: &Value) -> Result<Queue, Error> {
    if value.is_null() {
        return Ok(Queue {
            current: None,
            queued: vec![],
            next_up: vec![],
        });
    }
    let rows = |key: &str| {
        value[key]
            .as_array()
            .map(|v| v.iter().filter_map(item).collect())
            .ok_or_else(|| Error::Backend("invalid player queue".into()))
    };
    Ok(Queue {
        current: item(&value["current"]),
        queued: rows("queued")?,
        next_up: rows("nextUp")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn graphql_wrappers_keep_card_identity_names_and_artwork() {
        let card = item(&json!({"uri":"spotify:playlist:p", "data":{
            "uri":"spotify:playlist:p", "name":"Mix", "images":{"items":[{"sources":[{"url":"https://example.com/mix"}]}]}
        }})).unwrap();
        assert_eq!(card.name, "Mix");
        assert_eq!(card.images[0].url, "https://example.com/mix");
        let track = item(
            &json!({"uri":"spotify:track:t", "name":"Song", "albumOfTrack":{
                "coverArt":{"sources":[{"url":"https://example.com/album"}]}
            }}),
        )
        .unwrap();
        assert_eq!(track.images[0].url, "https://example.com/album");
        let release = item(&json!({"uri":"spotify:album:group", "releases":{"items":[{
            "uri":"spotify:album:a", "name":"Release", "coverArt":{"sources":[{"url":"https://example.com/release"}]}
        }]}})).unwrap();
        assert_eq!(release.uri, "spotify:album:a");
        assert_eq!(release.name, "Release");
    }
    #[test]
    fn snapshots_keep_optional_device_and_volume_metadata() {
        let state = snapshot(&json!({"deviceId":"a","deviceName":"Receiver","volume":0.5}))
            .unwrap()
            .unwrap();
        assert_eq!(state.device_id.as_deref(), Some("a"));
        assert_eq!(state.device_name.as_deref(), Some("Receiver"));
        assert_eq!(state.volume, Some(0.5));
        assert_eq!(
            snapshot(&json!({"volume":2.0})).unwrap().unwrap().volume,
            None
        );
        let mut old = serde_json::to_value(state).unwrap();
        for key in ["device_id", "device_name", "volume"] {
            old.as_object_mut().unwrap().remove(key);
        }
        let old: PlayerSnapshot = serde_json::from_value(old).unwrap();
        assert_eq!(old.device_id, None);
        assert_eq!(old.volume, None);
    }

    #[test]
    fn smart_shuffle_state_and_permission_are_separate() {
        let state = snapshot(&json!({"shuffle":true,"smartShuffle":false,"restrictions":{"canToggleSmartShuffle":true}})).unwrap().unwrap();
        assert_eq!(state.smart_shuffle, Some(false));
        assert!(
            state
                .capabilities
                .supports(&spotcore_core::Command::SetShuffleMode(
                    spotcore_core::ShuffleMode::Smart
                ))
        );
        let restricted = snapshot(&json!({"restrictions":{"canToggleSmartShuffle":false}}))
            .unwrap()
            .unwrap();
        assert!(!restricted.capabilities.smart_shuffle);
        assert_eq!(restricted.smart_shuffle, None);
    }
    #[test]
    fn speed_capability_respects_spotify_restrictions() {
        for (reasons, allowed) in [
            (serde_json::json!([]), true),
            (serde_json::json!(["not_supported"]), false),
            (serde_json::json!("malformed"), false),
        ] {
            let state =
                snapshot(&json!({"restrictions":{"disallowSettingPlaybackSpeedReasons":reasons}}))
                    .unwrap()
                    .unwrap();
            assert_eq!(state.capabilities.speed, allowed);
        }
    }

    #[test]
    fn pages_advance_by_raw_rows_including_unknown_entities() {
        let data = json!({"items":[{"item":{"data":{"uri":"spotify:track:id","name":"Song"}}}, {"data":{"__typename":"Unavailable"}}], "totalCount":5});
        let page = page(
            &data,
            "",
            Paging {
                offset: 2,
                limit: 2,
            },
        )
        .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.next_offset, Some(4));
    }
    #[test]
    fn library_uri_and_discography_wrappers_are_preserved() {
        let liked = json!({"track":{"_uri":"spotify:track:id","data":{"name":"Liked"}}});
        let track = item(&liked).unwrap();
        assert_eq!(track.uri, "spotify:track:id");
        assert_eq!(track.name, "Liked");
        let release = json!({"releases":{"items":[{"uri":"spotify:album:id","name":"Album"}]}});
        assert_eq!(item(&release).unwrap().kind, MediaKind::Album);
    }
    #[test]
    fn restricted_content_reports_market_unavailability() {
        let body = json!({"data":{"podcastUnionV2":{"__typename":"RestrictedContent"}}});
        let error = entity(&body, "/data/podcastUnionV2").unwrap_err();
        assert!(matches!(error, Error::Backend(message) if message.contains("market")));
        let error = page(&body, "/data/podcastUnionV2/episodesV2", Paging::default()).unwrap_err();
        assert!(matches!(error, Error::Backend(message) if message.contains("market")));
        let error = page(&json!({"data":{}}), "/data/other", Paging::default()).unwrap_err();
        assert!(matches!(error, Error::Backend(message) if message.contains("missing API page")));
    }
    #[test]
    fn ads_and_restrictions_are_preserved() {
        let value = json!({"track":{"uri":"spotify:ad:id","name":"Ad","kind":"ad"},"restrictions":{"canSeek":false,"canSkipNext":false},"isPaused":false});
        let state = snapshot(&value).unwrap().unwrap();
        assert!(!state.capabilities.seek);
        assert!(!state.capabilities.next);
        assert_eq!(state.track.unwrap().kind, MediaKind::Ad);
    }

    #[test]
    fn web_api_track_cover_and_duration_are_mapped() {
        let row = json!({"uri":"spotify:track:id","name":"Song","duration_ms":1234,"album":{"images":[{"url":"https://i.scdn.co/image/a","width":640,"height":640}]}});
        let item = item(&row).unwrap();
        assert_eq!(item.duration_ms, Some(1234));
        assert_eq!(item.images[0].url, "https://i.scdn.co/image/a");
        assert_eq!(item.images[0].width, Some(640));
    }

    #[test]
    fn home_sections_merge_and_deduplicate() {
        let body = json!({"data":{"homeSections":{"sections":[
            {"sectionItems":{"items":[
                {"content":{"data":{"__typename":"Playlist","uri":"spotify:playlist:1","name":"First"}}},
                {"content":{"data":{"__typename":"NotFound"}}}
            ],"totalCount":2}},
            {"sectionItems":{"items":[
                {"content":{"data":{"__typename":"Playlist","uri":"spotify:playlist:1","name":"First again"}}},
                {"content":{"data":{"__typename":"Playlist","uri":"spotify:playlist:2","name":"Second"}}}
            ],"totalCount":2}}
        ]}}});
        let page = home_section(
            &body,
            Paging {
                offset: 0,
                limit: 4,
            },
        )
        .unwrap();
        assert_eq!(
            page.items
                .iter()
                .map(|item| item.uri.as_str())
                .collect::<Vec<_>>(),
            ["spotify:playlist:1", "spotify:playlist:2"]
        );
        assert_eq!(page.total, Some(4));
        assert_eq!(page.next_offset, None);

        let mut larger = body.clone();
        larger["data"]["homeSections"]["sections"][1]["sectionItems"]["totalCount"] = json!(4);
        let more = home_section(
            &larger,
            Paging {
                offset: 0,
                limit: 1,
            },
        )
        .unwrap();
        assert_eq!(more.items.len(), 2);
        assert_eq!(more.total, Some(6));
        assert_eq!(more.next_offset, Some(4));
    }

    #[test]
    fn missing_home_sections_are_not_empty_successes() {
        assert!(home_section(&json!({"data":{}}), Paging::default()).is_err());
        assert!(
            home_section(
                &json!({"data":{"homeSections":{"sections":[]}}}),
                Paging::default()
            )
            .is_err()
        );
    }
}
