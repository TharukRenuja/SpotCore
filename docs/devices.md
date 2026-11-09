# Devices and Connect

Devices must belong to the authenticated account and be discoverable by Spotify. A Connect receiver can be the official app, a supported speaker, the local Web Player or an opted-in native instance.

```rust
use spotcore::{Command, SpotCore};

async fn remote(spotify: &mut SpotCore) -> Result<(), spotcore::Error> {
    let devices = spotify.devices().list().await?;
    if let Some(device) = devices.iter().find(|device| device.active) {
        println!("{}: volume={:?}", device.name, device.volume);
        spotify.execute_on(&device.id, Command::Pause).await?;
    }
    Ok(())
}
```

Device models expose identity, active/local flags, volume, restriction and quality flags. Missing fields are not proof that a receiver accepts a command.

## Transfer or target

`devices().transfer(device_id, play)` transfers the session and optionally starts playback. `pull_here()` requires a local browser playback device; cookie-only HTTP has no local receiver to pull to.

`play_on(device_id, uri)` starts content on that receiver, which may become active. `execute_on(device_id, command)` sends supported Connect controls directly without pulling playback into SpotCore. These use HTTP even when a Web session is attached.

Explicit targeting rejects missing/restricted devices. It does not borrow the active device's state for an inactive receiver. State-dependent queue replacement and Smart Shuffle are unavailable for inactive devices. Nested `Command::On`, quality changes, multi-item queue adds and browser-only queue edits are unsupported through this path.

## Remote quality

```rust
use spotcore::{AudioQuality, SpotCore};

async fn quality(spotify: &SpotCore) -> Result<(), spotcore::Error> {
    let devices = spotify.devices().list().await?;
    if let Some(device) = devices.iter().find(|device| device.active) {
        let preference = spotify.devices()
            .set_audio_quality(device, AudioQuality::VeryHigh).await?;
        println!("{preference:?}");
    }
    Ok(())
}
```

Quality settings depend on device/account support and do not measure delivered audio. A later `spotify.set_quality()` targets the SDK-selected backend, not necessarily the device most recently passed to `play_on()`.

In cookie-backed Services mode, ordinary controls act on an existing active receiver. No receiver means no local playback; handle `Disconnected`, `Unsupported` or the returned backend error rather than assuming a transfer succeeded.

## Devices method reference

Receiver arguments are omitted. `Value` means `serde_json::Value`; the return column includes the complete `Result` type.

| Method and inputs | Returns | Behavior |
| --- | --- | --- |
| `list()` | `Result<Vec<Device>, Error>` | Discover Connect devices for the account. |
| `audio_quality(device: &Device)` | `Result<AudioQuality, Error>` | Read the reported device-quality setting. |
| `set_audio_quality(device: &Device, quality: AudioQuality)` | `Result<AudioQuality, Error>` | Write the setting then read it back; not stream measurement. |
| `transfer(device_id: &str, play: bool)` | `Result<Value, Error>` | Transfer playback to a receiver, optionally starting it. |
| `pull_here()` | `Result<Value, Error>` | Transfer to the local browser receiver; no local HTTP-only receiver exists. |
