package dev.horizonlab.parity;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.function.Predicate;
import net.minecraft.core.BlockPos;
import net.minecraft.core.RegistryAccess;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.MoverType;
import net.minecraft.world.entity.vehicle.boat.AbstractBoat;
import net.minecraft.world.entity.vehicle.boat.Boat;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.Level;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.Fluids;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;

/** Runs the real AbstractBoat.tick across changes in local movement ownership.
 * The stand-in client level is empty air; base entity bookkeeping, collision
 * displacement and block effects are isolated from cached motion. The real
 * floatBoat and tick authority branches produce the observed values.
 */
public final class VanillaBoatAuthorityOracle {
    static final class Lvl extends ServerLevel {
        int next;
        Lvl() { super(null, null, null, null, null, null, false, 0L, List.of(), false); }
        @Override public boolean isClientSide() { return true; }
        @Override public int getNextEntityId() { return next++; }
        @Override public void sendPacketToServer(net.minecraft.network.protocol.Packet<?> packet) {}
        @Override public RegistryAccess registryAccess() { return ParityData.access; }
        @Override public BlockState getBlockState(BlockPos pos) { return Blocks.AIR.defaultBlockState(); }
        @Override public FluidState getFluidState(BlockPos pos) { return Fluids.EMPTY.defaultFluidState(); }
        @Override public List<Entity> getEntities(Entity entity, AABB box, Predicate<? super Entity> predicate) {
            return List.of();
        }
    }

    static final class Probe extends Boat {
        boolean owned;
        Probe(Lvl level) { super(EntityTypes.OAK_BOAT, level, () -> Items.OAK_BOAT); }
        @Override protected boolean isLocalClientAuthoritative() { return owned; }
        @Override public void baseTick() {}
        @Override public void move(MoverType type, Vec3 movement) {}
        @Override protected void applyEffectsFromBlocks() {}
        @Override public float getGroundFriction() { return 0.0f; }
        @Override public net.minecraft.world.scores.PlayerTeam getTeam() { return null; }
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        VanillaVehicleContainerOracle.reflect();
        Lvl level = (Lvl) VanillaVehicleContainerOracle.allocate.invoke(VanillaVehicleContainerOracle.unsafe, Lvl.class);
        Field registry = Level.class.getDeclaredField("registryAccess");
        long offset = (long) VanillaVehicleContainerOracle.objectFieldOffset.invoke(VanillaVehicleContainerOracle.unsafe, registry);
        VanillaVehicleContainerOracle.putObject.invoke(VanillaVehicleContainerOracle.unsafe, level, offset, ParityData.access);
        Field spin = AbstractBoat.class.getDeclaredField("deltaRotation");
        spin.setAccessible(true);
        JsonArray output = new JsonArray();
        try (var reader = Files.newBufferedReader(Path.of(args[0]))) {
            for (var input : JsonParser.parseReader(reader).getAsJsonArray()) {
                JsonObject c = input.getAsJsonObject();
                Probe boat = new Probe(level);
                boat.setPos(0, 100, 0);
                JsonArray velocity = c.getAsJsonArray("velocity");
                boat.setDeltaMovement(new Vec3(velocity.get(0).getAsDouble(), velocity.get(1).getAsDouble(), velocity.get(2).getAsDouble()));
                spin.setFloat(boat, c.get("spin").getAsFloat());
                JsonArray steps = new JsonArray();
                for (var ownership : c.getAsJsonArray("owned")) {
                    boat.owned = ownership.getAsBoolean();
                    boat.tick();
                    JsonObject state = new JsonObject();
                    state.addProperty("owned", boat.owned);
                    state.addProperty("spin", String.format("%08x", Float.floatToRawIntBits(spin.getFloat(boat))));
                    Vec3 v = boat.getDeltaMovement();
                    JsonArray bits = new JsonArray();
                    for (double component : new double[]{v.x, v.y, v.z}) bits.add(String.format("%016x", Double.doubleToRawLongBits(component)));
                    state.add("velocity", bits);
                    steps.add(state);
                }
                JsonObject result = new JsonObject();
                result.add("name", c.get("name"));
                result.add("steps", steps);
                output.add(result);
            }
        }
        Files.writeString(Path.of(args[1]), output.toString());
    }
}
