//! What the game tests do to the entities and chunks of a test's area:
//! `StructureUtils.clearSpaceForStructure`'s and `GameTestInfo.succeed`'s
//! entity removal, and the force-loaded chunks of
//! `TestInstanceBlockEntity.forceLoadChunks` and `GameTestRunner`'s release.
use super::Game;
use horizon_world::worldgen::structure::BoundingBox;

type Aabb = ([f64; 3], [f64; 3]);

/// `AABB.of(BoundingBox)`.
pub(super) fn aabb_of(b: &BoundingBox) -> Aabb {
    (b.min.map(f64::from), b.max.map(|v| f64::from(v) + 1.0))
}

/// `AABB.inflate(amount)`.
pub(super) fn inflate(b: Aabb, amount: f64) -> Aabb {
    (b.0.map(|v| v - amount), b.1.map(|v| v + amount))
}

/// The box of an entity of `width` by `height` standing at `p`
/// (`EntityDimensions.makeBoundingBox`).
fn standing(p: [f64; 3], width: f64, height: f64) -> Aabb {
    let half = width / 2.0;
    (
        [p[0] - half, p[1], p[2] - half],
        [p[0] + half, p[1] + height, p[2] + half],
    )
}

/// `AABB.intersects(AABB)`: the open boxes overlap.
fn intersects(a: Aabb, b: Aabb) -> bool {
    (0..3).all(|i| a.0[i] < b.1[i] && a.1[i] > b.0[i])
}

impl Game<'_> {
    /// `level.getEntitiesOfClass(Entity.class, bounds, mob -> !(mob instanceof
    /// Player)).forEach(Entity::discard)`: every entity but a player whose box
    /// meets `bounds` is removed without dying or dropping anything.
    pub(super) fn discard_entities_in(&mut self, bounds: Aabb) {
        let at = |v: horizon_core::geometry::Vec3| [v.x, v.y, v.z];
        for m in &mut self.mobs {
            if !m.removed
                && intersects(
                    standing(at(m.position), f64::from(m.width()), f64::from(m.height())),
                    bounds,
                )
            {
                m.removed = true;
            }
        }
        // EntityType.ITEM and EXPERIENCE_ORB.
        self.dropped
            .retain(|i| !intersects(standing(at(i.position), 0.25, 0.25), bounds));
        self.orbs
            .retain(|o| !intersects(standing(at(o.position), 0.5, 0.5), bounds));
        self.creatures
            .projectiles
            .retain(|p| !intersects(standing(at(p.position), 0.5, 0.5), bounds));
        self.armor_stands.retain(|a| {
            // ArmorStand.getDimensions: a marker has none, a small one is
            // half scale.
            let (w, h) = match (a.marker, a.small) {
                (true, _) => (0.0, 0.0),
                (_, true) => (0.25, 0.9875),
                _ => (0.5, 1.975),
            };
            !intersects(standing(at(a.position), w, h), bounds)
        });
        self.item_frames.retain(|f| !intersects(f.bounds(), bounds));
        self.paintings.retain(|p| !intersects(p.bounds(), bounds));
        let (boat_w, boat_h) = (super::boats::WIDTH, super::boats::HEIGHT);
        let gone: Vec<u32> = self
            .boats
            .iter()
            .filter(|b| intersects(standing(at(b.position), boat_w, boat_h), bounds))
            .map(|b| b.id)
            .collect();
        for id in gone {
            if let Some(i) = self.boats.iter().position(|b| b.id == id) {
                let b = self.boats.remove(i);
                self.boat_removed(&b);
            }
        }
        let (cart_w, cart_h) = (super::minecarts::WIDTH, super::minecarts::HEIGHT);
        self.carts
            .retain(|c| !intersects(standing(at(c.position), cart_w, cart_h), bounds));
        // EntityType.END_CRYSTAL is 2 by 2.
        self.crystals
            .retain(|c| !intersects(standing(at(c.position), 2.0, 2.0), bounds));
        // Entities Horizon does not simulate have no box of their own: the
        // position decides.
        self.others.retain(|o| {
            let p = at(o.position);
            !(0..3).all(|i| p[i] >= bounds.0[i] && p[i] < bounds.1[i])
        });
    }

    /// `ServerLevel.setChunkForced(x, z, true)` for the chunks of `area`
    /// (`TestInstanceBlockEntity.forceLoadChunks`).
    pub(super) fn force_load_test_chunks(&mut self, area: &BoundingBox) {
        let (x0, x1) = (area.min[0] >> 4, area.max[0] >> 4);
        let (z0, z1) = (area.min[2] >> 4, area.max[2] >> 4);
        for x in x0..=x1 {
            for z in z0..=z1 {
                self.streaming.forced.insert((x, z));
            }
        }
        self.streaming.rank_forced(x0, z0, x1, z1);
    }

    /// `GameTestRunner`: every forced chunk of the level is released when a
    /// batch is done or a failure halts the run.
    pub(super) fn release_forced_chunks(&mut self) {
        self.streaming.forced.clear();
        self.levels.release_parked_forced_chunks();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_boundary_box_covers_its_last_block_and_inflates_both_ways() {
        let b = BoundingBox::new([0, 1, 2], [3, 4, 5]);
        assert_eq!(aabb_of(&b), ([0.0, 1.0, 2.0], [4.0, 5.0, 6.0]));
        assert_eq!(
            inflate(aabb_of(&b), 1.0),
            ([-1.0, 0.0, 1.0], [5.0, 6.0, 7.0])
        );
    }

    #[test]
    fn boxes_that_only_touch_do_not_intersect() {
        let area = ([0.0; 3], [4.0; 3]);
        // An entity standing on the area's floor, inside it.
        assert!(intersects(standing([2.0, 0.0, 2.0], 0.6, 1.8), area));
        // Its box ends exactly where the area starts (AABB.intersects is open).
        assert!(!intersects(standing([-0.3, 0.0, 2.0], 0.6, 1.8), area));
        assert!(intersects(standing([-0.2, 0.0, 2.0], 0.6, 1.8), area));
        // Above the area.
        assert!(!intersects(standing([2.0, 4.0, 2.0], 0.6, 1.8), area));
    }
}
