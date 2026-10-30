package dev.horizonlab.parity;

import java.io.PrintWriter;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.function.Predicate;
import net.minecraft.SharedConstants;
import net.minecraft.core.*;
import net.minecraft.core.component.DataComponents;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.sounds.SoundEvent;
import net.minecraft.sounds.SoundSource;
import net.minecraft.tags.TagLoader;
import net.minecraft.util.RandomSource;
import net.minecraft.world.Difficulty;
import net.minecraft.world.DifficultyInstance;
import net.minecraft.world.effect.MobEffectInstance;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.ai.Brain;
import net.minecraft.world.entity.ai.attributes.Attribute;
import net.minecraft.world.entity.ai.attributes.AttributeInstance;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.entity.ai.memory.MemoryModuleType;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.enchantment.ItemEnchantments;
import net.minecraft.world.level.*;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.BiomeManager;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.border.WorldBorder;
import net.minecraft.world.level.dimension.DimensionType;
import net.minecraft.world.level.entity.EntityTypeTest;
import net.minecraft.world.level.gamerules.GameRules;
import net.minecraft.world.level.levelgen.structure.BoundingBox;
import net.minecraft.world.level.levelgen.structure.Structure;
import net.minecraft.world.level.levelgen.structure.StructurePiece;
import net.minecraft.world.level.levelgen.structure.StructureStart;
import net.minecraft.world.level.levelgen.structure.pieces.PiecesContainer;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;

/**
 * Original oracle: Mob.finalizeSpawn as vanilla runs it, for every mob type,
 * spawn reason, difficulty and local difficulty, on a stand-in ServerLevel
 * (one biome, an optional structure piece, a floor and an optional roof,
 * chickens standing by, the moon phase).
 *
 * Each case seeds the level random (what finalizeSpawn reads through
 * level.getRandom()) and the entities' own randoms: every entity created
 * in the case, the case's mobs and the jockeys finalizeSpawn makes, gets
 * a LegacyRandomSource seeded eseed + its creation index, swapped in
 * through each EntityType's factory. A pack of mobs is finalized in turn
 * with the group data passed along, as NaturalSpawner does, each added to
 * the level with its passengers.
 *
 * The log lists every entity of the case in creation order (equipment and
 * enchantments, drop chances, CanPickUpLoot, LeftHanded, persistence, age,
 * health, attribute values, effects, the species' variant state and spawn
 * memories, its vehicle) and the next draws of the level random and of each
 * entity's random, which pin every draw in between.
 */
public final class VanillaFinalizeSpawnOracle {
    static Object unsafe;
    static Method allocate, putObject, objectFieldOffset;

    static Object alloc(Class<?> c) throws Exception { return allocate.invoke(unsafe, c); }

    static void put(Object target, Class<?> owner, String field, Object value) throws Exception {
        Field f = owner.getDeclaredField(field);
        putObject.invoke(unsafe, target, (long) objectFieldOffset.invoke(unsafe, f), value);
    }

    static Object get(Object target, Class<?> owner, String field) {
        try {
            Field f = owner.getDeclaredField(field);
            f.setAccessible(true);
            return f.get(target);
        } catch (Exception e) { throw new RuntimeException(e); }
    }

    static final class Lvl extends ServerLevel {
        Lvl() { super(null, null, null, null, null, null, false, 0L, List.of(), false); }
        RegistryAccess registries;
        net.minecraft.server.MinecraftServer fake;
        BlockState floor;
        int floorY, roofY;
        Holder<Biome> biome;
        Difficulty difficulty;
        DimensionType dimensionType;
        ResourceKey<Level> dimensionKey;
        List<Entity> entities = new ArrayList<>();
        net.minecraft.world.attribute.EnvironmentAttributeSystem attributes;
        BiomeManager biomeManager;
        WorldBorder border = new WorldBorder();
        GameRules rules;
        Structures structures;
        RandomSource levelRandom;
        long time;
        int next = 1;
        @Override public net.minecraft.server.MinecraftServer getServer() { return fake; }
        @Override public RegistryAccess registryAccess() { return registries; }
        @Override public Difficulty getDifficulty() { return difficulty; }
        @Override public long getGameTime() { return time; }
        @Override public int getNextEntityId() { return next++; }
        @Override public net.minecraft.world.flag.FeatureFlagSet enabledFeatures() { return net.minecraft.world.flag.FeatureFlags.REGISTRY.allFlags(); }
        @Override public BlockState getBlockState(BlockPos pos) {
            if (pos.getY() == floorY) return floor;
            if (pos.getY() == roofY) return Blocks.STONE.defaultBlockState();
            return Blocks.AIR.defaultBlockState();
        }
        @Override public FluidState getFluidState(BlockPos pos) { return getBlockState(pos).getFluidState(); }
        @Override public DimensionType dimensionType() { return dimensionType; }
        @Override public ResourceKey<Level> dimension() { return dimensionKey; }
        @Override public Holder<Biome> getBiome(BlockPos pos) { return biome; }
        @Override public BiomeManager getBiomeManager() { return biomeManager; }
        @Override public net.minecraft.world.attribute.EnvironmentAttributeSystem environmentAttributes() { return attributes; }
        @Override public WorldBorder getWorldBorder() { return border; }
        @Override public GameRules getGameRules() { return rules; }
        @Override public RandomSource getRandom() { return levelRandom; }
        @Override public int getSeaLevel() { return 63; }
        net.minecraft.server.ServerScoreboard scoreboard;
        @Override public net.minecraft.server.ServerScoreboard getScoreboard() { return scoreboard; }
        @Override public net.minecraft.world.level.StructureManager structureManager() { return structures; }
        @Override public BlockGetter getChunkForCollisions(int chunkX, int chunkZ) { return this; }
        @Override public boolean isSpawningMonsters() { return rules.get(GameRules.SPAWN_MOBS) && rules.get(GameRules.SPAWN_MONSTERS); }
        @Override public void playSeededSound(net.minecraft.world.entity.Entity except, double x, double y, double z, Holder<SoundEvent> sound, SoundSource source, float volume, float pitch, long seed) {}
        @Override public void playSeededSound(net.minecraft.world.entity.Entity except, net.minecraft.world.entity.Entity sourceEntity, Holder<SoundEvent> sound, SoundSource source, float volume, float pitch, long seed) {}
        @Override public void gameEvent(Holder<net.minecraft.world.level.gameevent.GameEvent> gameEvent, Vec3 position, net.minecraft.world.level.gameevent.GameEvent.Context context) {}
        @Override public void broadcastEntityEvent(Entity entity, byte event) {}
        @SuppressWarnings({"unchecked", "rawtypes"})
        @Override public List players() { return List.of(); }
        @Override public Iterable<Entity> getAllEntities() { return entities; }
        @Override public List<Entity> getEntities(Entity except, AABB bb, Predicate<? super Entity> selector) {
            List<Entity> out = new ArrayList<>();
            for (Entity e : entities) if (e != except && e.getBoundingBox().intersects(bb) && selector.test(e)) out.add(e);
            return out;
        }
        @Override public <T extends Entity> List<T> getEntities(EntityTypeTest<Entity, T> type, AABB bb, Predicate<? super T> selector) {
            List<T> out = new ArrayList<>();
            for (Entity e : entities) {
                T t = type.tryCast(e);
                if (t != null && e.getBoundingBox().intersects(bb) && selector.test(t)) out.add(t);
            }
            return out;
        }
        @Override public boolean addFreshEntity(Entity entity) { if (!entities.contains(entity)) entities.add(entity); return true; }
        @Override public void addFreshEntityWithPassengers(Entity entity) { entity.getSelfAndPassengers().forEach(this::addFreshEntity); }
        @Override public Player getNearestPlayer(double x, double y, double z, double maxDist, boolean filter) { return null; }
    }

    /** One structure start whose single piece covers a box around the spawn point. */
    static final class Structures extends net.minecraft.world.level.StructureManager {
        Structures() { super(null, null, null); }
        Lvl level;
        Structure structure;
        StructureStart start;
        @Override public List<StructureStart> startsForStructure(int sectionX, int sectionZ, Predicate<Structure> matcher) {
            return start != null && matcher.test(structure) ? List.of(start) : List.of();
        }
        @Override public List<StructureStart> startsForStructure(int sectionX, int sectionZ, Structure s) {
            return start != null && s == structure ? List.of(start) : List.of();
        }
        @Override public RegistryAccess registryAccess() { return level.registries; }
    }

    static final class Piece extends StructurePiece {
        Piece(BoundingBox box) { super(null, 0, box); }
        @Override protected void addAdditionalSaveData(net.minecraft.world.level.levelgen.structure.pieces.StructurePieceSerializationContext context, net.minecraft.nbt.CompoundTag tag) {}
        @Override public void postProcess(WorldGenLevel level, net.minecraft.world.level.StructureManager structureManager, net.minecraft.world.level.chunk.ChunkGenerator generator, RandomSource random,
            BoundingBox chunkBB, ChunkPos chunkPos, BlockPos referencePos) {}
    }

    /** Entities created in the current case, and the seed base of their randoms. */
    static List<Entity> created = new ArrayList<>();
    static long entitySeed;
    static boolean seeding;
    static int otherIds;

    @SuppressWarnings({"unchecked", "rawtypes"})
    static void wrapFactories() throws Exception {
        Field factory = EntityType.class.getDeclaredField("factory");
        long offset = (long) objectFieldOffset.invoke(unsafe, factory);
        factory.setAccessible(true);
        for (EntityType<?> type : BuiltInRegistries.ENTITY_TYPE) {
            EntityType.EntityFactory original = (EntityType.EntityFactory) factory.get(type);
            EntityType.EntityFactory wrapped = (t, level) -> {
                Entity e = original.create(t, level);
                if (e != null && !seeding) e.setId(1000000 + otherIds++);
                if (e != null && seeding) {
                    try {
                        put(e, Entity.class, "random", RandomSource.create(entitySeed + created.size()));
                    } catch (Exception ex) { throw new RuntimeException(ex); }
                    created.add(e);
                    e.setId(created.size());
                }
                return e;
            };
            putObject.invoke(unsafe, type, offset, wrapped);
        }
    }

    static String hex(double v) { return Long.toHexString(Double.doubleToRawLongBits(v)); }
    static String hex(float v) { return Integer.toHexString(Float.floatToRawIntBits(v)); }
    static String id(Holder<?> h) { return h.unwrapKey().map(k -> k.identifier().toString()).orElse("?"); }

    static final EquipmentSlot[] SLOTS = {EquipmentSlot.MAINHAND, EquipmentSlot.OFFHAND, EquipmentSlot.FEET, EquipmentSlot.LEGS,
        EquipmentSlot.CHEST, EquipmentSlot.HEAD, EquipmentSlot.BODY, EquipmentSlot.SADDLE};

    static String stack(ItemStack s) {
        StringBuilder b = new StringBuilder(BuiltInRegistries.ITEM.getKey(s.getItem()).toString());
        if (s.getCount() != 1) b.append('x').append(s.getCount());
        ItemEnchantments e = s.getOrDefault(DataComponents.ENCHANTMENTS, ItemEnchantments.EMPTY);
        if (!e.isEmpty()) {
            List<String> list = new ArrayList<>();
            for (var entry : e.entrySet()) list.add(id(entry.getKey()) + "=" + entry.getIntValue());
            list.sort(null);
            b.append('[').append(String.join(",", list)).append(']');
        }
        // Components besides enchantments (the ominous banner's).
        int other = s.getComponentsPatch().forget(t -> t == DataComponents.ENCHANTMENTS).size();
        if (other > 0) b.append('*').append(other);
        return b.toString();
    }

    /** Loaded after the bootstrap. */
    static final class Lists {
    static final List<Map.Entry<String, Holder<Attribute>>> ATTRIBUTES = List.of(
            Map.entry("max_health", Attributes.MAX_HEALTH), Map.entry("movement_speed", Attributes.MOVEMENT_SPEED),
            Map.entry("follow_range", Attributes.FOLLOW_RANGE), Map.entry("attack_damage", Attributes.ATTACK_DAMAGE),
            Map.entry("knockback_resistance", Attributes.KNOCKBACK_RESISTANCE), Map.entry("spawn_reinforcements", Attributes.SPAWN_REINFORCEMENTS_CHANCE),
            Map.entry("jump_strength", Attributes.JUMP_STRENGTH));

        static final List<Map.Entry<String, MemoryModuleType<?>>> MEMORIES = List.of(
            Map.entry("long_jump_cooling_down", MemoryModuleType.LONG_JUMP_COOLDOWN_TICKS), Map.entry("ram_cooldown_ticks", MemoryModuleType.RAM_COOLDOWN_TICKS),
            Map.entry("attack_target_cooldown", MemoryModuleType.ATTACK_TARGET_COOLDOWN), Map.entry("hunted_recently", MemoryModuleType.HUNTED_RECENTLY),
            Map.entry("dig_cooldown", MemoryModuleType.DIG_COOLDOWN), Map.entry("is_emerging", MemoryModuleType.IS_EMERGING),
            Map.entry("home", MemoryModuleType.HOME));
    }

    /** The species state finalizeSpawn sets. */
    static void variant(Entity e, List<String> v) {
        if (e instanceof net.minecraft.world.entity.animal.pig.Pig p) {
            v.add(id(p.getVariant()));
            v.add(id((Holder<?>) data(p, p.getClass(), "DATA_SOUND_VARIANT_ID")));
        } else if (e instanceof net.minecraft.world.entity.animal.cow.Cow c) {
            v.add(id(c.getVariant()));
            v.add(id((Holder<?>) data(c, net.minecraft.world.entity.animal.cow.Cow.class, "DATA_SOUND_VARIANT_ID")));
        } else if (e instanceof net.minecraft.world.entity.animal.cow.MushroomCow m) {
            v.add(m.getVariant().getSerializedName());
        } else if (e instanceof net.minecraft.world.entity.animal.chicken.Chicken c) {
            v.add(id(c.getVariant()));
            v.add(id((Holder<?>) data(c, c.getClass(), "DATA_SOUND_VARIANT_ID")));
            v.add("jockey=" + c.isChickenJockey());
        } else if (e instanceof net.minecraft.world.entity.animal.feline.Cat c) {
            v.add(id(c.getVariant()));
            v.add(id((Holder<?>) data(c, c.getClass(), "DATA_SOUND_VARIANT_ID")));
        } else if (e instanceof net.minecraft.world.entity.animal.wolf.Wolf w) {
            v.add(id((Holder<?>) call(w, net.minecraft.world.entity.animal.wolf.Wolf.class, "getVariant")));
            v.add(id((Holder<?>) data(w, w.getClass(), "DATA_SOUND_VARIANT_ID")));
        } else if (e instanceof net.minecraft.world.entity.animal.frog.Frog f) {
            v.add(id(f.getVariant()));
        } else if (e instanceof net.minecraft.world.entity.animal.nautilus.ZombieNautilus n) {
            v.add(id(n.getVariant()));
        } else if (e instanceof net.minecraft.world.entity.animal.sheep.Sheep s) {
            v.add(s.getColor().getSerializedName());
        } else if (e instanceof net.minecraft.world.entity.animal.equine.Horse h) {
            v.add(h.getVariant().getSerializedName() + "/" + h.getMarkings().name().toLowerCase(Locale.ROOT));
        } else if (e instanceof net.minecraft.world.entity.animal.equine.Llama l) {
            v.add(l.getVariant().getSerializedName() + "/" + l.getStrength());
        } else if (e instanceof net.minecraft.world.entity.animal.equine.SkeletonHorse h) {
            v.add("trap=" + h.isTrap());
        } else if (e instanceof net.minecraft.world.entity.animal.fox.Fox f) {
            v.add(f.getVariant().getSerializedName());
        } else if (e instanceof net.minecraft.world.entity.animal.rabbit.Rabbit r) {
            v.add(r.getVariant().getSerializedName());
        } else if (e instanceof net.minecraft.world.entity.animal.parrot.Parrot p) {
            v.add(p.getVariant().getSerializedName());
        } else if (e instanceof net.minecraft.world.entity.animal.axolotl.Axolotl a) {
            v.add(a.getVariant().getSerializedName());
        } else if (e instanceof net.minecraft.world.entity.animal.fish.TropicalFish f) {
            v.add(String.valueOf(call(f, net.minecraft.world.entity.animal.fish.TropicalFish.class, "getPackedVariant")));
        } else if (e instanceof net.minecraft.world.entity.animal.fish.Salmon s) {
            v.add(s.getVariant().getSerializedName());
        } else if (e instanceof net.minecraft.world.entity.animal.panda.Panda p) {
            v.add(p.getMainGene().getSerializedName() + "/" + p.getHiddenGene().getSerializedName());
        } else if (e instanceof net.minecraft.world.entity.animal.goat.Goat g) {
            v.add("scream=" + g.isScreamingGoat() + " horns=" + g.hasLeftHorn() + "," + g.hasRightHorn());
        } else if (e instanceof net.minecraft.world.entity.monster.cubemob.AbstractCubeMob s) {
            v.add("size=" + s.getSize());
        } else if (e instanceof net.minecraft.world.entity.npc.villager.Villager vi) {
            v.add(id(vi.getVillagerData().type()) + "/" + id(vi.getVillagerData().profession()) + "/" + vi.getVillagerData().level());
        } else if (e instanceof net.minecraft.world.entity.monster.zombie.ZombieVillager z) {
            // The profession comes from the constructor's unseeded random.
            v.add(id(z.getVillagerData().type()) + "/" + z.getVillagerData().level());
        } else if (e instanceof net.minecraft.world.entity.monster.Phantom p) {
            v.add("size=" + p.getPhantomSize());
        } else if (e instanceof net.minecraft.world.entity.animal.turtle.Turtle t) {
            BlockPos h = (BlockPos) get(t, net.minecraft.world.entity.animal.turtle.Turtle.class, "homePos");
            v.add("home=" + (h == null ? "-" : h.getX() + "," + h.getY() + "," + h.getZ()));
        } else if (e instanceof net.minecraft.world.entity.animal.dolphin.Dolphin d) {
            v.add("air=" + d.getAirSupply());
        }
        if (e instanceof net.minecraft.world.entity.monster.zombie.Zombie z) v.add("doors=" + z.canBreakDoors());
        if (e instanceof net.minecraft.world.entity.monster.PatrollingMonster p) v.add("leader=" + p.isPatrolLeader() + " patrolling=" + call(p, net.minecraft.world.entity.monster.PatrollingMonster.class, "isPatrolling"));
        if (e instanceof net.minecraft.world.entity.raid.Raider r) v.add("raid=" + r.canJoinRaid());
        if (e instanceof net.minecraft.world.entity.monster.Strider s) v.add("saddled=" + s.isSaddled());
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    static Object data(Entity e, Class<?> owner, String name) {
        return e.getEntityData().get((net.minecraft.network.syncher.EntityDataAccessor) fieldValue(owner, name));
    }

    static Object call(Object target, Class<?> owner, String name) {
        try {
            Method m = owner.getDeclaredMethod(name);
            m.setAccessible(true);
            return m.invoke(target);
        } catch (Exception ex) { throw new RuntimeException(ex); }
    }

    static Object fieldValue(Class<?> owner, String name) {
        try {
            Field f = owner.getDeclaredField(name);
            f.setAccessible(true);
            return f.get(null);
        } catch (Exception ex) { throw new RuntimeException(ex); }
    }

    static String describe(Entity e, List<Entity> all) {
        List<String> t = new ArrayList<>();
        t.add(BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString());
        Entity vehicle = e.getVehicle();
        t.add("veh=" + (vehicle == null ? "-" : Integer.toString(all.indexOf(vehicle))));
        if (e instanceof AgeableMob a) t.add("age=" + a.getAge());
        else if (e instanceof LivingEntity l) t.add("baby=" + l.isBaby());
        if (e instanceof LivingEntity l) t.add("hp=" + hex(l.getHealth()));
        if (e instanceof Mob m) {
            t.add("lh=" + m.isLeftHanded());
            t.add("loot=" + m.canPickUpLoot());
            t.add("persist=" + m.isPersistenceRequired());
            List<String> eq = new ArrayList<>();
            for (EquipmentSlot s : SLOTS) {
                ItemStack st = m.getItemBySlot(s);
                if (!st.isEmpty()) eq.add(s.getSerializedName() + ":" + stack(st));
            }
            t.add("eq=" + (eq.isEmpty() ? "-" : String.join(";", eq)));
            List<String> drops = new ArrayList<>();
            for (EquipmentSlot s : SLOTS) {
                float d = m.getDropChances().byEquipment(s);
                if (d != 0.085F) drops.add(s.getSerializedName() + ":" + hex(d));
            }
            t.add("drop=" + (drops.isEmpty() ? "-" : String.join(";", drops)));
        }
        if (e instanceof LivingEntity l) {
            List<String> attrs = new ArrayList<>();
            for (var a : Lists.ATTRIBUTES) {
                AttributeInstance inst = l.getAttribute(a.getValue());
                if (inst != null) attrs.add(a.getKey() + ":" + hex(inst.getValue()));
            }
            t.add("a=" + String.join(",", attrs));
            List<String> fx = new ArrayList<>();
            for (MobEffectInstance i : l.getActiveEffects()) fx.add(id(i.getEffect()) + ":" + i.getAmplifier() + ":" + i.getDuration());
            fx.sort(null);
            t.add("fx=" + (fx.isEmpty() ? "-" : String.join(",", fx)));
            Brain<?> brain = l.getBrain();
            List<String> mem = new ArrayList<>();
            for (var m : Lists.MEMORIES) {
                if (!brain.hasMemoryValue(m.getValue())) continue;
                Object value = brain.getMemory(m.getValue()).orElse(null);
                String text = value instanceof GlobalPos g ? g.pos().getX() + "," + g.pos().getY() + "," + g.pos().getZ() : String.valueOf(value);
                mem.add(m.getKey() + ":" + text + ":" + brain.getTimeUntilExpiry(m.getValue()));
            }
            if (!mem.isEmpty()) t.add("mem=" + String.join(",", mem));
        }
        List<String> v = new ArrayList<>();
        variant(e, v);
        if (!v.isEmpty()) t.add("v=" + String.join(" ", v).replace(' ', '_'));
        return String.join(" ", t);
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        var unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        unsafe = unsafeField.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        putObject = unsafe.getClass().getMethod("putObject", Object.class, long.class, Object.class);
        objectFieldOffset = unsafe.getClass().getMethod("objectFieldOffset", Field.class);
        var repo = ServerPacksSource.createVanillaTrustedRepository();
        repo.reload();
        repo.setSelected(List.of("vanilla"));
        var resources = new MultiPackResourceManager(PackType.SERVER_DATA, repo.openAllSelected());
        var layers = RegistryLayer.createRegistryAccess();
        var staticTags = TagLoader.loadTagsForExistingRegistries(resources, layers.getLayer(RegistryLayer.STATIC));
        var context = TagLoader.buildUpdatedLookups(layers.getAccessForLoading(RegistryLayer.WORLD), staticTags);
        RegistryAccess.Frozen world = RegistryDataLoader.load(resources, context, RegistryDataLoader.WORLD_REGISTRIES, Runnable::run).join();
        var worldLayers = layers.replaceFrom(RegistryLayer.WORLD, world);
        var loaded = net.minecraft.server.ReloadableServerRegistries.reload(worldLayers, staticTags, resources, Runnable::run).join();
        HolderLookup.Provider lookup = loaded.lookupWithUpdatedTags();
        var components = BuiltInRegistries.DATA_COMPONENT_INITIALIZERS.build(lookup);
        staticTags.forEach(Registry.PendingTags::apply);
        components.forEach(net.minecraft.core.component.DataComponentInitializers.PendingComponents::apply);
        RegistryAccess access = loaded.layers().compositeAccess();
        var biomes = access.lookupOrThrow(Registries.BIOME);
        var dimensions = access.lookupOrThrow(Registries.DIMENSION_TYPE);
        var blockLookup = access.lookupOrThrow(Registries.BLOCK);
        var structureLookup = access.lookupOrThrow(Registries.STRUCTURE);

        var server = (net.minecraft.server.MinecraftServer) alloc(net.minecraft.server.dedicated.DedicatedServer.class);
        put(server, net.minecraft.server.MinecraftServer.class, "debugSubscribers", alloc(net.minecraft.util.debug.ServerDebugSubscribers.class));
        Lvl level = (Lvl) alloc(Lvl.class);
        level.fake = server;
        put(level, ServerLevel.class, "server", server);
        level.registries = access;
        // Level.playSound draws a sound seed before the (ignored) seeded sound.
        put(level, net.minecraft.world.level.Level.class, "soundSeedGenerator", RandomSource.create(0L));
        level.entities = new ArrayList<>();
        level.scoreboard = new net.minecraft.server.ServerScoreboard(server);
        level.border = new WorldBorder();
        level.rules = new GameRules(net.minecraft.world.flag.FeatureFlags.REGISTRY.allFlags());
        level.biomeManager = new BiomeManager((x, y, z) -> level.biome, 0L) {
            @Override public Holder<Biome> getBiome(BlockPos pos) { return level.biome; }
            @Override public Holder<Biome> getBiome(int x, int y, int z) { return level.biome; }
        };
        Structures structures = (Structures) alloc(Structures.class);
        structures.level = level;
        level.structures = structures;
        wrapFactories();

        List<String> lines = Files.readAllLines(Path.of(args[0]));
        try (var w = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8))) {
            w.println("halloween " + net.minecraft.util.SpecialDates.isHalloween());
            for (String raw : lines) {
                if (raw.isBlank()) continue;
                Map<String, String> c = new LinkedHashMap<>();
                for (String token : raw.trim().split("\\s+")) {
                    int eq = token.indexOf('=');
                    if (eq < 0) c.put("name", token); else c.put(token.substring(0, eq), token.substring(eq + 1));
                }
                try {
                    level.dimensionKey = ResourceKey.create(Registries.DIMENSION, Identifier.parse(c.getOrDefault("dim", "minecraft:overworld")));
                    level.dimensionType = dimensions.getOrThrow(ResourceKey.create(Registries.DIMENSION_TYPE, level.dimensionKey.identifier())).value();
                    level.biome = biomes.getOrThrow(ResourceKey.create(Registries.BIOME, Identifier.parse(c.getOrDefault("biome", "minecraft:plains"))));
                    level.difficulty = Difficulty.byName(c.getOrDefault("difficulty", "normal"));
                    level.time = Long.parseLong(c.getOrDefault("time", "0"));
                    level.rules.set(GameRules.SPAWN_MONSTERS, !c.getOrDefault("monsters", "1").equals("0"), null);
                    int phase = Integer.parseInt(c.getOrDefault("phase", "0"));
                    level.attributes = net.minecraft.world.attribute.EnvironmentAttributeSystem.builder().addStaticLayers(level)
                        .addConstantLayer(net.minecraft.world.attribute.EnvironmentAttributeMap.builder()
                            .set(net.minecraft.world.attribute.EnvironmentAttributes.MOON_PHASE, MoonPhase.values()[phase]).build())
                        .build();
                    String[] p = c.get("pos").split(",");
                    double x = Double.parseDouble(p[0]), y = Double.parseDouble(p[1]), z = Double.parseDouble(p[2]);
                    BlockPos block = BlockPos.containing(x, y, z);
                    level.floorY = block.getY() - 1;
                    level.floor = net.minecraft.commands.arguments.blocks.BlockStateParser.parseForBlock(blockLookup, c.getOrDefault("floor", "minecraft:grass_block"), false).blockState();
                    level.roofY = c.containsKey("roof") ? block.getY() + Integer.parseInt(c.get("roof")) : Integer.MIN_VALUE;
                    structures.start = null;
                    if (c.containsKey("structure")) {
                        structures.structure = structureLookup.getValue(Identifier.parse(c.get("structure")));
                        BoundingBox box = new BoundingBox(block.getX() - 8, block.getY() - 8, block.getZ() - 8, block.getX() + 8, block.getY() + 8, block.getZ() + 8);
                        structures.start = new StructureStart(structures.structure, new ChunkPos(block.getX() >> 4, block.getZ() >> 4), 0,
                            new PiecesContainer(List.of(new Piece(box))));
                    }
                    level.entities.clear();
                    level.levelRandom = RandomSource.create(Long.parseLong(c.getOrDefault("seed", "0")));
                    put(level, Level.class, "random", level.levelRandom);
                    created = new ArrayList<>();
                    entitySeed = Long.parseLong(c.getOrDefault("eseed", "0"));
                    seeding = true;
                    // Chickens already standing by (zombie jockeys look for one).
                    int chickens = Integer.parseInt(c.getOrDefault("chickens", "0"));
                    for (int i = 0; i < chickens; i++) {
                        Entity chicken = EntityTypes.CHICKEN.create(level, EntitySpawnReason.LOAD);
                        chicken.setPos(x + i, y, z);
                        level.entities.add(chicken);
                    }
                    EntityType<?> type = BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse(c.get("type")));
                    EntitySpawnReason reason = EntitySpawnReason.valueOf(c.getOrDefault("reason", "NATURAL"));
                    float moon = DimensionType.MOON_BRIGHTNESS_PER_PHASE[phase];
                    DifficultyInstance difficulty = new DifficultyInstance(level.difficulty, level.time, Long.parseLong(c.getOrDefault("inhabited", "0")), moon);
                    int pack = Integer.parseInt(c.getOrDefault("pack", "1"));
                    SpawnGroupData group = null;
                    for (int i = 0; i < pack; i++) {
                        Entity e = type.create(level, reason);
                        if (e == null) break; // monsters do not exist in peaceful
                        if (!(e instanceof Mob mob)) throw new IllegalStateException("not a mob");
                        mob.snapTo(x, y, z, 0.0F, 0.0F);
                        group = mob.finalizeSpawn(level, difficulty, reason, group);
                        level.addFreshEntityWithPassengers(mob);
                    }
                    seeding = false;
                    List<String> out = new ArrayList<>();
                    for (Entity e : created) out.add(describe(e, created));
                    for (Entity e : created) out.add("rng " + e.getRandom().nextLong());
                    out.add("next " + level.levelRandom.nextLong());
                    w.println(c.get("name") + " " + String.join(" | ", out));
                } catch (Throwable t) {
                    seeding = false;
                    Throwable cause = t;
                    while ((cause instanceof InvocationTargetException || cause instanceof RuntimeException && cause.getCause() != null) && cause.getCause() != null) cause = cause.getCause();
                    StringBuilder line = new StringBuilder(c.get("name") + " error " + cause);
                    if (System.getenv("ORACLE_TRACE") != null) for (var frame : cause.getStackTrace()) line.append("\n  at ").append(frame);
                    w.println(line);
                }
            }
        }
        System.exit(0);
    }
}
