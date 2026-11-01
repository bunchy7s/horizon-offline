//! Server `minecraft:interaction` entities (`Interaction`): invisible boxes a
//! server (Denizen, Nexo) uses for clickable areas. They are pickable, render
//! nothing, and an interact ends at them: `SUCCESS` when they respond (the arm
//! swings), `CONSUME` otherwise (the click still goes to the server, nothing
//! behind them is used).
use super::{Game, ray_box};
use horizon_core::geometry::{Aabb, Vec3};
use horizon_protocol::entity_data::Value;

/// `Interaction`'s synced data and box: `DATA_WIDTH` (8), `DATA_HEIGHT` (9)
/// and `DATA_RESPONSE` (10), from its `defineSynchedData`.
pub(super) struct RemoteInteraction {
    pub position: Vec3,
    pub width: f32,
    pub height: f32,
    pub response: bool,
    /// `Entity.getVehicle`: ModelEngine's hitboxes ride the pivot that
    /// carries them.
    pub vehicle: Option<u32>,
}

impl RemoteInteraction {
    pub fn new(position: [f64; 3]) -> Self {
        Self {
            position: Vec3::new(position[0], position[1], position[2]),
            width: 1.0,
            height: 1.0,
            response: false,
            vehicle: None,
        }
    }
    /// `Entity.positionRider`'s move: a passenger is placed on its vehicle's
    /// seat, not moved on its own.
    pub fn ride_to(&mut self, at: glam::DVec3) {
        self.position = Vec3::new(at.x, at.y, at.z);
    }
    /// `Interaction.refreshDimensions`: `width` square about the entity's
    /// x and z, from its feet up `height` (its position is the box's bottom
    /// centre, not the model's).
    pub fn bounds(&self) -> Aabb {
        let half = f64::from(self.width) / 2.0;
        let p = self.position;
        Aabb::new(
            Vec3::new(p.x - half, p.y, p.z - half),
            Vec3::new(
                p.x + half,
                p.y + f64::from(self.height.max(0.0)),
                p.z + half,
            ),
        )
    }
    pub fn metadata(&mut self, values: &[(u8, i32, Value)]) {
        for (index, _, value) in values {
            match (index, value) {
                (8, Value::Float(width)) => self.width = *width,
                (9, Value::Float(height)) => self.height = *height,
                (10, Value::Bool(response)) => self.response = *response,
                _ => {}
            }
        }
    }
}

impl Game<'_> {
    /// `Interaction` under the crosshair within the entity reach, as its id
    /// and the hit distance along the view.
    pub(super) fn interaction_hit(&self) -> Option<(i32, f64)> {
        let (eye, dir) = self.aim()?;
        let reach = self.entity_reach;
        self.interactions
            .iter()
            .filter_map(|(&id, i)| ray_box(eye, dir, reach, i.bounds()).map(|d| (id, d)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }
    /// An `Interaction` keeps `Entity`'s no-op interpolation handler: the
    /// server's position replaces its own at once.
    pub(super) fn move_remote_interaction(&mut self, id: i32, position: [f64; 3]) {
        if let Some(i) = self.interactions.get_mut(&id) {
            i.position = Vec3::new(position[0], position[1], position[2]);
        }
    }
    pub(super) fn remote_interaction_data(&mut self, id: i32, values: &[(u8, i32, Value)]) -> bool {
        let Some(i) = self.interactions.get_mut(&id) else {
            return false;
        };
        i.metadata(values);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_box_is_centred_on_the_feet_and_grows_with_the_data() {
        let mut i = RemoteInteraction::new([10.0, 64.0, -3.0]);
        let b = i.bounds();
        assert_eq!(
            (b.min_position().components(), b.max_position().components()),
            ([9.5, 64.0, -3.5], [10.5, 65.0, -2.5])
        );
        i.metadata(&[
            (8, 0, Value::Float(2.2)),
            (9, 0, Value::Float(2.2)),
            (10, 0, Value::Bool(true)),
        ]);
        let b = i.bounds();
        let half = f64::from(2.2f32) / 2.0;
        let height = f64::from(2.2f32);
        let (lo, hi) = (b.min_position().components(), b.max_position().components());
        for (got, want) in lo.iter().zip([10.0 - half, 64.0, -3.0 - half]) {
            assert!((got - want).abs() < 1e-6, "{lo:?}");
        }
        for (got, want) in hi.iter().zip([10.0 + half, 64.0 + height, -3.0 + half]) {
            assert!((got - want).abs() < 1e-6, "{hi:?}");
        }
        assert!(i.response);
    }
    #[test]
    fn a_riding_hitbox_moves_with_its_pivot() {
        let mut i = RemoteInteraction::new([10.0, 64.0, -3.0]);
        i.vehicle = Some(9);
        i.ride_to(glam::DVec3::new(12.0, 65.5, -3.0));
        assert_eq!(i.position, Vec3::new(12.0, 65.5, -3.0));
        let b = i.bounds();
        assert_eq!(
            (b.min_position().components(), b.max_position().components()),
            ([11.5, 65.5, -3.5], [12.5, 66.5, -2.5])
        );
    }
}
