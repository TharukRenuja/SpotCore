use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandPayload {
    pub r#type: String,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

impl CommandPayload {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            r#type: "command".to_string(),
            command: command.into(),
            uri: None,
            position_ms: None,
            volume: None,
            enabled: None,
            limit: None,
        }
    }

    pub fn activate() -> Self {
        Self::new("activate")
    }

    pub fn deactivate() -> Self {
        Self::new("deactivate")
    }

    pub fn get_state() -> Self {
        Self::new("get_state")
    }

    pub fn play_uri(uri: impl Into<String>) -> Self {
        let mut cmd = Self::new("play");
        cmd.uri = Some(uri.into());
        cmd
    }

    pub fn resume() -> Self {
        Self::new("play")
    }

    pub fn pause() -> Self {
        Self::new("pause")
    }

    pub fn skip_next() -> Self {
        Self::new("skip_next")
    }

    pub fn skip_prev() -> Self {
        Self::new("skip_prev")
    }

    pub fn seek(position_ms: u64) -> Self {
        let mut cmd = Self::new("seek");
        cmd.position_ms = Some(position_ms);
        cmd
    }

    pub fn set_volume(vol_0_to_100: u32) -> Self {
        let mut cmd = Self::new("set_volume");
        cmd.volume = Some(vol_0_to_100.min(100));
        cmd
    }

    pub fn set_shuffle(enabled: bool) -> Self {
        let mut cmd = Self::new("set_shuffle");
        cmd.enabled = Some(enabled);
        cmd
    }

    pub fn set_repeat_track(enabled: bool) -> Self {
        let mut cmd = Self::new("set_repeat_track");
        cmd.enabled = Some(enabled);
        cmd
    }

    pub fn set_repeat_context(enabled: bool) -> Self {
        let mut cmd = Self::new("set_repeat_context");
        cmd.enabled = Some(enabled);
        cmd
    }

    pub fn add_to_queue(uri: impl Into<String>) -> Self {
        let mut cmd = Self::new("add_to_queue");
        cmd.uri = Some(uri.into());
        cmd
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionInfo {
    #[serde(default)]
    pub position_ms: u64,
    #[serde(default)]
    pub timestamp_ms: u64,
    #[serde(default)]
    pub speed: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackOptions {
    #[serde(default)]
    pub shuffle: bool,
    #[serde(default)]
    pub repeat: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackStateEvent {
    #[serde(default)]
    pub status: String, // "playing", "paused", "buffering", "idle"
    #[serde(default)]
    pub is_active: Option<bool>,
    #[serde(default)]
    pub position: Option<PositionInfo>,
    #[serde(default)]
    pub item: Option<serde_json::Value>,
    #[serde(default)]
    pub context: Option<serde_json::Value>,
    #[serde(default)]
    pub volume: Option<u32>,
    #[serde(default)]
    pub options: Option<PlaybackOptions>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceChangedEvent {
    #[serde(default)]
    pub is_active: Option<bool>,
    #[serde(default)]
    pub device_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaybackChangedEvent {
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrackChangedEvent {
    #[serde(default)]
    pub item: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PositionSyncEvent {
    #[serde(default)]
    pub position: Option<PositionInfo>,
}

// Inbound server events only; commands use CommandPayload.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum SoloistServerEvent {
    #[serde(rename = "playback_state")]
    PlaybackState(PlaybackStateEvent),
    #[serde(rename = "playback_changed")]
    PlaybackChanged(PlaybackChangedEvent),
    #[serde(rename = "track_changed")]
    TrackChanged(TrackChangedEvent),
    #[serde(rename = "position_sync")]
    PositionSync(PositionSyncEvent),
    #[serde(rename = "auth_state")]
    AuthState(serde_json::Value),
    #[serde(rename = "queue")]
    Queue(serde_json::Value),
    #[serde(rename = "device_changed")]
    DeviceChanged(DeviceChangedEvent),
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_serialization() {
        let play_cmd = CommandPayload::play_uri("spotify:track:4cOdK2wGLETKBW3PvgPWqT");
        let json = serde_json::to_string(&play_cmd).unwrap();
        assert!(json.contains("\"type\":\"command\""));
        assert!(json.contains("\"command\":\"play\""));
        assert!(json.contains("\"uri\":\"spotify:track:4cOdK2wGLETKBW3PvgPWqT\""));

        let seek_cmd = CommandPayload::seek(45000);
        let seek_json = serde_json::to_string(&seek_cmd).unwrap();
        assert!(seek_json.contains("\"command\":\"seek\""));
        assert!(seek_json.contains("\"position_ms\":45000"));

        let vol_cmd = CommandPayload::set_volume(85);
        let vol_json = serde_json::to_string(&vol_cmd).unwrap();
        assert!(vol_json.contains("\"command\":\"set_volume\""));
        assert!(vol_json.contains("\"volume\":85"));

        let act_cmd = CommandPayload::activate();
        let act_json = serde_json::to_string(&act_cmd).unwrap();
        assert!(act_json.contains("\"command\":\"activate\""));
    }

    #[test]
    fn test_event_deserialization() {
        let raw = r#"{
            "type": "playback_state",
            "status": "playing",
            "is_active": true,
            "position": {
                "position_ms": 12345,
                "timestamp_ms": 1740000000000,
                "speed": 1.0
            },
            "item": {
                "uri": "spotify:track:abc",
                "entity_type": "track",
                "decorations": {
                    "identity": { "name": "Never Gonna Give You Up" }
                }
            },
            "volume": 90,
            "options": {
                "shuffle": false,
                "repeat": "off"
            }
        }"#;

        let event: SoloistServerEvent = serde_json::from_str(raw).unwrap();
        match event {
            SoloistServerEvent::PlaybackState(state) => {
                assert_eq!(state.status, "playing");
                assert_eq!(state.is_active, Some(true));
                assert_eq!(state.position.unwrap().position_ms, 12345);
            }
            _ => panic!("Expected PlaybackState event"),
        }

        let raw_changed = r#"{"type": "playback_changed", "status": "playing"}"#;
        let event: SoloistServerEvent = serde_json::from_str(raw_changed).unwrap();
        match event {
            SoloistServerEvent::PlaybackChanged(p) => assert_eq!(p.status, "playing"),
            _ => panic!("Expected PlaybackChanged event"),
        }

        let raw_track = r#"{"type": "track_changed", "item": {"uri": "spotify:track:xyz"}}"#;
        let event: SoloistServerEvent = serde_json::from_str(raw_track).unwrap();
        match event {
            SoloistServerEvent::TrackChanged(t) => {
                assert_eq!(t.item.unwrap().get("uri").unwrap(), "spotify:track:xyz");
            }
            _ => panic!("Expected TrackChanged event"),
        }
    }
}
