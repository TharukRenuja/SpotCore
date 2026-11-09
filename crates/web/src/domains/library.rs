use super::*;
use crate::mapping::{entity, page};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use spotcore_core::{Image, MediaItem, Page, Paging};

/// One row of the user's library rootlist: a playlist, a folder, or another saved entity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LibraryEntry {
    pub uri: Option<String>,
    pub name: String,
    pub kind: String,
    pub description: Option<String>,
    pub owner: Option<String>,
    pub images: Vec<Image>,
    pub total_length: Option<u64>,
    pub editable: bool,
    pub items: Vec<LibraryEntry>,
    pub raw: Value,
}

impl LibraryEntry {
    fn parse(row: &Value) -> Self {
        Self {
            uri: row["uri"].as_str().map(str::to_owned),
            name: row["name"].as_str().unwrap_or_default().to_owned(),
            kind: row["type"].as_str().unwrap_or("unknown").to_owned(),
            description: row["description"]
                .as_str()
                .filter(|text| !text.is_empty())
                .map(str::to_owned),
            owner: row["owner"]["displayName"]
                .as_str()
                .or_else(|| row["owner"]["name"].as_str())
                .map(str::to_owned),
            images: row["images"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|image| {
                    Some(Image {
                        url: image["url"].as_str()?.to_owned(),
                        width: image["width"].as_u64().map(|value| value as u32),
                        height: image["height"].as_u64().map(|value| value as u32),
                    })
                })
                .collect(),
            total_length: row["totalLength"].as_u64(),
            // Owned, or open to this account: a collaborator can add but is not the owner.
            editable: row["isOwnedBySelf"].as_bool() == Some(true)
                || row["canAdd"].as_bool() == Some(true),
            items: rows(row),
            raw: row.clone(),
        }
    }
}

fn rows(value: &Value) -> Vec<LibraryEntry> {
    value["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(LibraryEntry::parse)
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LibraryItem {
    pub item: MediaItem,
    pub pinned: Option<bool>,
    pub added_at: Option<String>,
    pub owner_uri: Option<String>,
    pub count: Option<u64>,
    pub raw: Value,
}

fn library_item(row: &Value) -> Option<LibraryItem> {
    let mut item = crate::mapping::item(row)?;
    let data = &item.raw;
    let count = data["playlistCount"]
        .as_u64()
        .and_then(|v| v.checked_add(data["folderCount"].as_u64().unwrap_or(0)))
        .or_else(|| data["count"].as_u64());
    let owner_uri = data
        .pointer("/ownerV2/data/uri")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if data["__typename"] == "Folder" {
        item.kind = spotcore_core::MediaKind::Folder;
        if let Some((prefix, suffix)) = item.uri.split_once(":folder:") {
            item.uri = format!(
                "{prefix}:folder:{}",
                suffix.split(':').next().unwrap_or_default()
            );
        }
    }
    Some(LibraryItem {
        item,
        pinned: row["pinned"].as_bool(),
        added_at: row
            .pointer("/addedAt/isoString")
            .and_then(Value::as_str)
            .map(str::to_owned),
        owner_uri,
        count,
        raw: row.clone(),
    })
}

pub struct Library<'a>(pub(crate) &'a Api);
impl Library<'_> {
    pub async fn follow_artist(&self, value: &str) -> Result<Value, Error> {
        if !value.starts_with("spotify:artist:") {
            return Err(Error::InvalidInput("expected a spotify:artist URI"));
        }
        self.add(&[value.to_owned()]).await
    }
    pub async fn unfollow_artist(&self, value: &str) -> Result<Value, Error> {
        if !value.starts_with("spotify:artist:") {
            return Err(Error::InvalidInput("expected a spotify:artist URI"));
        }
        self.remove(&[value.to_owned()]).await
    }
    pub async fn follow_show(&self, value: &str) -> Result<Value, Error> {
        if !value.starts_with("spotify:show:") {
            return Err(Error::InvalidInput("expected a spotify:show URI"));
        }
        self.add(&[value.to_owned()]).await
    }
    pub async fn unfollow_show(&self, value: &str) -> Result<Value, Error> {
        if !value.starts_with("spotify:show:") {
            return Err(Error::InvalidInput("expected a spotify:show URI"));
        }
        self.remove(&[value.to_owned()]).await
    }

    pub async fn contains(&self, values: &[String]) -> Result<Vec<bool>, Error> {
        let mut output = vec![false; values.len()];
        for value in values {
            uri(value)?;
        }
        for playlists in [false, true] {
            let indexed: Vec<_> = values
                .iter()
                .enumerate()
                .filter(|(_, v)| v.contains(":playlist:") == playlists)
                .collect();
            for chunk in indexed.chunks(50) {
                let uris: Vec<_> = chunk.iter().map(|(_, v)| v.as_str()).collect();
                let result = if playlists {
                    service(self.0, "RootlistAPI", "contains", vec![json!(uris)]).await?
                } else if self.0.browser().is_err() {
                    let response = self
                        .0
                        .gql("areEntitiesInLibrary", json!({"uris":uris}))
                        .await?;
                    let lookup = response
                        .pointer("/data/lookup")
                        .and_then(Value::as_array)
                        .filter(|rows| rows.len() == uris.len())
                        .ok_or_else(|| Error::Backend("incomplete library lookup".into()))?;
                    Value::Array(
                        lookup
                            .iter()
                            .map(|row| {
                                row.pointer("/data/saved")
                                    .cloned()
                                    .filter(Value::is_boolean)
                                    .ok_or_else(|| {
                                        Error::Backend("missing library saved flag".into())
                                    })
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                    )
                } else {
                    self.0
                        .call("library.contains", json!({"uris":uris}))
                        .await?
                };
                let flags = result
                    .as_array()
                    .filter(|v| v.len() == chunk.len())
                    .ok_or_else(|| {
                        Error::Backend("library.contains returned an incomplete response".into())
                    })?;
                for ((index, _), flag) in chunk.iter().zip(flags) {
                    output[*index] = flag
                        .as_bool()
                        .ok_or_else(|| Error::Backend("invalid library membership flag".into()))?;
                }
            }
        }
        Ok(output)
    }
    pub async fn add(&self, values: &[String]) -> Result<Value, Error> {
        for v in values {
            uri(v)?;
        }
        self.write_library(values, true).await
    }
    pub async fn remove(&self, values: &[String]) -> Result<Value, Error> {
        for v in values {
            uri(v)?;
        }
        self.write_library(values, false).await
    }
    async fn write_library(&self, values: &[String], add: bool) -> Result<Value, Error> {
        if self.0.browser().is_ok() {
            return self
                .0
                .call(
                    if add { "library.add" } else { "library.remove" },
                    json!({"uris":values}),
                )
                .await;
        }
        if values.iter().any(|uri| {
            !matches!(
                uri.split(':').nth(1),
                Some("track" | "album" | "artist" | "show" | "episode" | "author")
            )
        }) {
            return Err(Error::InvalidInput("unsupported library item type"));
        }
        for chunk in values.chunks(50) {
            self.0
                .gql(
                    if add {
                        "addToLibrary"
                    } else {
                        "removeFromLibrary"
                    },
                    json!({"libraryItemUris":chunk}),
                )
                .await?;
        }
        Ok(Value::Null)
    }
    /// Filter/order use Spotify's own names; None requests the full library.
    pub async fn items(
        &self,
        filter: Option<&str>,
        order: &str,
        folder: Option<&str>,
        paging: Paging,
    ) -> Result<Page<MediaItem>, Error> {
        page(
            &self
                .items_response(filter, order, folder, "", paging)
                .await?,
            "/data/me/libraryV3",
            paging,
        )
    }
    /// Preserves library row metadata and searches within the user's library.
    pub async fn items_detailed(
        &self,
        filter: Option<&str>,
        order: &str,
        folder: Option<&str>,
        text_filter: &str,
        paging: Paging,
    ) -> Result<Page<LibraryItem>, Error> {
        let body = self
            .items_response(filter, order, folder, text_filter, paging)
            .await?;
        crate::mapping::page_map(&body, "/data/me/libraryV3", paging, library_item)
    }
    async fn items_response(
        &self,
        filter: Option<&str>,
        order: &str,
        folder: Option<&str>,
        text_filter: &str,
        paging: Paging,
    ) -> Result<Value, Error> {
        paging.validate()?;
        nonempty(order)?;
        if let Some(folder) = folder {
            uri(folder)?;
        }
        let filters: Vec<_> = filter.into_iter().collect();
        self.0.gql("libraryV3", json!({"filters":filters,"order":order,"textFilter":text_filter,"features":["LIKED_SONGS","YOUR_EPISODES_V2","PRERELEASES","PRERELEASES_V2","CLIPS","EVENTS"],"limit":paging.limit,"offset":paging.offset,"flatten":false,"expandedFolders":[],"folderUri":folder,"includeFoldersWhenFlattening":true})).await
    }
    pub async fn liked_tracks(&self, paging: Paging) -> Result<Page<MediaItem>, Error> {
        paging.validate()?;
        page(
            &self
                .0
                .gql(
                    "fetchLibraryTracks",
                    json!({"offset":paging.offset,"limit":paging.limit}),
                )
                .await?,
            "/data/me/library/tracks",
            paging,
        )
    }
    pub async fn pin(&self, value: &str, pinned: bool) -> Result<Value, Error> {
        uri(value)?;
        self.0
            .gql(
                if pinned {
                    "pinLibraryItem"
                } else {
                    "unpinLibraryItem"
                },
                json!({"pinnableItemUri":value}),
            )
            .await
    }
    pub async fn folders(&self) -> Result<Vec<LibraryEntry>, Error> {
        let raw = service(self.0, "RootlistAPI", "getContents", vec![]).await?;
        Ok(rows(&raw))
    }
    pub async fn create_folder(&self, name: &str) -> Result<Value, Error> {
        nonempty(name)?;
        service(
            self.0,
            "RootlistAPI",
            "createFolder",
            vec![json!(name), json!({"before":"start"})],
        )
        .await
    }
    async fn folder_uri(&self, value: &str) -> Result<String, Error> {
        uri(value)?;
        fn stable(value: &str) -> &str {
            value
                .find(":folder:")
                .map(|index| {
                    let start = index + 8;
                    let end = value[start..].find(':').map_or(value.len(), |n| start + n);
                    &value[..end]
                })
                .unwrap_or(value)
        }
        fn find(entries: &[LibraryEntry], wanted: &str) -> Option<String> {
            for row in entries {
                if row.kind == "folder" {
                    if let Some(uri) = row.uri.as_deref().filter(|u| stable(u) == wanted) {
                        return Some(uri.to_owned());
                    }
                    if let Some(uri) = find(&row.items, wanted) {
                        return Some(uri);
                    }
                }
            }
            None
        }
        find(&self.folders().await?, stable(value))
            .ok_or_else(|| Error::Backend("library folder not found".into()))
    }
    pub async fn rename_folder(&self, value: &str, name: &str) -> Result<Value, Error> {
        nonempty(name)?;
        let full = self.folder_uri(value).await?;
        service(
            self.0,
            "RootlistAPI",
            "renameFolder",
            vec![json!({"uri":full}), json!(name)],
        )
        .await
    }
    pub async fn delete_folder(&self, value: &str) -> Result<Value, Error> {
        let full = self.folder_uri(value).await?;
        service(self.0, "RootlistAPI", "removeFolder", vec![json!(full)]).await
    }
    pub async fn move_to_folder(&self, value: &str, folder: Option<&str>) -> Result<Value, Error> {
        uri(value)?;
        let position = if let Some(folder) = folder {
            json!({"after":{"uri":self.folder_uri(folder).await?}})
        } else {
            json!({"before":"start"})
        };
        if value.contains(":folder:") {
            service(
                self.0,
                "RootlistAPI",
                "moveFolder",
                vec![json!({"uri":self.folder_uri(value).await?}), position],
            )
            .await
        } else {
            service(
                self.0,
                "RootlistAPI",
                "move",
                vec![json!([{"uri":value}]), position],
            )
            .await
        }
    }
}

/// Who added one playlist row, keyed by the row's uid.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlaylistAdder {
    pub uid: String,
    pub uri: String,
    pub username: String,
}

fn playlist_adders(response: &Value, paging: Paging) -> Result<Page<PlaylistAdder>, Error> {
    let content = field(response, "/data/playlistV2/content")?;
    let rows = content["items"]
        .as_array()
        .ok_or_else(|| Error::Backend("playlist contents have no items".into()))?;
    let mapped = page(response, "/data/playlistV2/content", paging)?;
    let items = rows
        .iter()
        .filter_map(|row| {
            let user = row.pointer("/addedBy/data").unwrap_or(&row["addedBy"]);
            Some(PlaylistAdder {
                uid: row["uid"].as_str()?.into(),
                uri: crate::mapping::item(row)?.uri,
                username: user["username"].as_str()?.into(),
            })
        })
        .collect();
    Ok(Page {
        items,
        offset: mapped.offset,
        total: mapped.total,
        next_offset: mapped.next_offset,
    })
}

pub struct Playlists<'a>(pub(crate) &'a Api);
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlaylistAccess {
    Private,
    Public,
    Collaborative,
}
impl PlaylistAccess {
    fn permission(self) -> &'static str {
        match self {
            Self::Private => "BLOCKED",
            Self::Public => "VIEWER",
            Self::Collaborative => "CONTRIBUTOR",
        }
    }
}
impl Playlists<'_> {
    pub async fn playlist(&self, value: &str) -> Result<MediaItem, Error> {
        uri(value)?;
        entity(&self.0.gql("fetchPlaylist",json!({"uri":value,"offset":0,"limit":1,"enableWatchFeedEntrypoint":false,"includeEpisodeContentRatingsV2":true})).await?,"/data/playlistV2")
    }
    pub async fn tracks(&self, value: &str, paging: Paging) -> Result<Page<MediaItem>, Error> {
        uri(value)?;
        paging.validate()?;
        page(&self.0.gql("fetchPlaylistContents",json!({"uri":value,"offset":paging.offset,"limit":paging.limit,"includeEpisodeContentRatingsV2":true})).await?,"/data/playlistV2/content",paging)
    }
    /// Known adders for one page of playlist rows; missing/private adders are omitted.
    pub async fn added_by(
        &self,
        value: &str,
        paging: Paging,
    ) -> Result<Page<PlaylistAdder>, Error> {
        uri(value)?;
        paging.validate()?;
        let response = self.0.gql("fetchPlaylistContents", json!({"uri":value,"offset":paging.offset,"limit":paging.limit,"includeEpisodeContentRatingsV2":true})).await?;
        playlist_adders(&response, paging)
    }
    pub async fn mine(&self) -> Result<Vec<LibraryEntry>, Error> {
        let raw = service(self.0, "RootlistAPI", "getContents", vec![]).await?;
        Ok(rows(&raw))
    }
    pub async fn create(&self, name: &str, folder: Option<&str>) -> Result<Value, Error> {
        nonempty(name)?;
        let position = if let Some(folder) = folder {
            json!({"after":{"uri":self.0.library().folder_uri(folder).await?}})
        } else {
            json!({"before":"start"})
        };
        service(
            self.0,
            "RootlistAPI",
            "createPlaylist",
            vec![json!(name), position],
        )
        .await
    }
    pub async fn rename(&self, value: &str, name: &str) -> Result<Value, Error> {
        nonempty(name)?;
        self.update(value, json!({"name":name})).await
    }
    pub async fn set_description(&self, value: &str, description: &str) -> Result<Value, Error> {
        self.update(value, json!({"description":description})).await
    }
    async fn update(&self, value: &str, fields: Value) -> Result<Value, Error> {
        uri(value)?;
        service(
            self.0,
            "PlaylistAPI",
            "updateDetails",
            vec![json!(value), fields],
        )
        .await
    }
    pub async fn access(&self, value: &str) -> Result<Value, Error> {
        uri(value)?;
        service(
            self.0,
            "PlaylistPermissionsAPI",
            "getBasePermission",
            vec![json!(value)],
        )
        .await
    }
    pub async fn set_access(&self, value: &str, access: PlaylistAccess) -> Result<Value, Error> {
        uri(value)?;
        if access == PlaylistAccess::Private {
            self.set_published(value, false).await?;
        }
        service(
            self.0,
            "PlaylistPermissionsAPI",
            "setBasePermission",
            vec![json!(value), json!(access.permission())],
        )
        .await
    }
    pub async fn published(&self, value: &str) -> Result<Value, Error> {
        uri(value)?;
        service(
            self.0,
            "RootlistAPI",
            "getPublishedState",
            vec![json!(value)],
        )
        .await
    }
    pub async fn set_published(&self, value: &str, published: bool) -> Result<Value, Error> {
        uri(value)?;
        service(
            self.0,
            "RootlistAPI",
            "setPublishedState",
            vec![json!(value), json!(published)],
        )
        .await
    }
    pub async fn set_cover(&self, value: &str, jpeg: Option<&[u8]>) -> Result<Value, Error> {
        uri(value)?;
        if jpeg.is_some_and(|bytes| {
            bytes.len() > 4 * 1024 * 1024 || !bytes.starts_with(&[0xff, 0xd8, 0xff])
        }) {
            return Err(Error::InvalidInput("cover must be JPEG and at most 4 MiB"));
        }
        if self.0.browser().is_err() {
            return crate::direct::cover(self.0, value, jpeg).await;
        }
        let encoded = jpeg.map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes));
        self.0
            .call("playlist.setCover", json!({"uri":value,"jpeg":encoded}))
            .await
    }
    pub async fn follow(&self, value: &str) -> Result<Value, Error> {
        uri(value)?;
        service(
            self.0,
            "RootlistAPI",
            "add",
            vec![json!([value]), json!({"before":"start"})],
        )
        .await
    }
    /// Removes from the library; for owned playlists this is Spotify's delete operation.
    pub async fn unfollow(&self, value: &str) -> Result<Value, Error> {
        uri(value)?;
        service(
            self.0,
            "RootlistAPI",
            "remove",
            vec![json!([{"uri":value}])],
        )
        .await
    }
    pub async fn delete(&self, value: &str) -> Result<Value, Error> {
        self.unfollow(value).await
    }
    pub async fn follow_artist(&self, value: &str) -> Result<Value, Error> {
        Library(self.0).follow_artist(value).await
    }
    pub async fn unfollow_artist(&self, value: &str) -> Result<Value, Error> {
        Library(self.0).unfollow_artist(value).await
    }
    pub async fn follow_show(&self, value: &str) -> Result<Value, Error> {
        Library(self.0).follow_show(value).await
    }
    pub async fn unfollow_show(&self, value: &str) -> Result<Value, Error> {
        Library(self.0).unfollow_show(value).await
    }
    pub async fn add_tracks(&self, value: &str, tracks: &[String]) -> Result<(), Error> {
        uri(value)?;
        for track in tracks {
            uri(track)?;
        }
        for chunk in tracks.chunks(100) {
            self.0.gql("addToPlaylist",json!({"playlistUri":value,"playlistItemUris":chunk,"newPosition":{"moveType":"BOTTOM_OF_PLAYLIST","fromUid":null}})).await?;
        }
        Ok(())
    }
    pub async fn remove_tracks(&self, value: &str, uids: &[String]) -> Result<Value, Error> {
        uri(value)?;
        self.0
            .gql(
                "removeFromPlaylist",
                json!({"playlistUri":value,"uids":uids}),
            )
            .await
    }
    pub async fn move_tracks(
        &self,
        value: &str,
        uids: &[String],
        before: Option<&str>,
    ) -> Result<Value, Error> {
        uri(value)?;
        self.0.gql("moveItemsInPlaylist",json!({"playlistUri":value,"uids":uids,"newPosition":{"moveType":if before.is_some(){"BEFORE_UID"}else{"BOTTOM_OF_PLAYLIST"},"fromUid":before}})).await
    }
    pub async fn members(&self, value: &str) -> Result<Value, Error> {
        uri(value)?;
        service(
            self.0,
            "PlaylistPermissionsAPI",
            "getMembers",
            vec![json!(value)],
        )
        .await
    }
    pub async fn set_member_collaborator(
        &self,
        value: &str,
        username: &str,
        enabled: bool,
    ) -> Result<Value, Error> {
        uri(value)?;
        nonempty(username)?;
        service(
            self.0,
            "PlaylistPermissionsAPI",
            "setMemberPermission",
            vec![
                json!(value),
                json!(username),
                json!(if enabled { "CONTRIBUTOR" } else { "VIEWER" }),
            ],
        )
        .await
    }
    pub async fn remove_member(&self, value: &str, username: &str) -> Result<Value, Error> {
        uri(value)?;
        nonempty(username)?;
        service(
            self.0,
            "PlaylistPermissionsAPI",
            "removeMember",
            vec![json!(value), json!(username)],
        )
        .await
    }
    pub async fn join(&self, value: &str, invitation_token: &str) -> Result<Value, Error> {
        uri(value)?;
        nonempty(invitation_token)?;
        let granted = service(
            self.0,
            "PlaylistPermissionsAPI",
            "claimPermissions",
            vec![json!(value), json!(invitation_token)],
        )
        .await?;
        let exists = self.0.library().contains(&[value.to_owned()]).await?;
        if exists != [true] {
            self.follow(value).await?;
        }
        Ok(granted)
    }
    pub async fn collaborator_invite(&self, value: &str) -> Result<String, Error> {
        uri(value)?;
        let grant = service(
            self.0,
            "PlaylistPermissionsAPI",
            "getPermissionGrant",
            vec![json!(value), json!("CONTRIBUTOR")],
        )
        .await?;
        let token = grant["token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Backend("no collaborator invitation token".into()))?;
        Ok(format!(
            "https://open.spotify.com/playlist/{}?pt={}",
            segment(value.rsplit(':').next().unwrap_or_default()),
            segment(token)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rootlist_folders_keep_their_children() {
        let raw = json!({"items":[
            {"type":"folder","uri":"spotify:folder:abc","name":"Metal","items":[
                {"type":"playlist","uri":"spotify:playlist:1","name":"Doom","totalLength":3,
                 "owner":{"displayName":"me"},"isOwnedBySelf":true}
            ]},
            {"type":"playlist","uri":"spotify:playlist:2","name":"Shared","canAdd":true,
             "images":[{"url":"https://img","width":300}]}
        ]});
        let entries = rows(&raw);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].kind, "folder");
        assert_eq!(entries[0].items[0].name, "Doom");
        assert!(entries[0].items[0].editable);
        assert_eq!(entries[0].items[0].total_length, Some(3));
        assert!(entries[1].editable);
        assert_eq!(entries[1].images[0].width, Some(300));
        assert_eq!(entries[1].owner, None);
    }
}

#[cfg(test)]
mod adder_tests {
    use super::*;
    #[test]
    fn adders_use_raw_row_paging_and_skip_private_users() {
        let response = json!({"data":{"playlistV2":{"content":{"totalCount":3,"items":[
            {"uid":"1","addedBy":{"data":{"username":"user"}},"itemV2":{"data":{"uri":"spotify:track:a","name":"Song"}}},
            {"uid":"2","addedBy":null,"itemV2":{"data":{"uri":"spotify:track:b","name":"Other"}}}
        ]}}}});
        let page = playlist_adders(
            &response,
            Paging {
                offset: 0,
                limit: 2,
            },
        )
        .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].username, "user");
        assert_eq!(page.items[0].uri, "spotify:track:a");
        assert_eq!(page.next_offset, Some(2));
    }
}

/// A track hidden within a specific Spotify playback context.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HiddenSong {
    pub context_uri: String,
    pub track_uri: String,
}

fn hidden_page(
    body: &Value,
    cursor: Option<&str>,
) -> Result<spotcore_core::CursorPage<HiddenSong>, Error> {
    let rows = match body.get("items") {
        Some(Value::Array(rows)) => rows.as_slice(),
        None if body["sync_token"].is_string() => &[],
        _ => return Err(Error::Backend("missing hidden-song collection".into())),
    };
    let mut items = Vec::new();
    for row in rows.iter().filter(|v| v["is_removed"] != true) {
        let context_uri = row["context_uri"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| Error::Backend("hidden song has no context".into()))?;
        let track_uri = row["uri"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| Error::Backend("hidden song has no URI".into()))?;
        items.push(HiddenSong {
            context_uri: context_uri.into(),
            track_uri: track_uri.into(),
        });
    }
    let next_cursor = match body.get("next_page_token") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => (!value.is_empty()).then(|| value.clone()),
        _ => return Err(Error::Backend("invalid hidden-song cursor".into())),
    };
    if next_cursor
        .as_deref()
        .is_some_and(|next| Some(next) == cursor)
    {
        return Err(Error::Backend("hidden-song cursor did not advance".into()));
    }
    Ok(spotcore_core::CursorPage { items, next_cursor })
}

fn hidden_write(
    username: &str,
    playlist: &str,
    track: &str,
    hidden: bool,
    now: u64,
) -> Result<Value, Error> {
    super::details::validate_kind("playlist", playlist)?;
    super::details::validate_kind("track", track)?;
    Ok(
        json!({"username":username,"set":"playcontextbans","items":[{
            "uri":track,"context_uri":playlist,"added_at":if hidden {now} else {0},"is_removed":!hidden
        }]}),
    )
}

impl Library<'_> {
    pub async fn hidden_songs(
        &self,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<spotcore_core::CursorPage<HiddenSong>, Error> {
        Paging { offset: 0, limit }.validate()?;
        if cursor.is_some_and(|v| v.len() > 8192) {
            return Err(Error::InvalidInput("collection cursor is too long"));
        }
        let username = self.0.users().me().await?.username;
        let body = self.0.request("POST", "https://spclient.wg.spotify.com/collection/v2/paging", Some(json!({
            "username":username,"set":"playcontextbans","pagination_token":cursor.unwrap_or_default(),"limit":limit
        }))).await?;
        hidden_page(&body, cursor)
    }
}
impl Playlists<'_> {
    pub async fn set_hidden(&self, playlist: &str, track: &str, hidden: bool) -> Result<(), Error> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| Error::Backend("system clock precedes Unix epoch".into()))?
            .as_secs();
        let mut body = hidden_write("", playlist, track, hidden, now)?;
        body["username"] = json!(self.0.users().me().await?.username);
        self.0
            .request(
                "POST",
                "https://spclient.wg.spotify.com/collection/v2/write",
                Some(body),
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod hidden_tests {
    use super::*;
    #[test]
    fn hidden_pages_preserve_context_and_cursor_without_tombstones() {
        let page = hidden_page(
            &json!({"items":[
            {"uri":"spotify:track:a","context_uri":"spotify:playlist:p"},
            {"uri":"spotify:track:b","is_removed":true}
        ],"next_page_token":"next"}),
            None,
        )
        .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].context_uri, "spotify:playlist:p");
        assert_eq!(page.next_cursor.as_deref(), Some("next"));
        assert!(
            hidden_page(&json!({"sync_token":"sync"}), None)
                .unwrap()
                .items
                .is_empty()
        );
        assert!(
            hidden_page(
                &json!({"sync_token":"sync","next_page_token":"same"}),
                Some("same")
            )
            .is_err()
        );
        assert!(hidden_page(&json!({}), None).is_err());
        assert!(hidden_page(&json!({"items":{},"sync_token":"s"}), None).is_err());
        assert!(hidden_page(&json!({"items":[],"next_page_token":42}), None).is_err());
        assert!(hidden_page(&json!({"items":[{"uri":"spotify:track:a"}]}), None).is_err());
    }
    #[test]
    fn hidden_writes_keep_context_and_validate_entity_kinds() {
        let body =
            hidden_write("user", "spotify:playlist:p", "spotify:track:t", true, 123).unwrap();
        assert_eq!(body["items"][0]["context_uri"], "spotify:playlist:p");
        assert_eq!(body["items"][0]["added_at"], 123);
        let body =
            hidden_write("user", "spotify:playlist:p", "spotify:track:t", false, 123).unwrap();
        assert_eq!(body["items"][0]["added_at"], 0);
        assert_eq!(body["items"][0]["is_removed"], true);
        assert!(hidden_write("user", "spotify:album:p", "spotify:track:t", true, 0).is_err());
    }
}

#[cfg(test)]
mod detailed_library_tests {
    use super::*;
    #[test]
    fn library_rows_keep_pins_dates_owners_and_stable_folders() {
        let row = json!({"pinned":true,"addedAt":{"isoString":"2026-11-01"},"item":{"data":{
            "uri":"spotify:user:u:folder:0123456789abcdef:revision","__typename":"Folder","name":"Folder","playlistCount":2,"folderCount":3,
            "ownerV2":{"data":{"uri":"spotify:user:u"}}
        }}});
        let mapped = library_item(&row).unwrap();
        assert_eq!(mapped.pinned, Some(true));
        assert_eq!(mapped.added_at.as_deref(), Some("2026-11-01"));
        assert_eq!(mapped.count, Some(5));
        assert_eq!(mapped.owner_uri.as_deref(), Some("spotify:user:u"));
        assert_eq!(mapped.item.uri, "spotify:user:u:folder:0123456789abcdef");
        assert_eq!(mapped.raw, row);
        let body = json!({"data":{"me":{"libraryV3":{"items":[row,{"item":{"data":{}}}],"totalCount":3}}}});
        let page = crate::mapping::page_map(
            &body,
            "/data/me/libraryV3",
            Paging {
                offset: 0,
                limit: 2,
            },
            library_item,
        )
        .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.next_offset, Some(2));
    }
}
