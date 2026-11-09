use super::*;
use crate::mapping::{entity, home_section, item, page};
use serde::{Deserialize, Serialize};
use serde_json::json;
use spotcore_core::{MediaItem, MediaKind, Page, Paging};

pub struct Search<'a>(pub(crate) &'a Api);
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum SearchKind {
    Tracks,
    Artists,
    Albums,
    Playlists,
    Shows,
    Episodes,
    Audiobooks,
    Users,
    Genres,
}
impl SearchKind {
    fn route(self) -> (&'static str, &'static str) {
        match self {
            Self::Tracks => ("searchTracks", "/data/searchV2/tracksV2"),
            Self::Artists => ("searchArtists", "/data/searchV2/artists"),
            Self::Albums => ("searchAlbums", "/data/searchV2/albumsV2"),
            Self::Playlists => ("searchPlaylists", "/data/searchV2/playlists"),
            Self::Shows => ("searchPodcasts", "/data/searchV2/podcasts"),
            Self::Episodes => ("searchFullEpisodes", "/data/searchV2/episodes"),
            Self::Audiobooks => ("searchAudiobooks", "/data/searchV2/audiobooks"),
            Self::Users => ("searchUsers", "/data/searchV2/users"),
            Self::Genres => ("searchGenres", "/data/searchV2/genres"),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchResults {
    pub tracks: Page<MediaItem>,
    pub artists: Page<MediaItem>,
    pub albums: Page<MediaItem>,
    pub playlists: Page<MediaItem>,
    pub episodes: Page<MediaItem>,
    pub shows: Page<MediaItem>,
    pub audiobooks: Page<MediaItem>,
    pub users: Page<MediaItem>,
    pub genres: Page<MediaItem>,
}
impl Search<'_> {
    pub async fn query(
        &self,
        kind: SearchKind,
        query: &str,
        paging: Paging,
    ) -> Result<Page<MediaItem>, Error> {
        nonempty(query)?;
        paging.validate()?;
        let (operation, path) = kind.route();
        let variables = json!({"searchTerm":query,"offset":paging.offset,"limit":paging.limit,"numberOfTopResults":paging.limit,
            "includeAudiobooks":true,"includeAuthors":true,"includePreReleases":false,"includeAlbumPreReleases":false,"includeEpisodeContentRatingsV2":true});
        page(&self.0.gql(operation, variables).await?, path, paging)
    }
    pub async fn all(&self, query: &str, limit: u32) -> Result<SearchResults, Error> {
        let paging = Paging { offset: 0, limit };
        use futures_util::{StreamExt, TryStreamExt};
        let kinds = [
            SearchKind::Tracks,
            SearchKind::Artists,
            SearchKind::Albums,
            SearchKind::Playlists,
            SearchKind::Episodes,
            SearchKind::Shows,
            SearchKind::Audiobooks,
            SearchKind::Users,
            SearchKind::Genres,
        ];
        let pages: Vec<_> = futures_util::stream::iter(kinds)
            .map(|kind| self.query(kind, query, paging))
            .buffered(3)
            .try_collect()
            .await?;
        let [
            tracks,
            artists,
            albums,
            playlists,
            episodes,
            shows,
            audiobooks,
            users,
            genres,
        ]: [_; 9] = pages
            .try_into()
            .map_err(|_| Error::Backend("incomplete search results".into()))?;
        Ok(SearchResults {
            tracks,
            artists,
            albums,
            playlists,
            episodes,
            shows,
            audiobooks,
            users,
            genres,
        })
    }
    pub async fn suggestions(&self, query: &str, limit: u32) -> Result<Value, Error> {
        nonempty(query)?;
        Paging { offset: 0, limit }.validate()?;
        self.0.gql("searchSuggestions", json!({"query":query,"limit":limit,"numberOfTopResults":limit,"offset":0,"includeAuthors":true,"includeAlbumPreReleases":false,"includeEpisodeContentRatingsV2":true})).await
    }
    pub async fn recent(&self) -> Result<Value, Error> {
        self.0
            .gql(
                "recentSearches",
                json!({"limit":50,"includeAuthors":true,"includeEpisodeContentRatingsV2":true}),
            )
            .await
    }
    pub async fn remove_recent(&self, uris: &[String]) -> Result<Value, Error> {
        for value in uris {
            uri(value)?;
        }
        self.0
            .gql("removeRecentSearches", json!({"uris":uris}))
            .await
    }
}

pub struct Catalog<'a>(pub(crate) &'a Api);
impl Catalog<'_> {
    pub async fn track(&self, value: &str) -> Result<MediaItem, Error> {
        uri(value)?;
        entity(
            &self
                .0
                .gql(
                    "getTrack",
                    json!({"uri":value,"includeVideoAssociationItems":false}),
                )
                .await?,
            "/data/trackUnion",
        )
    }
    pub async fn album(&self, value: &str) -> Result<MediaItem, Error> {
        uri(value)?;
        entity(
            &self
                .0
                .gql(
                    "getAlbum",
                    json!({"uri":value,"locale":"","offset":0,"limit":1}),
                )
                .await?,
            "/data/albumUnion",
        )
    }
    pub async fn album_tracks(
        &self,
        value: &str,
        paging: Paging,
    ) -> Result<Page<MediaItem>, Error> {
        uri(value)?;
        paging.validate()?;
        page(
            &self
                .0
                .gql(
                    "getAlbum",
                    json!({"uri":value,"locale":"","offset":paging.offset,"limit":paging.limit}),
                )
                .await?,
            "/data/albumUnion/tracksV2",
            paging,
        )
    }
    pub async fn artist(&self, value: &str) -> Result<MediaItem, Error> {
        entity(&self.artist_page(value).await?, "/data/artistUnion")
    }
    pub async fn artist_page(&self, value: &str) -> Result<Value, Error> {
        uri(value)?;
        self.0
            .gql(
                "queryArtistOverview",
                json!({"uri":value,"locale":"","preReleaseV2":false}),
            )
            .await
    }
    pub async fn artist_top_tracks(&self, value: &str) -> Result<Vec<MediaItem>, Error> {
        let result = self.artist_page(value).await?;
        Ok(result
            .pointer("/data/artistUnion/discography/topTracks/items")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::Backend("missing artist top tracks".into()))?
            .iter()
            .filter_map(item)
            .collect())
    }
    pub async fn discography(&self, value: &str, paging: Paging) -> Result<Page<MediaItem>, Error> {
        uri(value)?;
        paging.validate()?;
        page(&self.0.gql("queryArtistDiscographyAll", json!({"uri":value,"offset":paging.offset,"limit":paging.limit,"order":"DATE_DESC"})).await?, "/data/artistUnion/discography/all", paging)
    }
    pub async fn related_artists(&self, value: &str) -> Result<Value, Error> {
        uri(value)?;
        self.0.gql("queryArtistRelated", json!({"uri":value})).await
    }
    pub async fn appears_on(&self, value: &str) -> Result<Value, Error> {
        uri(value)?;
        self.0
            .gql("queryArtistAppearsOn", json!({"uri":value}))
            .await
    }
    pub async fn similar_albums(&self, value: &str, limit: u32) -> Result<Value, Error> {
        uri(value)?;
        Paging { offset: 0, limit }.validate()?;
        self.0
            .gql(
                "similarAlbumsBasedOnThisTrack",
                json!({"uri":value,"limit":limit,"albumsOnly":true}),
            )
            .await
    }
    /// Client-side blend of separate Spotify radios, not joint multi-seed ranking.
    /// Reads only; excludes seed tracks and duplicates. Errors are not hidden.
    pub async fn radio_blend(
        &self,
        seed_tracks: &[String],
        seed_artists: &[String],
        limit: u32,
    ) -> Result<Vec<MediaItem>, Error> {
        let seeds = radio_seeds(seed_tracks, seed_artists, limit)?;
        let mut radios = Vec::new();
        for seed in seeds {
            if let Some(uri) = self.0.extras().radio_uri(&seed).await? {
                radios.push(
                    self.0
                        .playlists()
                        .tracks(&uri, Paging { offset: 0, limit })
                        .await?
                        .items,
                );
            }
        }
        Ok(blend_radios(radios, seed_tracks, limit as usize))
    }

    /// Recommendations seeded by up to five track or artist URIs.
    pub async fn recommendations(
        &self,
        seed_tracks: &[String],
        seed_artists: &[String],
        limit: u32,
    ) -> Result<Vec<MediaItem>, Error> {
        let url = recommendations_url(seed_tracks, seed_artists, limit)?;
        let body = self.0.request("GET", &url, None).await?;
        Ok(body["tracks"]
            .as_array()
            .ok_or_else(|| Error::Backend("missing recommendations".into()))?
            .iter()
            .filter_map(item)
            .collect())
    }
}

fn radio_seeds(tracks: &[String], artists: &[String], limit: u32) -> Result<Vec<String>, Error> {
    Paging { offset: 0, limit }.validate()?;
    let count = tracks.len().saturating_add(artists.len());
    if count == 0 || count > 5 {
        return Err(Error::InvalidInput("radio blend takes one to five seeds"));
    }
    let mut seen = std::collections::HashSet::new();
    let mut seeds = Vec::new();
    for (kind, values) in [("track", tracks), ("artist", artists)] {
        for value in values {
            super::details::validate_kind(kind, value)?;
            if seen.insert(value) {
                seeds.push(value.clone());
            }
        }
    }
    Ok(seeds)
}

fn blend_radios(radios: Vec<Vec<MediaItem>>, seeds: &[String], limit: usize) -> Vec<MediaItem> {
    let mut radios: Vec<_> = radios.into_iter().map(Vec::into_iter).collect();
    let mut seen: std::collections::HashSet<String> = seeds.iter().cloned().collect();
    let mut out = Vec::new();
    while out.len() < limit {
        let before = out.len();
        for radio in &mut radios {
            if let Some(track) =
                radio.find(|item| item.kind == MediaKind::Track && seen.insert(item.uri.clone()))
            {
                out.push(track);
                if out.len() == limit {
                    return out;
                }
            }
        }
        if out.len() == before {
            break;
        }
    }
    out
}

fn seed_ids(values: &[String], kind: &str) -> Result<Vec<String>, Error> {
    values
        .iter()
        .map(|value| {
            uri(value)?;
            let id = value.rsplit(':').next().unwrap_or_default();
            if value.split(':').nth(1) != Some(kind) || id.is_empty() {
                return Err(Error::InvalidInput("unexpected recommendation seed kind"));
            }
            Ok(id.to_owned())
        })
        .collect()
}

fn recommendations_url(
    seed_tracks: &[String],
    seed_artists: &[String],
    limit: u32,
) -> Result<String, Error> {
    Paging { offset: 0, limit }.validate()?;
    let tracks = seed_ids(seed_tracks, "track")?;
    let artists = seed_ids(seed_artists, "artist")?;
    if tracks.is_empty() && artists.is_empty() {
        return Err(Error::InvalidInput("recommendations need a seed"));
    }
    if tracks.len() + artists.len() > 5 {
        return Err(Error::InvalidInput(
            "recommendations take at most five seeds",
        ));
    }
    let mut url = format!("https://api.spotify.com/v1/recommendations?limit={limit}");
    if !tracks.is_empty() {
        url.push_str("&seed_tracks=");
        url.push_str(&tracks.join(","));
    }
    if !artists.is_empty() {
        url.push_str("&seed_artists=");
        url.push_str(&artists.join(","));
    }
    Ok(url)
}

pub struct Home<'a>(pub(crate) &'a Api);
impl Home<'_> {
    pub async fn feed(
        &self,
        timezone: &str,
        facet: Option<&str>,
        items_per_shelf: u32,
    ) -> Result<Value, Error> {
        nonempty(timezone)?;
        Paging {
            offset: 0,
            limit: items_per_shelf,
        }
        .validate()?;
        self.0.gql("home", json!({"timeZone":timezone,"sp_t":"","facet":facet,"sectionItemsLimit":items_per_shelf,"homeEndUserIntegration":"INTEGRATION_WEB_PLAYER","includeEpisodeContentRatingsV2":true,"includeHomeChapterVideoCards":false})).await
    }
    pub async fn categories(&self) -> Result<Value, Error> {
        self.0.gql("browseAll", json!({"pagePagination":{"offset":0,"limit":10},"sectionPagination":{"offset":0,"limit":99},"browseEndUserIntegration":"INTEGRATION_WEB_PLAYER"})).await
    }
    pub async fn category(&self, value: &str) -> Result<Value, Error> {
        uri(value)?;
        self.0.gql("browsePage", json!({"uri":value,"pagePagination":{"offset":0,"limit":10},"sectionPagination":{"offset":0,"limit":20},"browseEndUserIntegration":"INTEGRATION_WEB_PLAYER","includeEpisodeContentRatingsV2":true})).await
    }
    pub async fn whats_new(&self, paging: Paging) -> Result<Page<MediaItem>, Error> {
        paging.validate()?;
        page(
            &self
                .0
                .gql(
                    "queryWhatsNewFeed",
                    json!({"offset":paging.offset,"limit":paging.limit,"onlyUnPlayedItems":false}),
                )
                .await?,
            "/data/whatsNewFeedItems",
            paging,
        )
    }
    /// New releases from followed artists, albums only. `whats_new` keeps
    /// every feed item, including episodes and event notices.
    pub async fn new_releases(&self, paging: Paging) -> Result<Page<MediaItem>, Error> {
        paging.validate()?;
        let mut releases = page(
            &self
                .0
                .gql(
                    "queryWhatsNewFeed",
                    json!({"offset":paging.offset,"limit":paging.limit,"onlyUnPlayedItems":false,"includedContentTypes":["ALBUM"],"includeEpisodeContentRatingsV2":false}),
                )
                .await?,
            "/data/whatsNewFeedItems",
            paging,
        )?;
        releases.items.retain(|item| item.kind == MediaKind::Album);
        Ok(releases)
    }
    /// One Home shelf page, merging every section Spotify returns.
    pub async fn section(
        &self,
        value: &str,
        timezone: &str,
        paging: Paging,
    ) -> Result<Page<MediaItem>, Error> {
        uri(value)?;
        nonempty(timezone)?;
        paging.validate()?;
        let body = self
            .0
            .gql(
                "homeSection",
                json!({"uri":value,"timeZone":timezone,"sp_t":"","sectionItemsLimit":paging.limit,"sectionItemsOffset":paging.offset,"homeEndUserIntegration":"INTEGRATION_WEB_PLAYER","includeEpisodeContentRatingsV2":true}),
            )
            .await?;
        home_section(&body, paging)
    }
    /// Made For You. The section URI is Spotify's; it is not account-specific.
    pub async fn made_for_you(
        &self,
        timezone: &str,
        paging: Paging,
    ) -> Result<Page<MediaItem>, Error> {
        self.section(MADE_FOR_YOU, timezone, paging).await
    }
    /// Your top mixes, the sibling shelf of Made For You.
    pub async fn top_mixes(
        &self,
        timezone: &str,
        paging: Paging,
    ) -> Result<Page<MediaItem>, Error> {
        self.section(TOP_MIXES, timezone, paging).await
    }
}

// Spotify's own Home section URIs. They can change when Spotify reorganizes Home.
const MADE_FOR_YOU: &str = "spotify:section:0JQ5DAUnp4wcj0bCb3wh3S";
const TOP_MIXES: &str = "spotify:section:0JQ5DAnM3wGh0gz1MXnu89";

pub struct Podcasts<'a>(pub(crate) &'a Api);
impl Podcasts<'_> {
    pub async fn show(&self, value: &str) -> Result<MediaItem, Error> {
        entity(&self.show_details(value).await?, "/data/podcastUnionV2")
    }
    pub async fn show_details(&self, value: &str) -> Result<Value, Error> {
        uri(value)?;
        self.0.gql("queryShowMetadataV2", json!({"uri":value,"includeContentCapabilityTrait":true,"includeEpisodeContentRatingsV2":true})).await
    }
    pub async fn episode(&self, value: &str) -> Result<MediaItem, Error> {
        uri(value)?;
        entity(
            &self
                .0
                .gql(
                    "getEpisodeOrChapter",
                    json!({"uri":value,"includeEpisodeContentRatingsV2":true}),
                )
                .await?,
            "/data/episodeUnionV2",
        )
    }
    pub async fn episodes(&self, value: &str, paging: Paging) -> Result<Page<MediaItem>, Error> {
        uri(value)?;
        paging.validate()?;
        page(&self.0.gql("queryPodcastEpisodes", json!({"uri":value,"offset":paging.offset,"limit":paging.limit,"includeEpisodeContentRatingsV2":true})).await?, "/data/podcastUnionV2/episodesV2", paging)
    }
    pub async fn audiobook(&self, value: &str) -> Result<Value, Error> {
        self.show_details(value).await
    }
    pub async fn chapters(&self, value: &str, paging: Paging) -> Result<Page<MediaItem>, Error> {
        uri(value)?;
        paging.validate()?;
        let result = self
            .0
            .gql(
                "queryBookChapters",
                json!({"uri":value,"offset":paging.offset,"limit":paging.limit}),
            )
            .await?;
        let pointer = if result.pointer("/data/podcastUnionV2/chaptersV2").is_some() {
            "/data/podcastUnionV2/chaptersV2"
        } else {
            "/data/podcastUnionV2/episodesV2"
        };
        page(&result, pointer, paging)
    }
    pub async fn saved_episodes(&self) -> Result<Value, Error> {
        service(
            self.0,
            "CollectionPlatformAPI",
            "get",
            vec![json!("listenlater")],
        )
        .await
    }
    pub async fn set_saved(&self, value: &str, saved: bool) -> Result<Value, Error> {
        uri(value)?;
        service(
            self.0,
            "CollectionPlatformAPI",
            if saved { "add" } else { "remove" },
            vec![json!("listenlater"), json!([value])],
        )
        .await
    }
}

pub struct Recents<'a>(pub(crate) &'a Api);
impl Recents<'_> {
    /// Returns original grouped rows, retaining Myspoti's context/group attributes.
    pub async fn entries(&self, paging: Paging) -> Result<Value, Error> {
        paging.validate()?;
        self.0.gql("recents", json!({"uris":["spotify:list:recents:page"],"offset":paging.offset,"limit":paging.limit})).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radio_blend_interleaves_unique_tracks_and_excludes_seeds() {
        let track =
            |id: &str| item(&json!({"uri":format!("spotify:track:{id}"),"name":id})).unwrap();
        let radios = vec![
            vec![track("seed"), track("a"), track("shared"), track("b")],
            vec![track("shared"), track("c"), track("d")],
        ];
        let blended = blend_radios(radios, &["spotify:track:seed".into()], 4);
        assert_eq!(
            blended.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["a", "shared", "b", "c"]
        );
        assert!(
            blend_radios(
                vec![vec![track("seed")]],
                &["spotify:track:seed".into()],
                20
            )
            .is_empty()
        );
    }
    #[test]
    fn radio_blend_validates_and_deduplicates_seed_requests() {
        let tracks = vec!["spotify:track:a".into(), "spotify:track:a".into()];
        assert_eq!(
            radio_seeds(&tracks, &["spotify:artist:b".into()], 10).unwrap(),
            ["spotify:track:a", "spotify:artist:b"]
        );
        assert!(radio_seeds(&[], &[], 10).is_err());
        assert!(radio_seeds(&tracks, &[], 0).is_err());
        assert!(radio_seeds(&vec!["spotify:track:a".into(); 6], &[], 10).is_err());
        assert!(radio_seeds(&["spotify:album:a".into()], &[], 10).is_err());
        assert!(radio_seeds(&[], &["spotify:artist:a:extra".into()], 10).is_err());
    }

    #[test]
    fn recommendation_url_uses_typed_seed_ids() {
        let url = recommendations_url(
            &["spotify:track:abc".into()],
            &["spotify:artist:def".into()],
            10,
        )
        .unwrap();
        assert_eq!(
            url,
            "https://api.spotify.com/v1/recommendations?limit=10&seed_tracks=abc&seed_artists=def"
        );
    }

    #[test]
    fn recommendation_seeds_are_validated() {
        assert!(recommendations_url(&[], &[], 20).is_err());
        assert!(recommendations_url(&vec!["spotify:track:abc".into(); 6], &[], 20).is_err());
        assert!(recommendations_url(&["spotify:album:abc".into()], &[], 20).is_err());
        assert!(recommendations_url(&["spotify:track:abc".into()], &[], 0).is_err());
    }
}
