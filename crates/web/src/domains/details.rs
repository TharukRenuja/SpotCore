//! Rich metadata mapped from the same GraphQL entities as the catalog methods.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use spotcore_core::{Error, Image};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtistRef {
    pub id: String,
    pub uri: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaRef {
    pub uri: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OwnerRef {
    pub id: Option<String>,
    pub uri: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrackInfo {
    pub uri: String,
    pub name: String,
    pub artists: Vec<ArtistRef>,
    pub album: Option<MediaRef>,
    pub album_images: Vec<Image>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub duration_ms: Option<u64>,
    pub explicit: bool,
    pub playable: Option<bool>,
    pub popularity: Option<u8>,
    pub local: bool,
    pub play_count: Option<u64>,
    pub external_urls: BTreeMap<String, String>,
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlbumInfo {
    pub uri: String,
    pub name: String,
    pub album_type: Option<String>,
    pub total_tracks: Option<u32>,
    pub images: Vec<Image>,
    pub artists: Vec<ArtistRef>,
    pub release_date: Option<String>,
    pub label: Option<String>,
    pub copyrights: Vec<Copyright>,
    pub genres: Vec<String>,
    pub popularity: Option<u8>,
    pub external_urls: BTreeMap<String, String>,
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtistInfo {
    pub uri: String,
    pub name: String,
    pub images: Vec<Image>,
    pub genres: Vec<String>,
    pub followers: Option<u64>,
    pub popularity: Option<u8>,
    pub external_urls: BTreeMap<String, String>,
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShowInfo {
    pub uri: String,
    pub name: String,
    pub publisher: Option<String>,
    pub description: Option<String>,
    pub images: Vec<Image>,
    pub total_episodes: Option<u32>,
    pub external_urls: BTreeMap<String, String>,
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EpisodeInfo {
    pub uri: String,
    pub name: String,
    pub description: Option<String>,
    pub images: Vec<Image>,
    pub duration_ms: Option<u64>,
    pub release_date: Option<String>,
    pub explicit: bool,
    pub resume_position_ms: Option<u64>,
    pub show: Option<MediaRef>,
    pub external_urls: BTreeMap<String, String>,
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlaylistInfo {
    pub uri: String,
    pub name: String,
    pub description: Option<String>,
    pub images: Vec<Image>,
    pub owner: Option<OwnerRef>,
    pub public: Option<bool>,
    pub collaborative: Option<bool>,
    pub snapshot_id: Option<String>,
    pub followers: Option<u64>,
    pub total_items: Option<u64>,
    pub external_urls: BTreeMap<String, String>,
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Copyright {
    pub text: String,
    pub kind: Option<String>,
}

pub(super) fn validate_kind(kind: &str, value: &str) -> Result<(), Error> {
    uri(value)?;
    value
        .strip_prefix(&format!("spotify:{kind}:"))
        .filter(|id| !id.is_empty() && !id.contains(':'))
        .map(|_| ())
        .ok_or(Error::InvalidInput("unexpected Spotify URI kind"))
}

fn text(value: &Value, path: &str) -> Option<String> {
    value.pointer(path)?.as_str().map(str::to_owned)
}
fn number(value: &Value, path: &str) -> Option<u64> {
    let v = value.pointer(path)?;
    v.as_u64().or_else(|| v.as_str()?.parse().ok())
}
fn images(value: &Value) -> Vec<Image> {
    crate::mapping::item(value)
        .map(|v| v.images)
        .unwrap_or_default()
}
fn urls(value: &Value) -> BTreeMap<String, String> {
    text(value, "/sharingInfo/shareUrl")
        .map(|url| [("spotify".into(), url)].into())
        .unwrap_or_default()
}
fn artist(value: &Value) -> Option<ArtistRef> {
    let uri = text(value, "/uri")?;
    Some(ArtistRef {
        id: text(value, "/id").unwrap_or_else(|| uri.rsplit(':').next().unwrap_or_default().into()),
        uri,
        name: text(value, "/profile/name")
            .or_else(|| text(value, "/name"))
            .unwrap_or_default(),
    })
}
fn artists(value: &Value, paths: &[&str]) -> Vec<ArtistRef> {
    paths
        .iter()
        .filter_map(|path| value.pointer(path)?.as_array())
        .flatten()
        .filter_map(artist)
        .collect()
}
fn reference(value: &Value) -> Option<MediaRef> {
    Some(MediaRef {
        uri: text(value, "/uri")?,
        name: text(value, "/name").unwrap_or_default(),
    })
}
fn identity(value: &Value, kind: &str) -> Result<(String, String), Error> {
    let uri = text(value, "/uri").ok_or_else(|| Error::Backend("missing entity URI".into()))?;
    validate_kind(kind, &uri)?;
    let name = text(value, "/name")
        .or_else(|| text(value, "/profile/name"))
        .ok_or_else(|| Error::Backend("missing entity name".into()))?;
    Ok((uri, name))
}

pub(crate) fn track(value: Value) -> Result<TrackInfo, Error> {
    let (uri, name) = identity(&value, "track")?;
    let album = &value["albumOfTrack"];
    Ok(TrackInfo {
        uri,
        name,
        artists: artists(
            &value,
            &[
                "/firstArtist/items",
                "/otherArtists/items",
                "/artists/items",
            ],
        ),
        album: reference(album),
        album_images: images(album),
        track_number: number(&value, "/trackNumber").and_then(|v| v.try_into().ok()),
        disc_number: number(&value, "/discNumber").and_then(|v| v.try_into().ok()),
        duration_ms: number(&value, "/duration/totalMilliseconds"),
        explicit: value["contentRating"]["label"] == "EXPLICIT",
        playable: value["playability"]["playable"].as_bool(),
        popularity: None,
        local: false,
        play_count: number(&value, "/playcount"),
        external_urls: urls(&value),
        raw: value,
    })
}
pub(crate) fn album(value: Value) -> Result<AlbumInfo, Error> {
    let (uri, name) = identity(&value, "album")?;
    Ok(AlbumInfo {
        uri,
        name,
        album_type: text(&value, "/type"),
        total_tracks: number(&value, "/tracksV2/totalCount")
            .or_else(|| number(&value, "/tracks/totalCount"))
            .and_then(|v| v.try_into().ok()),
        images: images(&value),
        artists: artists(&value, &["/artists/items"]),
        release_date: text(&value, "/date/isoString"),
        label: text(&value, "/label"),
        copyrights: value
            .pointer("/copyright/items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|v| {
                Some(Copyright {
                    text: text(v, "/text")?,
                    kind: text(v, "/type"),
                })
            })
            .collect(),
        genres: Vec::new(),
        popularity: None,
        external_urls: urls(&value),
        raw: value,
    })
}
pub(crate) fn artist_info(value: Value) -> Result<ArtistInfo, Error> {
    let (uri, name) = identity(&value, "artist")?;
    Ok(ArtistInfo {
        uri,
        name,
        images: images(&value),
        genres: Vec::new(),
        followers: number(&value, "/stats/followers"),
        popularity: None,
        external_urls: urls(&value),
        raw: value,
    })
}
pub(crate) fn show(value: Value) -> Result<ShowInfo, Error> {
    let (uri, name) = identity(&value, "show")?;
    Ok(ShowInfo {
        uri,
        name,
        publisher: text(&value, "/publisher/name"),
        description: text(&value, "/description"),
        images: images(&value),
        total_episodes: number(&value, "/episodesV2/totalCount").and_then(|v| v.try_into().ok()),
        external_urls: urls(&value),
        raw: value,
    })
}
pub(crate) fn episode(value: Value) -> Result<EpisodeInfo, Error> {
    let (uri, name) = identity(&value, "episode")?;
    Ok(EpisodeInfo {
        uri,
        name,
        description: text(&value, "/description"),
        images: images(&value),
        duration_ms: number(&value, "/duration/totalMilliseconds"),
        release_date: text(&value, "/releaseDate/isoString"),
        explicit: value["contentRating"]["label"] == "EXPLICIT",
        resume_position_ms: number(&value, "/playedState/playPositionMilliseconds"),
        show: reference(&value["podcastV2"]["data"]),
        external_urls: urls(&value),
        raw: value,
    })
}
pub(crate) fn playlist(value: Value) -> Result<PlaylistInfo, Error> {
    let (uri, name) = identity(&value, "playlist")?;
    let owner = value.pointer("/ownerV2/data").filter(|v| v.is_object());
    Ok(PlaylistInfo {
        uri,
        name,
        description: text(&value, "/description"),
        images: images(&value),
        owner: owner.map(|v| OwnerRef {
            id: text(v, "/username"),
            uri: text(v, "/uri"),
            name: text(v, "/name"),
        }),
        public: None,
        collaborative: value["collaborative"].as_bool(),
        snapshot_id: text(&value, "/revisionId"),
        followers: number(&value, "/followers"),
        total_items: number(&value, "/content/totalCount"),
        external_urls: urls(&value),
        raw: value,
    })
}

impl Catalog<'_> {
    pub async fn track_info(&self, value: &str) -> Result<TrackInfo, Error> {
        validate_kind("track", value)?;
        track(self.track(value).await?.raw)
    }
    pub async fn album_info(&self, value: &str) -> Result<AlbumInfo, Error> {
        validate_kind("album", value)?;
        album(self.album(value).await?.raw)
    }
    pub async fn artist_info(&self, value: &str) -> Result<ArtistInfo, Error> {
        validate_kind("artist", value)?;
        artist_info(self.artist(value).await?.raw)
    }
}
impl Podcasts<'_> {
    pub async fn show_info(&self, value: &str) -> Result<ShowInfo, Error> {
        validate_kind("show", value)?;
        show(self.show(value).await?.raw)
    }
    pub async fn episode_info(&self, value: &str) -> Result<EpisodeInfo, Error> {
        validate_kind("episode", value)?;
        episode(self.episode(value).await?.raw)
    }
}
impl Playlists<'_> {
    pub async fn playlist_info(&self, value: &str) -> Result<PlaylistInfo, Error> {
        validate_kind("playlist", value)?;
        playlist(self.playlist(value).await?.raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(kind: &str) -> Value {
        let all: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/details.json")).unwrap();
        all[kind].clone()
    }
    #[test]
    fn typed_entities_map_graphql_fields() {
        let track = track(fixture("track")).unwrap();
        assert_eq!(track.artists.len(), 3);
        assert_eq!(track.artists[0].name, "Miles Davis");
        assert_eq!(track.track_number, Some(3));
        assert_eq!(track.duration_ms, Some(337733));
        assert!(!track.album_images.is_empty());
        assert!(track.album.is_some());
        assert!(track.play_count.is_some());
        assert_eq!(track.popularity, None);
        let album = album(fixture("album")).unwrap();
        assert_eq!(album.total_tracks, Some(21));
        assert_eq!(album.artists.len(), 1);
        assert!(!album.copyrights.is_empty());
        assert!(album.release_date.is_some());
        let artist = artist_info(fixture("artist")).unwrap();
        assert_eq!(artist.name, "Miles Davis");
        assert!(artist.followers.is_some());
        assert!(!artist.images.is_empty());
        let show = show(fixture("show")).unwrap();
        assert_eq!(show.publisher.as_deref(), Some("KSL Newsradio"));
        assert_eq!(show.total_episodes, None);
        let episode = episode(fixture("episode")).unwrap();
        assert_eq!(episode.resume_position_ms, Some(0));
        assert_eq!(episode.show.unwrap().name, "Jazz Notes");
        let playlist = playlist(fixture("playlist")).unwrap();
        assert_eq!(playlist.owner.unwrap().name.as_deref(), Some("Spotify"));
        assert_eq!(playlist.total_items, Some(100));
        assert!(playlist.snapshot_id.is_some());
        assert_eq!(playlist.public, None);
        assert_eq!(playlist.collaborative, None);
    }
    #[test]
    fn malformed_and_wrong_kind_entities_are_rejected() {
        assert!(track(serde_json::json!({})).is_err());
        assert!(track(serde_json::json!({"uri":"spotify:album:a","name":"Album"})).is_err());
        assert!(validate_kind("track", "spotify:album:a").is_err());
        assert!(validate_kind("track", "https://open.spotify.com/track/a").is_err());
    }
}
