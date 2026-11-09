# Playlists

Reads work for visible playlists; writes require the account's permission. Playlist metadata and some mutation responses remain JSON, so check the response before using returned IDs.

## Read and edit

```rust
use spotcore::{Paging, SpotCore};

async fn playlist(spotify: &SpotCore, playlist_uri: &str, track_uri: String) -> Result<(), spotcore::Error> {
    let info = spotify.playlists().playlist_info(playlist_uri).await?;
    let tracks = spotify.playlists().tracks(playlist_uri, Paging::default()).await?;
    println!("{}: {} rows", info.name, tracks.items.len());
    spotify.playlists().add_tracks(playlist_uri, &[track_uri]).await?;
    Ok(())
}
```

`mine()` returns owned/followed rootlist entries. `create(name, folder)` creates a playlist; `rename`, `set_description`, `set_published` and `set_access` edit its metadata and permissions. Publishing and collaboration are separate settings. `set_cover(uri, Some(jpeg_bytes))` uploads a JPEG up to 4 MiB; `None` removes the custom cover.

`follow` saves a playlist. `unfollow` removes it from the library; for owned playlists this is Spotify's delete operation. `delete` delegates to that operation.

## Exact row edits

```rust
use spotcore::{Paging, SpotCore};

async fn remove_first_row(spotify: &SpotCore, playlist_uri: &str) -> Result<(), spotcore::Error> {
    let page = spotify.playlists().tracks(playlist_uri, Paging::default()).await?;
    if let Some(uid) = page.items.first().and_then(|track| track.uid.as_ref()) {
        spotify.playlists().remove_tracks(playlist_uri, std::slice::from_ref(uid)).await?;
    }
    Ok(())
}
```

Use returned row UIDs to distinguish duplicates. `move_tracks(uri, uids, before_uid)` moves selected rows before a row, or to the bottom when `before_uid` is `None`. `added_by()` exposes who added playlist rows, with paging.

## Suggestions and hidden songs

```rust
use spotcore::SpotCore;

async fn preferences(spotify: &SpotCore, playlist_uri: &str, track_uri: &str) -> Result<(), spotcore::Error> {
    let suggestions = spotify.playlists().recommended(playlist_uri, 10, &[]).await?;
    spotify.playlists().set_hidden(playlist_uri, track_uri, true).await?;
    let hidden = spotify.library().hidden_songs(None, 50).await?;
    println!("{} suggestions; {} hidden rows", suggestions.len(), hidden.items.len());
    Ok(())
}
```

`hidden_songs(cursor, limit)` is an account-wide library collection, not a playlist method. Filter returned rows by `context_uri` for one playlist. Continue with its opaque `next_cursor`. `set_hidden(..., false)` removes that preference; actual receiver behavior remains Spotify's responsibility.

Collaborator methods include `members`, `collaborator_invite`, `join`, `set_member_collaborator` and `remove_member`. Their presence does not imply live coverage on every account; invitation/member-permission mutations remain less verified than ordinary playlist edits.

## Playlists method reference

Receiver arguments are omitted. `Value` means `serde_json::Value`; the return column includes the complete `Result` type.

| Method and inputs | Returns | Behavior |
| --- | --- | --- |
| `recommended(value: &str, limit: u32, skip: &[String])` | `Result<Vec<TrackInfo>, Error>` | Read normalized media metadata. |
| `playlist_info(value: &str)` | `Result<PlaylistInfo, Error>` | Read typed detailed metadata. |
| `playlist(value: &str)` | `Result<MediaItem, Error>` | Read normalized media metadata. |
| `tracks(value: &str, paging: Paging)` | `Result<Page<MediaItem>, Error>` | Page playlist rows; retain UIDs for exact edits. |
| `added_by(value: &str, paging: Paging)` | `Result<Page<PlaylistAdder>, Error>` | Page known adders; missing/private adders are omitted. |
| `mine()` | `Result<Vec<LibraryEntry>, Error>` | Read owned/followed rootlist entries. |
| `create(name: &str, folder: Option<&str>)` | `Result<Value, Error>` | Create a playlist, optionally positioned inside a folder. |
| `rename(value: &str, name: &str)` | `Result<Value, Error>` | Rename the playlist. |
| `set_description(value: &str, description: &str)` | `Result<Value, Error>` | Change the playlist description. |
| `access(value: &str)` | `Result<Value, Error>` | Read base playlist permissions. |
| `set_access(value: &str, access: PlaylistAccess)` | `Result<Value, Error>` | Change base permissions; Private also unpublishes the playlist. |
| `published(value: &str)` | `Result<Value, Error>` | Read profile-publication state. |
| `set_published(value: &str, published: bool)` | `Result<Value, Error>` | Set profile-publication state separately from collaboration. |
| `set_cover(value: &str, jpeg: Option<&[u8]>)` | `Result<Value, Error>` | Set JPEG bytes (max 4 MiB), or remove the cover with None. |
| `follow(value: &str)` | `Result<Value, Error>` | Save/follow the playlist. |
| `unfollow(value: &str)` | `Result<Value, Error>` | Remove the playlist from the library; owned-playlist removal is deletion. |
| `delete(value: &str)` | `Result<Value, Error>` | Alias for playlist unfollow/removal. |
| `follow_artist(value: &str)` | `Result<Value, Error>` | Follow an artist URI; prefer the Library domain. |
| `unfollow_artist(value: &str)` | `Result<Value, Error>` | Unfollow an artist URI; prefer the Library domain. |
| `follow_show(value: &str)` | `Result<Value, Error>` | Follow a show URI; prefer the Library domain. |
| `unfollow_show(value: &str)` | `Result<Value, Error>` | Unfollow a show URI; prefer the Library domain. |
| `add_tracks(value: &str, tracks: &[String])` | `Result<(), Error>` | Append track URIs in batches of up to 100; earlier batches can succeed before a later failure. |
| `remove_tracks(value: &str, uids: &[String])` | `Result<Value, Error>` | Remove exact playlist row UIDs. |
| `move_tracks(value: &str, uids: &[String], before: Option<&str>)` | `Result<Value, Error>` | Move row UIDs before another UID, or to bottom with None. |
| `members(value: &str)` | `Result<Value, Error>` | Read playlist members. |
| `set_member_collaborator(value: &str, username: &str, enabled: bool)` | `Result<Value, Error>` | Change a member’s collaboration permission; limited live verification. |
| `remove_member(value: &str, username: &str)` | `Result<Value, Error>` | Remove a playlist member identified by username; requires permission. |
| `join(value: &str, invitation_token: &str)` | `Result<Value, Error>` | Accept a playlist collaboration token; limited live verification. |
| `collaborator_invite(value: &str)` | `Result<String, Error>` | Return a playlist collaboration invitation token/link string. |
| `set_hidden(playlist: &str, track: &str, hidden: bool)` | `Result<(), Error>` | Write/remove a playlist-context hidden-song preference. |

## Permission and mutation details

`PlaylistAccess` variants are Private, Public and Collaborative. Private maps to blocked base access and also unpublishes; Public maps to viewer and Collaborative to contributor. Publication is a separate profile setting. Collaboration methods use the member username, while track edits use playlist row UIDs.

A returned JSON mutation value is not a normalized `PlaylistInfo`. Validate its shape rather than blindly unwrapping a URI. Multi-step edits and batched track adds are not transactions: a later error does not roll back earlier successful writes. Read the resulting playlist before retrying ambiguous operations.
