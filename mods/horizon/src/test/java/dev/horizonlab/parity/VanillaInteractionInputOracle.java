package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.PrintWriter;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.*;
import net.minecraft.client.gui.Gui;
import net.minecraft.client.multiplayer.*;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.core.*;
import net.minecraft.network.protocol.Packet;
import net.minecraft.world.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.player.*;
import net.minecraft.world.flag.*;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.component.SwingAnimation;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.border.WorldBorder;
import net.minecraft.world.phys.*;

/** Runs the shipped handleKeybinds; only game-mode effects and UI are shells. */
public final class VanillaInteractionInputOracle {
    static Object unsafe;
    static Method allocate;
    static List<String> events = new ArrayList<>();
    static Field field(Class<?> type, String name) throws Exception {
        var f = type.getDeclaredField(name); f.setAccessible(true); return f;
    }
    static <T> T make(Class<T> type) throws Exception { return type.cast(allocate.invoke(unsafe, type)); }
    public static class MC extends Minecraft {
        public MC() { super(null); }
        @Override public boolean hasControlDown() { return false; }
    }
    public static class UI extends Gui {
        public UI() { super(null, null, null); }
        @Override public void handleKeybinds() {}
    }
    public static class Mouse extends MouseHandler {
        public Mouse() { super(null); }
        @Override public boolean isMouseGrabbed() { return true; }
    }
    public static class Level extends ClientLevel {
        public Level() { super(null, null, null, null, 0, 0, null, false, 0L, 0); }
        @Override public BlockState getBlockState(BlockPos pos) { return Blocks.STONE.defaultBlockState(); }
        @Override public FeatureFlagSet enabledFeatures() { return FeatureFlags.REGISTRY.allFlags(); }
    }
    public static class Connection extends ClientPacketListener {
        public Connection() { super(null, null, null); }
        @Override public void send(Packet<?> packet) {}
    }
    public static class Human extends LocalPlayer {
        boolean using;
        ItemStack held = ItemStack.EMPTY;
        public Human() { super(null, null, null, null, null, null, false, null, null); }
        @Override public boolean isUsingItem() { return using; }
        @Override public boolean isSpectator() { return false; }
        @Override public boolean isCreative() { return false; }
        @Override public boolean isHandsBusy() { return false; }
        @Override public ItemStack getItemInHand(InteractionHand hand) {
            return hand == InteractionHand.MAIN_HAND ? held : ItemStack.EMPTY;
        }
        @Override public void itemUsed(InteractionHand hand) {}
        @Override public boolean cannotAttackWithItem(ItemStack item, int slot) { return false; }
        @Override public boolean isWithinEntityInteractionRange(Entity e, double buffer) { return true; }
        @Override public boolean swing(InteractionHand hand, SwingAnimation animation, boolean send) { return false; }
    }
    public static class Mode extends MultiPlayerGameMode {
        boolean destroying, digOnAttack, beginUsing, passing;
        Human player;
        public Mode() { super(null, null); }
        @Override public boolean isDestroying() { return destroying; }
        @Override public boolean isSpectator() { return false; }
        @Override public boolean startDestroyBlock(BlockPos pos, Direction face) {
            events.add("attack"); destroying = digOnAttack; return true;
        }
        @Override public boolean continueDestroyBlock(BlockPos pos, Direction face) {
            events.add("continue true"); return true;
        }
        @Override public void stopDestroyBlock() { events.add("continue false"); destroying = false; }
        @Override public void attack(Player p, Entity e) { events.add("attack"); }
        @Override public void releaseUsingItem(Player p) { events.add("release"); player.using = false; }
        InteractionResult use() { events.add("use"); player.using = beginUsing; return InteractionResult.SUCCESS_SERVER; }
        @Override public InteractionResult useItemOn(LocalPlayer p, InteractionHand h, BlockHitResult hit) {
            if (passing) { events.add("block"); return InteractionResult.PASS; }
            return use();
        }
        @Override public InteractionResult interact(Player p, Entity e, EntityHitResult hit, InteractionHand h) {
            if (passing) { events.add("entity"); return InteractionResult.PASS; }
            return use();
        }
        @Override public InteractionResult useItem(Player p, InteractionHand h) {
            events.add("item"); player.using = beginUsing; return InteractionResult.SUCCESS_SERVER;
        }
        @Override public void handlePickItemFromBlock(BlockPos pos, boolean data) { events.add("pick"); }
        @Override public void handlePickItemFromEntity(Entity entity, boolean data) { events.add("pick"); }
    }
    static boolean flag(JsonObject row, String name) { return row.get(name).getAsBoolean(); }
    public static void main(String[] args) throws Exception {
        ParityData.init();
        var u = Class.forName("sun.misc.Unsafe"); unsafe = field(u, "theUnsafe").get(null);
        allocate = u.getMethod("allocateInstance", Class.class);
        var handle = Minecraft.class.getDeclaredMethod("handleKeybinds"); handle.setAccessible(true);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1])));
             var lines = Files.lines(Path.of(args[0]))) {
            int i = 0;
            for (String line : (Iterable<String>) lines::iterator) {
                var row = JsonParser.parseString(line).getAsJsonObject();
                var mc = make(MC.class); var options = make(Options.class);
                for (var f : Options.class.getDeclaredFields()) {
                    if (Modifier.isStatic(f.getModifiers())) continue;
                    f.setAccessible(true);
                    if (f.getType() == KeyMapping.class) f.set(options, make(KeyMapping.class));
                    if (f.getType() == KeyMapping[].class) {
                        var keys = new KeyMapping[9];
                        for (int n = 0; n < keys.length; n++) keys[n] = make(KeyMapping.class);
                        f.set(options, keys);
                    }
                }
                var player = make(Human.class); player.using = flag(row, "using");
                boolean passing = row.has("passing") && flag(row, "passing");
                player.held = passing ? new ItemStack(Blocks.STONE.asItem()) : ItemStack.EMPTY;
                var level = make(Level.class);
                field(ClientLevel.class, "worldBorder").set(level, new WorldBorder());
                field(Player.class, "abilities").set(player, new Abilities());
                field(LocalPlayer.class, "connection").set(player, make(Connection.class));
                var mode = make(Mode.class); mode.player = player;
                mode.destroying = flag(row, "destroying"); mode.digOnAttack = flag(row, "dig_on_attack");
                mode.beginUsing = flag(row, "begin_using");
                mode.passing = passing;
                field(Minecraft.class, "options").set(mc, options);
                field(Minecraft.class, "gui").set(mc, make(UI.class));
                field(Minecraft.class, "mouseHandler").set(mc, make(Mouse.class));
                field(Minecraft.class, "player").set(mc, player);
                field(Minecraft.class, "level").set(mc, level);
                field(Minecraft.class, "gameMode").set(mc, mode);
                var entity = make(net.minecraft.world.entity.item.ItemEntity.class);
                field(Entity.class, "blockPosition").set(entity, BlockPos.ZERO);
                field(Minecraft.class, "hitResult").set(mc, flag(row, "entity")
                    ? new EntityHitResult(entity, Vec3.ZERO)
                    : new BlockHitResult(Vec3.ZERO, Direction.UP, BlockPos.ZERO, false));
                String[] names = {"keyAttack", "keyUse", "keyPickItem"};
                for (var click : row.getAsJsonArray("clicks")) {
                    var key = (KeyMapping)field(Options.class, names[click.getAsInt()]).get(options);
                    var count = field(KeyMapping.class, "clickCount");
                    count.setInt(key, count.getInt(key) + 1);
                }
                field(KeyMapping.class, "isDown").setBoolean(options.keyAttack, flag(row, "attack_down"));
                field(KeyMapping.class, "isDown").setBoolean(options.keyUse, flag(row, "use_down"));
                field(Minecraft.class, "rightClickDelay").setInt(mc, row.get("delay").getAsInt());
                if (row.has("screen") && flag(row, "screen")) {
                    // A keybind opened a screen: Gui.setScreen runs
                    // KeyMapping.releaseAll() before the same handleKeybinds call
                    // reaches its release branch.
                    var release = KeyMapping.class.getDeclaredMethod("release");
                    release.setAccessible(true);
                    for (var holder : Options.class.getDeclaredFields()) {
                        if (Modifier.isStatic(holder.getModifiers()) || holder.getType() != KeyMapping.class) continue;
                        holder.setAccessible(true);
                        release.invoke(holder.get(options));
                    }
                }
                events.clear(); handle.invoke(mc);
                out.println("== " + i++);
                for (String event : events) out.println(event);
                out.println("state " + player.using + " " + mode.destroying + " " + field(Minecraft.class, "rightClickDelay").getInt(mc));
            }
        }
    }
}
