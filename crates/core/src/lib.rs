//! Shared playback contract. No browser, audio, authentication, or UI dependencies.
mod models;
pub mod paths;
pub use models::*;
pub use paths::Paths;
use serde::{Deserialize, Serialize};
use std::{fmt, future::Future, pin::Pin, time::Duration};

// Browser adapters may hold thread-local handles, so futures need not be Send.
pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Command {
    /// Load one track or episode URI and start playback.
    Load(String),
    Stop,
    Resume,
    Pause,
    Seek(Duration),
    Next,
    Previous,
    SetVolume(f64),
    SetShuffle(bool),
    SetShuffleMode(ShuffleMode),
    SetRepeat(RepeatMode),
    AddToQueue(Vec<String>),
    /// Inserts URIs at the head of the manual queue, or before a queued row.
    InsertIntoQueue {
        uris: Vec<String>,
        before: Option<QueueItem>,
    },
    /// Moves queued rows before or after another queued row.
    ReorderQueue {
        items: Vec<QueueItem>,
        before: Option<QueueItem>,
        after: Option<QueueItem>,
    },
    /// Jumps to a queued or upcoming row.
    SkipToQueue(QueueItem),
    /// Runs a command on one Connect device without moving playback here.
    On {
        device: String,
        command: Box<Command>,
    },
    /// Clears everything queued after the current item.
    ClearQueue,
    RemoveFromQueue(Vec<QueueItem>),
    /// Plays a context at an optional track, index, or position.
    PlayContext {
        uri: String,
        track: Option<String>,
        index: Option<u32>,
        position: Option<Duration>,
    },
    /// Sets podcast playback speed, 0.5 to 3.5.
    SetSpeed(f64),
    SetQuality(AudioQuality),
}

/// Effective permissions reported by a connected backend, including account restrictions.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Capabilities {
    pub load: bool,
    pub stop: bool,
    pub resume: bool,
    pub pause: bool,
    pub seek: bool,
    pub next: bool,
    pub previous: bool,
    pub volume: bool,
    pub shuffle: bool,
    #[serde(default)]
    pub smart_shuffle: bool,
    pub repeat: bool,
    pub queue: bool,
    /// Supports clearing the queue and removing individual rows.
    #[serde(default)]
    pub queue_edit: bool,
    pub play_context: bool,
    pub speed: bool,
    pub quality: bool,
}

impl Capabilities {
    pub fn supports(self, command: &Command) -> bool {
        match command {
            Command::Load(_) => self.load,
            Command::Stop => self.stop,
            Command::Resume => self.resume,
            Command::Pause => self.pause,
            Command::Seek(_) => self.seek,
            Command::Next => self.next,
            Command::Previous => self.previous,
            Command::SetVolume(_) => self.volume,
            Command::SetShuffle(_) => self.shuffle,
            Command::SetShuffleMode(ShuffleMode::Smart) => self.smart_shuffle,
            Command::SetShuffleMode(_) => self.shuffle,
            Command::SetRepeat(_) => self.repeat,
            Command::AddToQueue(_) => self.queue,
            Command::InsertIntoQueue { .. }
            | Command::ReorderQueue { .. }
            | Command::SkipToQueue(_)
            | Command::ClearQueue
            | Command::RemoveFromQueue(_) => self.queue_edit,
            Command::PlayContext { .. } => self.play_context,
            Command::SetSpeed(_) => self.speed,
            Command::SetQuality(_) => self.quality,
            Command::On { command, .. } => self.supports(command),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlaybackState {
    Idle,
    Playing,
    Paused,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum PlaybackEvent {
    StateChanged(PlaybackState),
    EndOfTrack,
    TrackUnavailable(String),
    CapabilitiesChanged(Capabilities),
    Snapshot(Box<PlayerSnapshot>),
    QueueChanged(Queue),
    SessionChanged,
}

/// One engine candidate rejected during automatic startup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EngineFailure {
    pub engine: String,
    pub path: Option<std::path::PathBuf>,
    pub cause: Box<Error>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub enum Error {
    NoCompatibleEngine {
        attempts: Vec<EngineFailure>,
        requirements: Vec<String>,
    },
    Unsupported,
    InvalidInput(&'static str),
    DrmUnavailable(String),
    AccountRestricted(&'static str),
    NotAuthenticated,
    Disconnected,
    Timeout,
    EventsLagged(u64),
    Http {
        status: u16,
        retry_after: Option<u64>,
    },
    Backend(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCompatibleEngine { attempts, .. } => write!(
                f,
                "no compatible browser engine ({} candidates rejected); inspect attempts and requirements",
                attempts.len()
            ),
            Self::InvalidInput(message) => f.write_str(message),
            Self::AccountRestricted(message) => f.write_str(message),
            Self::DrmUnavailable(message) => {
                write!(f, "Spotify Web playback DRM is unavailable: {message}")
            }
            Self::Unsupported => f.write_str("command is not supported by this session"),
            Self::NotAuthenticated => f.write_str("authentication is required"),
            Self::Timeout => {
                f.write_str("operation timed out; a submitted write may have completed")
            }
            Self::EventsLagged(count) => write!(f, "missed {count} events; refresh playback state"),
            Self::Http {
                status,
                retry_after,
            } => write!(f, "Spotify HTTP {status} (retry after: {retry_after:?})"),
            Self::Disconnected => f.write_str("playback backend is disconnected"),
            Self::Backend(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for Error {}

/// Implementations must also enforce live permissions, which can change after a capability check.
/// Successful execution means a command was submitted; observe events for playback outcomes.
pub trait Backend {
    fn capabilities(&self) -> Capabilities;
    fn execute(&mut self, command: Command) -> BackendFuture<'_, Result<(), Error>>;

    /// Wait for the next playback event. Backends without event delivery return Unsupported.
    fn next_event(&mut self) -> BackendFuture<'_, Result<PlaybackEvent, Error>> {
        Box::pin(async { Err(Error::Unsupported) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_commands_inherit_the_inner_capability() {
        let capabilities = Capabilities {
            seek: true,
            ..Default::default()
        };
        assert!(capabilities.supports(&Command::On {
            device: "device".into(),
            command: Box::new(Command::Seek(Duration::ZERO)),
        }));
        assert!(!capabilities.supports(&Command::On {
            device: "device".into(),
            command: Box::new(Command::Next),
        }));
    }
}
