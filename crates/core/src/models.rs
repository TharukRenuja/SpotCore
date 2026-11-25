use crate::{Capabilities, PlaybackState};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Image {
    pub url: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum MediaKind {
    Track,
    Episode,
    Album,
    Artist,
    Playlist,
    Show,
    User,
    Folder,
    Ad,
    Unknown,
}

/// Normalized common metadata, plus the full upstream entity for less common fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaItem {
    pub uri: String,
    pub name: String,
    pub kind: MediaKind,
    pub images: Vec<Image>,
    pub duration_ms: Option<u64>,
    pub uid: Option<String>,
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub offset: u32,
    pub total: Option<u64>,
    /// Advances by raw rows, including rows with unrecognized metadata.
    pub next_offset: Option<u32>,
}

/// A collection page whose continuation is opaque to the caller.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CursorPage<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Paging {
    pub offset: u32,
    pub limit: u32,
}
impl Default for Paging {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: 30,
        }
    }
}
impl Paging {
    pub fn validate(self) -> Result<Self, crate::Error> {
        if self.limit == 0 || self.limit > 100 {
            return Err(crate::Error::InvalidInput("page limit must be 1..=100"));
        }
        self.offset
            .checked_add(self.limit)
            .ok_or(crate::Error::InvalidInput("page offset overflow"))?;
        Ok(self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlayerSnapshot {
    pub state: PlaybackState,
    pub track: Option<MediaItem>,
    pub context_uri: Option<String>,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    pub timestamp_ms: Option<u64>,
    /// Active Connect device metadata when reported; push/native events may omit it.
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default)]
    pub device_name: Option<String>,
    /// Normalized device volume, when reported and supported.
    #[serde(default)]
    pub volume: Option<f64>,
    pub buffering: bool,
    pub shuffle: bool,
    /// `None` when the playback backend does not report Smart Shuffle state.
    #[serde(default)]
    pub smart_shuffle: Option<bool>,
    pub repeat: RepeatMode,
    pub capabilities: Capabilities,
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Queue {
    #[serde(default)]
    pub context_uri: Option<String>,
    pub current: Option<MediaItem>,
    pub queued: Vec<MediaItem>,
    pub next_up: Vec<MediaItem>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum RepeatMode {
    Off,
    Context,
    Track,
}
impl RepeatMode {
    pub fn as_u8(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::Context => 1,
            Self::Track => 2,
        }
    }
}

/// Streaming quality. The Web Player stops at `VeryHigh`; `Lossless` and `HiRes`
/// only apply where the playback engine (native Soloist) supports them.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum AudioQuality {
    #[default]
    Auto,
    Low,
    Normal,
    High,
    VeryHigh,
    /// Lossless FLAC, 16-bit/44.1 kHz.
    Lossless,
    /// Lossless FLAC, 24-bit/44.1 kHz.
    HiRes,
}
impl AudioQuality {
    /// The Connect/`AudioQuality` proto names.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
            Self::VeryHigh => "very_high",
            Self::Lossless => "hifi",
            Self::HiRes => "hifi24",
        }
    }

    /// The `AUDIO_QUALITY` playback-settings enum value.
    pub fn as_level(self) -> i32 {
        match self {
            Self::Auto => 0,
            Self::Low => 1,
            Self::Normal => 2,
            Self::High => 3,
            Self::VeryHigh => 4,
            Self::Lossless => 5,
            Self::HiRes => 6,
        }
    }

    /// Parses the playback-settings enum name (`VERY_HIGH`, `HIFI`, `HIFI_24`).
    pub fn from_level_name(name: &str) -> Option<Self> {
        Some(match name {
            "DEFAULT" => Self::Auto,
            "LOW" => Self::Low,
            "NORMAL" => Self::Normal,
            "HIGH" => Self::High,
            "VERY_HIGH" => Self::VeryHigh,
            "HIFI" => Self::Lossless,
            "HIFI_24" => Self::HiRes,
            _ => return None,
        })
    }
}

/// Spotify's shuffle modes, including personalized recommendation insertion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShuffleMode {
    Off,
    On,
    Smart,
}

/// Identifies one queue row for removal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueItem {
    pub uri: String,
    pub uid: String,
}

#[cfg(test)]
mod tests {
    use super::AudioQuality;

    #[test]
    fn quality_levels_match_the_settings_enum() {
        for (name, level, quality) in [
            ("DEFAULT", 0, AudioQuality::Auto),
            ("LOW", 1, AudioQuality::Low),
            ("NORMAL", 2, AudioQuality::Normal),
            ("HIGH", 3, AudioQuality::High),
            ("VERY_HIGH", 4, AudioQuality::VeryHigh),
            ("HIFI", 5, AudioQuality::Lossless),
            ("HIFI_24", 6, AudioQuality::HiRes),
        ] {
            assert_eq!(quality.as_level(), level);
            assert_eq!(AudioQuality::from_level_name(name), Some(quality));
        }
        assert_eq!(AudioQuality::from_level_name("PETREL"), None);
    }
}

/// Normalized creators and access flags shared by catalog, queue and playback cards.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Creator {
    pub uri: Option<String>,
    pub name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaMetadata {
    pub artists: Vec<Creator>,
    pub authors: Vec<Creator>,
    pub narrators: Vec<Creator>,
    pub audiobook: bool,
    pub advertisement: bool,
    pub playable: Option<bool>,
    pub explicit: bool,
    pub queued_by: Option<String>,
}
fn creators(raw: &Value, paths: &[&str]) -> Vec<Creator> {
    let mut result = Vec::new();
    for person in paths
        .iter()
        .filter_map(|path| raw.pointer(path)?.as_array())
        .flatten()
    {
        let person = person
            .get("entity")
            .or_else(|| person.get("data"))
            .unwrap_or(person);
        let name = person
            .get("name")
            .or_else(|| person.pointer("/profile/name"))
            .or_else(|| person.pointer("/decorations/identity/name"))
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty());
        if let Some(name) = name {
            let creator = Creator {
                uri: person["uri"].as_str().map(str::to_owned),
                name: name.into(),
            };
            if !result.contains(&creator) {
                result.push(creator);
            }
        }
    }
    result
}
impl MediaItem {
    pub fn metadata(&self) -> MediaMetadata {
        let truthy = |value: &Value| value == true || value == "true";
        MediaMetadata {
            artists: creators(
                &self.raw,
                &[
                    "/firstArtist/items",
                    "/otherArtists/items",
                    "/artists/items",
                    "/artists",
                    "/decorations/creators",
                ],
            ),
            authors: creators(
                &self.raw,
                &[
                    "/authorsV2/items",
                    "/authors/items",
                    "/authors",
                    "/authorsV2",
                ],
            ),
            narrators: creators(
                &self.raw,
                &[
                    "/narratorsV2/items",
                    "/narrators/items",
                    "/narrators",
                    "/narratorsV2",
                ],
            ),
            audiobook: self.uri.starts_with("spotify:audiobook:")
                || self.raw["__typename"] == "Audiobook"
                || truthy(&self.raw["isAudiobook"]),
            advertisement: self.kind == MediaKind::Ad
                || self.uri.starts_with("spotify:ad:")
                || self.raw["provider"] == "ad"
                || truthy(&self.raw["metadata"]["is_advertisement"])
                || self.raw["metadata"]["ad_id"].is_string(),
            playable: self
                .raw
                .pointer("/playability/playable")
                .and_then(Value::as_bool)
                .or_else(|| self.raw.get("isPlayable").and_then(Value::as_bool)),
            explicit: truthy(&self.raw["explicit"])
                || self.raw["contentRating"]["label"] == "EXPLICIT",
            queued_by: self
                .raw
                .pointer("/metadata/queued_by")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned),
        }
    }
    pub fn require_playable(&self) -> Result<(), crate::Error> {
        if self.metadata().playable == Some(false) || self.raw["__typename"] == "RestrictedContent"
        {
            return Err(crate::Error::FeatureUnavailable {
                feature: "title_playback".into(),
                reason: crate::FeatureRestriction::TitleLocked,
            });
        }
        Ok(())
    }
}
impl PlayerSnapshot {
    /// Playback speed, not elapsed-time interpolation. Missing speed stays unknown.
    pub fn playback_speed(&self) -> Option<f64> {
        ["/speed", "/position/speed", "/playback_speed"]
            .iter()
            .find_map(|path| self.raw.pointer(path)?.as_f64())
            .filter(|speed| speed.is_finite() && (0.5..=3.5).contains(speed))
    }
}

#[cfg(test)]
mod metadata_tests {
    use super::*;
    #[test]
    fn metadata_normalizes_jam_artists_ads_and_spoken_types() {
        let mut item = MediaItem {
            uri: "spotify:show:book".into(),
            name: "Book".into(),
            kind: MediaKind::Show,
            images: vec![],
            duration_ms: None,
            uid: None,
            raw: serde_json::json!({"__typename":"Audiobook","authorsV2":{"items":[{"name":"Author"}]},"decorations":{"creators":[{"entity":{"uri":"spotify:artist:a","profile":{"name":"Artist"}}}]},"metadata":{"is_advertisement":"true","queued_by":"alice"},"playability":{"playable":false}}),
        };
        let metadata = item.metadata();
        assert!(metadata.audiobook && metadata.advertisement);
        assert_eq!(metadata.artists[0].name, "Artist");
        assert_eq!(metadata.authors[0].name, "Author");
        assert_eq!(metadata.queued_by.as_deref(), Some("alice"));
        assert!(item.require_playable().is_err());
        item.raw = Value::Null;
        assert_eq!(item.metadata().playable, None);
        assert!(item.require_playable().is_ok());
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum QualitySource {
    WebStream,
    DevicePreference,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QualityReport {
    pub quality: Option<AudioQuality>,
    pub source: QualitySource,
    /// Device/account capability, not proof of a delivered lossless stream.
    pub lossless_available: bool,
}
