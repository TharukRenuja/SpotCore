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

`devices().transfer(device_id, play)` transfers the session and optionally starts playback. `pull_here()` requires a local browser playback device. Cookie-only HTTP has no local receiver to pull to.

`play_on(device_id, uri)` starts content on that receiver, which may become active. `execute_on(device_id, command)` sends supported Connect controls directly without pulling playback into SpotCore. These use HTTP even when a Web session is attached.

Explicit targeting rejects missing/restricted devices. It does not borrow the active device's state for an inactive receiver. State-dependent queue replacement and Smart Shuffle are unavailable for inactive devices. Nested `Command::On` and quality changes are unsupported through this path. Queue commands use the shared HTTP queue service, retain receiver/Jam scope checks and support multiple additions. Handle partial completion when a batch fails.

## Remote quality

```rust
use spotcore::{AudioQuality, SpotCore};

async fn quality(spotify: &SpotCore) -> Result<(), spotcore::Error> {
    let devices = spotify.devices().list().await?;
    if let Some(device) = devices.iter().find(|device| device.active) {
        let available = spotify.devices().audio_quality_options(device).await?;
        if available.contains(&AudioQuality::VeryHigh) {
            let preference = spotify.devices()
                .set_audio_quality(device, AudioQuality::VeryHigh).await?;
            println!("{preference:?}");
        }
    }
    Ok(())
}
```

Quality choices require the receiver’s explicit remote-control flag and known maximum, plus the current account tier. Missing lossless eligibility excludes Lossless/HiRes even when the device decoder supports them. Region alone does not grant eligibility. A write refreshes the target receiver before validating the requested level; unavailable levels are rejected rather than silently replaced. Quality settings do not measure delivered audio. A later `spotify.set_quality()` targets the SDK-selected backend, not necessarily the device most recently passed to `play_on()`.

In cookie-backed Services mode, ordinary controls act on an existing active receiver. No receiver means no local playback. Handle `Disconnected`, `Unsupported` or the returned backend error rather than assuming a transfer succeeded.

## Receiver equalizer preferences

`devices().equalizer(&device)` reads an explicit enabled flag and six gain values in low-shelf, low-peak, low-mid, high-mid, high-peak and high-shelf order. `set_equalizer_enabled(&device, enabled)` changes only the enabled flag, preserving all gains, and checks readback. Both target the supplied receiver without transferring or starting playback. A write refreshes receiver discovery and rejects missing or restricted targets. Missing or malformed EQ fields return `Unsupported` rather than invented defaults.

These methods confirm Spotify’s stored receiver preference, not audible processing. Gain values retain the receiver’s representation; no dB conversion or gain-write API is provided until its scale is verified. A successful toggle with flat bands does not establish a change in the audio signal.

## Devices method reference

Receiver arguments are omitted. `Value` means `serde_json::Value`. The return column includes the complete `Result` type.

| Method and inputs | Returns | Behavior |
| --- | --- | --- |
| `list()` | `Result<Vec<Device>, Error>` | Discover Connect devices for the account. |
| `equalizer(device: &Device)` | `Result<EqualizerSettings, Error>` | Read explicit EQ enable and six-band gain preferences. |
| `set_equalizer_enabled(device: &Device, enabled: bool)` | `Result<EqualizerSettings, Error>` | Toggle EQ only, preserve gains and confirm enabled-state readback. |
| `audio_quality_options(device: &Device)` | `Result<Vec<AudioQuality>, Error>` | Intersect reported remote-control support, maximum quality, account tier and explicit lossless eligibility. Unknown support returns `Unsupported`. |
| `audio_quality(device: &Device)` | `Result<AudioQuality, Error>` | Read the reported device-quality setting. |
| `set_audio_quality(device: &Device, quality: AudioQuality)` | `Result<AudioQuality, Error>` | Write the setting then read it back. Not stream measurement. |
| `transfer(device_id: &str, play: bool)` | `Result<Value, Error>` | Transfer playback to a receiver, optionally starting it. |
| `pull_here()` | `Result<Value, Error>` | Transfer to the local browser receiver. No local HTTP-only receiver exists. |
