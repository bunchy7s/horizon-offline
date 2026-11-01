package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.sounds.SoundEvent;
import net.minecraft.sounds.SoundSource;
import net.minecraft.util.RandomSource;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.level.gamerules.GameRules;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.LevelChunk;
import net.minecraft.world.level.levelgen.LegacyRandomSource;
import net.minecraft.world.level.material.Fluid;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.ticks.LevelTicks;

/**
 * Drives vanilla block behaviour (updateShape, randomTick, tick, neighbour
 * updates, setBlock cascades and multi-tick runs) over a map-backed headless
 * ServerLevel. Shared by tools/test_block_update_parity.py,
 * test_random_tick_parity.py and test_redstone_parity.py. Block entities do not
 * exist here; entities never exist; every side effect is appended to an event
 * log compared against the Rust port.
 */
public final class VanillaBlockUpdateOracle {
    static Object unsafe;
    static Method allocate;

    @SuppressWarnings("unchecked")
    static <T> T make(Class<T> type) {
        try {
            return (T) allocate.invoke(unsafe, type);
        } catch (Exception e) {
            throw new AssertionError(e);
        }
    }

    static void set(Class<?> owner, Object target, String name, Object value) throws Exception {
        Field f = owner.getDeclaredField(name);
        f.setAccessible(true);
        f.set(target, value);
    }

    static BlockState AIR;

    static String pos(BlockPos p) {
        return p.getX() + "," + p.getY() + "," + p.getZ();
    }

    static final class Chunk extends LevelChunk {
        World world;
        Chunk() {
            super(null, null);
        }
        @Override
        public BlockState setBlockState(BlockPos pos, BlockState state, int flags) {
            BlockState old = world.cells.getOrDefault(pos.immutable(), AIR);
            if (old == state) {
                return null;
            }
            world.columns = null;
            if (state == AIR) {
                world.cells.remove(pos);
            } else {
                world.cells.put(pos.immutable(), state);
            }
            Block newBlock = state.getBlock();
            boolean blockChanged = !old.is(newBlock);
            boolean movedByPiston = (flags & 64) != 0;
            boolean sideEffects = (flags & 256) == 0;
            if (blockChanged && old.hasBlockEntity() && !state.shouldChangedStateKeepBlockEntity(old)) {
                // A real chunk always holds the block entity of an entity block.
                BlockEntity blockEntity = world.blockEntityFor(pos.immutable(), old);
                if (sideEffects && blockEntity != null) {
                    blockEntity.preRemoveSideEffects(pos, old);
                }
                // LevelChunk.removeBlockEntity.
                if (world.blockEntities != null) world.blockEntities.remove(pos);
                if (blockEntity != null) blockEntity.setRemoved();
            }
            if ((blockChanged || newBlock instanceof net.minecraft.world.level.block.BaseRailBlock) && ((flags & 1) != 0 || movedByPiston)) {
                old.affectNeighborsAfterRemoval(world, pos, movedByPiston);
            }
            if (!world.getBlockState(pos).is(newBlock)) {
                return null;
            }
            if ((flags & 512) == 0) {
                state.onPlace(world, pos, old, movedByPiston);
            }
            return old;
        }
        @Override
        public void markUnsaved() { }
        @Override
        public BlockState getBlockState(BlockPos pos) { return world.getBlockState(pos); }
        @Override
        public FluidState getFluidState(BlockPos pos) { return world.getFluidState(pos); }
        @Override
        public net.minecraft.world.level.chunk.status.ChunkStatus getPersistedStatus() {
            return net.minecraft.world.level.chunk.status.ChunkStatus.FULL;
        }
        @Override
        public net.minecraft.server.level.FullChunkStatus getFullStatus() {
            return net.minecraft.server.level.FullChunkStatus.ENTITY_TICKING;
        }
    }

    static final class World extends net.minecraft.server.level.ServerLevel {
        Map<BlockPos, BlockState> cells = new HashMap<>();
        RandomSource rng;
        long time;
        long subTick;
        Chunk chunk;
        LevelTicks<Block> blockTicks;
        LevelTicks<Fluid> fluidTicks;
        StringBuilder log = new StringBuilder();
        int skyLight = 15;
        int blockLight = 0;
        GameRules rules;
        Server server;
        boolean raining, thundering;
        World() {
            super(null, null, null, null, null, null, false, 0L, List.of(), false);
        }
        java.util.LinkedHashSet<String> blockEvents;
        void emit(String line) {
            log.append(line).append('\n');
        }
        @Override public BlockState getBlockState(BlockPos p) { return cells.getOrDefault(p, AIR); }
        @Override public FluidState getFluidState(BlockPos p) { return getBlockState(p).getFluidState(); }
        // A real level always holds the block entity of an entity block; this
        // one creates it on first use and replaces it when the block changes.
        Map<BlockPos, BlockEntity> blockEntities;
        @Override public BlockEntity getBlockEntity(BlockPos p) {
            return blockEntityFor(p, getBlockState(p));
        }
        /** The block entity at `p` for `state` (the block there, or the one being replaced). */
        BlockEntity blockEntityFor(BlockPos p, BlockState state) {
            if (!(state.getBlock() instanceof net.minecraft.world.level.block.EntityBlock entityBlock)) {
                return null;
            }
            if (blockEntities == null) blockEntities = new HashMap<>();
            BlockEntity be = blockEntities.get(p);
            if (be == null || !be.getType().isValid(state)) {
                be = entityBlock.newBlockEntity(p.immutable(), state);
                if (be == null) return null;
                be.setLevel(this);
                blockEntities.put(p.immutable(), be);
            } else {
                be.setBlockState(state);
            }
            return be;
        }
        @Override public LevelChunk getChunkAt(BlockPos p) { return chunk; }
        @Override public LevelChunk getChunk(int x, int z) { return chunk; }
        @Override public net.minecraft.world.level.chunk.ChunkAccess getChunk(int x, int z, net.minecraft.world.level.chunk.status.ChunkStatus status, boolean load) { return chunk; }
        net.minecraft.server.waypoints.ServerWaypointManager waypoints;
        @Override public net.minecraft.server.waypoints.ServerWaypointManager getWaypointManager() {
            if (waypoints == null) waypoints = new net.minecraft.server.waypoints.ServerWaypointManager();
            return waypoints;
        }
        net.minecraft.server.ServerScoreboard scoreboard;
        @Override public net.minecraft.server.ServerScoreboard getScoreboard() {
            if (scoreboard == null) scoreboard = new net.minecraft.server.ServerScoreboard(server);
            return scoreboard;
        }
        /** The biome every position reads (plains), for the oracles that load the world registries. */
        net.minecraft.core.Holder<net.minecraft.world.level.biome.Biome> biome;
        @Override public net.minecraft.core.Holder<net.minecraft.world.level.biome.Biome> getBiome(BlockPos p) {
            if (biome == null) biome = registryAccess().lookupOrThrow(net.minecraft.core.registries.Registries.BIOME).getOrThrow(net.minecraft.world.level.biome.Biomes.PLAINS);
            return biome;
        }
        @Override public boolean isLoaded(BlockPos p) { return true; }
        @Override public boolean hasChunkAt(BlockPos p) { return true; }
        @Override public boolean hasChunksAt(int x0, int z0, int x1, int z1) { return true; }
        @Override public boolean hasChunksAt(int x0, int y0, int z0, int x1, int y1, int z1) { return true; }
        @Override public boolean hasChunk(int x, int z) { return true; }
        @Override public boolean isClientSide() { return false; }
        @Override public RandomSource getRandom() { return rng; }
        // Another dimension, border and points of interest (the portal oracle);
        // the overworld and an unbounded border when unset.
        net.minecraft.world.level.dimension.DimensionType type;
        net.minecraft.resources.ResourceKey<net.minecraft.world.level.Level> dimensionKey;
        net.minecraft.world.level.border.WorldBorder border;
        net.minecraft.world.entity.ai.village.poi.PoiManager poi;
        @Override public int getMinY() { return type == null ? -64 : type.minY(); }
        @Override public int getHeight() { return type == null ? 384 : type.height(); }
        @Override public net.minecraft.world.entity.ai.village.poi.PoiManager getPoiManager() { return poi != null ? poi : super.getPoiManager(); }
        /** Level.getHeight from the cells (only for another dimension's level). */
        Map<Long, List<BlockPos>> columns;
        @Override public int getHeight(net.minecraft.world.level.levelgen.Heightmap.Types kind, int x, int z) {
            if (type == null) return super.getHeight(kind, x, z);
            if (columns == null) {
                columns = new HashMap<>();
                for (BlockPos c : cells.keySet()) columns.computeIfAbsent(BlockPos.asLong(c.getX(), 0, c.getZ()), k -> new ArrayList<>()).add(c);
            }
            int top = getMinY() - 1;
            for (BlockPos c : columns.getOrDefault(BlockPos.asLong(x, 0, z), List.of())) {
                if (c.getY() > top && kind.isOpaque().test(getBlockState(c))) top = c.getY();
            }
            return top + 1;
        }
        @Override public long getGameTime() { return time; }
        @Override public long nextSubTickCount() { return subTick++; }
        @Override public LevelTicks<Block> getBlockTicks() { return blockTicks; }
        @Override public LevelTicks<Fluid> getFluidTicks() { return fluidTicks; }
        @Override public void sendBlockUpdated(BlockPos p, BlockState a, BlockState b, int f) { }
        @Override public void setBlocksDirty(BlockPos p, BlockState a, BlockState b) { }
        @Override public net.minecraft.core.RegistryAccess registryAccess() { return registries != null ? registries : RegistryAccess.fromRegistryOfRegistries(BuiltInRegistries.REGISTRY); }
        @Override public net.minecraft.world.flag.FeatureFlagSet enabledFeatures() { return FeatureFlags.DEFAULT_FLAGS; }
        @Override public GameRules getGameRules() { return rules; }
        @Override public net.minecraft.server.MinecraftServer getServer() { return server; }
        @Override public net.minecraft.world.item.crafting.RecipeManager recipeAccess() { return Server.recipes; }
        @Override public net.minecraft.world.level.border.WorldBorder getWorldBorder() { return border != null ? border : new net.minecraft.world.level.border.WorldBorder(); }
        @Override public int getSeaLevel() { return 63; }
        @Override public int getBrightness(net.minecraft.world.level.LightLayer layer, BlockPos p) { return layer == net.minecraft.world.level.LightLayer.SKY ? skyLight : blockLight; }
        @Override public int getRawBrightness(BlockPos p, int darkening) { return Math.max(skyLight - darkening, blockLight); }
        @Override public int getSkyDarken() { return 0; }
        @Override public boolean isRaining() { return raining; }
        @Override public boolean isThundering() { return thundering; }
        @Override public boolean isRainingAt(BlockPos p) { return raining; }
        // With entities (a harness opted in) vanilla's own check runs on them
        // (CollisionGetter.isUnobstructed, then EntityGetter's blocksBuilding test).
        @Override public boolean isUnobstructed(BlockState state, BlockPos pos, net.minecraft.world.phys.shapes.CollisionContext c) { return entities == null || super.isUnobstructed(state, pos, c); }
        @Override public boolean isUnobstructed(Entity entity) { return true; }
        // Entities exist only when a harness opts in (the dispenser oracle);
        // they are searched in the order they were added, as within one entity section.
        List<Entity> entities;
        net.minecraft.core.RegistryAccess registries;
        @Override public List<Entity> getEntities(Entity except, AABB box, java.util.function.Predicate<? super Entity> filter) {
            if (entities == null) return List.of();
            List<Entity> out = new ArrayList<>();
            for (Entity e : List.copyOf(entities)) if (e != except && !e.isRemoved() && e.getBoundingBox().intersects(box) && filter.test(e)) out.add(e);
            return out;
        }
        @Override public <T extends Entity> void getEntities(net.minecraft.world.level.entity.EntityTypeTest<Entity, T> type, AABB box, java.util.function.Predicate<? super T> filter, List<? super T> out, int max) {
            if (entities == null) return;
            for (Entity e : List.copyOf(entities)) {
                T t = type.tryCast(e);
                if (t != null && !e.isRemoved() && e.getBoundingBox().intersects(box) && filter.test(t)) {
                    out.add(t);
                    if (out.size() >= max) return;
                }
            }
        }
        /** ServerLevel.getEntity(id) over the level's entities. */
        @Override public Entity getEntity(int id) {
            if (entities == null) return null;
            for (Entity e : entities) if (e.getId() == id && !e.isRemoved()) return e;
            return null;
        }
        /** Every entity of the level (no area), as ServerLevel's entity getter would. */
        @Override public <T extends Entity> void getEntities(net.minecraft.world.level.entity.EntityTypeTest<Entity, T> type, java.util.function.Predicate<? super T> filter, List<? super T> out, int max) {
            if (entities == null) return;
            for (Entity e : List.copyOf(entities)) {
                T t = type.tryCast(e);
                if (t != null && !e.isRemoved() && filter.test(t)) {
                    out.add(t);
                    if (out.size() >= max) return;
                }
            }
        }
        @Override public <T extends Entity> List<T> getEntitiesOfClass(Class<T> type, AABB box, java.util.function.Predicate<? super T> filter) {
            return getEntities(net.minecraft.world.level.entity.EntityTypeTest.forClass(type), box, filter);
        }
        @Override public List<net.minecraft.server.level.ServerPlayer> players() { return List.of(); }
        @Override public net.minecraft.world.entity.player.Player getNearestPlayer(double x, double y, double z, double d, boolean b) { return null; }
        @Override public boolean addFreshEntity(Entity entity) {
            emit("E " + BuiltInRegistries.ENTITY_TYPE.getKey(entity.getType()));
            if (entities != null) entities.add(entity);
            return true;
        }
        /** No players watch: the hurt animation and its sound are not sent. */
        @Override public void broadcastDamageEvent(Entity entity, net.minecraft.world.damagesource.DamageSource source) { }
        @Override public void levelEvent(Entity source, int type, BlockPos p, int data) { emit("L " + type + " " + pos(p) + " " + data); }
        @Override public void playSeededSound(Entity except, double x, double y, double z, net.minecraft.core.Holder<SoundEvent> sound, SoundSource source, float volume, float pitch, long seed) {
            emit("S " + pos(BlockPos.containing(x, y, z)) + " " + sound.value().location());
        }
        @Override public void playSeededSound(Entity except, Entity src, net.minecraft.core.Holder<SoundEvent> sound, SoundSource source, float volume, float pitch, long seed) {
            emit("S " + (entities != null ? pos(BlockPos.containing(src.position())) : "entity") + " " + sound.value().location());
        }
        @Override public void gameEvent(net.minecraft.core.Holder<net.minecraft.world.level.gameevent.GameEvent> e, Vec3 p, net.minecraft.world.level.gameevent.GameEvent.Context c) {
            emit("G " + pos(BlockPos.containing(p)) + " " + e.getRegisteredName());
        }
        @Override public <T extends net.minecraft.core.particles.ParticleOptions> int sendParticles(T p, double x, double y, double z, int n, double a, double b, double c, double s) { return 0; }
        @Override public void blockEvent(BlockPos p, Block block, int a, int b) { blockEvents.add("B " + pos(p) + " " + BuiltInRegistries.BLOCK.getKey(block) + " " + a + " " + b); }
        @Override public void updatePOIOnBlockStateChange(BlockPos p, BlockState a, BlockState b) { }
        @Override public boolean isPositionEntityTicking(BlockPos p) { return true; }
        @Override public boolean shouldTickBlocksAt(BlockPos p) { return true; }
        @Override public boolean shouldTickBlocksAt(long p) { return true; }
        @Override public boolean canSpreadFireAround(BlockPos p) { return true; }
        @Override public net.minecraft.world.Difficulty getDifficulty() { return net.minecraft.world.Difficulty.NORMAL; }
        // 26.3 block scans read chunk sections; this level has none, so scan
        // the same positions in the same order through getBlockState.
        @Override public net.minecraft.world.level.blockscan.BlockMatcher findBlocksIn(BlockPos from, BlockPos to) {
            return new Scan(this, boxOrder(from, to));
        }
        @Override public net.minecraft.world.level.blockscan.OrderedBlockMatcher findBlocksIn(Iterable<BlockPos> positions) {
            return new Scan(this, positions);
        }
        /** BlockScanUtils.sectionBasedScan's order: sections by x, z, y, then y, z, x inside; then below and above the world. */
        List<BlockPos> boxOrder(BlockPos from, BlockPos to) {
            int minX = Math.min(from.getX(), to.getX()), minY = Math.min(from.getY(), to.getY()), minZ = Math.min(from.getZ(), to.getZ());
            int maxX = Math.max(from.getX(), to.getX()), maxY = Math.max(from.getY(), to.getY()), maxZ = Math.max(from.getZ(), to.getZ());
            List<BlockPos> out = new ArrayList<>();
            for (int sx = minX >> 4; sx <= maxX >> 4; sx++) {
                for (int sz = minZ >> 4; sz <= maxZ >> 4; sz++) {
                    for (int sy = Math.max(getMinSectionY(), minY >> 4); sy <= Math.min(getMaxSectionY(), maxY >> 4); sy++) {
                        for (int y = Math.max(sy << 4, minY); y <= Math.min((sy << 4) + 15, maxY); y++) {
                            for (int z = Math.max(sz << 4, minZ); z <= Math.min((sz << 4) + 15, maxZ); z++) {
                                for (int x = Math.max(sx << 4, minX); x <= Math.min((sx << 4) + 15, maxX); x++) {
                                    out.add(new BlockPos(x, y, z));
                                }
                            }
                        }
                    }
                }
            }
            if (minY < getMinY()) {
                for (BlockPos q : BlockPos.betweenClosed(minX, minY, minZ, maxX, getMinY() - 1, maxZ)) out.add(q.immutable());
            }
            if (maxY > getMaxY()) {
                for (BlockPos q : BlockPos.betweenClosed(minX, getMaxY() + 1, minZ, maxX, maxY, maxZ)) out.add(q.immutable());
            }
            return out;
        }
        // Allocated without a constructor, so the sound seed generator, attribute
        // system and chunk source are missing. Sound seeds never touch the level
        // RNG, the overworld keeps every attribute default and entity ids are not
        // compared.
        int nextEntityId;
        net.minecraft.world.attribute.EnvironmentAttributeSystem attributes;
        @Override public int getNextEntityId() { return ++nextEntityId; }
        // The overworld's dimension attributes and timelines on a clock at the
        // fixture's day time (6000); biome and weather layers are not modelled.
        @Override public net.minecraft.world.attribute.EnvironmentAttributeSystem environmentAttributes() {
            if (attributes == null) {
                var builder = net.minecraft.world.attribute.EnvironmentAttributeSystem.builder().addConstantLayer(OVERWORLD_TYPE.attributes());
                OVERWORLD_TYPE.timelines().forEach(timeline -> builder.addTimelineLayer(timeline, clock -> DAY));
                attributes = builder.build();
            }
            return attributes;
        }
        @Override public net.minecraft.world.level.dimension.DimensionType dimensionType() { return type != null ? type : OVERWORLD_TYPE; }
        /**
         * ServerLevel.explode without the players to send the packet to: the
         * ServerExplosion runs, then the explosion sound the client plays at
         * the centre is logged.
         */
        @Override public void explode(Entity source, net.minecraft.world.damagesource.DamageSource damageSource,
            net.minecraft.world.level.ExplosionDamageCalculator calculator, double x, double y, double z, float r, boolean fire,
            net.minecraft.world.level.Level.ExplosionInteraction type, net.minecraft.core.particles.ParticleOptions small,
            net.minecraft.core.particles.ParticleOptions large, net.minecraft.util.random.WeightedList<net.minecraft.core.particles.ExplosionParticleInfo> blockParticles,
            net.minecraft.core.Holder<SoundEvent> sound) {
            java.util.function.Function<net.minecraft.world.level.gamerules.GameRule<Boolean>, net.minecraft.world.level.Explosion.BlockInteraction> destroy =
                rule -> rules.get(rule) ? net.minecraft.world.level.Explosion.BlockInteraction.DESTROY_WITH_DECAY : net.minecraft.world.level.Explosion.BlockInteraction.DESTROY;
            var interaction = switch (type) {
                case NONE -> net.minecraft.world.level.Explosion.BlockInteraction.KEEP;
                case BLOCK -> destroy.apply(GameRules.BLOCK_EXPLOSION_DROP_DECAY);
                case MOB -> rules.get(GameRules.MOB_GRIEFING) ? destroy.apply(GameRules.MOB_EXPLOSION_DROP_DECAY) : net.minecraft.world.level.Explosion.BlockInteraction.KEEP;
                case TNT -> destroy.apply(GameRules.TNT_EXPLOSION_DROP_DECAY);
                case TRIGGER -> net.minecraft.world.level.Explosion.BlockInteraction.TRIGGER_BLOCK;
            };
            new net.minecraft.world.level.ServerExplosion(this, source, damageSource, calculator, new Vec3(x, y, z), r, fire, interaction).explode();
            if (source == null || !source.isSilent()) emit("S " + pos(BlockPos.containing(x, y, z)) + " " + sound.value().location());
        }
        /** The overworld (fire in an obsidian frame lights a portal here). */
        @Override public net.minecraft.resources.ResourceKey<net.minecraft.world.level.Level> dimension() { return dimensionKey != null ? dimensionKey : net.minecraft.world.level.Level.OVERWORLD; }
        @Override public long getOverworldClockTime() { return DAY.totalTicks(); }
        @Override public float getMoonBrightness(BlockPos p) {
            return net.minecraft.world.level.dimension.DimensionType.MOON_BRIGHTNESS_PER_PHASE[environmentAttributes().getValue(net.minecraft.world.attribute.EnvironmentAttributes.MOON_PHASE, p).index()];
        }
        @Override public net.minecraft.world.level.storage.ServerLevelData getLevelData() { return LEVEL_DATA; }
        @Override public boolean anyPlayerCloseEnoughForSpawning(BlockPos p) { return false; }
        @Override public void playSound(Entity except, double x, double y, double z, SoundEvent sound, SoundSource source, float volume, float pitch) {
            playSeededSound(except, x, y, z, BuiltInRegistries.SOUND_EVENT.wrapAsHolder(sound), source, volume, pitch, 0L);
        }
        @Override public void playSound(Entity except, double x, double y, double z, net.minecraft.core.Holder<SoundEvent> sound, SoundSource source, float volume, float pitch) {
            playSeededSound(except, x, y, z, sound, source, volume, pitch, 0L);
        }
        @Override public void playSound(Entity except, Entity src, SoundEvent sound, SoundSource source, float volume, float pitch) {
            playSeededSound(except, src, BuiltInRegistries.SOUND_EVENT.wrapAsHolder(sound), source, volume, pitch, 0L);
        }
    }

    /** A block matcher over explicit positions; outside the build height every block reads as air. */
    static final class Scan extends net.minecraft.world.level.blockscan.OrderedBlockMatcher {
        final World world;
        Iterable<BlockPos> positions;
        Scan(World world, Iterable<BlockPos> positions) {
            super(world, positions);
            this.world = world;
            this.positions = positions;
        }
        @Override public Scan filterPos(java.util.function.Predicate<? super BlockPos> predicate) {
            positions = com.google.common.collect.Iterables.filter(positions, predicate::test);
            return this;
        }
        @Override public Scan filterState(java.util.function.Predicate<BlockState> predicate) {
            super.filterState(predicate);
            return this;
        }
        boolean scan(java.util.function.Predicate<BlockState> predicate, net.minecraft.world.level.blockscan.BlockStateConsumer consumer) {
            for (BlockPos p : positions) {
                BlockState s = p.getY() >= world.getMinY() && p.getY() <= world.getMaxY() ? world.getBlockState(p) : AIR;
                if (predicate.test(s) && consumer.apply(p, s).shouldAbort()) return true;
            }
            return false;
        }
        @Override public boolean atLeastMatched(int n) {
            int[] count = {0};
            return scan(statePredicate, (p, s) -> net.minecraft.util.Continuation.abortIf(++count[0] >= n));
        }
        @Override protected boolean anyMatched(java.util.function.Predicate<BlockState> predicate) {
            return scan(predicate, (p, s) -> net.minecraft.util.Continuation.ABORT);
        }
        @Override public void forEach(java.util.function.BiConsumer<BlockPos, BlockState> consumer) {
            scan(statePredicate, (p, s) -> { consumer.accept(p, s); return net.minecraft.util.Continuation.CONTINUE; });
        }
        @Override public boolean forEachUntil(net.minecraft.world.level.blockscan.BlockStateConsumer consumer) {
            return scan(statePredicate, consumer);
        }
    }

    static net.minecraft.world.level.dimension.DimensionType OVERWORLD_TYPE;
    static final net.minecraft.world.clock.ClockInstance DAY = new net.minecraft.world.clock.ClockInstance() {
        @Override public long totalTicks() { return 6000L; }
        @Override public float partialTick() { return 0f; }
        @Override public float rate() { return 1f; }
        @Override public boolean isPaused() { return false; }
    };
    static final net.minecraft.world.level.storage.ServerLevelData LEVEL_DATA = (net.minecraft.world.level.storage.ServerLevelData) java.lang.reflect.Proxy.newProxyInstance(
        VanillaBlockUpdateOracle.class.getClassLoader(), new Class<?>[] {net.minecraft.world.level.storage.ServerLevelData.class}, (proxy, method, values) -> {
            Class<?> type = method.getReturnType();
            if (type == long.class) return 0L;
            if (type == int.class) return 0;
            if (type == boolean.class) return false;
            if (type == float.class) return 0f;
            if (type == double.class) return 0d;
            if (type == net.minecraft.world.Difficulty.class) return net.minecraft.world.Difficulty.NORMAL;
            return null;
        });

    /** The overworld dimension type from the vanilla data pack (its infiniburn tag, attributes and timelines). */
    static net.minecraft.world.level.dimension.DimensionType loadOverworld() {
        return loadDimensionType(net.minecraft.world.level.dimension.BuiltinDimensionTypes.OVERWORLD);
    }

    static net.minecraft.world.level.dimension.DimensionType loadDimensionType(net.minecraft.resources.ResourceKey<net.minecraft.world.level.dimension.DimensionType> key) {
        var repo = net.minecraft.server.packs.repository.ServerPacksSource.createVanillaTrustedRepository();
        repo.reload();
        repo.setSelected(List.of("vanilla"));
        var resources = new net.minecraft.server.packs.resources.MultiPackResourceManager(net.minecraft.server.packs.PackType.SERVER_DATA, repo.openAllSelected());
        var layers = net.minecraft.server.RegistryLayer.createRegistryAccess();
        var staticTags = net.minecraft.tags.TagLoader.loadTagsForExistingRegistries(resources, layers.getLayer(net.minecraft.server.RegistryLayer.STATIC));
        var context = net.minecraft.tags.TagLoader.buildUpdatedLookups(layers.getAccessForLoading(net.minecraft.server.RegistryLayer.WORLD), staticTags);
        var world = net.minecraft.resources.RegistryDataLoader.load(resources, context, net.minecraft.resources.RegistryDataLoader.WORLD_REGISTRIES, Runnable::run).join();
        staticTags.forEach(net.minecraft.core.Registry.PendingTags::apply);
        return world.lookupOrThrow(net.minecraft.core.registries.Registries.DIMENSION_TYPE).getOrThrow(key).value();
    }

    static final class Loot extends net.minecraft.server.ReloadableServerRegistries.Holder {
        World world;
        /** The real loot tables, for the oracles that compare drops (the dispenser's). */
        net.minecraft.server.ReloadableServerRegistries.Holder delegate;
        Loot() {
            super(null);
        }
        @Override
        public net.minecraft.world.level.storage.loot.LootTable getLootTable(net.minecraft.resources.ResourceKey<net.minecraft.world.level.storage.loot.LootTable> id) {
            world.emit("D " + id.identifier());
            return delegate != null ? delegate.getLootTable(id) : net.minecraft.world.level.storage.loot.LootTable.EMPTY;
        }
        /** The registries loot contexts resolve providers in (a composter's compostable layers). */
        @Override
        public net.minecraft.core.HolderLookup.Provider lookup() {
            return delegate != null ? delegate.lookup() : ParityData.lookup;
        }
    }

    static final class Server extends net.minecraft.server.MinecraftServer {
        Loot loot;
        Server() {
            super(null, null, null, null, null, null, null, null, null, false, null);
        }
        /** Crafting recipes, for the oracles that craft (the crafter); null elsewhere. */
        static net.minecraft.world.item.crafting.RecipeManager recipes;
        @Override public net.minecraft.world.item.crafting.RecipeManager getRecipeManager() { return recipes; }
        @Override public net.minecraft.server.ReloadableServerRegistries.Holder reloadableRegistries() { return loot; }
        @Override protected boolean initServer() { return false; }
        /** The levels by dimension (the portal oracle's two). */
        Map<net.minecraft.resources.ResourceKey<net.minecraft.world.level.Level>, net.minecraft.server.level.ServerLevel> levels;
        @Override public net.minecraft.server.level.ServerLevel getLevel(net.minecraft.resources.ResourceKey<net.minecraft.world.level.Level> key) { return levels == null ? null : levels.get(key); }
        /** The player list of the oracles whose player is an operator; null elsewhere. */
        static net.minecraft.server.players.PlayerList players;
        @Override public net.minecraft.server.players.PlayerList getPlayerList() { return players; }
        @Override public int getMaxPlayers() { return 0; }
        @Override public net.minecraft.server.permissions.LevelBasedPermissionSet operatorUserPermissions() { return null; }
        @Override public net.minecraft.server.permissions.PermissionSet getFunctionCompilationPermissions() { return null; }
        @Override public boolean shouldRconBroadcast() { return false; }
        @Override protected net.minecraft.util.debugchart.SampleLogger getTickTimeLogger() { return null; }
        @Override public boolean isTickTimeLoggingEnabled() { return false; }
        @Override public net.minecraft.SystemReport fillServerSystemReport(net.minecraft.SystemReport r) { return r; }
        @Override public boolean isDedicatedServer() { return true; }
        @Override public int getRateLimitPacketsPerSecond() { return 0; }
        @Override public int getCommandSpamThresholdSeconds() { return 0; }
        @Override public int getChatSpamThresholdSeconds() { return 0; }
        @Override public boolean useNativeTransport() { return false; }
        @Override public boolean isPublished() { return false; }
        @Override public boolean shouldInformAdmins() { return false; }
        @Override public boolean isSingleplayerOwner(net.minecraft.server.players.NameAndId id) { return false; }
        net.minecraft.world.RandomSequences sequences;
        /** Loot tables with a random_sequence draw from the server's sequences (world seed 0). */
        @Override public RandomSource getRandomSequence(net.minecraft.resources.Identifier key) {
            if (sequences == null) sequences = new net.minecraft.world.RandomSequences();
            return sequences.get(key, 0L);
        }
    }

    static List<String> summary(World w, Map<BlockPos, BlockState> before) {
        List<String> out = new ArrayList<>();
        Set<BlockPos> all = new HashSet<>(before.keySet());
        all.addAll(w.cells.keySet());
        List<BlockPos> sorted = new ArrayList<>(all);
        sorted.sort(Comparator.<BlockPos>comparingInt(q -> q.getX()).thenComparingInt(q -> q.getY()).thenComparingInt(q -> q.getZ()));
        for (BlockPos p : sorted) {
            BlockState a = before.getOrDefault(p, AIR), b = w.cells.getOrDefault(p, AIR);
            if (a != b) {
                out.add("set " + pos(p) + " " + Block.getId(b));
            }
        }
        return out;
    }

    @SuppressWarnings("unchecked")
    static void pending(World w, StringBuilder out) throws Exception {
        for (var pair : List.of(new Object[]{"bt", w.blockTicks}, new Object[]{"ft", w.fluidTicks})) {
            Field f = LevelTicks.class.getDeclaredField("allContainers");
            f.setAccessible(true);
            var map = (it.unimi.dsi.fastutil.longs.Long2ObjectMap<net.minecraft.world.ticks.LevelChunkTicks<Object>>) f.get(pair[1]);
            List<String> rows = new ArrayList<>();
            for (var container : map.values()) {
                container.getAll().forEach(t -> {
                    String type = t.type() instanceof Block b ? BuiltInRegistries.BLOCK.getKey(b).toString() : BuiltInRegistries.FLUID.getKey((Fluid) t.type()).toString();
                    rows.add(pair[0] + " " + pos(t.pos()) + " " + type + " " + (t.triggerTick() - w.time) + " " + t.priority().getValue());
                });
            }
            Collections.sort(rows);
            for (String r : rows) out.append(r).append('\n');
        }
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        AIR = Blocks.AIR.defaultBlockState();
        Path resources = Path.of(args[0]);
        VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(resources.resolve("block-tags.json"))).getAsJsonObject());
        VanillaLevelShapeOracle.bindFluidTags(resources.resolve("data/minecraft/tags/fluid"));
        OVERWORLD_TYPE = loadOverworld();
        var allocator = Class.forName("sun.misc.Unsafe");
        var field = allocator.getDeclaredField("theUnsafe");
        field.setAccessible(true);
        unsafe = field.get(null);
        allocate = allocator.getMethod("allocateInstance", Class.class);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[2]))); var lines = Files.lines(Path.of(args[1]))) {
            int index = 0;
            for (var line : (Iterable<String>) lines::iterator) {
                var f = JsonParser.parseString(line).getAsJsonObject();
                StringBuilder result = new StringBuilder("case " + index++ + "\n");
                try {
                    run(f, result);
                } catch (Throwable e) {
                    Throwable c = e;
                    while (c.getCause() != null) c = c.getCause();
                    StackTraceElement[] st = c.getStackTrace();
                    result.append("error ").append(c.getClass().getSimpleName()).append(' ').append(st.length > 0 ? st[0].toString() : "").append('\n');
                }
                out.print(result);
            }
        }
    }

    static BlockPos readPos(JsonArray a) {
        return new BlockPos(a.get(0).getAsInt(), a.get(1).getAsInt(), a.get(2).getAsInt());
    }

    /**
     * The chunk source bone meal reaches for: saplings, grass and moss place
     * their features with `getChunkSource().getGenerator()`, which none of
     * them reads beyond passing it along.
     */
    static final class Chunks extends net.minecraft.server.level.ServerChunkCache {
        static net.minecraft.world.level.chunk.ChunkGenerator generator;

        private Chunks() {
            super(null, null, null, null, null, null, 0, 0, false, null);
        }

        @Override
        public net.minecraft.world.level.chunk.ChunkGenerator getGenerator() {
            return generator;
        }

        /** Leads send entity link packets; nobody is tracking. */
        @Override
        public void sendToTrackingPlayers(final net.minecraft.world.entity.Entity entity, final net.minecraft.network.protocol.Packet<? super net.minecraft.network.protocol.game.ClientGamePacketListener> packet) {
        }

        /** Entity events (an armor stand's wobble) go to the trackers and the entity itself; nobody is there. */
        @Override
        public void sendToTrackingPlayersAndSelf(final net.minecraft.world.entity.Entity entity, final net.minecraft.network.protocol.Packet<? super net.minecraft.network.protocol.game.ClientGamePacketListener> packet) {
        }
    }

    static World build(JsonObject f) throws Exception {
        World w = make(World.class);
        // Saplings, grass and moss place features through the chunk source's generator.
        if (Chunks.generator == null) Chunks.generator = make(net.minecraft.world.level.levelgen.FlatLevelSource.class);
        set(net.minecraft.server.level.ServerLevel.class, w, "chunkSource", make(Chunks.class));
        w.cells = new HashMap<>();
        w.log = new StringBuilder();
        w.blockEvents = new java.util.LinkedHashSet<>();
        for (var cell : f.getAsJsonArray("cells")) {
            var c = cell.getAsJsonArray();
            BlockState s = Block.stateById(c.get(3).getAsInt());
            if (s != AIR) w.cells.put(new BlockPos(c.get(0).getAsInt(), c.get(1).getAsInt(), c.get(2).getAsInt()), s);
        }
        w.rng = new LegacyRandomSource(f.get("seed").getAsLong());
        w.time = f.has("time") ? f.get("time").getAsLong() : 0;
        w.rules = new GameRules(FeatureFlags.DEFAULT_FLAGS);
        if (f.has("light")) {
            w.skyLight = f.getAsJsonArray("light").get(0).getAsInt();
            w.blockLight = f.getAsJsonArray("light").get(1).getAsInt();
        }
        w.raining = f.has("raining") && f.get("raining").getAsBoolean();
        w.thundering = f.has("thundering") && f.get("thundering").getAsBoolean();
        set(Level.class, w, "random", w.rng);
        // A server level chains up to max-chained-neighbor-updates (1,000,000 by default).
        set(Level.class, w, "neighborUpdater", new net.minecraft.world.level.redstone.CollectingNeighborUpdater(w, 1000000));
        w.server = make(Server.class);
        w.server.loot = make(Loot.class);
        w.server.loot.world = w;
        w.chunk = make(Chunk.class);
        w.chunk.world = w;
        w.blockTicks = new LevelTicks<>(l -> true);
        w.fluidTicks = new LevelTicks<>(l -> true);
        for (int x = -3; x <= 3; x++) {
            for (int z = -3; z <= 3; z++) {
                w.blockTicks.addContainer(new net.minecraft.world.level.ChunkPos(x, z), new net.minecraft.world.ticks.LevelChunkTicks<>());
                w.fluidTicks.addContainer(new net.minecraft.world.level.ChunkPos(x, z), new net.minecraft.world.ticks.LevelChunkTicks<>());
            }
        }
        return w;
    }

    static void run(JsonObject f, StringBuilder result) throws Exception {
        World w = build(f);
        Map<BlockPos, BlockState> before = new HashMap<>(w.cells);
        String mode = f.get("mode").getAsString();
        BlockPos p = readPos(f.getAsJsonArray("pos"));
        BlockState state = w.getBlockState(p);
        switch (mode) {
            case "shape" -> {
                Direction d = Direction.from3DDataValue(f.get("dir").getAsInt());
                BlockPos np = p.relative(d);
                BlockState n = Block.stateById(f.get("neighbor").getAsInt());
                BlockState s = state.updateShape(w, w, p, d, np, n, w.rng);
                result.append("shape ").append(Block.getId(s)).append('\n');
            }
            case "random" -> {
                if (state.isRandomlyTicking()) state.randomTick(w, p, w.rng);
                FluidState fs = state.getFluidState();
                if (fs.isRandomlyTicking()) fs.randomTick(w, p, w.rng);
            }
            case "tick" -> state.tick(w, p, w.rng);
            case "ftick" -> state.getFluidState().tick(w, p, state);
            case "neighbor" -> {
                BlockPos from = readPos(f.getAsJsonArray("from"));
                w.getBlockState(from);
                state.handleNeighborChanged(w, p, w.getBlockState(from).getBlock(), null, false);
            }
            case "run" -> {
                for (var op : f.getAsJsonArray("ops")) {
                    var o = op.getAsJsonObject();
                    switch (o.get("op").getAsString()) {
                        case "set" -> w.setBlock(readPos(o.getAsJsonArray("pos")), Block.stateById(o.get("state").getAsInt()), o.get("flags").getAsInt());
                        case "ticks" -> {
                            for (int i = 0; i < o.get("n").getAsInt(); i++) {
                                w.time++;
                                w.blockTicks.tick(w.time, 65536, (bp, b) -> { if (w.getBlockState(bp).is(b)) w.getBlockState(bp).tick(w, bp, w.rng); });
                                w.fluidTicks.tick(w.time, 65536, (bp, fl) -> {
                                    if (w.getFluidState(bp).getType() == fl) w.getFluidState(bp).tick(w, bp, w.getBlockState(bp));
                                });
                                if (o.has("snap") && o.get("snap").getAsBoolean()) {
                                    result.append("tick ").append(w.time).append('\n');
                                    for (String s : summary(w, before)) result.append(s).append('\n');
                                }
                            }
                        }
                        default -> throw new IllegalStateException(o.toString());
                    }
                }
            }
            default -> throw new IllegalStateException(mode);
        }
        for (String s : summary(w, before)) result.append(s).append('\n');
        pending(w, result);
        result.append(w.log);
        for (String s : w.blockEvents) result.append(s).append('\n');
        result.append("rng ").append(w.rng.nextLong()).append('\n');
    }
}
