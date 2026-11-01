package dev.horizonlab.parity;

import com.google.gson.JsonParser;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.renderer.entity.state.ArmedEntityRenderState;
import net.minecraft.client.renderer.item.ItemModelResolver;
import net.minecraft.client.renderer.item.ItemStackRenderState;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.entity.HumanoidArm;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.item.ItemStack;

/** Record the actual stacks the shipped extraction method passes to the model resolver. */
public final class VanillaHeldItemStackOracle {
    static final class Player extends LocalPlayer {
        ItemStack main, off;
        HumanoidArm arm;
        Player() { super(null, null, null, null, null, null, false, null, null); }
        @Override public HumanoidArm getMainArm() { return arm; }
        @Override public ItemStack getMainHandItem() { return main; }
        @Override public ItemStack getOffhandItem() { return off; }
        @Override public ItemStack getItemInHand(InteractionHand hand) {
            return hand == InteractionHand.MAIN_HAND ? main : off;
        }
        @Override public LivingEntity.SwingDescription getCurrentSwing() { return null; }
        @Override public float getSwingAnimation(float partial) { return 0; }
    }
    static final class Resolver extends ItemModelResolver {
        final java.io.PrintWriter out;
        Resolver(java.io.PrintWriter out) { super(null); this.out = out; }
        @Override public void updateForLiving(ItemStackRenderState state, ItemStack stack,
                                               ItemDisplayContext context, LivingEntity entity) {
            out.println(BuiltInRegistries.ITEM.getKey(stack.getItem()));
        }
    }
    static ItemStack stack(String id) {
        return new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse("minecraft:" + id)));
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        for (var item : BuiltInRegistries.ITEM) {
            item.builtInRegistryHolder().bindComponents(net.minecraft.core.component.DataComponentMap.EMPTY);
        }
        var allocator = Class.forName("sun.misc.Unsafe");
        var field = allocator.getDeclaredField("theUnsafe");
        field.setAccessible(true);
        var player = (Player) allocator.getMethod("allocateInstance", Class.class)
            .invoke(field.get(null), Player.class);
        try (var out = new java.io.PrintWriter(Files.newBufferedWriter(Path.of(args[1])))) {
            var resolver = new Resolver(out);
            for (var line : Files.readAllLines(Path.of(args[0]))) {
                var c = JsonParser.parseString(line).getAsJsonObject();
                player.main = stack(c.get("main").getAsString());
                player.off = stack(c.get("off").getAsString());
                player.arm = c.get("left").getAsBoolean() ? HumanoidArm.LEFT : HumanoidArm.RIGHT;
                ArmedEntityRenderState.extractArmedEntityRenderState(
                    player, new ArmedEntityRenderState(), resolver, 0);
            }
        }
    }
}
