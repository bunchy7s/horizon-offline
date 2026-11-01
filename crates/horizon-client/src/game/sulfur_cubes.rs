//! Sulfur cubes on the game side (`SulfurCube.mobInteract`, `pickUpItem`,
//! `remove`): feeding babies, swallowing a held block, shearing it back
//! out, lighting an explosive one, swallowing blocks from the ground and
//! splitting into two babies.
use super::Game;
use horizon_client::{
    items::ItemStack,
    mesher::Vertex,
    mobs::{Mob, Species, Who, sulfur_cube},
    projectiles::Projectile,
};
use horizon_core::{position::BlockPos, random::RandomSource};

fn block_transform(base: glam::Mat4, baby: bool) -> glam::Mat4 {
    base * glam::Mat4::from_scale(glam::Vec3::new(1.0, -1.0, -1.0))
        * glam::Mat4::from_scale(glam::Vec3::splat(if baby { 0.5 } else { 1.0 }))
        * glam::Mat4::from_translation(glam::Vec3::new(-0.5, -0.518, -0.5))
}

fn block_overlay(fuse_ticks: i32, alpha: f32, hurt: bool) -> u32 {
    let fuse = if fuse_ticks >= 0 {
        fuse_ticks as f32 - alpha + 1.0
    } else {
        0.0
    };
    if fuse > 0.0 && (fuse / 5.0) as i32 % 2 == 0 {
        horizon_client::overlay::living(1.0, false)
    } else {
        horizon_client::overlay::living(0.0, hurt)
    }
}

fn block_light(light: u32, emission: u8) -> u32 {
    let block = (light >> 8 & 0xFF).max(u32::from(emission) * 16);
    (light & 0xFF) | block << 8
}

impl Game<'_> {
    /// The swallowed block replaces the opaque core and precedes the shell.
    pub(super) fn sulfur_cube_body_geometry(
        &self,
        mob: &Mob,
        alpha: f32,
        base: glam::Mat4,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        let Some(item) = mob.extra.head.as_deref() else {
            return;
        };
        if super::armor_stands::skull_kind(item).is_some() {
            return;
        }
        // shortcut: the body slot retains only an item id; preserve the stack
        // before supporting its block-state components here.
        let Some(state) = self.items().get(item).and_then(|item| item.block) else {
            return;
        };
        let start = out.len();
        self.block_model_vertices(
            state,
            42,
            block_transform(base, mob.is_baby()),
            block_light(light, self.res.facts().get(state).emission),
            true,
            out,
        );
        let overlay = block_overlay(
            sulfur_cube::fuse(mob),
            alpha,
            mob.hurt_time > 0 || mob.death_time > 0,
        );
        if overlay != 0 {
            for vertex in &mut out[start..] {
                vertex.layer = overlay;
            }
        }
    }

    fn sulfur_sound(&mut self, index: usize, sound: &str) {
        let at = self.mobs[index].center();
        self.audio
            .play(&format!("minecraft:{sound}"), Some(at), 1.0, 1.0);
    }
    /// Spit the swallowed block out on top (`EntityAttachment.PASSENGER`).
    fn eject_sulfur_block(&mut self, index: usize) {
        let m = &mut self.mobs[index];
        let Some(item) = m.extra.head.take() else {
            return;
        };
        let at = [
            m.position.x,
            m.position.y + f64::from(m.height()),
            m.position.z,
        ];
        self.spawn_item(ItemStack::new(&item, 1), at, [0.0, 0.2, 0.0], 10);
    }
    /// `SulfurCube.mobInteract`. Returns whether the click was used.
    pub(super) fn interact_sulfur_cube(&mut self, index: usize, item: &str) -> bool {
        let m = &self.mobs[index];
        if m.species() != Species::SulfurCube || !m.alive() {
            return false;
        }
        if m.is_baby() {
            // Slime balls grow a baby up by a tenth (`canAgeUp`: not while an
            // age lock holds it).
            if !m.kind.food.iter().any(|f| f == item) || !horizon_client::mobs::ai::can_age_up(m) {
                return false;
            }
            let m = &mut self.mobs[index];
            let secs = (-m.extra.age / 20) as f32 * 0.1;
            // AgeableMob.ageUp: a grown cube is size 2 (SulfurCube.ageBoundaryReached).
            horizon_client::mobs::ai::age_up(m, secs as i32, false);
            self.consume_held();
            self.sulfur_sound(index, "entity.small_sulfur_cube.eat");
            return true;
        }
        if sulfur_cube::primed(m) {
            return false;
        }
        let explosive = sulfur_cube::archetype(m).is_some_and(|a| a.explosion.is_some());
        if explosive && matches!(item, "minecraft:flint_and_steel" | "minecraft:fire_charge") {
            if !self.ui.rules.bool("tnt_explodes") {
                return false;
            }
            sulfur_cube::prime(&mut self.mobs[index], false);
            if item == "minecraft:flint_and_steel" {
                self.damage_held_tool();
            } else {
                self.consume_held();
            }
            return true;
        }
        if item == "minecraft:shears" && sulfur_cube::has_body(m) {
            // shear: out comes the block, and nothing is swallowed for 5 s.
            let sheared = self.mobs[index].extra.head.clone();
            self.eject_sulfur_block(index);
            self.mobs[index].extra.ints[3] = sulfur_cube::PICKUP_TIMER;
            self.sulfur_sound(index, "entity.sulfur_cube.eject");
            self.damage_held_tool();
            // CriteriaTriggers.PLAYER_SHEARED_EQUIPMENT with the body block.
            if let Some(body) = sheared {
                let target = self.mob_entity(&self.mobs[index]);
                self.trigger_item_entity(
                    "minecraft:player_sheared_equipment",
                    &ItemStack::new(&body, 1),
                    &target,
                );
            }
            return true;
        }
        if sulfur_cube::swallowable(m, item) {
            // equipItem: not the same block again; the old one comes out.
            if m.extra.head.as_deref() == Some(item) {
                return false;
            }
            self.eject_sulfur_block(index);
            self.mobs[index].extra.head = Some(item.into());
            self.consume_held();
            self.sulfur_sound(index, "entity.sulfur_cube.absorb");
            return true;
        }
        false
    }
    /// `SulfurCube.primeWhenOnPoweredPosition`: an explosive cube standing in
    /// or next to redstone power lights its fuse (`getBestOwnOrNeighbourSignal`;
    /// no block has an own signal, so the neighbour signal is all there is).
    pub(super) fn sulfur_cube_power_priming(&mut self) {
        if !self.ui.rules.bool("tnt_explodes") {
            return;
        }
        for i in 0..self.mobs.len() {
            let m = &self.mobs[i];
            if m.species() != Species::SulfurCube
                || !sulfur_cube::archetype(m).is_some_and(|a| a.explosion.is_some())
                || !m.alive()
                || sulfur_cube::primed(m)
            {
                continue;
            }
            let at = BlockPos::new(
                m.position.x.floor() as i32,
                m.position.y.floor() as i32,
                m.position.z.floor() as i32,
            );
            let signal =
                self.with_level(|b, level, ticks| b.best_neighbor_signal(level, ticks, at));
            if signal != 0 {
                sulfur_cube::prime(&mut self.mobs[i], false);
            }
        }
    }
    /// `SulfurCube.hurtServer`: an arrow or a trident on fire that hits an
    /// explosive cube with a block lights its fuse (with `tnt_explodes`, as
    /// `primeTime` asks), called after the hit's hurt, which the priming
    /// leaves as it is, so its knock and the fuse both go ahead.
    pub(super) fn burning_arrow_primes(&mut self, p: &Projectile, who: Who) {
        if !p.arrow.as_ref().is_some_and(|a| a.on_fire()) || !self.ui.rules.bool("tnt_explodes") {
            return;
        }
        if let Who::Mob(id) = who
            && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
            && m.species() == Species::SulfurCube
        {
            sulfur_cube::burning_arrow_hit(m);
        }
    }
    /// `SulfurCubeSearchForItemsGoal` and `pickUpItem`: an adult with nothing
    /// inside hops to a block it can swallow within 8 and takes it in.
    pub(super) fn sulfur_cube_pickups(&mut self) {
        let griefing = self.ui.rules.bool("mob_griefing");
        for i in 0..self.mobs.len() {
            let m = &self.mobs[i];
            if m.species() != Species::SulfurCube {
                continue;
            }
            if !m.alive() || m.is_baby() || sulfur_cube::has_body(m) || m.extra.ints[3] > 0 {
                self.mobs[i].extra.point = None;
                continue;
            }
            let reach = m
                .bounds()
                .inflate(horizon_core::geometry::Vec3::new(8.0, 8.0, 8.0));
            let pos = m.position;
            let best = self
                .dropped
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    e.pickup_delay == 0
                        && reach.contains(e.position)
                        && sulfur_cube::swallowable(m, &e.stack.item)
                })
                .map(|(j, e)| (j, e.position.distance_squared(pos)))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let Some((j, _)) = best else {
                self.mobs[i].extra.point = None;
                continue;
            };
            // Mob.aiStep: items within the box grown by (1, 0, 1).
            let touch = m
                .bounds()
                .inflate(horizon_core::geometry::Vec3::new(1.0, 0.0, 1.0));
            let e = &self.dropped[j];
            let close = touch.contains(e.position);
            self.mobs[i].extra.point = Some([e.position.x, e.position.y, e.position.z]);
            if !griefing || !close {
                continue;
            }
            let item = e.stack.item.clone();
            let thrown_by_player = e
                .thrower
                .is_some_and(|uuid| Some(uuid) == self.level.player_uuid);
            let mut swallowed = e.stack.clone();
            swallowed.count = 1;
            let (taken_item, id) = (e.clone(), self.mobs[i].id);
            self.add_pickup(
                super::pickups::Taken::Item(Box::new(taken_item)),
                super::pickups::Collector::Mob(id),
            );
            let e = &mut self.dropped[j];
            e.stack.count -= 1;
            if e.stack.count == 0 {
                self.remove_dropped(j);
            }
            let m = &mut self.mobs[i];
            m.extra.head = Some(item);
            m.extra.point = None;
            self.sulfur_sound(i, "entity.sulfur_cube.absorb");
            if thrown_by_player {
                let target = self.mob_entity(&self.mobs[i]);
                self.trigger_item_entity(
                    "minecraft:thrown_item_picked_up_by_entity",
                    &swallowed,
                    &target,
                );
            }
        }
    }
    /// `AbstractCubeMob.remove`: a dead adult splits into two babies (none
    /// when it was primed).
    pub(super) fn split_sulfur_cube(&mut self, m: &Mob) {
        let size = m.extra.size;
        if size <= 1 {
            return;
        }
        let offset = f64::from(m.width()) / 2.0;
        for i in 0..sulfur_cube::split_count(m) {
            let xd = (f64::from(i % 2) - 0.5) * offset;
            let zd = (f64::from(i / 2) - 0.5) * offset;
            let at = [m.position.x + xd, m.position.y + 0.5, m.position.z + zd];
            let yaw = self.world.random.next_float() * 360.0;
            if let Some(id) = self.spawn_mob(&m.kind.id, at, m.persistent)
                && let Some(c) = self.mobs.iter_mut().find(|c| c.id == id)
            {
                c.extra.age = -24000;
                sulfur_cube::set_size(c, size / 2, true);
                c.yaw = yaw;
                c.body_yaw = yaw;
                c.head_yaw = yaw;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swallowed_block_uses_vanilla_lift_and_baby_scale() {
        let base = glam::Mat4::from_translation(glam::Vec3::new(3.0, 4.0, -2.0))
            * glam::Mat4::from_rotation_y(0.7)
            * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0));
        for baby in [false, true] {
            let size = if baby { 0.5 } else { 1.0 };
            let block = block_transform(base, baby);
            for (point, expected) in [
                (glam::Vec3::ZERO, glam::Vec3::new(-0.5, 0.518, 0.5)),
                (glam::Vec3::ONE, glam::Vec3::new(0.5, -0.482, -0.5)),
            ] {
                let actual = block.transform_point3(point);
                let expected = base.transform_point3(expected * size);
                assert!(actual.abs_diff_eq(expected, 1e-5), "baby={baby}");
            }
            assert!(block.transform_vector3(glam::Vec3::Y).y > 0.0);
        }
    }

    #[test]
    fn swallowed_tnt_flashes_white_over_hurt_using_the_interpolated_fuse() {
        let white = horizon_client::overlay::WHITE + 15;
        assert_eq!(block_overlay(-1, 0.0, false), 0);
        assert_eq!(block_overlay(-1, 0.0, true), 1);
        assert_eq!(block_overlay(9, 0.0, true), white);
        assert_eq!(block_overlay(9, 0.5, false), 0);
        assert_eq!(block_overlay(9, 0.5, true), 1);
        assert_eq!(block_overlay(0, 0.5, false), white);
    }

    #[test]
    fn contained_block_emission_preserves_brighter_light_and_the_sky_channel() {
        assert_eq!(block_light(0x2050, 1), 0x2050);
        assert_eq!(block_light(0x2050, 7), 0x7050);
        assert_eq!(block_light(0x0070, 0), 0x0070);
    }
}
