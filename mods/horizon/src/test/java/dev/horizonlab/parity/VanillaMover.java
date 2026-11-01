package dev.horizonlab.parity;

import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.util.Map;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Options;

/**
 * The vanilla client as an honest player for the anticheat's movement prediction
 * (tools/anticheat_e2e.py --check prediction-vanilla): it runs net.minecraft.client.main.Main
 * unchanged and presses its key mappings from lines on stdin, as a player's keyboard would:
 * "down forward", "up jump", "look 90 10" (yaw, pitch), "attack" (a click of the attack key:
 * Minecraft.startAttack, as handleKeybinds runs it). Everything the keys do is vanilla's own code. It prints "[VanillaMover] ready" once the player is in its level with no screen
 * open, and "[VanillaMover] at x y z" after each "where".
 */
public final class VanillaMover {
    public static void main(String[] args) throws Exception {
        Thread reader = new Thread(VanillaMover::read, "horizon-vanilla-mover");
        reader.setDaemon(true);
        reader.start();
        net.minecraft.client.main.Main.main(args);
    }

    private static void read() {
        try {
            Minecraft mc = waitForPlayer();
            System.out.println("[VanillaMover] ready");
            BufferedReader in = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
            for (String line; (line = in.readLine()) != null; ) {
                String[] words = line.trim().split("\\s+");
                mc.execute(() -> apply(mc, words));
            }
        } catch (Throwable t) {
            t.printStackTrace();
        }
    }

    private static Minecraft waitForPlayer() throws InterruptedException {
        while (true) {
            Thread.sleep(250);
            Minecraft mc = Minecraft.getInstance();
            if (mc != null && mc.player != null && mc.level != null && mc.gui.screen() == null) {
                return mc;
            }
        }
    }

    private static void apply(Minecraft mc, String[] words) {
        Options o = mc.options;
        Map<String, KeyMapping> keys = Map.of(
                "forward", o.keyUp, "back", o.keyDown, "left", o.keyLeft, "right", o.keyRight,
                "jump", o.keyJump, "sneak", o.keyShift, "sprint", o.keySprint, "use", o.keyUse);
        switch (words[0]) {
            case "down", "up" -> keys.get(words[1]).setDown(words[0].equals("down"));
            case "look" -> {
                mc.player.setYRot(Float.parseFloat(words[1]));
                mc.player.setXRot(Float.parseFloat(words[2]));
            }
            case "attack" -> {
                try {
                    var startAttack = Minecraft.class.getDeclaredMethod("startAttack");
                    startAttack.setAccessible(true);
                    startAttack.invoke(mc);
                } catch (ReflectiveOperationException e) {
                    throw new IllegalStateException(e);
                }
                System.out.println("[VanillaMover] attacked " + mc.hitResult.getType());
            }
            case "where" -> System.out.println("[VanillaMover] at " + mc.player.getX() + " " + mc.player.getY()
                    + " " + mc.player.getZ() + " screen " + mc.gui.screen());
            default -> System.out.println("[VanillaMover] unknown: " + String.join(" ", words));
        }
    }
}
