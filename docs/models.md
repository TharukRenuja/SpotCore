# Models, identifiers and pagination

Some methods return typed models; others return raw `serde_json::Value`. Raw responses keep Spotify’s original shape, so check their fields rather than treating them as a typed model. A successful HTTP response can still contain no matching entity.

## Common models

| Type | Fields / use |
| --- | --- |
| `MediaItem` | `uri`, `name`, `kind`, `images`, optional `duration_ms` and `uid`, plus `raw` |
| `MediaKind` | Track, Episode, Album, Artist, Playlist, Show, User, Folder, Ad, Unknown; not a guarantee an item can play |
| `Image` | URL and optional width/height; apps choose fetching/caching policy |
| `Page<T>` | `items`, starting `offset`, optional `total`, optional `next_offset` |
| `CursorPage<T>` | `items` and opaque optional `next_cursor` |
| `Paging` | `offset` and `limit`; default offset 0, limit 30; valid limits 1–100 with overflow checks |
| `Queue` | Optional `current`, manual `queued` rows and context `next_up` rows |
| `QueueItem` | Exact `uri` + `uid` pair for queue mutation |
| `AppliedQuality` | `requested`, `effective`, `lossless_available`; describes preference/readback, not audio measurement |
| `DrmSupport` | `widevine`, `aac`, optional `error`; `require_playback()` rejects insufficient capability |
| `AudioInfo` | Web-reported `bitrate` (0 means unknown), `mime` and `level` |

`MediaKind` does not have a separate audiobook variant. Spoken-audio entities use Spotify’s show/episode representation where available. Non-exhaustive enums need a fallback match arm. Ads are represented as `MediaKind::Ad`; they are not silently removed.

## Detailed domain models

| Family | Types and semantics |
| --- | --- |
| Catalog | `TrackInfo`, `AlbumInfo`, `ArtistInfo`; artist references, album references, artwork, numbers and optional fields are retained alongside `raw` |
| Spoken audio | `ShowInfo`, `EpisodeInfo`; descriptions, publisher/show references, optional duration/release date/resume position |
| Playlist | `PlaylistInfo`, `PlaylistAdder`, `OwnerRef`; optional ownership/access/count fields and row adder identity |
| References | `ArtistRef` has string id/URI/name; `MediaRef` has URI/name; `OwnerRef` fields are optional |
| Library | `LibraryEntry` is a nested rootlist row; `LibraryItem` keeps normalized media, pin/date/owner/count and original row |
| Hidden songs | `HiddenSong` contains `track_uri` and playlist `context_uri`; deletion markers are omitted |
| Browsing | `HomeFeed`, `HomeFilter`, `Shelf`, `BrowseCategory`, `BrowsePage`; preserve raw data and paging |
| Search | `SearchResults` has one `Page<MediaItem>` for each of the nine search kinds |
| Account | `Account` contains canonical username, URI, display name, tier, country, images and raw data |
| Connect | `web::Device` contains id/name/kind, active/local flags, optional volume, restriction/volume/quality flags and raw data |

Optional popularity, genres, public/collaborative flags and counts depend on the upstream query. `None` means not supplied/known, not false or zero. Lists may be empty. Consult generated Rust docs for every field and its exact type; the guides do not redefine these types.

## Offset pagination

```rust
use spotcore::{Paging, SpotCore};

async fn liked(spotify: &SpotCore) -> Result<(), spotcore::Error> {
    let mut paging = Paging::default();
    loop {
        let page = spotify.library().liked_tracks(paging).await?;
        for item in page.items {
            println!("{}", item.name);
        }
        match page.next_offset {
            Some(offset) => paging.offset = offset,
            None => break,
        }
    }
    Ok(())
}
```

Advance by `next_offset`, not mapped `items.len()`. Unknown/private rows can be omitted during mapping while still counting toward the upstream offset. `total` may be absent. Contents can change during pagination; this is not a transactional snapshot.

## Cursor pagination

```rust
use spotcore::SpotCore;

async fn hidden(spotify: &SpotCore) -> Result<(), spotcore::Error> {
    let mut cursor = None;
    loop {
        let page = spotify.library().hidden_songs(cursor.as_deref(), 50).await?;
        for row in page.items {
            println!("{} in {}", row.track_uri, row.context_uri);
        }
        cursor = page.next_cursor;
        if cursor.is_none() { break; }
    }
    Ok(())
}
```

Do not parse or manufacture cursors. Hidden-song cursors are bounded to 8192 bytes; a non-advancing continuation is an error.

## Identifiers and units

Use Spotify URIs (`spotify:track:…`, `spotify:artist:…`) for domain entities, Connect device IDs for device targeting, canonical usernames for account selection, and returned UIDs for playlist/queue rows. These are different identifier types even when represented as strings. A display name is not an account ID.

Playback APIs accept `Duration`; snapshot/media fields ending in `_ms` are milliseconds. Browser media diagnostics can use seconds: do not mix those units. Volume controls accept finite normalized values 0.0–1.0. Do not replace missing UIDs with empty strings or select a different row by URI alone.

`raw` can contain user-specific data. Do not dump entire models or service responses to public logs. Handle URLs and HTML-like descriptions as untrusted content in your UI.
