# Social, lyrics and Jam

[Users](#users-method-reference) · [Extras](#extras-method-reference) · [Recents](#recents-method-reference) · [Jam](#jam-method-reference) · [Blends](#blends-method-reference)

Social/extras methods share the authenticated service session. Most responses are JSON. Availability depends on the account, region and content.

## Profiles and track extras

```rust
use spotcore::SpotCore;

async fn extras(spotify: &SpotCore, track_uri: &str) -> Result<(), spotcore::Error> {
    let me = spotify.users().me().await?;
    println!("{}: {:?}", me.name, me.tier);
    let lyrics = spotify.extras().lyrics(track_uri).await?;
    let credits = spotify.extras().credits(track_uri).await?;
    println!("Lyrics: {lyrics}; credits: {credits}");
    Ok(())
}
```

`users()` also exposes public profiles, follows/followers, top listening data, friend activity, private-session controls and blocking. Do not promise that private-session controls erase existing history. `extras()` includes canvas, radio and taste exclusions. Lyrics can be missing (including 404) or have different timing detail. Translations and word timing are not guaranteed.

`blends()` exposes invitation, join, group, story, rename and leave operations. Blend and Jam are separate features. **Handle service rejection and verify the resulting state after social writes.** Account, region and privacy rules still apply.

## Edit the current profile

**`users().update_profile(name, image)` edits only the signed-in account.** Supply a display name of 1 to 32 characters and optionally a JPEG byte slice. `None` leaves the photo unchanged. An image must be no larger than 10 MiB. Prepare a square image of at least 300 by 300 pixels in the application.

The SDK resolves the current username, uploads a selected image, updates the name through the identity service, then assigns the uploaded image token. **Name and photo writes are not atomic.** Refresh the profile after an error because a preceding step may have succeeded. Writes are not automatically replayed. The request format follows Spotify's current Web Player, and remains subject to upstream changes. Live account validation is still required.

## Host a Jam and share its invite

Premium hosting requires an available Connect receiver on the same account. The receiver can play audio independently of the SDK's HTTP services.

```rust
use spotcore::SpotCore;

async fn host_jam(spotify: &SpotCore) -> Result<(), spotcore::Error> {
    let jam = spotify.jam();
    let session = jam.start_session().await?;
    let invite = jam.invitation_url(&session.id).await?;
    println!("{invite}");
    Ok(())
}
```

Keep the Jam active while sharing. Call `jam().end().await?` when the host chooses to end it. Avoid starting a new Jam over an unrelated existing session: read `state()` first before enabling Start.

**Host workflow:** explicitly start a Jam, retain the typed session, generate its HTTPS invite, and end it when the host chooses. Invites use the current device's session and reject mismatched session IDs.

## Join a Jam as a guest

Use `jam().join_on(invite, ListenOn::Host)` to join and control the host's receiver without requiring a local player. Use `ListenOn::ThisDevice` only when the account and receiver support listening locally. The joining account must differ from the host. `join_on()` returns optional confirmed membership. Enable guest controls only after receiving `Some(JamSession)`. Its permissions describe the current session; writes can still be rejected.

Invites may be full `https://open.spotify.com/socialsession/<token>` links, `spotify:socialsession:<token>` URIs or bare tokens. Resolve short `spotify.link` URLs with `extras().resolve_share_url()` first. The low-level `join()` method selects a target from account tier; `join_as()` accepts an explicit boolean target. Prefer `join_on()` when the application presents that choice.

```rust
use spotcore::{SpotCore, web::ListenOn};

async fn join_jam(spotify: &SpotCore, invite: &str) -> Result<(), spotcore::Error> {
    let jam = spotify.jam();
    jam.session_info(invite).await?;
    if let Some(session) = jam.join_on(invite, ListenOn::Host).await? {
        println!("Joined {}", session.id);
    }
    Ok(())
}
```

Call `session_info()` first to preview the Jam and confirm the invite is still valid. Use `state()` or `refresh()` for subsequent typed membership reads. Spotify can still reject a join with HTTP 403 `USER_NOT_ALLOWED` for account, region or eligibility rules; treat a rejection as terminal rather than retrying. Guest track control remains unverified across accounts. Never publish private invite tokens in logs.

`availability()` returns client configuration (`ENABLED`), not server-confirmed hosting eligibility. When no Jam is active, `session()` returns `null` without an error. This lookup is per-device, not an account-wide session index.

`set_queue_only(true)` requests queue-only guest permissions. Pass `false` to request playback controls again. `set_participant_volume_control` and `remove_member` request guest volume permissions and participant removal. **Verify the resulting session state after guest-control changes.** Guest-control effects are not validated across accounts. HTTP mode has no live `session_update` push stream. Soloist supplies a playback receiver. Hosting requests use the service API.

## Extras method reference

Receiver arguments are omitted. `Value` means `serde_json::Value`. The return column includes the complete `Result` type.

| Method and inputs | Returns | Behavior |
| --- | --- | --- |
| `radio_uri(seed: &str)` | `Result<Option<String>, Error>` | Resolve an optional single-seed radio context. Malformed data is an error. |
| `lyrics_info(value: &str)` | `Result<Option<SpotifyLyrics>, Error>` | Normalize lyrics; only HTTP 404 returns None. Parse failures remain errors. |
| `chapter_markers(value: &str)` | `Result<ChapterMarkers, Error>` | Read display-segment timestamps and their source. Not playable audiobook chapters. |
| `prerelease_info(value: &str)` | `Result<PrereleaseInfo, Error>` | Resolve countdown identity, album metadata and optional release date. |
| `resolve_share_url(value: &str)` | `Result<String, Error>` | Resolve Spotify-controlled HTTPS redirects without authorization headers. |
| `lyrics(value: &str)` | `Result<Value, Error>` | Read available lyric data. Missing lyrics can return 404. |
| `canvas(value: &str)` | `Result<Value, Error>` | Read available Canvas video metadata. |
| `credits(value: &str)` | `Result<Value, Error>` | Read track credit metadata. |
| `radio(value: &str)` | `Result<Value, Error>` | Read the raw single-seed radio response. |
| `set_excluded_from_taste(value: &str, enabled: bool)` | `Result<Value, Error>` | Write/remove an ignore-in-recommendations preference. |
| `set_artist_banned(value: &str, enabled: bool)` | `Result<Value, Error>` | Write/remove an artist-ban preference. |
| `is_excluded_from_taste(values: &[String])` | `Result<Value, Error>` | Read ignore-in-recommendations membership. |
| `is_artist_banned(values: &[String])` | `Result<Value, Error>` | Read artist-ban membership. |

## Recents method reference

Receiver arguments are omitted. `Value` means `serde_json::Value`. The return column includes the complete `Result` type.

| Method and inputs | Returns | Behavior |
| --- | --- | --- |
| `items(paging: Paging)` | `Result<Page<MediaItem>, Error>` | Read grouped recent entities with offset continuation. Not a timestamped play log. |
| `entries(paging: Paging)` | `Result<Value, Error>` | Read original grouped recent-listening rows. Not recent searches. |

## Users method reference

Receiver arguments are omitted. `Value` means `serde_json::Value`. The return column includes the complete `Result` type.

| Method and inputs | Returns | Behavior |
| --- | --- | --- |
| `update_profile(name: &str, image: Option<&[u8]>)` | `Result<(), Error>` | Update the current account name and optionally its JPEG photo. |
| `availability()` | `Result<AccountAvailability, Error>` | Evaluate country, tier and account feature policy. |
| `rankings(range: TimeRange, limit: u32)` | `Result<UserRankings, Error>` | Typed track and artist affinity rankings for a Spotify-defined period. |
| `profile_info(value: &str)` | `Result<UserProfileInfo, Error>` | Typed public profile with optional fields and retained raw data. |
| `followers_info(value: &str)` / `following_info(value: &str)` | `Result<Vec<ProfileCard>, Error>` | Normalize public profile cards. |
| `activity()` | `Result<Vec<FriendActivity>, Error>` | Normalize available friend activity. |
| `me()` | `Result<Account, Error>` | Resolve current profile, canonical identity, tier and country. |
| `profile(value: &str)` | `Result<Value, Error>` | Read a public user profile. |
| `followers(value: &str)` | `Result<Value, Error>` | Read the user’s followers. |
| `following(value: &str)` | `Result<Value, Error>` | Read accounts the user follows. |
| `top(range: TimeRange, limit: u32)` | `Result<Value, Error>` | Read both top tracks and artists for a time range. |
| `follow(values: &[String])` | `Result<Value, Error>` | Follow the supplied Spotify user URIs. |
| `unfollow(values: &[String])` | `Result<Value, Error>` | Unfollow the supplied Spotify user URIs. |
| `is_following(values: &[String])` | `Result<Value, Error>` | Read following status for user URIs. |
| `history()` | `Result<Value, Error>` | Read recent listening contexts through the service API. |
| `friend_activity()` | `Result<Value, Error>` | Read available friend-listening presence. |
| `set_private_session(enabled: bool)` | `Result<Value, Error>` | Check support before requesting private-session state. |
| `set_blocked(value: &str, blocked: bool)` | `Result<Value, Error>` | Set or clear a user block. |
| `is_blocked(value: &str)` | `Result<bool, Error>` | Check a user block. |

## Jam method reference

Receiver arguments are omitted. `Value` means `serde_json::Value`. The return column includes the complete `Result` type.

| Method and inputs | Returns | Behavior |
| --- | --- | --- |
| `start_session()` | `Result<JamSession, Error>` | Explicit Premium-checked hosting; reuse an owned session and reject starting while a guest. |
| `state()` / `refresh()` | `Result<Option<JamSession>, Error>` | Read typed membership; refresh forces a new service read. Neither creates a Jam. |
| `join_on(token: &str, target: ListenOn)` | `Result<Option<JamSession>, Error>` | Join with an explicit host/local listening target. |
| `invitation_url(session_id: &str)` | `Result<String, Error>` | Validate the current Jam's HTTPS invitation URL. |
| `availability()` | `Result<Value, Error>` | Return client configuration, not server-confirmed account eligibility. |
| `session()` | `Result<Value, Error>` | Read the selected device’s Jam, or null when absent. |
| `start()` | `Result<Value, Error>` | Request hosting. Premium and a Connect receiver are required. |
| `join(token: &str)` | `Result<Value, Error>` | Join as a guest from an invite link, `spotify:socialsession:` URI or bare token, choosing playback from the account tier. No Premium or receiver needed; the joining account must differ from the host. |
| `join_as(token: &str, listen_on_device: bool)` | `Result<Value, Error>` | Join with an explicit playback target: this account's own device or the host's. Spotify rejects `listen_on_device` for accounts that may not use it. |
| `session_info(token: &str)` | `Result<Value, Error>` | Preview a Jam from its invite link or token without joining. |
| `leave()` | `Result<Value, Error>` | Leave the current Jam as a guest. |
| `end()` | `Result<Value, Error>` | End a hosted Jam. Read the session afterward to confirm it ended. |
| `invite_link(session_id: &str)` | `Result<Value, Error>` | Generate a shareable URL for the matching current Jam. |
| `remove_member(value: &str)` | `Result<Value, Error>` | Request removal of a participant user URI. Verify the resulting session. |
| `set_queue_only(enabled: bool)` | `Result<Value, Error>` | Request queue-only permissions. Verify guest permissions afterward. |
| `set_participant_volume_control(enabled: bool)` | `Result<Value, Error>` | Request participant volume permissions. Verify the resulting state. |

## Blends method reference

Receiver arguments are omitted. `Value` means `serde_json::Value`. The return column includes the complete `Result` type.

| Method and inputs | Returns | Behavior |
| --- | --- | --- |
| `invite()` | `Result<Value, Error>` | Create a Blend invitation. |
| `invitation(link_or_id: &str)` | `Result<Value, Error>` | Read a Blend invitation from a link or ID. |
| `join(link_or_id: &str)` | `Result<Value, Error>` | Accept a Blend invitation link or ID. Separate from Jam joining. |
| `group(value: &str)` | `Result<Value, Error>` | Read a Blend group. |
| `story(value: &str)` | `Result<Value, Error>` | Read Blend story/taste-match data. |
| `rename(value: &str, name: &str)` | `Result<Value, Error>` | Rename the Blend identified by its playlist URI. |
| `leave(value: &str)` | `Result<Value, Error>` | Leave the Blend playlist identified by its URI. |

## Social inputs and time ranges

`Users` profile/follow/block methods accept Spotify user URIs. `top` accepts `TimeRange::{ShortTerm, MediumTerm, LongTerm}` and a limit 1–100. The response contains both track and artist sections. These are Spotify-defined windows, not an arbitrary start/end-date query. `history()` reads contexts while `recents().entries(Paging)` retains grouped recent-listening rows. Neither is the same as `search().recent_items()`.

Extras typically take track URIs for lyrics/canvas/credits. Taste exclusions and artist-ban methods write Spotify collection preferences. They do not filter the SDK’s local output or guarantee every receiver enforces them immediately.

Blend invitation methods take the supported invitation URL/ID form. Group/story, rename and leave take the Blend playlist URI. Jam `join` and `session_info` accept a `https://open.spotify.com/socialsession/<token>` link, a `spotify:socialsession:<token>` URI or a bare token; short `spotify.link` URLs must be resolved to that form first. Never publish private invite tokens in logs. Read the current session before host mutations.
