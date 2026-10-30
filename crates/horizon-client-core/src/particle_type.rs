//! Particle types (`ParticleTypes`): the vanilla particle types with a client
//! implementation, by registry name, and their per-type flags.

macro_rules! particle_types {
    ($($variant:ident = $name:literal),* $(,)?) => {
        /// Vanilla particle types with a client implementation here.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum ParticleType { $($variant),* }
        impl ParticleType {
            pub const ALL: &'static [ParticleType] = &[$(ParticleType::$variant),*];
            /// Registry path (`minecraft:<name>`), also the sprite-set file name.
            pub fn name(self) -> &'static str {
                match self { $(ParticleType::$variant => $name),* }
            }
            pub fn from_name(name: &str) -> Option<Self> {
                let name = name.strip_prefix("minecraft:").unwrap_or(name);
                match name { $($name => Some(ParticleType::$variant),)* _ => None }
            }
        }
    };
}
particle_types! {
    Block = "block",
    BlockCrumble = "block_crumble",
    Trail = "trail",
    DustPillar = "dust_pillar",
    FallingDust = "falling_dust",
    Smoke = "smoke",
    LargeSmoke = "large_smoke",
    WhiteSmoke = "white_smoke",
    Flame = "flame",
    SoulFireFlame = "soul_fire_flame",
    CopperFireFlame = "copper_fire_flame",
    SmallFlame = "small_flame",
    Lava = "lava",
    DrippingWater = "dripping_water",
    FallingWater = "falling_water",
    DrippingLava = "dripping_lava",
    FallingLava = "falling_lava",
    LandingLava = "landing_lava",
    DrippingHoney = "dripping_honey",
    FallingHoney = "falling_honey",
    LandingHoney = "landing_honey",
    FallingNectar = "falling_nectar",
    FallingSporeBlossom = "falling_spore_blossom",
    SporeBlossomAir = "spore_blossom_air",
    DrippingObsidianTear = "dripping_obsidian_tear",
    FallingObsidianTear = "falling_obsidian_tear",
    LandingObsidianTear = "landing_obsidian_tear",
    DrippingDripstoneWater = "dripping_dripstone_water",
    FallingDripstoneWater = "falling_dripstone_water",
    DrippingDripstoneLava = "dripping_dripstone_lava",
    FallingDripstoneLava = "falling_dripstone_lava",
    Splash = "splash",
    Rain = "rain",
    Bubble = "bubble",
    SulfurBubbles = "sulfur_bubbles",
    NoxiousGas = "noxious_gas",
    NoxiousGasCloud = "noxious_gas_cloud",
    Geyser = "geyser",
    GeyserBase = "geyser_base",
    GeyserPoof = "geyser_poof",
    GeyserPlume = "geyser_plume",
    BubblePop = "bubble_pop",
    BubbleColumnUp = "bubble_column_up",
    CurrentDown = "current_down",
    Underwater = "underwater",
    Portal = "portal",
    ReversePortal = "reverse_portal",
    Enchant = "enchant",
    Nautilus = "nautilus",
    VaultConnection = "vault_connection",
    Crit = "crit",
    EnchantedHit = "enchanted_hit",
    DamageIndicator = "damage_indicator",
    Heart = "heart",
    AngryVillager = "angry_villager",
    HappyVillager = "happy_villager",
    Composter = "composter",
    Mycelium = "mycelium",
    Dolphin = "dolphin",
    EggCrack = "egg_crack",
    Poof = "poof",
    Explosion = "explosion",
    ExplosionEmitter = "explosion_emitter",
    CampfireCosySmoke = "campfire_cosy_smoke",
    CampfireSignalSmoke = "campfire_signal_smoke",
    Note = "note",
    Dust = "dust",
    TotemOfUndying = "totem_of_undying",
    Firework = "firework",
    EndRod = "end_rod",
    Ash = "ash",
    WhiteAsh = "white_ash",
    CrimsonSpore = "crimson_spore",
    WarpedSpore = "warped_spore",
    CherryLeaves = "cherry_leaves",
    PaleOakLeaves = "pale_oak_leaves",
    TintedLeaves = "tinted_leaves",
    Effect = "effect",
    InstantEffect = "instant_effect",
    EntityEffect = "entity_effect",
    Witch = "witch",
    Infested = "infested",
    Cloud = "cloud",
    Sneeze = "sneeze",
    Snowflake = "snowflake",
    Soul = "soul",
    SculkSoul = "sculk_soul",
    DragonBreath = "dragon_breath",
    Gust = "gust",
    SmallGust = "small_gust",
    GustEmitterLarge = "gust_emitter_large",
    GustEmitterSmall = "gust_emitter_small",
    DustColorTransition = "dust_color_transition",
    SculkCharge = "sculk_charge",
    SculkChargePop = "sculk_charge_pop",
    Shriek = "shriek",
    SonicBoom = "sonic_boom",
    Vibration = "vibration",
    SweepAttack = "sweep_attack",
    Glow = "glow",
    WaxOn = "wax_on",
    WaxOff = "wax_off",
    ElectricSpark = "electric_spark",
    Scrape = "scrape",
    SquidInk = "squid_ink",
    GlowSquidInk = "glow_squid_ink",
    Item = "item",
    ItemSlime = "item_slime",
    ItemSnowball = "item_snowball",
    ItemCobweb = "item_cobweb",
    SulfurCubeGoo = "sulfur_cube_goo",
    Flash = "flash",
    Firefly = "firefly",
    TrialSpawnerDetection = "trial_spawner_detection",
    TrialSpawnerDetectionOminous = "trial_spawner_detection_ominous",
    TrialOmen = "trial_omen",
    RaidOmen = "raid_omen",
    OminousSpawning = "ominous_spawning",
    Spit = "spit",
    Fishing = "fishing",
    DustPlume = "dust_plume",
    RedPoplarLeaves = "red_poplar_leaves",
    OrangePoplarLeaves = "orange_poplar_leaves",
    YellowPoplarLeaves = "yellow_poplar_leaves",
    PauseMobGrowth = "pause_mob_growth",
    ResetMobGrowth = "reset_mob_growth",
    BlockMarker = "block_marker",
}
impl ParticleType {
    /// `ParticleType.getOverrideLimiter` (`ParticleTypes.register(name, true)`):
    /// spawned regardless of distance.
    pub fn override_limiter(self) -> bool {
        matches!(
            self,
            Self::BlockMarker
                | Self::Geyser
                | Self::GeyserBase
                | Self::GeyserPoof
                | Self::GeyserPlume
                | Self::DamageIndicator
                | Self::ExplosionEmitter
                | Self::Explosion
                | Self::Gust
                | Self::GustEmitterLarge
                | Self::GustEmitterSmall
                | Self::SonicBoom
                | Self::SculkCharge
                | Self::SculkChargePop
                | Self::Vibration
                | Self::Poof
                | Self::Spit
                | Self::SquidInk
                | Self::SweepAttack
                | Self::CampfireCosySmoke
                | Self::CampfireSignalSmoke
                | Self::GlowSquidInk
                | Self::Glow
                | Self::WaxOn
                | Self::WaxOff
                | Self::ElectricSpark
                | Self::Scrape
                | Self::TrialSpawnerDetection
                | Self::TrialSpawnerDetectionOminous
                | Self::VaultConnection
                | Self::OminousSpawning
        )
    }
    /// `SingleQuadParticle.Layer.translucent`: types whose vanilla particle class
    /// hardcodes the translucent layer, drawn after translucent terrain.
    /// `TerrainParticle`, `BreakingItemParticle` and `BlockMarker` resolve their
    /// layer from the sprite instead (`Layer.bySprite`); see the `translucent`
    /// sprite set on the engine.
    pub fn translucent_layer(self) -> bool {
        match self {
            // CampfireSmokeParticle.
            Self::CampfireCosySmoke | Self::CampfireSignalSmoke
            // PlayerCloudParticle.
            | Self::Cloud | Self::Sneeze
            // SpellParticle.
            | Self::Effect | Self::InstantEffect | Self::EntityEffect
            | Self::Witch | Self::Infested | Self::TrialOmen | Self::RaidOmen
            // NoxiousGasParticle.
            | Self::NoxiousGas
            // EmissiveRisingParticle.
            | Self::Soul | Self::SculkSoul
            // SculkChargeParticle, SculkChargePopParticle, ShriekParticle,
            // VibrationSignalParticle, FireflyParticle, OverlayParticle.
            | Self::SculkCharge | Self::SculkChargePop | Self::Shriek
            | Self::Vibration | Self::Firefly | Self::Flash
            // SimpleAnimatedParticle and its subclasses: EndRodParticle,
            // SparkParticle (firework), SquidInkParticle, TotemParticle.
            | Self::EndRod | Self::Firework | Self::SquidInk | Self::GlowSquidInk
            | Self::TotemOfUndying
            // FlyTowardsPositionParticle: its LifetimeAlpha(0, 0.6, 0.25, 1) is not opaque.
            | Self::VaultConnection => true,
            _ => false,
        }
    }
}
