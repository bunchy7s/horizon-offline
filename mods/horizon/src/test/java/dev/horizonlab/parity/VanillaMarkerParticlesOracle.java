package dev.horizonlab.parity;

import com.google.gson.JsonParser;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.multiplayer.MultiPlayerGameMode;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.core.BlockPos;
import net.minecraft.core.particles.ParticleOptions;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.util.RandomSource;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.CampfireBlock;
import net.minecraft.world.level.block.entity.CampfireBlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.Fluids;

/** Invoke shipped marker selection and ambient sampling in a dry barrier world,
 * and the campfire's own particles over a stand-in level that records them. */
public final class VanillaMarkerParticlesOracle {
    static Field field(Class<?> type, String name) throws Exception {
        var field = type.getDeclaredField(name);
        field.setAccessible(true);
        return field;
    }
    static String bits(double v) { return String.format("%016x", Double.doubleToLongBits(v)); }
    public static final class Player extends LocalPlayer {
        ItemStack main, off;
        Player() { super(null, null, null, null, null, null, false, null, null); }
        @Override public ItemStack getMainHandItem() { return main; }
        @Override public ItemStack getItemInHand(InteractionHand hand) {
            return hand == InteractionHand.MAIN_HAND ? main : off;
        }
    }
    public static final class Mode extends MultiPlayerGameMode {
        GameType mode;
        Mode() { super(null, null); }
        @Override public GameType getPlayerMode() { return mode; }
    }
    public static final class MarkerLevel extends ClientLevel {
        java.io.BufferedWriter out;
        MarkerLevel() { super(null, null, null, null, 0, 0, null, false, 0L, 0); }
        @Override public BlockState getBlockState(BlockPos pos) { return Blocks.BARRIER.defaultBlockState(); }
        @Override public FluidState getFluidState(BlockPos pos) { return Fluids.EMPTY.defaultFluidState(); }
        @Override public void addParticle(ParticleOptions options, double x, double y, double z,
                                          double dx, double dy, double dz) {
            if (dx != 0 || dy != 0 || dz != 0) throw new AssertionError("marker velocity");
            try {
                out.write("marker " + x + " " + y + " " + z);
                out.newLine();
            } catch (java.io.IOException e) { throw new RuntimeException(e); }
        }
    }
    /** The level a campfire is ticked over: it holds no blocks and records every
     * particle it is asked for by its registry name and position. */
    public static final class CampfireLevel extends ClientLevel {
        java.io.BufferedWriter out;
        CampfireLevel() { super(null, null, null, null, 0, 0, null, false, 0L, 0); }
        @Override public BlockState getBlockState(BlockPos pos) { return Blocks.AIR.defaultBlockState(); }
        @Override public FluidState getFluidState(BlockPos pos) { return Fluids.EMPTY.defaultFluidState(); }
        private void record(ParticleOptions options, double x, double y, double z,
                            double dx, double dy, double dz) {
            try {
                out.write(BuiltInRegistries.PARTICLE_TYPE.getKey(options.getType()).getPath()
                          + " " + bits(x) + " " + bits(y) + " " + bits(z)
                          + " " + bits(dx) + " " + bits(dy) + " " + bits(dz));
                out.newLine();
            } catch (java.io.IOException e) { throw new RuntimeException(e); }
        }
        @Override public void addParticle(ParticleOptions options, double x, double y, double z,
                                          double dx, double dy, double dz) {
            record(options, x, y, z, dx, dy, dz);
        }
        @Override public void addAlwaysVisibleParticle(ParticleOptions options, boolean force,
                                                       double x, double y, double z,
                                                       double dx, double dy, double dz) {
            record(options, x, y, z, dx, dy, dz);
        }
    }
    /** A `minecraft:name[k=v,...]` state through the real block state definition. */
    @SuppressWarnings({"unchecked", "rawtypes"})
    static BlockState parseState(String spec) {
        int bracket = spec.indexOf('[');
        var name = bracket < 0 ? spec : spec.substring(0, bracket);
        Block block = BuiltInRegistries.BLOCK.getValue(Identifier.parse(name));
        var state = block.defaultBlockState();
        if (bracket < 0) return state;
        for (var pair : spec.substring(bracket + 1, spec.length() - 1).split(",")) {
            var parts = pair.split("=");
            Property property = block.getStateDefinition().getProperty(parts[0]);
            if (property == null) throw new AssertionError("no property " + parts[0]);
            state = state.setValue(property, (Comparable) property.getValue(parts[1]).orElseThrow());
        }
        return state;
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        for (var item : BuiltInRegistries.ITEM) {
            item.builtInRegistryHolder().bindComponents(net.minecraft.core.component.DataComponentMap.EMPTY);
        }
        var allocator = Class.forName("sun.misc.Unsafe");
        var unsafe = field(allocator, "theUnsafe").get(null);
        var allocate = allocator.getMethod("allocateInstance", Class.class);
        var mc = (Minecraft) allocate.invoke(unsafe, Minecraft.class);
        var level = (MarkerLevel) allocate.invoke(unsafe, MarkerLevel.class);
        var campfireLevel = (CampfireLevel) allocate.invoke(unsafe, CampfireLevel.class);
        var player = (Player) allocate.invoke(unsafe, Player.class);
        var mode = (Mode) allocate.invoke(unsafe, Mode.class);
        field(ClientLevel.class, "minecraft").set(level, mc);
        field(ClientLevel.class, "minecraft").set(campfireLevel, mc);
        mc.player = player;
        mc.gameMode = mode;
        var select = ClientLevel.class.getDeclaredMethod("getMarkerParticleTarget");
        select.setAccessible(true);
        try (var out = Files.newBufferedWriter(Path.of(args[1]))) {
            level.out = out;
            campfireLevel.out = out;
            var lines = Files.readAllLines(Path.of(args[0]));
            for (int index = 0; index < lines.size(); index++) {
                var c = JsonParser.parseString(lines.get(index)).getAsJsonObject();
                var kind = c.has("kind") ? c.get("kind").getAsString() : "marker";
                if (!kind.equals("marker")) {
                    // The campfire cases seed the level random and tick the fire.
                    var random = RandomSource.create(c.get("seed").getAsLong());
                    field(Level.class, "random").set(campfireLevel, random);
                    var pos = new BlockPos(0, 64, 0);
                    out.write("== " + index + " " + kind);
                    out.newLine();
                    if (kind.equals("campfire")) {
                        var state = parseState(c.get("state").getAsString());
                        var entity = new CampfireBlockEntity(pos, state);
                        var slots = c.getAsJsonArray("slots");
                        for (int slot = 0; slot < 4; slot++) {
                            if (slots.get(slot).getAsInt() != 0) {
                                entity.getItems().set(slot, new ItemStack(Items.BEEF));
                            }
                        }
                        // ClientLevel only ticks a lit campfire (CampfireBlock.isLitCampfire).
                        if (state.getValue(CampfireBlock.LIT)) {
                            for (int tick = 0; tick < c.get("ticks").getAsInt(); tick++) {
                                CampfireBlockEntity.particleTick(campfireLevel, pos, state, entity);
                            }
                        }
                    } else {
                        for (int puff = 0; puff < c.get("puffs").getAsInt(); puff++) {
                            CampfireBlock.makeParticles(campfireLevel, pos,
                                                        c.get("signal").getAsBoolean(),
                                                        c.get("smoking").getAsBoolean());
                        }
                    }
                    out.write("next " + random.nextInt());
                    out.newLine();
                    continue;
                }
                mode.mode = GameType.byName(c.get("mode").getAsString());
                player.main = new ItemStack(BuiltInRegistries.ITEM.getValue(
                    Identifier.parse("minecraft:" + c.get("main").getAsString())), c.get("count").getAsInt());
                player.off = new ItemStack(BuiltInRegistries.ITEM.getValue(
                    Identifier.parse("minecraft:" + c.get("off").getAsString())));
                var block = (Block) select.invoke(level);
                out.write(block == null ? "none" : BuiltInRegistries.BLOCK.getKey(block).toString());
                out.newLine();
                var random = RandomSource.create(1057);
                field(Level.class, "random").set(level, random);
                level.animateTick(0, 64, 0);
                out.write("next " + random.nextInt());
                out.newLine();
            }
        }
    }
}
