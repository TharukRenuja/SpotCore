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
