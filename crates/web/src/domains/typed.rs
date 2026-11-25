//! Stable results for services whose low-level JSON APIs remain available.
use super::*;
use crate::{AccountAvailability, RecoveryPage};
use serde::{Deserialize, Serialize};
use spotcore_core::{MediaItem, Page, Paging};

impl Users<'_> {
    /// Fresh account capability read. Failures propagate rather than becoming regional denials.
    pub async fn availability(&self) -> Result<AccountAvailability, Error> {
        Ok(self.me().await?.availability())
    }
}

impl Home<'_> {
    pub async fn recover_media(
        &self,
        uris: &[String],
        paging: Paging,
    ) -> Result<RecoveryPage, Error> {
        self.0.recover_media(uris, paging).await
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRankings {
    pub tracks: Vec<MediaItem>,
    pub artists: Vec<MediaItem>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfileInfo {
    pub uri: String,
    pub name: Option<String>,
    pub images: Vec<spotcore_core::Image>,
    pub follower_count: Option<u64>,
    pub following_count: Option<u64>,
    pub playlists: Option<Vec<MediaItem>>,
    pub following: Option<bool>,
    pub blocked: Option<bool>,
    pub raw: Value,
}
fn optional_count(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}
fn profile_image(value: &Value) -> Option<String> {
    [
        "large_image_url",
        "largeImageUrl",
        "image_url",
        "imageUrl",
        "image",
    ]
    .iter()
    .filter_map(|key| value.get(*key)?.as_str())
    .chain(
        value
            .get("images")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|image| image.get("url")?.as_str()),
    )
    .map(|url| crate::mapping::artwork_url(url.trim()))
    .find(|value| {
        url::Url::parse(value)
            .is_ok_and(|url| matches!(url.scheme(), "https" | "http") && url.host_str().is_some())
    })
}
fn rows(value: Option<&Value>) -> Vec<MediaItem> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(crate::mapping::item)
        .collect()
}
impl Users<'_> {
    pub async fn rankings(&self, range: TimeRange, limit: u32) -> Result<UserRankings, Error> {
        let body = self.top(range, limit).await?;
        let profile = body
            .pointer("/data/me/profile")
            .ok_or_else(|| Error::Backend("Spotify returned no rankings".into()))?;
        let tracks = profile
            .pointer("/topTracks/items")
            .filter(|value| value.is_array())
            .ok_or_else(|| Error::Backend("Spotify returned no track ranking".into()))?;
        let artists = profile
            .pointer("/topArtists/items")
            .filter(|value| value.is_array())
            .ok_or_else(|| Error::Backend("Spotify returned no artist ranking".into()))?;
        Ok(UserRankings {
            tracks: rows(Some(tracks)),
            artists: rows(Some(artists)),
        })
    }
    pub async fn profile_info(&self, value: &str) -> Result<UserProfileInfo, Error> {
        let body = self.profile(value).await?;
        if !body.is_object() {
            return Err(Error::Backend("Spotify returned no profile".into()));
        }
        let image = profile_image(&body);
        Ok(UserProfileInfo {
            uri: value.into(),
            name: body
                .get("name")
                .or_else(|| body.get("display_name"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            images: image
                .map(|url| {
                    vec![spotcore_core::Image {
                        url,
                        width: None,
                        height: None,
                    }]
                })
                .unwrap_or_default(),
            follower_count: body
                .get("followers_count")
                .or_else(|| body.get("followersCount"))
                .and_then(optional_count),
            following_count: body
                .get("following_count")
                .or_else(|| body.get("followingCount"))
                .and_then(optional_count),
            playlists: body
                .get("public_playlists")
                .or_else(|| body.get("publicPlaylists"))
                .or_else(|| body.get("playlists"))
                .filter(|value| value.is_array())
                .map(|value| rows(Some(value))),
            following: body.get("is_following").and_then(Value::as_bool),
            blocked: body.get("is_blocked").and_then(Value::as_bool),
            raw: body,
        })
    }
    pub async fn activity(&self) -> Result<Vec<FriendActivity>, Error> {
        let body = self.friend_activity().await?;
        let friends = body
            .get("friends")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::Backend("Spotify returned no friend activity list".into()))?;
        Ok(friends
            .iter()
            .filter_map(|friend| {
                let user = friend.get("user")?;
                let track = friend.get("track")?;
                Some(FriendActivity {
                    user_uri: user.get("uri")?.as_str()?.into(),
                    user_name: user.get("name").and_then(Value::as_str).map(str::to_owned),
                    item: crate::mapping::item(track)?,
                    timestamp_ms: friend.get("timestamp").and_then(Value::as_i64),
                    playing: friend.get("isPlaying").and_then(Value::as_bool),
                    raw: friend.clone(),
                })
            })
            .collect())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendActivity {
    pub user_uri: String,
    pub user_name: Option<String>,
    pub item: MediaItem,
    pub timestamp_ms: Option<i64>,
    pub playing: Option<bool>,
    pub raw: Value,
}
impl Recents<'_> {
    /// These are Spotify's grouped recent entities, not complete timestamped listening history.
    pub async fn items(&self, paging: Paging) -> Result<Page<MediaItem>, Error> {
        let body = self.entries(paging).await?;
        crate::mapping::page(&body, "/data/recents", paging)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistSelection {
    pub uid: Option<String>,
    pub snapshot_id: Option<String>,
    pub uri: String,
    pub position: u32,
}
impl Playlists<'_> {
    /// Resolve positions only against their exact revision. Repeated songs require occurrence identity.
    pub async fn remove_rows(
        &self,
        value: &str,
        selected: Vec<PlaylistSelection>,
    ) -> Result<(), Error> {
        if selected.is_empty() {
            return Ok(());
        }
        let revision = self
            .playlist_info(value)
            .await?
            .snapshot_id
            .filter(|revision| !revision.trim().is_empty())
            .ok_or_else(|| Error::Backend("playlist revision unavailable".into()))?;
        let mut current = Vec::new();
        let mut offset = 0;
        loop {
            let page = self.rows(value, Paging { offset, limit: 100 }).await?;
            current.extend(page.items);
            match page.next_offset {
                Some(next) if next > offset && next <= 100_000 && current.len() <= 100_000 => {
                    offset = next
                }
                Some(_) => return Err(Error::Backend("playlist pagination did not finish".into())),
                None => break,
            }
        }
        if self.playlist_info(value).await?.snapshot_id.as_deref() != Some(revision.as_str()) {
            return Err(Error::InvalidInput(
                "playlist changed while resolving selected rows",
            ));
        }
        let uids = selected
            .iter()
            .map(|selection| {
                let row = match selection.uid.as_deref() {
                    Some(uid) => current.iter().find(|row| row.uid.as_deref() == Some(uid)),
                    None if selection.snapshot_id.as_deref() == Some(revision.as_str()) => {
                        current.get(selection.position as usize)
                    }
                    None => None,
                }
                .filter(|row| {
                    row.item
                        .as_ref()
                        .is_some_and(|item| item.uri == selection.uri)
                });
                row.and_then(|row| row.uid.clone())
                    .filter(|uid| !uid.trim().is_empty())
                    .ok_or(Error::InvalidInput(
                        "playlist occurrence changed; refresh before removing",
                    ))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.remove_tracks(value, &uids).await.map(|_| ())
    }
    pub async fn create_playlist(&self, name: &str, folder: Option<&str>) -> Result<String, Error> {
        let body = self.create(name, folder).await?;
        let uri = body
            .as_str()
            .or_else(|| body.get("uri")?.as_str())
            .filter(|uri| {
                uri.strip_prefix("spotify:playlist:").is_some_and(|id| {
                    !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
                })
            });
        uri.map(str::to_owned).ok_or_else(|| Error::Backend("Playlist was created but its identity was not returned; inspect the library before retrying".into()))
    }
}
impl Library<'_> {
    /// Complete playlist traversal across folders. Does not change Spotify's ordering.
    pub async fn all_playlists(&self) -> Result<Vec<MediaItem>, Error> {
        let mut pending = vec![None];
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();
        while let Some(folder) = pending.pop() {
            let mut offset = 0;
            loop {
                let page = self
                    .items(
                        Some("Playlists"),
                        "Recents",
                        folder.as_deref(),
                        Paging { offset, limit: 50 },
                    )
                    .await?;
                for item in page.items {
                    if !seen.insert(item.uri.clone()) {
                        continue;
                    }
                    if seen.len() > 100_000 {
                        return Err(Error::Backend(
                            "library traversal exceeded safety limit".into(),
                        ));
                    }
                    match item.kind {
                        spotcore_core::MediaKind::Folder => pending.push(Some(item.uri)),
                        spotcore_core::MediaKind::Playlist => result.push(item),
                        _ => {}
                    }
                }
                match page.next_offset {
                    Some(next) if next > offset && next <= 100_000 => offset = next,
                    Some(_) => {
                        return Err(Error::Backend("library pagination did not advance".into()));
                    }
                    None => break,
                }
            }
        }
        Ok(result)
    }
}

impl Extras<'_> {
    pub async fn chapter_markers(&self, value: &str) -> Result<crate::ChapterMarkers, Error> {
        crate::chapters::load(self.0, value).await
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LyricsLine {
    pub text: String,
    pub start_ms: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpotifyLyrics {
    pub lines: Vec<LyricsLine>,
    pub synced: bool,
    pub instrumental: bool,
}
impl SpotifyLyrics {
    pub fn parse(body: &Value) -> Result<Self, Error> {
        let lyrics = body
            .get("lyrics")
            .ok_or_else(|| Error::Backend("Spotify returned no lyrics payload".into()))?;
        let rows = lyrics
            .get("lines")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::Backend("Spotify returned no lyrics lines".into()))?;
        let mut lines: Vec<_> = rows
            .iter()
            .filter_map(|line| {
                let text = line.get("words")?.as_str()?.trim();
                (!text.is_empty()).then(|| LyricsLine {
                    text: text.into(),
                    start_ms: line.get("startTimeMs").and_then(optional_count),
                })
            })
            .collect();
        let instrumental = lyrics["instrumental"] == true
            || lyrics["isInstrumental"] == true
            || (!lines.is_empty()
                && lines.iter().all(|line| {
                    line.text
                        .chars()
                        .all(|ch| ch.is_whitespace() || matches!(ch, '♪' | '♫' | '♬'))
                }));
        let synced = lyrics["syncType"] == "LINE_SYNCED"
            && lines.iter().all(|line| line.start_ms.is_some())
            && lines
                .windows(2)
                .all(|pair| pair[0].start_ms <= pair[1].start_ms);
        if !synced {
            for line in &mut lines {
                line.start_ms = None;
            }
        }
        Ok(Self {
            lines,
            synced,
            instrumental,
        })
    }
}
impl Extras<'_> {
    pub async fn lyrics_info(&self, value: &str) -> Result<Option<SpotifyLyrics>, Error> {
        match self.lyrics(value).await {
            Ok(body) => SpotifyLyrics::parse(&body).map(Some),
            Err(Error::Http { status: 404, .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }
}
impl Podcasts<'_> {
    /// Conservative account policy plus a typed title; does not establish playback entitlement.
    pub async fn audiobook_info(&self, value: &str) -> Result<ShowInfo, Error> {
        self.0
            .users()
            .availability()
            .await?
            .audiobooks
            .require("audiobooks")?;
        if !matches!(value.split(':').nth(1), Some("show" | "audiobook")) {
            return Err(Error::InvalidInput("expected an audiobook URI"));
        }
        let info = super::details::show(self.show(value).await?.raw)?;
        if info.raw["__typename"] != "Audiobook"
            && info.raw["isAudiobook"] != true
            && !value.starts_with("spotify:audiobook:")
        {
            return Err(Error::InvalidInput("expected an audiobook"));
        }
        Ok(info)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogStatistics {
    pub play_counts: std::collections::BTreeMap<String, u64>,
    pub monthly_listeners: Option<u64>,
}
fn read_counts(page: &Value, result: &mut CatalogStatistics) -> Result<usize, Error> {
    let items = page
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Backend("Spotify returned no track statistics".into()))?;
    for row in items {
        let track = row.get("track").unwrap_or(row);
        if let (Some(uri), Some(count)) = (
            track["uri"].as_str(),
            track.get("playcount").and_then(optional_count),
        ) {
            result.play_counts.insert(uri.into(), count);
        }
    }
    Ok(items.len())
}
impl Catalog<'_> {
    pub async fn statistics(&self, value: &str) -> Result<CatalogStatistics, Error> {
        let mut result = CatalogStatistics {
            play_counts: std::collections::BTreeMap::new(),
            monthly_listeners: None,
        };
        match value.split(':').nth(1) {
            Some("album") => {
                let mut offset = 0;
                loop {
                    let body = self.album_page(value, Paging { offset, limit: 50 }).await?;
                    let page = body
                        .pointer("/data/albumUnion/tracksV2")
                        .ok_or_else(|| Error::Backend("Spotify returned no album tracks".into()))?;
                    let count = read_counts(page, &mut result)?;
                    let total = page["totalCount"].as_u64().ok_or_else(|| {
                        Error::Backend("Spotify returned no album track count".into())
                    })?;
                    offset += count as u32;
                    if u64::from(offset) >= total {
                        break;
                    }
                    if count == 0 || offset >= 1000 {
                        return Err(Error::Backend(
                            "Spotify returned an incomplete album track list".into(),
                        ));
                    }
                }
            }
            Some("artist") => {
                let body = self.artist_page(value).await?;
                let artist = body
                    .pointer("/data/artistUnion")
                    .ok_or_else(|| Error::Backend("Spotify returned no artist overview".into()))?;
                read_counts(
                    artist.pointer("/discography/topTracks").ok_or_else(|| {
                        Error::Backend("Spotify returned no artist tracks".into())
                    })?,
                    &mut result,
                )?;
                result.monthly_listeners = artist
                    .pointer("/stats/monthlyListeners")
                    .and_then(optional_count);
            }
            _ => {
                return Err(Error::InvalidInput(
                    "statistics require an artist or album URI",
                ));
            }
        }
        Ok(result)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrereleaseInfo {
    pub uri: String,
    pub album: AlbumInfo,
    pub release_date: Option<String>,
}
fn countdown_node<'a>(value: &'a Value, expected: &str, depth: u8) -> Option<&'a Value> {
    if depth == 0 {
        return None;
    }
    if value.get("uri").and_then(Value::as_str) == Some(expected)
        && (value.get("preReleaseContent").is_some() || value.get("album").is_some())
    {
        return Some(value);
    }
    match value {
        Value::Object(fields) => fields
            .values()
            .find_map(|value| countdown_node(value, expected, depth - 1)),
        Value::Array(values) => values
            .iter()
            .find_map(|value| countdown_node(value, expected, depth - 1)),
        _ => None,
    }
}
impl Extras<'_> {
    pub async fn prerelease_info(&self, value: &str) -> Result<PrereleaseInfo, Error> {
        let body = self.prerelease(value).await?;
        let node = countdown_node(&body, value, 12)
            .ok_or_else(|| Error::Backend("Spotify returned no matching countdown page".into()))?;
        let album = node
            .get("preReleaseContent")
            .or_else(|| node.get("album"))
            .ok_or_else(|| Error::Backend("countdown has no album".into()))?;
        let album_uri = album
            .get("uri")
            .and_then(Value::as_str)
            .filter(|uri| uri.starts_with("spotify:album:"))
            .ok_or_else(|| Error::Backend("countdown has no valid album identity".into()))?;
        Ok(PrereleaseInfo {
            uri: value.into(),
            album: self.0.catalog().album_info(album_uri).await?,
            release_date: node
                .pointer("/releaseDate/isoString")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }
    /// Resolve only Spotify-controlled redirects. No authorization headers are sent.
    pub async fn resolve_share_url(&self, value: &str) -> Result<String, Error> {
        fn allowed(url: &url::Url) -> bool {
            url.scheme() == "https"
                && url.username().is_empty()
                && url.password().is_none()
                && url.port().is_none()
                && matches!(
                    url.host_str(),
                    Some(
                        "spotify.link"
                            | "spotify.app.link"
                            | "blend.spotify.com"
                            | "open.spotify.com"
                            | "play.spotify.com"
                            | "www.spotify.com"
                    )
                )
        }
        let url =
            url::Url::parse(value).map_err(|_| Error::InvalidInput("invalid Spotify share URL"))?;
        if !allowed(&url) {
            return Err(Error::InvalidInput("expected a Spotify HTTPS share URL"));
        }
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 10 || !allowed(attempt.url()) {
                    attempt.error("Spotify share URL left allowed redirect scope")
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(|_| Error::Backend("share link client could not start".into()))?;
        let response = client
            .get(url)
            .header(reqwest::header::USER_AGENT, "Mozilla/5.0 SpotCore")
            .send()
            .await
            .map_err(|_| Error::Backend("Spotify share URL could not be resolved".into()))?;
        if !response.status().is_success() {
            return Err(Error::Http {
                status: response.status().as_u16(),
                retry_after: response
                    .headers()
                    .get("retry-after")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse().ok()),
            });
        }
        Ok(response.url().to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileCard {
    pub uri: String,
    pub name: Option<String>,
    pub image: Option<String>,
}
fn profile_cards(body: &Value) -> Result<Vec<ProfileCard>, Error> {
    let profiles = body
        .get("profiles")
        .or_else(|| body.get("items"))
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Backend("Spotify returned no profile list".into()))?;
    Ok(profiles
        .iter()
        .filter_map(|profile| {
            let uri = profile
                .get("uri")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    profile
                        .get("username")?
                        .as_str()
                        .map(|name| format!("spotify:user:{name}"))
                })?;
            let image = profile_image(profile);
            Some(ProfileCard {
                uri,
                name: profile
                    .get("name")
                    .or_else(|| profile.get("display_name"))
                    .or_else(|| profile.get("displayName"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                image,
            })
        })
        .collect())
}
impl Users<'_> {
    pub async fn followers_info(&self, value: &str) -> Result<Vec<ProfileCard>, Error> {
        profile_cards(&self.followers(value).await?)
    }
    pub async fn following_info(&self, value: &str) -> Result<Vec<ProfileCard>, Error> {
        profile_cards(&self.following(value).await?)
    }
}

#[cfg(test)]
mod typed_tests {
    use super::*;
    #[test]
    fn lyric_timing_is_unknown_when_one_timestamp_is_missing() {
        let lyrics = SpotifyLyrics::parse(&serde_json::json!({"lyrics":{"syncType":"LINE_SYNCED","lines":[{"words":"One","startTimeMs":"100"},{"words":"Two"}]}})).unwrap();
        assert!(!lyrics.synced);
        assert!(lyrics.lines.iter().all(|line| line.start_ms.is_none()));
        assert!(SpotifyLyrics::parse(&Value::Null).is_err());
    }
    #[test]
    fn followers_handle_bare_usernames_and_bad_avatar_fallbacks() {
        let cards = profile_cards(&serde_json::json!({"profiles":[{"username":"alice","name":"Alice","large_image_url":"bad","image_url":"https://example.org/a"}]})).unwrap();
        assert_eq!(cards[0].uri, "spotify:user:alice");
        assert_eq!(cards[0].image.as_deref(), Some("https://example.org/a"));
        assert!(profile_cards(&Value::Null).is_err());
    }
    #[test]
    fn audiobook_details_preserve_dedicated_uri_identity() {
        let info = super::super::details::show(serde_json::json!({"uri":"spotify:audiobook:book","name":"Book","__typename":"Audiobook"})).unwrap();
        assert_eq!(info.uri, "spotify:audiobook:book");
    }
}
