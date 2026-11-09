#[boltffi::data]
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionOptions {
    pub app_id: String,
    pub device_name: String,
    pub playback: bool,
    pub data_dir: Option<String>,
}

#[boltffi::data]
#[derive(Clone, Debug, serde::Serialize)]
pub struct Account {
    pub username: String,
    pub name: String,
    pub tier: String,
    pub country: Option<String>,
}
impl From<spotcore::web::Account> for Account {
    fn from(value: spotcore::web::Account) -> Self {
        Self {
            username: value.username,
            name: value.name,
            tier: match value.tier {
                spotcore::AccountTier::Free => "free",
                spotcore::AccountTier::Premium => "premium",
                _ => "unknown",
            }
            .into(),
            country: value.country,
        }
    }
}

#[boltffi::data]
#[derive(Clone, Debug, serde::Serialize)]
pub struct Media {
    pub uri: String,
    pub name: String,
    pub image_urls: Vec<String>,
    pub duration_ms: Option<u64>,
}
impl From<spotcore::MediaItem> for Media {
    fn from(value: spotcore::MediaItem) -> Self {
        Self {
            uri: value.uri,
            name: value.name,
            image_urls: value.images.into_iter().map(|image| image.url).collect(),
            duration_ms: value.duration_ms,
        }
    }
}

#[boltffi::data]
#[derive(Clone, Debug, serde::Serialize)]
pub struct SearchPage {
    pub items: Vec<Media>,
    pub total: Option<u64>,
    pub next_offset: Option<u32>,
}

#[boltffi::data]
#[derive(Clone, Debug, serde::Serialize)]
pub struct SessionInfo {
    pub account: Account,
    pub engine: String,
    pub fallback_reason: Option<String>,
}

#[boltffi::data]
#[derive(Clone, Debug, serde::Serialize)]
pub struct PlaybackUpdate {
    pub kind: String,
    pub state: Option<String>,
    pub item: Option<Media>,
    pub position_ms: Option<u64>,
}
impl From<spotcore::PlaybackEvent> for PlaybackUpdate {
    fn from(event: spotcore::PlaybackEvent) -> Self {
        let mut update = Self {
            kind: String::new(),
            state: None,
            item: None,
            position_ms: None,
        };
        match event {
            spotcore::PlaybackEvent::Snapshot(snapshot) => {
                update.kind = "snapshot".into();
                update.state = Some(format!("{:?}", snapshot.state).to_lowercase());
                update.item = snapshot.track.map(Into::into);
                update.position_ms = Some(snapshot.position_ms);
            }
            spotcore::PlaybackEvent::StateChanged(state) => {
                update.kind = "state".into();
                update.state = Some(format!("{state:?}").to_lowercase());
            }
            spotcore::PlaybackEvent::EndOfTrack => update.kind = "end_of_track".into(),
            spotcore::PlaybackEvent::TrackUnavailable(_) => {
                update.kind = "track_unavailable".into()
            }
            spotcore::PlaybackEvent::QueueChanged(_) => update.kind = "queue".into(),
            spotcore::PlaybackEvent::CapabilitiesChanged(_) => update.kind = "capabilities".into(),
            spotcore::PlaybackEvent::SessionChanged => update.kind = "session".into(),
            _ => update.kind = "other".into(),
        }
        update
    }
}
