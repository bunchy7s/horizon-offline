//! Entity shadows (`EntityRenderer.extractShadow` + `ShadowFeatureRenderer`):
//! `misc/shadow.png` projected onto block collision bounds under
//! an entity, faded by camera distance, depth below the entity and the light
//! at each block.
use crate::{
    blocks::BlockFacts, hud::Rect, mesher::Vertex, mobs::Mob, text_display::TextDisplayData,
    world::World,
};
use glam::DVec3;

/// Vanilla renderer shadow radius and strength for an entity type id.
pub fn shadow_params(entity: &str) -> (f32, f32) {
    let id = entity.strip_prefix("minecraft:").unwrap_or(entity);
    let radius = match id {
        "item" | "experience_orb" | "tropical_fish" => 0.15,
        "tadpole" => 0.14,
        "pufferfish" => 0.2,
        "slime" | "magma_cube" | "bat" => 0.25,
        "chicken" | "cod" | "parrot" | "rabbit" | "frog" | "silverfish" | "endermite" | "vex" => {
            0.3
        }
        "cat" | "ocelot" | "fox" | "bee" | "allay" | "salmon" | "armadillo" => 0.4,
        "zombie" | "husk" | "drowned" | "zombie_villager" | "zombified_piglin" | "skeleton"
        | "stray" | "wither_skeleton" | "bogged" | "creeper" | "enderman" | "villager"
        | "wandering_trader" | "witch" | "pillager" | "vindicator" | "evoker" | "illusioner"
        | "blaze" | "breeze" | "snow_golem" | "copper_golem" | "strider" | "wolf" | "axolotl"
        | "tnt" | "falling_block" | "end_crystal" => 0.5,
        "cave_spider" => 0.56,
        "creaking" => 0.6,
        "pig" | "cow" | "mooshroom" | "sheep" | "goat" | "llama" | "trader_llama"
        | "iron_golem" | "camel" | "camel_husk" | "squid" | "glow_squid" | "dolphin" | "turtle"
        | "hoglin" | "zoglin" | "nautilus" | "zombie_nautilus" | "minecart" => 0.7,
        "horse" | "donkey" | "mule" | "skeleton_horse" | "zombie_horse" | "phantom" => 0.75,
        "spider" | "boat" => 0.8,
        "polar_bear" | "panda" | "warden" => 0.9,
        // GiantMobRenderer: 0.5 times its scale of 6.
        "giant" => 3.0,
        "elder_guardian" => 1.2,
        "happy_ghast" => 2.0,
        // ShulkerRenderer: none.
        "shulker" => 0.0,
        "wither" => 1.0,
        "sniffer" | "ravager" => 1.1,
        "ghast" => 1.5,
        _ => 0.5,
    };
    let strength = if matches!(id, "item" | "experience_orb") {
        0.75
    } else {
        1.0
    };
    (radius, strength)
}

/// A living entity's shadow radius: `LivingEntityRenderer.getShadowRadius`,
/// the type's radius times `minecraft:scale` (`LivingEntity.getScale`), and
/// for a mob (all but players and mannequins, whose `AvatarRenderer` is no
/// `MobRenderer`) times `LivingEntity.getAgeScale` too. Villagers' and
/// striders' renderers halve a baby's again and the turtle's takes 0.83 of
/// it; slimes, magma cubes, sulfur cubes and pufferfish size theirs alone.
pub fn mob_shadow_radius(m: &Mob) -> f32 {
    use crate::mobs::Species as S;
    let species = m.species();
    match species {
        // AbstractCubeMobRenderer: size * 0.25.
        S::Slime | S::MagmaCube | S::SulfurCube => return m.extra.size as f32 * 0.25,
        // PufferfishRenderer: 0.1 + 0.1 * puffState.
        S::Pufferfish => return 0.1 + 0.1 * m.extra.size as f32,
        _ => {}
    }
    let scale = if m.extra.model_scale > 0.0 {
        m.extra.model_scale
    } else {
        1.0
    };
    let radius = shadow_params(&m.kind.id).0 * scale;
    if matches!(&*m.kind.id, "minecraft:player" | "minecraft:mannequin") || !m.is_baby() {
        return radius;
    }
    let age = match species {
        S::Armadillo | S::Camel | S::CamelHusk => 0.6,
        S::Dolphin => 0.65,
        S::Goat => 0.55,
        S::HappyGhast => 0.2375,
        S::Turtle => 0.3,
        _ => 0.5,
    };
    let radius = radius * age;
    match species {
        S::Villager | S::Strider => radius * 0.5,
        S::Turtle => radius * 0.83,
        _ => radius,
    }
}

/// The mobs that cast a shadow, at their feet position interpolated by
/// `alpha`, with their type's shadow radius and strength.
/// `EntityRenderer.extractShadow` skips an entity whose
/// `EntityRenderState.isInvisible` is set (`Entity.isInvisible`).
pub fn mob_casters(mobs: &[Mob], alpha: f64) -> impl Iterator<Item = (DVec3, f32, f32)> {
    mobs.iter().filter(|m| !m.is_invisible()).map(move |m| {
        let (p, q) = (m.previous, m.position);
        let pos = DVec3::new(
            p.x + (q.x - p.x) * alpha,
            p.y + (q.y - p.y) * alpha,
            p.z + (q.z - p.z) * alpha,
        );
        (pos, mob_shadow_radius(m), shadow_params(&m.kind.id).1)
    })
}

/// `DisplayRenderer.getShadowRadius` and `DisplayRenderer.getShadowStrength`:
/// the display's radius and strength interpolated by `t`, the two fields of
/// its render state. `EntityRenderer.extractShadow` draws nothing at a radius
/// of 0 or less, and text, block and item displays all go through this renderer.
pub fn display_caster(
    pos: DVec3,
    from: &TextDisplayData,
    shown: &TextDisplayData,
    t: f32,
) -> Option<(DVec3, f32, f32)> {
    let d = shown.blend(from, t);
    (d.shadow_radius > 0.0).then_some((pos, d.shadow_radius, d.shadow_strength))
}

/// `EntityRenderer.extractShadow`'s gates: the radius clamped to
/// `MAX_SHADOW_RADIUS` (32) and the strength faded over 256 blocks squared
/// from the camera. None when either is 0 or less, so no shadow is drawn.
fn shadow_gates(radius: f32, strength: f32, dist_sq: f64) -> Option<(f32, f32)> {
    let radius = radius.min(32.0);
    if radius <= 0.0 {
        return None;
    }
    let pow = ((1.0 - dist_sq / 256.0) * f64::from(strength)) as f32;
    (pow > 0.0).then_some((radius, pow))
}

/// Shadow quads for an entity at `pos` (interpolated feet position).
#[allow(clippy::too_many_arguments)]
pub fn shadow(
    world: &World,
    facts: &BlockFacts,
    collision_bounds: &impl Fn(
        horizon_world::registry::BlockStateId,
    ) -> Option<horizon_core::geometry::Aabb>,
    sprite: Rect,
    camera: DVec3,
    pos: DVec3,
    radius: f32,
    strength: f32,
    ambient_light: f32,
    sky_darken: i32,
    out: &mut Vec<Vertex>,
) {
    let Some((radius, pow)) = shadow_gates(radius, strength, pos.distance_squared(camera)) else {
        return;
    };
    let r = f64::from(radius);
    let (x0, x1) = ((pos.x - r).floor() as i32, (pos.x + r).floor() as i32);
    let (z0, z1) = ((pos.z - r).floor() as i32, (pos.z + r).floor() as i32);
    let depth = (pow / 0.5 - 1.0).min(radius);
    let (y0, y1) = (
        (pos.y - f64::from(depth)).floor() as i32,
        pos.y.floor() as i32,
    );
    for z in z0..=z1 {
        for x in x0..=x1 {
            for y in y0..=y1.saturating_add(1) {
                let state = world.block(x, y - 1, z);
                let below = facts.get(state);
                if below.render_shape == crate::blocks::RenderShape::Invisible {
                    continue;
                }
                let Some(bounds) = collision_bounds(state) else {
                    continue;
                };
                let top = f64::from(y - 1) + bounds.max_position().y;
                if top > pos.y + 1e-6 {
                    continue;
                }
                let packed = world.light(x, y, z);
                let raw = (i32::from(packed >> 4) - sky_darken).max(i32::from(packed & 15));
                if raw <= 3 {
                    continue;
                }
                let power_at_depth = pow - (pos.y - top) as f32 * 0.5;
                let alpha = (power_at_depth
                    * 0.5
                    * crate::screen_effects::lightmap_brightness(ambient_light, raw as u8))
                .clamp(0.0, 1.0);
                // The block top, clipped to the shadow square so the UVs stay inside the sprite.
                let (bx0, bx1) = (
                    (f64::from(x) + bounds.min_position().x).max(pos.x - r),
                    (f64::from(x) + bounds.max_position().x).min(pos.x + r),
                );
                let (bz0, bz1) = (
                    (f64::from(z) + bounds.min_position().z).max(pos.z - r),
                    (f64::from(z) + bounds.max_position().z).min(pos.z + r),
                );
                if bx1 <= bx0 || bz1 <= bz0 {
                    continue;
                }
                let u = |wx: f64| (-((wx - pos.x) as f32) / 2.0 / radius + 0.5).clamp(0.0, 1.0);
                let v = |wz: f64| (-((wz - pos.z) as f32) / 2.0 / radius + 0.5).clamp(0.0, 1.0);
                let a = (alpha * 255.0) as u32;
                let color = a << 24 | 0x00FF_FFFF;
                // ShadowFeatureRenderer: relativeY + shapeBelow.minY, the block top
                // itself. A model's feet sit 0.001 above it (LivingEntityRenderer's
                // translate(0, -1.501, 0)), so a lifted shadow fights with them.
                for (wx, wz) in [(bx0, bz0), (bx0, bz1), (bx1, bz1), (bx1, bz0)] {
                    let p = DVec3::new(wx, top, wz) - camera;
                    out.push(Vertex {
                        normal: [0; 4],
                        pos: p.as_vec3().to_array(),
                        uv: [
                            sprite.u0 + (sprite.u1 - sprite.u0) * u(wx),
                            sprite.v0 + (sprite.v1 - sprite.v0) * v(wz),
                        ],
                        layer: crate::gfx::SHADOW_OVERLAY,
                        color,
                        light: 240 | 240 << 8,

                        glint_uv: [0.0, 0.0],
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shadows_lie_on_the_block_top_under_model_feet() {
        use crate::chunk::Chunk;
        use horizon_world::level::data;
        use std::{path::PathBuf, sync::Arc};
        let root = std::env::var_os("HORIZON_RESOURCES")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/horizon-client")
                    .join(horizon_core::VANILLA_VERSION)
            });
        if !root.join("gameplay-facts.json").exists() {
            eprintln!("skipping resource-backed shadow check: {}", root.display());
            return;
        }
        let registry = data::load_registry(&root).unwrap();
        let facts = BlockFacts::load(&root, &registry).unwrap();
        let stone = registry.default_state("minecraft:stone").unwrap();
        let mut world = World::new(facts.air, 0);
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        for x in 0..16 {
            for z in 0..16 {
                world.set(x, 63, z, stone).unwrap();
            }
        }
        let (camera, pos) = (DVec3::new(8.5, 66.0, 4.0), DVec3::new(8.5, 64.0, 8.5));
        let mut out = Vec::new();
        let sprite = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 16,
            h: 16,
        };
        shadow(
            &world,
            &facts,
            &|_| {
                Some(horizon_core::geometry::Aabb::new(
                    horizon_core::geometry::Vec3::ZERO,
                    horizon_core::geometry::Vec3::new(1.0, 1.0, 1.0),
                ))
            },
            sprite,
            camera,
            pos,
            0.3,
            1.0,
            0.0,
            0,
            &mut out,
        );
        assert_eq!(out.len(), 4);
        let feet = (pos.y + (1.501 - 24.0 / 16.0) - camera.y) as f32;
        for v in &out {
            assert_eq!(v.pos[1], (pos.y - camera.y) as f32);
            assert!(v.pos[1] < feet);
        }
        let [v0, v1, v2] = [out[0].pos, out[1].pos, out[2].pos].map(glam::Vec3::from);
        assert!(
            (v1 - v0).cross(v2 - v0).y > 0.0,
            "shadow quad winds upward (+Y) so back-face culling hides it from below"
        );
        let farmland = registry.default_state("minecraft:farmland").unwrap();
        world.set(8, 63, 8, farmland).unwrap();
        out.clear();
        let pos = DVec3::new(8.5, 63.9375, 8.5);
        shadow(
            &world,
            &facts,
            &|state| {
                (state == farmland).then(|| {
                    horizon_core::geometry::Aabb::new(
                        horizon_core::geometry::Vec3::ZERO,
                        horizon_core::geometry::Vec3::new(1.0, 0.9375, 1.0),
                    )
                })
            },
            sprite,
            camera,
            pos,
            0.3,
            1.0,
            0.0,
            0,
            &mut out,
        );
        assert_eq!(out.len(), 4);
        assert!(out.iter().all(|v| v.pos[1] == (pos.y - camera.y) as f32));
    }
    #[test]
    fn invisible_mobs_cast_no_shadow() {
        use horizon_core::geometry::Vec3;
        let kind = crate::mobs::sensor_scan_tests::kind();
        let mut seen = Mob::new(1, kind.clone(), Vec3::new(1.0, 64.0, 1.0), 0.0);
        seen.previous = Vec3::new(0.0, 64.0, 1.0);
        // A server's invisible mob (shared flag 0x20) and one ticked here
        // with the Invisibility effect.
        let mut flagged = Mob::new(2, kind.clone(), Vec3::ZERO, 0.0);
        flagged.invisible = true;
        let mut potion = Mob::new(3, kind, Vec3::ZERO, 0.0);
        potion.add_effect("minecraft:invisibility", 600, 0);
        let mobs = [seen, flagged, potion];
        let casters: Vec<_> = mob_casters(&mobs, 0.5).collect();
        assert_eq!(
            casters,
            vec![(DVec3::new(0.5, 64.0, 1.0), 0.7, 1.0)],
            "only the visible mob casts a shadow"
        );
    }
    fn mob(id: &str, species: crate::mobs::Species) -> Mob {
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.id = id.into();
        kind.species = species;
        Mob::new(
            1,
            std::sync::Arc::new(kind),
            horizon_core::geometry::Vec3::ZERO,
            0.0,
        )
    }
    #[test]
    fn babies_cast_their_age_scaled_shadow() {
        use crate::mobs::Species as S;
        let mut cow = mob("minecraft:cow", S::Cow);
        cow.extra.age = -24000;
        // MobRenderer: 0.7 * ageScale 0.5.
        assert_eq!(mob_shadow_radius(&cow), 0.35);
        let mut villager = mob("minecraft:villager", S::Villager);
        assert_eq!(mob_shadow_radius(&villager), 0.5);
        villager.extra.age = -24000;
        // VillagerRenderer halves MobRenderer's 0.5 * 0.5 again.
        assert_eq!(mob_shadow_radius(&villager), 0.125);
        let mut goat = mob("minecraft:goat", S::Goat);
        goat.extra.age = -24000;
        // Goat.getAgeScale: 0.55.
        assert_eq!(mob_shadow_radius(&goat), 0.7 * 0.55);
    }
    #[test]
    fn scaled_players_cast_a_scaled_shadow() {
        use crate::mobs::Species as S;
        let mut player = mob("minecraft:player", S::Zombie);
        assert_eq!(mob_shadow_radius(&player), 0.5);
        player.extra.model_scale = 2.0;
        assert_eq!(mob_shadow_radius(&player), 1.0);
        let mut mannequin = mob("minecraft:mannequin", S::Zombie);
        mannequin.extra.model_scale = 0.25;
        assert_eq!(mob_shadow_radius(&mannequin), 0.125);
        // AvatarRenderer is no MobRenderer: no age scale.
        mannequin.extra.age = -1;
        assert_eq!(mob_shadow_radius(&mannequin), 0.125);
    }
    #[test]
    fn sized_mobs_keep_their_own_radius() {
        use crate::mobs::Species as S;
        let mut slime = mob("minecraft:slime", S::Slime);
        slime.extra.size = 4;
        slime.extra.model_scale = 2.0;
        assert_eq!(mob_shadow_radius(&slime), 1.0);
        let mut puffer = mob("minecraft:pufferfish", S::Pufferfish);
        puffer.extra.size = 2;
        assert_eq!(mob_shadow_radius(&puffer), 0.3);
    }
    #[test]
    fn items_and_orbs_keep_their_radius() {
        // Not living: the caller takes the type's radius as it is.
        assert_eq!(shadow_params("minecraft:item").0, 0.15);
        assert_eq!(shadow_params("minecraft:experience_orb").0, 0.15);
    }
    #[test]
    fn vanilla_radii() {
        assert_eq!(shadow_params("minecraft:zombie"), (0.5, 1.0));
        assert_eq!(shadow_params("minecraft:item"), (0.15, 0.75));
        assert_eq!(shadow_params("minecraft:spider"), (0.8, 1.0));
    }
    /// `DisplayRenderer.getShadowRadius`/`getShadowStrength` behind
    /// `EntityRenderer.extractShadow`: a display casts a shadow only above a
    /// radius of 0, with the radius clamped to 32 and the strength faded by
    /// the camera distance.
    #[test]
    fn displays_cast_a_gated_shadow() {
        let pos = DVec3::new(4.0, 64.0, 4.0);
        let empty = TextDisplayData::default();
        assert_eq!(display_caster(pos, &empty, &empty, 0.5), None);
        let shown = TextDisplayData {
            shadow_radius: 40.0,
            shadow_strength: 0.5,
            ..TextDisplayData::default()
        };
        // Halfway from the default (no radius, full strength).
        assert_eq!(
            display_caster(pos, &empty, &shown, 0.5),
            Some((pos, 20.0, 0.75))
        );
        let (_, radius, strength) = display_caster(pos, &empty, &shown, 1.0).unwrap();
        assert_eq!((radius, strength), (40.0, 0.5));
        // Math.min(radius, 32) and (1 - distanceSq / 256) * strength.
        assert_eq!(shadow_gates(radius, strength, 64.0), Some((32.0, 0.375)));
        assert_eq!(shadow_gates(radius, strength, 256.0), None);
        assert_eq!(shadow_gates(radius, 0.0, 0.0), None);
    }
}
