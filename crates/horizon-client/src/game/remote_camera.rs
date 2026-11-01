//! A server's spectator camera (`ClientPacketListener.handleSetCamera`,
//! `Minecraft.cameraEntity`): `/spectate` or a spectator's click (sent as
//! `MultiPlayerGameMode.spectate`) attaches the view to another entity. The
//! view follows that entity while the player stands still and sends no moves
//! (`LocalPlayer.isControlledCamera`); the server keeps its player on the
//! camera and lets go when the player sneaks.
use super::Game;
use horizon_client::{
    camera::{CameraEntity, LivingCamera},
    mobs::Mob,
};

/// `Minecraft.setCameraEntity` from a ClientboundSetCameraPacket: the player's
/// own id puts the view back on the player; an id the level does not hold
/// changes nothing (`ClientboundSetCameraPacket.getEntity` is null).
pub(super) fn camera_after_packet(
    current: Option<i32>,
    player_id: i32,
    camera_id: i32,
    in_level: bool,
) -> Option<i32> {
    if camera_id == player_id {
        None
    } else if in_level {
        Some(camera_id)
    } else {
        current
    }
}

/// `Camera.alignWithEntity`'s view of a mob or another player: its
/// interpolated position, `getViewYRot`/`getViewXRot` and eye height, with
/// its scale and camera distance for a detached view. A living entity's
/// `getViewYRot` is its head's yaw (`yHeadRot`), not its body's `yRot`.
pub(super) fn mob_camera_entity(m: &Mob) -> CameraEntity {
    let attribute =
        |name: &str, default: f64| m.kind.attributes.get(name).copied().unwrap_or(default);
    CameraEntity {
        position: [m.position.x, m.position.y, m.position.z],
        previous: [m.previous.x, m.previous.y, m.previous.z],
        x_rot: m.head_pitch,
        y_rot: m.head_yaw,
        x_rot_o: m.previous_head_pitch,
        y_rot_o: m.previous_head_yaw,
        eye_height: m.eye_height() as f32,
        living: Some(LivingCamera {
            scale: attribute("minecraft:scale", 1.0) as f32,
            camera_distance: attribute("minecraft:camera_distance", 4.0),
            sleeping: None,
            dead: None,
        }),
        mount: None,
    }
}

/// `GameRenderer.checkEntityPostEffect`: the chain a spectated creeper, spider
/// or enderman puts on the screen.
pub(super) fn entity_post_effect(kind: &str) -> Option<&'static str> {
    match kind.strip_prefix("minecraft:").unwrap_or(kind) {
        "creeper" => Some("minecraft:creeper"),
        // CaveSpider extends Spider.
        "spider" | "cave_spider" => Some("minecraft:spider"),
        "enderman" => Some("minecraft:invert"),
        _ => None,
    }
}

impl Game<'_> {
    /// `ClientPacketListener.handleSetCamera`.
    pub(super) fn remote_set_camera(&mut self, camera_id: i32) {
        let in_level = self.mobs.iter().any(|m| m.id == camera_id as u32);
        let player = self
            .remote
            .as_ref()
            .is_some_and(|r| r.entity_id == camera_id);
        if let Some(r) = self.remote.as_mut() {
            r.camera = camera_after_packet(r.camera, r.entity_id, camera_id, in_level);
        }
        // Minecraft.setCameraEntity, for an entity the level holds.
        if in_level || player {
            self.check_entity_post_effect(true);
        }
    }
    /// `GameRenderer.checkEntityPostEffect` for the camera entity, or for none
    /// (`first_person` false: the third-person view clears it).
    pub(super) fn check_entity_post_effect(&mut self, first_person: bool) {
        self.spectated_post_effect = self
            .camera_mob()
            .filter(|_| first_person)
            .and_then(|m| entity_post_effect(&m.kind.id));
    }
    /// The entity a server attached the view to, when it is not the player.
    pub(super) fn remote_camera_id(&self) -> Option<u32> {
        self.remote.as_ref()?.camera.map(|id| id as u32)
    }
    /// `LocalPlayer.isControlledCamera`: the view is the player's own.
    pub(super) fn controls_camera(&self) -> bool {
        self.remote_camera_id().is_none()
    }
    /// The mob the view follows, while the level still holds it.
    pub(super) fn camera_mob(&self) -> Option<&Mob> {
        let id = self.remote_camera_id()?;
        self.mobs.iter().find(|m| m.id == id)
    }
    /// The entity under a spectator's crosshair (`Minecraft.hitResult` as an
    /// `EntityHitResult`): the nearest mob or object before any block.
    pub(super) fn spectator_click_target(&self) -> Option<i32> {
        let mob = self.mob_hit().map(|(i, d)| (self.mobs[i].id as i32, d));
        let object = self.remote_object_hit(false).map(|h| (h.id, h.distance));
        [mob, object]
            .into_iter()
            .flatten()
            .filter(|(_, d)| self.before_block(*d))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
    }
    /// `LevelRenderer`: the camera entity is not drawn from its own eyes.
    pub(super) fn hides_as_camera(&self, id: u32) -> bool {
        self.camera_type.first_person() && self.remote_camera_id() == Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::{camera_after_packet, entity_post_effect};

    /// Only a creeper, spider or enderman has a spectator chain.
    #[test]
    fn spectated_entities_pick_their_chain() {
        assert_eq!(
            entity_post_effect("minecraft:creeper"),
            Some("minecraft:creeper")
        );
        assert_eq!(
            entity_post_effect("minecraft:spider"),
            Some("minecraft:spider")
        );
        assert_eq!(
            entity_post_effect("minecraft:enderman"),
            Some("minecraft:invert")
        );
        assert_eq!(
            entity_post_effect("minecraft:cave_spider"),
            Some("minecraft:spider")
        );
        assert_eq!(entity_post_effect("minecraft:cow"), None);
    }

    /// The camera goes to a known entity, back to the player on its own id,
    /// and stays where it is for an entity the level does not hold.
    #[test]
    fn set_camera_follows_vanilla() {
        assert_eq!(camera_after_packet(None, 5, 9, true), Some(9));
        assert_eq!(camera_after_packet(Some(9), 5, 5, false), None);
        assert_eq!(camera_after_packet(Some(9), 5, 12, false), Some(9));
        assert_eq!(camera_after_packet(None, 5, 12, false), None);
    }
}
