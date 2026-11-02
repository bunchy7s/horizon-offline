package dev.horizonlab.bench.mixin;

import dev.horizonlab.bench.VanillaBench;

import net.minecraft.client.Minecraft;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** Times the client tick (Minecraft.tick), for the stress scenes' client tick figure. */
@Mixin(Minecraft.class)
public abstract class TickHookMixin {
    @Inject(method = "tick", at = @At("HEAD"))
    private void horizonBenchTickStart(CallbackInfo callback) {
        VanillaBench.tick(true);
    }

    @Inject(method = "tick", at = @At("RETURN"))
    private void horizonBenchTickEnd(CallbackInfo callback) {
        VanillaBench.tick(false);
    }
}
