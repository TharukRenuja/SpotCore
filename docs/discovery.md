# Discovery and catalog

[Search](#search-method-reference) · [Catalog](#catalog-method-reference) · [Home](#home-method-reference)

Search, Home and catalog methods work through cookie-backed HTTP services with either playback backend. Content and metadata vary by account region; missing optional fields are not invented.

## Search and details

```rust
use spotcore::{Paging, SearchKind, SpotCore};

async fn search(spotify: &SpotCore) -> Result<(), spotcore::Error> {
    let tracks = spotify.search()
        .query(SearchKind::Tracks, "Miles Davis", Paging::default()).await?;
    if let Some(track) = tracks.items.first() {
        let info = spotify.catalog().track_info(&track.uri).await?;
        println!("{}: {:?}", info.name, info.duration_ms);
    }
    let suggestions = spotify.search().suggestion_texts("Miles", 5).await?;
    println!("{suggestions:?}");
    Ok(())
}
```

Search kinds are Tracks, Artists, Albums, Playlists, Shows, Episodes, Audiobooks, Users and Genres. `all(query, limit)` reads all nine with at most three concurrent requests and returns an error if any fails. Apps own debouncing and cancellation. `recent_items()` reads recent search entities.

`catalog()` offers typed `track_info`, `album_info`, `artist_info`, album tracks, artist top tracks and discography. Use `playlists().playlist_info`, `podcasts().show_info` and `episode_info` for those entities. Existing simpler `MediaItem` methods and raw responses remain available.

## Home and browse

```rust
use spotcore::{Paging, SpotCore};

async fn home(spotify: &SpotCore) -> Result<(), spotcore::Error> {
    let feed = spotify.home().feed_info("Asia/Colombo", None, 10).await?;
    for shelf in feed.shelves.items {
        println!("{:?}: {} items", shelf.title, shelf.items.items.len());
    }
    let categories = spotify.home().browse_categories().await?;
    let releases = spotify.home().new_releases(Paging::default()).await?;
    println!("{} categories, {} releases", categories.len(), releases.items.len());
    Ok(())
}
```

Home preserves greeting, filter chips, shelf order and raw data. Use returned facet IDs rather than inventing them. `section(uri, timezone, paging)` fetches shelf cards; `browse_page(uri, section_paging, item_paging)` pages browse shelves. `made_for_you` and `top_mixes` use Spotify section IDs that can change.

`Page::next_offset` advances by raw rows, including unrecognized cards. Follow that offset instead of calculating it from the mapped item count. Unknown fields/cards remain available in raw responses. `Paging` accepts limits 1–100.

## Recommendations and radio

```rust
use spotcore::SpotCore;

async fn recommendations(spotify: &SpotCore, track_uri: String) -> Result<(), spotcore::Error> {
    let tracks = spotify.catalog().radio_blend(&[track_uri], &[], 20).await?;
    for track in tracks {
        println!("{}", track.name);
    }
    Ok(())
}
```

`radio_blend(track_uris, artist_uris, limit)` accepts one to five supplied seeds, reads separate Spotify radios sequentially, interleaves unique tracks and excludes seed tracks. It can return fewer items. It is a client-side blend, not Spotify's joint multi-seed ranking.

`playlists().recommended(playlist_uri, limit, skip_track_uris)` provides playlist-context suggestions. `extras().radio_uri(seed_uri)` resolves an optional radio context. The separate `catalog().recommendations(track_uris, artist_uris, limit)` currently returns upstream 429 in live checks; it does not silently switch algorithms.

## Podcasts and audiobooks

`podcasts()` exposes shows, episodes, paged episodes, audiobook metadata and chapters, plus saved-episode reads/writes. Audiobook metadata can be returned as JSON. Reading a catalog item does not establish playback entitlement, and audiobook progress/save parity is not guaranteed. LK podcast restrictions and successful US reads were observed; handle regional errors rather than treating every empty search as an SDK failure.

## Home method reference

Receiver arguments are omitted. `Value` means `serde_json::Value`; the return column includes the complete `Result` type.

| Method and inputs | Returns | Behavior |
| --- | --- | --- |
| `feed_info(timezone: &str, facet: Option<&str>, items_per_shelf: u32)` | `Result<HomeFeed, Error>` | Preserve greeting, facets, shelves and their paging in typed data. |
| `browse_categories()` | `Result<Vec<BrowseCategory>, Error>` | Map category names, images and colors. |
| `browse_page(value: &str, sections: Paging, items: Paging)` | `Result<BrowsePage, Error>` | Read typed browse shelves with separate section/item paging. |
| `feed(timezone: &str, facet: Option<&str>, items_per_shelf: u32)` | `Result<Value, Error>` | Read the raw personalized Home response. |
| `categories()` | `Result<Value, Error>` | Read raw browse categories. |
| `category(value: &str)` | `Result<Value, Error>` | Read the raw page for a browse URI. |
| `whats_new(paging: Paging)` | `Result<Page<MediaItem>, Error>` | Read the full What’s New feed, including non-album rows. |
| `new_releases(paging: Paging)` | `Result<Page<MediaItem>, Error>` | Read album-only releases from followed artists. |
| `section(value: &str, timezone: &str, paging: Paging)` | `Result<Page<MediaItem>, Error>` | Read a Home shelf; merges returned sections and deduplicates media URIs. |
| `made_for_you(timezone: &str, paging: Paging)` | `Result<Page<MediaItem>, Error>` | Read Spotify’s Made For You shelf. |
| `top_mixes(timezone: &str, paging: Paging)` | `Result<Page<MediaItem>, Error>` | Read Spotify’s Top Mixes shelf. |

## Search method reference

Receiver arguments are omitted. `Value` means `serde_json::Value`; the return column includes the complete `Result` type.

| Method and inputs | Returns | Behavior |
| --- | --- | --- |
| `suggestion_texts(query: &str, limit: u32)` | `Result<Vec<String>, Error>` | Extract autocomplete strings from the raw response. |
| `recent_items()` | `Result<Vec<MediaItem>, Error>` | Map recent searches to media items. |
| `query(kind: SearchKind, query: &str, paging: Paging)` | `Result<Page<MediaItem>, Error>` | Search one entity kind; validates a nonempty query and paging. |
| `all(query: &str, limit: u32)` | `Result<SearchResults, Error>` | Read all nine search kinds with bounded concurrency; any failure returns an error. |
| `suggestions(query: &str, limit: u32)` | `Result<Value, Error>` | Read the raw autocomplete response. |
| `recent()` | `Result<Value, Error>` | Read raw recent searches. |
| `remove_recent(uris: &[String])` | `Result<Value, Error>` | Remove the supplied URIs from search history. |

## Catalog method reference

Receiver arguments are omitted. `Value` means `serde_json::Value`; the return column includes the complete `Result` type.

| Method and inputs | Returns | Behavior |
| --- | --- | --- |
| `track(value: &str)` | `Result<MediaItem, Error>` | Read normalized media metadata. |
| `album(value: &str)` | `Result<MediaItem, Error>` | Read normalized media metadata. |
| `album_tracks(value: &str, paging: Paging)` | `Result<Page<MediaItem>, Error>` | Page album track rows. |
| `artist(value: &str)` | `Result<MediaItem, Error>` | Read normalized media metadata. |
| `artist_page(value: &str)` | `Result<Value, Error>` | Read the raw artist overview. |
| `artist_top_tracks(value: &str)` | `Result<Vec<MediaItem>, Error>` | Map the artist overview’s top tracks. |
| `discography(value: &str, paging: Paging)` | `Result<Page<MediaItem>, Error>` | Page artist releases. |
| `related_artists(value: &str)` | `Result<Value, Error>` | Read related-artist data. |
| `appears_on(value: &str)` | `Result<Value, Error>` | Read releases featuring the artist. |
| `similar_albums(value: &str, limit: u32)` | `Result<Value, Error>` | Read album-context suggestions. |
| `radio_blend(seed_tracks: &[String], seed_artists: &[String], limit: u32)` | `Result<Vec<MediaItem>, Error>` | Interleave separate single-seed radios; not joint multi-seed ranking. |
| `recommendations(seed_tracks: &[String], seed_artists: &[String], limit: u32)` | `Result<Vec<MediaItem>, Error>` | Call the separate multi-seed endpoint; live checks currently return 429. |
| `track_info(value: &str)` | `Result<TrackInfo, Error>` | Read typed detailed metadata. |
| `album_info(value: &str)` | `Result<AlbumInfo, Error>` | Read typed detailed metadata. |
| `artist_info(value: &str)` | `Result<ArtistInfo, Error>` | Read typed detailed metadata. |
