use crate::{Api, domains::segment};
use serde_json::{Value, json};
use spotcore_core::Error;

const PLAYLIST: &str = "https://spclient.wg.spotify.com/playlist/v2";
const PERMISSION: &str = "https://spclient.wg.spotify.com/playlist-permission/v1/playlist";
const COLLECTION: &str = "https://spclient.wg.spotify.com/collection/v2";
const SOCIAL: &str = "https://spclient.wg.spotify.com/social-connect";

fn arg(args: &[Value], index: usize) -> Result<&str, Error> {
    args.get(index)
        .and_then(Value::as_str)
        .ok_or(Error::InvalidInput("missing service string argument"))
}
fn playlist_id(uri: &str) -> Result<&str, Error> {
    uri.strip_prefix("spotify:playlist:")
        .filter(|id| !id.is_empty() && id.bytes().all(|c| c.is_ascii_alphanumeric()))
        .ok_or(Error::InvalidInput("expected a playlist URI"))
}
fn folder_hash(uri: &str) -> Result<&str, Error> {
    uri.split_once(":folder:")
        .map(|(_, tail)| tail.split(':').next().unwrap_or_default())
        .filter(|id| id.len() == 16 && id.bytes().all(|c| c.is_ascii_hexdigit()))
        .ok_or(Error::InvalidInput("expected a folder URI"))
}
fn encode_name(name: &str) -> String {
    url::form_urlencoded::byte_serialize(name.as_bytes()).collect()
}
fn marker(uri: &str) -> Result<String, Error> {
    if uri.contains(":folder:") {
        Ok(format!("spotify:start-group:{}", folder_hash(uri)?))
    } else {
        Ok(uri.to_owned())
    }
}
fn position(value: &Value) -> Result<Value, Error> {
    if value["before"] == "start" {
        return Ok(json!({"addFirst":true}));
    }
    if value["after"] == "end" {
        return Ok(json!({"addLast":true}));
    }
    for (key, output) in [("before", "addBeforeItem"), ("after", "addAfterItem")] {
        if let Some(uri) = value[key]["uri"].as_str() {
            return Ok(json!({output:{"uri":marker(uri)?}}));
        }
    }
    Err(Error::InvalidInput("unsupported rootlist position"))
}
fn add_op(uris: &[String], at: &Value) -> Result<Value, Error> {
    let mut add = position(at)?;
    add["items"] = json!(
        uris.iter()
            .map(|uri| json!({"uri":uri}))
            .collect::<Vec<_>>()
    );
    Ok(json!({"kind":"ADD","add":add}))
}
fn remove_op(uris: &[String]) -> Value {
    json!({"kind":"REM","rem":{"itemsAsKey":true,"items":uris.iter().map(|uri| json!({"uri":uri})).collect::<Vec<_>>()}})
}
fn changes(ops: Vec<Value>, revision: Option<&Value>) -> Value {
    let mut body = json!({"deltas":[{"ops":ops,"info":{"source":{"client":"WEBPLAYER"}}}]});
    if let Some(revision) = revision {
        body["baseRevision"] = revision.clone();
    }
    body
}
async fn root(api: &Api) -> Result<(String, Value), Error> {
    let username = api.users().me().await?.username;
    let data = api.request("GET", &format!("{PLAYLIST}/user/{}/rootlist?decorate=revision,length,attributes,timestamp,owner,capabilities", segment(&username)), None).await?;
    Ok((username, data))
}
async fn root_write(
    api: &Api,
    username: &str,
    ops: Vec<Value>,
    revision: Option<&Value>,
) -> Result<Value, Error> {
    api.request(
        "POST",
        &format!("{PLAYLIST}/user/{}/rootlist/changes", segment(username)),
        Some(changes(ops, revision)),
    )
    .await
}
fn root_items(data: &Value) -> Result<&Vec<Value>, Error> {
    data.pointer("/contents/items")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Backend("missing rootlist items".into()))
}
fn root_tree(username: &str, data: &Value) -> Result<Value, Error> {
    let mut stack: Vec<(String, Value)> = vec![(String::new(), json!({"items":[]}))];
    for (index, item) in root_items(data)?.iter().enumerate() {
        let uri = item["uri"]
            .as_str()
            .ok_or_else(|| Error::Backend("missing rootlist URI".into()))?;
        if let Some(tail) = uri.strip_prefix("spotify:start-group:") {
            let (hash, encoded) = tail.split_once(':').unwrap_or((tail, ""));
            let name = url::form_urlencoded::parse(format!("name={encoded}").as_bytes())
                .next()
                .map(|(_, v)| v.into_owned())
                .unwrap_or_default();
            stack.push((hash.to_owned(), json!({"type":"folder","uri":format!("spotify:user:{username}:folder:{hash}:{encoded}"),"name":name,"isOwnedBySelf":true,"items":[]})));
        } else if let Some(hash) = uri.strip_prefix("spotify:end-group:") {
            if stack.len() <= 1 || stack.last().map(|v| v.0.as_str()) != Some(hash) {
                return Err(Error::Backend("unbalanced rootlist folders".into()));
            }
            let (_, folder) = stack
                .pop()
                .ok_or_else(|| Error::Backend("missing rootlist folder".into()))?;
            stack.last_mut().unwrap().1["items"]
                .as_array_mut()
                .unwrap()
                .push(folder);
        } else {
            let meta = &data["contents"]["metaItems"][index];
            let owner = meta["ownerUsername"].as_str().unwrap_or_default();
            let row = json!({"type":"playlist","uri":uri,"name":meta["attributes"]["name"].as_str().unwrap_or_default(),"description":meta["attributes"]["description"],"owner":{"displayName":owner},"totalLength":meta["length"],"isOwnedBySelf":owner==username,"canAdd":meta["capabilities"]["canEditItems"],"isPublished":item["attributes"]["public"]});
            stack.last_mut().unwrap().1["items"]
                .as_array_mut()
                .unwrap()
                .push(row);
        }
    }
    if stack.len() != 1 {
        return Err(Error::Backend("unclosed rootlist folder".into()));
    }
    Ok(stack.pop().unwrap().1)
}
fn folder_range(data: &Value, uri: &str) -> Result<(usize, usize), Error> {
    let hash = folder_hash(uri)?;
    let items = root_items(data)?;
    let start = items
        .iter()
        .position(|row| {
            row["uri"].as_str().is_some_and(|uri| {
                uri == format!("spotify:start-group:{hash}")
                    || uri.starts_with(&format!("spotify:start-group:{hash}:"))
            })
        })
        .ok_or_else(|| Error::Backend("folder not found".into()))?;
    let end = items
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, row)| row["uri"] == format!("spotify:end-group:{hash}"))
        .map(|(i, _)| i)
        .ok_or_else(|| Error::Backend("folder closing marker missing".into()))?;
    // Validate nesting before an index-based operation can remove other entries.
    root_tree("", data)?;
    Ok((start, end))
}
fn destination_index(data: &Value, at: &Value) -> Result<usize, Error> {
    let items = root_items(data)?;
    if at["before"] == "start" {
        return Ok(0);
    }
    if at["after"] == "end" {
        return Ok(items.len());
    }
    for (key, offset) in [("before", 0), ("after", 1)] {
        if let Some(uri) = at[key]["uri"].as_str() {
            let wanted = marker(uri)?;
            let index = items
                .iter()
                .position(|item| {
                    item["uri"].as_str().is_some_and(|uri| {
                        uri == wanted
                            || (wanted.starts_with("spotify:start-group:")
                                && uri.starts_with(&format!("{wanted}:")))
                    })
                })
                .ok_or_else(|| Error::Backend("rootlist destination not found".into()))?;
            return Ok(index + offset);
        }
    }
    Err(Error::InvalidInput("unsupported rootlist destination"))
}
fn move_op(data: &Value, method: &str, args: &[Value]) -> Result<Value, Error> {
    if data["contents"]["truncated"] == true {
        return Err(Error::Backend(
            "cannot move entries from a truncated rootlist".into(),
        ));
    }
    let at = args
        .get(1)
        .ok_or(Error::InvalidInput("missing move position"))?;
    let destination = destination_index(data, at)?;
    if method == "moveFolder" {
        let uri = args
            .first()
            .and_then(|value| value["uri"].as_str())
            .ok_or(Error::InvalidInput("missing folder"))?;
        let (start, end) = folder_range(data, uri)?;
        if (start..=end).contains(&destination) {
            return Err(Error::InvalidInput("cannot move a folder inside itself"));
        }
        Ok(
            json!({"kind":"MOV","mov":{"fromIndex":start,"toIndex":destination,"length":end-start+1}}),
        )
    } else {
        let values = args
            .first()
            .and_then(Value::as_array)
            .filter(|values| !values.is_empty())
            .ok_or(Error::InvalidInput("missing move items"))?;
        let mut items = Vec::new();
        for value in values {
            let uri = value["uri"]
                .as_str()
                .ok_or(Error::InvalidInput("missing playlist URI"))?;
            playlist_id(uri)?;
            if !root_items(data)?.iter().any(|row| row["uri"] == uri) {
                return Err(Error::InvalidInput("playlist is not in the library"));
            }
            items.push(json!({"uri":uri}));
        }
        let mut mov = position(at)?;
        mov["items"] = json!(items);
        Ok(json!({"kind":"MOV","mov":mov}))
    }
}

async fn update(api: &Api, uri: &str, fields: &Value) -> Result<Value, Error> {
    let id = playlist_id(uri)?;
    let mut values = json!({});
    let mut clear = Vec::new();
    for (field, flag) in [
        ("name", "LIST_NAME"),
        ("description", "LIST_DESCRIPTION"),
        ("picture", "LIST_PICTURE"),
    ] {
        if let Some(value) = fields.get(field) {
            if value.is_null() || value == "" {
                clear.push(flag);
            } else {
                values[field] = value.clone();
            }
        }
    }
    api.request("POST", &format!("{PLAYLIST}/playlist/{id}/changes"), Some(changes(vec![json!({"kind":"UPDATE_LIST_ATTRIBUTES","updateListAttributes":{"newAttributes":{"values":values,"noValue":clear}}})],None))).await
}
pub(crate) async fn cover(api: &Api, uri: &str, jpeg: Option<&[u8]>) -> Result<Value, Error> {
    let id = playlist_id(uri)?;
    let picture = if let Some(bytes) = jpeg {
        let response = api
            .request_raw(
                "POST",
                "https://image-upload.spotify.com/v4/playlist",
                Some(bytes.to_vec()),
                "image/jpeg",
            )
            .await?;
        let value: Value = serde_json::from_slice(&response.body)
            .map_err(|_| Error::Backend("invalid cover upload response".into()))?;
        let token = value["uploadToken"]
            .as_str()
            .ok_or_else(|| Error::Backend("missing image upload token".into()))?;
        let image = api
            .request(
                "POST",
                &format!("{PLAYLIST}/playlist/{id}/register-image"),
                Some(json!({"uploadToken":token})),
            )
            .await?;
        image
            .get("picture")
            .filter(|v| v.is_string())
            .cloned()
            .ok_or_else(|| Error::Backend("missing registered image".into()))?
    } else {
        Value::Null
    };
    update(api, uri, &json!({"picture":picture})).await
}
fn valid_uri(uri: &str) -> bool {
    uri.strip_prefix("spotify:")
        .is_some_and(|rest| rest.split(':').count() >= 2 && !rest.contains(char::is_whitespace))
}
fn collection_body(username: &str, set: &str, uris: &[String], removed: bool) -> Value {
    let items: Vec<Value> = uris
        .iter()
        .map(|uri| {
            if removed {
                json!({"uri": uri, "is_removed": true})
            } else {
                json!({"uri": uri})
            }
        })
        .collect();
    json!({"username": username, "set": set, "items": items})
}
fn collection_items(body: &Value) -> Result<Vec<Value>, Error> {
    let items = match body.get("items").and_then(Value::as_array) {
        Some(items) => items.as_slice(),
        // Spotify omits `items` for an empty set and sends only a sync token.
        None if body.get("sync_token").is_some() => &[],
        None => return Err(Error::Backend("missing collection items".into())),
    };
    Ok(items
        .iter()
        .map(|item| json!({"uri": item["uri"], "addTime": item["added_at"]}))
        .collect())
}
async fn collection(api: &Api, method: &str, args: &[Value]) -> Result<Value, Error> {
    let set = arg(args, 0)?;
    let username = api.users().me().await?.username;
    match method {
        "get" => {
            let body = api
                .request(
                    "POST",
                    &format!("{COLLECTION}/paging"),
                    Some(json!({"username": username, "set": set})),
                )
                .await?;
            let items = collection_items(&body)?;
            Ok(Value::Array(items))
        }
        "add" | "remove" => {
            let uris = args
                .get(1)
                .and_then(Value::as_array)
                .ok_or(Error::InvalidInput("missing collection items"))?;
            let uris = uris
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .filter(|uri| valid_uri(uri))
                        .map(str::to_owned)
                        .ok_or(Error::InvalidInput("invalid collection item"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            // /write handles both directions; remove marks each item
            api.request(
                "POST",
                &format!("{COLLECTION}/write"),
                Some(collection_body(&username, set, &uris, method == "remove")),
            )
            .await?;
            Ok(Value::Null)
        }
        _ => Err(Error::Unsupported),
    }
}
fn history_url(username: &str) -> String {
    format!(
        "https://spclient.wg.spotify.com/recently-played/v3/user/{}/recently-played?format=json&offset=0&limit=50&filter=default%2Ccollection-new-episodes",
        segment(username)
    )
}
pub(crate) async fn history(api: &Api) -> Result<Value, Error> {
    let username = api.users().me().await?.username;
    let body = api.request("GET", &history_url(&username), None).await?;
    Ok(body["playContexts"].clone())
}
fn session_id(body: &Value) -> Result<&str, Error> {
    body.pointer("/session/session_id")
        .or_else(|| body.get("session_id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| Error::Backend("no active Jam session".into()))
}
fn jam_join_url(token: &str, device: &str) -> String {
    format!(
        "{SOCIAL}/v2/sessions/join/{}?playback_control=listen_and_control&join_type=link&local_device_id={}",
        segment(token),
        segment(device)
    )
}
async fn jam_device(api: &Api) -> Result<String, Error> {
    let cluster = cluster(api).await?;
    if let Some(id) = cluster["active_device_id"].as_str() {
        return Ok(id.to_owned());
    }
    cluster["devices"]
        .as_object()
        .and_then(|devices| devices.keys().next())
        .cloned()
        .ok_or_else(|| Error::Backend("no active Connect device".into()))
}
async fn jam_session(api: &Api) -> Result<Value, Error> {
    let device = jam_device(api).await?;
    match api
        .request(
            "GET",
            &format!("{SOCIAL}/v2/devices/{}/jam_status", segment(&device)),
            None,
        )
        .await
    {
        Ok(body) => Ok(body.get("session").cloned().unwrap_or(Value::Null)),
        Err(Error::Http { status: 404, .. }) => Ok(Value::Null),
        Err(error) => Err(error),
    }
}
fn jam_invite_credentials<'a>(
    info: &'a Value,
    expected_id: &str,
) -> Result<(&'a str, &'a str), Error> {
    if session_id(info)? != expected_id {
        return Err(Error::InvalidInput(
            "requested Jam is not the current session",
        ));
    }
    let token = info["join_session_token"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::Backend("session has no join token".into()))?;
    let uri = info["join_session_uri"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::Backend("session has no join URI".into()))?;
    Ok((token, uri))
}
async fn jam(api: &Api, method: &str, args: &[Value]) -> Result<Value, Error> {
    match method {
        // The desktop client hardcodes this configuration as enabled.
        "getEnabled" => Ok(json!("ENABLED")),
        "fetchCurrentSession" => jam_session(api).await,
        "createSession" => {
            api.request(
                "GET",
                &format!("{SOCIAL}/v2/sessions/current_or_new?activate=true"),
                None,
            )
            .await
        }
        "joinSession" => {
            let token = arg(args, 0)?;
            let device = jam_device(api).await?;
            api.request("POST", &jam_join_url(token, &device), None)
                .await
        }
        "leaveSession" | "deleteSession" => {
            let current = jam_session(api).await?;
            let id = match args.first().and_then(Value::as_str) {
                Some(id) => id,
                None => session_id(&current)?,
            };
            if method == "leaveSession" {
                match api
                    .request(
                        "POST",
                        &format!("{SOCIAL}/v3/sessions/{}/leave", segment(id)),
                        None,
                    )
                    .await
                {
                    Err(Error::Http { status: 404, .. }) => {}
                    other => {
                        other?;
                    }
                }
            } else {
                match api
                    .request(
                        "DELETE",
                        &format!("{SOCIAL}/v3/sessions/{}", segment(id)),
                        None,
                    )
                    .await
                {
                    Err(Error::Http { status: 404, .. }) => {}
                    other => {
                        other?;
                    }
                }
            }
            Ok(json!(true))
        }
        "getSessionInfo" => {
            let url = format!("{SOCIAL}/v2/sessions/info/{}", segment(arg(args, 0)?));
            match api.request("GET", &url, None).await {
                Ok(body) => Ok(body),
                Err(Error::Http { status: 404, .. }) => Ok(Value::Null),
                Err(error) => Err(error),
            }
        }
        "removeSessionMember" | "kickAllParticipants" => {
            let current = jam_session(api).await?;
            let id = session_id(&current)?;
            let url = if method == "removeSessionMember" {
                format!(
                    "{SOCIAL}/v3/sessions/{}/member/{}/kick",
                    segment(id),
                    segment(arg(args, 0)?)
                )
            } else {
                format!("{SOCIAL}/v3/sessions/{}/participant/all/kick", segment(id))
            };
            api.request("POST", &url, None).await
        }
        "setQueueOnlyMode" => {
            let enabled = args
                .first()
                .and_then(Value::as_bool)
                .ok_or(Error::InvalidInput("missing queue-only mode"))?;
            api.request(
                "PUT",
                &format!(
                    "{SOCIAL}/v2/sessions/current/queue_only_mode/{}",
                    if enabled { "enabled" } else { "disabled" }
                ),
                None,
            )
            .await
        }
        "setParticipantVolumeControl" => {
            let enabled = args
                .first()
                .and_then(Value::as_bool)
                .ok_or(Error::InvalidInput("missing volume control"))?;
            api.request(
                "PUT",
                &format!(
                    "{SOCIAL}/v2/sessions/current/volume_control/{}",
                    if enabled { "ENABLED" } else { "DISABLED" }
                ),
                None,
            )
            .await
        }
        "getShortInviteLink" => {
            let info = jam_session(api).await?;
            let (token, uri) = jam_invite_credentials(&info, arg(args, 0)?)?;
            let body = api
                .request(
                    "POST",
                    "https://spclient.wg.spotify.com/url-dispenser/v1/generate-url",
                    Some(json!({
                        "spotify_uri": uri,
                        "custom_data": [
                            {"key": "ssp", "value": "1"},
                            {"key": "app_destination", "value": "socialsession"}
                        ],
                        "link_preview": {
                            "title": "Join my Jam",
                            "image_url": format!("https://shareables.scdn.co/publish/socialsession/{token}")
                        },
                        "utm_parameters": {
                            "utm_campaign": null,
                            "utm_term": null,
                            "utm_medium": "share-link",
                            "utm_content": null,
                            "utm_source": "share-options-sheet"
                        }
                    })),
                )
                .await?;
            Ok(json!({
                "shareableUrl": body["shareable_url"],
                "shareId": body["share_id"],
                "spotifyUri": body["spotify_uri"]
            }))
        }
        _ => Err(Error::Unsupported),
    }
}
pub(crate) async fn service(
    api: &Api,
    name: &str,
    method: &str,
    args: &[Value],
) -> Result<Value, Error> {
    if name == "PlaylistAPI" && method == "updateDetails" {
        return update(
            api,
            arg(args, 0)?,
            args.get(1)
                .filter(|v| v.is_object())
                .ok_or(Error::InvalidInput("missing playlist fields"))?,
        )
        .await;
    }
    if name == "PlaylistPermissionsAPI" {
        let id = playlist_id(arg(args, 0)?)?;
        let url = format!("{PERMISSION}/{id}/permission/base");
        return match method {
            "getBasePermission" => {
                let value = api.request("GET", &url, None).await?;
                value
                    .get("permissionLevel")
                    .cloned()
                    .ok_or_else(|| Error::Backend("missing playlist permission".into()))
            }
            "setBasePermission" => {
                let level = arg(args, 1)?;
                if !matches!(level, "BLOCKED" | "VIEWER" | "CONTRIBUTOR") {
                    return Err(Error::InvalidInput("invalid permission level"));
                }
                let mut value = api.request("GET", &url, None).await?;
                if !value.is_object() {
                    return Err(Error::Backend("invalid permission response".into()));
                }
                value["permissionLevel"] = json!(level);
                api.request("POST", &url, Some(value)).await
            }
            "getMembers" => {
                let value = api
                    .gql("playlistPermissions", json!({"uri":arg(args,0)?}))
                    .await?;
                value
                    .pointer("/data/playlistV2/members/items")
                    .cloned()
                    .ok_or_else(|| Error::Backend("missing playlist members".into()))
            }
            _ => Err(Error::Unsupported),
        };
    }
    if name == "SocialConnectAPI" {
        return jam(api, method, args).await;
    }
    if name == "CollectionPlatformAPI" {
        return collection(api, method, args).await;
    }
    if name == "RecentlyPlayedAPI" {
        return if method == "getContexts" {
            history(api).await
        } else {
            Err(Error::Unsupported)
        };
    }
    if name != "RootlistAPI" {
        return Err(Error::Unsupported);
    }
    let (username, data) = root(api).await?;
    match method {
        "getContents" => root_tree(&username, &data),
        "contains" => {
            let uris = args
                .first()
                .and_then(Value::as_array)
                .ok_or(Error::InvalidInput("missing URI list"))?;
            let items = root_items(&data)?;
            Ok(json!(
                uris.iter()
                    .map(|uri| items.iter().any(|row| row["uri"] == *uri))
                    .collect::<Vec<_>>()
            ))
        }
        "createPlaylist" => {
            let at = args.get(1).ok_or(Error::InvalidInput("missing position"))?;
            position(at)?;
            let title = arg(args, 0)?;
            if title.trim().is_empty() {
                return Err(Error::InvalidInput("empty playlist name"));
            }
            let value=api.request("POST",&format!("{PLAYLIST}/playlist"),Some(json!({"ops":[{"kind":"UPDATE_LIST_ATTRIBUTES","updateListAttributes":{"newAttributes":{"values":{"name":title},"noValue":[]}}}]}))).await?;
            let uri = value["uri"]
                .as_str()
                .filter(|uri| playlist_id(uri).is_ok())
                .ok_or_else(|| Error::Backend("missing created playlist URI".into()))?;
            root_write(
                api,
                &username,
                vec![add_op(
                    &[uri.to_owned()],
                    args.get(1).ok_or(Error::InvalidInput("missing position"))?,
                )?],
                None,
            )
            .await
            .map_err(|_| {
                Error::Backend(format!(
                    "playlist created at {uri}; rootlist write failed, outcome must be checked"
                ))
            })?;
            Ok(json!(uri))
        }
        "createFolder" => {
            let title = arg(args, 0)?;
            let hash = uuid::Uuid::new_v4().simple().to_string()[..16].to_owned();
            let encoded = encode_name(title);
            let uris = vec![
                format!("spotify:start-group:{hash}:{encoded}"),
                format!("spotify:end-group:{hash}"),
            ];
            root_write(
                api,
                &username,
                vec![add_op(
                    &uris,
                    args.get(1).ok_or(Error::InvalidInput("missing position"))?,
                )?],
                None,
            )
            .await?;
            Ok(json!({"uri":format!("spotify:user:{username}:folder:{hash}:{encoded}")}))
        }
        "renameFolder" => {
            let uri = args
                .first()
                .and_then(|v| v["uri"].as_str())
                .ok_or(Error::InvalidInput("missing folder"))?;
            folder_range(&data, uri)?;
            let hash = folder_hash(uri)?;
            let title = encode_name(arg(args, 1)?);
            let temporary = uuid::Uuid::new_v4().simple().to_string()[..16].to_owned();
            let old = format!("spotify:start-group:{hash}");
            let temp = format!("spotify:start-group:{temporary}:{title}");
            let new = format!("spotify:start-group:{hash}:{title}");
            root_write(
                api,
                &username,
                vec![
                    add_op(std::slice::from_ref(&temp), &json!({"before":{"uri":old}}))?,
                    remove_op(&[old]),
                    add_op(&[new], &json!({"before":{"uri":temp}}))?,
                    remove_op(&[temp]),
                ],
                None,
            )
            .await
        }
        "move" | "moveFolder" => {
            let op = move_op(&data, method, args)?;
            let revision = data
                .get("revision")
                .filter(|value| value.is_string())
                .ok_or_else(|| Error::Backend("missing rootlist revision".into()))?;
            root_write(api, &username, vec![op], Some(revision)).await
        }
        "removeFolder" => {
            let (start, end) = folder_range(&data, arg(args, 0)?)?;
            let revision = data
                .get("revision")
                .filter(|v| v.is_string())
                .ok_or_else(|| Error::Backend("missing rootlist revision".into()))?;
            root_write(
                api,
                &username,
                vec![json!({"kind":"REM","rem":{"fromIndex":start,"length":end-start+1}})],
                Some(revision),
            )
            .await
        }
        "add" | "remove" => {
            let items = args
                .first()
                .and_then(Value::as_array)
                .ok_or(Error::InvalidInput("missing rootlist items"))?;
            let uris = items
                .iter()
                .map(|v| {
                    v.as_str()
                        .or_else(|| v["uri"].as_str())
                        .map(str::to_owned)
                        .ok_or(Error::InvalidInput("invalid rootlist item"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            if uris.iter().any(|uri| playlist_id(uri).is_err()) {
                return Err(Error::InvalidInput("expected playlist items"));
            }
            let op = if method == "add" {
                add_op(
                    &uris,
                    args.get(1).ok_or(Error::InvalidInput("missing position"))?,
                )?
            } else {
                remove_op(&uris)
            };
            root_write(api, &username, vec![op], None).await
        }
        "getPublishedState" => {
            let uri = arg(args, 0)?;
            playlist_id(uri)?;
            Ok(json!(
                root_items(&data)?
                    .iter()
                    .find(|v| v["uri"] == uri)
                    .and_then(|v| v["attributes"]["public"].as_bool())
                    .unwrap_or(false)
            ))
        }
        "setPublishedState" => {
            let uri = arg(args, 0)?;
            playlist_id(uri)?;
            let public = args
                .get(1)
                .and_then(Value::as_bool)
                .ok_or(Error::InvalidInput("missing publication flag"))?;
            root_write(api,&username,vec![json!({"kind":"UPDATE_ITEM_ATTRIBUTES","updateItemAttributes":{"item":{"uri":uri},"newAttributes":{"values":{"public":public},"noValue":[]}}})],None).await
        }
        _ => Err(Error::Unsupported),
    }
}

pub(crate) async fn cluster(api: &Api) -> Result<Value, Error> {
    api.request(
        "GET",
        "https://spclient.wg.spotify.com/connect-state/v1/cluster",
        None,
    )
    .await
}
// path-safe so a caller cannot redirect the transfer to another host or path
fn valid_device_id(target: &str) -> bool {
    !target.is_empty()
        && target
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}
pub(crate) async fn transfer(api: &Api, target: &str, play: bool) -> Result<Value, Error> {
    if !valid_device_id(target) {
        return Err(Error::InvalidInput("invalid Connect device ID"));
    }
    api.request("POST",&format!("https://spclient.wg.spotify.com/connect-state/v1/connect/transfer/from/{}/to/{}",api.device_id(),segment(target)),
        Some(json!({"transfer_options":{"restore_paused":if play {"resume"} else {"pause"}},"command_id":uuid::Uuid::new_v4().to_string(),"interaction_id":uuid::Uuid::new_v4().to_string()}))).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn moving_folders_preserves_the_whole_range_and_rejects_descendants() {
        let data = json!({"contents":{"items":[
            {"uri":"spotify:playlist:before"},
            {"uri":"spotify:start-group:0123456789abcdef:parent"},
            {"uri":"spotify:playlist:child"},
            {"uri":"spotify:start-group:fedcba9876543210:nested"},
            {"uri":"spotify:end-group:fedcba9876543210"},
            {"uri":"spotify:end-group:0123456789abcdef"},
            {"uri":"spotify:playlist:after"}]}});
        let mut truncated = data.clone();
        truncated["contents"]["truncated"] = json!(true);
        assert!(
            move_op(
                &truncated,
                "move",
                &[
                    json!([{"uri":"spotify:playlist:before"}]),
                    json!({"before":"start"})
                ]
            )
            .is_err()
        );
        let parent = json!({"uri":"spotify:user:test:folder:0123456789abcdef"});
        let op = move_op(
            &data,
            "moveFolder",
            &[parent.clone(), json!({"before":"start"})],
        )
        .unwrap();
        assert_eq!(op["mov"], json!({"fromIndex":1,"toIndex":0,"length":5}));
        assert!(
            move_op(
                &data,
                "moveFolder",
                &[
                    parent,
                    json!({"after":{"uri":"spotify:user:test:folder:fedcba9876543210"}})
                ]
            )
            .is_err()
        );
        let playlist = json!([{"uri":"spotify:playlist:before"}]);
        let op = move_op(
            &data,
            "move",
            &[
                playlist,
                json!({"after":{"uri":"spotify:user:test:folder:0123456789abcdef"}}),
            ],
        )
        .unwrap();
        assert_eq!(
            op["mov"]["addAfterItem"]["uri"],
            "spotify:start-group:0123456789abcdef"
        );
        assert!(
            move_op(
                &data,
                "move",
                &[
                    json!([{"uri":"spotify:playlist:missing"}]),
                    json!({"before":"start"})
                ]
            )
            .is_err()
        );
    }

    #[test]
    fn folders_require_balanced_markers_and_keep_revision() {
        let data = json!({"revision":"revision","contents":{"items":[{"uri":"spotify:start-group:0123456789abcdef:hello+world"},{"uri":"spotify:playlist:abc"},{"uri":"spotify:end-group:0123456789abcdef"}]}});
        let uri = "spotify:user:test:folder:0123456789abcdef";
        assert_eq!(folder_range(&data, uri).unwrap(), (0, 2));
        let tree = root_tree("test", &data).unwrap();
        assert_eq!(tree["items"][0]["name"], "hello world");
        assert_eq!(tree["items"][0]["items"][0]["uri"], "spotify:playlist:abc");
        let malformed =
            json!({"contents":{"items":[{"uri":"spotify:start-group:0123456789abcdef:x"}]}});
        assert!(folder_range(&malformed, uri).is_err());
        assert_eq!(
            changes(vec![], data.get("revision"))["baseRevision"],
            "revision"
        );
        assert!(playlist_id("spotify:playlist:../../evil").is_err());
        assert!(position(&json!({})).is_err());
    }

    #[test]
    fn connect_device_ids_are_path_safe() {
        assert!(valid_device_id("7a653f23262cace414a81db0ea054a46a4a460f6"));
        assert!(valid_device_id("abc_DEF-123"));
        for bad in ["", "../evil", "a/b", "a?b", "a b", "a#b"] {
            assert!(!valid_device_id(bad), "{bad:?} was accepted");
        }
    }

    #[test]
    fn collection_items_accept_empty_sets() {
        assert!(
            collection_items(&json!({"sync_token": "t"}))
                .unwrap()
                .is_empty()
        );
        assert!(collection_items(&json!({})).is_err());
        assert_eq!(
            collection_items(
                &json!({"items":[{"uri":"spotify:episode:x","added_at":"2026-01-01"}]})
            )
            .unwrap(),
            vec![json!({"uri":"spotify:episode:x","addTime":"2026-01-01"})]
        );
    }

    #[test]
    fn collection_bodies_and_history_url_are_stable() {
        let uris = ["spotify:track:abc".to_owned()];
        let add = collection_body("me", "ignoreinrecs", &uris, false);
        assert_eq!(add["username"], "me");
        assert_eq!(add["set"], "ignoreinrecs");
        assert_eq!(add["items"][0]["uri"], "spotify:track:abc");
        assert!(add["items"][0].get("is_removed").is_none());
        let remove = collection_body("me", "ignoreinrecs", &uris, true);
        assert_eq!(remove["items"][0]["is_removed"], true);
        assert!(valid_uri("spotify:track:abc"));
        for bad in ["spotify:track", "track:abc", "spotify:track:a b", ""] {
            assert!(!valid_uri(bad), "{bad:?} was accepted");
        }
        assert!(history_url("me").contains("/recently-played/v3/user/me/recently-played?"));
        assert!(history_url("me").contains("filter=default%2Ccollection-new-episodes"));
        assert!(history_url("a b").contains("/user/a%20b/"));
    }

    #[test]
    fn jam_invites_require_the_matching_session_and_join_credentials() {
        let mut info = json!({
            "session_id": "hosted",
            "join_session_token": "invite-token",
            "join_session_uri": "spotify:socialsession:invite-token"
        });
        assert_eq!(
            jam_invite_credentials(&info, "hosted").unwrap(),
            ("invite-token", "spotify:socialsession:invite-token")
        );
        assert!(jam_invite_credentials(&info, "old-session").is_err());
        assert!(jam_invite_credentials(&Value::Null, "hosted").is_err());
        info["join_session_token"] = json!("");
        assert!(jam_invite_credentials(&info, "hosted").is_err());
        info["join_session_token"] = json!("invite-token");
        info.as_object_mut().unwrap().remove("join_session_uri");
        assert!(jam_invite_credentials(&info, "hosted").is_err());
    }

    #[test]
    fn jam_session_and_join_urls_are_stable() {
        assert_eq!(
            session_id(&json!({"session":{"session_id":"abc"}})).unwrap(),
            "abc"
        );
        assert_eq!(session_id(&json!({"session_id":"abc"})).unwrap(), "abc");
        assert!(session_id(&json!({"session":null})).is_err());
        let url = jam_join_url("a/b c", "dev-1");
        assert!(url.ends_with("/join/a%2Fb%20c?playback_control=listen_and_control&join_type=link&local_device_id=dev-1"));
    }
}
