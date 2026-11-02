//! Mounting for native mods (`native_entity_mount`: NeoForge's `EntityMountEvent`). Each place
//! the acting player or a mob gets on or off a vehicle asks `native_may_board` or
//! `native_may_leave` first, and does nothing when a mod says no.
use super::{Game, native_hooks::mob_kind};

impl Game<'_> {
    /// Whether this game is a server whose mounts the mods hear: the server thread or a
    /// dedicated server, never a client nor the deprecated in-frame path.
    fn native_hears_mounts(&self) -> bool {
        self.serving.is_some() || self.options.dedicated.is_some()
    }

    /// The entity id of the acting player: a guest's while their action runs (kept with the
    /// action, as the hub is often out of `lan` then), else the host's.
    fn native_acting_entity(&self) -> u32 {
        match self.lan_acting {
            Some(conn) => self.lan_acting_entity.unwrap_or_else(|| {
                let guests = self.lan_guest_entities();
                guests.iter().find(|g| g.0 == conn).map_or(0, |g| g.1)
            }),
            None => self.lan.as_ref().map_or(0, |host| host.host_entity as u32),
        }
    }

    /// The type of the vehicle with entity id `id`.
    fn native_vehicle_kind(&self, id: u32) -> String {
        if let Some(b) = self.boats.iter().find(|b| b.id == id) {
            b.entity_type()
        } else if let Some(c) = self.carts.iter().find(|c| c.id == id) {
            c.kind.id().into()
        } else if self.cushions.iter().any(|c| c.id == id) {
            "minecraft:cushion".into()
        } else if let Some(m) = self.mobs.iter().find(|m| m.id == id) {
            mob_kind(m)
        } else {
            String::new()
        }
    }

    /// `native_entity_mount` for `rider` (the acting player when `None`) and `vehicle`: whether
    /// the mounting or dismounting goes ahead.
    fn native_mount_allowed(&mut self, rider: Option<u32>, vehicle: u32, mounting: bool) -> bool {
        if !self.native_hears_mounts() {
            return true;
        }
        let (entity, kind) = match rider {
            Some(id) => (
                id,
                self.mobs
                    .iter()
                    .find(|m| m.id == id)
                    .map_or_else(String::new, mob_kind),
            ),
            None => (self.native_acting_entity(), "minecraft:player".into()),
        };
        let vehicle_kind = self.native_vehicle_kind(vehicle);
        let mut canceled = false;
        self.native_entity_mount(
            self.dim.id(),
            entity,
            &kind,
            vehicle,
            &vehicle_kind,
            mounting,
            &mut canceled,
        );
        !canceled
    }

    /// The acting player is about to board `vehicle`: false when a mod canceled it.
    pub(super) fn native_may_board(&mut self, vehicle: u32) -> bool {
        self.native_mount_allowed(None, vehicle, true)
    }

    /// The acting player is about to get off `vehicle`: false when a mod canceled it.
    pub(super) fn native_may_leave(&mut self, vehicle: u32) -> bool {
        self.native_mount_allowed(None, vehicle, false)
    }

    /// The mob `rider` is about to board (`mounting`) or leave `vehicle`: false when a mod
    /// canceled it.
    pub(super) fn native_mob_may_ride(&mut self, rider: u32, vehicle: u32, mounting: bool) -> bool {
        self.native_mount_allowed(Some(rider), vehicle, mounting)
    }
}
