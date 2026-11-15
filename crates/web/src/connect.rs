use crate::{Api, direct};
use serde_json::{Value, json};
use spotcore_core::{Capabilities, Error};
fn denied(state: &Value, action: &str) -> bool {
    match state["restrictions"].get(format!("disallow_{action}_reasons")) {
        None | Some(Value::Null) => false,
        Some(Value::Array(reasons)) => !reasons.is_empty(),
        _ => true,
    }
}
fn is_ad(value: &Value) -> bool {
    value["provider"] == "ad"
        || value["uri"]
            .as_str()
            .is_some_and(|uri| uri.starts_with("spotify:ad:"))
        || value["metadata"]["is_advertisement"] == "true"
        || value["metadata"]["is_advertisement"] == true
        || value["metadata"]["ad_id"].is_string()
}
pub(crate) fn track(value: &Value) -> Value {
    if value.is_null() {
        return Value::Null;
    }
    let metadata = &value["metadata"];
    let duration = metadata
        .get("duration")
        .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()));
    let images = metadata
        .get("image_url")
        .and_then(Value::as_str)
        .map(|url| json!([{"url":crate::mapping::artwork_url(url)}]))
        .unwrap_or_else(|| json!([]));
    json!({"uri":value["uri"],"uid":value["uid"],"name":metadata["title"],"durationMs":duration,
        "kind":if is_ad(value){"ad"}else{"track"},"metadata":metadata,
        "artists":[{"uri":metadata["artist_uri"],"name":metadata["artist_name"]}],
        "album":{"uri":metadata["album_uri"],"name":metadata["album_title"],"images":images}})
}
fn queued(value: &Value) -> bool {
    !is_ad(value)
        && (value["metadata"]["is_queued"] == "true" || value["metadata"]["is_queued"] == true)
}

fn queue_view(state: &Value) -> Result<Value, Error> {
    let next = rows(state, "next_tracks")?;
    // Connect includes hidden future context iterations that the Web player's queue omits.
    let visible = || {
        next.iter().filter(|row| {
            row["uri"] != "spotify:delimiter"
                && row["metadata"]["hidden"] != "true"
                && row["metadata"]["hidden"] != true
        })
    };
    Ok(
        json!({"contextUri":state["context_uri"],"current":track(&state["track"]),
        "queued":visible().filter(|row| queued(row)).map(track).collect::<Vec<_>>(),
        "nextUp":visible().filter(|row| !queued(row)).map(track).collect::<Vec<_>>()}),
    )
}
fn rows(state: &Value, key: &str) -> Result<Vec<Value>, Error> {
    state[key].as_array().cloned().ok_or_else(|| {
        Error::Backend(format!(
            "incomplete Connect {key}; refusing queue replacement"
        ))
    })
}
pub(crate) fn capabilities(cluster: &Value) -> Capabilities {
    if cluster["active_device_id"]
        .as_str()
        .is_none_or(str::is_empty)
    {
        return Capabilities::default();
    }
    let state = &cluster["player_state"];
    Capabilities {
        load: !denied(state, "playing"),
        pause: !denied(state, "pausing"),
        resume: !denied(state, "resuming"),
        seek: !denied(state, "seeking"),
        next: !denied(state, "skipping_next"),
        previous: !denied(state, "skipping_prev"),
        shuffle: !denied(state, "toggling_shuffle"),
        smart_shuffle: !denied(state, "toggling_shuffle"),
        repeat: !denied(state, "toggling_repeat_context")
            && !denied(state, "toggling_repeat_track"),
        queue: true,
        queue_edit: state["next_tracks"].is_array()
            && state["prev_tracks"].is_array()
            && !denied(state, "setting_queue"),
        play_context: !denied(state, "playing"),
        speed: !denied(state, "setting_playback_speed"),
        volume: supports_volume(
            &cluster["devices"][cluster["active_device_id"].as_str().unwrap_or_default()],
        ) && !denied(state, "setting_volume"),
        ..Default::default()
    }
}
fn number(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}
fn duration(state: &Value) -> Option<u64> {
    number(&state["duration"])
        .or_else(|| number(&state["track"]["metadata"]["duration"]))
        .filter(|duration| *duration > 0)
}

fn position_at(state: &Value, now: u64) -> u64 {
    let position = number(&state["position_as_of_timestamp"]).unwrap_or(0);
    let elapsed = if state["is_paused"] == false && state["is_buffering"] != true {
        number(&state["timestamp"])
            .filter(|timestamp| *timestamp > 0)
            .map_or(0, |timestamp| now.saturating_sub(timestamp))
    } else {
        0
    };
    let speed = state["playback_speed"]
        .as_f64()
        .filter(|speed| speed.is_finite() && *speed > 0.0)
        .unwrap_or(1.0);
    let target = position.saturating_add((elapsed as f64 * speed).floor() as u64);
    duration(state).map_or(target, |duration| target.min(duration))
}

pub(crate) fn snapshot(cluster: &Value) -> Value {
    if cluster["active_device_id"]
        .as_str()
        .is_none_or(str::is_empty)
    {
        return Value::Null;
    }
    let state = &cluster["player_state"];
    let smart = state["options"]["modes"]["context_enhancement"] == "RECOMMENDATION";
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64;
    let id = cluster["active_device_id"].as_str().unwrap_or_default();
    let device = &cluster["devices"][id];
    let volume = number(&device["volume"])
        .filter(|_| supports_volume(device))
        .map(|v| (v as f64 / 65535.0).clamp(0.0, 1.0));
    json!({"deviceId":id,"deviceName":device["name"],"volume":volume,"track":track(&state["track"]),"contextUri":state["context_uri"],"positionMs":position_at(state, now),"timestamp":now,"durationMs":duration(state),"speed":state["playback_speed"],
        "isPaused":state["is_paused"].as_bool().unwrap_or(true),"isBuffering":state["is_buffering"],"shuffle":state["options"]["shuffling_context"],"smartShuffle":smart,
        "repeat":if state["options"]["repeating_track"]==true{2}else if state["options"]["repeating_context"]==true{1}else{0}})
}
fn queue_position(rows: &[Value], wanted: &Value) -> Result<usize, Error> {
    let uid = wanted["uid"]
        .as_str()
        .filter(|uid| !uid.is_empty())
        .ok_or(Error::InvalidInput("queue editing requires exact UIDs"))?;
    let matches: Vec<_> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            !is_ad(row)
                && row["uri"] != "spotify:delimiter"
                && row["metadata"]["hidden"] != "true"
                && row["metadata"]["hidden"] != true
                && row["uid"] == uid
                && row["uri"] == wanted["uri"]
        })
        .map(|(index, _)| index)
        .collect();
    if matches.len() != 1 {
        return Err(Error::InvalidInput(
            "queue row changed; refresh before editing",
        ));
    }
    Ok(matches[0])
}

fn new_queue_rows(args: &Value) -> Result<Vec<Value>, Error> {
    let uris = args["uris"]
        .as_array()
        .filter(|uris| uris.len() <= 500)
        .ok_or(Error::InvalidInput("queue edit accepts at most 500 URIs"))?;
    uris.iter().map(|value| {
        let uri = value.as_str().ok_or(Error::InvalidInput("invalid queue URI"))?;
        if !uri.starts_with("spotify:track:") && !uri.starts_with("spotify:episode:") {
            return Err(Error::InvalidInput("only tracks and episodes can be queued"));
        }
        crate::domains::uri(uri)?;
        Ok(json!({"uri":uri,"uid":uuid::Uuid::new_v4().simple().to_string(),"provider":"queue","metadata":{"is_queued":"true"}}))
    }).collect()
}

fn edit_queue(method: &str, args: &Value, state: &Value) -> Result<Value, Error> {
    let mut next = rows(state, "next_tracks")?;
    let previous = rows(state, "prev_tracks")?;
    match method {
        "queue.insert" => {
            let mut insert = new_queue_rows(args)?;
            if insert.is_empty() {
                return Err(Error::InvalidInput("no queue URIs"));
            }
            if args["queued"] == false {
                for row in &mut insert {
                    row["provider"] = json!("context");
                    row["metadata"]["is_queued"] = json!("false");
                }
            }
            if !args["before"].is_null() && !args["after"].is_null() {
                return Err(Error::InvalidInput("queue insertion accepts one anchor"));
            }
            let at = if !args["after"].is_null() {
                queue_position(&next, &args["after"])? + 1
            } else if args["before"].is_null() {
                next.iter().position(queued).unwrap_or_else(|| {
                    next.iter()
                        .position(|row| !is_ad(row))
                        .unwrap_or(next.len())
                })
            } else {
                queue_position(&next, &args["before"])?
            };
            next.splice(at..at, insert);
        }
        "queue.reorder" => {
            let selected = args["items"]
                .as_array()
                .filter(|items| !items.is_empty())
                .ok_or(Error::InvalidInput("no queue rows selected"))?;
            let mut positions = std::collections::BTreeSet::new();
            let mut moved = Vec::new();
            for item in selected {
                let at = queue_position(&next, item)?;
                if !positions.insert(at) {
                    return Err(Error::InvalidInput("queue row selected twice"));
                }
                let mut row = next[at].clone();
                if let Some(manual) = args["queued"]
                    .as_bool()
                    .filter(|manual| *manual != queued(&row))
                {
                    row["provider"] = json!(if manual { "queue" } else { "context" });
                    if !row["metadata"].is_object() {
                        row["metadata"] = json!({});
                    }
                    row["metadata"]["is_queued"] = json!(if manual { "true" } else { "false" });
                }
                moved.push(row);
            }
            let before = !args["before"].is_null();
            let after = !args["after"].is_null();
            if !before && !after && args["queued"].is_boolean() {
                for row in moved {
                    let at = queue_position(&next, &row)?;
                    next[at] = row;
                }
            } else {
                if before == after {
                    return Err(Error::InvalidInput("queue reorder needs one anchor"));
                }
                let anchor = if before {
                    &args["before"]
                } else {
                    &args["after"]
                };
                if positions.contains(&queue_position(&next, anchor)?) {
                    return Err(Error::InvalidInput(
                        "queue anchor is part of the moved selection",
                    ));
                }
                for at in positions.into_iter().rev() {
                    next.remove(at);
                }
                let at = queue_position(&next, anchor)? + usize::from(after);
                next.splice(at..at, moved);
            }
        }
        "queue.replace" => {
            let mut replacement = new_queue_rows(args)?;
            let mut existing: Vec<_> = next.iter().filter(|row| queued(row)).cloned().collect();
            for row in &mut replacement {
                if let Some(at) = existing.iter().position(|old| old["uri"] == row["uri"]) {
                    *row = existing.remove(at);
                }
            }
            let at = next.iter().position(queued).unwrap_or_else(|| {
                next.iter()
                    .position(|row| !is_ad(row))
                    .unwrap_or(next.len())
            });
            next.retain(|row| !queued(row));
            next.splice(at..at, replacement);
        }
        _ => return Err(Error::Unsupported),
    }
    Ok(json!({"next_tracks":next,"prev_tracks":previous}))
}

fn command(method: &str, args: &Value, state: &Value) -> Result<(Value, &'static str), Error> {
    let mut value = json!({});
    let (endpoint, restriction) = match method {
        "player.pause" => ("pause", "pausing"),
        "player.resume" => ("resume", "resuming"),
        "player.next" => ("skip_next", "skipping_next"),
        "queue.skipTo" => {
            let uri = args["uri"]
                .as_str()
                .ok_or(Error::InvalidInput("missing queue URI"))?;
            crate::domains::uri(uri)?;
            let uid = args["uid"]
                .as_str()
                .filter(|uid| !uid.is_empty())
                .ok_or(Error::InvalidInput("queue item has no uid"))?;
            let next = rows(state, "next_tracks")?;
            if !next.iter().any(|row| {
                row["uri"] == uri
                    && row["uid"] == uid
                    && row["metadata"]["hidden"] != "true"
                    && row["metadata"]["hidden"] != true
            }) {
                return Err(Error::InvalidInput("queue occurrence is no longer present"));
            }
            value = json!({"track":{"uri":uri,"uid":uid}});
            ("skip_next", "skipping_next")
        }
        "player.previous" => ("skip_prev", "skipping_prev"),
        "player.seek" => {
            let ms = args["positionMs"]
                .as_u64()
                .filter(|ms| *ms <= 9_007_199_254_740_991)
                .ok_or(Error::InvalidInput("invalid seek position"))?;
            value["value"] = json!(ms);
            ("seek_to", "seeking")
        }
        "player.setShuffle" => {
            value["value"] = json!(
                args["on"]
                    .as_bool()
                    .ok_or(Error::InvalidInput("missing shuffle flag"))?
            );
            ("set_shuffling_context", "toggling_shuffle")
        }
        "player.setShuffleMode" => {
            let mode = args["mode"]
                .as_u64()
                .filter(|m| *m <= 2)
                .ok_or(Error::InvalidInput("invalid shuffle mode"))?;
            value = json!({"shuffling_context":mode!=0,"modes":{"context_enhancement":if mode==2{"RECOMMENDATION"}else{"NONE"}}});
            ("set_options", "toggling_shuffle")
        }
        "player.setRepeat" => {
            let mode = args["mode"]
                .as_u64()
                .filter(|m| *m <= 2)
                .ok_or(Error::InvalidInput("invalid repeat mode"))?;
            value = json!({"repeating_context":mode==1,"repeating_track":mode==2});
            if denied(state, "toggling_repeat_track") {
                return Err(Error::Unsupported);
            }
            ("set_options", "toggling_repeat_context")
        }
        "player.setSpeed" => {
            let speed = args["speed"]
                .as_f64()
                .filter(|speed| speed.is_finite() && (0.5..=3.5).contains(speed))
                .ok_or(Error::InvalidInput("invalid playback speed"))?;
            value = json!({"playback_speed":speed});
            ("set_options", "setting_playback_speed")
        }
        "player.play" => {
            let uri = args["contextUri"]
                .as_str()
                .ok_or(Error::InvalidInput("missing context URI"))?;
            crate::domains::uri(uri)?;
            let mut options = json!({});
            if let Some(uri) = args["trackUri"].as_str() {
                crate::domains::uri(uri)?;
                options["skip_to"] = json!({"track_uri":uri});
            } else if let Some(index) = args["index"].as_u64() {
                options["skip_to"] = json!({"track_index":index});
            }
            if let Some(ms) = args["positionMs"].as_u64() {
                if ms > 9_007_199_254_740_991 {
                    return Err(Error::InvalidInput("invalid play position"));
                }
                options["seek_to"] = json!(ms);
            }
            let mut context = json!({"uri":uri,"url":format!("context://{uri}"),"metadata":{}});
            if let Some(order) = args.get("order") {
                let order = order
                    .as_array()
                    .filter(|items| !items.is_empty() && items.len() <= 1000)
                    .ok_or(Error::InvalidInput("invalid ordered playback list"))?;
                let tracks = order
                    .iter()
                    .map(|item| {
                        let item = item
                            .as_str()
                            .ok_or(Error::InvalidInput("invalid playback URI"))?;
                        crate::domains::uri(item)?;
                        Ok(json!({"uri":item}))
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                context["pages"] = json!([{"tracks":tracks}]);
                context["url"] = json!(format!(
                    "context://{uri}?order={}",
                    uuid::Uuid::new_v4().simple()
                ));
            }
            value = json!({"context":context,"options":options});
            ("play", "playing")
        }
        "queue.add" => {
            let uris = args["uris"]
                .as_array()
                .ok_or(Error::InvalidInput("missing queue URIs"))?;
            if uris.len() != 1 {
                return Err(Error::Unsupported);
            }
            let uri = uris[0]
                .as_str()
                .ok_or(Error::InvalidInput("invalid queue URI"))?;
            crate::domains::uri(uri)?;
            value = json!({"track":{"uri":uri,"provider":"queue","metadata":{"is_queued":"true"}}});
            ("add_to_queue", "inserting_into_next_tracks")
        }
        "queue.insert" | "queue.reorder" | "queue.replace" => {
            value = edit_queue(method, args, state)?;
            ("set_queue", "setting_queue")
        }
        "queue.clear" | "queue.remove" => {
            let mut next = rows(state, "next_tracks")?;
            let previous = rows(state, "prev_tracks")?;
            if method == "queue.clear" {
                next.retain(|item| !queued(item));
            } else {
                let remove = args["items"]
                    .as_array()
                    .ok_or(Error::InvalidInput("missing queue items"))?;
                if remove
                    .iter()
                    .any(|item| item["uid"].as_str().is_none_or(str::is_empty))
                {
                    return Err(Error::InvalidInput("queue removal requires exact UIDs"));
                }
                for item in remove {
                    queue_position(&next, item)?;
                }
                next.retain(|item| {
                    !remove
                        .iter()
                        .any(|wanted| wanted["uid"] == item["uid"] && wanted["uri"] == item["uri"])
                });
            }
            value = json!({"next_tracks":next,"prev_tracks":previous});
            ("set_queue", "setting_queue")
        }
        _ => return Err(Error::Unsupported),
    };
    if denied(state, restriction) {
        return Err(Error::Unsupported);
    }
    value["endpoint"] = json!(endpoint);
    Ok((value, restriction))
}
pub(crate) async fn call(api: &Api, method: &str, args: Value) -> Result<Value, Error> {
    call_to(api, None, method, args).await
}

fn volume_body(args: &Value) -> Result<Value, Error> {
    let level = args["level"]
        .as_f64()
        .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
        .ok_or(Error::InvalidInput("invalid volume"))?;
    Ok(json!({"volume":(level * 65535.0).round() as u64}))
}

fn supports_volume(info: &Value) -> bool {
    info["capabilities"]["disable_volume"] != true && number(&info["volume"]).is_some()
}

fn target_state<'a>(cluster: &'a Value, device: &str, method: &str) -> Result<&'a Value, Error> {
    let info = cluster["devices"].get(device).ok_or(Error::Disconnected)?;
    if info["is_restricted"] == true || info["capabilities"]["is_restricted"] == true {
        return Err(Error::Unsupported);
    }
    if matches!(method, "volume.set" | "volume.get") && !supports_volume(info) {
        return Err(Error::Unsupported);
    }
    if cluster["active_device_id"] == device {
        return Ok(&cluster["player_state"]);
    }
    if matches!(
        method,
        "queue.clear"
            | "queue.remove"
            | "queue.insert"
            | "queue.reorder"
            | "queue.replace"
            | "player.setShuffleMode"
    ) {
        return Err(Error::Unsupported);
    }
    // Inactive devices have no cluster playback state; the receiver enforces its restrictions.
    Ok(&Value::Null)
}

/// `target` names a Connect device to command without transferring the session.
pub(crate) async fn call_to(
    api: &Api,
    target: Option<&str>,
    method: &str,
    args: Value,
) -> Result<Value, Error> {
    if !matches!(
        method,
        "player.state"
            | "player.queue"
            | "player.pause"
            | "player.resume"
            | "player.next"
            | "player.previous"
            | "player.seek"
            | "player.play"
            | "player.setShuffle"
            | "player.setShuffleMode"
            | "player.setRepeat"
            | "player.setSpeed"
            | "volume.set"
            | "volume.get"
            | "queue.skipTo"
            | "queue.clear"
            | "queue.remove"
            | "queue.add"
            | "queue.insert"
            | "queue.reorder"
            | "queue.replace"
    ) {
        return Err(Error::Unsupported);
    }
    let cluster = direct::cluster(api).await?;
    let state = &cluster["player_state"];
    if target.is_none() {
        if method == "player.state" {
            return Ok(snapshot(&cluster));
        }
        if method == "player.queue" {
            if state.is_null() {
                return Ok(Value::Null);
            }
            return queue_view(state);
        }
    } else if matches!(method, "player.state" | "player.queue") {
        return Err(Error::Unsupported);
    }
    let device = match target {
        Some(device) => device,
        None => cluster["active_device_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or(Error::Disconnected)?,
    };
    if device.is_empty() {
        return Err(Error::InvalidInput("missing Connect device"));
    }
    let state = target_state(&cluster, device, method)?;
    if method == "volume.get" {
        return number(&cluster["devices"][device]["volume"])
            .map(|value| json!((value as f64 / 65535.0).clamp(0.0, 1.0)))
            .ok_or(Error::Unsupported);
    }
    if method == "volume.set" {
        if denied(state, "setting_volume") {
            return Err(Error::Unsupported);
        }
        return api
            .request(
                "PUT",
                &format!(
                    "https://spclient.wg.spotify.com/connect-state/v1/connect/volume/from/{}/to/{}",
                    api.device_id(),
                    crate::domains::segment(device)
                ),
                Some(volume_body(&args)?),
            )
            .await;
    }
    let (mut value, _) = command(method, &args, state)?;
    value["logging_params"] = json!({"command_id":uuid::Uuid::new_v4().to_string()});
    let response = api
        .request_raw(
            "POST",
            &format!(
                "https://spclient.wg.spotify.com/connect-state/v1/player/command/from/{}/to/{}",
                api.device_id(),
                crate::domains::segment(device)
            ),
            Some(
                serde_json::to_vec(&json!({"command":value}))
                    .map_err(|_| Error::InvalidInput("invalid command"))?,
            ),
            "application/json",
        )
        .await?;
    let result = if response.body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice::<Value>(&response.body).map_err(|_| {
            Error::Backend("invalid Connect acknowledgement; command outcome unknown".into())
        })?
    };
    if (response.status == 200 && result["ack_id"].is_string())
        || (response.status == 202 && result["ack_id"].is_null())
    {
        Ok(result)
    } else {
        Err(Error::Backend(
            "Connect command was not acknowledged; outcome unknown".into(),
        ))
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn ordered_playback_uses_context_tracks_without_creating_manual_rows() {
        let state = serde_json::json!({"next_tracks":[{"uri":"spotify:track:manual","uid":"manual","metadata":{"is_queued":"true"}}]});
        let args = serde_json::json!({"contextUri":"spotify:playlist:sorted", "order":["spotify:track:b","spotify:track:a","spotify:track:b"], "index":0, "positionMs":1234});
        let (body, restriction) = super::command("player.play", &args, &state).unwrap();
        assert_eq!(restriction, "playing");
        assert_eq!(body["context"]["uri"], "spotify:playlist:sorted");
        assert_eq!(
            body["context"]["pages"][0]["tracks"],
            serde_json::json!([
                {"uri":"spotify:track:b"},{"uri":"spotify:track:a"},{"uri":"spotify:track:b"}
            ])
        );
        assert_eq!(body["options"]["skip_to"]["track_index"], 0);
        assert_eq!(body["options"]["seek_to"], 1234);
        assert!(body.get("next_tracks").is_none());
        assert_eq!(state["next_tracks"][0]["uid"], "manual");
        for order in [
            serde_json::json!([]),
            serde_json::json!([null]),
            serde_json::json!(["https://example.com"]),
        ] {
            let mut invalid = args.clone();
            invalid["order"] = order;
            assert!(super::command("player.play", &invalid, &state).is_err());
        }
    }

    #[test]
    fn selected_queue_occurrence_is_targeted_in_one_skip() {
        let state = serde_json::json!({"next_tracks":[
            {"uri":"spotify:track:same","uid":"first"},
            {"uri":"spotify:track:same","uid":"second"}
        ]});
        let args = serde_json::json!({"uri":"spotify:track:same","uid":"second"});
        let (body, _) = super::command("queue.skipTo", &args, &state).unwrap();
        assert_eq!(body["endpoint"], "skip_next");
        assert_eq!(body["track"]["uid"], "second");
        assert!(
            super::command(
                "queue.skipTo",
                &serde_json::json!({"uri":"spotify:track:same","uid":"gone"}),
                &state
            )
            .is_err()
        );
    }

    use super::*;

    #[test]
    fn queue_view_hides_context_continuations_without_deduplicating_visible_rows() {
        let state = json!({"context_uri":"spotify:episode:current","track":null,"next_tracks":[
            {"uri":"spotify:episode:next","uid":"next","metadata":{"title":"Next episode"}},
            {"uri":"spotify:delimiter","metadata":{"hidden":"true"}},
            {"uri":"spotify:episode:current","uid":"repeat","metadata":{"hidden":"true","iteration":"1"}},
            {"uri":"spotify:episode:current","uid":"repeat","metadata":{"hidden":true,"iteration":"2"}},
            {"uri":"spotify:episode:manual","uid":"one","metadata":{"is_queued":"true"}},
            {"uri":"spotify:episode:manual","uid":"two","metadata":{"is_queued":"true"}}
        ],"prev_tracks":[]});
        let view = queue_view(&state).unwrap();
        assert_eq!(view["nextUp"].as_array().unwrap().len(), 1);
        assert_eq!(view["nextUp"][0]["uri"], "spotify:episode:next");
        assert_eq!(view["queued"].as_array().unwrap().len(), 2);
        assert_eq!(view["queued"][0]["uid"], "one");
        assert_eq!(view["queued"][1]["uid"], "two");
        let (edited, _) = command("queue.replace", &json!({"uris":[]}), &state).unwrap();
        assert_eq!(
            edited["next_tracks"].as_array().unwrap().len(),
            4,
            "view filtering must not remove continuation state from writes"
        );
    }
    fn edit_state() -> Value {
        json!({"next_tracks":[
            {"uri":"spotify:track:same","uid":"one","provider":"queue","metadata":{"is_queued":"true"}},
            {"uri":"spotify:track:same","uid":"two","provider":"queue","metadata":{"is_queued":"true"}},
            {"uri":"spotify:ad:ad","uid":"ad","provider":"ad"},
            {"uri":"spotify:track:context","uid":"context"}
        ],"prev_tracks":[{"uri":"spotify:track:previous"}]})
    }

    #[test]
    fn connect_image_identifiers_become_fetchable_cover_urls() {
        let id = "a".repeat(40);
        let media = crate::mapping::item(&track(&json!({"uri":"spotify:track:t", "metadata":{"image_url":format!("spotify:image:{id}")}}))).unwrap();
        assert_eq!(media.images[0].url, format!("https://i.scdn.co/image/{id}"));
    }

    #[test]
    fn http_queue_edits_keep_context_ads_and_exact_duplicate_rows() {
        let state = edit_state();
        let (moved, _) = command("queue.reorder", &json!({"items":[{"uri":"spotify:track:same","uid":"two"}],"before":{"uri":"spotify:track:same","uid":"one"}}), &state).unwrap();
        assert_eq!(moved["endpoint"], "set_queue");
        assert_eq!(moved["next_tracks"][0]["uid"], "two");
        assert_eq!(moved["next_tracks"][1]["uid"], "one");
        assert_eq!(
            &moved["next_tracks"].as_array().unwrap()[2..],
            &state["next_tracks"].as_array().unwrap()[2..]
        );
        assert_eq!(moved["prev_tracks"], state["prev_tracks"]);
        let (head, _) = command(
            "queue.insert",
            &json!({"uris":["spotify:track:new"]}),
            &state,
        )
        .unwrap();
        assert_eq!(head["next_tracks"][0]["uri"], "spotify:track:new");
        let (inserted, _) = command("queue.insert", &json!({"uris":["spotify:track:new","spotify:track:new"],"before":{"uri":"spotify:track:same","uid":"two"}}), &state).unwrap();
        assert_eq!(inserted["next_tracks"][0]["uid"], "one");
        assert_eq!(inserted["next_tracks"][3]["uid"], "two");
        assert_ne!(
            inserted["next_tracks"][1]["uid"],
            inserted["next_tracks"][2]["uid"]
        );
        let (replaced, _) = command(
            "queue.replace",
            &json!({"uris":["spotify:track:same","spotify:track:new","spotify:track:same"]}),
            &state,
        )
        .unwrap();
        assert_eq!(replaced["next_tracks"][0]["uid"], "one");
        assert_eq!(replaced["next_tracks"][2]["uid"], "two");
        assert_eq!(replaced["next_tracks"][3], state["next_tracks"][2]);
        assert_eq!(replaced["next_tracks"][4], state["next_tracks"][3]);
    }

    #[test]
    fn http_queue_insertion_targets_context_slots_without_losing_continuation() {
        let mut state = edit_state();
        state["next_tracks"].as_array_mut().unwrap().push(json!({
            "uri":"spotify:track:context", "uid":"hidden", "metadata":{"hidden":"true"}
        }));
        let anchor = json!({"uri":"spotify:track:context", "uid":"context"});
        let inserted = edit_queue(
            "queue.insert",
            &json!({
                "uris":["spotify:track:new", "spotify:track:new"],
                "before":anchor, "queued":false
            }),
            &state,
        )
        .unwrap();
        assert_eq!(inserted["next_tracks"][3]["uri"], "spotify:track:new");
        assert_eq!(inserted["next_tracks"][3]["provider"], "context");
        assert!(!queued(&inserted["next_tracks"][3]));
        assert_ne!(
            inserted["next_tracks"][3]["uid"],
            inserted["next_tracks"][4]["uid"]
        );
        assert_eq!(inserted["next_tracks"][5], state["next_tracks"][3]);
        assert_eq!(inserted["next_tracks"][6], state["next_tracks"][4]);
        assert_eq!(inserted["prev_tracks"], state["prev_tracks"]);
        let appended = edit_queue(
            "queue.insert",
            &json!({
                "uris":["spotify:track:new"], "after":anchor, "queued":false
            }),
            &state,
        )
        .unwrap();
        assert_eq!(appended["next_tracks"][4]["uri"], "spotify:track:new");
        assert_eq!(appended["next_tracks"][5]["uid"], "hidden");
        assert!(
            edit_queue(
                "queue.insert",
                &json!({
                    "uris":["spotify:track:new"], "before":anchor, "after":anchor
                }),
                &state
            )
            .is_err()
        );
    }

    #[test]
    fn http_queue_edits_refuse_stale_ambiguous_or_restricted_rows() {
        let state = edit_state();
        let wanted = json!({"uri":"spotify:track:same","uid":"missing"});
        assert!(
            command(
                "queue.insert",
                &json!({"uris":["spotify:track:new"],"before":wanted}),
                &state
            )
            .is_err()
        );
        assert!(
            command(
                "queue.reorder",
                &json!({"items":[state["next_tracks"][0]],"before":state["next_tracks"][0]}),
                &state
            )
            .is_err()
        );
        let mut restricted = state.clone();
        restricted["restrictions"] = json!({"disallow_setting_queue_reasons":["restricted"]});
        assert!(command("queue.replace", &json!({"uris":[]}), &restricted).is_err());
        assert!(command("queue.replace", &json!({"uris":["spotify:ad:ad"]}), &state).is_err());
    }

    #[test]
    fn target_state_never_reuses_another_devices_restrictions_or_queue() {
        let cluster = json!({"active_device_id":"a", "player_state":{"restrictions":{"disallow_pausing_reasons":["ad"]}},
            "devices":{"a":{"volume":32768},"b":{"volume":123},"fixed":{"capabilities":{"disable_volume":true}},"restricted":{"is_restricted":true}}});
        assert!(target_state(&cluster, "missing", "player.play").is_err());
        assert!(target_state(&cluster, "restricted", "player.play").is_err());
        let active = target_state(&cluster, "a", "player.pause").unwrap();
        assert!(command("player.pause", &json!({}), active).is_err());
        let other = target_state(&cluster, "b", "player.pause").unwrap();
        assert!(other.is_null());
        assert!(command("player.pause", &json!({}), other).is_ok());
        assert!(target_state(&cluster, "b", "queue.clear").is_err());
        assert!(target_state(&cluster, "fixed", "volume.set").is_err());
        assert!(target_state(&cluster, "b", "volume.set").is_ok());
    }

    #[test]
    fn live_position_accounts_for_elapsed_time_speed_and_bounds() {
        let mut state = json!({"position_as_of_timestamp":"1000","timestamp":"1000","is_paused":false,"playback_speed":1.5,"duration":"10000"});
        assert_eq!(position_at(&state, 5000), 7000);
        assert_eq!(position_at(&state, 50000), 10000);
        assert_eq!(position_at(&state, 500), 1000);
        state["is_paused"] = true.into();
        assert_eq!(position_at(&state, 5000), 1000);
        state["is_paused"] = false.into();
        state["is_buffering"] = true.into();
        assert_eq!(position_at(&state, 5000), 1000);
        state["is_buffering"] = false.into();
        state["timestamp"] = Value::Null;
        assert_eq!(position_at(&state, 5000), 1000);
        let reported = snapshot(&json!({"active_device_id":"test","player_state":state}));
        assert_eq!(reported["durationMs"], 10000);
        assert_eq!(reported["speed"], 1.5);
    }

    #[test]
    fn context_queue_edits_preserve_duplicate_occurrences_and_reject_ads() {
        let state = json!({"next_tracks":[
                {"uri":"spotify:track:same","uid":"one"},
                {"uri":"spotify:track:same","uid":"two"},
                {"uri":"spotify:ad:advert","uid":"ad","provider":"ad"}
            ],"prev_tracks":[]});
        let (moved, _) = command("queue.reorder", &json!({"items":[{"uri":"spotify:track:same","uid":"two"}],"before":{"uri":"spotify:track:same","uid":"one"}}), &state).unwrap();
        assert_eq!(moved["next_tracks"][0]["uid"], "two");
        let (removed, _) = command(
            "queue.remove",
            &json!({"items":[{"uri":"spotify:track:same","uid":"two"}]}),
            &state,
        )
        .unwrap();
        assert_eq!(removed["next_tracks"].as_array().unwrap().len(), 2);
        assert_eq!(removed["next_tracks"][0]["uid"], "one");
        let (promoted, _) = command("queue.reorder", &json!({"items":[{"uri":"spotify:track:same","uid":"two"}],"before":{"uri":"spotify:track:same","uid":"one"},"queued":true}), &state).unwrap();
        assert_eq!(promoted["next_tracks"][0]["metadata"]["is_queued"], "true");
        assert_eq!(promoted["next_tracks"][0]["provider"], "queue");
        let (demoted, _) = command(
            "queue.reorder",
            &json!({"items":[{"uri":"spotify:track:same","uid":"two"}],"queued":false}),
            &promoted,
        )
        .unwrap();
        assert_eq!(demoted["next_tracks"][0]["metadata"]["is_queued"], "false");
        assert_eq!(demoted["next_tracks"][0]["uid"], "two");
        assert!(
            command(
                "queue.remove",
                &json!({"items":[{"uri":"spotify:ad:advert","uid":"ad"}]}),
                &state
            )
            .is_err()
        );
    }

    #[test]
    fn queue_edits_preserve_context_ads_and_exact_identity() {
        let state = json!({"next_tracks":[{"uri":"spotify:track:queued","uid":"q","metadata":{"is_queued":"true"}},{"uri":"spotify:ad:advert","uid":"ad","provider":"ad","metadata":{"is_queued":"true"}},{"uri":"spotify:track:context","uid":"c"}],"prev_tracks":[{"uri":"spotify:track:previous"}]});
        let (clear, _) = command("queue.clear", &json!({}), &state).unwrap();
        assert_eq!(clear["next_tracks"].as_array().unwrap().len(), 2);
        assert_eq!(clear["prev_tracks"], state["prev_tracks"]);
        assert!(command("queue.clear", &json!({}), &json!({"next_tracks":[]})).is_err());
        assert!(
            command(
                "queue.remove",
                &json!({"items":[{"uri":"spotify:track:queued"}]}),
                &state
            )
            .is_err()
        );
        let (smart, _) = command("player.setShuffleMode", &json!({"mode":2}), &state).unwrap();
        assert_eq!(smart["modes"]["context_enhancement"], "RECOMMENDATION");
        assert_eq!(
            volume_body(&json!({"level":0.5})).unwrap(),
            json!({"volume":32768})
        );
        assert_eq!(
            volume_body(&json!({"level":0.0})).unwrap(),
            json!({"volume":0})
        );
        assert_eq!(
            volume_body(&json!({"level":1.0})).unwrap(),
            json!({"volume":65535})
        );
        assert!(volume_body(&json!({"level":2.0})).is_err());
        assert!(
            command(
                "player.pause",
                &json!({}),
                &json!({"restrictions":{"disallow_pausing_reasons":["ad"]}})
            )
            .is_err()
        );
    }
}
