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
fn track(value: &Value) -> Value {
    if value.is_null() {
        return Value::Null;
    }
    let metadata = &value["metadata"];
    json!({"uri":value["uri"],"uid":value["uid"],"name":metadata["title"],"durationMs":metadata["duration"].as_str().and_then(|n|n.parse::<u64>().ok()),
        "kind":if is_ad(value){"ad"}else{"track"},"metadata":metadata})
}
fn queued(value: &Value) -> bool {
    !is_ad(value)
        && (value["metadata"]["is_queued"] == "true" || value["metadata"]["is_queued"] == true)
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
fn command(method: &str, args: &Value, state: &Value) -> Result<(Value, &'static str), Error> {
    let mut value = json!({});
    let (endpoint, restriction) = match method {
        "player.pause" => ("pause", "pausing"),
        "player.resume" => ("resume", "resuming"),
        "player.next" => ("skip_next", "skipping_next"),
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
            value = json!({"context":{"uri":uri,"url":format!("context://{uri}"),"metadata":{}},"options":options});
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
                next.retain(|item| {
                    !queued(item)
                        || !remove.iter().any(|wanted| {
                            wanted["uid"] == item["uid"] && wanted["uri"] == item["uri"]
                        })
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
        "queue.clear" | "queue.remove" | "player.setShuffleMode"
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
            | "queue.clear"
            | "queue.remove"
            | "queue.add"
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
            let next = rows(state, "next_tracks")?;
            return Ok(
                json!({"current":track(&state["track"]),"queued":next.iter().filter(|v|queued(v)).map(track).collect::<Vec<_>>(),"nextUp":next.iter().filter(|v|!queued(v)).map(track).collect::<Vec<_>>()}),
            );
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
    use super::*;
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
