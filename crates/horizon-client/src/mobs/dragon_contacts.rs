//! `EnderDragon.knockBack` and `hurt` for living mobs.
//! The dragon lives outside the normal mob array; its contacts must still reach them.
// Contact rules compared with the installed vanilla Java 26.3 EnderDragon.
use crate::mobs::{Damage, DamageKind, Mob, Who};
use horizon_core::geometry::{Aabb, Vec3};

pub struct Frame {
    pub dragon_id: u32,
    pub origin: Vec3,
    pub center: (f64, f64),
    pub wings: [Aabb; 2],
    pub heads: [Aabb; 2],
    pub sitting: bool,
    pub recently_hurt: bool,
}

/// Wings push even while perched and even when teams forbid damage.
/// The normal mob damage handler owns armor, invulnerability frames and last-attacker bookkeeping.
pub fn contact(m: &mut Mob, f: &Frame, teams_allow: bool) {
    if f.recently_hurt || m.removed || m.id == f.dragon_id {
        return;
    }
    let bounds = m.bounds();
    let source = Damage {
        kind: DamageKind::MobAttack,
        attacker: Some(Who::Mob(f.dragon_id)),
        from: Some([f.origin.x, f.origin.y, f.origin.z]),
    };
    for wing in &f.wings {
        if wing.intersects(bounds) {
            let dx = m.position.x - f.center.0;
            let dz = m.position.z - f.center.1;
            let dd = (dx * dx + dz * dz).max(0.1);
            m.velocity.x += dx / dd * 4.0;
            m.velocity.y += f64::from(0.2_f32);
            m.velocity.z += dz / dd * 4.0;
            if !f.sitting
                && teams_allow
                && m.ai.last_hurt_by_time < (m.age as i32).saturating_sub(2)
            {
                m.damage(5.0, source);
            }
        }
    }
    if teams_allow {
        for head in &f.heads {
            if head.intersects(bounds) {
                m.damage(10.0, source);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn box_at(x: f64) -> Aabb {
        Aabb::new(Vec3::new(x, -1.0, -1.0), Vec3::new(x + 2.0, 3.0, 1.0))
    }
    fn setup() -> (Mob, Frame) {
        let mut m = Mob::new(
            22,
            crate::mobs::sensor_scan_tests::kind(),
            Vec3::new(1.0, 0.0, 0.0),
            0.0,
        );
        m.health = 40.0;
        m.age = 100;
        let f = Frame {
            dragon_id: 99,
            origin: Vec3::ZERO,
            center: (0.0, 0.0),
            wings: [box_at(0.0), box_at(100.0)],
            heads: [box_at(100.0), box_at(100.0)],
            sitting: false,
            recently_hurt: false,
        };
        (m, f)
    }
    #[test]
    fn flying_wing_hurts_and_names_the_dragon() {
        let (mut m, f) = setup();
        contact(&mut m, &f, true);
        assert_eq!(m.health, 35.0);
        assert_eq!(m.ai.last_hurt_by, Some(Who::Mob(99)));
    }
    #[test]
    fn perched_wing_pushes_without_damage() {
        let (mut m, mut f) = setup();
        f.sitting = true;
        contact(&mut m, &f, true);
        assert_eq!(m.health, 40.0);
        assert_eq!(m.velocity, Vec3::new(4.0, f64::from(0.2_f32), 0.0));
    }
    #[test]
    fn team_protection_keeps_push_but_blocks_damage() {
        let (mut m, f) = setup();
        contact(&mut m, &f, false);
        assert_eq!(m.health, 40.0);
        assert_eq!(m.velocity, Vec3::new(4.0, f64::from(0.2_f32), 0.0));
    }
    #[test]
    fn hurt_dragon_does_not_contact_mobs() {
        let (mut m, mut f) = setup();
        f.recently_hurt = true;
        contact(&mut m, &f, true);
        assert_eq!(m.health, 40.0);
        assert_eq!(m.velocity, Vec3::ZERO);
    }
    #[test]
    fn head_hurts_even_when_perched() {
        let (mut m, mut f) = setup();
        f.sitting = true;
        f.wings = [box_at(100.0), box_at(100.0)];
        f.heads[0] = box_at(0.0);
        contact(&mut m, &f, true);
        assert_eq!(m.health, 30.0);
    }
    #[test]
    fn recent_mob_attack_suppresses_wing_damage() {
        let (mut m, f) = setup();
        m.ai.last_hurt_by_time = 99;
        contact(&mut m, &f, true);
        assert_eq!(m.health, 40.0);
        assert_eq!(m.velocity, Vec3::new(4.0, f64::from(0.2_f32), 0.0));
    }
}

#[cfg(test)]
mod parity {
    use super::*;
    #[test]
    #[ignore = "writes the vanilla parity transcript"]
    fn contact_transcript() {
        let mut rows = Vec::new();
        for name in [
            "flying",
            "sitting",
            "two_wings",
            "team",
            "head",
            "hurt_dragon",
            "recent_victim",
        ] {
            let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
            kind.id = "minecraft:enderman".into();
            kind.species = crate::mobs::Species::Enderman;
            kind.max_health = 40.0;
            kind.width = 0.6;
            kind.height = 2.9;
            let mut m = Mob::new(22, std::sync::Arc::new(kind), Vec3::new(1.0, 0.0, 0.0), 0.0);
            m.age = 100;
            let near = Aabb::new(Vec3::new(-10.0, -10.0, -10.0), Vec3::new(10.0, 10.0, 10.0));
            let far = near.move_by(Vec3::new(100.0, 0.0, 0.0));
            let mut f = Frame {
                dragon_id: 99,
                origin: Vec3::ZERO,
                center: (0.0, 0.0),
                wings: [near, far],
                heads: [far, far],
                sitting: name == "sitting",
                recently_hurt: name == "hurt_dragon",
            };
            if name == "two_wings" {
                f.wings = [near, near];
            }
            if name == "head" {
                f.wings = [far, far];
                f.heads = [near, far];
            }
            if name == "recent_victim" {
                m.ai.last_hurt_by = Some(Who::Mob(99));
                m.ai.last_hurt_by_time = 99;
            }
            contact(&mut m, &f, name != "team");
            rows.push(serde_json::json!({"case":name,"health":m.health,"velocity":[m.velocity.x,m.velocity.y,m.velocity.z],"revenge":m.ai.last_hurt_by==Some(Who::Mob(99))}));
        }
        let path = std::env::var_os("HORIZON_DRAGON_CONTACT_OUTPUT").expect("parity output path");
        std::fs::write(path, serde_json::to_vec_pretty(&rows).unwrap()).unwrap();
    }
}
