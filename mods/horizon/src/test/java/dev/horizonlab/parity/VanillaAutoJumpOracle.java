package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.player.ClientInput;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.core.*;
import net.minecraft.world.effect.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.player.Input;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.phys.*;
import net.minecraft.world.phys.shapes.VoxelShape;
import static dev.horizonlab.parity.VanillaRegistryCollisionScanOracle.*;

/**
 * Runs the real LocalPlayer.updateAutoJump (with canAutoJump, getBlockJumpFactor,
 * CollisionGetter.getCollisions/BlockCollisions and the player's collision
 * context) on a LocalPlayer shell standing in an independently constructed copy
 * of the client's resident test scene, with each case's blocks placed.
 */
public final class VanillaAutoJumpOracle {
    static Object unsafe;
    static Method allocate;

    @SuppressWarnings("unchecked")
    static <T> T make(Class<T> type) throws Exception {
        return (T) allocate.invoke(unsafe, type);
    }

    static Field field(Class<?> type, String name) throws Exception {
        Field f = type.getDeclaredField(name);
        f.setAccessible(true);
        return f;
    }

    static float f(JsonElement v) {
        return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(), 16));
    }

    static double d(JsonElement v) {
        return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(), 16));
    }

    static Map<Long, Chunk> chunks;

    static final class AutoLevel extends net.minecraft.server.level.ServerLevel {
        AutoLevel() {
            super(null, null, null, null, null, null, false, 0L, List.of(), false);
        }

        @Override public BlockState getBlockState(BlockPos p) { return chunks.get(key(p.getX() >> 4, p.getZ() >> 4)).getBlockState(p); }
        @Override public BlockGetter getChunkForCollisions(int x, int z) { return chunks.get(key(x, z)); }
        @Override public List<VoxelShape> getEntityCollisions(Entity entity, AABB bounds) { return List.of(); }
    }

    static final class Jumper extends LocalPlayer {
        boolean ground;
        boolean riding;
        net.minecraft.world.entity.ai.attributes.AttributeMap attributes;

        Jumper() {
            super(null, null, null, null, null, Input.EMPTY, false, null, null);
        }

        @Override public boolean onGround() { return ground; }
        @Override public boolean isPassenger() { return riding; }
        @Override public net.minecraft.world.entity.ai.attributes.AttributeMap getAttributes() { return attributes; }
        @Override public ItemStack getMainHandItem() { return ItemStack.EMPTY; }
        @Override public ItemStack getItemBySlot(EquipmentSlot slot) { return ItemStack.EMPTY; }
    }

    static <T extends Comparable<T>> BlockState with(BlockState state, Property<T> key, String value) {
        return state.setValue(key, key.getValue(value).orElseThrow());
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var u = Class.forName("sun.misc.Unsafe");
        unsafe = field(u, "theUnsafe").get(null);
        allocate = u.getMethod("allocateInstance", Class.class);
        Path resources = Path.of(args[2]);
        VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(resources.resolve("block-tags.json"))).getAsJsonObject());
        var description = JsonParser.parseString(Files.readString(Path.of(args[3]))).getAsJsonObject();
        var trace = new Trace();
        trace.out = new PrintWriter(OutputStream.nullOutputStream());
        trace.fail = -1;
        chunks = VanillaSceneCollisionOracle.scene(description, trace);
        AutoLevel level = make(AutoLevel.class);
        var update = LocalPlayer.class.getDeclaredMethod("updateAutoJump", float.class, float.class);
        update.setAccessible(true);
        var enabled = field(LocalPlayer.class, "autoJumpEnabled");
        var time = field(LocalPlayer.class, "autoJumpTime");
        var moveVector = field(ClientInput.class, "moveVector");
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            int n = 0;
            for (var line : (Iterable<String>) lines::iterator) {
                var c = JsonParser.parseString(line).getAsJsonObject();
                out.println("== " + n++);
                var restore = new ArrayList<Object[]>();
                for (var e : c.getAsJsonArray("blocks")) {
                    var b = e.getAsJsonArray();
                    var pos = new BlockPos(b.get(0).getAsInt(), b.get(1).getAsInt(), b.get(2).getAsInt());
                    BlockState state = VanillaSceneCollisionOracle.state(b.get(3).getAsString());
                    for (var p : b.get(4).getAsJsonObject().entrySet()) {
                        state = with(state, Objects.requireNonNull(state.getBlock().getStateDefinition().getProperty(p.getKey())), p.getValue().getAsString());
                    }
                    restore.add(new Object[] {pos, level.getBlockState(pos)});
                    VanillaSceneCollisionOracle.put(chunks, pos.getX(), pos.getY(), pos.getZ(), state);
                }
                var p = c.getAsJsonObject("player");
                Jumper player = make(Jumper.class);
                field(Entity.class, "level").set(player, level);
                var position = new Vec3(d(p.getAsJsonArray("pos").get(0)), d(p.getAsJsonArray("pos").get(1)), d(p.getAsJsonArray("pos").get(2)));
                field(Entity.class, "position").set(player, position);
                field(Entity.class, "blockPosition").set(player, BlockPos.containing(position));
                player.mainSupportingBlockPos = Optional.empty();
                var dimensions = EntityDimensions.scalable(f(p.get("width")), f(p.get("height")));
                field(Entity.class, "dimensions").set(player, dimensions);
                player.setBoundingBox(dimensions.makeBoundingBox(position));
                player.ground = p.get("ground").getAsBoolean();
                player.riding = p.has("passenger") && p.get("passenger").getAsBoolean();
                field(Entity.class, "yRot").setFloat(player, f(p.get("yr")));
                field(Entity.class, "xRot").setFloat(player, f(p.get("xr")));
                // Player.getSpeed reads the movement speed attribute.
                player.attributes = new net.minecraft.world.entity.ai.attributes.AttributeMap(Player.createAttributes().build());
                player.attributes.getInstance(net.minecraft.world.entity.ai.attributes.Attributes.MOVEMENT_SPEED).setBaseValue(f(p.get("speed")));
                var input = new ClientInput();
                boolean shift = p.get("shift").getAsBoolean();
                input.keyPresses = new Input(false, false, false, false, false, shift, false);
                var move = p.getAsJsonArray("move");
                moveVector.set(input, new Vec2(f(move.get(0)), f(move.get(1))));
                player.input = input;
                var effects = new HashMap<net.minecraft.core.Holder<MobEffect>, MobEffectInstance>();
                if (!p.get("boost").isJsonNull()) {
                    effects.put(MobEffects.JUMP_BOOST, new MobEffectInstance(MobEffects.JUMP_BOOST, 100, p.get("boost").getAsInt()));
                }
                field(LivingEntity.class, "activeEffects").set(player, effects);
                enabled.setBoolean(player, p.get("enabled").getAsBoolean());
                time.setInt(player, p.get("time").getAsInt());
                update.invoke(player, f(c.get("xa")), f(c.get("za")));
                out.println("jump " + time.getInt(player));
                Collections.reverse(restore);
                for (var r : restore) {
                    var pos = (BlockPos) r[0];
                    VanillaSceneCollisionOracle.put(chunks, pos.getX(), pos.getY(), pos.getZ(), (BlockState) r[1]);
                }
            }
        }
        System.exit(0);
    }
}
