# Binding operation reference

Generated from the public service methods and session playback controls. Run `python3 scripts/generate_ffi_operations.py --check` to detect missing adapters.

Use `invoke(operation, arguments_json)` on a language session. Arguments are a JSON object with the names below. Results retain the Rust SDK's serialized field names; methods returning `Value` retain Spotify's original JSON. Errors use `BindingError`.

Direct duration arguments (`position`, `amount`) are integer **milliseconds**. `Command` arguments use the Rust externally tagged enum encoding; a nested `Duration` inside a command is `{"secs":30,"nanos":0}`. Prefer the named seek/rewind/context operations for millisecond arguments. `Paging` is `{"limit":20,"offset":0}`. Optional arguments may be omitted or null. JPEG covers are arrays of byte values, or null to remove a cover. Enum strings use Rust variant names: `Tracks`, `ShortTerm`, `Public`, `VeryHigh`, `Smart`, `Off`, etc. See the category guides for permissions, response types and feature limitations.

A listed operation means it is exposed, not that every account supports it. `jam.join` preserves the SDK's current unsupported/unverified behavior. Session creation/login/shutdown and event consumption use the existing lifecycle methods; worker events are not fetched through `invoke`.
## Blends

| Operation | Arguments | Result |
| --- | --- | --- |
| `blends.group` | `value: String` | `Value` |
| `blends.invitation` | `link_or_id: String` | `Value` |
| `blends.invite` | `{}` | `Value` |
| `blends.join` | `link_or_id: String` | `Value` |
| `blends.leave` | `value: String` | `Value` |
| `blends.rename` | `value: String, name: String` | `Value` |
| `blends.story` | `value: String` | `Value` |

## Catalog

| Operation | Arguments | Result |
| --- | --- | --- |
| `catalog.album` | `value: String` | `MediaItem` |
| `catalog.album_info` | `value: String` | `AlbumInfo` |
| `catalog.album_tracks` | `value: String, paging: Paging` | `Page<MediaItem>` |
| `catalog.appears_on` | `value: String` | `Value` |
| `catalog.artist` | `value: String` | `MediaItem` |
| `catalog.artist_info` | `value: String` | `ArtistInfo` |
| `catalog.artist_page` | `value: String` | `Value` |
| `catalog.artist_top_tracks` | `value: String` | `Vec<MediaItem>` |
| `catalog.discography` | `value: String, paging: Paging` | `Page<MediaItem>` |
| `catalog.radio_blend` | `seed_tracks: Vec<String>, seed_artists: Vec<String>, limit: u32` | `Vec<MediaItem>` |
| `catalog.recommendations` | `seed_tracks: Vec<String>, seed_artists: Vec<String>, limit: u32` | `Vec<MediaItem>` |
| `catalog.related_artists` | `value: String` | `Value` |
| `catalog.similar_albums` | `value: String, limit: u32` | `Value` |
| `catalog.track` | `value: String` | `MediaItem` |
| `catalog.track_info` | `value: String` | `TrackInfo` |

## Devices

| Operation | Arguments | Result |
| --- | --- | --- |
| `devices.audio_quality` | `device: Device` | `AudioQuality` |
| `devices.list` | `{}` | `Vec<Device>` |
| `devices.pull_here` | `{}` | `Value` |
| `devices.set_audio_quality` | `device: Device, quality: AudioQuality` | `AudioQuality` |
| `devices.transfer` | `device_id: String, play: bool` | `Value` |

## Extras

| Operation | Arguments | Result |
| --- | --- | --- |
| `extras.canvas` | `value: String` | `Value` |
| `extras.credits` | `value: String` | `Value` |
| `extras.is_artist_banned` | `values: Vec<String>` | `Value` |
| `extras.is_excluded_from_taste` | `values: Vec<String>` | `Value` |
| `extras.lyrics` | `value: String` | `Value` |
| `extras.radio` | `value: String` | `Value` |
| `extras.radio_uri` | `seed: String` | `Option<String>` |
| `extras.set_artist_banned` | `value: String, enabled: bool` | `Value` |
| `extras.set_excluded_from_taste` | `value: String, enabled: bool` | `Value` |

## Home

| Operation | Arguments | Result |
| --- | --- | --- |
| `home.browse_categories` | `{}` | `Vec<BrowseCategory>` |
| `home.browse_page` | `value: String, sections: Paging, items: Paging` | `BrowsePage` |
| `home.categories` | `{}` | `Value` |
| `home.category` | `value: String` | `Value` |
| `home.feed` | `timezone: String, facet: Option<String>, items_per_shelf: u32` | `Value` |
| `home.feed_info` | `timezone: String, facet: Option<String>, items_per_shelf: u32` | `HomeFeed` |
| `home.made_for_you` | `timezone: String, paging: Paging` | `Page<MediaItem>` |
| `home.new_releases` | `paging: Paging` | `Page<MediaItem>` |
| `home.section` | `value: String, timezone: String, paging: Paging` | `Page<MediaItem>` |
| `home.top_mixes` | `timezone: String, paging: Paging` | `Page<MediaItem>` |
| `home.whats_new` | `paging: Paging` | `Page<MediaItem>` |

## Jam

| Operation | Arguments | Result |
| --- | --- | --- |
| `jam.availability` | `{}` | `Value` |
| `jam.end` | `{}` | `Value` |
| `jam.invite_link` | `session_id: String` | `Value` |
| `jam.join` | `token: String` | `Value` |
| `jam.leave` | `{}` | `Value` |
| `jam.remove_member` | `value: String` | `Value` |
| `jam.session` | `{}` | `Value` |
| `jam.set_participant_volume_control` | `enabled: bool` | `Value` |
| `jam.set_queue_only` | `enabled: bool` | `Value` |
| `jam.start` | `{}` | `Value` |

## Library

| Operation | Arguments | Result |
| --- | --- | --- |
| `library.add` | `values: Vec<String>` | `Value` |
| `library.contains` | `values: Vec<String>` | `Vec<bool>` |
| `library.create_folder` | `name: String` | `Value` |
| `library.delete_folder` | `value: String` | `Value` |
| `library.folders` | `{}` | `Vec<LibraryEntry>` |
| `library.follow_artist` | `value: String` | `Value` |
| `library.follow_show` | `value: String` | `Value` |
| `library.hidden_songs` | `cursor: Option<String>, limit: u32` | `spotcore_core::CursorPage<HiddenSong>` |
| `library.items` | `filter: Option<String>, order: String, folder: Option<String>, paging: Paging` | `Page<MediaItem>` |
| `library.items_detailed` | `filter: Option<String>, order: String, folder: Option<String>, text_filter: String, paging: Paging` | `Page<LibraryItem>` |
| `library.liked_tracks` | `paging: Paging` | `Page<MediaItem>` |
| `library.move_to_folder` | `value: String, folder: Option<String>` | `Value` |
| `library.pin` | `value: String, pinned: bool` | `Value` |
| `library.remove` | `values: Vec<String>` | `Value` |
| `library.rename_folder` | `value: String, name: String` | `Value` |
| `library.unfollow_artist` | `value: String` | `Value` |
| `library.unfollow_show` | `value: String` | `Value` |

## Playback

| Operation | Arguments | Result |
| --- | --- | --- |
| `playback.add_to_queue` | `uris: Vec<String>` | `()` |
| `playback.audio_info` | `{}` | `AudioInfo` |
| `playback.capabilities` | `{}` | `Capabilities` |
| `playback.clear_queue` | `{}` | `()` |
| `playback.current_track` | `{}` | `Option<MediaItem>` |
| `playback.drm_support` | `{}` | `crate::web::DrmSupport` |
| `playback.execute` | `command: Command` | `()` |
| `playback.execute_on` | `device: String, command: Command` | `()` |
| `playback.forward` | `amount: u64` | `()` |
| `playback.insert_into_queue` | `uris: Vec<String>, before: Option<QueueItem>` | `()` |
| `playback.is_playing` | `{}` | `bool` |
| `playback.next` | `{}` | `()` |
| `playback.pause` | `{}` | `()` |
| `playback.play` | `uri: String` | `()` |
| `playback.play_context` | `uri: String, track: Option<String>, index: Option<u32>, position: Option<u64>` | `()` |
| `playback.play_on` | `device: String, uri: String` | `()` |
| `playback.position_ms` | `{}` | `u64` |
| `playback.previous` | `{}` | `()` |
| `playback.queue` | `{}` | `Queue` |
| `playback.refresh_state` | `{}` | `Option<PlayerSnapshot>` |
| `playback.remove_from_queue` | `items: Vec<QueueItem>` | `()` |
| `playback.reorder_queue` | `items: Vec<QueueItem>, before: Option<QueueItem>, after: Option<QueueItem>` | `()` |
| `playback.resume` | `{}` | `()` |
| `playback.rewind` | `amount: u64` | `()` |
| `playback.seek` | `position: u64` | `()` |
| `playback.set_quality` | `quality: AudioQuality` | `AppliedQuality` |
| `playback.set_repeat` | `mode: RepeatMode` | `()` |
| `playback.set_shuffle` | `on: bool` | `()` |
| `playback.set_shuffle_mode` | `mode: ShuffleMode` | `()` |
| `playback.set_speed` | `speed: f64` | `()` |
| `playback.set_volume` | `volume: f64` | `()` |
| `playback.skip_to_queue` | `item: QueueItem` | `()` |
| `playback.state` | `{}` | `Option<PlayerSnapshot>` |
| `playback.stop` | `{}` | `()` |
| `playback.toggle` | `{}` | `()` |
| `playback.volume` | `{}` | `f64` |

## Playlists

| Operation | Arguments | Result |
| --- | --- | --- |
| `playlists.access` | `value: String` | `Value` |
| `playlists.add_tracks` | `value: String, tracks: Vec<String>` | `()` |
| `playlists.added_by` | `value: String, paging: Paging` | `Page<PlaylistAdder>` |
| `playlists.collaborator_invite` | `value: String` | `String` |
| `playlists.create` | `name: String, folder: Option<String>` | `Value` |
| `playlists.delete` | `value: String` | `Value` |
| `playlists.follow` | `value: String` | `Value` |
| `playlists.follow_artist` | `value: String` | `Value` |
| `playlists.follow_show` | `value: String` | `Value` |
| `playlists.join` | `value: String, invitation_token: String` | `Value` |
| `playlists.members` | `value: String` | `Value` |
| `playlists.mine` | `{}` | `Vec<LibraryEntry>` |
| `playlists.move_tracks` | `value: String, uids: Vec<String>, before: Option<String>` | `Value` |
| `playlists.playlist` | `value: String` | `MediaItem` |
| `playlists.playlist_info` | `value: String` | `PlaylistInfo` |
| `playlists.published` | `value: String` | `Value` |
| `playlists.recommended` | `value: String, limit: u32, skip: Vec<String>` | `Vec<TrackInfo>` |
| `playlists.remove_member` | `value: String, username: String` | `Value` |
| `playlists.remove_tracks` | `value: String, uids: Vec<String>` | `Value` |
| `playlists.rename` | `value: String, name: String` | `Value` |
| `playlists.set_access` | `value: String, access: PlaylistAccess` | `Value` |
| `playlists.set_cover` | `value: String, jpeg: Option<Vec<u8>>` | `Value` |
| `playlists.set_description` | `value: String, description: String` | `Value` |
| `playlists.set_hidden` | `playlist: String, track: String, hidden: bool` | `()` |
| `playlists.set_member_collaborator` | `value: String, username: String, enabled: bool` | `Value` |
| `playlists.set_published` | `value: String, published: bool` | `Value` |
| `playlists.tracks` | `value: String, paging: Paging` | `Page<MediaItem>` |
| `playlists.unfollow` | `value: String` | `Value` |
| `playlists.unfollow_artist` | `value: String` | `Value` |
| `playlists.unfollow_show` | `value: String` | `Value` |

## Podcasts

| Operation | Arguments | Result |
| --- | --- | --- |
| `podcasts.audiobook` | `value: String` | `Value` |
| `podcasts.chapters` | `value: String, paging: Paging` | `Page<MediaItem>` |
| `podcasts.episode` | `value: String` | `MediaItem` |
| `podcasts.episode_info` | `value: String` | `EpisodeInfo` |
| `podcasts.episodes` | `value: String, paging: Paging` | `Page<MediaItem>` |
| `podcasts.saved_episodes` | `{}` | `Value` |
| `podcasts.set_saved` | `value: String, saved: bool` | `Value` |
| `podcasts.show` | `value: String` | `MediaItem` |
| `podcasts.show_details` | `value: String` | `Value` |
| `podcasts.show_info` | `value: String` | `ShowInfo` |

## Recents

| Operation | Arguments | Result |
| --- | --- | --- |
| `recents.entries` | `paging: Paging` | `Value` |

## Search

| Operation | Arguments | Result |
| --- | --- | --- |
| `search.all` | `query: String, limit: u32` | `SearchResults` |
| `search.query` | `kind: SearchKind, query: String, paging: Paging` | `Page<MediaItem>` |
| `search.recent` | `{}` | `Value` |
| `search.recent_items` | `{}` | `Vec<MediaItem>` |
| `search.remove_recent` | `uris: Vec<String>` | `Value` |
| `search.suggestion_texts` | `query: String, limit: u32` | `Vec<String>` |
| `search.suggestions` | `query: String, limit: u32` | `Value` |

## Users

| Operation | Arguments | Result |
| --- | --- | --- |
| `users.follow` | `values: Vec<String>` | `Value` |
| `users.followers` | `value: String` | `Value` |
| `users.following` | `value: String` | `Value` |
| `users.friend_activity` | `{}` | `Value` |
| `users.history` | `{}` | `Value` |
| `users.is_blocked` | `value: String` | `bool` |
| `users.is_following` | `values: Vec<String>` | `Value` |
| `users.me` | `{}` | `Account` |
| `users.profile` | `value: String` | `Value` |
| `users.set_blocked` | `value: String, blocked: bool` | `Value` |
| `users.set_private_session` | `enabled: bool` | `Value` |
| `users.top` | `range: TimeRange, limit: u32` | `Value` |
| `users.unfollow` | `values: Vec<String>` | `Value` |
