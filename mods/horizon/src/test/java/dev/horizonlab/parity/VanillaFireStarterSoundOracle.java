package dev.horizonlab.parity;

import java.io.*;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.Holder;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.util.RandomSource;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.player.Input;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.item.context.UseOnContext;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.BlockStateProperties;
import net.minecraft.world.level.gameevent.GameEvent;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.Vec3;

/**
 * Original oracle for a fire starter's use sound: the real
 * FireChargeItem.useOn and FlintAndSteelItem.useOn on a campfire, a candle and
 * a stone block, on a level stand-in whose random is seeded from the case,
 * recording the sound, its volume and pitch and the level random's next long
 * (which shows how many values the sound drew).
 */
public final class VanillaFireStarterSoundOracle {
    static final StringBuilder sounds = new StringBuilder();
    static final BlockPos POS = new BlockPos(3, 64, -7);

    static final class World extends ServerLevel {
        BlockState state = Blocks.AIR.defaultBlockState();
        World() {
            super(null, null, null, null, null, null, false, 0L, List.of(), false);
        }
        @Override public BlockState getBlockState(BlockPos p) {
            return p.equals(POS) ? state : Blocks.AIR.defaultBlockState();
        }
        @Override public boolean setBlock(BlockPos p, BlockState s, int flags) { state = s; return true; }
        @Override public boolean setBlockAndUpdate(BlockPos p, BlockState s) { state = s; return true; }
        @Override public void gameEvent(Entity entity, Holder<GameEvent> event, BlockPos p) {}
        @Override public void playSeededSound(net.minecraft.world.entity.Entity p, double x, double y, double z,
                Holder<net.minecraft.sounds.SoundEvent> s, net.minecraft.sounds.SoundSource c, float v, float pitch, long seed) {
            sounds.append(s.unwrapKey().orElseThrow().identifier()).append(' ')
                .append(Float.floatToIntBits(v)).append(' ').append(Float.floatToIntBits(pitch));
        }
    }

    /** The player a use is made by: only its level and its empty hand are read. */
    static final class Shell extends LocalPlayer {
        Shell() {
            super(null, null, null, null, null, Input.EMPTY, false, null, null);
        }
        @Override public ItemStack getMainHandItem() { return ItemStack.EMPTY; }
        @Override public ItemStack getItemBySlot(EquipmentSlot slot) { return ItemStack.EMPTY; }
        @Override public boolean hasInfiniteMaterials() { return false; }
    }

    static void put(Object unsafe, Object target, String name, Object value) throws Exception {
        Field f = Level.class.getDeclaredField(name);
        long offset = (long) unsafe.getClass().getMethod("objectFieldOffset", Field.class).invoke(unsafe, f);
        unsafe.getClass().getMethod("putObject", Object.class, long.class, Object.class).invoke(unsafe, target, offset, value);
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var u = Class.forName("sun.misc.Unsafe");
        Field theUnsafe = u.getDeclaredField("theUnsafe");
        theUnsafe.setAccessible(true);
        Object unsafe = theUnsafe.get(null);
        World world = (World) u.getMethod("allocateInstance", Class.class).invoke(unsafe, World.class);
        Object shell = u.getMethod("allocateInstance", Class.class).invoke(unsafe, Shell.class);
        Field levelField = Entity.class.getDeclaredField("level");
        levelField.setAccessible(true);
        levelField.set(shell, world);
        var hit = new BlockHitResult(new Vec3(3.5, 65.0, -6.5), Direction.UP, POS, false);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            for (var line : (Iterable<String>) lines::iterator) {
                var parts = line.trim().split(" ");
                boolean fireCharge = parts[0].equals("fire_charge");
                world.state = switch (parts[1]) {
                    case "campfire" -> Blocks.CAMPFIRE.defaultBlockState().setValue(BlockStateProperties.LIT, false);
                    case "candle" -> Blocks.CANDLE.defaultBlockState().setValue(BlockStateProperties.LIT, false);
                    default -> Blocks.STONE.defaultBlockState();
                };
                RandomSource random = RandomSource.create(Long.parseLong(parts[2]));
                put(unsafe, world, "random", random);
                put(unsafe, world, "soundSeedGenerator", RandomSource.create(0));
                sounds.setLength(0);
                var context = new UseOnContext((net.minecraft.world.entity.player.Player) shell, InteractionHand.MAIN_HAND, hit);
                (fireCharge ? Items.FIRE_CHARGE : Items.FLINT_AND_STEEL).useOn(context);
                out.println(sounds + " " + random.nextLong());
            }
        }
        System.exit(0);
    }
}
