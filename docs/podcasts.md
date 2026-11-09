# Podcasts and audiobooks

Use `podcasts()` for show, episode, audiobook and chapter metadata. Spoken-audio playback uses the same player as music; see [playback](playback.md) for speed and relative seeking.

```rust
use spotcore::{Paging, SpotCore};

async fn episodes(spotify: &SpotCore, show_uri: &str) -> Result<(), spotcore::Error> {
    let show = spotify.podcasts().show_info(show_uri).await?;
    let episodes = spotify.podcasts().episodes(show_uri, Paging::default()).await?;
    println!("{}: {} episodes on this page", show.name, episodes.items.len());
    Ok(())
}
```

A show URI is not an episode URI. Pass show URIs to show/episode-list methods and episode URIs to episode/saved-episode methods. Audiobooks share parts of Spotify’s show/episode representation; do not infer audiobook entitlement from a successful metadata response.

`set_saved(episode_uri, true)` saves an episode; `false` removes it. `saved_episodes()` returns a raw collection response. Audiobook save/progress parity is not provided. The SDK does not download audio for offline use.

Regional availability matters: a query may be restricted or empty in one market and succeed in another. Missing resume position, duration or description stays optional. Follow `next_offset` for episode/chapter pages, including pages with unrecognized rows.

## Podcasts method reference

Receiver arguments are omitted. `Value` means `serde_json::Value`; the return column includes the complete `Result` type.

| Method and inputs | Returns | Behavior |
| --- | --- | --- |
| `show(value: &str)` | `Result<MediaItem, Error>` | Read normalized media metadata. |
| `show_details(value: &str)` | `Result<Value, Error>` | Read raw show metadata. |
| `episode(value: &str)` | `Result<MediaItem, Error>` | Read normalized media metadata. |
| `episodes(value: &str, paging: Paging)` | `Result<Page<MediaItem>, Error>` | Page episodes for a show. |
| `audiobook(value: &str)` | `Result<Value, Error>` | Read audiobook/show metadata as JSON. |
| `chapters(value: &str, paging: Paging)` | `Result<Page<MediaItem>, Error>` | Page audiobook chapters, with the response’s episode-list fallback. |
| `saved_episodes()` | `Result<Value, Error>` | Read saved-episode collection data. |
| `set_saved(value: &str, saved: bool)` | `Result<Value, Error>` | Add/remove an episode from the saved-episode collection. |
| `show_info(value: &str)` | `Result<ShowInfo, Error>` | Read typed detailed metadata. |
| `episode_info(value: &str)` | `Result<EpisodeInfo, Error>` | Read typed detailed metadata. |
