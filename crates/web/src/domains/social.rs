use super::*;
use crate::{JamSession, ListenOn};
use serde::{Deserialize, Serialize};
use serde_json::json;
use spotcore_auth::AccountTier;
use spotcore_core::{AudioQuality, Paging};

fn sparse_media(item: &spotcore_core::MediaItem) -> bool {
    item.name.is_empty()
        || !item.images.iter().any(|image| !image.url.trim().is_empty())
        || (matches!(
            item.kind,
            spotcore_core::MediaKind::Track | spotcore_core::MediaKind::Episode
        ) && item.duration_ms.is_none_or(|duration| duration == 0))
}

fn fill_media_metadata(item: &mut spotcore_core::MediaItem, full: spotcore_core::MediaItem) {
    crate::recovery::fill(item, &full);
}

pub(crate) async fn hydrate_media(api: &Api, mut items: Vec<&mut spotcore_core::MediaItem>) {
    // Legacy best-effort reads stay bounded. Explicit RecoveryPage callers can continue all pages.
    for item in &mut items {
        if let Some(full) = api.cached_media(&item.uri) {
            fill_media_metadata(item, full);
        }
    }
    let missing: Vec<_> = items
        .iter()
        .filter(|item| sparse_media(item) && api.should_lookup_media(&item.uri))
        .map(|item| item.uri.clone())
        .collect();
    if !missing.is_empty()
        && let Ok(page) = api
            .recover_media(
                &missing,
                spotcore_core::Paging {
                    offset: 0,
                    limit: 4,
                },
            )
            .await
    {
        for resolution in page.items {
            if matches!(
                resolution.error,
                Some(Error::Http {
                    status: 404 | 410,
                    ..
                })
            ) {
                api.cache_media(resolution.uri, None);
            }
        }
    }
    for item in items {
        if sparse_media(item)
            && let Some(full) = api.cached_media(&item.uri)
        {
            fill_media_metadata(item, full);
        }
    }
}

pub struct Users<'a>(pub(crate) &'a Api);
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Account {
    pub uri: String,
    pub username: String,
    pub name: String,
    pub tier: AccountTier,
    pub country: Option<String>,
    #[serde(default)]
    pub images: Vec<spotcore_core::Image>,
    pub raw: Value,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum TimeRange {
    ShortTerm,
    MediumTerm,
    LongTerm,
}
impl TimeRange {
    fn gql(self) -> &'static str {
        match self {
            Self::ShortTerm => "SHORT_TERM",
            Self::MediumTerm => "MID_TERM",
            Self::LongTerm => "LONG_TERM",
        }
    }
}
fn profile_name_update(name: &str) -> Result<Vec<u8>, Error> {
    if name.trim().is_empty() || name.chars().count() > 32 {
        return Err(Error::InvalidInput(
            "profile name must contain 1 to 32 characters",
        ));
    }
    fn field(tag: u8, bytes: &[u8]) -> Vec<u8> {
        let mut result = vec![tag];
        let mut length = bytes.len();
        while length >= 128 {
            result.push((length as u8 & 0x7f) | 0x80);
            length >>= 7;
        }
        result.push(length as u8);
        result.extend_from_slice(bytes);
        result
    }
    // UpdateUserProfileRequest: FieldMask(name), UserProfile(StringValue(name)).
    let mut body = field(10, &field(10, b"name"));
    body.extend(field(18, &field(18, &field(10, name.as_bytes()))));
    Ok(body)
}

impl Users<'_> {
    /// Updates this account's display name and optionally assigns a JPEG profile image.
    /// Writes are sequential and are not atomic. Refresh the profile after a failure.
    pub async fn update_profile(&self, name: &str, image: Option<&[u8]>) -> Result<(), Error> {
        let name_body = profile_name_update(name)?;
        if image.is_some_and(|bytes| {
            bytes.len() > 10 * 1024 * 1024 || !bytes.starts_with(&[0xff, 0xd8, 0xff])
        }) {
            return Err(Error::InvalidInput(
                "profile image must be a JPEG no larger than 10 MiB",
            ));
        }
        let account = self.me().await?;
        let token = if let Some(bytes) = image {
            let response = self
                .0
                .request_raw(
                    "POST",
                    "https://image-upload.spotify.com/v4/user-profile",
                    Some(bytes.to_vec()),
                    "image/jpeg",
                )
                .await?;
            let value: Value = serde_json::from_slice(&response.body)
                .map_err(|_| Error::Backend("invalid profile image upload response".into()))?;
            Some(
                value["uploadToken"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| Error::Backend("profile image upload returned no token".into()))?
                    .to_owned(),
            )
        } else {
            None
        };
        let username = segment(&account.username);
        self.0
            .request_raw(
                "POST",
                &format!("https://spclient.wg.spotify.com/identity/v3/user/username/{username}"),
                Some(name_body),
                "application/x-protobuf",
            )
            .await?;
        if let Some(token) = token {
            self.0
                .request_raw(
                    "POST",
                    &format!(
                        "https://spclient.wg.spotify.com/identity/v3/profile-image/{username}/{}",
                        segment(&token)
                    ),
                    None,
                    "application/x-protobuf",
                )
                .await?;
        }
        Ok(())
    }

    pub async fn me(&self) -> Result<Account, Error> {
        let (profile, account) = tokio::try_join!(
            self.0.gql("profileAttributes", json!({})),
            self.0.gql("accountAttributes", json!({}))
        )?;
        let p = field(&profile, "/data/me/profile")?;
        let a = field(&account, "/data/me/account")?;
        let username = p["username"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(Error::NotAuthenticated)?
            .to_owned();
        let tier = match a["product"]
            .as_str()
            .map(str::to_ascii_uppercase)
            .as_deref()
        {
            Some("FREE") => AccountTier::Free,
            Some("PREMIUM") => AccountTier::Premium,
            _ => AccountTier::Unknown,
        };
        Ok(Account {
            uri: p["uri"].as_str().unwrap_or_default().into(),
            name: p["name"].as_str().unwrap_or(&username).into(),
            username,
            tier,
            country: a["country"].as_str().map(str::to_owned),
            images: crate::mapping::item(&p)
                .map(|item| item.images)
                .unwrap_or_default(),
            raw: json!({"profile":p,"account":a}),
        })
    }
    pub async fn profile(&self, value: &str) -> Result<Value, Error> {
        uri(value)?;
        self.0.request("GET",&format!("https://spclient.wg.spotify.com/user-profile-view/v3/profile/{}?playlist_limit=50&artist_limit=10&episode_limit=10&market=from_token",segment(value.rsplit(':').next().unwrap_or_default())),None).await
    }
    pub async fn followers(&self, value: &str) -> Result<Value, Error> {
        self.profile_list(value, "followers").await
    }
    pub async fn following(&self, value: &str) -> Result<Value, Error> {
        self.profile_list(value, "following").await
    }
    async fn profile_list(&self, value: &str, kind: &str) -> Result<Value, Error> {
        uri(value)?;
        self.0.request("GET",&format!("https://spclient.wg.spotify.com/user-profile-view/v3/profile/{}/{kind}?market=from_token",segment(value.rsplit(':').next().unwrap_or_default())),None).await
    }
    pub async fn top(&self, range: TimeRange, limit: u32) -> Result<Value, Error> {
        Paging { offset: 0, limit }.validate()?;
        let input = json!({"offset":0,"limit":limit,"sortBy":"AFFINITY","timeRange":range.gql()});
        self.0.gql("userTopContent",json!({"includeTopArtists":true,"topArtistsInput":input,"includeTopTracks":true,"topTracksInput":input})).await
    }
    pub async fn follow(&self, values: &[String]) -> Result<Value, Error> {
        self.0
            .gql("followUsers", json!({"usernames":ids(values)?}))
            .await
    }
    pub async fn unfollow(&self, values: &[String]) -> Result<Value, Error> {
        self.0
            .gql("unfollowUsers", json!({"usernames":ids(values)?}))
            .await
    }
    pub async fn is_following(&self, values: &[String]) -> Result<Value, Error> {
        for v in values {
            uri(v)?;
        }
        self.0.gql("isFollowingUsers", json!({"uris":values})).await
    }
    pub async fn history(&self) -> Result<Value, Error> {
        service(self.0, "RecentlyPlayedAPI", "getContexts", vec![]).await
    }
    pub async fn friend_activity(&self) -> Result<Value, Error> {
        self.0
            .request(
                "GET",
                "https://guc-spclient.spotify.com/presence-view/v1/buddylist",
                None,
            )
            .await
    }
    pub async fn set_private_session(&self, enabled: bool) -> Result<Value, Error> {
        let capabilities = service(self.0, "PrivateSessionAPI", "getCapabilities", vec![]).await?;
        if capabilities["isSupported"] != true {
            return Err(Error::Unsupported);
        }
        service(
            self.0,
            "PrivateSessionAPI",
            "setPrivateSession",
            vec![json!(enabled)],
        )
        .await
    }
    async fn block_url(&self, value: &str) -> Result<String, Error> {
        uri(value)?;
        let me = service(self.0, "UserAPI", "getUser", vec![]).await?;
        let username = me["username"].as_str().ok_or(Error::NotAuthenticated)?;
        Ok(format!(
            "https://spclient.wg.spotify.com/socialgraph/v4/{}/is-blocking/{}",
            segment(username),
            segment(value.rsplit(':').next().unwrap_or_default())
        ))
    }
    pub async fn set_blocked(&self, value: &str, blocked: bool) -> Result<Value, Error> {
        self.0
            .request(
                if blocked { "POST" } else { "DELETE" },
                &self.block_url(value).await?,
                None,
            )
            .await
    }
    pub async fn is_blocked(&self, value: &str) -> Result<bool, Error> {
        let response = self
            .0
            .request_raw(
                "GET",
                &self.block_url(value).await?,
                None,
                "application/json",
            )
            .await?;
        match response.body.as_slice() {
            [] => Ok(false),
            [8, 0] => Ok(false),
            [8, 1] => Ok(true),
            _ => Err(Error::Backend("unexpected block status response".into())),
        }
    }
}

pub struct Extras<'a>(pub(crate) &'a Api);
impl Extras<'_> {
    /// Resolves Spotify's countdown page; its URI is not an album URI.
    pub async fn prerelease(&self, value: &str) -> Result<Value, Error> {
        uri(value)?;
        if !value.starts_with("spotify:prerelease:") {
            return Err(Error::InvalidInput("expected prerelease URI"));
        }
        self.0.gql("albumPreRelease", json!({"uri":value})).await
    }
    pub async fn lyrics(&self, value: &str) -> Result<Value, Error> {
        uri(value)?;
        self.0.request("GET",&format!("https://spclient.wg.spotify.com/color-lyrics/v2/track/{}?format=json&vocalRemoval=false&market=from_token",segment(value.rsplit(':').next().unwrap_or_default())),None).await
    }
    pub async fn canvas(&self, value: &str) -> Result<Value, Error> {
        uri(value)?;
        self.0.gql("canvas", json!({"trackUri":value})).await
    }
    pub async fn credits(&self, value: &str) -> Result<Value, Error> {
        uri(value)?;
        self.0
            .gql("queryTrackCreditsGroupedModal", json!({"trackUri":value}))
            .await
    }
    pub async fn radio(&self, value: &str) -> Result<Value, Error> {
        uri(value)?;
        self.0.request("GET",&format!("https://spclient.wg.spotify.com/inspiredby-mix/v2/seed_to_playlist/{}?response-format=json",segment(value)),None).await
    }
    pub async fn set_excluded_from_taste(
        &self,
        value: &str,
        enabled: bool,
    ) -> Result<Value, Error> {
        self.set_collection("ignoreinrecs", value, enabled).await
    }
    pub async fn set_artist_banned(&self, value: &str, enabled: bool) -> Result<Value, Error> {
        self.set_collection("artistban", value, enabled).await
    }
    async fn set_collection(&self, set: &str, value: &str, enabled: bool) -> Result<Value, Error> {
        uri(value)?;
        service(
            self.0,
            "CollectionPlatformAPI",
            if enabled { "add" } else { "remove" },
            vec![json!(set), json!([value])],
        )
        .await
    }
    pub async fn is_excluded_from_taste(&self, values: &[String]) -> Result<Value, Error> {
        self.collection_contains("ignoreinrecs", values).await
    }
    pub async fn is_artist_banned(&self, values: &[String]) -> Result<Value, Error> {
        self.collection_contains("artistban", values).await
    }
    async fn collection_contains(&self, set: &str, values: &[String]) -> Result<Value, Error> {
        for v in values {
            uri(v)?;
        }
        let me = self.0.users().me().await?;
        let username = me.username.as_str();
        let items: Vec<_> = values.iter().map(|v| json!({"uri":v})).collect();
        self.0
            .request(
                "POST",
                "https://spclient.wg.spotify.com/collection/v2/contains?market=from_token",
                Some(json!({"username":username,"set":set,"items":items})),
            )
            .await
    }
}

/// One Spotify Connect endpoint, as reported by `ConnectDevicesAPI`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub active: bool,
    pub local: bool,
    pub volume: Option<f64>,
    /// Spotify reports the device can decode lossless.
    pub lossless_supported: bool,
    /// Spotify reports this account may stream lossless on the device.
    pub lossless_eligible: bool,
    /// Spotify refuses commands to this device from other clients.
    #[serde(default)]
    pub restricted: bool,
    /// The device accepts volume changes.
    #[serde(default)]
    pub supports_volume: bool,
    pub raw: Value,
}

impl Device {
    fn from_cluster(id: &str, row: &Value, active: bool) -> Result<Self, Error> {
        Ok(Self {
            id: id.to_owned(),
            name: row["name"]
                .as_str()
                .ok_or_else(|| Error::Backend("missing Connect device name".into()))?
                .to_owned(),
            kind: row["device_type"]
                .as_str()
                .unwrap_or("unknown")
                .to_lowercase(),
            active,
            local: false,
            volume: if row["capabilities"]["disable_volume"] == true {
                None
            } else {
                row["volume"].as_f64().map(|v| v / 65535.0)
            },
            lossless_supported: row["capabilities"]["supports_lossless_audio"]
                .as_bool()
                .or_else(|| {
                    row.pointer("/capabilities/supports_hifi/device_supported")
                        .and_then(Value::as_bool)
                })
                .unwrap_or(false),
            lossless_eligible: row
                .pointer("/capabilities/supports_hifi/user_eligible")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            restricted: row["is_restricted"]
                .as_bool()
                .or(row["capabilities"]["is_restricted"].as_bool())
                .unwrap_or(false),
            supports_volume: !row["capabilities"]["disable_volume"]
                .as_bool()
                .unwrap_or(false),
            raw: row.clone(),
        })
    }

    fn parse(row: &Value) -> Option<Self> {
        let lossless = &row["losslessSupport"];
        Some(Self {
            id: row["id"].as_str()?.to_owned(),
            name: row["name"].as_str().unwrap_or_default().to_owned(),
            kind: row["type"].as_str().unwrap_or("unknown").to_owned(),
            active: row["isActive"].as_bool().unwrap_or(false),
            local: row["isLocal"].as_bool().unwrap_or(false),
            volume: row["volume"].as_f64(),
            lossless_supported: lossless["deviceSupported"].as_bool().unwrap_or(false),
            lossless_eligible: lossless["userEligible"].as_bool().unwrap_or(false),
            restricted: row["is_restricted"].as_bool().unwrap_or(false),
            supports_volume: row["supports_volume"].as_bool().unwrap_or(true),
            raw: row.clone(),
        })
    }
}

const PLAYBACK_SETTINGS: &str = "https://spclient.wg.spotify.com/playback-settings/spotify.playbacksettings.PlaybackSettingsService";

fn descriptor(device: &Device) -> Value {
    json!({"device_id": device.id, "device_name": device.name})
}

fn quality_request(device: &Device) -> Value {
    json!({"devices": [{"device_descriptor": descriptor(device)}]})
}

fn reported_quality(response: &Value) -> Option<AudioQuality> {
    AudioQuality::from_level_name(response["device_settings"][0]["audio_quality"].as_str()?)
}

/// Receiver-reported six-band EQ preferences, not a measurement of the audio signal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EqualizerSettings {
    pub enabled: bool,
    /// Low shelf, low peak, low-mid peak, high-mid peak, high peak, high shelf.
    /// Gains in decibels, from -12 through +12.
    pub gains: [f64; 6],
}

impl EqualizerSettings {
    /// A recognized curve, or `None` for Manual. No preset identity is stored by Spotify.
    pub fn preset(&self) -> Option<crate::EqualizerPreset> {
        crate::EqualizerPreset::from_gains(self.gains)
    }
}

fn equalizer_fields(gains: [f64; 6]) -> Result<Value, Error> {
    if gains
        .iter()
        .any(|gain| !gain.is_finite() || !(-12.0..=12.0).contains(gain))
    {
        return Err(Error::InvalidInput(
            "equalizer gains must be finite and between -12 and +12 dB",
        ));
    }
    Ok(Value::Array(
        [
            "EQUALIZER_LOW_SHELF_GAIN",
            "EQUALIZER_LOW_PEAK_GAIN",
            "EQUALIZER_LOW_MID_PEAK_GAIN",
            "EQUALIZER_HIGH_MID_PEAK_GAIN",
            "EQUALIZER_HIGH_PEAK_GAIN",
            "EQUALIZER_HIGH_SHELF_GAIN",
        ]
        .into_iter()
        .zip(gains)
        .map(|(id, gain)| json!({"id":id,"value":{"float_value":gain}}))
        .collect(),
    ))
}

fn reported_equalizer(response: &Value) -> Result<EqualizerSettings, Error> {
    let settings = response
        .pointer("/device_settings/0")
        .ok_or(Error::Unsupported)?;
    let enabled = settings["equalizer_enabled"]
        .as_bool()
        .ok_or(Error::Unsupported)?;
    let mut gains = [0.0; 6];
    for (gain, key) in gains.iter_mut().zip([
        "equalizer_low_shelf_gain",
        "equalizer_low_peak_gain",
        "equalizer_low_mid_peak_gain",
        "equalizer_high_mid_peak_gain",
        "equalizer_high_peak_gain",
        "equalizer_high_shelf_gain",
    ]) {
        *gain = settings[key]
            .as_f64()
            .filter(|gain| gain.is_finite())
            .ok_or(Error::Unsupported)?;
    }
    Ok(EqualizerSettings { enabled, gains })
}

fn quality_options(device: &Device, tier: AccountTier) -> Result<Vec<AudioQuality>, Error> {
    let caps = &device.raw["capabilities"];
    if device.restricted || caps["supports_remote_audio_quality_control"] != true {
        return Err(Error::Unsupported);
    }
    let maximum = caps["supported_audio_quality"]
        .as_str()
        .and_then(AudioQuality::from_level_name)
        .filter(|q| *q != AudioQuality::Auto)
        .ok_or(Error::Unsupported)?;
    let account_max = match tier {
        AccountTier::Premium => {
            if device.lossless_supported && device.lossless_eligible {
                AudioQuality::HiRes
            } else {
                AudioQuality::VeryHigh
            }
        }
        AccountTier::Free => AudioQuality::High,
        _ => return Err(Error::Unsupported),
    };
    let maximum = maximum.as_level().min(account_max.as_level());
    Ok([
        AudioQuality::Auto,
        AudioQuality::Low,
        AudioQuality::Normal,
        AudioQuality::High,
        AudioQuality::VeryHigh,
        AudioQuality::Lossless,
        AudioQuality::HiRes,
    ]
    .into_iter()
    .filter(|q| q.as_level() <= maximum)
    .collect())
}

pub struct Devices<'a>(pub(crate) &'a Api);
impl Devices<'_> {
    /// The account's active Connect receiver, independent of the local audio engine.
    pub async fn playback_state(&self) -> Result<Option<spotcore_core::PlayerSnapshot>, Error> {
        let cluster = crate::direct::cluster(self.0).await?;
        let mut state = crate::mapping::snapshot(&crate::connect::snapshot(&cluster))?;
        if let Some(state) = &mut state {
            state.capabilities = crate::connect::capabilities(&cluster);
        }
        Ok(state)
    }
    /// Plays an ordered context without adding its remaining items to the manual queue.
    pub async fn play_items(
        &self,
        target: Option<&str>,
        context: Option<&str>,
        uris: &[String],
        position: std::time::Duration,
    ) -> Result<(), Error> {
        if uris.is_empty() || uris.len() > 1000 {
            return Err(Error::InvalidInput(
                "playback list must contain 1 to 1000 items",
            ));
        }
        for item in uris {
            uri(item)?;
        }
        let context = context.unwrap_or(&uris[0]);
        uri(context)?;
        let position = u64::try_from(position.as_millis())
            .map_err(|_| Error::InvalidInput("invalid play position"))?;
        if position > 9_007_199_254_740_991 {
            return Err(Error::InvalidInput("invalid play position"));
        }
        crate::connect::call_to(
            self.0,
            target,
            "player.play",
            json!({
                "contextUri":context, "order":uris, "index":0, "positionMs":position
            }),
        )
        .await?;
        Ok(())
    }

    /// Fast queue state without network metadata enrichment; context and row identities share one cluster read.
    pub async fn queue_snapshot(&self) -> Result<spotcore_core::Queue, Error> {
        let mut queue =
            crate::mapping::queue(&crate::connect::call(self.0, "player.queue", json!({})).await?)?;
        for item in queue
            .current
            .iter_mut()
            .chain(&mut queue.queued)
            .chain(&mut queue.next_up)
        {
            if sparse_media(item)
                && let Some(full) = self.0.cached_media(&item.uri)
            {
                fill_media_metadata(item, full);
            }
        }
        Ok(queue)
    }

    /// The account's current queue over HTTP, without a browser player.
    pub async fn queue(&self) -> Result<spotcore_core::Queue, Error> {
        let mut queue = self.queue_snapshot().await?;
        hydrate_media(
            self.0,
            queue
                .current
                .iter_mut()
                .chain(&mut queue.queued)
                .chain(&mut queue.next_up)
                .collect(),
        )
        .await;
        Ok(queue)
    }
    /// Edits one exact visible occurrence on the named active Connect receiver.
    /// No anchors or membership means removal; membership alone changes section in place.
    pub async fn edit_queue_occurrence(
        &self,
        device: &str,
        item: spotcore_core::QueueItem,
        before: Option<spotcore_core::QueueItem>,
        after: Option<spotcore_core::QueueItem>,
        manual: Option<bool>,
    ) -> Result<(), Error> {
        let snapshot = self.0.queues().snapshot().await?;
        if snapshot.scope.as_ref().map(|scope| scope.receiver.as_str()) != Some(device) {
            return Err(Error::Backend(
                "queue receiver changed; refresh before editing".into(),
            ));
        }
        if before.is_none() && after.is_none() && manual.is_none() {
            self.0.queues().remove(&snapshot, &[item]).await
        } else {
            self.0
                .queues()
                .move_items(&snapshot, &[item], before, after, manual)
                .await
        }
    }

    /// Inserts tracks at an exact visible slot on the named active receiver.
    /// Without an anchor, inserts at the manual-queue head.
    pub async fn insert_queue_items(
        &self,
        device: &str,
        uris: &[String],
        before: Option<spotcore_core::QueueItem>,
        after: Option<spotcore_core::QueueItem>,
        manual: bool,
    ) -> Result<(), Error> {
        let snapshot = self.0.queues().snapshot().await?;
        if snapshot.scope.as_ref().map(|scope| scope.receiver.as_str()) != Some(device) {
            return Err(Error::Backend(
                "queue receiver changed; refresh before editing".into(),
            ));
        }
        self.0
            .queues()
            .insert(&snapshot, uris, before, after, manual)
            .await
    }

    /// Replaces manual entries while preserving context and reusable occurrence identities.
    pub async fn replace_queued(&self, device: &str, uris: &[String]) -> Result<(), Error> {
        let snapshot = self.0.queues().snapshot().await?;
        if snapshot.scope.as_ref().map(|scope| scope.receiver.as_str()) != Some(device) {
            return Err(Error::Backend(
                "queue receiver changed; refresh before editing".into(),
            ));
        }
        self.0
            .queues()
            .write(&snapshot, "queue.replace", json!({"uris":uris}))
            .await
    }
    pub async fn list(&self) -> Result<Vec<Device>, Error> {
        if self.0.browser().is_err() {
            let cluster = crate::direct::cluster(self.0).await?;
            let Some(rows) = cluster.get("devices") else {
                return Ok(Vec::new());
            };
            let rows = rows
                .as_object()
                .ok_or_else(|| Error::Backend("invalid Connect devices".into()))?;
            return rows
                .iter()
                .map(|(id, row)| Device::from_cluster(id, row, cluster["active_device_id"] == *id))
                .collect();
        }
        let raw = service(self.0, "ConnectDevicesAPI", "getDevices", vec![]).await?;
        let rows = raw
            .as_array()
            .ok_or_else(|| Error::Backend("device list was not an array".into()))?;
        Ok(rows.iter().filter_map(Device::parse).collect())
    }
    /// Reads the six-band device-settings preference. Absent/unknown fields return `Unsupported`.
    pub async fn equalizer(&self, device: &Device) -> Result<EqualizerSettings, Error> {
        let response = self
            .0
            .request(
                "POST",
                &format!("{PLAYBACK_SETTINGS}/GetDeviceSettings"),
                Some(quality_request(device)),
            )
            .await?;
        reported_equalizer(&response)
    }

    /// Changes only EQ's enabled state, preserving all band gains, then confirms readback.
    /// This confirms a stored preference, not audible signal processing.
    pub async fn set_equalizer_enabled(
        &self,
        device: &Device,
        enabled: bool,
    ) -> Result<EqualizerSettings, Error> {
        let current = self
            .list()
            .await?
            .into_iter()
            .find(|current| current.id == device.id)
            .ok_or(Error::Disconnected)?;
        if current.restricted {
            return Err(Error::Unsupported);
        }
        self.equalizer(&current).await?;
        self.0
            .request(
                "POST",
                &format!("{PLAYBACK_SETTINGS}/WriteDeviceValue"),
                Some(json!({
                    "device_descriptor":descriptor(&current),
                    "field":[{"id":"EQUALIZER_ENABLED","value":{"bool_value":enabled}}],
                })),
            )
            .await?;
        let confirmed = self.equalizer(&current).await?;
        if confirmed.enabled != enabled {
            return Err(Error::Backend("Spotify has not confirmed the equalizer change; read the current setting before retrying".into()));
        }
        Ok(confirmed)
    }

    /// Writes all six gains in dB without changing the enabled state, then verifies readback.
    /// A failed confirmation can leave a partial change. Refresh before retrying.
    /// Stored settings do not prove the receiver is applying audible processing.
    pub async fn set_equalizer_gains(
        &self,
        device: &Device,
        gains: [f64; 6],
    ) -> Result<EqualizerSettings, Error> {
        let fields = equalizer_fields(gains)?;
        let current = self
            .list()
            .await?
            .into_iter()
            .find(|current| current.id == device.id)
            .ok_or(Error::Disconnected)?;
        if current.restricted {
            return Err(Error::Unsupported);
        }
        let previous = self.equalizer(&current).await?;
        self.0
            .request(
                "POST",
                &format!("{PLAYBACK_SETTINGS}/WriteDeviceValue"),
                Some(json!({"device_descriptor":descriptor(&current),"field":fields})),
            )
            .await?;
        let confirmed = self.equalizer(&current).await?;
        if confirmed.enabled != previous.enabled
            || !confirmed
                .gains
                .iter()
                .zip(gains)
                .all(|(a, b)| (a - b).abs() <= 0.001)
        {
            return Err(Error::Backend(
                "Spotify has not confirmed the equalizer bands; refresh before retrying".into(),
            ));
        }
        Ok(confirmed)
    }

    /// Applies a built-in Spotify curve while preserving the EQ enabled state.
    pub async fn set_equalizer_preset(
        &self,
        device: &Device,
        preset: crate::EqualizerPreset,
    ) -> Result<EqualizerSettings, Error> {
        self.set_equalizer_gains(device, preset.gains()).await
    }

    /// Verified remote-control support and receiver/account limits, not delivered stream formats.
    /// Unknown capabilities or account tier return `Unsupported`; missing lossless eligibility excludes lossless.
    pub async fn audio_quality_options(&self, device: &Device) -> Result<Vec<AudioQuality>, Error> {
        quality_options(device, self.0.users().me().await?.tier)
    }
    /// Reads the audio-quality setting Spotify holds for one device.
    pub async fn audio_quality(&self, device: &Device) -> Result<AudioQuality, Error> {
        let response = self
            .0
            .request(
                "POST",
                &format!("{PLAYBACK_SETTINGS}/GetDeviceSettings"),
                Some(quality_request(device)),
            )
            .await?;
        reported_quality(&response)
            .ok_or_else(|| Error::Backend("device reported an unknown audio quality".into()))
    }
    /// Writes the device's audio-quality setting and returns what Spotify reports afterwards.
    pub async fn set_audio_quality(
        &self,
        device: &Device,
        quality: AudioQuality,
    ) -> Result<AudioQuality, Error> {
        let current = self
            .list()
            .await?
            .into_iter()
            .find(|current| current.id == device.id)
            .ok_or(Error::Disconnected)?;
        if !self
            .audio_quality_options(&current)
            .await?
            .contains(&quality)
        {
            return Err(Error::InvalidInput(
                "audio quality is unavailable for this account and receiver",
            ));
        }
        self.0
            .request(
                "POST",
                &format!("{PLAYBACK_SETTINGS}/WriteDeviceValue"),
                Some(json!({
                    "device_descriptor": descriptor(&current),
                    "field": [{"id": "AUDIO_QUALITY", "value": {"int_value": quality.as_level()}}],
                })),
            )
            .await?;
        self.audio_quality(&current).await
    }
    pub async fn transfer(&self, device_id: &str, play: bool) -> Result<Value, Error> {
        nonempty(device_id)?;
        if self.0.browser().is_err() {
            return crate::direct::transfer(self.0, device_id, play).await;
        }
        service(
            self.0,
            "ConnectTransferAPI",
            "transferPlayback",
            vec![json!(device_id), json!({"playOnTransfer":play})],
        )
        .await
    }
    pub async fn pull_here(&self) -> Result<Value, Error> {
        service(
            self.0,
            "ConnectTransferAPI",
            "pullToLocalDevice",
            vec![json!({})],
        )
        .await
    }
}

pub struct Jam<'a>(pub(crate) &'a Api);
impl Jam<'_> {
    /// Explicit creation only. This is never called from refresh, invitations or restoration.
    pub async fn start_session(&self) -> Result<JamSession, Error> {
        self.0
            .users()
            .availability()
            .await?
            .jam_hosting
            .require("jam_hosting")?;
        if let Some(session) = self.state().await? {
            if !session.is_owner {
                return Err(Error::InvalidInput(
                    "leave the current Jam before starting another",
                ));
            }
            return Ok(session);
        }
        let created = self.start().await?;
        JamSession::parse(created)?
            .ok_or_else(|| Error::Backend("Spotify did not confirm the created Jam".into()))
    }
    pub async fn invitation_url(&self, id: &str) -> Result<String, Error> {
        let value = self.invite_link(id).await?;
        value
            .get("shareableUrl")
            .and_then(Value::as_str)
            .filter(|value| {
                url::Url::parse(value).is_ok_and(|url| {
                    url.scheme() == "https"
                        && matches!(
                            url.host_str(),
                            Some("open.spotify.com" | "spotify.link" | "spotify.app.link")
                        )
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.port().is_none()
                })
            })
            .map(str::to_owned)
            .ok_or_else(|| Error::Backend("Spotify returned no valid Jam invitation".into()))
    }
    pub async fn availability(&self) -> Result<Value, Error> {
        service(self.0, "SocialConnectAPI", "getEnabled", vec![]).await
    }
    pub async fn session(&self) -> Result<Value, Error> {
        crate::jam::current(self.0, false).await
    }
    pub async fn refresh(&self) -> Result<Option<JamSession>, Error> {
        JamSession::parse(crate::jam::current(self.0, true).await?)
    }
    pub async fn state(&self) -> Result<Option<JamSession>, Error> {
        JamSession::parse(self.session().await?)
    }
    pub async fn join_on(
        &self,
        token: &str,
        target: ListenOn,
    ) -> Result<Option<JamSession>, Error> {
        let receiver = if target == ListenOn::ThisDevice {
            match self.0.local_receiver() {
                Some(receiver) => receiver,
                None => {
                    let mut devices = self
                        .0
                        .devices()
                        .list()
                        .await?
                        .into_iter()
                        .filter(|device| device.local);
                    let device = devices.next().ok_or(Error::InvalidInput(
                        "this SDK instance has no ready local receiver",
                    ))?;
                    if devices.next().is_some() {
                        return Err(Error::InvalidInput("local receiver is ambiguous"));
                    }
                    device.id
                }
            }
        } else {
            self.0
                .local_receiver()
                .unwrap_or_else(|| self.0.device_id().to_owned())
        };
        self.join_with_receiver(token, target, &receiver).await
    }
    pub async fn join_with_receiver(
        &self,
        token: &str,
        target: ListenOn,
        receiver: &str,
    ) -> Result<Option<JamSession>, Error> {
        let token = jam_token(token)?;
        JamSession::parse(
            crate::jam::join(self.0, &token, receiver, target == ListenOn::ThisDevice).await?,
        )
    }
    pub async fn leave_session(&self, id: &str) -> Result<(), Error> {
        nonempty(id)?;
        crate::jam::finish(self.0, Some(id), false).await?;
        Ok(())
    }
    pub async fn end_session(&self, id: &str) -> Result<(), Error> {
        nonempty(id)?;
        crate::jam::finish(self.0, Some(id), true).await?;
        Ok(())
    }
    pub async fn start(&self) -> Result<Value, Error> {
        let enabled = self.availability().await?;
        if !matches!(enabled.as_str(), Some("ENABLED" | "AVAILABLE")) {
            return Err(Error::Unsupported);
        }
        service(self.0, "SocialConnectAPI", "createSession", vec![]).await
    }
    /// Join a Jam as a guest. Accepts an invite link, a `spotify:socialsession:`
    /// URI, or a bare join token.
    pub async fn join(&self, token: &str) -> Result<Value, Error> {
        Ok(self
            .join_on(token, ListenOn::Host)
            .await?
            .map_or(Value::Null, |session| session.raw))
    }
    /// Joins following the host, or using this instance's ready receiver.
    pub async fn join_as(&self, token: &str, listen_on_device: bool) -> Result<Value, Error> {
        let target = if listen_on_device {
            ListenOn::ThisDevice
        } else {
            ListenOn::Host
        };
        Ok(self
            .join_on(token, target)
            .await?
            .map_or(Value::Null, |session| session.raw))
    }
    /// Preview the Jam behind an invite link or token without joining it.
    pub async fn session_info(&self, token: &str) -> Result<Value, Error> {
        let token = jam_token(token)?;
        service(
            self.0,
            "SocialConnectAPI",
            "getSessionInfo",
            vec![json!(token)],
        )
        .await
    }
    pub async fn leave(&self) -> Result<Value, Error> {
        crate::jam::finish(self.0, None, false).await
    }
    pub async fn end(&self) -> Result<Value, Error> {
        crate::jam::finish(self.0, None, true).await
    }
    pub async fn invite_link(&self, session_id: &str) -> Result<Value, Error> {
        nonempty(session_id)?;
        service(
            self.0,
            "SocialConnectAPI",
            "getShortInviteLink",
            vec![json!(session_id)],
        )
        .await
    }
    pub async fn remove_member(&self, value: &str) -> Result<Value, Error> {
        uri(value)?;
        service(
            self.0,
            "SocialConnectAPI",
            "removeSessionMember",
            vec![json!(value.rsplit(':').next().unwrap_or_default())],
        )
        .await
    }
    pub async fn set_queue_only(&self, enabled: bool) -> Result<Value, Error> {
        service(
            self.0,
            "SocialConnectAPI",
            "setQueueOnlyMode",
            vec![json!(enabled)],
        )
        .await
    }
    pub async fn set_participant_volume_control(&self, enabled: bool) -> Result<Value, Error> {
        service(
            self.0,
            "SocialConnectAPI",
            "setParticipantVolumeControl",
            vec![json!(enabled)],
        )
        .await
    }
}

pub struct Blends<'a>(pub(crate) &'a Api);
impl Blends<'_> {
    pub async fn invite(&self) -> Result<Value, Error> {
        self.0
            .request(
                "POST",
                "https://spclient.wg.spotify.com/blend-invitation/v1/generate",
                None,
            )
            .await
    }
    pub async fn invitation(&self, link_or_id: &str) -> Result<Value, Error> {
        self.0
            .request(
                "GET",
                &format!(
                    "https://spclient.wg.spotify.com/blend-invitation/v3/view-invitation/{}",
                    blend_id(link_or_id)?
                ),
                None,
            )
            .await
    }
    pub async fn join(&self, link_or_id: &str) -> Result<Value, Error> {
        self.0
            .request(
                "PUT",
                &format!(
                    "https://spclient.wg.spotify.com/blend-invitation/v2/join/{}",
                    blend_id(link_or_id)?
                ),
                None,
            )
            .await
    }
    pub async fn group(&self, value: &str) -> Result<Value, Error> {
        self.read_playlist("get-invitation", value).await
    }
    pub async fn story(&self, value: &str) -> Result<Value, Error> {
        self.read_playlist("data-stories", value).await
    }
    async fn read_playlist(&self, method: &str, value: &str) -> Result<Value, Error> {
        uri(value)?;
        self.0
            .request(
                "GET",
                &format!(
                    "https://spclient.wg.spotify.com/blend-invitation/v3/{method}/{}",
                    segment(value.rsplit(':').next().unwrap_or_default())
                ),
                None,
            )
            .await
    }
    pub async fn rename(&self, value: &str, name: &str) -> Result<Value, Error> {
        uri(value)?;
        nonempty(name)?;
        self.0
            .request(
                "PUT",
                &format!(
                    "https://spclient.wg.spotify.com/blend-writer/v1/rename/{}",
                    segment(value.rsplit(':').next().unwrap_or_default())
                ),
                Some(json!({"name":name})),
            )
            .await
    }
    pub async fn leave(&self, value: &str) -> Result<Value, Error> {
        uri(value)?;
        self.0
            .request(
                "PUT",
                "https://spclient.wg.spotify.com/blend-invitation/v1/leave",
                Some(json!({"playlist_uri":value})),
            )
            .await
    }
}
/// Resolve an invite link or `spotify:socialsession:` URI to its join token.
/// Bare tokens pass through after a non-empty check.
fn jam_token(value: &str) -> Result<String, Error> {
    let value = value.trim();
    let token = if let Some(id) = value.strip_prefix("spotify:socialsession:") {
        id.to_owned()
    } else if let Ok(url) = url::Url::parse(value) {
        if url.scheme() != "https" || url.host_str() != Some("open.spotify.com") {
            return Err(Error::InvalidInput("invalid Jam invitation URL"));
        }
        let parts: Vec<_> = url
            .path_segments()
            .ok_or(Error::InvalidInput("invalid Jam invitation URL"))?
            .collect();
        match parts.as_slice() {
            ["socialsession", id] | ["socialsession", id, ""] => (*id).to_owned(),
            _ => return Err(Error::InvalidInput("invalid Jam invitation path")),
        }
    } else {
        value.to_owned()
    };
    nonempty(&token)?;
    Ok(token)
}

fn blend_id(value: &str) -> Result<String, Error> {
    let value = value.trim();
    let id = if let Ok(url) = url::Url::parse(value) {
        if url.scheme() != "https"
            || url.host_str() != Some("open.spotify.com")
            || !url.username().is_empty()
        {
            return Err(Error::InvalidInput("invalid Blend invitation URL"));
        }
        let parts: Vec<_> = url
            .path_segments()
            .ok_or(Error::InvalidInput("invalid Blend invitation URL"))?
            .collect();
        match parts.as_slice() {
            ["blend", "taste-match" | "invitation", id] => id.to_string(),
            _ => return Err(Error::InvalidInput("invalid Blend invitation path")),
        }
    } else {
        value.into()
    };
    if id.len() < 8 || !id.bytes().all(|c| c.is_ascii_alphanumeric()) {
        return Err(Error::InvalidInput("invalid Blend invitation ID"));
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    #[test]
    fn equalizer_requires_explicit_enable_and_all_six_band_values() {
        let mut response = json!({"device_settings":[{
            "equalizer_enabled":false,"equalizer_low_shelf_gain":0.0,
            "equalizer_low_peak_gain":1.0,"equalizer_low_mid_peak_gain":2.0,
            "equalizer_high_mid_peak_gain":3.0,"equalizer_high_peak_gain":4.0,
            "equalizer_high_shelf_gain":5.0
        }]});
        let state = reported_equalizer(&response).unwrap();
        assert!(!state.enabled);
        assert_eq!(state.gains, [0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
        response["device_settings"][0]["equalizer_high_shelf_gain"] = Value::Null;
        assert!(reported_equalizer(&response).is_err());
        assert!(reported_equalizer(&json!({"device_settings":[{}]})).is_err());
    }

    #[test]
    fn quality_choices_intersect_receiver_tier_and_explicit_entitlement() {
        let row = json!({"name":"Receiver","capabilities":{"supports_remote_audio_quality_control":true,"supported_audio_quality":"HIFI","supports_hifi":{"device_supported":true}}});
        let mut device = Device::from_cluster("owned", &row, true).unwrap();
        assert!(device.lossless_supported);
        assert!(!device.lossless_eligible);
        assert_eq!(
            quality_options(&device, AccountTier::Premium)
                .unwrap()
                .last(),
            Some(&AudioQuality::VeryHigh)
        );
        assert_eq!(
            quality_options(&device, AccountTier::Free).unwrap().last(),
            Some(&AudioQuality::High)
        );
        assert!(quality_options(&device, AccountTier::Unknown).is_err());
        device.lossless_eligible = true;
        assert_eq!(
            quality_options(&device, AccountTier::Premium)
                .unwrap()
                .last(),
            Some(&AudioQuality::Lossless)
        );
        device.raw["capabilities"]["supported_audio_quality"] = json!("NORMAL");
        assert_eq!(
            quality_options(&device, AccountTier::Premium)
                .unwrap()
                .last(),
            Some(&AudioQuality::Normal)
        );
        device.raw["capabilities"]["supported_audio_quality"] = json!("UNKNOWN_FUTURE_LEVEL");
        assert!(quality_options(&device, AccountTier::Premium).is_err());
        device.raw["capabilities"]["supported_audio_quality"] = json!("HIFI");
        device.raw["capabilities"]["supports_remote_audio_quality_control"] = json!(false);
        assert!(quality_options(&device, AccountTier::Premium).is_err());
        device.raw["capabilities"]["supports_remote_audio_quality_control"] = json!(true);
        device.restricted = true;
        assert!(quality_options(&device, AccountTier::Premium).is_err());
    }

    #[test]
    fn profile_name_update_preserves_wrapper_and_field_mask_wire_format() {
        assert_eq!(
            super::profile_name_update("Alex").unwrap(),
            [
                10, 6, 10, 4, 110, 97, 109, 101, 18, 8, 18, 6, 10, 4, 65, 108, 101, 120
            ]
        );
        assert!(super::profile_name_update(" ").is_err());
        assert!(super::profile_name_update(&"a".repeat(33)).is_err());
        assert!(super::profile_name_update(&"🎵".repeat(32)).is_ok());
    }

    use super::*;
    use serde_json::json;

    #[test]
    fn jam_token_accepts_links_uris_and_bare_tokens() {
        assert_eq!(
            jam_token("https://open.spotify.com/socialsession/abc123").unwrap(),
            "abc123"
        );
        assert_eq!(
            jam_token("https://open.spotify.com/socialsession/abc123/").unwrap(),
            "abc123"
        );
        assert_eq!(jam_token("spotify:socialsession:abc123").unwrap(), "abc123");
        assert_eq!(jam_token("  abc123  ").unwrap(), "abc123");
    }

    #[test]
    fn jam_token_rejects_wrong_host_scheme_and_path() {
        for bad in [
            "http://open.spotify.com/socialsession/abc123",
            "https://evil.example/socialsession/abc123",
            "https://open.spotify.com/jam/context/abc123",
            "https://open.spotify.com/socialsession/abc123/extra",
            " ",
        ] {
            assert!(jam_token(bad).is_err(), "{bad:?} was accepted");
        }
    }

    #[test]
    fn queue_metadata_hydration_preserves_duplicate_identity() {
        let mut sparse =
            crate::mapping::item(&json!({"uri":"spotify:track:original", "uid":"second-copy"}))
                .unwrap();
        let full = crate::mapping::item(&json!({"uri":"spotify:track:relinked", "name":"Song", "durationMs":123000,
            "artists":[{"name":"Artist"}], "album":{"name":"Album","images":[{"url":"https://example.com/cover"}]}})).unwrap();
        fill_media_metadata(&mut sparse, full);
        assert_eq!(sparse.uri, "spotify:track:original");
        assert_eq!(sparse.uid.as_deref(), Some("second-copy"));
        assert_eq!(sparse.raw["uri"], "spotify:track:original");
        assert_eq!(sparse.raw["uid"], "second-copy");
        assert_eq!(sparse.duration_ms, Some(123000));
        assert_eq!(sparse.images[0].url, "https://example.com/cover");
        assert_eq!(sparse.raw["artists"][0]["name"], "Artist");
        assert!(!sparse_media(&sparse));
    }

    #[test]
    fn device_quality_flags_come_from_connect() {
        let device = Device::parse(&json!({
            "id": "abc",
            "name": "Soloist",
            "type": "computer",
            "losslessSupport": {"deviceSupported": true, "userEligible": false},
        }))
        .unwrap();
        assert!(device.lossless_supported);
        assert!(!device.lossless_eligible);
        let plain = Device::parse(&json!({"id": "abc"})).unwrap();
        assert!(!plain.lossless_supported);
        assert!(!plain.lossless_eligible);
    }

    #[test]
    fn cluster_rows_map_to_devices() {
        let row = json!({
            "name": "hpdesk",
            "device_type": "Computer",
            "volume": 32768,
            "capabilities": {"supports_lossless_audio": true},
        });
        let device = Device::from_cluster("abc", &row, true).unwrap();
        assert_eq!(device.name, "hpdesk");
        assert_eq!(device.kind, "computer");
        assert!(device.active);
        assert!(!device.local);
        assert!((device.volume.unwrap() - 0.5).abs() < 0.001);
        assert!(device.lossless_supported);
        assert!(!device.lossless_eligible);
        assert_eq!(device.raw, row);

        let muted = json!({"name": "tv", "capabilities": {"disable_volume": true}});
        let device = Device::from_cluster("abc", &muted, false).unwrap();
        assert_eq!(device.volume, None);
        assert_eq!(device.kind, "unknown");
        assert!(!device.lossless_supported);
        assert!(Device::from_cluster("abc", &json!({}), false).is_err());
    }

    #[test]
    fn playback_settings_request_wraps_the_device_descriptor() {
        let device =
            Device::from_cluster("receiver-id", &json!({"name": "SpotDesk"}), false).unwrap();
        assert_eq!(
            quality_request(&device),
            json!({"devices": [{"device_descriptor": {
                "device_id": "receiver-id", "device_name": "SpotDesk"
            }}]})
        );
    }

    #[test]
    fn equalizer_bands_validate_before_building_a_six_field_write() {
        let gains = crate::EqualizerPreset::BassBooster.gains();
        let fields = equalizer_fields(gains).unwrap();
        let fields = fields.as_array().unwrap();
        assert_eq!(fields.len(), 6);
        assert_eq!(
            fields[0],
            json!({"id":"EQUALIZER_LOW_SHELF_GAIN","value":{"float_value":4.25}})
        );
        assert_eq!(
            fields[5],
            json!({"id":"EQUALIZER_HIGH_SHELF_GAIN","value":{"float_value":0.0}})
        );
        assert!(equalizer_fields([-12.0; 6]).is_ok());
        assert!(equalizer_fields([12.0; 6]).is_ok());
        for bad in [f64::NAN, f64::INFINITY, -12.001, 12.001] {
            let mut invalid = gains;
            invalid[3] = bad;
            assert!(equalizer_fields(invalid).is_err());
        }
    }

    #[test]
    fn playback_settings_response_maps_to_a_quality() {
        for (name, quality) in [
            ("DEFAULT", AudioQuality::Auto),
            ("VERY_HIGH", AudioQuality::VeryHigh),
            ("HIFI", AudioQuality::Lossless),
            ("HIFI_24", AudioQuality::HiRes),
        ] {
            let response = json!({"device_settings": [{"audio_quality": name}]});
            assert_eq!(reported_quality(&response), Some(quality));
        }
        assert_eq!(reported_quality(&json!({"device_settings": [{}]})), None);
        assert_eq!(
            reported_quality(&json!({"device_settings": [{"audio_quality": "PETREL"}]})),
            None
        );
    }
}

#[cfg(test)]
mod time_range_tests {
    use super::*;
    #[test]
    fn affinity_uses_the_private_api_mid_term_enum() {
        assert_eq!(TimeRange::MediumTerm.gql(), "MID_TERM");
    }
}
