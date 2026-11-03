package dev.horizonlab.parity;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientboundAddEntityPacket;
import net.minecraft.network.protocol.game.ClientboundEntityPositionSyncPacket;
import net.minecraft.network.protocol.game.ClientboundMoveEntityPacket;
import net.minecraft.network.protocol.game.ClientboundPlayerPositionPacket;
import net.minecraft.network.protocol.game.ClientboundRemoveEntitiesPacket;
import net.minecraft.network.protocol.game.ClientboundRespawnPacket;
import net.minecraft.network.protocol.game.ClientboundSetEntityMotionPacket;
import net.minecraft.network.protocol.game.ClientboundSetPassengersPacket;
import net.minecraft.network.protocol.game.ClientboundSystemChatPacket;
import net.minecraft.network.protocol.game.ClientboundTeleportEntityPacket;
import net.minecraft.world.entity.EntityTypes;

/**
 * What a player riding a vehicle through a portal sees, in the order the packets arrive: the level change
 * (respawn), the vehicle and its passengers (spawn, passengers), where and how fast the vehicle moves, and the
 * player's own teleports. The console puts a chat line "RIDE T<n>" after each tick it steps, so the lines
 * carry the tick they belong to; "RIDE END" ends the recording. Entity ids differ between servers: the player
 * is "player", the others are named by type and order of appearance.
 */
final class RideTranscript {
    private final List<String> lines = new ArrayList<>();
    private final Map<Integer, String> names = new HashMap<>();
    private final Map<String, Integer> counts = new HashMap<>();
    private int self = Integer.MIN_VALUE;
    private boolean done;
    private boolean boardAsked;
    private int vehicle = -1;

    /** The vehicle to right-click once the console asks for it ("RIDE BOARD"), else -1; asked for once. */
    int takeBoard() {
        if (!boardAsked || vehicle < 0) return -1;
        boardAsked = false;
        add("board");
        return vehicle;
    }

    int boarded() {
        return vehicle;
    }

    boolean done() {
        return done;
    }

    List<String> lines() {
        return lines;
    }

    /** The player's own id, from its login (the packets before the scene count too). */
    void self(Packet<?> packet) {
        if (packet instanceof net.minecraft.network.protocol.game.ClientboundLoginPacket p) self = p.playerId();
    }

    private String name(int id) {
        return id == self ? "player" : names.getOrDefault(id, "unknown");
    }

    private void add(String line) {
        lines.add(line);
    }

    void accept(Packet<?> packet) {
        switch (packet) {
            case ClientboundSystemChatPacket p -> {
                String text = p.content().getString();
                if (text.contains("RIDE END")) done = true;
                else if (text.contains("RIDE BOARD")) boardAsked = true;
                else if (text.contains("RIDE T")) add(text.substring(text.indexOf("RIDE T")));
            }
            case ClientboundRespawnPacket p -> {
                java.util.regex.Matcher m = java.util.regex.Pattern.compile("minecraft:(overworld|the_nether|the_end)")
                    .matcher(VanillaProtocolClient.text(p));
                add("respawn into " + (m.find() ? m.group(1) : "?"));
                // The client's level is cleared: every entity but the player is gone until it is sent again.
                names.clear();
                counts.clear();
            }
            case ClientboundAddEntityPacket p -> {
                if (p.getType() == EntityTypes.PLAYER) return;
                String type = BuiltInRegistries.ENTITY_TYPE.getKey(p.getType()).getPath();
                int n = counts.merge(type, 1, Integer::sum);
                names.put(p.getId(), type + n);
                if (type.endsWith("minecart") || type.endsWith("boat") || type.endsWith("raft")) vehicle = p.getId();
                add("add " + type + n + " at " + p.getX() + " " + p.getY() + " " + p.getZ() + " motion " + p.getMovement());
            }
            case ClientboundSetPassengersPacket p -> {
                List<String> riders = new ArrayList<>();
                for (int id : p.getPassengers()) riders.add(name(id));
                add("passengers of " + name(p.getVehicle()) + " " + riders);
            }
            case ClientboundRemoveEntitiesPacket p -> {
                List<String> gone = new ArrayList<>();
                p.entityIds().forEach(id -> gone.add(name(id)));
                add("remove " + gone);
            }
            case ClientboundSetEntityMotionPacket p -> add("motion " + name(p.id()) + " " + p.movement());
            case ClientboundTeleportEntityPacket p -> add("teleport " + name(p.id()) + " " + p.change().position()
                + " delta " + p.change().deltaMovement());
            case ClientboundEntityPositionSyncPacket p -> add("sync " + name(p.id()) + " " + p.position().endPosition());
            case ClientboundMoveEntityPacket p -> add("move " + name(VanillaProtocolClient.moveEntityId(p)));
            case ClientboundPlayerPositionPacket p -> add("player position " + p.change().position() + " delta "
                + p.change().deltaMovement());
            default -> {
            }
        }
    }
}
