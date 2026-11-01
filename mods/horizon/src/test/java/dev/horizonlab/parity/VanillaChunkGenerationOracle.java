package dev.horizonlab.parity;

import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.CompletableFuture;
import net.minecraft.SharedConstants;
import net.minecraft.core.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.level.GenerationChunkHolder;
import net.minecraft.server.level.ServerChunkCache;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.WorldGenRegion;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.tags.TagLoader;
import net.minecraft.util.StaticCache2D;
import net.minecraft.util.datafix.DataFixers;
import net.minecraft.world.level.*;
import net.minecraft.world.level.biome.*;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.chunk.*;
import net.minecraft.world.level.chunk.status.ChunkPyramid;
import net.minecraft.world.level.chunk.status.ChunkStatus;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.blending.Blender;
import net.minecraft.world.level.levelgen.blending.BlendingData;
import net.minecraft.world.level.levelgen.structure.*;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplateManager;
import net.minecraft.world.level.storage.LevelStorageSource;

/**
 * Original oracle: whole-chunk generation through vanilla's own pipeline.
 *
 * Chunks run through STRUCTURE_STARTS, STRUCTURE_REFERENCES, BIOMES, TERRAIN
 * (noise, surface, carvers) and FEATURES exactly as the ChunkStatusTasks do,
 * with actual WorldGenRegion instances over an allocation-only ServerLevel.
 * FEATURES run in row-major order over the target rectangle grown by one chunk,
 * so every target chunk has all eight neighbours decorated. Spill from
 * neighbouring features therefore matches a canonical vanilla order.
 *
 * usage: dump OUT SEED DIMENSION X0,Z0,X1,Z1...  (target chunks, inclusive)
 *        scan OUT SEED DIMENSION X0 Z0 X1 Z1  (structure starts per chunk)
 * dump takes any number of X0,Z0,X1,Z1 rectangles, each generated from scratch.
 */
public final class VanillaChunkGenerationOracle {
    private static final class OracleCache extends ServerChunkCache {
        private OracleCache() {
            super(null, null, null, null, null, null, 0, 0, false, null);
        }
        static ChunkGeneratorStructureState state;
        static RandomState random;
        static ChunkGenerator generator;
        @Override
        public ChunkGeneratorStructureState getGeneratorState() {
            return state;
        }
        @Override
        public RandomState randomState() {
            return random;
        }
        @Override
        public ChunkGenerator getGenerator() {
            return generator;
        }
    }

    private static final class OracleLevel extends ServerLevel {
        private OracleLevel() {
            super(null, null, null, null, null, null, false, 0L, null, false);
        }
        static long seed;
        static int nextEntityId;
        @Override
        public int getNextEntityId() {
            return nextEntityId++;
        }
        @Override
        public float getMoonBrightness(final BlockPos pos) {
            return 0.0F;
        }
        @Override
        public long getOverworldClockTime() {
            return 0L;
        }
        static RegistryAccess access;
        static net.minecraft.world.level.dimension.DimensionType type;
        static StructureManager structures;
        static BiomeResolver resolver;
        static ServerChunkCache cache;
        static net.minecraft.server.MinecraftServer server;
        static ResourceKey<Level> key;
        @Override
        public net.minecraft.server.MinecraftServer getServer() {
            return server;
        }
        static int minY, height;
        static Map<Long, ProtoChunk> chunks;
        static net.minecraft.world.level.lighting.LevelLightEngine light;
        @Override
        public net.minecraft.world.level.lighting.LevelLightEngine getLightEngine() {
            return light;
        }
        @Override
        public long getSeed() {
            return seed;
        }
        @Override
        public net.minecraft.world.flag.FeatureFlagSet enabledFeatures() {
            return net.minecraft.world.flag.FeatureFlags.DEFAULT_FLAGS; // with none enabled, holderLookup filters every block out and jigsaw final states fail to parse
        }
        @Override
        public RegistryAccess registryAccess() {
            return access;
        }
        @Override
        public net.minecraft.world.level.dimension.DimensionType dimensionType() {
            return type;
        }
        @Override
        public StructureManager structureManager() {
            return structures;
        }
        @Override
        public BiomeResolver uncachedBiomeResolver() {
            return resolver;
        }
        @Override
        public ServerChunkCache getChunkSource() {
            return cache;
        }
        @Override
        public ResourceKey<Level> dimension() {
            return key;
        }
        @Override
        public net.minecraft.world.level.storage.ServerLevelData getLevelData() {
            return DATA;
        }
        static final net.minecraft.world.level.storage.ServerLevelData DATA = (net.minecraft.world.level.storage.ServerLevelData) java.lang.reflect.Proxy.newProxyInstance(
            OracleLevel.class.getClassLoader(), new Class<?>[] {net.minecraft.world.level.storage.ServerLevelData.class}, (proxy, method, values) -> {
                Class<?> type = method.getReturnType();
                if (type == long.class) return 0L;
                if (type == int.class) return 0;
                if (type == boolean.class) return false;
                if (type == float.class) return 0f;
                if (type == net.minecraft.world.Difficulty.class) return net.minecraft.world.Difficulty.NORMAL;
                if (type == double.class) return 0d;
                return null;
            });
        @Override
        public int getMinY() {
            return minY;
        }
        @Override
        public int getHeight() {
            return height;
        }
        @Override
        public int getSeaLevel() {
            return OracleCache.generator.getSeaLevel();
        }
        @Override
        public void updatePOIOnBlockStateChange(net.minecraft.core.BlockPos pos, net.minecraft.world.level.block.state.BlockState a, net.minecraft.world.level.block.state.BlockState b) {}
        @Override
        public ChunkAccess getChunk(int x, int z, ChunkStatus status, boolean load) {
            return chunks.get(ChunkPos.pack(x, z));
        }
    }

    private static final class ChunkHolder extends GenerationChunkHolder {
        final ProtoChunk chunk;
        ChunkHolder(ProtoChunk chunk) {
            super(chunk.getPos());
            this.chunk = chunk;
        }
        @Override
        public ChunkAccess getChunkIfPresentUnchecked(ChunkStatus status) {
            return status.isOrBefore(chunk.getPersistedStatus()) ? chunk : null;
        }
        @Override
        protected void addSaveDependency(CompletableFuture<?> sync) {}
        @Override
        public int getTicketLevel() {
            return 0;
        }
        @Override
        public int getQueueLevel() {
            return 0;
        }
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        // A crash report of a chunk request vanilla refused names the chunk and its distance.
        Thread.setDefaultUncaughtExceptionHandler((t, e) -> {
            for (Throwable c = e; c != null; c = c.getCause()) {
                if (c instanceof net.minecraft.ReportedException r) System.err.println(r.getReport().getFriendlyReport(net.minecraft.ReportType.CRASH));
            }
            e.printStackTrace();
        });
        String mode = args[0];
        Path out = Path.of(args[1]);
        long seed = Long.parseLong(args[2]);
        String dimension = args[3];
        // HORIZON_DATAPACKS: a world's datapacks folder, every pack in it enabled after vanilla in name order.
        String datapacks = System.getenv("HORIZON_DATAPACKS");
        var repo = datapacks == null ? ServerPacksSource.createVanillaTrustedRepository()
            : ServerPacksSource.createPackRepository(Path.of(datapacks), new net.minecraft.world.level.validation.DirectoryValidator(path -> true));
        repo.reload();
        var selected = new ArrayList<String>(List.of("vanilla"));
        repo.getAvailableIds().stream().filter(id -> id.startsWith("file/")).sorted().forEach(selected::add);
        repo.setSelected(selected);
        if (!repo.getSelectedIds().containsAll(selected)) throw new IllegalStateException("packs not selected: " + selected + " vs " + repo.getSelectedIds());
        var resources = new MultiPackResourceManager(PackType.SERVER_DATA, repo.openAllSelected());
        var layers = RegistryLayer.createRegistryAccess();
        var staticTags = TagLoader.loadTagsForExistingRegistries(resources, layers.getLayer(RegistryLayer.STATIC));
        var context = TagLoader.buildUpdatedLookups(layers.getAccessForLoading(RegistryLayer.WORLD), staticTags);
        RegistryAccess.Frozen world = RegistryDataLoader.load(resources, context, RegistryDataLoader.WORLD_REGISTRIES, Runnable::run).join();
        var loaded = net.minecraft.server.ReloadableServerRegistries.reload(layers.replaceFrom(RegistryLayer.WORLD, world), staticTags, resources, Runnable::run).join();
        var components = BuiltInRegistries.DATA_COMPONENT_INITIALIZERS.build(loaded.lookupWithUpdatedTags());
        staticTags.forEach(Registry.PendingTags::apply);
        components.forEach(net.minecraft.core.component.DataComponentInitializers.PendingComponents::apply);
        RegistryAccess access = loaded.layers().compositeAccess();
        var settings = world.lookupOrThrow(Registries.NOISE_SETTINGS).getOrThrow(switch (dimension) {
            case "the_nether" -> NoiseGeneratorSettings.NETHER;
            case "the_end" -> NoiseGeneratorSettings.END;
            default -> NoiseGeneratorSettings.OVERWORLD;
        });
        var presets = world.lookupOrThrow(Registries.MULTI_NOISE_BIOME_SOURCE_PARAMETER_LIST);
        BiomeSource biomeSource = switch (dimension) {
            case "the_nether" -> MultiNoiseBiomeSource.createFromPreset(presets.getOrThrow(MultiNoiseBiomeSourceParameterLists.NETHER));
            case "the_end" -> TheEndBiomeSource.create(world.lookupOrThrow(Registries.BIOME));
            default -> MultiNoiseBiomeSource.createFromPreset(presets.getOrThrow(MultiNoiseBiomeSourceParameterLists.OVERWORLD));
        };
        ResourceKey<Level> levelKey = switch (dimension) {
            case "the_nether" -> Level.NETHER;
            case "the_end" -> Level.END;
            default -> Level.OVERWORLD;
        };
        var randomState = RandomState.create(world.lookupOrThrow(Registries.NOISE), seed, settings.value());
        var generator = new NoiseBasedChunkGenerator(biomeSource, settings);
        String phase = System.getenv().getOrDefault("HORIZON_PHASE", "full");
        if (phase.equals("structures")) {
            // Structure pieces only: every biome keeps its carvers but loses its placed features.
            java.util.function.Function<Holder<Biome>, BiomeGenerationSettings> noFeatures = biome -> {
                var builder = new BiomeGenerationSettings.PlainBuilder();
                for (var carver : biome.value().getGenerationSettings().getCarvers()) builder.addCarver(carver);
                return builder.build();
            };
            var field = ChunkGenerator.class.getDeclaredField("generationSettingsGetter");
            field.setAccessible(true);
            field.set(generator, noFeatures);
        }
        var state = ChunkGeneratorStructureState.createForNormal(randomState, seed, new ChunkPos(0, 0), biomeSource, world.lookupOrThrow(Registries.STRUCTURE_SET));
        Path tmp = Files.createTempDirectory("horizon-chunks");
        var storage = LevelStorageSource.createDefault(tmp).createAccess("oracle");
        var templates = new StructureTemplateManager(resources, storage, DataFixers.getDataFixer(), BuiltInRegistries.BLOCK);
        var dimensionType = world.lookupOrThrow(Registries.DIMENSION_TYPE).getOrThrow(switch (dimension) {
            case "the_nether" -> net.minecraft.world.level.dimension.BuiltinDimensionTypes.NETHER;
            case "the_end" -> net.minecraft.world.level.dimension.BuiltinDimensionTypes.END;
            default -> net.minecraft.world.level.dimension.BuiltinDimensionTypes.OVERWORLD;
        }).value();
        // The level's build range is its dimension type's: a data pack's when one changes it.
        LevelHeightAccessor height = LevelHeightAccessor.create(dimensionType.minY(), dimensionType.height());
        var factory = PalettedContainerFactory.create(access);
        var unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        Object unsafe = unsafeField.get(null);
        var allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        Map<Long, ProtoChunk> chunks = new HashMap<>();
        OracleCache.state = state;
        OracleCache.random = randomState;
        OracleCache.generator = generator;
        OracleLevel.seed = seed;
        OracleLevel.access = access;
        OracleLevel.type = dimensionType;
        OracleLevel.resolver = biomeSource.createUncachedResolver(randomState);
        OracleLevel.cache = (ServerChunkCache) allocate.invoke(unsafe, OracleCache.class);
        OracleLevel.server = (net.minecraft.server.MinecraftServer) allocate.invoke(unsafe, net.minecraft.server.dedicated.DedicatedServer.class);
        var templateField = net.minecraft.server.MinecraftServer.class.getDeclaredField("structureTemplateManager");
        templateField.setAccessible(true);
        templateField.set(OracleLevel.server, templates);
        OracleLevel.key = levelKey;
        OracleLevel.minY = height.getMinY();
        OracleLevel.height = height.getHeight();
        OracleLevel.chunks = chunks;
        var blockGetter = (BlockGetter) java.lang.reflect.Proxy.newProxyInstance(BlockGetter.class.getClassLoader(), new Class<?>[] {BlockGetter.class}, (proxy, method, values) -> {
            switch (method.getName()) {
                case "getMinY": return height.getMinY();
                case "getHeight": return height.getHeight();
            }
            if (method.isDefault()) return java.lang.reflect.InvocationHandler.invokeDefault(proxy, method, values);
            return null;
        });
        var lightGetter = new LightChunkGetter() {
            public LightChunk getChunkForLighting(int x, int z) { return null; }
            public BlockGetter getLevel() { return blockGetter; }
        };
        // Features run before any light exists in a real server, so every brightness query reads 0 (an empty engine would claim full sky light).
        OracleLevel.light = new net.minecraft.world.level.lighting.LevelLightEngine(lightGetter, true, dimensionType.hasSkyLight()) {
            @Override
            public int getRawBrightness(final net.minecraft.core.BlockPos pos, final int skyDampen) {
                return 0;
            }
        };
        var check = new StructureCheck((pos, visitor) -> CompletableFuture.completedFuture(null), access, templates, levelKey, generator, randomState, height, biomeSource, seed, DataFixers.getDataFixer());
        var level = (ServerLevel) allocate.invoke(unsafe, OracleLevel.class);
        OracleLevel.structures = new StructureManager(level, new WorldOptions(seed, true, false), check);
        var pyramid = ChunkPyramid.GENERATION_PYRAMID;
        // HORIZON_OLD_CHUNKS=SEED:x0,z0,x1,z1: chunks another seed generated, which count as saved by an older
        // version (blending data, no worldgen heightmap) and which the target chunks blend with.
        List<Long> oldChunks = new ArrayList<>();
        String oldSpec = System.getenv("HORIZON_OLD_CHUNKS");
        long oldSeed = 0;
        if (oldSpec != null) {
            oldSeed = Long.parseLong(oldSpec.substring(0, oldSpec.indexOf(':')));
            String[] c = oldSpec.substring(oldSpec.indexOf(':') + 1).split(",");
            for (int x = Integer.parseInt(c[0]); x <= Integer.parseInt(c[2]); x++) for (int z = Integer.parseInt(c[1]); z <= Integer.parseInt(c[3]); z++) oldChunks.add(ChunkPos.pack(x, z));
        }
        java.util.function.BiFunction<Integer, Integer, ProtoChunk> chunkAt = (x, z) -> chunks.computeIfAbsent(ChunkPos.pack(x, z), k -> new ProtoChunk(new ChunkPos(x, z), UpgradeData.EMPTY, height, factory, null));
        java.util.function.BiFunction<ChunkStatus, ProtoChunk, WorldGenRegion> regionFor = (status, chunk) -> {
            var step = pyramid.getStepTo(status);
            int radius = step.directDependencies().size() - 1;
            var cache = StaticCache2D.<GenerationChunkHolder>create(chunk.getPos().x(), chunk.getPos().z(), radius, (x, z) -> new ChunkHolder(chunkAt.apply(x, z)));
            return new WorldGenRegion(level, cache, step, chunk) {
                @Override
                public boolean isOldChunkAround(final ChunkPos pos, final int range) {
                    for (long key : oldChunks) {
                        if (Math.abs(ChunkPos.getX(key) - pos.x()) <= range && Math.abs(ChunkPos.getZ(key) - pos.z()) <= range) return true;
                    }
                    return false;
                }
            };
        };
        Runnable noop = () -> {};
        try (var w = new PrintWriter(Files.newBufferedWriter(out, StandardCharsets.UTF_8))) {
            if (mode.equals("random")) {
                // diagnostics: first floats after setFeatureSeed for chunk X0,Z0 (args 4..) index step
                int cx = Integer.parseInt(args[4]), cz = Integer.parseInt(args[5]), idx = Integer.parseInt(args[6]), step = Integer.parseInt(args[7]);
                WorldgenRandom r = new WorldgenRandom(new XoroshiroRandomSource(0L));
                long ds = r.setDecorationSeed(seed, cx << 4, cz << 4);
                r.setFeatureSeed(ds, idx, step);
                w.println("random " + ds + " " + r.nextFloat() + " " + r.nextFloat() + " " + r.nextFloat());
                return;
            }
            if (mode.equals("order")) {
                var byStep = new TreeMap<Integer, List<String>>();
                var registry = access.lookupOrThrow(Registries.STRUCTURE);
                registry.stream().forEach(st -> byStep.computeIfAbsent(st.step().ordinal(), k -> new ArrayList<>()).add(registry.getKey(st).toString()));
                byStep.forEach((step, names) -> { for (int i = 0; i < names.size(); i++) w.println("order " + step + " " + i + " " + names.get(i)); });
                return;
            }
            if (mode.equals("scan")) {
                int x0 = Integer.parseInt(args[4]), z0 = Integer.parseInt(args[5]), x1 = Integer.parseInt(args[6]), z1 = Integer.parseInt(args[7]);
                for (int x = x0; x <= x1; x++) for (int z = z0; z <= z1; z++) {
                    var chunk = chunkAt.apply(x, z);
                    generator.createStructures(access, state, OracleLevel.structures, chunk, templates, levelKey);
                    for (var start : chunk.getAllStarts().values()) {
                        if (!start.isValid()) continue;
                        var box = start.getBoundingBox();
                        var id = access.lookupOrThrow(Registries.STRUCTURE).getKey(start.getStructure());
                        if (System.getenv("HORIZON_PIECES") != null) for (var piece : start.getPieces()) {
                            var pb = piece.getBoundingBox();
                            w.println("piece " + x + " " + z + " " + net.minecraft.core.registries.BuiltInRegistries.STRUCTURE_PIECE.getKey(piece.getType()) + " " + pb.minX() + " " + pb.minY() + " " + pb.minZ() + " " + pb.maxX() + " " + pb.maxY() + " " + pb.maxZ() + (piece instanceof net.minecraft.world.level.levelgen.structure.PoolElementStructurePiece pool ? " " + pool.getElement() + " rot=" + pool.getRotation() : ""));
                        }
                        w.println("start " + id + " " + x + " " + z + " " + box.minX() + " " + box.minY() + " " + box.minZ() + " " + box.maxX() + " " + box.maxY() + " " + box.maxZ());
                    }
                }
                return;
            }
            for (int rect = 4; rect < args.length; rect++) {
            chunks.clear();
            if (!oldChunks.isEmpty()) {
                var oldState = RandomState.create(world.lookupOrThrow(Registries.NOISE), oldSeed, settings.value());
                int ox0 = Integer.MAX_VALUE, oz0 = Integer.MAX_VALUE, ox1 = Integer.MIN_VALUE, oz1 = Integer.MIN_VALUE;
                for (long key : oldChunks) { ox0 = Math.min(ox0, ChunkPos.getX(key)); ox1 = Math.max(ox1, ChunkPos.getX(key)); oz0 = Math.min(oz0, ChunkPos.getZ(key)); oz1 = Math.max(oz1, ChunkPos.getZ(key)); }
                // The pyramid asks structure starts of every chunk within 8 (there are none here).
                for (int z = oz0 - 9; z <= oz1 + 9; z++) for (int x = ox0 - 9; x <= ox1 + 9; x++) chunkAt.apply(x, z).setPersistedStatus(ChunkStatus.STRUCTURE_STARTS);
                for (int z = oz0 - 1; z <= oz1 + 1; z++) for (int x = ox0 - 1; x <= ox1 + 1; x++) {
                    var chunk = chunkAt.apply(x, z);
                    var region = regionFor.apply(ChunkStatus.BIOMES, chunk);
                    generator.createBiomes(oldState, Blender.empty(), OracleLevel.structures.forWorldGenRegion(region), chunk).join();
                    chunk.setPersistedStatus(ChunkStatus.BIOMES);
                }
                for (int z = oz0; z <= oz1; z++) for (int x = ox0; x <= ox1; x++) {
                    var chunk = chunkAt.apply(x, z);
                    var region = regionFor.apply(ChunkStatus.TERRAIN, chunk);
                    Set<Holder<Biome>> possible = new HashSet<>();
                    for (int dz = -1; dz <= 1; dz++) for (int dx = -1; dx <= 1; dx++) region.getChunk(x + dx, z + dz).collectBiomesInPalette(possible);
                    generator.buildTerrain(chunk, Blender.empty(), oldState, OracleLevel.structures.forWorldGenRegion(region), region.getBiomeManager(), region, possible).join();
                    Heightmap.primeHeightmaps(chunk, EnumSet.of(Heightmap.Types.MOTION_BLOCKING, Heightmap.Types.MOTION_BLOCKING_NO_LEAVES, Heightmap.Types.OCEAN_FLOOR, Heightmap.Types.WORLD_SURFACE));
                    chunk.setPersistedStatus(ChunkStatus.TERRAIN);
                }
                // The ring around the old chunks was only there for their biomes; it is generated afresh below.
                for (int z = oz0 - 9; z <= oz1 + 9; z++) for (int x = ox0 - 9; x <= ox1 + 9; x++) if (!oldChunks.contains(ChunkPos.pack(x, z))) chunks.remove(ChunkPos.pack(x, z));
                var blendingField = ChunkAccess.class.getDeclaredField("blendingData");
                blendingField.setAccessible(true);
                var heightmapsField = ChunkAccess.class.getDeclaredField("heightmaps");
                heightmapsField.setAccessible(true);
                for (long key : oldChunks) {
                    var chunk = chunks.get(key);
                    blendingField.set(chunk, BlendingData.unpack(new BlendingData.Packed(height.getMinSectionY(), height.getMaxSectionY() + 1, Optional.empty())));
                    ((Map<?, ?>) heightmapsField.get(chunk)).remove(Heightmap.Types.WORLD_SURFACE_WG);
                }
            }
            String[] corners = args[rect].split(",");
            int x0 = Integer.parseInt(corners[0]), z0 = Integer.parseInt(corners[1]), x1 = Integer.parseInt(corners[2]), z1 = Integer.parseInt(corners[3]);
            // Margins: FEATURES need TERRAIN at 1, which needs BIOMES at 1 more; STRUCTURE_STARTS reach 8 further.
            int m = System.getenv("HORIZON_ISOLATED") != null ? 0 : 1; // diagnostics: decorate the targets only
            int fx0 = x0 - m, fz0 = z0 - m, fx1 = x1 + m, fz1 = z1 + m;
            int tx0 = fx0 - 1, tz0 = fz0 - 1, tx1 = fx1 + 1, tz1 = fz1 + 1;
            int bx0 = tx0 - 1, bz0 = tz0 - 1, bx1 = tx1 + 1, bz1 = tz1 + 1;
            int sm = oldChunks.isEmpty() ? 8 : 9; // blending reads the neighbours of old chunks 7 away
            int sx0 = tx0 - sm, sz0 = tz0 - sm, sx1 = tx1 + sm, sz1 = tz1 + sm;
            for (int z = sz0; z <= sz1; z++) for (int x = sx0; x <= sx1; x++) {
                if (oldChunks.contains(ChunkPos.pack(x, z))) continue;
                var chunk = chunkAt.apply(x, z);
                generator.createStructures(access, state, OracleLevel.structures, chunk, templates, levelKey);
                chunk.setPersistedStatus(ChunkStatus.STRUCTURE_STARTS);
            }
            int rm = oldChunks.isEmpty() ? 0 : 1; // Blender.of asks for the centre chunk itself at STRUCTURE_REFERENCES
            for (int z = tz0 - rm; z <= tz1 + rm; z++) for (int x = tx0 - rm; x <= tx1 + rm; x++) {
                if (oldChunks.contains(ChunkPos.pack(x, z))) continue;
                var chunk = chunkAt.apply(x, z);
                var region = regionFor.apply(ChunkStatus.STRUCTURE_REFERENCES, chunk);
                generator.createReferences(region, OracleLevel.structures.forWorldGenRegion(region), chunk);
                chunk.setPersistedStatus(ChunkStatus.STRUCTURE_REFERENCES);
            }
            for (int z = bz0; z <= bz1; z++) for (int x = bx0; x <= bx1; x++) {
                if (oldChunks.contains(ChunkPos.pack(x, z))) continue;
                var chunk = chunkAt.apply(x, z);
                if (chunk.getPersistedStatus() != ChunkStatus.STRUCTURE_STARTS && chunk.getPersistedStatus() != ChunkStatus.EMPTY && chunk.getPersistedStatus() != ChunkStatus.STRUCTURE_REFERENCES) continue;
                var region = regionFor.apply(ChunkStatus.BIOMES, chunk);
                generator.createBiomes(randomState, Blender.of(region), OracleLevel.structures.forWorldGenRegion(region), chunk).join();
                chunk.setPersistedStatus(ChunkStatus.BIOMES);
            }
            for (int z = tz0; z <= tz1; z++) for (int x = tx0; x <= tx1; x++) {
                if (oldChunks.contains(ChunkPos.pack(x, z))) continue;
                var chunk = chunkAt.apply(x, z);
                var region = regionFor.apply(ChunkStatus.TERRAIN, chunk);
                Set<Holder<Biome>> possible = new HashSet<>();
                for (int dz = -1; dz <= 1; dz++) for (int dx = -1; dx <= 1; dx++) region.getChunk(x + dx, z + dz).collectBiomesInPalette(possible);
                generator.buildTerrain(chunk, Blender.of(region), randomState, OracleLevel.structures.forWorldGenRegion(region), region.getBiomeManager(), region, possible).join();
                Heightmap.primeHeightmaps(chunk, EnumSet.of(Heightmap.Types.MOTION_BLOCKING, Heightmap.Types.MOTION_BLOCKING_NO_LEAVES, Heightmap.Types.OCEAN_FLOOR, Heightmap.Types.WORLD_SURFACE));
                chunk.setPersistedStatus(ChunkStatus.TERRAIN);
            }
            // HORIZON_ORDER: a file of "x z" lines, the chunks to decorate in that order (default: row-major).
            List<int[]> order = new ArrayList<>();
            if (System.getenv("HORIZON_ORDER") != null) {
                for (String line : java.nio.file.Files.readAllLines(java.nio.file.Path.of(System.getenv("HORIZON_ORDER")))) {
                    String[] t = line.trim().split("\\s+");
                    if (t.length == 2) order.add(new int[]{Integer.parseInt(t[0]), Integer.parseInt(t[1])});
                }
            } else {
                for (int z = fz0; z <= fz1; z++) for (int x = fx0; x <= fx1; x++) order.add(new int[]{x, z});
            }
            for (int[] at : order) {
                if (phase.equals("terrain")) break;
                int x = at[0], z = at[1];
                if (oldChunks.contains(ChunkPos.pack(x, z))) continue; // an old chunk is not decorated, only written into
                var chunk = chunkAt.apply(x, z);
                var region = regionFor.apply(ChunkStatus.FEATURES, chunk);
                generator.applyBiomeDecoration(region, chunk, OracleLevel.structures.forWorldGenRegion(region));
                chunk.setPersistedStatus(ChunkStatus.FEATURES);
            }
            var registry = access;
            for (int z = z0; z <= z1; z++) for (int x = x0; x <= x1; x++) {
                var target = chunkAt.apply(x, z);
                w.println("chunk " + x + " " + z);
                // Biomes: one line per quart layer, 16 columns in z-major order.
                for (int qy = height.getMinY() >> 2; qy <= height.getMaxY() >> 2; qy++) {
                    StringBuilder line = new StringBuilder("q " + qy);
                    for (int qz = 0; qz < 4; qz++) for (int qx = 0; qx < 4; qx++)
                        line.append(' ').append(target.getNoiseBiome((x << 2) + qx, qy, (z << 2) + qz).unwrapKey().orElseThrow().identifier());
                    w.println(line);
                }
                for (int y = height.getMinY(); y <= height.getMaxY(); y++) {
                    StringBuilder line = new StringBuilder("y " + y);
                    for (int zz = 0; zz < 16; zz++) for (int xx = 0; xx < 16; xx++)
                        line.append(' ').append(Block.getId(target.getBlockState(new BlockPos((x << 4) + xx, y, (z << 4) + zz))));
                    w.println(line);
                }
                var positions = new TreeSet<BlockPos>(Comparator.<BlockPos>comparingInt(BlockPos::getX).thenComparingInt(BlockPos::getY).thenComparingInt(BlockPos::getZ));
                positions.addAll(target.getBlockEntitiesPos());
                for (var pos : positions) {
                    var tag = target.getBlockEntityNbtForSaving(pos, registry);
                    if (tag == null) continue;
                    var st = target.getBlockState(pos);
                    var block = (net.minecraft.world.level.block.EntityBlock) st.getBlock();
                    var fresh = block.newBlockEntity(pos, st).saveWithFullMetadata(registry);
                    // The value of each key Horizon leaves out: what loading a tag without
                    // it gives (BrewingStandBlockEntity reads a missing total_brew_time as 400).
                    var empty = block.newBlockEntity(pos, st);
                    empty.loadWithComponents(net.minecraft.world.level.storage.TagValueInput.create(
                        net.minecraft.util.ProblemReporter.DISCARDING, registry, new net.minecraft.nbt.CompoundTag()));
                    var absent = empty.saveWithFullMetadata(registry);
                    // A DUMMY pending entry becomes a fresh block entity, not a loaded one.
                    if ("DUMMY".equals(tag.getStringOr("id", ""))) tag = absent = fresh;
                    w.println("be " + pos.getX() + " " + pos.getY() + " " + pos.getZ() + "\t" + canonicalOf(tag) + "\t" + canonicalOf(absent));
                }
                // Scheduled ticks, in scheduling order (the order they run in).
                if (target instanceof net.minecraft.world.level.chunk.ProtoChunk proto) {
                    for (var t : ((net.minecraft.world.ticks.ProtoChunkTicks<net.minecraft.world.level.block.Block>) proto.getBlockTicks()).scheduledTicks())
                        w.println("t block " + net.minecraft.core.registries.BuiltInRegistries.BLOCK.getKey(t.type()) + " " + t.pos().getX() + " " + t.pos().getY() + " " + t.pos().getZ() + " " + t.priority().getValue());
                    for (var t : ((net.minecraft.world.ticks.ProtoChunkTicks<net.minecraft.world.level.material.Fluid>) proto.getFluidTicks()).scheduledTicks())
                        w.println("t fluid " + net.minecraft.core.registries.BuiltInRegistries.FLUID.getKey(t.type()) + " " + t.pos().getX() + " " + t.pos().getY() + " " + t.pos().getZ() + " " + t.priority().getValue());
                }
                // Postprocessing marks, per section in list order (LevelChunk.postProcessGeneration
                // ticks their fluids and reshapes their blocks once the chunk is live).
                var marks = target.getPostProcessing();
                for (int i = 0; i < marks.length; i++) {
                    if (marks[i] == null) continue;
                    for (short packed : marks[i]) {
                        var p = ProtoChunk.unpackOffsetCoordinates(packed, target.getSectionYFromSectionIndex(i), target.getPos());
                        w.println("p " + p.getX() + " " + p.getY() + " " + p.getZ());
                    }
                }
            }
            }
        }
        storage.close();
        System.exit(0);
    }

    private static String canonicalOf(net.minecraft.nbt.Tag tag) {
        var out = new StringBuilder();
        canonical(tag, out);
        return out.toString();
    }

    /** Canonical single-line form of a tag: type-prefixed scalars, sorted compound keys. */
    private static void canonical(net.minecraft.nbt.Tag tag, StringBuilder out) {
        switch (tag) {
            case net.minecraft.nbt.ByteTag t -> out.append("b").append(t.byteValue());
            case net.minecraft.nbt.ShortTag t -> out.append("s").append(t.shortValue());
            case net.minecraft.nbt.IntTag t -> out.append("i").append(t.intValue());
            case net.minecraft.nbt.LongTag t -> out.append("l").append(t.longValue());
            case net.minecraft.nbt.FloatTag t -> out.append("f").append(Float.floatToRawIntBits(t.floatValue()));
            case net.minecraft.nbt.DoubleTag t -> out.append("d").append(Double.doubleToRawLongBits(t.doubleValue()));
            case net.minecraft.nbt.StringTag t -> out.append('"').append(t.value().replace("\\", "\\\\").replace("\"", "\\\"")).append('"');
            case net.minecraft.nbt.ByteArrayTag t -> { out.append("B["); for (byte v : t.getAsByteArray()) out.append(v).append(','); out.append(']'); }
            case net.minecraft.nbt.IntArrayTag t -> { out.append("I["); for (int v : t.getAsIntArray()) out.append(v).append(','); out.append(']'); }
            case net.minecraft.nbt.LongArrayTag t -> { out.append("L["); for (long v : t.getAsLongArray()) out.append(v).append(','); out.append(']'); }
            case net.minecraft.nbt.ListTag t -> { out.append('['); for (var item : t) { canonical(item, out); out.append(','); } out.append(']'); }
            case net.minecraft.nbt.CompoundTag t -> {
                out.append('{');
                for (String key : new TreeSet<>(t.keySet())) { out.append(key).append('='); canonical(t.get(key), out); out.append(','); }
                out.append('}');
            }
            default -> throw new IllegalStateException(tag.getClass().getName());
        }
    }
}
