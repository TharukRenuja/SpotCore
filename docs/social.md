# Social, lyrics and Jam

[Users](#users-method-reference) · [Extras](#extras-method-reference) · [Recents](#recents-method-reference) · [Jam](#jam-method-reference) · [Blends](#blends-method-reference)

Social/extras methods share the authenticated service session. Most responses are JSON; availability depends on the account, region and content.

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

`users()` also exposes public profiles, follows/followers, top listening data, friend activity, private-session controls and blocking. Do not promise that private-session controls erase existing history. `extras()` includes canvas, radio and taste exclusions. Lyrics can be missing (including 404) or have different timing detail; translations and word timing are not guaranteed.

`blends()` exposes invitation, join, group, story, rename and leave operations. Blend and Jam are separate features. Method availability is not proof every social mutation has been live-verified.

## Host a Jam and share its invite

Premium hosting requires an available Connect receiver on the same account. The receiver can play audio independently of the SDK's HTTP services.

```rust
use spotcore::SpotCore;

async fn host_jam(spotify: &SpotCore) -> Result<(), spotcore::Error> {
    spotify.jam().start().await?;
    let session = spotify.jam().session().await?;
    if let Some(id) = session["session_id"].as_str() {
        let invite = spotify.jam().invite_link(id).await?;
        println!("{}", invite["shareableUrl"]);
    }
    Ok(())
}
```

Keep the Jam active while sharing. Call `jam().end().await?` when the host chooses to end it. Avoid starting a new Jam over an unrelated existing session: read `session()` first in production UI.

**Verified live:** Premium browserless start, session read, end with cleanup, and HTTPS invite generation through the SDK. Invites use the current device's session and reject mismatched session IDs. Invite generation succeeded; completing a guest join using that generated link was not tested.

**Not supported as a verified feature:** guest joining through SpotCore. The public join call returned 411; a diagnostic request reached 403 `USER_NOT_ALLOWED`. Its cause remains unresolved. Guests should use Spotify's official app.

`availability()` returns client configuration (`ENABLED`), not server-confirmed hosting eligibility. When no Jam is active, `session()` returns `null` without an error; this lookup is per-device, not an account-wide session index.

`set_queue_only(true)` expresses queue-only guest permissions; `false` requests playback controls again. `set_participant_volume_control` and `remove_member` are also exposed. Their actual guest effects and guest leave remain unverified. HTTP mode has no live `session_update` push stream. Soloist supplies a playback receiver, not the SDK's Jam hosting API.

## Extras method reference

Receiver arguments are omitted. `Value` means `serde_json::Value`; the return column includes the complete `Result` type.

| Method and inputs | Returns | Behavior |
| --- | --- | --- |
| `radio_uri(seed: &str)` | `Result<Option<String>, Error>` | Resolve an optional single-seed radio context; malformed data is an error. |
| `lyrics(value: &str)` | `Result<Value, Error>` | Read available lyric data; missing lyrics can return 404. |
| `canvas(value: &str)` | `Result<Value, Error>` | Read available Canvas video metadata. |
| `credits(value: &str)` | `Result<Value, Error>` | Read track credit metadata. |
| `radio(value: &str)` | `Result<Value, Error>` | Read the raw single-seed radio response. |
| `set_excluded_from_taste(value: &str, enabled: bool)` | `Result<Value, Error>` | Write/remove an ignore-in-recommendations preference. |
| `set_artist_banned(value: &str, enabled: bool)` | `Result<Value, Error>` | Write/remove an artist-ban preference. |
| `is_excluded_from_taste(values: &[String])` | `Result<Value, Error>` | Read ignore-in-recommendations membership. |
| `is_artist_banned(values: &[String])` | `Result<Value, Error>` | Read artist-ban membership. |

## Recents method reference

Receiver arguments are omitted. `Value` means `serde_json::Value`; the return column includes the complete `Result` type.

| Method and inputs | Returns | Behavior |
| --- | --- | --- |
| `entries(paging: Paging)` | `Result<Value, Error>` | Read original grouped recent-listening rows; not recent searches. |

## Users method reference

Receiver arguments are omitted. `Value` means `serde_json::Value`; the return column includes the complete `Result` type.

| Method and inputs | Returns | Behavior |
| --- | --- | --- |
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

Receiver arguments are omitted. `Value` means `serde_json::Value`; the return column includes the complete `Result` type.

| Method and inputs | Returns | Behavior |
| --- | --- | --- |
| `availability()` | `Result<Value, Error>` | Return client configuration, not server-confirmed account eligibility. |
| `session()` | `Result<Value, Error>` | Read the selected device’s Jam, or null when absent. |
| `start()` | `Result<Value, Error>` | Request hosting; Premium and a Connect receiver are required. |
| `join(token: &str)` | `Result<Value, Error>` | Consume an invitation; Jam joining is not verified/supported. |
| `leave()` | `Result<Value, Error>` | Request guest leave; not live-verified. |
| `end()` | `Result<Value, Error>` | End a hosted Jam; live-verified with absence checked afterward. |
| `invite_link(session_id: &str)` | `Result<Value, Error>` | Generate a shareable URL for the matching current Jam; live-verified. |
| `remove_member(value: &str)` | `Result<Value, Error>` | Remove the supplied participant user URI from the current Jam; unverified live. |
| `set_queue_only(enabled: bool)` | `Result<Value, Error>` | Request queue-only permissions; actual guest effects unverified. |
| `set_participant_volume_control(enabled: bool)` | `Result<Value, Error>` | Request participant volume permissions; guest effects unverified. |

## Blends method reference

Receiver arguments are omitted. `Value` means `serde_json::Value`; the return column includes the complete `Result` type.

| Method and inputs | Returns | Behavior |
| --- | --- | --- |
| `invite()` | `Result<Value, Error>` | Create a Blend invitation. |
| `invitation(link_or_id: &str)` | `Result<Value, Error>` | Read a Blend invitation from a link or ID. |
| `join(link_or_id: &str)` | `Result<Value, Error>` | Accept a Blend invitation link or ID; separate from Jam joining. |
| `group(value: &str)` | `Result<Value, Error>` | Read a Blend group. |
| `story(value: &str)` | `Result<Value, Error>` | Read Blend story/taste-match data. |
| `rename(value: &str, name: &str)` | `Result<Value, Error>` | Rename the Blend identified by its playlist URI. |
| `leave(value: &str)` | `Result<Value, Error>` | Leave the Blend playlist identified by its URI. |

## Social inputs and time ranges

`Users` profile/follow/block methods accept Spotify user URIs. `top` accepts `TimeRange::{ShortTerm, MediumTerm, LongTerm}` and a limit 1–100; the response contains both track and artist sections. These are Spotify-defined windows, not an arbitrary start/end-date query. `history()` reads contexts while `recents().entries(Paging)` retains grouped recent-listening rows; neither is the same as `search().recent_items()`.

Extras typically take track URIs for lyrics/canvas/credits; taste exclusions and artist-ban methods write Spotify collection preferences. They do not filter the SDK’s local output or guarantee every receiver enforces them immediately.

Blend invitation methods take the supported invitation URL/ID form; group/story, rename and leave take the Blend playlist URI. Jam `join` expects a join token, not an entire redirect URL; it remains unverified/unsupported. Never publish private invite tokens in logs. Read the current session before host mutations.
