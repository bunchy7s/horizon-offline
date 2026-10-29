//! Supported vanilla mob types: model layers, default textures, animation
//! family, sounds, baby dimensions and the per-class constants
//! (`getMaxHeadXRot`, `getAmbientSoundInterval`, `getWaterSlowDown`...).
use super::{Brain, Flags, anim::Anim};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Species {
    Tadpole,
    ZombieHorse,
    SkeletonHorse,
    Giant,
    Pig,
    Cow,
    Mooshroom,
    Sheep,
    Chicken,
    Rabbit,
    Horse,
    Donkey,
    Mule,
    Llama,
    TraderLlama,
    Goat,
    Panda,
    PolarBear,
    Fox,
    Wolf,
    Cat,
    Ocelot,
    Parrot,
    Bee,
    Turtle,
    Armadillo,
    Frog,
    Axolotl,
    Squid,
    GlowSquid,
    Cod,
    Salmon,
    TropicalFish,
    Pufferfish,
    Dolphin,
    Bat,
    Villager,
    WanderingTrader,
    IronGolem,
    SnowGolem,
    CopperGolem,
    Zombie,
    Husk,
    Drowned,
    ZombieVillager,
    Skeleton,
    Stray,
    Bogged,
    Parched,
    WitherSkeleton,
    Creeper,
    Spider,
    CaveSpider,
    Enderman,
    Endermite,
    Silverfish,
    Slime,
    MagmaCube,
    Witch,
    Phantom,
    Pillager,
    Vindicator,
    Piglin,
    PiglinBrute,
    ZombifiedPiglin,
    Hoglin,
    Zoglin,
    Ghast,
    Blaze,
    Strider,
    Guardian,
    ElderGuardian,
    Shulker,
    Evoker,
    Illusioner,
    Vex,
    Ravager,
    Wither,
    Camel,
    Allay,
    Breeze,
    Sniffer,
    HappyGhast,
    Warden,
    Creaking,
    CamelHusk,
    Nautilus,
    ZombieNautilus,
    SulfurCube,
    Mannequin,
}
/// Baby bounding boxes (each class's BABY_DIMENSIONS).
#[derive(Clone, Copy, Debug)]
pub enum Baby {
    /// Not an ageable mob.
    None,
    /// EntityDimensions.scalable(w, h).withEyeHeight(e)
    Abs(f64, f64, f64),
    /// The adult dimensions scaled, with an optional explicit eye height.
    Scale(f64, Option<f64>),
}
pub struct Def {
    pub species: Species,
    pub name: &'static str,
    pub model: &'static str,
    pub texture: &'static str,
    pub anim: Anim,
    pub sounds: [&'static str; 3],
    pub step_sound: Option<&'static str>,
    pub baby: Baby,
}
macro_rules! def {
    ($s:ident, $name:literal, $model:literal, $tex:literal, $anim:expr, [$a:literal, $h:literal, $d:literal], $step:expr, $baby:expr) => {
        Def {
            species: Species::$s,
            name: $name,
            model: $model,
            texture: $tex,
            anim: $anim,
            sounds: [$a, $h, $d],
            step_sound: $step,
            baby: $baby,
        }
    };
}
use Anim as A;
pub const ALL: &[Def] = &[
    def!(
        Pig,
        "pig",
        "pig#main",
        "entity/pig/pig_temperate",
        A::Quadruped,
        ["entity.pig.ambient", "entity.pig.hurt", "entity.pig.death"],
        Some("entity.pig.step"),
        Baby::Abs(0.45, 0.45, 0.40625)
    ),
    def!(
        Cow,
        "cow",
        "cow#main",
        "entity/cow/cow_temperate",
        A::Quadruped,
        ["entity.cow.ambient", "entity.cow.hurt", "entity.cow.death"],
        Some("entity.cow.step"),
        Baby::Abs(0.45, 0.7, 0.69)
    ),
    def!(
        Mooshroom,
        "mooshroom",
        "mooshroom#main",
        "entity/cow/mooshroom_red",
        A::Quadruped,
        ["entity.cow.ambient", "entity.cow.hurt", "entity.cow.death"],
        Some("entity.cow.step"),
        Baby::Abs(0.45, 0.7, 0.69)
    ),
    def!(
        Sheep,
        "sheep",
        "sheep#main",
        "entity/sheep/sheep",
        A::Sheep,
        [
            "entity.sheep.ambient",
            "entity.sheep.hurt",
            "entity.sheep.death"
        ],
        Some("entity.sheep.step"),
        Baby::Abs(0.45, 0.65, 0.65625)
    ),
    def!(
        Chicken,
        "chicken",
        "chicken#main",
        "entity/chicken/chicken_temperate",
        A::Chicken,
        [
            "entity.chicken.ambient",
            "entity.chicken.hurt",
            "entity.chicken.death"
        ],
        Some("entity.chicken.step"),
        Baby::Abs(0.3, 0.4, 0.28125)
    ),
    def!(
        Rabbit,
        "rabbit",
        "rabbit#main",
        "entity/rabbit/rabbit_brown",
        A::Rabbit,
        [
            "entity.rabbit.ambient",
            "entity.rabbit.hurt",
            "entity.rabbit.death"
        ],
        None,
        Baby::Abs(0.24, 0.4, 0.39)
    ),
    def!(
        ZombieHorse,
        "zombie_horse",
        "zombie_horse#main",
        "entity/horse/horse_zombie",
        A::Horse,
        [
            "entity.zombie_horse.ambient",
            "entity.zombie_horse.hurt",
            "entity.zombie_horse.death"
        ],
        Some("entity.horse.step"),
        Baby::Scale(0.7, None)
    ),
    def!(
        SkeletonHorse,
        "skeleton_horse",
        "skeleton_horse#main",
        "entity/horse/horse_skeleton",
        A::Horse,
        [
            "entity.skeleton_horse.ambient",
            "entity.skeleton_horse.hurt",
            "entity.skeleton_horse.death"
        ],
        Some("entity.horse.step"),
        Baby::Scale(0.7, None)
    ),
    def!(
        Giant,
        "giant",
        "giant#main",
        "entity/zombie/zombie",
        A::Humanoid { zombie_arms: true },
        ["", "entity.generic.hurt", "entity.generic.death"],
        None,
        Baby::None
    ),
    def!(
        Horse,
        "horse",
        "horse#main",
        "entity/horse/horse_brown",
        A::Horse,
        [
            "entity.horse.ambient",
            "entity.horse.hurt",
            "entity.horse.death"
        ],
        Some("entity.horse.step"),
        Baby::Scale(0.7, None)
    ),
    def!(
        Donkey,
        "donkey",
        "donkey#main",
        "entity/horse/donkey",
        A::Horse,
        [
            "entity.donkey.ambient",
            "entity.donkey.hurt",
            "entity.donkey.death"
        ],
        Some("entity.horse.step"),
        Baby::Scale(0.5, None)
    ),
    def!(
        Mule,
        "mule",
        "mule#main",
        "entity/horse/mule",
        A::Horse,
        [
            "entity.mule.ambient",
            "entity.mule.hurt",
            "entity.mule.death"
        ],
        Some("entity.horse.step"),
        Baby::Scale(0.5, None)
    ),
    def!(
        Llama,
        "llama",
        "llama#main",
        "entity/llama/llama_creamy",
        A::Llama,
        [
            "entity.llama.ambient",
            "entity.llama.hurt",
            "entity.llama.death"
        ],
        Some("entity.llama.step"),
        Baby::Scale(0.5, None)
    ),
    def!(
        TraderLlama,
        "trader_llama",
        "trader_llama#main",
        "entity/llama/llama_creamy",
        A::Llama,
        [
            "entity.llama.ambient",
            "entity.llama.hurt",
            "entity.llama.death"
        ],
        Some("entity.llama.step"),
        Baby::Scale(0.5, None)
    ),
    def!(
        Goat,
        "goat",
        "goat#main",
        "entity/goat/goat",
        A::Goat,
        [
            "entity.goat.ambient",
            "entity.goat.hurt",
            "entity.goat.death"
        ],
        Some("entity.goat.step"),
        Baby::Abs(0.45, 0.65, 0.59375)
    ),
    def!(
        Panda,
        "panda",
        "panda#main",
        "entity/panda/panda",
        A::Panda,
        [
            "entity.panda.ambient",
            "entity.panda.hurt",
            "entity.panda.death"
        ],
        Some("entity.panda.step"),
        Baby::Scale(0.5, Some(0.28125))
    ),
    def!(
        PolarBear,
        "polar_bear",
        "polar_bear#main",
        "entity/bear/polarbear",
        A::PolarBear,
        [
            "entity.polar_bear.ambient",
            "entity.polar_bear.hurt",
            "entity.polar_bear.death"
        ],
        Some("entity.polar_bear.step"),
        Baby::Abs(0.7, 0.7, 0.34375)
    ),
    def!(
        Fox,
        "fox",
        "fox#main",
        "entity/fox/fox",
        A::Fox,
        ["entity.fox.ambient", "entity.fox.hurt", "entity.fox.death"],
        None,
        Baby::Scale(0.6, Some(0.34375))
    ),
    def!(
        Wolf,
        "wolf",
        "wolf#main",
        "entity/wolf/wolf",
        A::Wolf,
        [
            "entity.wolf.ambient",
            "entity.wolf.hurt",
            "entity.wolf.death"
        ],
        Some("entity.wolf.step"),
        Baby::Abs(0.3, 0.425, 0.34375)
    ),
    def!(
        Cat,
        "cat",
        "cat#main",
        "entity/cat/cat_tabby",
        A::Feline,
        ["entity.cat.ambient", "entity.cat.hurt", "entity.cat.death"],
        None,
        Baby::Abs(0.3, 0.35, 0.34375)
    ),
    def!(
        Ocelot,
        "ocelot",
        "ocelot#main",
        "entity/cat/ocelot",
        A::Feline,
        [
            "entity.ocelot.ambient",
            "entity.ocelot.hurt",
            "entity.ocelot.death"
        ],
        None,
        Baby::Abs(0.3, 0.35, 0.34375)
    ),
    def!(
        Parrot,
        "parrot",
        "parrot#main",
        "entity/parrot/parrot_red_blue",
        A::Parrot,
        [
            "entity.parrot.ambient",
            "entity.parrot.hurt",
            "entity.parrot.death"
        ],
        Some("entity.parrot.step"),
        Baby::None
    ),
    def!(
        Bee,
        "bee",
        "bee#main",
        "entity/bee/bee",
        A::Bee,
        ["", "entity.bee.hurt", "entity.bee.death"],
        None,
        Baby::Scale(0.5, None)
    ),
    def!(
        Turtle,
        "turtle",
        "turtle#main",
        "entity/turtle/turtle",
        A::Turtle,
        [
            "entity.turtle.ambient_land",
            "entity.turtle.hurt",
            "entity.turtle.death"
        ],
        Some("entity.turtle.shamble"),
        Baby::Scale(0.3, None)
    ),
    def!(
        Armadillo,
        "armadillo",
        "armadillo#main",
        "entity/armadillo/armadillo",
        A::Armadillo,
        [
            "entity.armadillo.ambient",
            "entity.armadillo.hurt",
            "entity.armadillo.death"
        ],
        Some("entity.armadillo.step"),
        Baby::Scale(0.6, Some(0.21875))
    ),
    def!(
        Frog,
        "frog",
        "frog#main",
        "entity/frog/frog_temperate",
        A::Frog,
        [
            "entity.frog.ambient",
            "entity.frog.hurt",
            "entity.frog.death"
        ],
        Some("entity.frog.step"),
        Baby::None
    ),
    def!(
        Axolotl,
        "axolotl",
        "axolotl#main",
        "entity/axolotl/axolotl_lucy",
        A::Axolotl,
        [
            "entity.axolotl.idle_water",
            "entity.axolotl.hurt",
            "entity.axolotl.death"
        ],
        None,
        Baby::Abs(0.375, 0.21, 0.09375)
    ),
    def!(
        Squid,
        "squid",
        "squid#main",
        "entity/squid/squid",
        A::Squid,
        [
            "entity.squid.ambient",
            "entity.squid.hurt",
            "entity.squid.death"
        ],
        None,
        Baby::Abs(0.5, 0.5, 0.37)
    ),
    def!(
        GlowSquid,
        "glow_squid",
        "glow_squid#main",
        "entity/squid/glow_squid",
        A::Squid,
        [
            "entity.glow_squid.ambient",
            "entity.glow_squid.hurt",
            "entity.glow_squid.death"
        ],
        None,
        Baby::Abs(0.5, 0.5, 0.37)
    ),
    def!(
        Tadpole,
        "tadpole",
        "tadpole#main",
        "entity/tadpole/tadpole",
        A::Fish,
        ["", "entity.tadpole.hurt", "entity.tadpole.death"],
        None,
        Baby::None
    ),
    def!(
        Cod,
        "cod",
        "cod#main",
        "entity/fish/cod",
        A::Fish,
        ["entity.cod.ambient", "entity.cod.hurt", "entity.cod.death"],
        None,
        Baby::None
    ),
    def!(
        Salmon,
        "salmon",
        "salmon#main",
        "entity/fish/salmon",
        A::Fish,
        [
            "entity.salmon.ambient",
            "entity.salmon.hurt",
            "entity.salmon.death"
        ],
        None,
        Baby::None
    ),
    def!(
        TropicalFish,
        "tropical_fish",
        "tropical_fish_small#main",
        "entity/fish/tropical_a",
        A::Fish,
        [
            "entity.tropical_fish.ambient",
            "entity.tropical_fish.hurt",
            "entity.tropical_fish.death"
        ],
        None,
        Baby::None
    ),
    def!(
        Pufferfish,
        "pufferfish",
        "pufferfish_big#main",
        "entity/fish/pufferfish",
        A::Pufferfish,
        ["", "entity.puffer_fish.hurt", "entity.puffer_fish.death"],
        None,
        Baby::None
    ),
    def!(
        Dolphin,
        "dolphin",
        "dolphin#main",
        "entity/dolphin/dolphin",
        A::Dolphin,
        [
            "entity.dolphin.ambient_water",
            "entity.dolphin.hurt",
            "entity.dolphin.death"
        ],
        None,
        Baby::Scale(0.65, Some(0.093_75))
    ),
    def!(
        Bat,
        "bat",
        "bat#main",
        "entity/bat/bat",
        A::Bat,
        ["entity.bat.ambient", "entity.bat.hurt", "entity.bat.death"],
        None,
        Baby::None
    ),
    def!(
        Villager,
        "villager",
        "villager#main",
        "entity/villager/villager",
        A::Villager,
        [
            "entity.villager.ambient",
            "entity.villager.hurt",
            "entity.villager.death"
        ],
        None,
        Baby::Abs(0.49, 0.98, 0.63)
    ),
    def!(
        WanderingTrader,
        "wandering_trader",
        "wandering_trader#main",
        "entity/wandering_trader/wandering_trader",
        A::Villager,
        [
            "entity.wandering_trader.ambient",
            "entity.wandering_trader.hurt",
            "entity.wandering_trader.death"
        ],
        None,
        Baby::None
    ),
    def!(
        IronGolem,
        "iron_golem",
        "iron_golem#main",
        "entity/iron_golem/iron_golem",
        A::IronGolem,
        ["", "entity.iron_golem.hurt", "entity.iron_golem.death"],
        Some("entity.iron_golem.step"),
        Baby::None
    ),
    def!(
        SnowGolem,
        "snow_golem",
        "snow_golem#main",
        "entity/snow_golem/snow_golem",
        A::SnowGolem,
        [
            "entity.snow_golem.ambient",
            "entity.snow_golem.hurt",
            "entity.snow_golem.death"
        ],
        None,
        Baby::None
    ),
    def!(
        CopperGolem,
        "copper_golem",
        "copper_golem#main",
        "entity/copper_golem/copper_golem",
        A::CopperGolem,
        ["", "entity.copper_golem.hurt", "entity.copper_golem.death"],
        Some("entity.copper_golem.step"),
        Baby::None
    ),
    def!(
        Zombie,
        "zombie",
        "zombie#main",
        "entity/zombie/zombie",
        A::Zombie,
        [
            "entity.zombie.ambient",
            "entity.zombie.hurt",
            "entity.zombie.death"
        ],
        Some("entity.zombie.step"),
        Baby::Abs(0.49, 0.98, 0.775)
    ),
    def!(
        Husk,
        "husk",
        "husk#main",
        "entity/zombie/husk",
        A::Zombie,
        [
            "entity.husk.ambient",
            "entity.husk.hurt",
            "entity.husk.death"
        ],
        Some("entity.husk.step"),
        Baby::Abs(0.49, 0.98, 0.825)
    ),
    def!(
        Drowned,
        "drowned",
        "drowned#main",
        "entity/zombie/drowned",
        A::Drowned,
        [
            "entity.drowned.ambient",
            "entity.drowned.hurt",
            "entity.drowned.death"
        ],
        Some("entity.drowned.step"),
        Baby::Abs(0.49, 0.98, 0.775)
    ),
    def!(
        ZombieVillager,
        "zombie_villager",
        "zombie_villager#main",
        "entity/zombie_villager/zombie_villager",
        A::ZombieVillager,
        [
            "entity.zombie_villager.ambient",
            "entity.zombie_villager.hurt",
            "entity.zombie_villager.death"
        ],
        Some("entity.zombie_villager.step"),
        Baby::Abs(0.49, 0.98, 0.67)
    ),
    def!(
        Skeleton,
        "skeleton",
        "skeleton#main",
        "entity/skeleton/skeleton",
        A::Skeleton,
        [
            "entity.skeleton.ambient",
            "entity.skeleton.hurt",
            "entity.skeleton.death"
        ],
        Some("entity.skeleton.step"),
        Baby::None
    ),
    def!(
        Stray,
        "stray",
        "stray#main",
        "entity/skeleton/stray",
        A::Skeleton,
        [
            "entity.stray.ambient",
            "entity.stray.hurt",
            "entity.stray.death"
        ],
        Some("entity.stray.step"),
        Baby::None
    ),
    def!(
        Parched,
        "parched",
        "parched#main",
        "entity/skeleton/parched",
        A::Skeleton,
        [
            "entity.parched.ambient",
            "entity.parched.hurt",
            "entity.parched.death"
        ],
        Some("entity.parched.step"),
        Baby::None
    ),
    def!(
        Bogged,
        "bogged",
        "bogged#main",
        "entity/skeleton/bogged",
        A::Skeleton,
        [
            "entity.bogged.ambient",
            "entity.bogged.hurt",
            "entity.bogged.death"
        ],
        Some("entity.bogged.step"),
        Baby::None
    ),
    def!(
        WitherSkeleton,
        "wither_skeleton",
        "wither_skeleton#main",
        "entity/skeleton/wither_skeleton",
        A::Skeleton,
        [
            "entity.wither_skeleton.ambient",
            "entity.wither_skeleton.hurt",
            "entity.wither_skeleton.death"
        ],
        Some("entity.wither_skeleton.step"),
        Baby::None
    ),
    def!(
        Creeper,
        "creeper",
        "creeper#main",
        "entity/creeper/creeper",
        A::Creeper,
        ["", "entity.creeper.hurt", "entity.creeper.death"],
        None,
        Baby::None
    ),
    def!(
        Spider,
        "spider",
        "spider#main",
        "entity/spider/spider",
        A::Spider,
        [
            "entity.spider.ambient",
            "entity.spider.hurt",
            "entity.spider.death"
        ],
        Some("entity.spider.step"),
        Baby::None
    ),
    def!(
        CaveSpider,
        "cave_spider",
        "cave_spider#main",
        "entity/spider/cave_spider",
        A::Spider,
        [
            "entity.spider.ambient",
            "entity.spider.hurt",
            "entity.spider.death"
        ],
        Some("entity.spider.step"),
        Baby::None
    ),
    def!(
        Enderman,
        "enderman",
        "enderman#main",
        "entity/enderman/enderman",
        A::Enderman,
        [
            "entity.enderman.ambient",
            "entity.enderman.hurt",
            "entity.enderman.death"
        ],
        None,
        Baby::None
    ),
    def!(
        Endermite,
        "endermite",
        "endermite#main",
        "entity/endermite/endermite",
        A::Silverfish,
        [
            "entity.endermite.ambient",
            "entity.endermite.hurt",
            "entity.endermite.death"
        ],
        Some("entity.endermite.step"),
        Baby::None
    ),
    def!(
        Silverfish,
        "silverfish",
        "silverfish#main",
        "entity/silverfish/silverfish",
        A::Silverfish,
        [
            "entity.silverfish.ambient",
            "entity.silverfish.hurt",
            "entity.silverfish.death"
        ],
        Some("entity.silverfish.step"),
        Baby::None
    ),
    def!(
        Slime,
        "slime",
        "slime#main",
        "entity/slime/slime",
        A::Slime,
        ["", "entity.slime.hurt", "entity.slime.death"],
        None,
        Baby::None
    ),
    def!(
        MagmaCube,
        "magma_cube",
        "magma_cube#main",
        "entity/slime/magmacube",
        A::MagmaCube,
        ["", "entity.magma_cube.hurt", "entity.magma_cube.death"],
        None,
        Baby::None
    ),
    def!(
        Witch,
        "witch",
        "witch#main",
        "entity/witch/witch",
        A::Witch,
        [
            "entity.witch.ambient",
            "entity.witch.hurt",
            "entity.witch.death"
        ],
        None,
        Baby::None
    ),
    def!(
        Phantom,
        "phantom",
        "phantom#main",
        "entity/phantom/phantom",
        A::Phantom,
        [
            "entity.phantom.ambient",
            "entity.phantom.hurt",
            "entity.phantom.death"
        ],
        None,
        Baby::None
    ),
    def!(
        Evoker,
        "evoker",
        "evoker#main",
        "entity/illager/evoker",
        A::Illager,
        [
            "entity.evoker.ambient",
            "entity.evoker.hurt",
            "entity.evoker.death"
        ],
        None,
        Baby::None
    ),
    def!(
        Illusioner,
        "illusioner",
        "illusioner#main",
        "entity/illager/illusioner",
        A::Illager,
        [
            "entity.illusioner.ambient",
            "entity.illusioner.hurt",
            "entity.illusioner.death"
        ],
        None,
        Baby::None
    ),
    def!(
        Allay,
        "allay",
        "allay#main",
        "entity/allay/allay",
        A::Allay,
        [
            "entity.allay.ambient_without_item",
            "entity.allay.hurt",
            "entity.allay.death"
        ],
        None,
        Baby::None
    ),
    def!(
        Creaking,
        "creaking",
        "creaking#main",
        "entity/creaking/creaking",
        A::Creaking,
        [
            "entity.creaking.ambient",
            "entity.generic.hurt",
            "entity.creaking.death"
        ],
        Some("entity.creaking.step"),
        Baby::None
    ),
    def!(
        Camel,
        "camel",
        "camel#main",
        "entity/camel/camel",
        A::Camel,
        [
            "entity.camel.ambient",
            "entity.camel.hurt",
            "entity.camel.death"
        ],
        Some("entity.camel.step"),
        Baby::Abs(0.95, 1.4, 1.38)
    ),
    def!(
        HappyGhast,
        "happy_ghast",
        "happy_ghast#main",
        "entity/ghast/happy_ghast",
        A::HappyGhast,
        [
            "entity.happy_ghast.ambient",
            "entity.happy_ghast.hurt",
            "entity.happy_ghast.death"
        ],
        None,
        Baby::Scale(0.2375, Some(0.46875))
    ),
    def!(
        Sniffer,
        "sniffer",
        "sniffer#main",
        "entity/sniffer/sniffer",
        A::Sniffer,
        [
            "entity.sniffer.idle",
            "entity.sniffer.hurt",
            "entity.sniffer.death"
        ],
        Some("entity.sniffer.step"),
        Baby::Scale(0.5, None)
    ),
    def!(
        Breeze,
        "breeze",
        "breeze#main",
        "entity/breeze/breeze",
        A::Breeze,
        [
            "entity.breeze.idle_ground",
            "entity.breeze.hurt",
            "entity.breeze.death"
        ],
        None,
        Baby::None
    ),
    def!(
        Wither,
        "wither",
        "wither#main",
        "entity/wither/wither",
        A::Wither,
        [
            "entity.wither.ambient",
            "entity.wither.hurt",
            "entity.wither.death"
        ],
        None,
        Baby::None
    ),
    def!(
        Ravager,
        "ravager",
        "ravager#main",
        "entity/illager/ravager",
        A::Ravager,
        [
            "entity.ravager.ambient",
            "entity.ravager.hurt",
            "entity.ravager.death"
        ],
        Some("entity.ravager.step"),
        Baby::None
    ),
    def!(
        Vex,
        "vex",
        "vex#main",
        "entity/illager/vex",
        A::Vex,
        ["entity.vex.ambient", "entity.vex.hurt", "entity.vex.death"],
        None,
        Baby::None
    ),
    def!(
        Shulker,
        "shulker",
        "shulker#main",
        "entity/shulker/shulker",
        A::Shulker,
        [
            "entity.shulker.ambient",
            "entity.shulker.hurt",
            "entity.shulker.death"
        ],
        None,
        Baby::None
    ),
    def!(
        Guardian,
        "guardian",
        "guardian#main",
        "entity/guardian/guardian",
        A::Guardian,
        [
            "entity.guardian.ambient",
            "entity.guardian.hurt",
            "entity.guardian.death"
        ],
        None,
        Baby::None
    ),
    def!(
        ElderGuardian,
        "elder_guardian",
        "elder_guardian#main",
        "entity/guardian/guardian_elder",
        A::Guardian,
        [
            "entity.elder_guardian.ambient",
            "entity.elder_guardian.hurt",
            "entity.elder_guardian.death"
        ],
        None,
        Baby::None
    ),
    def!(
        Pillager,
        "pillager",
        "pillager#main",
        "entity/illager/pillager",
        A::Illager,
        [
            "entity.pillager.ambient",
            "entity.pillager.hurt",
            "entity.pillager.death"
        ],
        None,
        Baby::None
    ),
    def!(
        Vindicator,
        "vindicator",
        "vindicator#main",
        "entity/illager/vindicator",
        A::Illager,
        [
            "entity.vindicator.ambient",
            "entity.vindicator.hurt",
            "entity.vindicator.death"
        ],
        None,
        Baby::None
    ),
    def!(
        Piglin,
        "piglin",
        "piglin#main",
        "entity/piglin/piglin",
        A::Piglin,
        [
            "entity.piglin.ambient",
            "entity.piglin.hurt",
            "entity.piglin.death"
        ],
        Some("entity.piglin.step"),
        Baby::Abs(0.49, 0.98, 0.78)
    ),
    def!(
        PiglinBrute,
        "piglin_brute",
        "piglin_brute#main",
        "entity/piglin/piglin_brute",
        A::Piglin,
        [
            "entity.piglin_brute.ambient",
            "entity.piglin_brute.hurt",
            "entity.piglin_brute.death"
        ],
        Some("entity.piglin_brute.step"),
        Baby::None
    ),
    def!(
        ZombifiedPiglin,
        "zombified_piglin",
        "zombified_piglin#main",
        "entity/piglin/zombified_piglin",
        A::ZombifiedPiglin,
        [
            "entity.zombified_piglin.ambient",
            "entity.zombified_piglin.hurt",
            "entity.zombified_piglin.death"
        ],
        None,
        Baby::Abs(0.49, 0.98, 0.78)
    ),
    def!(
        Hoglin,
        "hoglin",
        "hoglin#main",
        "entity/hoglin/hoglin",
        A::Hoglin,
        [
            "entity.hoglin.ambient",
            "entity.hoglin.hurt",
            "entity.hoglin.death"
        ],
        Some("entity.hoglin.step"),
        Baby::Abs(0.75, 0.85, 0.625)
    ),
    def!(
        Zoglin,
        "zoglin",
        "zoglin#main",
        "entity/hoglin/zoglin",
        A::Hoglin,
        [
            "entity.zoglin.ambient",
            "entity.zoglin.hurt",
            "entity.zoglin.death"
        ],
        Some("entity.zoglin.step"),
        Baby::Abs(0.75, 0.85, 0.625)
    ),
    def!(
        Ghast,
        "ghast",
        "ghast#main",
        "entity/ghast/ghast",
        A::Ghast,
        [
            "entity.ghast.ambient",
            "entity.ghast.hurt",
            "entity.ghast.death"
        ],
        None,
        Baby::None
    ),
    def!(
        Blaze,
        "blaze",
        "blaze#main",
        "entity/blaze/blaze",
        A::Blaze,
        [
            "entity.blaze.ambient",
            "entity.blaze.hurt",
            "entity.blaze.death"
        ],
        None,
        Baby::None
    ),
    def!(
        Strider,
        "strider",
        "strider#main",
        "entity/strider/strider",
        A::Strider,
        [
            "entity.strider.ambient",
            "entity.strider.hurt",
            "entity.strider.death"
        ],
        Some("entity.strider.step"),
        Baby::Abs(0.45, 0.85, 0.4375)
    ),
    def!(
        Warden,
        "warden",
        "warden#main",
        "entity/warden/warden",
        A::Warden,
        [
            "entity.warden.ambient",
            "entity.warden.hurt",
            "entity.warden.death"
        ],
        None,
        Baby::None
    ),
    // After the camel: the camel husk reuses `camel#main`.
    def!(
        CamelHusk,
        "camel_husk",
        "camel#main",
        "entity/camel/camel_husk",
        A::Camel,
        [
            "entity.camel_husk.ambient",
            "entity.camel_husk.hurt",
            "entity.camel_husk.death"
        ],
        Some("entity.camel_husk.step"),
        Baby::None
    ),
    def!(
        Nautilus,
        "nautilus",
        "nautilus#main",
        "entity/nautilus/nautilus",
        A::Nautilus,
        [
            "entity.nautilus.ambient",
            "entity.nautilus.hurt",
            "entity.nautilus.death"
        ],
        None,
        Baby::Scale(0.5, None)
    ),
    def!(
        ZombieNautilus,
        "zombie_nautilus",
        "zombie_nautilus#main",
        "entity/nautilus/zombie_nautilus",
        A::Nautilus,
        [
            "entity.zombie_nautilus.ambient",
            "entity.zombie_nautilus.hurt",
            "entity.zombie_nautilus.death"
        ],
        None,
        Baby::None
    ),
    // Babies are size 1 and adults size 2: the size scales the dimensions.
    def!(
        SulfurCube,
        "sulfur_cube",
        "sulfur_cube#main",
        "entity/sulfur_cube/sulfur_cube_outer",
        A::Slime,
        ["", "entity.sulfur_cube.hurt", "entity.sulfur_cube.death"],
        None,
        Baby::None
    ),
    // Mannequin: the player model; the DATA_PROFILE ResolvableProfile picks
    // its skin, this texture only while that has none.
    def!(
        Mannequin,
        "mannequin",
        "player#main",
        "entity/player/wide/steve",
        A::Humanoid { zombie_arms: false },
        ["", "entity.generic.hurt", "entity.generic.death"],
        None,
        Baby::None
    ),
];
pub fn def(s: Species) -> &'static Def {
    ALL.iter().find(|d| d.species == s).unwrap_or(&ALL[0])
}
impl Def {
    /// Model layer families to load for this entity (adult, baby, variants).
    pub fn layer_prefixes(&self) -> Vec<&'static str> {
        let base = self.model.split('#').next().unwrap_or(self.name);
        let mut v = vec![base];
        let extra: &[&str] = match self.species {
            Species::Pig => &["pig_baby", "cold_pig"],
            Species::Mannequin => &["player_slim"],
            Species::Cow => &[
                "cow_baby",
                "cold_cow",
                "cold_cow_baby",
                "warm_cow",
                "warm_cow_baby",
            ],
            Species::Mooshroom => &["mooshroom_baby"],
            Species::Sheep => &["sheep_baby"],
            Species::Chicken => &["chicken_baby", "cold_chicken"],
            Species::Rabbit => &["rabbit_baby"],
            Species::Horse => &["horse_baby", "horse_armor"],
            Species::Donkey => &["donkey_baby"],
            Species::Mule => &["mule_baby"],
            Species::Llama => &["llama_baby"],
            Species::TraderLlama => &["trader_llama_baby", "llama", "llama_baby"],
            Species::Goat => &["goat_baby"],
            Species::Panda => &["panda_baby"],
            Species::PolarBear => &["polar_bear_baby"],
            Species::Fox => &["fox_baby"],
            Species::Wolf => &["wolf_baby", "wolf_armor"],
            Species::Sniffer => &["sniffer_baby"],
            Species::HappyGhast => &[
                "happy_ghast_baby",
                "happy_ghast_harness",
                "happy_ghast_ropes",
                "happy_ghast_baby_harness",
                "happy_ghast_baby_ropes",
            ],
            Species::Cat => &["cat_baby"],
            Species::Ocelot => &["ocelot_baby"],
            Species::Bee => &["bee_baby"],
            Species::Turtle => &["turtle_baby"],
            Species::Armadillo => &["armadillo_baby"],
            Species::Axolotl => &["axolotl_baby"],
            Species::Squid => &["squid_baby"],
            Species::GlowSquid => &["glow_squid_baby"],
            Species::Salmon => &["salmon_small", "salmon_large"],
            Species::TropicalFish => &["tropical_fish_large"],
            Species::Pufferfish => &["pufferfish_small", "pufferfish_medium"],
            Species::Dolphin => &["dolphin_baby"],
            Species::Villager => &["villager_baby", "villager_no_hat", "villager_baby_no_hat"],
            Species::Zombie => &["zombie_baby"],
            Species::Husk => &["husk_baby"],
            Species::Drowned => &["drowned_baby"],
            Species::ZombieVillager => &[
                "zombie_villager_baby",
                "zombie_villager_no_hat",
                "zombie_villager_baby_no_hat",
            ],
            Species::Piglin => &["piglin_baby"],
            Species::ZombifiedPiglin => &["zombified_piglin_baby"],
            Species::Hoglin => &["hoglin_baby"],
            Species::Zoglin => &["zoglin_baby"],
            Species::Strider => &["strider_baby"],
            Species::Camel => &["camel_baby"],
            Species::CamelHusk => &["camel_husk"],
            Species::Nautilus => &["nautilus_baby", "nautilus_armor"],
            Species::ZombieNautilus => &["zombie_nautilus_coral", "nautilus_armor", "nautilus"],
            Species::SulfurCube => &["sulfur_cube_small"],
            Species::ZombieHorse => &["zombie_horse_baby", "undead_horse_armor"],
            Species::SkeletonHorse => &["skeleton_horse_baby", "undead_horse_armor"],
            _ => &[],
        };
        v.extend_from_slice(extra);
        v
    }
    pub fn flags(&self, fire_immune: bool) -> Flags {
        use Species as S;
        let s = self.species;
        let undead = matches!(
            s,
            S::Zombie
                | S::Husk
                | S::Drowned
                | S::ZombieVillager
                | S::Skeleton
                | S::Stray
                | S::Bogged
                | S::Parched
                | S::WitherSkeleton
                | S::ZombieHorse
                | S::SkeletonHorse
                | S::Phantom
                | S::ZombifiedPiglin
                | S::Zoglin
                | S::Wither
                | S::CamelHusk
                | S::ZombieNautilus
        );
        let aquatic = matches!(
            s,
            S::Squid
                | S::GlowSquid
                | S::Cod
                | S::Tadpole
                | S::Salmon
                | S::TropicalFish
                | S::Pufferfish
                | S::Dolphin
        );
        Flags {
            aquatic_swimmer: aquatic,
            fall_immune: matches!(
                s,
                S::Chicken
                    | S::Bat
                    | S::Parrot
                    | S::Bee
                    | S::Ghast
                    | S::HappyGhast
                    | S::Blaze
                    | S::Phantom
                    | S::IronGolem
                    | S::Cat
                    | S::Ocelot
                    | S::Slime
                    | S::MagmaCube
                    | S::Wither
                    | S::Allay
                    | S::Breeze
            ),
            flying: matches!(s, S::Ghast | S::Phantom | S::HappyGhast),
            climber: matches!(s, S::Spider | S::CaveSpider),
            lava_walker: s == S::Strider,
            omnidirectional: matches!(s, S::Bee | S::Parrot),
            fire_immune: fire_immune
                || matches!(
                    s,
                    S::Ghast
                        | S::Blaze
                        | S::MagmaCube
                        | S::Strider
                        | S::WitherSkeleton
                        | S::ZombifiedPiglin
                        | S::Zoglin
                        | S::Wither
                ),
            // The `burn_in_daylight` entity type tag.
            burns_in_day: matches!(
                s,
                S::Zombie
                    | S::ZombieHorse
                    | S::Drowned
                    | S::ZombieVillager
                    | S::Skeleton
                    | S::Stray
                    | S::Bogged
                    | S::WitherSkeleton
                    | S::Phantom
                    | S::ZombieNautilus
            ),
            undead,
            breathes_water: aquatic
                || undead
                || matches!(
                    s,
                    S::Axolotl
                        | S::Frog
                        | S::Turtle
                        | S::Guardian
                        | S::ElderGuardian
                        | S::IronGolem
                        | S::SnowGolem
                        | S::Ghast
                        | S::Blaze
                        | S::Nautilus
                ),
            // Nautilus.handleAirSupply: it dries out on land.
            needs_water: aquatic || s == S::Nautilus,
            water_sensitive: matches!(s, S::Enderman | S::Blaze | S::SnowGolem | S::Strider),
            animal: !matches!(s, S::Bat | S::SulfurCube | S::Mannequin)
                && !self.hostile_class()
                && !aquatic
                && !matches!(
                    s,
                    S::Villager | S::WanderingTrader | S::IronGolem | S::SnowGolem | S::CopperGolem
                ),
        }
    }
    fn hostile_class(&self) -> bool {
        use Species as S;
        matches!(
            self.species,
            S::Zombie
                | S::Husk
                | S::Drowned
                | S::ZombieVillager
                | S::Skeleton
                | S::Stray
                | S::Bogged
                | S::Parched
                | S::WitherSkeleton
                | S::Creeper
                | S::Spider
                | S::CaveSpider
                | S::Enderman
                | S::Endermite
                | S::Silverfish
                | S::Slime
                | S::MagmaCube
                | S::Witch
                | S::Phantom
                | S::Pillager
                | S::Vindicator
                | S::Piglin
                | S::PiglinBrute
                | S::ZombifiedPiglin
                | S::Hoglin
                | S::Zoglin
                | S::Ghast
                | S::Blaze
                | S::Guardian
                | S::ElderGuardian
                | S::Shulker
                | S::Evoker
                | S::Illusioner
                | S::Vex
                | S::Ravager
                | S::Wither
                | S::Warden
                | S::Breeze
                | S::Creaking
        )
    }
    /// The item tag Animal.isFood tests.
    pub fn food_tag(&self) -> Option<&'static str> {
        use Species as S;
        Some(match self.species {
            S::Pig => "pig_food",
            S::Cow | S::Mooshroom => "cow_food",
            S::Sheep => "sheep_food",
            S::Chicken => "chicken_food",
            S::Rabbit => "rabbit_food",
            S::Horse | S::Donkey | S::Mule => "horse_food",
            S::Llama | S::TraderLlama => "llama_food",
            S::Goat => "goat_food",
            S::Sniffer => "sniffer_food",
            S::HappyGhast => "happy_ghast_food",
            S::CamelHusk => "camel_husk_food",
            S::Nautilus | S::ZombieNautilus => "nautilus_food",
            S::SulfurCube => "sulfur_cube_food",
            S::Panda => "panda_food",
            S::Fox => "fox_food",
            S::Wolf => "wolf_food",
            S::Cat => "cat_food",
            S::Ocelot => "ocelot_food",
            S::Parrot => "parrot_food",
            S::Bee => "bee_food",
            S::Turtle => "turtle_food",
            S::Armadillo => "armadillo_food",
            S::Frog | S::Tadpole => "frog_food",
            S::Axolotl => "axolotl_food",
            S::Hoglin => "hoglin_food",
            S::Strider => "strider_food",
            S::Piglin => "piglin_food",
            _ => return None,
        })
    }
    pub fn tempt_tag(&self) -> Option<&'static str> {
        Some(match self.species {
            Species::Horse | Species::Donkey | Species::Mule => "horse_tempt_items",
            Species::Llama | Species::TraderLlama => "llama_tempt_items",
            Species::Strider => "strider_tempt_items",
            // An adult sulfur cube follows what it can swallow.
            Species::SulfurCube => "sulfur_cube_swallowable",
            _ => return None,
        })
    }
    pub fn water_slowdown(&self) -> f32 {
        match self.species {
            Species::PolarBear => 0.98,
            // SkeletonHorse.getWaterSlowDown.
            Species::SkeletonHorse => 0.96,
            _ => 0.8,
        }
    }
    pub fn air_speed(&self) -> f32 {
        match self.species {
            Species::Phantom => 0.2,
            Species::HappyGhast => super::happy_ghast::AIR_SPEED,
            _ => 0.02,
        }
    }
    pub fn baby_scale(&self) -> f64 {
        match self.baby {
            Baby::Scale(s, _) => s,
            _ => 0.5,
        }
    }
    pub fn head_rot_speed(&self) -> f32 {
        match self.species {
            Species::Frog => 35.0,
            Species::Breeze => 25.0,
            _ => 10.0,
        }
    }
    pub fn max_head_x_rot(&self) -> f32 {
        match self.species {
            Species::Dolphin | Species::Axolotl => 1.0,
            Species::Slime | Species::MagmaCube | Species::SulfurCube => 0.0,
            Species::Guardian | Species::ElderGuardian | Species::Shulker => 180.0,
            _ => 40.0,
        }
    }
    pub fn max_head_y_rot(&self) -> f32 {
        match self.species {
            Species::Dolphin | Species::Axolotl => 1.0,
            Species::Frog => 5.0,
            Species::Breeze => 30.0,
            Species::Sniffer => 50.0,
            Species::Goat => 15.0,
            Species::Armadillo => 32.0,
            Species::Ravager => 45.0,
            Species::Camel | Species::CamelHusk => 30.0,
            Species::Shulker => 180.0,
            _ => 75.0,
        }
    }
    pub fn ambient_interval(&self) -> i32 {
        use Species as S;
        match self.species {
            S::Turtle => 200,
            // AbstractHorse.getAmbientSoundInterval, for every subclass.
            S::Horse
            | S::Donkey
            | S::Mule
            | S::ZombieHorse
            | S::SkeletonHorse
            | S::Llama
            | S::TraderLlama
            | S::Camel
            | S::CamelHusk => 400,
            S::Ocelot => 900,
            // Guardian.getAmbientSoundInterval (the elder too).
            S::Guardian | S::ElderGuardian => 160,
            // Hoglin extends Animal; Shulker extends AbstractGolem.
            S::Hoglin | S::Shulker => 120,
            // Allay is a PathfinderMob: the Mob default.
            S::Allay => 80,
            S::Squid
            | S::GlowSquid
            | S::Cod
            | S::Tadpole
            | S::Salmon
            | S::TropicalFish
            | S::Pufferfish
            | S::Dolphin
            | S::Axolotl
            | S::IronGolem
            | S::SnowGolem
            | S::CopperGolem => 120,
            _ if self.flags(false).animal => 120,
            _ => 80,
        }
    }
    pub fn brain(&self) -> Brain {
        match self.species {
            Species::Chicken => Brain::Chicken,
            Species::Zombie | Species::Husk | Species::Drowned | Species::ZombieVillager => {
                Brain::Zombie
            }
            Species::Skeleton | Species::Stray | Species::Bogged | Species::Parched => {
                Brain::Skeleton
            }
            Species::Spider | Species::CaveSpider => Brain::Spider,
            Species::Creeper => Brain::Creeper,
            _ => Brain::Passive,
        }
    }
}
/// Mobs whose `body` equipment slot Horizon keeps in `Extra::head` (a
/// happy ghast's harness, nautilus armour, a sulfur cube's swallowed block,
/// a wolf's armor).
pub fn body_slot(s: Species) -> bool {
    matches!(
        s,
        Species::HappyGhast
            | Species::Nautilus
            | Species::ZombieNautilus
            | Species::SulfurCube
            | Species::Wolf
    )
}
