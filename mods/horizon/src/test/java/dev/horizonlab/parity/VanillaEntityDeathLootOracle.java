package dev.horizonlab.parity;

import com.mojang.authlib.GameProfile;
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
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.ReloadableServerRegistries;
import net.minecraft.server.ReloadableServerResources;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.tags.TagLoader;
import net.minecraft.util.RandomSource;
import net.minecraft.world.DifficultyInstance;
import net.minecraft.world.RandomSequences;
import net.minecraft.world.damagesource.DamageSource;
import net.minecraft.world.damagesource.DamageSources;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.DyeColor;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.entity.EntityTypeTest;
import net.minecraft.world.level.gamerules.GameRules;
import net.minecraft.world.level.levelgen.WorldGenSettings;
import net.minecraft.world.level.levelgen.WorldOptions;
import net.minecraft.world.phys.AABB;

/**
 * Original oracle: entity death drops. Each input line describes a victim, its
 * equipment and state, a killer and a damage source; the oracle creates the
 * real entities on a stand-in ServerLevel (whose addFreshEntity records the
 * spawned item entities and experience orbs) and runs LivingEntity.die's drop
 * step: the source entity's killedEntity hook, then dropAllDeathLoot (loot
 * table with the world's random sequence, dropCustomDeathLoot with the entity
 * random, dropEquipment, dropExperience). Loot table randoms come from a fresh
 * RandomSequences per case; each case kills `kills` identical victims in turn.
 */
public final class VanillaEntityDeathLootOracle {
    static Object unsafe;
    static Method allocate, putObject, objectFieldOffset;

    static Object alloc(Class<?> c) throws Exception { return allocate.invoke(unsafe, c); }

    static void put(Object target, Class<?> owner, String field, Object value) throws Exception {
        Field f = owner.getDeclaredField(field);
        long offset = (long) objectFieldOffset.invoke(unsafe, f);
        putObject.invoke(unsafe, target, offset, value);
    }

    static final class Lvl extends ServerLevel {
        Lvl() { super(null, null, null, null, null, null, false, 0L, List.of(), false); }
        MinecraftServer fake;
        RegistryAccess registries;
        GameRules rules;
        List<String> out;
        int next = 1;
        @Override public MinecraftServer getServer() { return fake; }
        @Override public RegistryAccess registryAccess() { return registries; }
        @Override public net.minecraft.world.Difficulty getDifficulty() { return net.minecraft.world.Difficulty.NORMAL; }
        @Override public long getGameTime() { return 0L; }
        @Override public int getNextEntityId() { return next++; }
        @Override public net.minecraft.world.flag.FeatureFlagSet enabledFeatures() { return net.minecraft.world.flag.FeatureFlags.REGISTRY.allFlags(); }
        @Override public GameRules getGameRules() { return rules; }
        @Override public boolean addFreshEntity(Entity entity) {
            if (entity instanceof ItemEntity item) out.add("drop " + stack(item.getItem()));
            else if (entity instanceof ExperienceOrb orb) out.add("orb " + orb.getValue());
            else out.add("entity " + BuiltInRegistries.ENTITY_TYPE.getKey(entity.getType()));
            return true;
        }
        @Override public <T extends Entity> List<T> getEntities(EntityTypeTest<Entity, T> type, AABB bb, Predicate<? super T> selector) { return new ArrayList<>(); }
        net.minecraft.world.attribute.EnvironmentAttributeSystem attributes;
        @Override public net.minecraft.world.attribute.EnvironmentAttributeSystem environmentAttributes() { return attributes; }
        @Override public int getSeaLevel() { return 63; }
        @Override public net.minecraft.world.entity.raid.Raid getRaidAt(BlockPos pos) { return null; }
        @Override public net.minecraft.world.level.BlockGetter getChunkForCollisions(int chunkX, int chunkZ) { return null; }
        @Override public List<net.minecraft.world.phys.shapes.VoxelShape> getEntityCollisions(Entity source, AABB area) { return List.of(); }
        @Override public DifficultyInstance getCurrentDifficultyAt(BlockPos pos) { return new DifficultyInstance(net.minecraft.world.Difficulty.NORMAL, 0L, 0L, 0.0F); }
    }

    static final class OraclePlayer extends Player {
        OraclePlayer(Level level) { super(level, new GameProfile(new UUID(1L, 2L), "oracle")); }
        @Override public GameType gameMode() { return GameType.SURVIVAL; }
        @Override public net.minecraft.world.item.component.ResolvableProfile getProfile() { return net.minecraft.world.item.component.ResolvableProfile.createResolved(this.getGameProfile()); }
    }

    static String stack(ItemStack stack) {
        StringBuilder line = new StringBuilder(BuiltInRegistries.ITEM.getKey(stack.getItem()) + " " + stack.getCount());
        if (stack.getDamageValue() != 0) line.append(" damage=").append(stack.getDamageValue());
        var enchantments = stack.getOrDefault(DataComponents.ENCHANTMENTS, net.minecraft.world.item.enchantment.ItemEnchantments.EMPTY);
        for (var e : enchantments.entrySet()) line.append(" ench=").append(e.getKey().unwrapKey().get().identifier()).append(':').append(e.getIntValue());
        var potion = stack.get(DataComponents.POTION_CONTENTS);
        if (potion != null && potion.potion().isPresent()) line.append(" potion=").append(potion.potion().get().unwrapKey().get().identifier());
        var ominous = stack.get(DataComponents.OMINOUS_BOTTLE_AMPLIFIER);
        if (ominous != null) line.append(" ominous=").append(ominous.value());
        return line.toString();
    }

    static Map<String, String> parse(String line) {
        Map<String, String> m = new LinkedHashMap<>();
        for (String token : line.trim().split("\\s+")) {
            int eq = token.indexOf('=');
            if (eq < 0) m.put("name", token); else m.put(token.substring(0, eq), token.substring(eq + 1));
        }
        return m;
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
        var loaded = ReloadableServerRegistries.reload(worldLayers, staticTags, resources, Runnable::run).join();
        HolderLookup.Provider lookup = loaded.lookupWithUpdatedTags();
        var components = BuiltInRegistries.DATA_COMPONENT_INITIALIZERS.build(lookup);
        staticTags.forEach(Registry.PendingTags::apply);
        components.forEach(net.minecraft.core.component.DataComponentInitializers.PendingComponents::apply);
        RegistryAccess access = loaded.layers().compositeAccess();

        // A server that only answers reloadableRegistries, getRandomSequence and debug subscribers.
        MinecraftServer server = (MinecraftServer) alloc(net.minecraft.server.dedicated.DedicatedServer.class);
        put(server, MinecraftServer.class, "debugSubscribers", alloc(net.minecraft.util.debug.ServerDebugSubscribers.class));
        ReloadableServerResources managers = (ReloadableServerResources) alloc(ReloadableServerResources.class);
        put(managers, ReloadableServerResources.class, "fullRegistryHolder", new ReloadableServerRegistries.Holder(lookup));
        Class<?> holderRecord = Class.forName("net.minecraft.server.MinecraftServer$ReloadableResources");
        Constructor<?> rr = holderRecord.getDeclaredConstructors()[0];
        rr.setAccessible(true);
        put(server, MinecraftServer.class, "resources", rr.newInstance(null, managers));
        var recipes = new net.minecraft.world.item.crafting.RecipeManager(lookup);
        recipes.finalizeRecipeLoading(net.minecraft.world.flag.FeatureFlags.REGISTRY.allFlags());
        put(managers, ReloadableServerResources.class, "recipes", recipes);
        put(server, MinecraftServer.class, "scoreboard", new net.minecraft.server.ServerScoreboard(server));

        Lvl level = (Lvl) alloc(Lvl.class);
        level.fake = server;
        level.registries = access;
        put(level, ServerLevel.class, "server", server);
        level.attributes = net.minecraft.world.attribute.EnvironmentAttributeSystem.builder().build();
        level.rules = new GameRules(net.minecraft.world.flag.FeatureFlags.REGISTRY.allFlags());
        put(level, Level.class, "damageSources", new DamageSources(access));
        put(level, Level.class, "random", RandomSource.create(0L));
        Method dropAll = LivingEntity.class.getDeclaredMethod("dropAllDeathLoot", ServerLevel.class, DamageSource.class);
        dropAll.setAccessible(true);
        var enchantments = access.lookupOrThrow(Registries.ENCHANTMENT);

        if (args.length > 2 && args[2].equals("facts")) {
            // Per-type facts the death step reads (a debugging aid, not compared).
            Field xp = Mob.class.getDeclaredField("xpReward");
            xp.setAccessible(true);
            try (var w = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8))) {
                for (var type : BuiltInRegistries.ENTITY_TYPE) {
                    Entity e;
                    try { e = type.create(level, EntitySpawnReason.LOAD); } catch (Throwable t) { continue; }
                    if (!(e instanceof Mob mob)) continue;
                    StringBuilder sb = new StringBuilder(BuiltInRegistries.ENTITY_TYPE.getKey(type).toString());
                    sb.append(" monster=").append(e instanceof net.minecraft.world.entity.monster.Monster).append(" xp=").append(xp.get(mob))
                        .append(" baby=").append(mob.isBaby()).append(" class=").append(e.getClass().getSimpleName())
                        .append(" animal=").append(e instanceof net.minecraft.world.entity.animal.Animal)
                        .append(" agwater=").append(e instanceof net.minecraft.world.entity.animal.AgeableWaterCreature)
                        .append(" water=").append(e instanceof net.minecraft.world.entity.animal.fish.WaterAnimal)
                        .append(" ageable=").append(e instanceof AgeableMob);
                    mob.setBaby(true);
                    sb.append(" canbaby=").append(mob.isBaby());
                    for (EquipmentSlot slot : EquipmentSlot.VALUES) if (mob.getDropChances().byEquipment(slot) != 0.085F) sb.append(' ').append(slot.getName()).append('=').append(mob.getDropChances().byEquipment(slot));
                    w.println(sb);
                }
            }
            System.exit(0);
        }
        List<String> lines = Files.readAllLines(Path.of(args[0]));
        try (var w = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8))) {
            for (String raw : lines) {
                if (raw.isBlank() || raw.startsWith("#")) continue;
                Map<String, String> c = parse(raw);
                w.println("== " + c.get("name"));
                List<String> out = new ArrayList<>();
                level.out = out;
                try {
                    long worldSeed = Long.parseLong(c.getOrDefault("seed", "0"));
                    put(server, MinecraftServer.class, "worldGenSettings", new WorldGenSettings(new WorldOptions(worldSeed, false, false), null));
                    put(server, MinecraftServer.class, "randomSequences", new RandomSequences());
                    int kills = Integer.parseInt(c.getOrDefault("kills", "1"));
                    long rand = Long.parseLong(c.getOrDefault("rand", "0"));
                    for (int k = 0; k < kills; k++) {
                        out.add("kill " + k);
                        run(level, access, enchantments, c, rand + k, dropAll);
                    }
                } catch (Throwable t) {
                    Throwable cause = t instanceof InvocationTargetException ite ? ite.getCause() : t;
                    StringBuilder trace = new StringBuilder("error " + cause);
                    if (System.getenv("ORACLE_TRACE") != null) for (var frame : cause.getStackTrace()) trace.append("\n  at ").append(frame);
                    out.add(trace.toString());
                }
                out.forEach(w::println);
            }
        }
        System.exit(0);
    }

    static ItemStack item(HolderLookup.RegistryLookup<net.minecraft.world.item.enchantment.Enchantment> enchantments, String spec) {
        // id[;enchantment:level...]
        String[] parts = spec.split(";");
        ItemStack stack = new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse(parts[0])));
        for (int i = 1; i < parts.length; i++) {
            int colon = parts[i].lastIndexOf(':');
            String name = parts[i].substring(0, colon);
            if (!name.contains(":")) name = "minecraft:" + name;
            stack.enchant(enchantments.getOrThrow(ResourceKey.create(Registries.ENCHANTMENT, Identifier.parse(name))), Integer.parseInt(parts[i].substring(colon + 1)));
        }
        return stack;
    }

    static Entity create(Lvl level, String id) {
        EntityType<?> type = BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse(id));
        return type.create(level, EntitySpawnReason.LOAD);
    }

    @SuppressWarnings("unchecked")
    static void run(Lvl level, RegistryAccess access, HolderLookup.RegistryLookup<net.minecraft.world.item.enchantment.Enchantment> enchantments,
                    Map<String, String> c, long rand, Method dropAll) throws Exception {
        LivingEntity victim = (LivingEntity) create(level, c.get("type"));
        victim.setPos(0.5, 64.0, 0.5);
        if (c.containsKey("baby") && victim instanceof Mob mob) mob.setBaby(c.get("baby").equals("1"));
        if (c.getOrDefault("fire", "0").equals("1")) victim.setRemainingFireTicks(100);
        if (c.containsKey("size")) victim.getClass().getMethod("setSize", int.class, boolean.class).invoke(victim, Integer.parseInt(c.get("size")), true);
        if (c.containsKey("color") && victim instanceof net.minecraft.world.entity.animal.sheep.Sheep sheep) sheep.setColor(DyeColor.byName(c.get("color"), DyeColor.WHITE));
        if (c.getOrDefault("sheared", "0").equals("1") && victim instanceof net.minecraft.world.entity.animal.sheep.Sheep sheep) sheep.setSheared(true);
        if (c.containsKey("carried")) ((net.minecraft.world.entity.monster.Enderman) victim).setCarriedBlock(BuiltInRegistries.BLOCK.getValue(Identifier.parse(c.get("carried"))).defaultBlockState());
        if (c.getOrDefault("leader", "0").equals("1")) ((net.minecraft.world.entity.monster.PatrollingMonster) victim).setPatrolLeader(true);
        if (c.getOrDefault("captain", "0").equals("1"))
            victim.setItemSlot(EquipmentSlot.HEAD, net.minecraft.world.entity.raid.Raid.getOminousBannerInstance(access.lookupOrThrow(Registries.BANNER_PATTERN)));
        for (EquipmentSlot slot : EquipmentSlot.VALUES) {
            String spec = c.get(slot.getName());
            if (spec != null) victim.setItemSlot(slot, item(enchantments, spec));
            String chance = c.get("chance_" + slot.getName());
            if (chance != null && victim instanceof Mob mob) mob.setDropChance(slot, Float.parseFloat(chance));
        }
        if (c.containsKey("vehicle")) {
            Entity vehicle = create(level, c.get("vehicle"));
            vehicle.setPos(0.5, 64.0, 0.5);
            if (!victim.startRiding(vehicle, true, false)) throw new IllegalStateException("could not ride");
        }
        // Killer and damage source.
        Player player = null;
        LivingEntity killer = null;
        String killerId = c.getOrDefault("killer", "none");
        if (killerId.equals("player") || c.getOrDefault("pkill", "0").equals("1")) {
            player = new OraclePlayer(level);
            player.setPos(2.5, 64.0, 0.5);
            if (c.containsKey("luck")) player.getAttribute(Attributes.LUCK).setBaseValue(Double.parseDouble(c.get("luck")));
            if (killerId.equals("player") && c.containsKey("weapon")) player.setItemSlot(EquipmentSlot.MAINHAND, item(enchantments, c.get("weapon")));
        }
        if (killerId.equals("player")) killer = player;
        else if (!killerId.equals("none")) {
            killer = (LivingEntity) create(level, killerId);
            killer.setPos(2.5, 64.0, 0.5);
            if (c.containsKey("weapon")) killer.setItemSlot(EquipmentSlot.MAINHAND, item(enchantments, c.get("weapon")));
            if (c.containsKey("frog")) killer.setComponent(DataComponents.FROG_VARIANT,
                access.lookupOrThrow(Registries.FROG_VARIANT).getOrThrow(ResourceKey.create(Registries.FROG_VARIANT, Identifier.parse(c.get("frog")))));
            if (c.getOrDefault("charged", "0").equals("1")) {
                Field powered = net.minecraft.world.entity.monster.Creeper.class.getDeclaredField("DATA_IS_POWERED");
                powered.setAccessible(true);
                killer.getEntityData().set((net.minecraft.network.syncher.EntityDataAccessor<Boolean>) powered.get(null), true);
            }
        }
        if (player != null && c.getOrDefault("pkill", killerId.equals("player") ? "1" : "0").equals("1")) victim.setLastHurtByPlayer(player, 100);
        DamageSources sources = level.damageSources();
        DamageSource source = switch (c.getOrDefault("source", "attack")) {
            case "attack" -> killer == null ? sources.generic() : killer instanceof Player p ? sources.playerAttack(p) : sources.mobAttack(killer);
            case "arrow" -> {
                var arrow = new net.minecraft.world.entity.projectile.arrow.Arrow(EntityTypes.ARROW, level);
                arrow.setPos(1.5, 64.0, 0.5);
                yield sources.arrow(arrow, killer);
            }
            case "fireball" -> {
                var fireball = new net.minecraft.world.entity.projectile.hurtingprojectile.LargeFireball(EntityTypes.FIREBALL, level);
                fireball.setPos(1.5, 64.0, 0.5);
                yield sources.fireball(fireball, killer);
            }
            case "explosion" -> sources.explosion(killer, killer);
            case "lightning" -> sources.lightningBolt();
            case "fire" -> sources.onFire();
            default -> sources.generic();
        };
        victim.getRandom().setSeed(rand);
        Entity sourceEntity = source.getEntity();
        if (sourceEntity == null || sourceEntity.killedEntity(level, victim, source)) dropAll.invoke(victim, level, source);
    }
}
