package dev.horizonlab.parity;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientboundAddEntityPacket;
import net.minecraft.network.protocol.game.ClientboundDisguisedChatPacket;
import net.minecraft.network.protocol.game.ClientboundSoundPacket;
import net.minecraft.network.protocol.game.ClientboundSoundEntityPacket;
import net.minecraft.network.protocol.game.ClientboundSystemChatPacket;
import net.minecraft.world.phys.Vec3;

/** The sound packets of the frozen goat scene, with server-chat phase barriers. */
final class GoatVoiceTranscript {
    private final Map<Integer, Vec3> goats = new HashMap<>();
    private final List<String> lines = new ArrayList<>();
    private String phase = "setup";
    private String pendingAck;
    private boolean tracked;
    private boolean done;

    void accept(Packet<?> packet) {
        if (packet instanceof ClientboundAddEntityPacket p
                && BuiltInRegistries.ENTITY_TYPE.getKey(p.getType()).toString().equals("minecraft:goat")) {
            // Keep spawn positions through removal: a death sound can name the removed entity.
            goats.put(p.getId(), new Vec3(p.getX(), p.getY(), p.getZ()));
        }
        String text = null;
        if (packet instanceof ClientboundSystemChatPacket p) text = p.content().getString();
        else if (packet instanceof ClientboundDisguisedChatPacket p) text = p.message().getString();
        if (text != null) {
            for (String name : List.of("ambient", "hurt", "death")) {
                if (text.endsWith("goat_voice_" + name)) {
                    phase = name;
                    pendingAck = "goat voice phase " + name;
                }
            }
            if (text.endsWith("goat_voice_done")) done = true;
        }
        if (packet instanceof ClientboundSoundPacket p) {
            sound(p.getSound().value().location().toString(), p.getSource().ordinal(),
                p.getVolume(), p.getPitch(), new Vec3(p.getX(), p.getY(), p.getZ()));
        } else if (packet instanceof ClientboundSoundEntityPacket p) {
            sound(p.getSound().value().location().toString(), p.getSource().ordinal(),
                p.getVolume(), p.getPitch(), goats.get(p.getId()));
        }
    }

    private void sound(String name, int category, float volume, float pitch, Vec3 at) {
        if (at != null && !phase.equals("setup") && name.startsWith("minecraft:entity.goat.")) {
            lines.add(phase + " " + name + " " + category + " " + volume + " " + pitch
                + " " + at.x + " " + at.z);
        }
    }

    String acknowledgement() {
        if (!tracked && goats.size() >= 18) {
            tracked = true;
            return "goat voices tracked";
        }
        String ack = pendingAck;
        pendingAck = null;
        return ack;
    }

    boolean done() { return done; }
    List<String> lines() { return lines; }
}
