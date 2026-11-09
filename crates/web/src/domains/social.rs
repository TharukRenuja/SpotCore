use super::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use spotcore_auth::AccountTier;
use spotcore_core::{AudioQuality, Paging};

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
impl Users<'_> {
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
                .unwrap_or(false),
            lossless_eligible: false,
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

fn reported_quality(response: &Value) -> Option<AudioQuality> {
    AudioQuality::from_level_name(response["device_settings"][0]["audio_quality"].as_str()?)
}

pub struct Devices<'a>(pub(crate) &'a Api);
impl Devices<'_> {
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
    /// Reads the audio-quality setting Spotify holds for one device.
    pub async fn audio_quality(&self, device: &Device) -> Result<AudioQuality, Error> {
        let response = self
            .0
            .request(
                "POST",
                &format!("{PLAYBACK_SETTINGS}/GetDeviceSettings"),
                Some(json!({"devices": [descriptor(device)]})),
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
        self.0
            .request(
                "POST",
                &format!("{PLAYBACK_SETTINGS}/WriteDeviceValue"),
                Some(json!({
                    "device_descriptor": descriptor(device),
                    "field": [{"id": "AUDIO_QUALITY", "value": {"int_value": quality.as_level()}}],
                })),
            )
            .await?;
        self.audio_quality(device).await
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
    pub async fn availability(&self) -> Result<Value, Error> {
        service(self.0, "SocialConnectAPI", "getEnabled", vec![]).await
    }
    pub async fn session(&self) -> Result<Value, Error> {
        service(self.0, "SocialConnectAPI", "fetchCurrentSession", vec![]).await
    }
    pub async fn start(&self) -> Result<Value, Error> {
        let enabled = self.availability().await?;
        if !matches!(enabled.as_str(), Some("ENABLED" | "AVAILABLE")) {
            return Err(Error::Unsupported);
        }
        service(self.0, "SocialConnectAPI", "createSession", vec![]).await
    }
    pub async fn join(&self, token: &str) -> Result<Value, Error> {
        nonempty(token)?;
        service(
            self.0,
            "SocialConnectAPI",
            "joinSession",
            vec![json!(token)],
        )
        .await
    }
    pub async fn leave(&self) -> Result<Value, Error> {
        service(self.0, "SocialConnectAPI", "leaveSession", vec![]).await
    }
    pub async fn end(&self) -> Result<Value, Error> {
        service(self.0, "SocialConnectAPI", "deleteSession", vec![]).await
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
    use super::*;
    use serde_json::json;

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
