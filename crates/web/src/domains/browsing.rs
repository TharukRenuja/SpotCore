use super::*;
use crate::mapping::{item, page_map};
use serde::{Deserialize, Serialize};
use spotcore_core::{Image, MediaItem, Page, Paging};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HomeFilter {
    pub id: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Shelf {
    pub uri: Option<String>,
    pub title: Option<String>,
    pub kind: Option<String>,
    pub items: Page<MediaItem>,
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HomeFeed {
    pub greeting: Option<String>,
    pub filters: Vec<HomeFilter>,
    pub shelves: Page<Shelf>,
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowseCategory {
    pub uri: String,
    pub name: String,
    pub images: Vec<Image>,
    pub background_color: Option<String>,
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowsePage {
    pub uri: String,
    pub title: Option<String>,
    pub shelves: Page<Shelf>,
    pub raw: Value,
}

fn text(value: &Value, paths: &[&str]) -> Option<String> {
    paths
        .iter()
        .find_map(|path| value.pointer(path)?.as_str().map(str::to_owned))
}

fn shelves(
    body: &Value,
    path: &str,
    paging: Paging,
    item_limit: u32,
) -> Result<Page<Shelf>, Error> {
    let mut result = page_map(body, path, paging, |_| None::<Shelf>)?;
    for section in body.pointer(path).unwrap()["items"].as_array().unwrap() {
        if section.get("sectionItems").is_none() {
            continue;
        }
        let offset = section
            .pointer("/sectionItems/pagingInfo/offset")
            .and_then(Value::as_u64)
            .map(u32::try_from)
            .transpose()
            .map_err(|_| Error::Backend("shelf offset overflow".into()))?
            .unwrap_or(0);
        let items = page_map(
            section,
            "/sectionItems",
            Paging {
                offset,
                limit: item_limit,
            },
            |row| item(row.get("content").unwrap_or(row)),
        )?;
        result.items.push(Shelf {
            uri: text(section, &["/uri"]),
            title: text(
                section,
                &[
                    "/data/title/transformedLabel",
                    "/data/title/text",
                    "/title/transformedLabel",
                    "/title/text",
                ],
            ),
            kind: text(section, &["/data/__typename", "/__typename"]),
            items,
            raw: section.clone(),
        });
    }
    Ok(result)
}

fn home_feed(body: Value, item_limit: u32) -> Result<HomeFeed, Error> {
    let home = field(&body, "/data/home")?;
    let filters = home["homeChips"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|chip| {
            Some(HomeFilter {
                id: text(chip, &["/id"])?,
                label: text(
                    chip,
                    &[
                        "/label/transformedLabel",
                        "/label/text",
                        "/title/transformedLabel",
                        "/title/text",
                        "/label",
                    ],
                ),
            })
        })
        .collect();
    Ok(HomeFeed {
        greeting: text(&home, &["/greeting/transformedLabel", "/greeting/text"]),
        filters,
        shelves: shelves(
            &home,
            "/sectionContainer/sections",
            Paging {
                offset: 0,
                limit: 10,
            },
            item_limit,
        )?,
        raw: home,
    })
}

fn primary_categories(body: &Value) -> Result<Vec<BrowseCategory>, Error> {
    let sections = body
        .pointer("/data/browseStart/sections/items")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Backend("missing browse categories".into()))?;
    let mut out = Vec::new();
    for row in sections
        .iter()
        .filter_map(|v| v["sectionItems"]["items"].as_array())
        .flatten()
    {
        let Some(card) = row
            .pointer("/content/data/data/cardRepresentation")
            .or_else(|| row.pointer("/content/data/cardRepresentation"))
        else {
            continue;
        };
        let Some(uri) = text(
            row,
            &["/uri", "/content/data/uri", "/content/data/data/uri"],
        ) else {
            continue;
        };
        let images = card
            .pointer("/artwork/sources")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|image| {
                Some(Image {
                    url: image["url"].as_str()?.into(),
                    width: image["width"].as_u64().and_then(|v| v.try_into().ok()),
                    height: image["height"].as_u64().and_then(|v| v.try_into().ok()),
                })
            })
            .collect();
        out.push(BrowseCategory {
            uri,
            name: text(card, &["/title/transformedLabel", "/title/text"]).unwrap_or_default(),
            images,
            background_color: text(card, &["/backgroundColor/hex"]),
            raw: row.clone(),
        });
    }
    Ok(out)
}

impl Home<'_> {
    pub async fn feed_info(
        &self,
        timezone: &str,
        facet: Option<&str>,
        items_per_shelf: u32,
    ) -> Result<HomeFeed, Error> {
        home_feed(
            self.feed(timezone, facet, items_per_shelf).await?,
            items_per_shelf,
        )
    }
    pub async fn browse_categories(&self) -> Result<Vec<BrowseCategory>, Error> {
        categories(&self.categories().await?)
    }
    pub async fn browse_page(
        &self,
        value: &str,
        sections: Paging,
        items: Paging,
    ) -> Result<BrowsePage, Error> {
        uri(value)?;
        sections.validate()?;
        items.validate()?;
        let body = self.0.gql("browsePage", serde_json::json!({"uri":value,"pagePagination":sections,"sectionPagination":items,
            "browseEndUserIntegration":"INTEGRATION_WEB_PLAYER","includeEpisodeContentRatingsV2":true})).await?;
        let page = field(&body, "/data/browse")?;
        Ok(BrowsePage {
            uri: value.into(),
            title: text(
                &page,
                &["/header/title/transformedLabel", "/header/title/text"],
            ),
            shelves: shelves(&page, "/sections", sections, items.limit)?,
            raw: page,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn home_keeps_shelves_order_and_raw_paging_when_cards_are_unknown() {
        let body = json!({"data":{"home":{"greeting":{"transformedLabel":"Hello"},"homeChips":[{"id":"MUSIC","label":{"text":"Music"}}],
        "sectionContainer":{"sections":{"totalCount":2,"items":[
            {"uri":"spotify:section:a","data":{"title":{"text":"For you"}},"sectionItems":{"totalCount":3,"items":[
                {"content":{"data":{"uri":"spotify:album:a","name":"Album"}}},{"content":{"data":{"__typename":"UnknownCard"}}}
            ]}},
            {"uri":"spotify:section:b","sectionItems":{"totalCount":0,"items":[]}}
        ]}}}}});
        let feed = home_feed(body, 2).unwrap();
        assert_eq!(feed.greeting.as_deref(), Some("Hello"));
        assert_eq!(feed.filters[0].id, "MUSIC");
        assert_eq!(feed.shelves.items.len(), 2);
        assert_eq!(feed.shelves.items[0].items.items.len(), 1);
        assert_eq!(feed.shelves.items[0].items.next_offset, Some(2));
        assert_eq!(
            feed.shelves.items[1].uri.as_deref(),
            Some("spotify:section:b")
        );
        assert!(home_feed(json!({"data":{"home":null}}), 2).is_err());
    }
    #[test]
    fn browse_cards_keep_title_artwork_and_color() {
        let body = json!({"data":{"browseStart":{"sections":{"items":[{"sectionItems":{"items":[
            {"uri":"spotify:genre:jazz","content":{"data":{"cardRepresentation":{"title":{"transformedLabel":"Jazz"},"artwork":{"sources":[{"url":"https://example.com/jazz"}]},"backgroundColor":{"hex":"#123456"}}}}}
        ]}}]}}}});
        let cards = categories(&body).unwrap();
        assert_eq!(cards[0].name, "Jazz");
        assert_eq!(cards[0].images.len(), 1);
        assert_eq!(cards[0].background_color.as_deref(), Some("#123456"));
        assert!(categories(&json!({})).is_err());
    }
}

fn suggestion_texts(body: &Value) -> Result<Vec<String>, Error> {
    let rows = body
        .pointer("/data/searchV2/topResultsV2/itemsV2")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Backend("missing search suggestions".into()))?;
    Ok(rows
        .iter()
        .filter(|row| row["item"]["__typename"] == "SearchAutoCompleteEntity")
        .filter_map(|row| text(&row["item"], &["/text", "/data/text"]))
        .collect())
}

impl Search<'_> {
    pub async fn suggestion_texts(&self, query: &str, limit: u32) -> Result<Vec<String>, Error> {
        suggestion_texts(&self.suggestions(query, limit).await?)
    }
    pub async fn recent_items(&self) -> Result<Vec<MediaItem>, Error> {
        let body = self.recent().await?;
        let rows = body
            .pointer("/data/recentSearches/recentSearchesItems/items")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::Backend("missing recent searches".into()))?;
        Ok(rows.iter().filter_map(item).collect())
    }
}

fn reference_uri(kind: &str, value: &Value) -> Option<String> {
    let id = value.as_str()?;
    let uri = if id.starts_with("spotify:") {
        id.to_owned()
    } else {
        format!("spotify:{kind}:{id}")
    };
    super::details::validate_kind(kind, &uri).ok()?;
    Some(uri)
}

fn recommended_track(value: &Value) -> Option<TrackInfo> {
    let uri = reference_uri(
        "track",
        value
            .get("originalId")
            .filter(|v| v.is_string())
            .unwrap_or(&value["id"]),
    )?;
    let album = &value["album"];
    let album_images = [("largeImageUrl", 640), ("imageUrl", 300)]
        .into_iter()
        .filter_map(|(key, size)| {
            Some(Image {
                url: album[key].as_str()?.into(),
                width: Some(size),
                height: Some(size),
            })
        })
        .collect();
    let artists = value["artists"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|artist| {
            let uri = reference_uri("artist", &artist["id"])?;
            Some(ArtistRef {
                id: uri.rsplit(':').next()?.into(),
                uri,
                name: artist["name"].as_str().unwrap_or_default().into(),
            })
        })
        .collect();
    Some(TrackInfo {
        uri,
        name: value["name"].as_str().unwrap_or_default().into(),
        artists,
        album: reference_uri("album", &album["id"]).map(|uri| MediaRef {
            uri,
            name: album["name"].as_str().unwrap_or_default().into(),
        }),
        album_images,
        track_number: None,
        disc_number: None,
        duration_ms: value["duration"].as_u64(),
        explicit: value["explicit"].as_bool().unwrap_or(false),
        playable: None,
        popularity: None,
        local: false,
        play_count: None,
        external_urls: Default::default(),
        raw: value.clone(),
    })
}

fn recommendation_body(value: &str, limit: u32, skip: &[String]) -> Result<Value, Error> {
    super::details::validate_kind("playlist", value)?;
    Paging { offset: 0, limit }.validate()?;
    let ids = skip
        .iter()
        .map(|uri| {
            super::details::validate_kind("track", uri)?;
            Ok(uri.rsplit(':').next().unwrap_or_default())
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(serde_json::json!({"playlistURI":value,"trackSkipIDs":ids,"numResults":limit}))
}

impl Playlists<'_> {
    /// Suggestions for an existing playlist, distinct from multi-seed recommendations.
    pub async fn recommended(
        &self,
        value: &str,
        limit: u32,
        skip: &[String],
    ) -> Result<Vec<TrackInfo>, Error> {
        let body = recommendation_body(value, limit, skip)?;
        let response = self
            .0
            .request(
                "POST",
                "https://spclient.wg.spotify.com/playlistextender/extendp/",
                Some(body),
            )
            .await?;
        let rows = response["recommendedTracks"]
            .as_array()
            .ok_or_else(|| Error::Backend("missing playlist recommendations".into()))?;
        Ok(rows.iter().filter_map(recommended_track).collect())
    }
}

fn radio_uri(body: &Value) -> Result<Option<String>, Error> {
    let rows = body["mediaItems"]
        .as_array()
        .ok_or_else(|| Error::Backend("missing radio contexts".into()))?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    let value = row["uri"]
        .as_str()
        .ok_or_else(|| Error::Backend("radio context has no URI".into()))?;
    uri(value)?;
    Ok(Some(value.into()))
}

fn radio_is_playlist(value: &str) -> Result<bool, Error> {
    uri(value)?;
    match value.split(':').nth(1) {
        Some("playlist" | "playlist_v2") => Ok(true),
        Some("station") => Ok(false),
        _ => Err(Error::Unsupported),
    }
}

fn context_track_uri(row: &Value) -> Option<String> {
    if let Some(uri) = row["uri"].as_str().filter(|uri| !uri.is_empty()) {
        return (uri.starts_with("spotify:track:") && !uri.ends_with(':')).then(|| uri.into());
    }
    use base64::Engine;
    let bytes: [u8; 16] = base64::engine::general_purpose::STANDARD
        .decode(row["gid"].as_str()?)
        .ok()?
        .try_into()
        .ok()?;
    let mut number = u128::from_be_bytes(bytes);
    let alphabet = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut id = [b'0'; 22];
    for digit in id.iter_mut().rev() {
        *digit = alphabet[(number % 62) as usize];
        number /= 62;
    }
    Some(format!("spotify:track:{}", std::str::from_utf8(&id).ok()?))
}

fn station_tracks(body: &Value, limit: usize) -> Result<Vec<MediaItem>, Error> {
    let pages = body["pages"]
        .as_array()
        .ok_or_else(|| Error::Backend("radio context has no pages".into()))?;
    let mut seen = std::collections::BTreeSet::new();
    let mut tracks = Vec::new();
    for page in pages {
        let rows = page["tracks"]
            .as_array()
            .ok_or_else(|| Error::Backend("radio page has no inline tracks".into()))?;
        for row in rows {
            let Some(uri) = context_track_uri(row).filter(|uri| seen.insert(uri.clone())) else {
                continue;
            };
            let mut normalized = crate::connect::track(row);
            normalized["uri"] = uri.into();
            if let Some(track) = item(&normalized) {
                tracks.push(track);
                if tracks.len() >= limit {
                    return Ok(tracks);
                }
            }
        }
    }
    Ok(tracks)
}

fn station_needs_metadata(track: &MediaItem) -> bool {
    let artists = track
        .raw
        .pointer("/artists/items")
        .or_else(|| track.raw.get("artists"));
    let has_artists = artists.and_then(Value::as_array).is_some_and(|artists| {
        artists.iter().any(|artist| {
            artist
                .get("name")
                .or_else(|| artist.pointer("/profile/name"))
                .and_then(Value::as_str)
                .is_some_and(|name| !name.trim().is_empty())
        })
    });
    track.name.is_empty() || track.images.is_empty() || track.duration_ms.is_none() || !has_artists
}

impl Extras<'_> {
    pub async fn radio_uri(&self, seed: &str) -> Result<Option<String>, Error> {
        radio_uri(&self.radio(seed).await?)
    }

    /// Reads radio tracks from either a generated playlist or a station context.
    pub async fn radio_tracks(&self, seed: &str, limit: u32) -> Result<Vec<MediaItem>, Error> {
        Paging { offset: 0, limit }.validate()?;
        let Some(context) = self.radio_uri(seed).await? else {
            return Ok(Vec::new());
        };
        if radio_is_playlist(&context)? {
            return Ok(self
                .0
                .playlists()
                .tracks(&context, Paging { offset: 0, limit })
                .await?
                .items);
        }
        let body = self
            .0
            .request(
                "GET",
                &format!(
                    "https://spclient.wg.spotify.com/context-resolve/v1/{}",
                    segment(&context)
                ),
                None,
            )
            .await?;
        let mut tracks = station_tracks(&body, limit as usize)?;
        use futures_util::{StreamExt, stream};
        let api = self.0;
        let enrich = stream::iter(
            tracks
                .iter_mut()
                .filter(|track| station_needs_metadata(track)),
        )
        .map(|track| async move {
            if let Ok(full) = api.catalog().track(&track.uri).await {
                *track = full;
            }
        })
        .buffer_unordered(4)
        .collect::<Vec<_>>();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(8), enrich).await;
        Ok(tracks)
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn radio_context_type_determines_its_reader() {
        assert!(radio_is_playlist("spotify:playlist:p").unwrap());
        assert!(radio_is_playlist("spotify:playlist_v2:p").unwrap());
        assert!(!radio_is_playlist("spotify:station:playlist:p").unwrap());
        assert!(radio_is_playlist("spotify:show:s").is_err());
        assert_eq!(
            radio_uri(&json!({"mediaItems":[{"uri":"spotify:station:playlist:p"}]}))
                .unwrap()
                .as_deref(),
            Some("spotify:station:playlist:p")
        );
    }

    #[test]
    fn station_rows_with_missing_artist_names_need_catalog_metadata() {
        let rows = station_tracks(
            &json!({"pages":[{"tracks":[{
                "uri":"spotify:track:a", "metadata":{
                    "title":"Song", "duration":"123000", "image_url":"https://example.com/cover"
                }
            }]}]}),
            50,
        )
        .unwrap();
        let mut track = rows.into_iter().next().unwrap();
        assert!(station_needs_metadata(&track));
        track.raw["artists"] = json!([{"name":"  ", "uri":"spotify:artist:a"}]);
        assert!(station_needs_metadata(&track));
        track.raw["artists"] = json!([{"name":"Artist", "uri":"spotify:artist:a"}]);
        assert!(!station_needs_metadata(&track));
    }

    #[test]
    fn station_pages_preserve_order_and_metadata_without_playlist_queries() {
        let body = json!({"pages":[{"tracks":[
            {"uri":"spotify:track:a","metadata":{"title":"First","artist_name":"Artist","artist_uri":"spotify:artist:one","album_title":"Album","album_uri":"spotify:album:one","duration":"123000","image_url":"https://i.scdn.co/image/a"}},
            {"gid":"AAAAAAAAAAAAAAAAAAAAAQ==","metadata":{"title":"Second"}},
            {"uri":"spotify:track:a"},
            {"uri":"spotify:episode:not-a-song"}
        ]}]});
        let tracks = station_tracks(&body, 50).unwrap();
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].uri, "spotify:track:a");
        assert_eq!(tracks[0].name, "First");
        assert_eq!(tracks[0].duration_ms, Some(123000));
        assert_eq!(tracks[0].raw["artists"][0]["name"], "Artist");
        assert_eq!(tracks[1].uri, "spotify:track:0000000000000000000001");
        assert_eq!(station_tracks(&body, 1).unwrap().len(), 1);
        assert!(station_tracks(&json!({}), 50).is_err());
        assert!(station_tracks(&json!({"pages":[{"page_url":"not-followed"}]}), 50).is_err());
        assert!(context_track_uri(&json!({"gid":"invalid"})).is_none());
    }

    #[test]
    fn suggestions_only_return_autocomplete_text() {
        let body = json!({"data":{"searchV2":{"topResultsV2":{"itemsV2":[
            {"item":{"__typename":"SearchAutoCompleteEntity","text":"Jazz"}},
            {"item":{"__typename":"Track","text":"not a suggestion"}},
            {"item":{"__typename":"SearchAutoCompleteEntity","data":{"text":"Jazz piano"}}}
        ]}}}});
        assert_eq!(suggestion_texts(&body).unwrap(), ["Jazz", "Jazz piano"]);
        assert!(suggestion_texts(&json!({})).is_err());
    }
    #[test]
    fn playlist_suggestions_preserve_track_metadata_and_skip_ids() {
        let body =
            recommendation_body("spotify:playlist:p", 5, &["spotify:track:t".into()]).unwrap();
        assert_eq!(body["trackSkipIDs"], json!(["t"]));
        assert!(recommendation_body("spotify:album:a", 5, &[]).is_err());
        assert!(recommendation_body("spotify:playlist:p", 0, &[]).is_err());
        assert!(
            recommendation_body("spotify:playlist:p", 5, &["spotify:artist:a".into()]).is_err()
        );
        let track=recommended_track(&json!({"originalId":null,"id":"t","name":"Song","duration":5000,"explicit":true,
            "artists":[{"id":"a","name":"Artist"}],"album":{"id":"b","name":"Album","imageUrl":"https://example.com/art"}})).unwrap();
        assert_eq!(track.uri, "spotify:track:t");
        assert_eq!(track.artists[0].uri, "spotify:artist:a");
        assert_eq!(track.album.unwrap().uri, "spotify:album:b");
        assert_eq!(track.album_images.len(), 1);
        assert_eq!(track.duration_ms, Some(5000));
        assert!(track.explicit);
    }
    #[test]
    fn radio_empty_and_malformed_results_are_distinct() {
        assert_eq!(radio_uri(&json!({"mediaItems":[]})).unwrap(), None);
        assert_eq!(
            radio_uri(&json!({"mediaItems":[{"uri":"spotify:playlist:p"}]}))
                .unwrap()
                .as_deref(),
            Some("spotify:playlist:p")
        );
        assert!(radio_uri(&json!({})).is_err());
    }
}

fn categories(body: &Value) -> Result<Vec<BrowseCategory>, Error> {
    match primary_categories(body) {
        Ok(cards) if !cards.is_empty() => Ok(cards),
        primary => {
            let cards = fallback_categories(body);
            if cards.is_empty() { primary } else { Ok(cards) }
        }
    }
}
fn fallback_categories(body: &serde_json::Value) -> Vec<BrowseCategory> {
    fn visit(
        node: &serde_json::Value,
        inherited: Option<&str>,
        depth: usize,
        found: &mut Vec<BrowseCategory>,
    ) {
        if depth > 12 {
            return;
        }
        let uri = node
            .get("uri")
            .and_then(serde_json::Value::as_str)
            .or(inherited);
        if let Some(card) = node
            .get("cardRepresentation")
            .or_else(|| (node["__typename"] == "BrowseCard").then_some(node))
            && let Some(uri) = uri
                .filter(|uri| matches!(uri.split(':').nth(1), Some("genre" | "page" | "section")))
            && let Some(name) = card
                .pointer("/title/transformedLabel")
                .or_else(|| card.pointer("/title/text"))
                .or_else(|| card.get("title"))
                .and_then(serde_json::Value::as_str)
                .filter(|name| !name.trim().is_empty())
            && !found.iter().any(|item| item.uri == uri)
        {
            found.push(BrowseCategory {
                uri: uri.to_owned(),
                name: name.to_owned(),
                images: card
                    .pointer("/artwork/sources")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|image| {
                        Some(Image {
                            url: crate::mapping::artwork_url(image.get("url")?.as_str()?.trim()),
                            width: image["width"]
                                .as_u64()
                                .and_then(|value| value.try_into().ok()),
                            height: image["height"]
                                .as_u64()
                                .and_then(|value| value.try_into().ok()),
                        })
                    })
                    .filter(|image| !image.url.is_empty())
                    .collect(),
                background_color: card
                    .pointer("/backgroundColor/hex")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                raw: node.clone(),
            });
        }
        match node {
            serde_json::Value::Object(fields) => {
                for (key, child) in fields {
                    let inherited =
                        if matches!(key.as_str(), "content" | "data" | "item" | "itemV2") {
                            uri
                        } else {
                            None
                        };
                    visit(child, inherited, depth + 1, found);
                }
            }
            serde_json::Value::Array(items) => {
                for child in items {
                    visit(child, uri, depth + 1, found);
                }
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    if let Some(data) = body.get("data") {
        visit(data, None, 0, &mut found);
    }
    found
}

#[cfg(test)]
mod alternate_browse_tests {
    use super::*;
    #[test]
    fn wrapped_browse_cards_use_the_shared_parser() {
        let body = serde_json::json!({"data":{"browseStart":{"sections":{"items":[{"sectionItems":{"items":[{"uri":"spotify:genre:pop","content":{"data":{"cardRepresentation":{"title":{"transformedLabel":"Pop"},"artwork":{"sources":[{"url":"https://example.org/pop"}]}}}}}]}}]}}}});
        let cards = categories(&body).unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].name, "Pop");
        assert_eq!(cards[0].images[0].url, "https://example.org/pop");
    }
}
