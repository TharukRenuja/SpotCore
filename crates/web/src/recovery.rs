//! Paged metadata recovery shared by Home, queues and collection cards.
use crate::{Api, BrowsePage, HomeFeed, Shelf};
use futures_util::{StreamExt, stream};
use serde::Serialize;
use spotcore_core::{Error, MediaItem, Paging};
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
pub struct MediaResolution {
    pub uri: String,
    pub item: Option<MediaItem>,
    pub error: Option<Error>,
}
#[derive(Debug, Clone, Serialize)]
pub struct RecoveryPage {
    pub items: Vec<MediaResolution>,
    pub next_offset: Option<u32>,
    pub retry_after: Option<u64>,
}

fn recovery_uris(uris: &[String]) -> Result<Vec<String>, Error> {
    if uris.len() > 100_000 {
        return Err(Error::InvalidInput("too many metadata recovery inputs"));
    }
    let mut result = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for uri in uris {
        crate::domains::uri(uri)?;
        if seen.insert(uri) {
            result.push(uri.clone());
        }
    }
    Ok(result)
}

impl Api {
    /// Resolve at most four unique URIs per call. Follow next_offset over the same input.
    /// One failed read does not erase successful neighbors. Does not play or transfer audio.
    pub async fn recover_media(
        &self,
        uris: &[String],
        paging: Paging,
    ) -> Result<RecoveryPage, Error> {
        paging.validate()?;
        if paging.limit > 4 {
            return Err(Error::InvalidInput("metadata recovery batch limit is four"));
        }
        let unique = recovery_uris(uris)?;
        let start = (paging.offset as usize).min(unique.len());
        let end = (start + paging.limit as usize).min(unique.len());
        let next_offset = (end < unique.len()).then_some(end as u32);
        // Serialize recovery batches per account client, so simultaneous views share cached reads.
        let _guard = self.media_reads().lock().await;
        let reads = stream::iter(unique[start..end].iter().cloned().map(|uri| async move {
            if let Some(item) = self.cached_media(&uri) {
                return MediaResolution {
                    uri,
                    item: Some(item),
                    error: None,
                };
            }
            let read = async {
                match uri.split(':').nth(1) {
                    Some("track") => self.catalog().track(&uri).await,
                    Some("episode" | "chapter") => self.podcasts().episode(&uri).await,
                    Some("album") => self.catalog().album(&uri).await,
                    Some("playlist") => self.playlists().playlist(&uri).await,
                    Some("artist") => self.catalog().artist(&uri).await,
                    Some("show" | "audiobook") => self.podcasts().show(&uri).await,
                    _ => Err(Error::Unsupported),
                }
            };
            let result = tokio::time::timeout(Duration::from_secs(8), read)
                .await
                .unwrap_or(Err(Error::Timeout));
            match result {
                Ok(item) => {
                    self.cache_media(uri.clone(), Some(item.clone()));
                    MediaResolution {
                        uri,
                        item: Some(item),
                        error: None,
                    }
                }
                Err(error) => MediaResolution {
                    uri,
                    item: None,
                    error: Some(error),
                },
            }
        }))
        .buffered(4);
        let items: Vec<MediaResolution> = reads.collect().await;
        let retry_after = items
            .iter()
            .filter_map(|resolution| match &resolution.error {
                Some(Error::Http {
                    status: 429,
                    retry_after,
                }) => Some(retry_after.unwrap_or(30)),
                _ => None,
            })
            .max();
        Ok(RecoveryPage {
            items,
            next_offset,
            retry_after,
        })
    }
}

/// Preserve row identity and collection annotations while filling sparse entity metadata.
pub(crate) fn fill(item: &mut MediaItem, full: &MediaItem) {
    // Relinked metadata may use another URI; the requested row keeps its identity.
    if item.name.is_empty() {
        item.name.clone_from(&full.name);
    }
    if item.duration_ms.is_none_or(|duration| duration == 0) {
        item.duration_ms = full.duration_ms;
    }
    if !item.images.iter().any(|image| !image.url.trim().is_empty()) {
        item.images.clone_from(&full.images);
    }
    if !item.raw.is_object() {
        item.raw = serde_json::json!({});
    }
    if let (Some(row), Some(metadata)) = (item.raw.as_object_mut(), full.raw.as_object()) {
        for (key, value) in metadata {
            if !matches!(
                key.as_str(),
                "uri" | "uid" | "addedAt" | "added_at" | "addedBy" | "added_by" | "metadata"
            ) && row.get(key).is_none_or(|existing| {
                existing.is_null() || existing.as_array().is_some_and(Vec::is_empty)
            }) {
                row.insert(key.clone(), value.clone());
            }
        }
    }
}

impl HomeFeed {
    /// Stable recovery paging includes every unique card, so filling a page never shifts later offsets.
    pub async fn recover_metadata(
        &mut self,
        api: &Api,
        paging: Paging,
    ) -> Result<crate::RecoveryPage, Error> {
        seed_complete(api, &self.shelves.items);
        let uris: Vec<_> = self
            .shelves
            .items
            .iter()
            .flat_map(|shelf| &shelf.items.items)
            .map(|item| item.uri.clone())
            .collect();
        let result = api.recover_media(&uris, paging).await?;
        apply_recovery(&mut self.shelves.items, &result);
        Ok(result)
    }
}
impl BrowsePage {
    pub async fn recover_metadata(
        &mut self,
        api: &Api,
        paging: Paging,
    ) -> Result<crate::RecoveryPage, Error> {
        seed_complete(api, &self.shelves.items);
        let uris: Vec<_> = self
            .shelves
            .items
            .iter()
            .flat_map(|shelf| &shelf.items.items)
            .map(|item| item.uri.clone())
            .collect();
        let result = api.recover_media(&uris, paging).await?;
        apply_recovery(&mut self.shelves.items, &result);
        Ok(result)
    }
}
fn apply_recovery(shelves: &mut [Shelf], result: &crate::RecoveryPage) {
    for item in shelves.iter_mut().flat_map(|shelf| &mut shelf.items.items) {
        if let Some(full) = result
            .items
            .iter()
            .find(|resolution| resolution.uri == item.uri)
            .and_then(|resolution| resolution.item.as_ref())
        {
            crate::recovery::fill(item, full);
        }
    }
}

fn seed_complete(api: &Api, shelves: &[Shelf]) {
    for item in shelves.iter().flat_map(|shelf| &shelf.items.items) {
        if !item.name.is_empty()
            && item.images.iter().any(|image| !image.url.trim().is_empty())
            && (!matches!(
                item.kind,
                spotcore_core::MediaKind::Track | spotcore_core::MediaKind::Episode
            ) || item.duration_ms.is_some_and(|duration| duration > 0))
            && api.cached_media(&item.uri).is_none()
        {
            api.cache_media(item.uri.clone(), Some(item.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_cursor_is_stable_across_duplicate_cards() {
        let input: Vec<_> = ["a", "b", "a", "c", "d", "e"]
            .map(|id| format!("spotify:playlist:{id}"))
            .into();
        let unique = recovery_uris(&input).unwrap();
        assert_eq!(unique.len(), 5);
        assert_eq!(unique[4], "spotify:playlist:e");
        assert!(recovery_uris(&["invalid".into()]).is_err());
    }
    #[test]
    fn recovery_preserves_occurrence_and_collection_metadata() {
        let mut row = MediaItem {
            uri: "spotify:track:a".into(),
            name: "".into(),
            kind: spotcore_core::MediaKind::Track,
            images: vec![],
            duration_ms: None,
            uid: Some("occurrence".into()),
            raw: serde_json::json!({"addedBy":"alice","metadata":{"is_queued":true}}),
        };
        let mut full = row.clone();
        full.uid = Some("other".into());
        full.name = "Song".into();
        full.raw = serde_json::json!({"addedBy":"bob","metadata":{},"artists":[{"name":"Artist"}]});
        fill(&mut row, &full);
        assert_eq!(row.uid.as_deref(), Some("occurrence"));
        assert_eq!(row.raw["addedBy"], "alice");
        assert_eq!(row.raw["metadata"]["is_queued"], true);
        assert_eq!(row.name, "Song");
        assert!(row.raw["artists"].is_array());
    }
}
