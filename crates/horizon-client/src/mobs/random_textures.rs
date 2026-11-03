//! OptiFine random entity textures on mobs: which variant of each of its textures a mob wears
//! (`horizon_optifine::random_textures` finds them in the packs).
//!
//! [`choose`] runs when the client first sees a mob, about once a second after that, and on the
//! next tick when the mob draws a texture with rules it hasn't chosen for (a wolf just tamed;
//! at most once between those checks), never per frame. The spawn-time facts (biome, height, blocks, moon, time, weather) are fixed
//! when the mob is first seen; the others (name, profession, colour, baby, health, size) are
//! read fresh each time. Drawing only looks the choice up ([`texture`]). It walks the textures
//! drawing does ([`super::render::drawn_textures`]): a custom model's own texture and the
//! warden's glowing layers included.
//!
//! Each texture with its own rules (or those of the name it extends) wears that rule file's
//! pick. A texture without rules follows the body's variant, so a sheep wearing `sheep3` has
//! `sheep_wool3` (ETF), or an even choice when the body has no variants; villagers' layers each
//! choose their own, as ETF does.

use super::render::{DYE_NAMES, PROFESSIONS};
use super::{Mob, MobData, Species};
use horizon_optifine::random::{self, Block, Facts, Weather};
use horizon_optifine::random_textures::RandomTextures;
use std::sync::atomic::{AtomicBool, Ordering};

/// Where the mob was when the client first saw it.
#[derive(Clone, Debug)]
pub struct Seen {
    /// `minecraft:plains`; `None` in an unloaded chunk.
    pub biome: Option<String>,
    pub y: i32,
    /// The block it stood on and the block it stood in: ids and state properties.
    pub blocks: Vec<(String, Vec<(String, String)>)>,
    pub moon_phase: i32,
    pub day_time: i32,
    pub weather: Weather,
}

/// A mob's variants.
#[derive(Debug, Default)]
pub struct Choice {
    seen: Option<Seen>,
    seed: i32,
    /// The [`RandomTextures::generation`] the picks are from: another (a pack reload) makes
    /// them stale.
    generation: u64,
    /// The variant each rule file chose, by its index in [`RandomTextures::rules`].
    picks: Vec<(usize, u32)>,
    /// The body texture's variant.
    primary: Option<u32>,
    /// Ticks until the next check of the facts that change.
    countdown: u8,
    /// Drawing found a texture with rules and no pick: choose on the next tick, unless a miss
    /// was answered since the last regular check.
    missed: AtomicBool,
    /// A miss was answered since the last regular check: one per check, so a texture that
    /// drawing keeps missing (one `drawn_textures` doesn't list) can't make the mob choose
    /// every tick.
    answered: bool,
}

impl Clone for Choice {
    fn clone(&self) -> Self {
        Self {
            seen: self.seen.clone(),
            seed: self.seed,
            generation: self.generation,
            picks: self.picks.clone(),
            primary: self.primary,
            countdown: self.countdown,
            missed: AtomicBool::new(self.missed.load(Ordering::Relaxed)),
            answered: self.answered,
        }
    }
}

/// Ticks between checks of the facts that change.
const RECHECK: u8 = 20;

impl Choice {
    /// Whether the mob's first sighting is recorded.
    pub fn has_seen(&self) -> bool {
        self.seen.is_some()
    }

    /// Called once a tick: whether [`choose`] should run now. It does on first sight, after the
    /// packs changed, every [`RECHECK`] ticks (the first spread by `stagger`, the entity id, so
    /// a crowd seen at once doesn't check at once), and when drawing missed a pick, once
    /// between those checks.
    pub fn due(&mut self, textures: &RandomTextures, stagger: u32) -> bool {
        let fresh = !self.has_seen() || self.generation != textures.generation();
        if fresh || self.countdown == 0 {
            self.countdown = if fresh {
                (stagger % u32::from(RECHECK)) as u8
            } else {
                RECHECK - 1
            };
            self.answered = false;
            self.missed.store(false, Ordering::Relaxed);
            return true;
        }
        self.countdown -= 1;
        if self.missed.swap(false, Ordering::Relaxed) && !self.answered {
            self.answered = true;
            return true;
        }
        false
    }
}

/// An entity's seed from the UUID ints the client keeps (`UUIDUtil.uuidToIntArray`, most
/// significant first); its entity id when the UUID is unknown.
pub fn seed(m: &Mob, uuid: Option<[i32; 4]>) -> i32 {
    let long = |hi: i32, lo: i32| (i64::from(hi) << 32) | i64::from(lo as u32);
    uuid.map_or(m.id as i32, |u| {
        random::seed(long(u[0], u[1]), long(u[2], u[3]))
    })
}

/// Chooses the variant of each texture `m` draws now (its custom models' from `data`).
/// `seen` describes its surroundings the first time (when [`Choice::has_seen`] is false);
/// later calls keep that.
pub fn choose(
    m: &mut Mob,
    textures: &RandomTextures,
    data: &MobData,
    seen: Option<Seen>,
    seed: i32,
) {
    let names = super::render::drawn_textures(m, data);
    let mut choice = std::mem::take(&mut m.random_textures);
    if choice.seen.is_none() {
        choice.seen = seen;
    }
    let Some(seen) = choice.seen.take() else {
        m.random_textures = choice;
        return;
    };
    choice.seed = seed;
    choice.generation = textures.generation();
    choice.picks.clear();
    choice.primary = None;
    let states: Vec<Vec<(&str, &str)>> = seen
        .blocks
        .iter()
        .map(|(_, s)| s.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect())
        .collect();
    let blocks: Vec<Block> = seen
        .blocks
        .iter()
        .zip(&states)
        .map(|((id, _), states)| Block { id, states })
        .collect();
    let profession =
        matches!(m.species(), Species::Villager | Species::ZombieVillager).then(|| {
            let name = PROFESSIONS[m.extra.variant2 as usize % PROFESSIONS.len()];
            (format!("minecraft:{name}"), i32::from(m.extra.variant3))
        });
    let facts = Facts {
        biome: seen.biome.as_deref(),
        y: Some(seen.y),
        blocks: &blocks,
        name: m.custom_name.as_deref(),
        profession: profession.as_ref().map(|(p, level)| (p.as_str(), *level)),
        color: color(m),
        baby: Some(m.is_baby()),
        health: Some((m.health, super::finalize::max_health(m))),
        moon_phase: Some(seen.moon_phase),
        day_time: Some(seen.day_time),
        weather: Some(seen.weather),
        size: size(m),
    };
    for (i, name) in names.iter().enumerate() {
        let Some(entry) = textures.get(name) else {
            continue;
        };
        let index = match entry.rules {
            Some(id) => match choice.picks.iter().find(|p| p.0 == id) {
                Some(&(_, index)) => index,
                None => {
                    // No rule matching is the texture itself, as is ETF's 0.
                    let index = textures.rules(id).matching(&facts, seed).unwrap_or(0);
                    choice.picks.push((id, index));
                    index
                }
            },
            None => entry.even(seed),
        };
        if i == 0 {
            choice.primary = Some(index);
        }
    }
    choice.seen = Some(seen);
    m.random_textures = choice;
}

/// The atlas name of the variant `m` wears for `texture`, or `None` for the texture itself.
pub fn texture<'a>(m: &Mob, textures: &'a RandomTextures, texture: &str) -> Option<&'a str> {
    let entry = textures.get(texture)?;
    let choice = &m.random_textures;
    if !choice.has_seen() || choice.generation != textures.generation() {
        return None;
    }
    let index = match entry.rules {
        Some(id) => match choice.picks.iter().find(|p| p.0 == id) {
            Some(&(_, index)) => index,
            None => {
                choice.missed.store(true, Ordering::Relaxed);
                return None;
            }
        },
        None => match choice.primary {
            Some(primary)
                if !matches!(m.species(), Species::Villager | Species::ZombieVillager) =>
            {
                primary
            }
            _ => entry.even(choice.seed),
        },
    };
    entry.texture(index)
}

/// `colors`: a sheep's wool, a wolf's or cat's collar, a llama's carpet, a shulker's shell.
fn color(m: &Mob) -> Option<&str> {
    match m.species() {
        Species::Sheep | Species::Wolf | Species::Cat => {
            Some(DYE_NAMES[usize::from(m.extra.color & 15)])
        }
        Species::Shulker => DYE_NAMES.get(usize::from(m.extra.color)).copied(),
        Species::Llama | Species::TraderLlama => super::llama::carpet(m),
        _ => None,
    }
}

/// `sizes`, as the docs number them: a slime's or magma cube's size less one (0 small, 1 medium,
/// 3 big), a phantom's size.
fn size(m: &Mob) -> Option<i32> {
    match m.species() {
        Species::Slime | Species::MagmaCube => Some((m.extra.size - 1).max(0)),
        Species::Phantom => Some(m.extra.size),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::geometry::Vec3;
    use std::sync::Arc;

    fn mob(species: Species) -> Mob {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        Arc::make_mut(&mut kind).species = species;
        Mob::new(1, kind, Vec3::ZERO, 0.0)
    }

    /// No custom models.
    fn data() -> MobData {
        MobData {
            states: Vec::new(),
            placements: Default::default(),
            heightmaps: Default::default(),
            animations: Default::default(),
            villager_hats: Default::default(),
            cem: Default::default(),
            random_textures: Default::default(),
            emissive: Default::default(),
            blinks: Default::default(),
            biome_tags: Default::default(),
            entity_tags: Default::default(),
            pois: Default::default(),
            species_placements: Default::default(),
        }
    }

    fn seen(biome: &str) -> Seen {
        Seen {
            biome: Some(biome.into()),
            y: 64,
            blocks: vec![(
                "minecraft:grass_block".into(),
                vec![("snowy".into(), "false".into())],
            )],
            moon_phase: 0,
            day_time: 1000,
            weather: Weather::Clear,
        }
    }

    /// A temp pack of `files` (path under `optifine/random/entity`, or from the root when it
    /// starts with `assets/`; contents), scanned.
    fn pack(name: &str, files: &[(&str, &str)]) -> RandomTextures {
        let root =
            std::env::temp_dir().join(format!("horizon-mob-random-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (path, text) in files {
            let path = if path.starts_with("assets/") {
                root.join(path)
            } else {
                root.join("assets/minecraft/optifine/random/entity")
                    .join(path)
            };
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let textures = RandomTextures::scan(&root);
        let _ = std::fs::remove_dir_all(&root);
        textures
    }

    const SHEEP: &str = "entity/sheep/sheep";
    const WOOL: &str = "entity/sheep/sheep_wool";
    fn variant(name: &str) -> String {
        format!("minecraft:optifine/random/{name}")
    }

    #[test]
    fn a_rule_picks_the_body_and_its_layers_and_follows_the_colour() {
        let t = pack(
            "colour",
            &[
                (
                    "sheep/sheep.properties",
                    "textures.1=3\ncolors.1=red\ntextures.2=2",
                ),
                ("sheep/sheep2.png", ""),
                ("sheep/sheep3.png", ""),
                ("sheep/sheep_wool2.png", ""),
                ("sheep/sheep_wool3.png", ""),
            ],
        );
        let data = data();
        let mut m = mob(Species::Sheep);
        assert_eq!(texture(&m, &t, SHEEP), None, "not chosen yet");
        // Entity 1: chosen at first sight, then 1 tick later (the stagger), then every 20.
        assert!(m.random_textures.due(&t, m.id));
        m.extra.color = 14; // red
        choose(&mut m, &t, &data, Some(seen("minecraft:plains")), 0);
        let sheep3 = variant("entity/sheep/sheep3");
        assert_eq!(texture(&m, &t, SHEEP), Some(sheep3.as_str()));
        let wool3 = variant("entity/sheep/sheep_wool3");
        assert_eq!(texture(&m, &t, WOOL), Some(wool3.as_str()));
        let ticks: Vec<bool> = (0..23).map(|_| m.random_textures.due(&t, 1)).collect();
        let due: Vec<usize> = (0..23).filter(|&i| ticks[i]).collect();
        assert_eq!(due, [1, 21]);
        m.extra.color = 0;
        choose(&mut m, &t, &data, None, 0);
        let sheep2 = variant("entity/sheep/sheep2");
        assert_eq!(texture(&m, &t, SHEEP), Some(sheep2.as_str()));
        // Another scan (a pack reload): nothing until chosen again, at once, from the same
        // first sighting.
        let reloaded = pack(
            "colour-again",
            &[
                ("sheep/sheep.properties", "textures.1=2"),
                ("sheep/sheep2.png", ""),
            ],
        );
        assert_eq!(texture(&m, &reloaded, SHEEP), None);
        assert!(m.random_textures.due(&reloaded, m.id));
        choose(&mut m, &reloaded, &data, None, 0);
        assert_eq!(texture(&m, &reloaded, SHEEP), Some(sheep2.as_str()));
    }

    #[test]
    fn no_rule_matching_is_the_texture_itself() {
        // ETF: `sheep1.png` replaces variant 1, but a mob no rule matches (or a rule's 0)
        // wears vanilla's texture, and so does the wool that follows it.
        let t = pack(
            "zero",
            &[
                (
                    "sheep/sheep.properties",
                    "textures.1=2\nbiomes.1=desert\ntextures.2=0\nbiomes.2=forest",
                ),
                ("sheep/sheep1.png", ""),
                ("sheep/sheep2.png", ""),
                ("sheep/sheep_wool1.png", ""),
            ],
        );
        let data = data();
        for biome in ["minecraft:plains", "minecraft:forest"] {
            let mut m = mob(Species::Sheep);
            choose(&mut m, &t, &data, Some(seen(biome)), 0);
            assert_eq!(texture(&m, &t, SHEEP), None, "{biome}");
            assert_eq!(texture(&m, &t, WOOL), None, "{biome}");
        }
    }

    #[test]
    fn the_first_sighting_is_kept() {
        let t = pack(
            "biome",
            &[
                ("sheep/sheep.properties", "textures.1=2\nbiomes.1=desert"),
                ("sheep/sheep2.png", ""),
            ],
        );
        let data = data();
        let mut m = mob(Species::Sheep);
        choose(&mut m, &t, &data, Some(seen("minecraft:desert")), 0);
        let sheep2 = variant("entity/sheep/sheep2");
        assert_eq!(texture(&m, &t, SHEEP), Some(sheep2.as_str()));
        // Walking into plains doesn't change it: biomes are read when first seen.
        choose(&mut m, &t, &data, Some(seen("minecraft:plains")), 0);
        assert_eq!(texture(&m, &t, SHEEP), Some(sheep2.as_str()));
        // Without a first sighting nothing is chosen.
        let mut unseen = mob(Species::Sheep);
        choose(&mut unseen, &t, &data, None, 0);
        assert_eq!(texture(&unseen, &t, SHEEP), None);
    }

    #[test]
    fn layers_without_rules_follow_the_body() {
        // No rules: an even choice of sheep 1-3 by seed, and the wool wears the same number.
        let t = pack(
            "even",
            &[
                ("sheep/sheep2.png", ""),
                ("sheep/sheep3.png", ""),
                ("sheep/sheep_wool2.png", ""),
            ],
        );
        let data = data();
        let mut m = mob(Species::Sheep);
        choose(&mut m, &t, &data, Some(seen("minecraft:plains")), 4);
        // 4 % 3 + 1 = 2.
        let wool2 = variant("entity/sheep/sheep_wool2");
        assert_eq!(texture(&m, &t, WOOL), Some(wool2.as_str()));
        choose(&mut m, &t, &data, None, 5);
        // 5 % 3 + 1 = 3: there is no sheep_wool3, so the wool is vanilla's.
        let sheep3 = variant("entity/sheep/sheep3");
        assert_eq!(texture(&m, &t, SHEEP), Some(sheep3.as_str()));
        assert_eq!(texture(&m, &t, WOOL), None);
    }

    #[test]
    fn a_layer_with_its_own_rules_and_a_warden_glow() {
        // The warden's pulsating spots, OptiFine's own example: a layer `render_visible` adds
        // with rules of its own (and a name ending in a digit).
        let t = pack(
            "warden",
            &[
                ("warden/warden_pulsating_spots_2.properties", "textures.1=2"),
                ("warden/warden_pulsating_spots_2.2.png", ""),
            ],
        );
        let mut m = mob(Species::Warden);
        choose(&mut m, &t, &data(), Some(seen("minecraft:deep_dark")), 0);
        let spots = variant("entity/warden/warden_pulsating_spots_2.2");
        assert_eq!(
            texture(&m, &t, "entity/warden/warden_pulsating_spots_2"),
            Some(spots.as_str())
        );
    }

    #[test]
    fn a_texture_drawn_without_a_pick_is_chosen_next_tick() {
        let t = pack(
            "missed",
            &[
                ("sheep/sheep.properties", "textures.1=2"),
                ("sheep/sheep2.png", ""),
                ("cow/cow.properties", "textures.1=2"),
                ("cow/cow2.png", ""),
            ],
        );
        let mut m = mob(Species::Sheep);
        choose(&mut m, &t, &data(), Some(seen("minecraft:plains")), 0);
        assert!(m.random_textures.due(&t, 1), "checked, then 19 quiet ticks");
        assert!(!m.random_textures.due(&t, 1));
        // The sheep never drew a cow texture: drawing one asks for a choice at once.
        assert_eq!(texture(&m, &t, "entity/cow/cow"), None);
        assert!(m.random_textures.due(&t, 1));
        assert!(!m.random_textures.due(&t, 1));
        // Still missing (never listed): no second choice until the regular check.
        assert_eq!(texture(&m, &t, "entity/cow/cow"), None);
        assert!(!m.random_textures.due(&t, 1));
        // 15 more quiet ticks, the regular check, then a miss is answered at once again.
        for _ in 0..15 {
            assert!(!m.random_textures.due(&t, 1));
        }
        assert!(m.random_textures.due(&t, 1));
        assert_eq!(texture(&m, &t, "entity/cow/cow"), None);
        assert!(m.random_textures.due(&t, 1));
    }

    #[test]
    fn a_layer_without_rules_follows_the_bodys_pick_or_miss() {
        // The wool's variants are in ETF's folder, which holds no rules, so it follows the body
        // (whose rules are in OptiFine's): `sheep_wool1.png` doesn't stand in for a miss.
        let t = pack(
            "follow",
            &[
                ("sheep/sheep.properties", "textures.1=2\nbiomes.1=desert"),
                ("sheep/sheep2.png", ""),
                (
                    "assets/minecraft/etf/random/entity/sheep/sheep_wool1.png",
                    "",
                ),
                (
                    "assets/minecraft/etf/random/entity/sheep/sheep_wool2.png",
                    "",
                ),
            ],
        );
        let mut plains = mob(Species::Sheep);
        choose(&mut plains, &t, &data(), Some(seen("minecraft:plains")), 0);
        assert_eq!(texture(&plains, &t, WOOL), None);
        let mut desert = mob(Species::Sheep);
        choose(&mut desert, &t, &data(), Some(seen("minecraft:desert")), 0);
        assert_eq!(
            texture(&desert, &t, WOOL),
            Some("minecraft:etf/random/entity/sheep/sheep_wool2")
        );
    }

    #[test]
    fn villager_layers_choose_for_themselves() {
        let t = pack(
            "villager",
            &[
                ("villager/villager2.png", ""),
                ("villager/type/plains2.png", ""),
                ("villager/type/plains3.png", ""),
            ],
        );
        let plains = "entity/villager/type/plains";
        let mut m = mob(Species::Villager);
        // Seed 2: the body 1 + 2 % 2 = 1 (itself), the plains layer its own 1 + 2 % 3 = 3.
        choose(&mut m, &t, &data(), Some(seen("minecraft:plains")), 2);
        assert_eq!(texture(&m, &t, "entity/villager/villager"), None);
        let plains3 = variant("entity/villager/type/plains3");
        assert_eq!(texture(&m, &t, plains), Some(plains3.as_str()));
        // Seed 3: the body's 2 would give plains2; the layer's own 1 + 3 % 3 = 1 is itself.
        choose(&mut m, &t, &data(), None, 3);
        let villager2 = variant("entity/villager/villager2");
        assert_eq!(
            texture(&m, &t, "entity/villager/villager"),
            Some(villager2.as_str())
        );
        assert_eq!(texture(&m, &t, plains), None);
    }

    #[test]
    fn the_facts_a_mob_gives() {
        let mut slime = mob(Species::Slime);
        slime.extra.size = 4;
        assert_eq!(size(&slime), Some(3), "big: the docs' 3");
        slime.extra.size = 1;
        assert_eq!(size(&slime), Some(0));
        let mut phantom = mob(Species::Phantom);
        phantom.extra.size = 5;
        assert_eq!(size(&phantom), Some(5));
        assert_eq!(size(&mob(Species::Sheep)), None);
        let mut shulker = mob(Species::Shulker);
        shulker.extra.color = 16;
        assert_eq!(color(&shulker), None, "16: no colour");
        shulker.extra.color = 3;
        assert_eq!(color(&shulker), Some("light_blue"));
        // A librarian of level 3 matches `professions.1=librarian:3`.
        let t = pack(
            "librarian",
            &[
                (
                    "villager/villager.properties",
                    "textures.1=2\nprofessions.1=librarian:3",
                ),
                ("villager/villager2.png", ""),
            ],
        );
        let mut m = mob(Species::Villager);
        m.extra.variant2 = PROFESSIONS.iter().position(|&p| p == "librarian").unwrap() as u8;
        m.extra.variant3 = 3;
        choose(&mut m, &t, &data(), Some(seen("minecraft:plains")), 0);
        let villager2 = variant("entity/villager/villager2");
        assert_eq!(
            texture(&m, &t, "entity/villager/villager"),
            Some(villager2.as_str())
        );
        m.extra.variant3 = 2;
        choose(&mut m, &t, &data(), None, 0);
        assert_eq!(texture(&m, &t, "entity/villager/villager"), None);
    }

    #[test]
    fn seeds_from_uuid_ints() {
        let m = mob(Species::Sheep);
        // new UUID(1, 2).hashCode() = 3.
        assert_eq!(seed(&m, Some([0, 1, 0, 2])), 3);
        // The low int is unsigned: UUID(0x00000000ffffffff, 0).hashCode() = 0 ^ 0xffffffff.
        assert_eq!(seed(&m, Some([0, -1, 0, 0])), -1);
        assert_eq!(seed(&m, None), 1, "the entity id");
    }
}
