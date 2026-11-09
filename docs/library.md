# Library

Use `library()` for saves and artist/show follows. Playlist following belongs to `playlists()`.

```rust
use spotcore::{Paging, SpotCore};

async fn saved_music(spotify: &SpotCore, track_uri: String, artist_uri: &str) -> Result<(), spotcore::Error> {
    let library = spotify.library();
    library.add(std::slice::from_ref(&track_uri)).await?;
    library.follow_artist(artist_uri).await?;
    let saved = library.contains(&[track_uri]).await?;
    let liked = library.liked_tracks(Paging::default()).await?;
    println!("Saved: {saved:?}; {} liked tracks on this page", liked.items.len());
    Ok(())
}
```

Reverse these actions with `remove`, `unfollow_artist` and `unfollow_show`. `contains()` preserves input order. It is distinct from downloading media for offline use.

## Browse your library

```rust
use spotcore::{Paging, SpotCore};

async fn library_rows(spotify: &SpotCore) -> Result<(), spotcore::Error> {
    let rows = spotify.library()
        .items_detailed(None, "Recents", None, "Miles", Paging::default()).await?;
    for row in rows.items {
        println!("{}: pinned={:?}", row.item.name, row.pinned);
    }
    Ok(())
}
```

Filter/order strings use Spotify's names. `None` requests all item types; the optional folder selects a subtree. `items_detailed()` keeps pinned state, dates, owner URI, counts and the original row alongside normalized media. `items()` retains its simpler result. Follow `next_offset` for pagination. Folder revisions remain in raw data; detailed normalized folder URIs are stable.

## Pins and folders

```rust
use spotcore::SpotCore;

async fn organize(spotify: &SpotCore, playlist_uri: &str, folder_uri: &str) -> Result<(), spotcore::Error> {
    spotify.library().pin(playlist_uri, true).await?;
    spotify.library().move_to_folder(playlist_uri, Some(folder_uri)).await?;
    Ok(())
}
```

`folders()` reads the rootlist hierarchy. `create_folder`, `rename_folder`, `move_to_folder` and `delete_folder` expose folder changes. Pass `None` to `move_to_folder` to move an item to the root. Use actual returned folder URIs; do not construct revision markers. Folder mutations can affect nested contents: inspect the hierarchy before deleting or moving a folder.

See [playlists](playlists.md) for hidden-song preferences and playlist edits.

## Library method reference

Receiver arguments are omitted. `Value` means `serde_json::Value`; the return column includes the complete `Result` type.

| Method and inputs | Returns | Behavior |
| --- | --- | --- |
| `follow_artist(value: &str)` | `Result<Value, Error>` | Follow an artist URI; prefer the Library domain. |
| `unfollow_artist(value: &str)` | `Result<Value, Error>` | Unfollow an artist URI; prefer the Library domain. |
| `follow_show(value: &str)` | `Result<Value, Error>` | Follow a show URI; prefer the Library domain. |
| `unfollow_show(value: &str)` | `Result<Value, Error>` | Unfollow a show URI; prefer the Library domain. |
| `contains(values: &[String])` | `Result<Vec<bool>, Error>` | Return membership flags in the same order as input URIs. |
| `add(values: &[String])` | `Result<Value, Error>` | Save supplied media URIs; playlist follows use Playlists. |
| `remove(values: &[String])` | `Result<Value, Error>` | Remove supplied media URIs from the library. |
| `items(filter: Option<&str>, order: &str, folder: Option<&str>, paging: Paging)` | `Result<Page<MediaItem>, Error>` | Page normalized library media; filter/order use Spotify names. |
| `items_detailed(filter: Option<&str>, order: &str, folder: Option<&str>, text_filter: &str, paging: Paging)` | `Result<Page<LibraryItem>, Error>` | Page library rows with metadata and text filtering. |
| `liked_tracks(paging: Paging)` | `Result<Page<MediaItem>, Error>` | Page Liked Songs. |
| `pin(value: &str, pinned: bool)` | `Result<Value, Error>` | Set or clear a library pin. |
| `folders()` | `Result<Vec<LibraryEntry>, Error>` | Read rootlist entries with nested folder children. |
| `create_folder(name: &str)` | `Result<Value, Error>` | Create a rootlist folder. |
| `rename_folder(value: &str, name: &str)` | `Result<Value, Error>` | Rename a folder, resolving its current revision. |
| `delete_folder(value: &str)` | `Result<Value, Error>` | Remove a folder through rootlist services; inspect nested contents first. |
| `move_to_folder(value: &str, folder: Option<&str>)` | `Result<Value, Error>` | Move a library entry into a folder, or to root with None. |
| `hidden_songs(cursor: Option<&str>, limit: u32)` | `Result<spotcore_core::CursorPage<HiddenSong>, Error>` | Read account-wide hidden-song preferences with an opaque cursor. |

## Input and write behavior

Save/remove accepts supported media URIs (tracks, albums, artists, shows, episodes and authors). Follow playlists through `playlists().follow`, not `library().add`. Artist/show wrappers validate the expected URI kind. Membership requests are split into collection and playlist checks while preserving original input order. Library save/remove and membership operations may be batched; do not assume all-or-nothing updates if a later request fails.

Folder methods accept returned Spotify folder URIs, including stable normalized ones that the implementation resolves to the current rootlist revision. Do not use display names as identifiers. Optional folder `None` means the root. The `order` argument must be nonempty; filter/order values are Spotify protocol strings, not a closed SDK enum. Unknown values can be rejected by Spotify.
