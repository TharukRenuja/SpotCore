//! Queue services follow the active Connect receiver or shared Jam, never a local engine.
use crate::{Api, connect, direct, mapping};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use spotcore_core::{Command, Error, Queue, QueueItem};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueScope {
    pub receiver: String,
    pub jam: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct QueuePermissions {
    pub add: bool,
    pub reorder: bool,
    pub remove: bool,
    pub clear: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueueSnapshot {
    #[serde(default)]
    origin: String,
    pub scope: Option<QueueScope>,
    pub permissions: QueuePermissions,
    pub queue: Queue,
}

/// An account's queue. All methods use HTTP services and do not transfer playback.
pub struct Queues<'a>(pub(crate) &'a Api);

fn scope(session: &Value, cluster: &Value) -> Option<QueueScope> {
    if direct::jam_guest(session) && session["listen_on_device"] != true {
        Some(QueueScope {
            receiver: direct::jam_host_device(session)?,
            jam: crate::jam::session_id(session).map(str::to_owned),
        })
    } else {
        let receiver = cluster["active_device_id"]
            .as_str()
            .filter(|s| !s.is_empty())?;
        Some(QueueScope {
            receiver: receiver.to_owned(),
            jam: None,
        })
    }
}

fn same_scope(expected: Option<&QueueScope>, actual: Option<&QueueScope>) -> Result<(), Error> {
    if expected.is_some() && expected == actual {
        Ok(())
    } else {
        Err(Error::Backend(
            "queue owner changed; refresh before editing".into(),
        ))
    }
}

impl Queues<'_> {
    async fn read(&self, fresh: bool) -> Result<QueueSnapshot, Error> {
        let mut session = crate::jam::current(self.0, fresh).await?;
        let cluster = direct::cluster(self.0).await?;
        let target = scope(&session, &cluster);
        if target.as_ref().is_some_and(|scope| scope.jam.is_some()) {
            let latest = crate::jam::current(self.0, true).await?;
            same_scope(target.as_ref(), scope(&latest, &cluster).as_ref())?;
            session = latest;
        }
        let state = &cluster["player_state"];
        let queue = mapping::queue(&connect::queue_view(state)?)?;
        let complete = connect::queue_state_usable(state);
        let guest = target.as_ref().is_some_and(|s| s.jam.is_some());
        let allowed = !guest || (session["active"] != false && session["queue_only_mode"] != true);
        let denied = |action: &str| {
            state["restrictions"]
                .get(format!("disallow_{action}_reasons"))
                .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))
        };
        let editable = target.is_some() && complete && allowed && !denied("setting_queue");
        Ok(QueueSnapshot {
            origin: self.0.device_id().to_owned(),
            scope: target,
            permissions: QueuePermissions {
                add: scope(&session, &cluster).is_some() && !denied("inserting_into_next_tracks"),
                reorder: editable,
                remove: editable,
                clear: editable,
            },
            queue,
        })
    }

    /// Fast authoritative order and identities; artwork hydration is independent.
    pub async fn snapshot(&self) -> Result<QueueSnapshot, Error> {
        self.read(false).await
    }

    pub(crate) async fn write(
        &self,
        expected: &QueueSnapshot,
        method: &str,
        mut args: Value,
    ) -> Result<(), Error> {
        if expected.origin != self.0.device_id() {
            return Err(Error::InvalidInput(
                "queue snapshot belongs to another SDK session",
            ));
        }
        let _write = self.0.queue_writes().lock().await;
        let latest = self.read(true).await?;
        same_scope(expected.scope.as_ref(), latest.scope.as_ref())?;
        let owner = latest.scope.as_ref().ok_or(Error::Disconnected)?;
        let permitted = match method {
            "queue.add" | "queue.insert" => latest.permissions.add,
            "queue.remove" => latest.permissions.remove,
            "queue.clear" => latest.permissions.clear,
            // The Connect command checks skip restrictions independently of editing.
            "queue.skipTo" => true,
            _ => latest.permissions.reorder,
        };
        if !permitted {
            return Err(Error::Unsupported);
        }
        if args["appendManual"] == true {
            args["after"] = json!(latest.queue.queued.last().and_then(identity));
            args.as_object_mut().unwrap().remove("appendManual");
        }
        if method == "queue.remove"
            && let Some(items) = args["items"].as_array_mut()
        {
            retain_present(items, &latest.queue);
            if items.is_empty() {
                return Ok(());
            }
        }
        if let Some(id) = &owner.jam {
            args["sessionId"] = json!(id);
        }
        connect::call_to(self.0, Some(&owner.receiver), method, args).await?;
        Ok(())
    }

    pub async fn add(&self, uris: &[String]) -> Result<(), Error> {
        if uris.is_empty() {
            return Ok(());
        }
        self.execute(Command::AddToQueue(uris.to_vec())).await
    }

    /// Moves existing occurrences, preserving repeated songs and section membership.
    pub async fn move_items(
        &self,
        snapshot: &QueueSnapshot,
        items: &[QueueItem],
        before: Option<QueueItem>,
        after: Option<QueueItem>,
        manual: Option<bool>,
    ) -> Result<(), Error> {
        self.write(
            snapshot,
            "queue.reorder",
            json!({"items":items,"before":before,"after":after,"queued":manual}),
        )
        .await
    }
    pub async fn remove(&self, snapshot: &QueueSnapshot, items: &[QueueItem]) -> Result<(), Error> {
        if items.is_empty() {
            return Ok(());
        }
        self.write(snapshot, "queue.remove", json!({"items":items}))
            .await
    }
    pub async fn clear_upcoming(&self, snapshot: &QueueSnapshot) -> Result<(), Error> {
        let items = snapshot
            .queue
            .queued
            .iter()
            .chain(&snapshot.queue.next_up)
            .map(|item| {
                identity(item).ok_or(Error::InvalidInput("queue row has no occurrence identity"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.remove(snapshot, &items).await
    }
    pub async fn insert(
        &self,
        snapshot: &QueueSnapshot,
        uris: &[String],
        before: Option<QueueItem>,
        after: Option<QueueItem>,
        manual: bool,
    ) -> Result<(), Error> {
        self.write(
            snapshot,
            "queue.insert",
            json!({"uris":uris,"before":before,"after":after,"queued":manual}),
        )
        .await
    }

    pub async fn replace_manual(&self, uris: &[String]) -> Result<(), Error> {
        let snapshot = self.snapshot().await?;
        self.write(&snapshot, "queue.replace", json!({"uris":uris}))
            .await
    }
    /// Executes queue commands through services without activating playback.
    pub async fn execute(&self, command: Command) -> Result<(), Error> {
        validate_command(&command)?;
        let snapshot = self.snapshot().await?;
        self.execute_snapshot(&snapshot, command).await
    }

    pub(crate) async fn execute_on(&self, device: &str, command: Command) -> Result<(), Error> {
        validate_command(&command)?;
        let snapshot = self.snapshot().await?;
        if snapshot.scope.as_ref().map(|scope| scope.receiver.as_str()) != Some(device) {
            return Err(Error::Backend(
                "queue receiver changed; refresh before editing".into(),
            ));
        }
        self.execute_snapshot(&snapshot, command).await
    }

    async fn execute_snapshot(&self, s: &QueueSnapshot, command: Command) -> Result<(), Error> {
        match command {
            Command::AddToQueue(uris) => {
                if uris.is_empty() {
                    return Ok(());
                }
                if uris.len() == 1 {
                    self.write(s, "queue.add", json!({"uris":uris})).await
                } else {
                    self.write(
                        s,
                        "queue.insert",
                        json!({"uris":uris,"appendManual":true,"queued":true}),
                    )
                    .await
                }
            }
            Command::ClearQueue => self.clear_upcoming(s).await,
            Command::RemoveFromQueue(items) => self.remove(s, &items).await,
            Command::ReorderQueue {
                items,
                before,
                after,
            } => self.move_items(s, &items, before, after, None).await,
            Command::InsertIntoQueue { uris, before } => {
                self.write(
                    s,
                    "queue.insert",
                    json!({"uris":uris,"before":before,"queued":true}),
                )
                .await
            }
            Command::SkipToQueue(item) => {
                self.write(s, "queue.skipTo", json!({"uri":item.uri,"uid":item.uid}))
                    .await
            }
            _ => Err(Error::Unsupported),
        }
    }
}

fn validate_command(command: &Command) -> Result<(), Error> {
    let item = |row: &QueueItem| -> Result<(), Error> {
        super::uri(&row.uri)?;
        if row.uid.is_empty() {
            return Err(Error::InvalidInput("queue item has no uid"));
        }
        Ok(())
    };
    match command {
        Command::AddToQueue(uris) | Command::InsertIntoQueue { uris, .. } => {
            if uris.is_empty() || uris.len() > 500 {
                return Err(Error::InvalidInput("expected 1 to 500 queue URIs"));
            }
            for uri in uris {
                super::uri(uri)?;
            }
            if let Command::InsertIntoQueue {
                before: Some(anchor),
                ..
            } = command
            {
                item(anchor)?;
            }
        }
        Command::ReorderQueue {
            items,
            before,
            after,
        } => {
            if items.is_empty()
                || (before.is_none() && after.is_none())
                || (before.is_some() && after.is_some())
            {
                return Err(Error::InvalidInput(
                    "queue move requires items and one anchor",
                ));
            }
            for row in items.iter().chain(before).chain(after) {
                item(row)?;
            }
        }
        Command::RemoveFromQueue(items) => {
            for row in items {
                item(row)?;
            }
        }
        Command::SkipToQueue(row) => item(row)?,
        Command::ClearQueue => {}
        _ => return Err(Error::Unsupported),
    }
    Ok(())
}

pub(crate) fn is_queue_command(command: &Command) -> bool {
    matches!(
        command,
        Command::AddToQueue(_)
            | Command::ClearQueue
            | Command::RemoveFromQueue(_)
            | Command::InsertIntoQueue { .. }
            | Command::ReorderQueue { .. }
            | Command::SkipToQueue(_)
    )
}

fn retain_present(items: &mut Vec<Value>, queue: &Queue) {
    items.retain(|wanted| {
        queue
            .queued
            .iter()
            .chain(&queue.next_up)
            .any(|row| row.uri == wanted["uri"] && row.uid.as_deref() == wanted["uid"].as_str())
    });
}

pub fn identity(item: &spotcore_core::MediaItem) -> Option<QueueItem> {
    Some(QueueItem {
        uri: item.uri.clone(),
        uid: item.uid.as_ref().filter(|s| !s.is_empty())?.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn snapshots_cannot_write_through_another_api_instance() {
        let session = serde_json::from_value(json!({"accessToken":"test-secret","expiresAtMs":u64::MAX,"clientId":"client","appVersion":"version","isAnonymous":false})).unwrap();
        let api = Api::from_session(
            session,
            &spotcore_core::Paths::new(".cache/unused-queue-test"),
        )
        .unwrap();
        let snapshot = QueueSnapshot {
            origin: "another-api".into(),
            scope: None,
            permissions: QueuePermissions::default(),
            queue: Queue {
                context_uri: None,
                current: None,
                queued: vec![],
                next_up: vec![],
            },
        };
        assert!(matches!(
            api.queues()
                .write(&snapshot, "queue.clear", json!({}))
                .await,
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn removal_keeps_exact_requested_occurrences_without_clearing_new_additions() {
        let row = |uid: &str| spotcore_core::MediaItem {
            uri: "spotify:track:same".into(),
            uid: Some(uid.into()),
            name: String::new(),
            kind: spotcore_core::MediaKind::Track,
            images: vec![],
            duration_ms: None,
            raw: Value::Null,
        };
        let queue = Queue {
            queued: vec![row("old"), row("new")],
            context_uri: None,
            current: None,
            next_up: vec![],
        };
        let mut requested = vec![
            json!({"uri":"spotify:track:same","uid":"old"}),
            json!({"uri":"spotify:track:same","uid":"consumed"}),
        ];
        retain_present(&mut requested, &queue);
        assert_eq!(
            requested,
            vec![json!({"uri":"spotify:track:same","uid":"old"})]
        );
    }

    #[test]
    fn guest_scope_does_not_depend_on_a_visible_active_connect_receiver() {
        let session =
            json!({"session_id":"jam","is_session_owner":false,"hostActiveDeviceId":"host"});
        let expected = QueueScope {
            receiver: "host".into(),
            jam: Some("jam".into()),
        };
        assert_eq!(scope(&session, &json!({})), Some(expected.clone()));
        assert!(
            same_scope(
                Some(&expected),
                Some(&QueueScope {
                    receiver: "host".into(),
                    jam: Some("new".into())
                })
            )
            .is_err()
        );
        assert!(same_scope(None, None).is_err());
        assert!(scope(&Value::Null, &json!({})).is_none());
    }
    #[test]
    fn guest_listening_here_uses_the_active_receiver() {
        let session = json!({"session_id":"jam","is_session_owner":false,"hostActiveDeviceId":"host","listen_on_device":true});
        assert_eq!(
            scope(&session, &json!({"active_device_id":"local"})),
            Some(QueueScope {
                receiver: "local".into(),
                jam: None
            })
        );
    }
}
