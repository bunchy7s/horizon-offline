package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.blaze3d.platform.NativeImage;
import java.io.InputStreamReader;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.atomic.AtomicLong;
import net.minecraft.SharedConstants;
import net.minecraft.client.Minecraft;
import net.minecraft.client.color.block.BlockColors;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.particle.*;
import net.minecraft.client.renderer.block.BlockStateModelSet;
import net.minecraft.client.renderer.item.ItemModelResolver;
import net.minecraft.client.renderer.item.ItemStackRenderState;
import net.minecraft.client.renderer.texture.*;
import net.minecraft.client.resources.metadata.animation.FrameSize;
import net.minecraft.client.resources.model.ModelManager;
import net.minecraft.client.resources.model.sprite.Material;
import net.minecraft.core.BlockPos;
import net.minecraft.core.particles.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.Bootstrap;
import net.minecraft.sounds.SoundEvent;
import net.minecraft.sounds.SoundSource;
import net.minecraft.util.RandomSource;
import net.minecraft.world.entity.ItemOwner;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.ItemStackTemplate;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.gameevent.BlockPositionSource;
import net.minecraft.world.level.levelgen.LegacyRandomSource;
import net.minecraft.world.level.levelgen.RandomSupport;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.Fluids;
import net.minecraft.world.phys.Vec3;
import org.jspecify.annotations.Nullable;

/** Builds each particle through its real registered ParticleProvider (the real
 * ParticleResources, sprite sets bound to stand-in sprites named after the
 * vanilla particles/*.json textures) in a stand-in ClientLevel (air, an
 * optional stone floor at y 63, optionally still water from y 64 to 67),
 * then ticks it and records its state, the particles and sounds it asks the
 * level for, tick by tick.
 *
 * Each vanilla Particle owns a RandomSource.create() random seeded from the
 * clock; its seed is recovered after construction (the SEED_UNIQUIFIER value
 * and the System.nanoTime window around the call, checked against the
 * random's state stepped back over its draws) and printed first so the probe
 * can replay the same draws. The provider random, the level random and
 * SpellParticle.RANDOM are seeded from the case. */
public final class VanillaParticleProvidersOracle {
    private static Object unsafe;
    private static Method allocate;
    private static final long MULTIPLIER = 0x5DEECE66DL, MASK = (1L << 48) - 1, INVERSE = 0xDFE05BCB1365L;
    static Field field(Class<?> type, String name) throws Exception {
        var f = type.getDeclaredField(name); f.setAccessible(true); return f;
    }
    static <T> T make(Class<T> type) throws Exception { return type.cast(allocate.invoke(unsafe, type)); }

    static final class Level0 extends ClientLevel {
        boolean floor, water;
        List<String> events;
        Level0() { super(null, null, null, null, 0, 0, null, false, 0L, 0); }
        // Water: still water sources from y 64 to 67 (above the floor, below the open air).
        boolean wet(BlockPos p) { return water && p.getY() >= 64 && p.getY() <= 67; }
        @Override public BlockState getBlockState(BlockPos p) {
            if (wet(p)) return Blocks.WATER.defaultBlockState();
            return floor && p.getY() == 63 ? Blocks.STONE.defaultBlockState() : Blocks.AIR.defaultBlockState();
        }
        @Override public FluidState getFluidState(BlockPos p) {
            return wet(p) ? Fluids.WATER.getSource(false) : Fluids.EMPTY.defaultFluidState();
        }
        @Override public @Nullable BlockGetter getChunkForCollisions(int x, int z) { return this; }
        @Override public boolean hasChunkAt(BlockPos p) { return false; }
        @Override public void addParticle(ParticleOptions o, double x, double y, double z, double xd, double yd, double zd) {
            events.add("+ " + name(o) + " " + d(x) + " " + d(y) + " " + d(z) + " " + d(xd) + " " + d(yd) + " " + d(zd));
        }
        @Override public void addParticle(ParticleOptions o, boolean override, boolean always, double x, double y, double z, double xd, double yd, double zd) {
            addParticle(o, x, y, z, xd, yd, zd);
        }
        @Override public void addAlwaysVisibleParticle(ParticleOptions o, double x, double y, double z, double xd, double yd, double zd) {
            addParticle(o, x, y, z, xd, yd, zd);
        }
        @Override public void addAlwaysVisibleParticle(ParticleOptions o, boolean override, double x, double y, double z, double xd, double yd, double zd) {
            addParticle(o, x, y, z, xd, yd, zd);
        }
        @Override public void playLocalSound(double x, double y, double z, SoundEvent sound, SoundSource source, float volume, float pitch, boolean delay) {
            events.add("~ " + sound.location() + " " + d(x) + " " + d(y) + " " + d(z) + " " + f(volume) + " " + f(pitch));
        }
        @Override public net.minecraft.world.entity.player.@Nullable Player getNearestPlayer(double x, double y, double z, double distance, boolean creative) { return null; }
    }
    static String name(ParticleOptions o) { return BuiltInRegistries.PARTICLE_TYPE.getKey(o.getType()).getPath(); }
    static String d(double v) { return String.format("%016x", Double.doubleToRawLongBits(v)); }
    static String f(float v) { return String.format("%08x", Float.floatToRawIntBits(v)); }

    static final class Sprite extends TextureAtlasSprite {
        Sprite(Identifier atlas, Identifier name) { super(atlas, new SpriteContents(name, new FrameSize(16, 16), new NativeImage(16, 16, true)), 16, 16, 0, 0, 0); }
    }
    static final Map<String, TextureAtlasSprite> SPRITES = new HashMap<>();
    static TextureAtlasSprite sprite(Identifier atlas, String name) {
        return SPRITES.computeIfAbsent(name, n -> new Sprite(atlas, Identifier.withDefaultNamespace(n)));
    }
    static final class Models extends BlockStateModelSet {
        Models() { super(Map.of(), null); }
        @Override public Material.Baked getParticleMaterial(BlockState state) {
            return new Material.Baked(sprite(TextureAtlas.LOCATION_BLOCKS, "terrain"), false);
        }
    }
    /** Every item resolves to one layer whose particle sprite is named after the item. */
    static final class Items0 extends ItemModelResolver {
        Items0() { super(null); }
        @Override public void updateForTopItem(ItemStackRenderState out, ItemStack item, ItemDisplayContext context, @Nullable Level level, @Nullable ItemOwner owner, int seed) {
            out.clear();
            out.newLayer().setParticleMaterial(new Material.Baked(sprite(TextureAtlas.LOCATION_BLOCKS, "item_" + BuiltInRegistries.ITEM.getKey(item.getItem()).getPath()), false));
        }
    }

    static ParticleOptions options(JsonObject c) {
        var id = Identifier.withDefaultNamespace(c.get("type").getAsString());
        ParticleType<?> type = BuiltInRegistries.PARTICLE_TYPE.getValue(id);
        if (type == null) throw new AssertionError("unknown particle " + id);
        var o = c.has("options") ? c.getAsJsonObject("options") : new JsonObject();
        if (type instanceof SimpleParticleType simple) return simple;
        @SuppressWarnings("unchecked") var any = (ParticleType<ParticleOptions>) (ParticleType<?>) type;
        int color = o.has("color") ? (int) o.get("color").getAsLong() : 0xFFFFFFFF;
        float scale = o.has("scale") ? o.get("scale").getAsFloat() : 1.0F;
        if (type == ParticleTypes.DUST) return new DustParticleOptions(color, scale);
        if (type == ParticleTypes.DUST_COLOR_TRANSITION) return new DustColorTransitionOptions(color, (int) o.get("to").getAsLong(), scale);
        if (type == ParticleTypes.EFFECT || type == ParticleTypes.INSTANT_EFFECT)
            return SpellParticleOption.create(castSpell(type), color, scale);
        if (type == ParticleTypes.DRAGON_BREATH) return PowerParticleOption.create(ParticleTypes.DRAGON_BREATH, scale);
        if (type == ParticleTypes.SCULK_CHARGE) return new SculkChargeParticleOptions(scale);
        if (type == ParticleTypes.GEYSER || type == ParticleTypes.GEYSER_PLUME)
            return new GeyserParticleOptions(geyser(type), o.get("water_blocks").getAsInt());
        if (type == ParticleTypes.GEYSER_BASE || type == ParticleTypes.GEYSER_POOF)
            return new GeyserBaseParticleOptions(geyserBase(type), o.get("water_blocks").getAsInt(),
                                                 o.get("burst_impulse_base").getAsFloat());
        if (type == ParticleTypes.SHRIEK) return new ShriekParticleOption((int) scale);
        if (type == ParticleTypes.ITEM)
            return new ItemParticleOption(ParticleTypes.ITEM, new ItemStackTemplate(BuiltInRegistries.ITEM.getValue(Identifier.parse(o.get("item").getAsString()))));
        if (type == ParticleTypes.TRAIL) {
            var t = o.getAsJsonArray("target");
            return new TrailParticleOption(new Vec3(t.get(0).getAsDouble(), t.get(1).getAsDouble(), t.get(2).getAsDouble()), color, o.get("duration").getAsInt());
        }
        if (type == ParticleTypes.VIBRATION) {
            var t = o.getAsJsonArray("target");
            return new VibrationParticleOption(new BlockPositionSource(new BlockPos(t.get(0).getAsInt(), t.get(1).getAsInt(), t.get(2).getAsInt())), o.get("duration").getAsInt());
        }
        if (o.has("state")) return new BlockParticleOption(castBlock(type), Block.stateById(o.get("state").getAsInt()));
        if (o.has("color")) return ColorParticleOption.create(castColor(type), color);
        throw new AssertionError("no options for " + id);
    }
    @SuppressWarnings("unchecked") static ParticleType<SpellParticleOption> castSpell(ParticleType<?> t) { return (ParticleType<SpellParticleOption>) t; }
    @SuppressWarnings("unchecked") static ParticleType<GeyserParticleOptions> geyser(ParticleType<?> t) { return (ParticleType<GeyserParticleOptions>) t; }
    @SuppressWarnings("unchecked") static ParticleType<GeyserBaseParticleOptions> geyserBase(ParticleType<?> t) { return (ParticleType<GeyserBaseParticleOptions>) t; }
    @SuppressWarnings("unchecked") static ParticleType<BlockParticleOption> castBlock(ParticleType<?> t) { return (ParticleType<BlockParticleOption>) t; }
    @SuppressWarnings("unchecked") static ParticleType<ColorParticleOption> castColor(ParticleType<?> t) { return (ParticleType<ColorParticleOption>) t; }

    static Field X, Y, Z, XO, YO, ZO, XD, YD, ZD, AGE, LIFETIME, REMOVED, ON_GROUND, RANDOM, SEED;
    static Field QUAD, R, G, B, A, ROLL, O_ROLL, SPRITE;
    static Method U0, U1, V0, V1, LIGHT;

    static String state(Particle p) throws Exception {
        var s = new StringBuilder();
        s.append(AGE.getInt(p)).append(' ').append(LIFETIME.getInt(p)).append(' ').append(REMOVED.getBoolean(p) ? 1 : 0)
            .append(' ').append(ON_GROUND.getBoolean(p) ? 1 : 0);
        for (var f : new Field[] {X, Y, Z, XD, YD, ZD, XO, YO, ZO}) s.append(' ').append(d(f.getDouble(p)));
        if (p instanceof SingleQuadParticle q) {
            s.append(" | ").append(f(q.getQuadSize(0.0F))).append(' ').append(f(q.getQuadSize(0.5F)));
            for (var f : new Field[] {R, G, B, A, ROLL, O_ROLL}) s.append(' ').append(f(f.getFloat(q)));
            var sprite = (TextureAtlasSprite) SPRITE.get(q);
            s.append(' ').append(sprite == null ? "-" : sprite.contents().name().getPath());
            for (var m : new Method[] {U0, U1, V0, V1}) s.append(' ').append(f((float) m.invoke(q)));
            s.append(' ').append(Integer.toHexString((int) LIGHT.invoke(q, 0.5F)));
        }
        return s.toString();
    }

    /** Recover the seed of RandomSource.create() that made `random`, which was
     * created after `before` was the uniquifier and inside [t0, t1]. */
    static long recover(LegacyRandomSource random, long before, long t0, long t1) throws Exception {
        long end = ((AtomicLong) SEED.get(random)).get();
        var back = new HashMap<Long, Integer>();
        long s = end;
        for (int k = 0; k <= 4096; k++) { back.putIfAbsent(s, k); s = ((s - 0xBL) * INVERSE) & MASK; }
        long u = before;
        Long found = null;
        for (int j = 0; j < 8 && found == null; j++) {
            u *= 1181783497276652981L;
            for (long t = t0; t <= t1; t++) {
                long seed = u ^ t;
                if (back.containsKey((seed ^ MULTIPLIER) & MASK)) {
                    if (found != null && ((found ^ MULTIPLIER) & MASK) != ((seed ^ MULTIPLIER) & MASK)) throw new AssertionError("ambiguous seed");
                    found = seed;
                }
            }
        }
        if (found == null) throw new AssertionError("particle seed not recovered");
        return found;
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion(); Bootstrap.bootStrap();
        ParityData.init(); // item components, for the item particle providers' stacks
        var u = Class.forName("sun.misc.Unsafe"); unsafe = field(u, "theUnsafe").get(null); allocate = u.getMethod("allocateInstance", Class.class);
        var mc = make(Minecraft.class); field(Minecraft.class, "instance").set(null, mc);
        var manager = make(ModelManager.class); field(ModelManager.class, "blockStateModelSet").set(manager, new Models());
        field(Minecraft.class, "modelManager").set(mc, manager); field(Minecraft.class, "blockColors").set(mc, new BlockColors());
        field(Minecraft.class, "itemModelResolver").set(mc, new Items0());
        var level = make(Level0.class); level.events = new ArrayList<>(); field(ClientLevel.class, "minecraft").set(level, mc);
        field(Minecraft.class, "particleEngine").set(mc, new ParticleEngine(level, null));
        X = field(Particle.class, "x"); Y = field(Particle.class, "y"); Z = field(Particle.class, "z");
        XO = field(Particle.class, "xo"); YO = field(Particle.class, "yo"); ZO = field(Particle.class, "zo");
        XD = field(Particle.class, "xd"); YD = field(Particle.class, "yd"); ZD = field(Particle.class, "zd");
        AGE = field(Particle.class, "age"); LIFETIME = field(Particle.class, "lifetime"); REMOVED = field(Particle.class, "removed");
        ON_GROUND = field(Particle.class, "onGround"); RANDOM = field(Particle.class, "random"); SEED = field(LegacyRandomSource.class, "seed");
        QUAD = field(SingleQuadParticle.class, "quadSize"); R = field(SingleQuadParticle.class, "rCol"); G = field(SingleQuadParticle.class, "gCol");
        B = field(SingleQuadParticle.class, "bCol"); A = field(SingleQuadParticle.class, "alpha"); ROLL = field(SingleQuadParticle.class, "roll");
        O_ROLL = field(SingleQuadParticle.class, "oRoll"); SPRITE = field(SingleQuadParticle.class, "sprite");
        U0 = SingleQuadParticle.class.getDeclaredMethod("getU0"); U1 = SingleQuadParticle.class.getDeclaredMethod("getU1");
        V0 = SingleQuadParticle.class.getDeclaredMethod("getV0"); V1 = SingleQuadParticle.class.getDeclaredMethod("getV1");
        LIGHT = Particle.class.getDeclaredMethod("getLightCoords", float.class);
        for (var m : new Method[] {U0, U1, V0, V1, LIGHT}) m.setAccessible(true);
        var uniquifier = (AtomicLong) field(RandomSupport.class, "SEED_UNIQUIFIER").get(null);
        // Initialise SpellParticle first: its static initializer would replace a seeded RANDOM.
        Class.forName(SpellParticle.class.getName(), true, SpellParticle.class.getClassLoader());
        var spellRandom = field(SpellParticle.class, "RANDOM");
        var unsafeObject = unsafe;
        var staticBase = u.getMethod("staticFieldBase", Field.class).invoke(unsafeObject, spellRandom);
        long staticOffset = (long) u.getMethod("staticFieldOffset", Field.class).invoke(unsafeObject, spellRandom);
        var putObject = u.getMethod("putObject", Object.class, long.class, Object.class);

        // The real providers, their sprite sets bound to one stand-in sprite per listed texture.
        var resources = new ParticleResources();
        @SuppressWarnings("unchecked") var sets = (Map<Identifier, Object>) field(ParticleResources.class, "spriteSets").get(resources);
        for (var e : sets.entrySet()) {
            var stream = Minecraft.class.getResourceAsStream("/assets/minecraft/particles/" + e.getKey().getPath() + ".json");
            var sprites = new ArrayList<TextureAtlasSprite>();
            if (stream != null) {
                try (var reader = new InputStreamReader(stream, StandardCharsets.UTF_8)) {
                    for (var t : JsonParser.parseReader(reader).getAsJsonObject().getAsJsonArray("textures"))
                        sprites.add(sprite(TextureAtlas.LOCATION_PARTICLES, Identifier.parse(t.getAsString()).getPath()));
                }
            }
            if (sprites.isEmpty()) sprites.add(sprite(TextureAtlas.LOCATION_PARTICLES, "missingno"));
            var rebind = e.getValue().getClass().getDeclaredMethod("rebind", List.class); rebind.setAccessible(true);
            rebind.invoke(e.getValue(), sprites);
        }
        var providers = resources.getProviders();
        // Warm up the classes the providers touch so the clock window stays small.
        for (int i = 0; i < 3; i++) RandomSource.create();

        try (var output = Files.newBufferedWriter(Path.of(args[1]))) {
            int n = 0;
            for (String line : Files.readAllLines(Path.of(args[0]))) {
                var c = JsonParser.parseString(line).getAsJsonObject();
                output.write("== " + n++); output.newLine();
                var options = options(c);
                @SuppressWarnings("unchecked") var provider = (ParticleProvider<ParticleOptions>) providers.get(BuiltInRegistries.PARTICLE_TYPE.getId(options.getType()));
                if (provider == null) { output.write("no provider"); output.newLine(); continue; }
                long seed = c.get("seed").getAsLong();
                RandomSource random = null;
                level.floor = c.get("floor").getAsBoolean();
                level.water = c.has("water") && c.get("water").getAsBoolean();
                level.events.clear();
                var p = c.getAsJsonArray("pos"); var v = c.getAsJsonArray("vel");
                Particle particle = null;
                long before = 0, t0 = 0, t1 = 0;
                for (int attempt = 0; attempt < 4; attempt++) {
                    level.events.clear();
                    random = RandomSource.create(seed);
                    field(Level.class, "random").set(level, RandomSource.create(seed ^ 0x6C6576656CL));
                    putObject.invoke(unsafeObject, staticBase, staticOffset, RandomSource.create(seed ^ 0x7370656C6CL));
                    before = uniquifier.get(); t0 = System.nanoTime();
                    particle = provider.createParticle(options, level, p.get(0).getAsDouble(), p.get(1).getAsDouble(), p.get(2).getAsDouble(),
                        v.get(0).getAsDouble(), v.get(1).getAsDouble(), v.get(2).getAsDouble(), random);
                    t1 = System.nanoTime();
                    if (t1 - t0 < 50_000_000L) break;
                }
                if (particle == null) { output.write("null " + random.nextLong()); output.newLine(); continue; }
                long local = recover((LegacyRandomSource) RANDOM.get(particle), before, t0, t1);
                output.write("seed " + local + " " + particle.getClass().getSimpleName()); output.newLine();
                output.write("next " + random.nextLong() + " " + level.getRandom().nextLong() + " "
                    + ((RandomSource) spellRandom.get(null)).nextLong()); output.newLine();
                output.write("0 " + state(particle)); output.newLine();
                for (var e : level.events) { output.write(e); output.newLine(); }
                int ticks = c.get("ticks").getAsInt();
                for (int t = 1; t <= ticks && particle.isAlive(); t++) {
                    level.events.clear();
                    particle.tick();
                    output.write(t + " " + state(particle)); output.newLine();
                    for (var e : level.events) { output.write(e); output.newLine(); }
                }
            }
        }
    }
}
