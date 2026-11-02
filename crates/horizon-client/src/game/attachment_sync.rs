//! Attachment sync, server side (docs/native-mods.md, "Server attachments"):
//! the synced values of the holders a level pass lent to mods, and whole
//! snapshots for the players who started seeing a holder.
use super::{Game, lan::LanHost};
use crate::lan::{
    guest::Guest,
    hub::{AttachmentSnapshot, Hub},
};
use horizon_protocol::attachment_sync::Message;
use horizon_world::attachments::Holder;

/// The guests `holder`'s changes go to: those in the level in place who see it.
fn recipients(hub: &Hub, holder: Holder) -> impl Iterator<Item = &Guest> {
    let owner = match holder {
        Holder::Player(uuid) => hub.guests.iter().find(|g| g.uuid == uuid),
        _ => None,
    };
    hub.guests.iter().filter(move |g| {
        hub.here(g)
            && match holder {
                Holder::Level => true,
                Holder::Chunk([x, z]) => g.sent_chunks.contains(&(x, z)),
                Holder::BlockEntity([x, _, z]) => g.sent_chunks.contains(&(x >> 4, z >> 4)),
                Holder::Entity(id) => g.tracker.is_tracking(id as i32),
                Holder::Player(uuid) => {
                    g.uuid == uuid || owner.is_some_and(|o| g.tracker.is_tracking(o.entity_id))
                }
                _ => false,
            }
    })
}

impl Game<'_> {
    /// The messages that give a player `holder`'s synced attachments whole: its
    /// own, and for a chunk those of its block entities. Holders with none
    /// give no message.
    fn attachment_snapshot(&mut self, holder: Holder) -> Vec<Message> {
        let mut holders = vec![];
        if let Holder::Chunk([x, z]) = holder {
            // Read in place: a chunk lent to a mod is copied and saved.
            let changes = self
                .world
                .chunks
                .get(&(x, z))
                .and_then(|chunk| chunk.saved.as_ref())
                .map(|extras| extras.attachments.synced())
                .unwrap_or_default();
            holders.push((holder, changes));
            holders.extend(
                self.world
                    .block_entities
                    .chunk_attachments(x, z)
                    .into_iter()
                    .map(|((x, y, z), map)| (Holder::BlockEntity([x, y, z]), map.synced())),
            );
        } else {
            let changes = self
                .attachment_map_quiet(holder)
                .map(|map| map.synced())
                .unwrap_or_default();
            holders.push((holder, changes));
        }
        holders
            .into_iter()
            .filter(|(_, changes)| !changes.is_empty())
            .map(|(holder, changes)| Message {
                holder,
                replace: true,
                changes,
            })
            .collect()
    }

    /// The holders players started seeing are sent whole to them, then the
    /// synced changes of the holders lent to mods since the last pass to the
    /// players who see them.
    pub(super) fn flush_attachment_sync(&mut self, host: &mut LanHost) {
        let hub = &mut host.hub;
        for AttachmentSnapshot { player, holder } in std::mem::take(&mut hub.attachment_snapshots)
        {
            let Some(guest) = hub.guests.iter().find(|g| g.uuid == player) else {
                continue;
            };
            // A guest is a player holder, whose entity another player tracks.
            let holder = match holder {
                Holder::Entity(id) => hub
                    .guests
                    .iter()
                    .find(|g| g.entity_id == id as i32)
                    .map_or(holder, |g| Holder::Player(g.uuid)),
                _ => holder,
            };
            for message in self.attachment_snapshot(holder) {
                guest.send_attachment_sync(&message);
            }
        }
        for holder in std::mem::take(&mut self.world.attachment_syncs) {
            let Ok(map) = self.attachment_map_quiet(holder) else {
                continue;
            };
            let changes = map.take_sync_changes();
            if changes.is_empty() {
                continue;
            }
            let message = Message {
                holder,
                replace: false,
                changes,
            };
            for guest in recipients(hub, holder) {
                guest.send_attachment_sync(&message);
            }
        }
    }
}
