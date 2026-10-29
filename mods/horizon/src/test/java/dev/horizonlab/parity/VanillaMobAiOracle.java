package dev.horizonlab.parity;

import com.mojang.authlib.GameProfile;
import com.mojang.serialization.Lifecycle;
import io.netty.channel.embedded.EmbeddedChannel;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.net.Proxy;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collection;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.Set;
import java.util.UUID;
import java.util.function.BooleanSupplier;
import net.minecraft.SharedConstants;
import net.minecraft.SystemReport;
import net.minecraft.commands.Commands;
import net.minecraft.commands.arguments.blocks.BlockStateParser;
import net.minecraft.core.BlockPos;
import net.minecraft.core.GlobalPos;
import net.minecraft.core.MappedRegistry;
import net.minecraft.core.Registry;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.gametest.framework.GameTestServer;
import net.minecraft.gizmos.GizmoCollector;
import net.minecraft.gizmos.Gizmos;
import net.minecraft.network.Connection;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.PacketFlow;
import net.minecraft.network.protocol.game.ClientboundSoundEntityPacket;
import net.minecraft.network.protocol.game.ClientboundSoundPacket;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.Services;
import net.minecraft.server.WorldLoader;
import net.minecraft.server.WorldStem;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.level.progress.LoggingLevelLoadListener;
import net.minecraft.server.network.CommonListenerCookie;
import net.minecraft.server.notifications.EmptyNotificationService;
import net.minecraft.server.notifications.NotificationManager;
import net.minecraft.server.packs.repository.PackRepository;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.permissions.LevelBasedPermissionSet;
import net.minecraft.server.permissions.PermissionSet;
import net.minecraft.server.players.NameAndId;
import net.minecraft.server.players.PlayerList;
import net.minecraft.util.Util;
import net.minecraft.util.datafix.DataFixers;
import net.minecraft.util.debugchart.LocalSampleLogger;
import net.minecraft.util.debugchart.SampleLogger;
import net.minecraft.world.clock.WorldClocks;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntitySpawnReason;
import net.minecraft.world.entity.EntitySpawnRequest;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.monster.creaking.Creaking;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.entity.Mob;
import net.minecraft.world.entity.AgeableMob;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.npc.villager.Villager;
import net.minecraft.world.entity.ai.Brain;
import net.minecraft.world.entity.ai.behavior.BehaviorControl;
import net.minecraft.world.entity.ai.behavior.BlockPosTracker;
import net.minecraft.world.entity.ai.behavior.GateBehavior;
import net.minecraft.world.entity.ai.behavior.ShufflingList;
import net.minecraft.world.entity.ai.behavior.EntityTracker;
import net.minecraft.world.entity.ai.control.LookControl;
import net.minecraft.world.entity.ai.control.MoveControl;
import net.minecraft.world.entity.ai.goal.GoalSelector;
import net.minecraft.world.entity.ai.goal.WrappedGoal;
import net.minecraft.world.entity.ai.memory.MemoryModuleType;
import net.minecraft.world.entity.ai.memory.WalkTarget;
import net.minecraft.world.entity.ai.navigation.PathNavigation;
import net.minecraft.world.entity.ai.sensing.Sensor;
import net.minecraft.world.flag.FeatureFlagSet;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.level.DataPackConfig;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.LevelSettings;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.dimension.LevelStem;
import net.minecraft.world.level.gamerules.GameRules;
import net.minecraft.world.level.levelgen.WorldDimensions;
import net.minecraft.world.level.levelgen.WorldGenSettings;
import net.minecraft.world.level.levelgen.WorldOptions;
import net.minecraft.world.level.levelgen.presets.WorldPresets;
import net.minecraft.world.level.storage.LevelDataAndDimensions;
import net.minecraft.world.level.storage.LevelStorageSource;
import net.minecraft.world.level.storage.PrimaryLevelData;
import net.minecraft.world.phys.Vec3;

/**
 * Mob AI oracle: a real (headless, flat, structure-free) vanilla server level. Each scenario
 * builds blocks, places a target player and mobs with seeded random sources, then ticks the
 * mobs exactly as ServerLevel.tick does (despawn check, then tickNonPassenger) while the rest of
 * the level stands still (no block ticks, weather, spawning or random ticks), printing the
 * AI-visible state of every mob after each tick: position, motion, rotations, running goals,
 * targets, navigation, move/look/jump controls, brain activities and memories, and sounds.
 */
public final class VanillaMobAiOracle {
    static final List<Scenario> SCENARIOS = new ArrayList<>();
    static PrintWriter out;
    /** The temporary world directory, deleted before the oracle halts. */
    static Path UNIVERSE;
    static final List<String> SOUNDS = new ArrayList<>();

    /** Draws of a traced mob random: the caller frames of each draw (AI_TRACE=scenario:from-to). */
    static final class TracingRandom extends net.minecraft.world.level.levelgen.LegacyRandomSource {
        static int tick;
        static boolean on;
        final List<String> log;
        final int who;
        TracingRandom(long seed, List<String> log, int who) { super(seed); this.log = log; this.who = who; }
        @Override
        public int next(int bits) {
            int v = super.next(bits);
            if (on) {
                StringBuilder sb = new StringBuilder(tick + " trace " + who + " " + bits + "=" + Integer.toHexString(v) + ":");
                int n = 0;
                for (StackTraceElement e : new Throwable().getStackTrace()) {
                    String c = e.getClassName();
                    if (c.contains("RandomSource") || c.contains("TracingRandom") || c.contains("Mth")) continue;
                    sb.append(' ').append(c.substring(c.lastIndexOf('.') + 1)).append('.').append(e.getMethodName()).append(':').append(e.getLineNumber());
                    if (++n == 7) break;
                }
                log.add(sb.toString());
            }
            return v;
        }
    }
    static final List<String> TRACE = new ArrayList<>();
    /** The entities a scenario ticks, in order: its mobs, then what they spawned. */
    static final List<Entity> ENTS = new ArrayList<>();
    static final Map<Entity, Integer> LABELS = new java.util.IdentityHashMap<>();
    static final UUID PLAYER_UUID = new UUID(0x1234L, 0x5678L);

    static final class MobSpec {
        String type;
        double x, y, z;
        float yaw;
        long seed;
        boolean baby;
        List<String> flags = new ArrayList<>();
    }

    static final class Scenario {
        String name;
        int ticks;
        int difficulty = 2;
        long time = 6000;
        boolean rain, thunder;
        int originX, originZ;
        List<String[]> blocks = new ArrayList<>();
        double[] player;
        boolean creative;
        /** The tick from which the player faces turnYaw (head and body), else 0. */
        int turnTick = Integer.MAX_VALUE;
        float turnYaw;
        /** What the player holds in its main hand (an item id), or null. */
        String hand;
        List<MobSpec> mobs = new ArrayList<>();
        long levelSeed = 0;
        /** The day time advances with the game time (sleeping, waking). */
        boolean daycycle;
        /** Blocks changed before a given tick: (tick, block or fill line). */
        List<Object[]> timed = new ArrayList<>();
        /** Boxes whose block changes are printed every tick. */
        List<int[]> watches = new ArrayList<>();
        /** Blocks placed when the scenario starts, so their block entities start afresh and tick. */
        List<String[]> hearts = new ArrayList<>();
        /** ServerLevel.isSpawningMonsters during the scenario (a heart spawns creakings). */
        boolean spawning;
        /** The player hits a mob for 1 at the start of a tick: {tick, mob index}. */
        List<int[]> hurts = new ArrayList<>();
        /** The player feeds an animal, or uses another item on it, at the start of a tick
         *  (Animal.mobInteract with the item in its main hand): {tick, mob index, item id}. */
        List<Object[]> feeds = new ArrayList<>();
    }

    static void parse(Path path) throws Exception {
        Scenario s = null;
        for (String raw : Files.readAllLines(path)) {
            String line = raw.strip();
            if (line.isEmpty() || line.startsWith("#")) continue;
            String[] f = line.split("\\s+");
            switch (f[0]) {
                case "scenario" -> {
                    s = new Scenario();
                    s.name = f[1];
                    for (int i = 2; i < f.length; i++) {
                        String[] kv = f[i].split("=", 2);
                        switch (kv[0]) {
                            case "ticks" -> s.ticks = Integer.parseInt(kv[1]);
                            case "time" -> s.time = Long.parseLong(kv[1]);
                            case "difficulty" -> s.difficulty = Integer.parseInt(kv[1]);
                            case "rain" -> s.rain = true;
                            case "thunder" -> s.thunder = true;
                            case "levelseed" -> s.levelSeed = Long.parseLong(kv[1]);
                            case "daycycle" -> s.daycycle = true;
                            case "spawning" -> s.spawning = true;
                            default -> throw new IllegalArgumentException(line);
                        }
                    }
                }
                case "origin" -> { s.originX = Integer.parseInt(f[1]); s.originZ = Integer.parseInt(f[2]); }
                case "fill", "block", "chest" -> s.blocks.add(f);
                case "at" -> s.timed.add(new Object[] {Integer.parseInt(f[1]), java.util.Arrays.copyOfRange(f, 2, f.length)});
                case "watch" -> {
                    int[] box = new int[6];
                    for (int i = 0; i < 6; i++) box[i] = Integer.parseInt(f[i + 1]);
                    s.watches.add(box);
                }
                case "heart" -> s.hearts.add(f);
                case "hurt" -> s.hurts.add(new int[] {Integer.parseInt(f[1]), Integer.parseInt(f[2])});
                case "feed", "interact" -> s.feeds.add(new Object[] {Integer.parseInt(f[1]), Integer.parseInt(f[2]), f[3]});
                case "player" -> {
                    s.player = new double[] {Double.parseDouble(f[1]), Double.parseDouble(f[2]), Double.parseDouble(f[3])};
                    for (int i = 4; i < f.length; i++) {
                        if (f[i].equals("creative")) s.creative = true;
                        else if (f[i].startsWith("turn=")) {
                            String[] tv = f[i].substring(5).split(",");
                            s.turnTick = Integer.parseInt(tv[0]);
                            s.turnYaw = Float.parseFloat(tv[1]);
                        } else if (f[i].startsWith("hand=")) s.hand = f[i].substring(5);
                        else throw new IllegalArgumentException(line);
                    }
                }
                case "mob" -> {
                    MobSpec m = new MobSpec();
                    m.type = f[1];
                    m.x = Double.parseDouble(f[2]);
                    m.y = Double.parseDouble(f[3]);
                    m.z = Double.parseDouble(f[4]);
                    m.yaw = Float.parseFloat(f[5]);
                    m.seed = Long.parseLong(f[6]);
                    for (int i = 7; i < f.length; i++) {
                        if (f[i].equals("baby")) m.baby = true; else m.flags.add(f[i]);
                    }
                    s.mobs.add(m);
                }
                case "end" -> { SCENARIOS.add(s); s = null; }
                default -> throw new IllegalArgumentException(line);
            }
        }
    }

    public static void main(String[] args) throws Exception {
        parse(Path.of(args[0]));
        out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1])));
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        Util.startTimerHackThread();
        // The harness passes an empty world directory under its build directory.
        Path universe = args.length > 2 ? Path.of(args[2]) : Files.createTempDirectory("horizon-mob-ai");
        Files.createDirectories(universe);
        UNIVERSE = universe;
        LevelStorageSource.LevelStorageAccess access = LevelStorageSource.createDefault(universe).createAccess("aiworld");
        PackRepository packs = ServerPacksSource.createPackRepository(access);
        MinecraftServer.spin(thread -> Server.create(thread, access, packs));
    }

    static String traceSpec() { String v = System.getenv("AI_TRACE"); return v == null ? "" : v; }
    // Every mob random is traced unless AI_TRACE=0.
    static boolean tracing(Scenario s) { return !traceSpec().equals("0"); }
    static int traceFrom() { return 0; }
    static int traceTo() { return Integer.MAX_VALUE; }

    static String d(double v) { return Long.toHexString(Double.doubleToRawLongBits(v)); }
    static String f(float v) { return Integer.toHexString(Float.floatToRawIntBits(v)); }

    static Field field(Class<?> c, String name) throws Exception {
        for (Class<?> k = c; k != null; k = k.getSuperclass()) {
            try {
                Field fl = k.getDeclaredField(name);
                fl.setAccessible(true);
                return fl;
            } catch (NoSuchFieldException e) {
                // keep looking
            }
        }
        throw new NoSuchFieldException(name);
    }

    static Method method(Class<?> c, String name, Class<?>... types) throws Exception {
        for (Class<?> k = c; k != null; k = k.getSuperclass()) {
            try {
                Method m = k.getDeclaredMethod(name, types);
                m.setAccessible(true);
                return m;
            } catch (NoSuchMethodException e) {
                // keep looking
            }
        }
        throw new NoSuchMethodException(name);
    }

    static Object get(Object o, String name) throws Exception { return field(o.getClass(), name).get(o); }

    static final class Server extends MinecraftServer {
        static final FeatureFlagSet FEATURES = FeatureFlags.REGISTRY
            .allFlags()
            .subtract(FeatureFlagSet.of(FeatureFlags.REDSTONE_EXPERIMENTS, FeatureFlags.MINECART_IMPROVEMENTS));
        final LocalSampleLogger sampleLogger = new LocalSampleLogger(4);
        int scenario = 0;
        int phase = 0;
        int wait = 0;
        /** The light engine's post-update tasks queued after the scene was built. */
        final List<java.util.concurrent.CompletableFuture<?>> lit = new ArrayList<>();
        ServerPlayer player;
        int failures = 0;

        static Server create(Thread thread, LevelStorageSource.LevelStorageAccess access, PackRepository packs) {
            try {
                packs.reload();
                ArrayList<String> enabled = new ArrayList<>(packs.getAvailableIds());
                enabled.remove("vanilla");
                enabled.addFirst("vanilla");
                WorldDataConfiguration config = new WorldDataConfiguration(new DataPackConfig(enabled, List.of()), FEATURES);
                LevelSettings settings = new LevelSettings("AI Level", GameType.SURVIVAL, LevelSettings.DifficultySettings.DEFAULT, true, config);
                WorldLoader.PackConfig packConfig = new WorldLoader.PackConfig(packs, config, false, true);
                WorldLoader.InitConfig init = new WorldLoader.InitConfig(packConfig, Commands.CommandSelection.DEDICATED, LevelBasedPermissionSet.OWNER);
                WorldStem stem = Util.<WorldStem>blockUntilDone(executor -> WorldLoader.load(init, context -> {
                    Registry<LevelStem> none = new MappedRegistry<>(Registries.LEVEL_STEM, Lifecycle.stable()).freeze();
                    WorldDimensions dims = context.datapackWorldRegistries().lookupOrThrow(Registries.WORLD_PRESET)
                        .getOrThrow(WorldPresets.FLAT).value().createWorldDimensions();
                    WorldDimensions.Complete complete = dims.bake(none);
                    PrimaryLevelData data = new PrimaryLevelData(settings, complete.specialWorldProperty(), complete.lifecycle());
                    return new WorldLoader.DataLoadOutput<>(
                        new LevelDataAndDimensions.WorldDataAndGenSettings(data, new WorldGenSettings(new WorldOptions(0L, false, false), dims)),
                        complete.dimensionsRegistryAccess());
                }, WorldStem::new, Util.backgroundExecutor(), executor)).get();
                GameRules rules = new GameRules(FEATURES);
                rules.set(GameRules.SPAWN_MOBS, false, null);
                rules.set(GameRules.SPAWN_MONSTERS, false, null);
                rules.set(GameRules.SPAWN_PATROLS, false, null);
                rules.set(GameRules.SPAWN_PHANTOMS, false, null);
                rules.set(GameRules.SPAWN_WANDERING_TRADERS, false, null);
                rules.set(GameRules.SPAWN_WARDENS, false, null);
                rules.set(GameRules.ADVANCE_TIME, false, null);
                rules.set(GameRules.ADVANCE_WEATHER, false, null);
                rules.set(GameRules.RAIDS, false, null);
                Field nf = GameTestServer.class.getDeclaredField("NO_SERVICES");
                nf.setAccessible(true);
                return new Server(thread, access, packs, stem, rules, (Services) nf.get(null));
            } catch (Exception e) {
                e.printStackTrace();
                System.exit(2);
                throw new IllegalStateException(e);
            }
        }

        Server(Thread thread, LevelStorageSource.LevelStorageAccess access, PackRepository packs, WorldStem stem, GameRules rules, Services services) {
            super(thread, access, packs, stem, Optional.of(rules), Proxy.NO_PROXY, DataFixers.getDataFixer(), services,
                LoggingLevelLoadListener.forDedicatedServer(), false, new NotificationManager());
        }

        @Override
        protected boolean initServer() {
            this.setPlayerList(new PlayerList(this, this.registries(), this.playerDataStorage, new EmptyNotificationService()) {
                @Override
                public void broadcast(Player except, double x, double y, double z, double range, ResourceKey<Level> dim, Packet<?> packet) {
                    capture(packet);
                }
            });
            Gizmos.withCollector(GizmoCollector.NOOP);
            this.loadLevel();
            return true;
        }

        static void capture(Packet<?> packet) {
            if (packet instanceof ClientboundSoundPacket p) {
                SOUNDS.add(p.getSound().value().location() + " " + f(p.getVolume()) + " " + f(p.getPitch()));
            } else if (packet instanceof ClientboundSoundEntityPacket p) {
                SOUNDS.add(p.getSound().value().location() + " " + f(p.getVolume()) + " " + f(p.getPitch()));
            }
        }

        @Override
        protected void tickServer(BooleanSupplier haveTime) {
            try {
                step(haveTime);
            } catch (Throwable t) {
                t.printStackTrace();
                out.flush();
                System.exit(3);
            }
        }

        ServerLevel level() { return this.overworld(); }

        void step(BooleanSupplier haveTime) throws Exception {
            if (scenario >= SCENARIOS.size()) {
                out.close();
                // Nothing to save: skip the shutdown, but leave no world behind.
                try (var paths = Files.walk(UNIVERSE)) {
                    paths.sorted(java.util.Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
                } catch (Exception e) {
                    // best effort
                }
                Runtime.getRuntime().halt(0);
                return;
            }
            Scenario s = SCENARIOS.get(scenario);
            ServerLevel level = level();
            int cx = s.originX >> 4, cz = s.originZ >> 4;
            switch (phase) {
                case 0 -> {
                    for (int dx = -4; dx <= 4; dx++)
                        for (int dz = -4; dz <= 4; dz++)
                            level.setChunkForced(cx + dx, cz + dz, true);
                    phase = 1;
                    super.tickServer(haveTime);
                }
                case 1 -> {
                    super.tickServer(haveTime);
                    boolean ready = true;
                    for (int dx = -4; dx <= 4 && ready; dx++)
                        for (int dz = -4; dz <= 4 && ready; dz++)
                            ready = level.isPositionEntityTicking(new BlockPos((cx + dx) * 16, -60, (cz + dz) * 16));
                    if (ready) {
                        build(s, level);
                        // The scene's light updates reach the light thread through the chunk task
                        // dispatcher, where hasLightWork() can't see them: a mob's first sun-burn
                        // roll then reads stale light. A post-update task queued behind them on
                        // every chunk completes once they have all run.
                        var light = (net.minecraft.server.level.ThreadedLevelLightEngine) level.getChunkSource().getLightEngine();
                        lit.clear();
                        for (int dx = -4; dx <= 4; dx++)
                            for (int dz = -4; dz <= 4; dz++)
                                lit.add(light.waitForPendingTasks(cx + dx, cz + dz));
                        phase = 2;
                        wait = 0;
                    }
                }
                case 2 -> {
                    super.tickServer(haveTime);
                    if (lit.stream().allMatch(java.util.concurrent.CompletableFuture::isDone)
                        && !level.getChunkSource().getLightEngine().hasLightWork() && ++wait > 5) {
                        run(s, level);
                        for (int dx = -4; dx <= 4; dx++)
                            for (int dz = -4; dz <= 4; dz++)
                                level.setChunkForced(cx + dx, cz + dz, false);
                        scenario++;
                        phase = 0;
                    }
                }
                default -> throw new IllegalStateException();
            }
        }

        void build(Scenario s, ServerLevel level) throws Exception {
            for (String[] b : s.blocks) place(b, level);
        }

        static void place(String[] b, ServerLevel level) throws Exception {
            var blocks = level.holderLookup(Registries.BLOCK);
            {
                if (b[0].equals("block") || b[0].equals("chest")) {
                    BlockState state = BlockStateParser.parseForBlock(blocks, b[4], false).blockState();
                    BlockPos at = new BlockPos(Integer.parseInt(b[1]), Integer.parseInt(b[2]), Integer.parseInt(b[3]));
                    level.setBlock(at, state, 2 | 16);
                    // chest X Y Z STATE SLOTS: the slots' contents (item*count, - for empty).
                    if (b[0].equals("chest") && b.length > 5) {
                        var chest = (net.minecraft.world.level.block.entity.ChestBlockEntity) level.getBlockEntity(at);
                        String[] slots = b[5].split(",");
                        for (int k = 0; k < slots.length; k++) {
                            if (slots[k].equals("-")) continue;
                            String[] ic = slots[k].split("\\*");
                            chest.setItem(k, new net.minecraft.world.item.ItemStack(
                                BuiltInRegistries.ITEM.getValue(net.minecraft.resources.Identifier.parse(ic[0])), Integer.parseInt(ic[1])));
                        }
                    }
                } else {
                    BlockState state = BlockStateParser.parseForBlock(blocks, b[7], false).blockState();
                    int x0 = Integer.parseInt(b[1]), y0 = Integer.parseInt(b[2]), z0 = Integer.parseInt(b[3]);
                    int x1 = Integer.parseInt(b[4]), y1 = Integer.parseInt(b[5]), z1 = Integer.parseInt(b[6]);
                    for (int x = Math.min(x0, x1); x <= Math.max(x0, x1); x++)
                        for (int y = Math.min(y0, y1); y <= Math.max(y0, y1); y++)
                            for (int z = Math.min(z0, z1); z <= Math.max(z0, z1); z++)
                                level.setBlock(new BlockPos(x, y, z), state, 2 | 16);
                }
            }
        }

        void run(Scenario s, ServerLevel level) throws Exception {
            this.setDifficulty(net.minecraft.world.Difficulty.byId(s.difficulty), true);
            out.println("== " + s.name);
            var clock = level.registryAccess().lookupOrThrow(Registries.WORLD_CLOCK).getOrThrow(WorldClocks.OVERWORLD);
            this.clockManager().setTotalTicks(clock, s.time);
            // Every scenario starts at game time 1000.
            this.getWorldData().overworldData().setGameTime(1000L);
            // isRaining/isThundering read the rain and thunder levels only.
            level.setRainLevel(s.rain ? 1.0F : 0.0F);
            level.setThunderLevel(s.thunder ? 1.0F : 0.0F);
            if (s.player != null) {
                if (player == null) {
                    CommonListenerCookie cookie = CommonListenerCookie.createInitial(new GameProfile(PLAYER_UUID, "horizon-ai"), false);
                    player = new ServerPlayer(this, level, cookie.gameProfile(), cookie.clientInformation());
                    Connection connection = new Connection(PacketFlow.SERVERBOUND);
                    new EmbeddedChannel(connection);
                    this.getPlayerList().placeNewPlayer(connection, player, cookie);
                    // A client that has finished loading: otherwise the player stays invulnerable
                    // until 60 of the server ticks between scenarios have passed, so whether mobs can
                    // hurt it would depend on the scenario order.
                    method(player.connection.getClass(), "markClientLoaded").invoke(player.connection);
                }
                player.setGameMode(s.creative ? GameType.CREATIVE : GameType.SURVIVAL);
                player.setItemInHand(net.minecraft.world.InteractionHand.MAIN_HAND, s.hand == null ? net.minecraft.world.item.ItemStack.EMPTY
                    : new net.minecraft.world.item.ItemStack(BuiltInRegistries.ITEM.getValue(net.minecraft.resources.Identifier.parse(s.hand))));
                player.snapTo(s.player[0], s.player[1], s.player[2], 0.0F, 0.0F);
                // snapTo leaves the head and body as the player was created
                // (a random head yaw): face +z like the probe's player.
                player.setYHeadRot(0.0F);
                player.yHeadRotO = 0.0F;
                player.setYBodyRot(0.0F);
                // The player is never ticked during a scenario: give it the water state of
                // where it stands (isInWater), not the previous scenario's.
                method(Entity.class, "updateFluidInteraction").invoke(player);
                player.setDeltaMovement(Vec3.ZERO);
            } else if (player != null) {
                player.snapTo(s.originX + 5000, -60, s.originZ, 0.0F, 0.0F);
            }
            ENTS.clear();
            LABELS.clear();
            List<Brain<?>> brains = new ArrayList<>();
            List<Long> seeds = new ArrayList<>();
            int id = 100000;
            for (MobSpec spec : s.mobs) {
                EntityType<?> type = BuiltInRegistries.ENTITY_TYPE.getValue(net.minecraft.resources.Identifier.parse(spec.type));
                // Ignore canSpawn: a Peaceful scenario's hostile mob is one that was already
                // in the world when the difficulty changed (create returns null at Peaceful).
                Mob mob = (Mob) type.create(level, new EntitySpawnRequest(EntitySpawnReason.COMMAND, true));
                if (tracing(s)) field(Entity.class, "random").set(mob, new TracingRandom(0L, TRACE, ENTS.size()));
                mob.setId(id++);
                mob.snapTo(spec.x, spec.y, spec.z, spec.yaw, 0.0F);
                mob.setYHeadRot(spec.yaw);
                mob.setYBodyRot(spec.yaw);
                if (spec.baby) {
                    if (mob instanceof AgeableMob a) a.setBaby(true);
                    else mob.setBaby(true);
                }
                List<Runnable> afterAdd = new ArrayList<>();
                for (String flag : spec.flags) {
                    String[] kv = flag.split("=", 2);
                    String v = kv.length > 1 ? kv[1] : "";
                    switch (kv[0]) {
                        case "persistent" -> mob.setPersistenceRequired();
                        case "named" -> {
                            mob.setCustomName(net.minecraft.network.chat.Component.literal("Peaceful regression"));
                            mob.setPersistenceRequired();
                        }
                        case "ridden" -> mob.setItemSlot(net.minecraft.world.entity.EquipmentSlot.BODY,
                            new net.minecraft.world.item.ItemStack(BuiltInRegistries.ITEM.getValue(
                                net.minecraft.resources.Identifier.parse("minecraft:red_harness"))));
                        case "noai" -> mob.setNoAi(true);
                        case "profession" -> {
                            Villager villager = (Villager) mob;
                            villager.setVillagerData(villager.getVillagerData().withProfession(level.registryAccess(),
                                ResourceKey.create(Registries.VILLAGER_PROFESSION, net.minecraft.resources.Identifier.withDefaultNamespace(v))));
                            villager.refreshBrain(level);
                        }
                        case "level" -> {
                            Villager villager = (Villager) mob;
                            villager.setVillagerData(villager.getVillagerData().withLevel(Integer.parseInt(v)));
                        }
                        case "xp" -> ((Villager) mob).setVillagerXp(Integer.parseInt(v));
                        case "food" -> field(Villager.class, "foodLevel").setInt(mob, Integer.parseInt(v));
                        case "age" -> ((AgeableMob) mob).setAge(Integer.parseInt(v));
                        case "inv" -> {
                            for (String item : v.split(",")) {
                                String[] ic = item.split("\\*");
                                ((Villager) mob).getInventory().addItem(new net.minecraft.world.item.ItemStack(
                                    BuiltInRegistries.ITEM.getValue(net.minecraft.resources.Identifier.parse(ic[0])),
                                    ic.length > 1 ? Integer.parseInt(ic[1]) : 1));
                            }
                        }
                        case "gossip" -> {
                            // gossip=player:minor_negative:100
                            String[] g = v.split(":");
                            UUID target = g[0].equals("player") ? PLAYER_UUID : UUID.fromString(g[0]);
                            ((Villager) mob).getGossips().add(target,
                                net.minecraft.world.entity.ai.gossip.GossipType.valueOf(g[1].toUpperCase(java.util.Locale.ROOT)), Integer.parseInt(g[2]));
                        }
                        case "slept" -> {
                            long at = Long.parseLong(v);
                            afterAdd.add(() -> mob.getBrain().setMemory(MemoryModuleType.LAST_SLEPT, at));
                        }
                        // Warden.finalizeSpawn's own part (Mob.finalizeSpawn draws the level random,
                        // which is not seeded yet): a minute before digging, or climbing out first.
                        case "digcooldown", "emerge" -> {
                            mob.getBrain().setMemoryWithExpiry(MemoryModuleType.DIG_COOLDOWN, net.minecraft.util.Unit.INSTANCE, 1200L);
                            if (kv[0].equals("emerge")) {
                                mob.setPose(net.minecraft.world.entity.Pose.EMERGING);
                                mob.getBrain().setMemoryWithExpiry(MemoryModuleType.IS_EMERGING, net.minecraft.util.Unit.INSTANCE,
                                    net.minecraft.world.entity.monster.warden.WardenAi.EMERGE_DURATION);
                            }
                        }
                        case "inlove" -> ((net.minecraft.world.entity.animal.Animal) mob).setInLoveTime(600);
                        case "nectar" -> method(net.minecraft.world.entity.animal.bee.Bee.class, "setHasNectar", boolean.class).invoke(mob, true);
                        case "tame" -> { if (mob instanceof net.minecraft.world.entity.animal.equine.AbstractHorse horse) horse.setTamed(true); }
                        case "strength" -> method(net.minecraft.world.entity.animal.equine.Llama.class, "setStrength", int.class)
                            .invoke(mob, Integer.parseInt(v));
                        case "dead" -> mob.setHealth(0.0F);
                        case "leashed", "leashto" -> {
                            // Leashed once every mob exists (below).
                        }
                        case "hand" -> {
                            String[] ic = v.split("\\*");
                            mob.setItemSlot(net.minecraft.world.entity.EquipmentSlot.MAINHAND, new net.minecraft.world.item.ItemStack(
                                BuiltInRegistries.ITEM.getValue(net.minecraft.resources.Identifier.parse(ic[0])), Integer.parseInt(ic[1])));
                        }
                        case "cooldown" -> {
                            // Set after the seeded constructor draw (below).
                        }
                        default -> throw new IllegalArgumentException(flag);
                    }
                }
                level.addFreshEntity(mob);
                for (Runnable r : afterAdd) r.run();
                mob.getRandom().setSeed(spec.seed);
                Brain<?> brain = mob.getBrain();
                if (mob instanceof net.minecraft.world.entity.animal.golem.CopperGolem) {
                    // The constructor's TRANSPORT_ITEMS_COOLDOWN_TICKS draw, made with the seeded
                    // random; "cooldown=N" then sets it (0: none, transporting at once).
                    int cooldown = mob.getRandom().nextInt(60, 100);
                    for (String flag : spec.flags)
                        if (flag.startsWith("cooldown=")) cooldown = Integer.parseInt(flag.substring(9));
                    if (cooldown > 0) brain.setMemory(MemoryModuleType.TRANSPORT_ITEMS_COOLDOWN_TICKS, cooldown);
                    else brain.eraseMemory(MemoryModuleType.TRANSPORT_ITEMS_COOLDOWN_TICKS);
                }
                if (mob instanceof net.minecraft.world.entity.animal.bee.Bee bee) seedBee(bee);
                @SuppressWarnings("unchecked")
                Map<Object, Sensor<?>> sensors = (Map<Object, Sensor<?>>) get(brain, "sensors");
                for (Sensor<?> sensor : sensors.values()) sensor.randomlyDelayStart(mob.getRandom());
                seedGates(brain, spec.seed);
                LABELS.put(mob, ENTS.size());
                ENTS.add(mob);
                brains.add(brain);
                seeds.add(spec.seed);
            }
            // Leads: to the player ("leashed") or to the n-th mob ("leashto=n").
            for (int i = 0; i < s.mobs.size(); i++) {
                for (String flag : s.mobs.get(i).flags) {
                    if (flag.equals("ridden")) {
                        if (!player.startRiding(ENTS.get(i))) throw new IllegalStateException("Could not mount fixture");
                    }
                    if (flag.equals("leashed")) ((net.minecraft.world.entity.Leashable) ENTS.get(i)).setLeashedTo(player, true);
                    else if (flag.startsWith("leashto=")) ((net.minecraft.world.entity.Leashable) ENTS.get(i))
                        .setLeashedTo(ENTS.get(Integer.parseInt(flag.substring(8))), true);
                }
            }
            // Hearts start with the scenario: their tickers have not run during the warm-up.
            var blockLookup = level.holderLookup(Registries.BLOCK);
            List<BlockPos> heartPositions = new ArrayList<>();
            for (String[] h : s.hearts) {
                BlockPos at = new BlockPos(Integer.parseInt(h[1]), Integer.parseInt(h[2]), Integer.parseInt(h[3]));
                level.setBlock(at, BlockStateParser.parseForBlock(blockLookup, h[4], false).blockState(), 2 | 16);
                heartPositions.add(at);
            }
            // Mobs the level creates during the scenario (a heart's creakings) get a seeded
            // random, delayed sensors and seeded gates as the scenario's own mobs do, the
            // n-th with seed 1000 + n; they join the list once in the level.
            List<Mob> created = new ArrayList<>();
            Field factoryField = field(EntityType.class, "factory");
            Object creakingFactory = factoryField.get(EntityTypes.CREAKING);
            factoryField.set(EntityTypes.CREAKING, (EntityType.EntityFactory<Creaking>) (type, lvl) -> {
                Creaking c = new Creaking(type, lvl);
                try {
                    long seed = 1000 + created.size();
                    if (tracing(s)) field(Entity.class, "random").set(c, new TracingRandom(0L, TRACE, ENTS.size() + created.size()));
                    c.getRandom().setSeed(seed);
                    @SuppressWarnings("unchecked")
                    Map<Object, Sensor<?>> cs = (Map<Object, Sensor<?>>) get(c.getBrain(), "sensors");
                    for (Sensor<?> sensor : cs.values()) sensor.randomlyDelayStart(c.getRandom());
                    seedGates(c.getBrain(), seed);
                } catch (Exception e) {
                    throw new IllegalStateException(e);
                }
                created.add(c);
                return c;
            });
            boolean spawnMobs = this.getGameRules().get(GameRules.SPAWN_MOBS);
            boolean spawnMonsters = this.getGameRules().get(GameRules.SPAWN_MONSTERS);
            this.getGameRules().set(GameRules.SPAWN_MOBS, s.spawning, this);
            this.getGameRules().set(GameRules.SPAWN_MONSTERS, s.spawning, this);
            level.getRandom().setSeed(s.levelSeed);
            // Loot drawn from random sequences starts afresh in every scenario.
            this.getRandomSequences().clear();
            SOUNDS.clear();
            Method tickTime = method(ServerLevel.class, "tickTime");
            Object manager = get(level, "entityManager");
            Method managerTick = method(manager.getClass(), "tick");
            Map<BlockPos, BlockState> watched = new java.util.LinkedHashMap<>();
            for (int[] box : s.watches)
                for (int y = box[1]; y <= box[4]; y++)
                    for (int z = box[2]; z <= box[5]; z++)
                        for (int x = box[0]; x <= box[3]; x++) {
                            BlockPos pos = new BlockPos(x + s.originX, y, z + s.originZ);
                            watched.put(pos, level.getBlockState(pos));
                        }
            for (int i = 0; i < ENTS.size(); i++) print(0, i, ENTS.get(i));
            for (int t = 1; t <= s.ticks; t++) {
                if (s.daycycle) this.clockManager().setTotalTicks(clock, s.time + t);
                for (Object[] e : s.timed) if ((Integer) e[0] == t) place((String[]) e[1], level);
                level.environmentAttributes().invalidateTickCache();
                level.updateSkyBrightness();
                tickTime.invoke(level);
                if (player != null && s.player != null) {
                    player.setHealth(player.getMaxHealth());
                    player.setInvulnerableTime(0);
                    // The hurt cooldown (only ServerPlayer.tick counts it down).
                    field(LivingEntity.class, "damageCooldownTime").setInt(player, 0);
                    player.hurtTime = 0;
                    player.snapTo(s.player[0], s.player[1], s.player[2], 0.0F, 0.0F);
                    player.setDeltaMovement(Vec3.ZERO);
                    if (t >= s.turnTick) {
                        // The view vector follows the head (LivingEntity.getViewYRot).
                        player.setYRot(s.turnYaw);
                        player.setYHeadRot(s.turnYaw);
                        player.setYBodyRot(s.turnYaw);
                    }
                }
                TracingRandom.tick = t;
                TracingRandom.on = tracing(s) && t >= traceFrom() && t <= traceTo();
                SOUNDS.clear();
                for (int i = 0; i < ENTS.size(); i++) {
                    // Villager.refreshBrain builds a new brain with new gates: seed those too.
                    if (ENTS.get(i) instanceof Mob m && m.getBrain() != brains.get(i)) {
                        brains.set(i, m.getBrain());
                        seedGates(brains.get(i), seeds.get(i));
                    }
                }
                for (int[] hurt : s.hurts) {
                    if (hurt[0] == t && hurt[1] < ENTS.size() && ENTS.get(hurt[1]) instanceof LivingEntity hurtMob && !hurtMob.isRemoved())
                        hurtMob.hurtServer(level, level.damageSources().playerAttack(player), 1.0F);
                }
                for (Object[] feed : s.feeds) {
                    int index = (Integer) feed[1];
                    if ((Integer) feed[0] != t || index >= ENTS.size()
                        || !(ENTS.get(index) instanceof net.minecraft.world.entity.animal.Animal animal) || animal.isRemoved()) continue;
                    var hand = net.minecraft.world.InteractionHand.MAIN_HAND;
                    var held = player.getItemInHand(hand);
                    player.setItemInHand(hand, new net.minecraft.world.item.ItemStack(
                        BuiltInRegistries.ITEM.getValue(net.minecraft.resources.Identifier.parse((String) feed[2])), 64));
                    animal.mobInteract(player, hand);
                    player.setItemInHand(hand, held);
                }
                for (Entity e : List.copyOf(ENTS)) {
                    if (e.isRemoved()) continue;
                    e.checkDespawn();
                    if (e.isRemoved()) continue;
                    level.tickNonPassenger(e);
                }
                if (s.blocks.stream().anyMatch(Server::isHive)) {
                    // A bee leaving its hive takes the id the probe gives it, 100000 + its index
                    // (goals run in full on ticks where tickCount + id is even).
                    ((java.util.concurrent.atomic.AtomicInteger) field(ServerLevel.class, "ENTITY_COUNTER").get(null)).set(100000 + ENTS.size() - 1);
                    level.tickBlockEntities();
                } else if (!heartPositions.isEmpty()) level.tickBlockEntities();
                managerTick.invoke(manager);
                // Villagers the tick spawned (babies), the iron golems they summoned and a heart's
                // creakings tick from the next one, in the order they were added, with a seeded
                // random of their own. Other newcomers (animal babies, dropped items) are left out,
                // as the probe leaves them.
                List<Entity> fresh = new ArrayList<>();
                for (Entity e : level.getAllEntities())
                    if (!LABELS.containsKey(e) && (e instanceof Villager || e instanceof net.minecraft.world.entity.animal.bee.Bee
                        || e instanceof net.minecraft.world.entity.animal.golem.IronGolem || created.contains(e))) fresh.add(e);
                fresh.sort(java.util.Comparator.comparingInt(Entity::getId));
                for (Entity e : fresh) {
                    int index = ENTS.size();
                    int made = created.indexOf(e);
                    if (made >= 0) {
                        // A heart's creaking: seeded when it was created.
                        LABELS.put(e, index);
                        ENTS.add(e);
                        seeds.add(1000L + made);
                        brains.add(((Mob) e).getBrain());
                        continue;
                    }
                    long seed = spawnSeed(index);
                    LABELS.put(e, index);
                    ENTS.add(e);
                    seeds.add(seed);
                    if (e instanceof Mob mob) {
                        // LivingEntity's constructor turns the head at random (Math.random):
                        // face it the way the body does, as the probe's newborns.
                        mob.setYHeadRot(mob.getYRot());
                        mob.yHeadRotO = mob.getYRot();
                        if (tracing(s)) field(Entity.class, "random").set(mob, new TracingRandom(0L, TRACE, index));
                        mob.getRandom().setSeed(seed);
                        // A bee out of a hive: its constructor drew from an unseeded random.
                        if (mob instanceof net.minecraft.world.entity.animal.bee.Bee bee) seedBee(bee);
                        @SuppressWarnings("unchecked")
                        Map<Object, Sensor<?>> sensors = (Map<Object, Sensor<?>>) get(mob.getBrain(), "sensors");
                        for (Sensor<?> sensor : sensors.values()) sensor.randomlyDelayStart(mob.getRandom());
                        seedGates(mob.getBrain(), seed);
                        brains.add(mob.getBrain());
                    } else {
                        brains.add(null);
                    }
                }
                for (int i = 0; i < ENTS.size(); i++) print(t, i, ENTS.get(i));
                for (BlockPos h : heartPositions) printHeart(t, level, h);
                for (String[] b : s.blocks) if (b[0].equals("chest")) printChest(t, s, level, b);
                for (String[] b : s.blocks) if (isHive(b)) printHive(t, level, b);
                if (s.name.startsWith("leash_death_")) {
                    int leads = level.getEntitiesOfClass(ItemEntity.class, ENTS.get(0).getBoundingBox().inflate(32.0),
                        item -> item.getItem().is(net.minecraft.world.item.Items.LEAD))
                        .stream().mapToInt(item -> item.getItem().getCount()).sum();
                    out.println(t + " leads " + leads);
                }
                for (String sound : SOUNDS) out.println(t + " sound " + sound);
                for (var w : watched.entrySet()) {
                    BlockState now = level.getBlockState(w.getKey());
                    if (now != w.getValue()) {
                        w.setValue(now);
                        BlockPos p = w.getKey();
                        out.println(t + " block " + (p.getX() - s.originX) + "," + p.getY() + "," + (p.getZ() - s.originZ) + " " + stateName(now));
                    }
                }
                for (String line : TRACE) out.println(line);
                TRACE.clear();
            }
            for (Entity e : ENTS) e.discard();
            for (BlockPos h : heartPositions) level.setBlock(h, net.minecraft.world.level.block.Blocks.AIR.defaultBlockState(), 2 | 16);
            factoryField.set(EntityTypes.CREAKING, creakingFactory);
            this.getGameRules().set(GameRules.SPAWN_MOBS, spawnMobs, this);
            this.getGameRules().set(GameRules.SPAWN_MONSTERS, spawnMonsters, this);
            managerTick.invoke(manager);
            out.flush();
        }

        /**
         * A bee's constructor draws, made again with its seeded random: the validate goals'
         * cooldowns (registerGoals), then the flower cooldown's initialiser.
         */
        static void seedBee(net.minecraft.world.entity.animal.bee.Bee bee) throws Exception {
            var rnd = bee.getRandom();
            int hive = net.minecraft.util.Mth.nextInt(rnd, 20, 40);
            int flower = net.minecraft.util.Mth.nextInt(rnd, 20, 40);
            for (WrappedGoal g : ((GoalSelector) get(bee, "goalSelector")).getAvailableGoals()) {
                String n = g.getGoal().getClass().getSimpleName();
                if (n.equals("ValidateHiveGoal")) field(g.getGoal().getClass(), "VALIDATE_HIVE_COOLDOWN").setInt(g.getGoal(), hive);
                if (n.equals("ValidateFlowerGoal")) field(g.getGoal().getClass(), "validateFlowerCooldown").setInt(g.getGoal(), flower);
            }
            field(net.minecraft.world.entity.animal.bee.Bee.class, "remainingCooldownBeforeLocatingNewFlower")
                .setInt(bee, net.minecraft.util.Mth.nextInt(rnd, 20, 60));
        }

        /** A hive's honey level and bees (ticks inside / ticks to stay). */
        static void printHive(int t, ServerLevel level, String[] b) {
            BlockPos at = new BlockPos(Integer.parseInt(b[1]), Integer.parseInt(b[2]), Integer.parseInt(b[3]));
            if (!(level.getBlockEntity(at) instanceof net.minecraft.world.level.block.entity.BeehiveBlockEntity hive)) return;
            net.minecraft.nbt.CompoundTag tag = hive.saveWithoutMetadata(level.registryAccess());
            StringBuilder bees = new StringBuilder();
            for (var bee : tag.getListOrEmpty("bees")) {
                net.minecraft.nbt.CompoundTag c = (net.minecraft.nbt.CompoundTag) bee;
                if (!bees.isEmpty()) bees.append(',');
                bees.append(c.getIntOr("ticks_in_hive", 0)).append('/').append(c.getIntOr("min_ticks_in_hive", 0));
            }
            BlockState state = level.getBlockState(at);
            out.println(t + " hive " + b[1] + "," + b[2] + "," + b[3] + " honey "
                + (state.hasProperty(net.minecraft.world.level.block.BeehiveBlock.HONEY_LEVEL) ? state.getValue(net.minecraft.world.level.block.BeehiveBlock.HONEY_LEVEL) : -1)
                + " bees " + (bees.isEmpty() ? "-" : bees) + " flower " + tag.getIntArray("flower_pos").map(p -> p[0] + "," + p[1] + "," + p[2]).orElse("-"));
        }

        static boolean isHive(String[] b) {
            return b[0].equals("block") && (b[4].startsWith("minecraft:beehive") || b[4].startsWith("minecraft:bee_nest"));
        }

        /** The random seed of the entity a scenario spawned at this index. */
        static long spawnSeed(int index) { return 1000L + 7919L * index; }

        static String stateName(BlockState state) {
            StringBuilder sb = new StringBuilder(BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString());
            List<String> props = new ArrayList<>();
            state.getValues().forEach(v -> props.add(v.toString()));
            props.sort(null);
            if (!props.isEmpty()) sb.append('[').append(String.join(",", props)).append(']');
            return sb.toString();
        }

        /** A chest's slots (trailing empties dropped) and opener count. */
        static void printChest(int t, Scenario s, ServerLevel level, String[] b) throws Exception {
            BlockPos at = new BlockPos(Integer.parseInt(b[1]), Integer.parseInt(b[2]), Integer.parseInt(b[3]));
            if (!(level.getBlockEntity(at) instanceof net.minecraft.world.level.block.entity.ChestBlockEntity chest)) return;
            List<String> slots = new ArrayList<>();
            for (int k = 0; k < chest.getContainerSize(); k++) slots.add(stack(chest.getItem(k)));
            while (!slots.isEmpty() && slots.getLast().equals("-")) slots.removeLast();
            Object counter = get(chest, "openersCounter");
            out.println(t + " chest " + (at.getX() - s.originX) + "," + at.getY() + "," + (at.getZ() - s.originZ) + " "
                + (slots.isEmpty() ? "-" : String.join(",", slots)) + " "
                + ((net.minecraft.world.level.block.entity.ContainerOpenersCounter) counter).getOpenerCount());
        }

        /** A heart's block state and block entity, and the level random. */
        static void printHeart(int t, ServerLevel level, BlockPos at) throws Exception {
            StringBuilder sb = new StringBuilder();
            sb.append(t).append(" heart ").append(pos(at)).append('\t')
                .append(level.getBlockState(at).getValue(net.minecraft.world.level.block.CreakingHeartBlock.STATE).getSerializedName());
            if (level.getBlockEntity(at) instanceof net.minecraft.world.level.block.entity.CreakingHeartBlockEntity be) {
                sb.append('\t').append(get(be, "ticker")).append(' ').append(get(be, "emitter")).append(' ').append(get(be, "outputSignal"))
                    .append(' ').append(get(be, "ticksExisted"));
                Object info = get(be, "creakingInfo");
                sb.append(' ').append(info == null ? "none" : "linked");
            }
            sb.append("\tlrng ").append(Long.toHexString(((java.util.concurrent.atomic.AtomicLong) field(net.minecraft.world.level.levelgen.LegacyRandomSource.class, "seed").get(level.getRandom())).get()));
            out.println(sb);
        }

        /**
         * Every GateBehavior (RunOne and the gates of TradeWithVillager and VillagerMakeLove) shuffles
         * its behaviours with its own ShufflingList random, which RandomSource.create() seeds from the
         * clock: give each a LegacyRandomSource seeded with the mob's seed so runs are reproducible.
         */
        static void seedGates(Brain<?> brain, long seed) throws Exception {
            @SuppressWarnings("unchecked")
            Map<Integer, Map<?, Set<BehaviorControl<?>>>> byPriority = (Map<Integer, Map<?, Set<BehaviorControl<?>>>>) get(brain, "availableBehaviorsByPriority");
            for (Map<?, Set<BehaviorControl<?>>> byActivity : byPriority.values())
                for (Set<BehaviorControl<?>> behaviors : byActivity.values())
                    for (BehaviorControl<?> behavior : behaviors) seedGate(behavior, seed);
        }

        static void seedGate(Object behavior, long seed) throws Exception {
            seedLists(behavior, seed, java.util.Collections.newSetFromMap(new java.util.IdentityHashMap<>()), 0);
        }

        /**
         * Seeds every ShufflingList a behaviour holds: a GateBehavior's, and the ones TriggerGate
         * captures in its lambdas (BehaviorBuilder's one-shots), found through their fields.
         */
        static void seedLists(Object o, long seed, Set<Object> seen, int depth) throws Exception {
            if (o == null || depth > 8 || !seen.add(o)) return;
            if (o instanceof ShufflingList<?> list) {
                field(ShufflingList.class, "random").set(list, new net.minecraft.world.level.levelgen.LegacyRandomSource(seed));
                for (Object child : list) seedLists(child, seed, seen, depth + 1);
                return;
            }
            String name = o.getClass().getName();
            if (!name.startsWith("net.minecraft.world.entity.ai.behavior") || o instanceof Enum<?>) return;
            for (Class<?> c = o.getClass(); c != null && c.getName().startsWith("net.minecraft"); c = c.getSuperclass()) {
                for (Field f : c.getDeclaredFields()) {
                    if (java.lang.reflect.Modifier.isStatic(f.getModifiers()) || f.getType().isPrimitive()) continue;
                    f.setAccessible(true);
                    Object v = f.get(o);
                    if (v instanceof Iterable<?> it && !(v instanceof ShufflingList<?>)) {
                        for (Object x : it) seedLists(x, seed, seen, depth + 1);
                    } else if (v instanceof com.mojang.datafixers.util.Pair<?, ?> pair) {
                        seedLists(pair.getFirst(), seed, seen, depth + 1);
                        seedLists(pair.getSecond(), seed, seen, depth + 1);
                    } else {
                        seedLists(v, seed, seen, depth + 1);
                    }
                }
            }
        }

        static String who(Entity e) {
            if (e == null) return "none";
            if (e instanceof ServerPlayer) return "player";
            Integer label = LABELS.get(e);
            return "e" + (label != null ? 100000 + label : e.getId());
        }

        static String goals(GoalSelector selector) {
            StringBuilder sb = new StringBuilder();
            for (WrappedGoal g : selector.getAvailableGoals()) {
                if (!g.isRunning()) continue;
                if (sb.length() > 0) sb.append('|');
                Class<?> c = g.getGoal().getClass();
                String name = c.getSimpleName();
                if (name.isEmpty()) name = c.getName().substring(c.getName().lastIndexOf('.') + 1);
                sb.append(name);
            }
            return sb.length() == 0 ? "-" : sb.toString();
        }

        static String pos(BlockPos p) { return p.getX() + "," + p.getY() + "," + p.getZ(); }

        static String value(Object v) {
            if (v == null) return "null";
            if (v instanceof Entity e) return who(e);
            if (v instanceof BlockPos p) return pos(p);
            if (v instanceof GlobalPos g) return pos(g.pos());
            if (v instanceof WalkTarget w) return value(w.getTarget()) + "/" + f(w.getSpeedModifier()) + "/" + w.getCloseEnoughDist();
            if (v instanceof EntityTracker e) return who(e.getEntity());
            if (v instanceof BlockPosTracker b) return pos(b.currentBlockPosition());
            if (v instanceof Boolean || v instanceof Integer || v instanceof Long) return v.toString();
            if (v instanceof net.minecraft.util.Unit) return "u";
            if (v instanceof Collection<?> c) return "n" + c.size();
            if (v instanceof net.minecraft.world.level.pathfinder.Path p) return "path" + pos(p.getTarget());
            return v.getClass().getSimpleName();
        }

        void print(int t, int i, Entity e) throws Exception {
            StringBuilder sb = new StringBuilder();
            sb.append(t).append(' ').append(i);
            if (e.isRemoved()) {
                out.println(sb.append(" removed ").append(e.getRemovalReason()));
                return;
            }
            if (e instanceof ItemEntity item) {
                sb.append("\tp ").append(d(item.getX())).append(',').append(d(item.getY())).append(',').append(d(item.getZ()));
                Vec3 v = item.getDeltaMovement();
                sb.append("\tv ").append(d(v.x)).append(',').append(d(v.y)).append(',').append(d(v.z));
                sb.append("\tg ").append(item.onGround() ? 1 : 0).append(item.horizontalCollision ? 1 : 0);
                sb.append("\titem ").append(stack(item.getItem())).append(' ').append(item.getAge()).append(' ')
                    .append(field(ItemEntity.class, "pickupDelay").getInt(item));
                out.println(sb);
                return;
            }
            Mob m = (Mob) e;
            sb.append("\tleashed ").append(((net.minecraft.world.entity.Leashable) m).isLeashed() ? 1 : 0);
            sb.append("\tp ").append(d(m.getX())).append(',').append(d(m.getY())).append(',').append(d(m.getZ()));
            Vec3 v = m.getDeltaMovement();
            sb.append("\tv ").append(d(v.x)).append(',').append(d(v.y)).append(',').append(d(v.z));
            sb.append("\tr ").append(f(m.getYRot())).append(',').append(f(m.getXRot())).append(',').append(f(m.getYHeadRot())).append(',').append(f(m.yBodyRot));
            sb.append("\tg ").append(m.onGround() ? 1 : 0).append(m.horizontalCollision ? 1 : 0);
            sb.append("\thp ").append(f(m.getHealth()));
            sb.append("\tgoals ").append(goals((GoalSelector) get(m, "goalSelector")));
            sb.append("\ttargets ").append(goals((GoalSelector) get(m, "targetSelector")));
            sb.append("\ttarget ").append(who(m.getTarget()));
            PathNavigation nav = m.getNavigation();
            var path = nav.getPath();
            sb.append("\tnav ");
            if (path == null) sb.append("none");
            else sb.append(pos(path.getTarget())).append(' ').append(path.getNextNodeIndex()).append('/').append(path.getNodeCount())
                .append(path.isDone() ? " done" : "");
            MoveControl mc = m.getMoveControl();
            sb.append("\tmove ").append(get(mc, "operation")).append(' ').append(d((double) get(mc, "wantedX"))).append(',')
                .append(d((double) get(mc, "wantedY"))).append(',').append(d((double) get(mc, "wantedZ"))).append(' ')
                .append(d((double) get(mc, "speedModifier")));
            LookControl lc = m.getLookControl();
            sb.append("\tlook ").append(get(lc, "lookAtCooldown")).append(' ').append(d((double) get(lc, "wantedX"))).append(',')
                .append(d((double) get(lc, "wantedY"))).append(',').append(d((double) get(lc, "wantedZ")));
            sb.append("\tin ").append(f(m.xxa)).append(',').append(f(m.zza)).append(',').append(f(m.getSpeed())).append(',')
                .append(field(LivingEntity.class, "jumping").getBoolean(m) ? 1 : 0);
            sb.append("\trng ").append(Long.toHexString(((java.util.concurrent.atomic.AtomicLong) field(net.minecraft.world.level.levelgen.LegacyRandomSource.class, "seed").get(m.getRandom())).get()));
            Brain<?> brain = m.getBrain();
            if (!brain.getActiveActivities().isEmpty() && !((Map<?, ?>) get(brain, "sensors")).isEmpty()) {
                List<String> acts = new ArrayList<>();
                for (var a : brain.getActiveActivities()) acts.add(a.getName());
                acts.sort(null);
                sb.append("\tact ").append(String.join("|", acts));
                List<String> mem = new ArrayList<>();
                brain.forEach(new Brain.Visitor() {
                    @Override public <U> void acceptEmpty(MemoryModuleType<U> type) {}
                    @Override public <U> void accept(MemoryModuleType<U> type, U value) { mem.add(name(type) + "=" + value(value)); }
                    @Override public <U> void accept(MemoryModuleType<U> type, U value, long ttl) { mem.add(name(type) + "=" + value(value) + "@" + ttl); }
                });
                mem.sort(null);
                sb.append("\tmem ").append(String.join(" ", mem));
            }
            if (m instanceof Villager villager) sb.append("\tvx ").append(villager(villager));
            if (m instanceof net.minecraft.world.entity.animal.golem.CopperGolem golem) {
                Object opened = get(golem, "openedChestPos");
                sb.append("\tcg ").append(golem.getState().ordinal()).append(' ').append(stack(golem.getMainHandItem())).append(' ')
                    .append(opened == null ? "-" : pos((BlockPos) opened));
            }
            if (m instanceof net.minecraft.world.entity.animal.wolf.Wolf wolf) {
                // NeutralMob anger left (the angry texture), -1 when calm.
                long end = wolf.getPersistentAngerEndTime();
                sb.append("	wolf ").append(end > 0 ? end - wolf.level().getGameTime() : -1);
            }
            if (m instanceof net.minecraft.world.entity.animal.bee.Bee bee) {
                // Flags (roll 2, stung 4, nectar 8), anger left, ticks without nectar, flower cooldown, flower.
                Object acc = field(net.minecraft.world.entity.animal.bee.Bee.class, "DATA_FLAGS_ID").get(null);
                @SuppressWarnings("unchecked")
                byte flags = bee.getEntityData().get((net.minecraft.network.syncher.EntityDataAccessor<Byte>) acc);
                long end = bee.getPersistentAngerEndTime();
                BlockPos flower = bee.getSavedFlowerPos();
                sb.append("\tbee ").append(flags).append(' ').append(end > 0 ? end - bee.level().getGameTime() : -1).append(' ')
                    .append(field(net.minecraft.world.entity.animal.bee.Bee.class, "ticksWithoutNectarSinceExitingHive").getInt(bee)).append(' ')
                    .append(field(net.minecraft.world.entity.animal.bee.Bee.class, "remainingCooldownBeforeLocatingNewFlower").getInt(bee)).append(' ')
                    .append(flower == null ? "-" : pos(flower));
            }
            out.println(sb);
        }

        static String stack(net.minecraft.world.item.ItemStack stack) {
            if (stack.isEmpty()) return "-";
            return BuiltInRegistries.ITEM.getKey(stack.getItem()).getPath() + "*" + stack.getCount();
        }

        static String uuidLabel(UUID uuid) {
            if (uuid.equals(PLAYER_UUID)) return "player";
            for (Entity e : ENTS) if (e.getUUID().equals(uuid)) return who(e);
            return uuid.toString();
        }

        /** A villager's data, food, inventory slots, gossip and offers. */
        static String villager(Villager v) throws Exception {
            StringBuilder sb = new StringBuilder();
            var data = v.getVillagerData();
            sb.append("prof ").append(data.profession().unwrapKey().get().identifier().getPath()).append(" lvl ").append(data.level())
                .append(" xp ").append(v.getVillagerXp()).append(" food ").append(field(Villager.class, "foodLevel").getInt(v))
                .append(" age ").append(v.getAge());
            List<String> slots = new ArrayList<>();
            var inv = v.getInventory();
            for (int k = 0; k < inv.getContainerSize(); k++) slots.add(stack(inv.getItem(k)));
            while (!slots.isEmpty() && slots.getLast().equals("-")) slots.removeLast();
            sb.append(" inv ").append(slots.isEmpty() ? "-" : String.join(",", slots));
            List<String> gossips = new ArrayList<>();
            v.getGossips().getGossipEntries().forEach((uuid, types) -> types.forEach((type, value) ->
                gossips.add(uuidLabel(uuid) + ":" + type.getSerializedName() + "=" + value)));
            gossips.sort(null);
            sb.append(" gossip ").append(gossips.isEmpty() ? "-" : String.join(",", gossips));
            sb.append(" rep ").append(v.getGossips().getReputation(PLAYER_UUID, x -> true));
            sb.append(" sleep ").append(v.getSleepingPos().map(Server::pos).orElse("-"));
            Object offers = get(v, "offers");
            if (offers != null) {
                List<String> list = new ArrayList<>();
                for (var o : (net.minecraft.world.item.trading.MerchantOffers) offers) {
                    list.add(stack(o.getBaseCostA()) + "+" + stack(o.getCostB()) + ">" + stack(o.getResult()) + "/" + o.getUses() + "/"
                        + o.getMaxUses() + "/" + o.getDemand() + "/" + o.getSpecialPriceDiff());
                }
                sb.append(" offers ").append(list.isEmpty() ? "-" : String.join(",", list));
            }
            return sb.toString();
        }

        static String name(MemoryModuleType<?> type) {
            return BuiltInRegistries.MEMORY_MODULE_TYPE.getKey(type).getPath();
        }

        @Override protected SampleLogger getTickTimeLogger() { return sampleLogger; }
        @Override public boolean isTickTimeLoggingEnabled() { return false; }
        @Override protected void waitUntilNextTick() { this.runAllTasks(); }
        @Override public SystemReport fillServerSystemReport(SystemReport report) { report.setDetail("Type", "Mob AI oracle"); return report; }
        @Override protected void onServerExit() { super.onServerExit(); System.exit(0); }
        @Override protected void onServerCrash(net.minecraft.CrashReport report) { super.onServerCrash(report); System.exit(1); }
        @Override public boolean isHardcore() { return false; }
        @Override public LevelBasedPermissionSet operatorUserPermissions() { return LevelBasedPermissionSet.ALL; }
        @Override public PermissionSet getFunctionCompilationPermissions() { return LevelBasedPermissionSet.OWNER; }
        @Override public boolean shouldRconBroadcast() { return false; }
        @Override public boolean isDedicatedServer() { return false; }
        @Override public int getRateLimitPacketsPerSecond() { return 0; }
        @Override public int getCommandSpamThresholdSeconds() { return 0; }
        @Override public int getChatSpamThresholdSeconds() { return 0; }
        @Override public boolean useNativeTransport() { return false; }
        @Override public boolean isPublished() { return false; }
        @Override public boolean shouldInformAdmins() { return false; }
        @Override public boolean isSingleplayerOwner(NameAndId nameAndId) { return false; }
        @Override public int getMaxPlayers() { return 1; }
    }
}
