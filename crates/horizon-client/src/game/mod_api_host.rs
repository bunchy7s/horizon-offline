use super::{BlockStateId, Game};
use horizon_client::mod_api::BlockPos;
use horizon_world::{
    attachments::{ATTACHMENTS_KEY, attachment_compound},
    level::Ticks,
};

fn world_attachments(
    world: &mut horizon_client::world::World,
    holder: horizon_client::mod_api::Holder,
) -> Result<&mut horizon_client::mod_api::Attachments, horizon_client::mod_api::AttachmentError> {
    use horizon_client::mod_api::{AttachmentError, Holder};
    if holder == Holder::Level {
        return Ok(&mut world.attachments);
    }
    if let Holder::BlockEntity([x, y, z]) = holder {
        let pos = (x, y, z);
        let chunk = world
            .chunks
            .get(&(x >> 4, z >> 4))
            .ok_or(AttachmentError::MissingHolder)?;
        attachment_compound(
            chunk
                .saved
                .as_ref()
                .and_then(|e| e.block_entity_sources.iter().find(|(p, _)| *p == pos))
                .and_then(|(_, tag)| tag.get(ATTACHMENTS_KEY)),
        )?;
        let map = world
            .block_entities
            .attachments_mut(pos)
            .ok_or(AttachmentError::MissingHolder)?;
        world.modified.insert((x >> 4, z >> 4));
        return Ok(map);
    }
    let Holder::Chunk([x, z]) = holder else {
        return Err(AttachmentError::MissingHolder);
    };
    let chunk = world
        .chunks
        .get_mut(&(x, z))
        .ok_or(AttachmentError::MissingHolder)?;
    let chunk = std::sync::Arc::make_mut(chunk);
    let extras = std::sync::Arc::make_mut(chunk.saved.get_or_insert_with(Default::default));
    attachment_compound(extras.root.get(&ATTACHMENTS_KEY.into()))?;
    world.modified.insert((x, z));
    Ok(&mut extras.attachments)
}

impl Game<'_> {
    /// A guest's attachments back in their record, for `playerdata` to save.
    pub(super) fn guest_attachments_into(&self, guest: &mut horizon_client::lan::guest::Guest) {
        guest.attachments = self
            .guest_attachments
            .get(&guest.uuid)
            .cloned()
            .unwrap_or_default();
    }

    /// Whether entity `id` is in the level, as a holder of attachments.
    fn attachment_entity_exists(&self, id: u32) -> bool {
        use super::commands::Target;
        match i32::try_from(id).ok().and_then(|id| self.level_entity(id)) {
            Some(Target::Mob(_)) => self.mobs.iter().any(|e| e.id == id && !e.removed),
            Some(
                Target::Item(_)
                | Target::Boat(_)
                | Target::Cart(_)
                | Target::Stand(_)
                | Target::Frame(_)
                | Target::Painting(_)
                | Target::Other(_),
            ) => true,
            _ => self.cushions.iter().any(|e| e.id == id),
        }
    }

    /// The attachments of entities that left the level, dropped: each tick
    /// while any entity has some. Only those entities are looked at, and
    /// nothing else of the entities' memory changes.
    pub(super) fn prune_entity_attachments(&mut self) {
        let gone: Vec<[i32; 4]> = self
            .persist
            .uuids
            .iter()
            .filter(|(id, uuid)| {
                self.entity_attachments.contains_key(*uuid) && !self.attachment_entity_exists(**id)
            })
            .map(|(_, uuid)| *uuid)
            .collect();
        for uuid in gone {
            self.entity_attachments.remove(&uuid);
        }
    }

    pub(super) fn native_authoritative(&self) -> bool {
        self.serving.is_some() || self.options.dedicated.is_some()
    }

    /// Access a server-owned holder's attachment map.
    pub(super) fn attachment_map(
        &mut self,
        holder: horizon_client::mod_api::Holder,
    ) -> Result<&mut horizon_client::mod_api::Attachments, horizon_client::mod_api::AttachmentError>
    {
        use horizon_client::mod_api::AttachmentError;
        if !self.native_authoritative() {
            return Err(AttachmentError::MissingHolder);
        }
        if let horizon_client::mod_api::Holder::Entity(id) = holder {
            if !self.attachment_entity_exists(id) {
                return Err(AttachmentError::MissingHolder);
            }
            attachment_compound(
                self.persist
                    .raw
                    .get(&id)
                    .and_then(|tag| tag.get(ATTACHMENTS_KEY)),
            )?;
            let uuid = self.uuid_for(id);
            return Ok(self.entity_attachments.entry(uuid).or_default());
        }
        if let horizon_client::mod_api::Holder::Player(uuid) = holder {
            if self.lan_acting_uuid == Some(uuid) {
                return Ok(&mut self.player_attachments);
            }
            return self
                .guest_attachments
                .get_mut(&uuid)
                .ok_or(AttachmentError::MissingHolder);
        }
        world_attachments(&mut self.world, holder)
    }
}

#[path = "mod_api_host_generated.rs"]
mod generated;

impl horizon_client::mod_api::LevelHost for super::GameLevel<'_> {
    fn block_state(&self, pos: [i32; 3]) -> BlockStateId {
        horizon_world::level::LevelAccess::block_state(self, BlockPos::new(pos[0], pos[1], pos[2]))
    }
    fn is_client_side(&self) -> bool {
        horizon_world::level::LevelAccess::is_client_side(self)
    }
    fn state_with_property(
        &self,
        state: BlockStateId,
        key: &str,
        value: &str,
    ) -> Option<BlockStateId> {
        self.native_state_with_property(state, key, value)
    }
}

impl horizon_client::mod_api::LevelRandomHost for super::GameLevel<'_> {
    fn attachments(
        &mut self,
        holder: horizon_client::mod_api::Holder,
    ) -> Result<&mut horizon_client::mod_api::Attachments, horizon_client::mod_api::AttachmentError>
    {
        if !self.native_authoritative {
            return Err(horizon_client::mod_api::AttachmentError::MissingHolder);
        }
        world_attachments(self.world, holder)
    }
    fn random(&mut self) -> Option<&mut horizon_client::mod_api::LegacyRandom> {
        self.native_level_random()
    }
}

pub(super) struct LevelTicks<'a, L> {
    pub level: L,
    pub ticks: &'a mut Ticks,
}

impl<'a, L: std::ops::Deref<Target = super::GameLevel<'a>>> horizon_client::mod_api::LevelHost
    for LevelTicks<'_, L>
{
    fn block_state(&self, pos: [i32; 3]) -> BlockStateId {
        horizon_client::mod_api::LevelHost::block_state(&*self.level, pos)
    }
    fn is_client_side(&self) -> bool {
        horizon_client::mod_api::LevelHost::is_client_side(&*self.level)
    }
    fn state_with_property(
        &self,
        state: BlockStateId,
        key: &str,
        value: &str,
    ) -> Option<BlockStateId> {
        horizon_client::mod_api::LevelHost::state_with_property(&*self.level, state, key, value)
    }
}

impl horizon_client::mod_api::LevelRandomHost for LevelTicks<'_, &mut super::GameLevel<'_>> {
    fn attachments(
        &mut self,
        holder: horizon_client::mod_api::Holder,
    ) -> Result<&mut horizon_client::mod_api::Attachments, horizon_client::mod_api::AttachmentError>
    {
        horizon_client::mod_api::LevelRandomHost::attachments(&mut *self.level, holder)
    }
    fn random(&mut self) -> Option<&mut horizon_client::mod_api::LegacyRandom> {
        horizon_client::mod_api::LevelRandomHost::random(&mut *self.level)
    }
}

impl<'a, L: std::ops::Deref<Target = super::GameLevel<'a>>> horizon_client::mod_api::LevelTicksHost
    for LevelTicks<'_, L>
{
    fn schedule_block_tick(&mut self, pos: [i32; 3], block: &str, delay: i32) -> bool {
        self.level
            .native_schedule_block_tick(self.ticks, pos, block, delay)
    }
    fn schedule_fluid_tick(&mut self, pos: [i32; 3], fluid: &str, delay: i32) -> bool {
        self.level
            .native_schedule_fluid_tick(self.ticks, pos, fluid, delay)
    }
}

impl horizon_client::mod_api::LevelWriteHost for LevelTicks<'_, &mut super::GameLevel<'_>> {
    fn set_block_property(&mut self, pos: [i32; 3], key: &str, value: &str, flags: i32) -> bool {
        self.level
            .native_set_block_property(self.ticks, pos, key, value, flags)
    }
}
