# Typed services and metadata recovery

The typed helpers centralize Spotify response variants and safe operation sequencing. Existing raw APIs remain available. Applications own navigation, presentation, refresh scheduling, optimistic state and image rendering. Service methods are available through `SpotCore` or an authenticated `Api`; playback workflows require a `SpotCore` session. Method names below omit the receiver unless shown explicitly.

## Progressive metadata recovery

Home feeds and Browse grids can contain names and URIs while artwork fields are null. `feed_info()` keeps that initial read separate from enrichment, so a large feed does not block behind hundreds of metadata reads.

For a `HomeFeed` or `BrowsePage`, call `recover_metadata(&api, Paging { offset, limit: 4 }).await?` on the background runtime. The method merges each completed page into every matching card in that result. Follow the returned `next_offset` over the same card order until it is `None`. Publish the updated result after each batch. Filling artwork does not shift the cursor, and repeated URIs are read once per batch sequence.

`api.recover_media(&uris, paging)` and the binding operation `home.recover_media` provide the same account-scoped resolver for queues, library cards and custom views. Results include the requested URI, optional normalized media and the actual per-item error. A successful neighbor survives another item's failure. The page's `retry_after` exposes the largest reported 429 delay; pause before continuing and retry failed URIs separately with a bounded application policy.

Recovery supports tracks, episodes, chapters, albums, artists, playlists, shows and audiobook representations through existing domains. Each read has an eight-second timeout. Batches contain at most four unique items and share a per-client lock/cache to prevent overlapping views from duplicating successful reads. No playback or transfer occurs. Retry-After handling remains in the shared authenticated HTTP layer. Missing metadata is never replaced with invented titles, artists or URLs.

Complete feed cards seed the session cache before recovery. Enrichment preserves occurrence UIDs, requested URIs, queue metadata and collection attribution, including Spotify relinking. Source artwork is returned as URLs; downloading, decoding, scaling and drawing images remain application responsibilities.

Legacy queue and section convenience reads perform a bounded best-effort batch using the same resolver. They do not enrich every card in a feed. Applications that need complete enrichment should use the explicit continuation API. The existing session cache is bounded and is not a persistent artwork database.

## Typed metadata and services

`MediaItem::metadata()` normalizes artists, authors, narrators, audiobook/ad classification, explicit/playability flags and queue contributor metadata. Unknown playability stays unknown. `require_playable()` rejects an explicit lock/restriction; a successful local check still requires handling Spotify's playback response. `PlayerSnapshot::playback_speed()` preserves missing speed as unknown.

| Operation | Result |
| --- | --- |
| `users.availability` | Country/account/tier policy results; see [availability](availability.md) |
| `users.rankings` | Tracks and artists for a supported Spotify time range; affinity rankings, not play counts |
| `users.profile_info` | Typed profile with optional unavailable fields and retained raw data |
| `users.followers_info` / `users.following_info` | Normalized profile cards |
| `users.activity` | Normalized friends and current media; unknown status remains optional |
| `recents.items` | Spotify's grouped recent entities with offset continuation, not complete timestamped listening history |
| `catalog.statistics` | Available album/artist play counts and optional monthly listeners |
| `extras.prerelease_info` | Countdown identity, resolved album metadata and optional release-date string |
| `extras.lyrics_info` | Typed Spotify lyrics, instrumental state and valid ordered timing; only a 404 means no lyrics |
| `extras.chapter_markers` | Display-segment timestamps, with validated description fallback and source indication |
| `extras.resolve_share_url` | Spotify-controlled HTTPS redirects without account authorization headers |
| `playlists.create_playlist` | Validated created playlist URI; ambiguous creation is not automatically replayed |
| `playlists.remove_rows` | Exact occurrence removal with revision checks and bounded paging |
| `library.all_playlists` | Playlist traversal across folders with deduplication and pagination guards |

Chapter display segments are separate from playable audiobook chapters. The chapter decoder validates item identity, payload type, wire lengths, marker order and duration. An authorization, rate-limit or transport failure is returned, rather than quietly becoming an empty chapter list. Description fallback requires a zero start, at least three titled entries and at least thirty seconds between markers.

Browse-category normalization also handles nested BrowseCard representations. Shelf selection and localized display-family grouping remain product decisions.

## Jam and queue operations

Use `jam.state()` or `jam.refresh()` for typed membership, `members()` for participant records, and `permissions()` for effective shared-host versus listen-here controls. Unknown participant-volume permission stays optional. These are snapshots; the server checks permissions again on writes.

`jam.start_session()` is an explicit, Premium-checked creation operation. It refuses to start another Jam while this account is a guest. State reads, refreshes and invitations do not create sessions. `jam.join_on(token, ListenOn::Host)` does not require a ready local player. `ListenOn::ThisDevice` still requires the account and receiver to support that operation. End and leave retire the session identity so delayed membership responses cannot restore it.

`queues.snapshot()` retains receiver/Jam scope and permissions. Keep the snapshot when editing. `snapshot.item_at(index, expected_uri)` resolves an exact occurrence, including repeated songs. `queues.skip_to(&snapshot, &item)` validates the current scope and delegates to the existing Connect skip command. Do not rebuild the queue to simulate a jump or reorder.

## Playback

`wait_ready(timeout)` waits for the owned receiver without activation or transfer and honors rate-limit delays. Native repeat changes use Connect so both repeat flags are set together. Native volume reads prefer the receiver's reported value, with the native snapshot as a fallback when volume is supported. If neither provides a value, `volume()` returns `Error::Unsupported`.

`play_spoken(uri, position, device)` handles episode/chapter routing and confirms a Native selection before restoring a nonzero position. `play_audiobook(value, chapter, position, device)` applies account policy and verifies the selected chapter's membership/access. Without an explicit chapter it starts at chapter one. It rejects a resume offset without a chapter URI; automatic account-wide audiobook resume discovery is not implemented.

`quality_report()` distinguishes `WebStream` from `DevicePreference`. `lossless_available` describes device/account capability and does not prove a delivered lossless stream. Audio processing and OS output configuration remain application responsibilities.

## Adopting typed workflows

Replace one raw-response integration at a time. Compare identifiers, optional fields, pagination and failure behavior before removing an application parser. Keep cancellation, optimistic state and per-account request isolation intact. Send each mutation through one implementation; do not execute the raw and typed write paths together.

Typed helpers normalize supported response variants. Check these compatibility limits when adopting them:

- **Profiles:** `UserProfileInfo::raw` retains fields outside the normalized profile.
- **Recent items:** `recents().items()` returns grouped recent entities, not a complete timestamped play log.
- **Lyrics:** the typed parser requires a `lyrics.lines` array, including for instrumental responses. Use `extras().lyrics()` for payloads an application handles outside the typed parser.
- **Errors:** a parse error, HTTP 404 and an empty successful result have different meanings. Preserve those distinctions in the application.
