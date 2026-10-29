//! Entity shadows (`EntityRenderer.extractShadow` + `ShadowFeatureRenderer`):
//! `misc/shadow.png` projected onto the tops of full-collision blocks under
//! an entity, faded by camera distance, depth below the entity and the light
//! at each block.
use crate::{blocks::BlockFacts, hud::Rect, mesher::Vertex, mobs::Mob, world::World};
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

/// The mobs that cast a shadow, at their feet position interpolated by
/// `alpha`, with their type id. `EntityRenderer.extractShadow` skips an entity
/// whose `EntityRenderState.isInvisible` is set (`Entity.isInvisible`).
pub fn mob_casters(mobs: &[Mob], alpha: f64) -> impl Iterator<Item = (DVec3, &str)> {
    mobs.iter().filter(|m| !m.is_invisible()).map(move |m| {
        let (p, q) = (m.previous, m.position);
        let pos = DVec3::new(
            p.x + (q.x - p.x) * alpha,
            p.y + (q.y - p.y) * alpha,
            p.z + (q.z - p.z) * alpha,
        );
        (pos, m.kind.id.as_str())
    })
}

/// Shadow quads for an entity at `pos` (interpolated feet position).
#[allow(clippy::too_many_arguments)]
pub fn shadow(
    world: &World,
    facts: &BlockFacts,
    sprite: Rect,
    camera: DVec3,
    pos: DVec3,
    radius: f32,
    strength: f32,
    ambient_light: f32,
    sky_darken: i32,
    out: &mut Vec<Vertex>,
) {
    let radius = radius.min(32.0);
    if radius <= 0.0 {
        return;
    }
    let dist_sq = pos.distance_squared(camera);
    let pow = ((1.0 - dist_sq / 256.0) * f64::from(strength)) as f32;
    if pow <= 0.0 {
        return;
    }
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
            for y in y0..=y1 {
                let below = facts.get(world.block(x, y - 1, z));
                if below.render_shape == crate::blocks::RenderShape::Invisible
                    || !below.full_collision
                {
                    continue;
                }
                let packed = world.light(x, y, z);
                let raw = (i32::from(packed >> 4) - sky_darken).max(i32::from(packed & 15));
                if raw <= 3 {
                    continue;
                }
                let power_at_depth = pow - (pos.y - f64::from(y)) as f32 * 0.5;
                let alpha = (power_at_depth
                    * 0.5
                    * crate::screen_effects::lightmap_brightness(ambient_light, raw as u8))
                .clamp(0.0, 1.0);
                // The block top, clipped to the shadow square so the UVs stay inside the sprite.
                let (bx0, bx1) = (f64::from(x).max(pos.x - r), f64::from(x + 1).min(pos.x + r));
                let (bz0, bz1) = (f64::from(z).max(pos.z - r), f64::from(z + 1).min(pos.z + r));
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
                let top = f64::from(y);
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
            &world, &facts, sprite, camera, pos, 0.3, 1.0, 0.0, 0, &mut out,
        );
        assert_eq!(out.len(), 4);
        let feet = (pos.y + (1.501 - 24.0 / 16.0) - camera.y) as f32;
        for v in &out {
            assert_eq!(v.pos[1], (pos.y - camera.y) as f32);
            assert!(v.pos[1] < feet);
        }
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
            vec![(DVec3::new(0.5, 64.0, 1.0), "minecraft:cow")],
            "only the visible mob casts a shadow"
        );
    }
    #[test]
    fn vanilla_radii() {
        assert_eq!(shadow_params("minecraft:zombie"), (0.5, 1.0));
        assert_eq!(shadow_params("minecraft:item"), (0.15, 0.75));
        assert_eq!(shadow_params("minecraft:spider"), (0.8, 1.0));
    }
}
