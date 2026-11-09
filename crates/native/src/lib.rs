//! Native playback through Spotify's Soloist daemon, controlled over a local
//! WebSocket.

pub mod backend;
pub mod daemon;
pub mod manager;
pub mod protocol;

pub use backend::{NativeBackend, SoloistBackend};
pub use daemon::{DEFAULT_DEVICE_NAME, SoloistConfig, SoloistProcess};
pub use manager::SoloistManager;

pub type NativeConfig = SoloistConfig;
pub type NativeProcess = SoloistProcess;
pub type NativeManager = SoloistManager;
