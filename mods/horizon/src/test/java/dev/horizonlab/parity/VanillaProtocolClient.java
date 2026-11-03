package dev.horizonlab.parity;

import com.google.gson.JsonElement;
import com.mojang.serialization.JsonOps;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import java.io.DataInputStream;
import java.io.EOFException;
import java.io.InputStream;
import java.io.OutputStream;
import java.io.PrintWriter;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.net.SocketTimeoutException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.time.Instant;
import java.util.ArrayList;
import java.util.BitSet;
import java.util.HexFormat;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.TreeMap;
import java.util.UUID;
import java.util.zip.DataFormatException;
import java.util.zip.Deflater;
import java.util.zip.Inflater;
import net.minecraft.SharedConstants;
import net.minecraft.core.UUIDUtil;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.world.level.chunk.LevelChunkSection;
import net.minecraft.world.level.chunk.PalettedContainerFactory;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.ComponentSerialization;
import net.minecraft.network.chat.LastSeenMessages;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.ProtocolInfo;
import net.minecraft.network.protocol.common.ClientboundCustomPayloadPacket;
import net.minecraft.network.protocol.common.ClientboundDisconnectPacket;
import net.minecraft.network.protocol.common.ClientboundKeepAlivePacket;
import net.minecraft.network.protocol.common.ClientboundPingPacket;
import net.minecraft.network.protocol.common.ServerboundClientInformationPacket;
import net.minecraft.network.protocol.common.ServerboundCustomPayloadPacket;
import net.minecraft.network.protocol.common.ServerboundKeepAlivePacket;
import net.minecraft.network.protocol.common.ServerboundPongPacket;
import net.minecraft.network.protocol.common.custom.BrandPayload;
import net.minecraft.network.protocol.configuration.ClientboundFinishConfigurationPacket;
import net.minecraft.network.protocol.configuration.ClientboundSelectKnownPacks;
import net.minecraft.network.protocol.configuration.ConfigurationProtocols;
import net.minecraft.network.protocol.configuration.ServerboundFinishConfigurationPacket;
import net.minecraft.network.protocol.configuration.ServerboundSelectKnownPacks;
import net.minecraft.network.protocol.game.ClientboundChunkBatchFinishedPacket;
import net.minecraft.network.protocol.game.ClientboundDisguisedChatPacket;
import net.minecraft.network.protocol.game.ClientboundLevelChunkPacketData;
import net.minecraft.network.protocol.game.ClientboundLevelChunkWithLightPacket;
import net.minecraft.network.protocol.game.ClientboundPlayerChatPacket;
import net.minecraft.network.protocol.game.ClientboundPlayerPositionPacket;
import net.minecraft.network.protocol.game.ClientboundSystemChatPacket;
import net.minecraft.network.protocol.game.GameProtocols;
import net.minecraft.network.protocol.game.ServerboundAcceptTeleportationPacket;
import net.minecraft.network.protocol.game.ServerboundChatCommandPacket;
import net.minecraft.network.protocol.game.ServerboundChatPacket;
import net.minecraft.network.protocol.game.ServerboundChunkBatchReceivedPacket;
import net.minecraft.network.protocol.game.ServerboundClientTickEndPacket;
import net.minecraft.network.protocol.game.ServerboundMovePlayerPacket;
import net.minecraft.network.protocol.game.ServerboundPlayerLoadedPacket;
import net.minecraft.network.protocol.handshake.ClientIntent;
import net.minecraft.network.protocol.handshake.ClientIntentionPacket;
import net.minecraft.network.protocol.handshake.HandshakeProtocols;
import net.minecraft.network.protocol.login.ClientboundLoginCompressionPacket;
import net.minecraft.network.protocol.login.ClientboundLoginDisconnectPacket;
import net.minecraft.network.protocol.login.ClientboundLoginFinishedPacket;
import net.minecraft.network.protocol.login.LoginProtocols;
import net.minecraft.network.protocol.login.ServerboundHelloPacket;
import net.minecraft.network.protocol.login.ServerboundLoginAcknowledgedPacket;
import net.minecraft.network.protocol.ping.ClientboundPongResponsePacket;
import net.minecraft.network.protocol.ping.ServerboundPingRequestPacket;
import net.minecraft.network.protocol.status.ClientboundStatusResponsePacket;
import net.minecraft.network.protocol.status.ServerStatus;
import net.minecraft.network.protocol.status.ServerboundStatusRequestPacket;
import net.minecraft.network.protocol.status.StatusProtocols;
import net.minecraft.server.level.ClientInformation;
import net.minecraft.world.entity.PositionMoveRotation;
import net.minecraft.world.phys.Vec3;

/**
 * A headless client made of vanilla's own protocol: it frames, compresses and
 * decodes every packet with the codecs of the pinned server JAR (the client
 * shares them), so any byte a server sends that vanilla would not read is
 * recorded as an error. tools/test_dedicated_server_parity.py points it at a
 * vanilla dedicated server and at Horizon's, with the same settings and
 * world, and compares the transcripts.
 *
 * <p>{@code VanillaProtocolClient HOST PORT NAME OUT} pings the server (status
 * and ping), joins as NAME in offline mode, goes through login and
 * configuration, accepts the spawn teleport, waits for the chunks around the
 * spawn, sends a chat message and the {@code /list} command, and leaves.
 * Each transcript line is a section and a summary: the login and
 * configuration packets in order; in play, the packets up to the first chunk
 * batch in order, then the chunks by position with the hash of their bytes
 * and the replies to the chat and command. Values that change between runs
 * (entity ids, keep-alive ids, times, salts) are not part of the summaries.
 */
public final class VanillaProtocolClient {
    private Socket socket;
    private DataInputStream in;
    private OutputStream out;
    private int threshold = -1;
    private final List<String> lines = new ArrayList<>();
    private ProtocolInfo<?> inbound;
    private ProtocolInfo<?> outbound;
    private String state;
    /** With a fifth argument {@code entities}: the entity transcript mode (EntityTranscript). */
    private EntityTranscript entities;
    /** With {@code ride}: the entity mode, plus the order of what the player sees of a ride through a portal (RideTranscript). */
    private RideTranscript ride;
    private GoatVoiceTranscript goatVoices;

    public static void main(String[] args) throws Exception {
        ParityData.init();
        String host = args[0];
        int port = Integer.parseInt(args[1]);
        String name = args[2];
        Path output = Path.of(args[3]);
        VanillaProtocolClient client = new VanillaProtocolClient();
        if (args.length > 4 && args[4].equals("entities")) client.entities = new EntityTranscript();
        if (args.length > 4 && args[4].equals("ride")) {
            client.entities = new EntityTranscript();
            client.ride = new RideTranscript();
        }
        if (args.length > 4 && args[4].equals("goat-voices")) {
            client.entities = new EntityTranscript();
            client.goatVoices = new GoatVoiceTranscript();
        }
        try {
            client.status(host, port);
            client.join(host, port, name);
        } catch (Exception e) {
            // With its causes: a decoder error names the field it could not read there.
            StringBuilder line = new StringBuilder("error " + client.state + " " + e);
            for (Throwable c = e.getCause(); c != null; c = c.getCause()) line.append(" / caused by ").append(c);
            client.lines.add(line.toString());
        } finally {
            if (client.socket != null) client.socket.close();
        }
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(output, StandardCharsets.UTF_8))) {
            for (String line : client.lines) w.println(line);
        }
        System.exit(0);
    }

    // ---- connection ---------------------------------------------------

    private void open(String host, int port) throws Exception {
        socket = new Socket();
        socket.connect(new InetSocketAddress(host, port), 10_000);
        socket.setSoTimeout(30_000);
        socket.setTcpNoDelay(true);
        in = new DataInputStream(socket.getInputStream());
        out = socket.getOutputStream();
        threshold = -1;
    }

    private static int readVarInt(InputStream in) throws Exception {
        int value = 0;
        for (int i = 0; i < 5; i++) {
            int b = in.read();
            if (b < 0) throw new EOFException();
            value |= (b & 0x7F) << (7 * i);
            if ((b & 0x80) == 0) return value;
        }
        throw new IllegalStateException("VarInt too big");
    }

    private static void writeVarInt(ByteBuf buf, int value) {
        while ((value & ~0x7F) != 0) {
            buf.writeByte((value & 0x7F) | 0x80);
            value >>>= 7;
        }
        buf.writeByte(value);
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    private void send(Packet<?> packet) throws Exception {
        ByteBuf body = Unpooled.buffer();
        ((ProtocolInfo) outbound).codec().encode(body, packet);
        byte[] raw = new byte[body.readableBytes()];
        body.readBytes(raw);
        ByteBuf frame = Unpooled.buffer();
        if (threshold >= 0) {
            ByteBuf inner = Unpooled.buffer();
            if (raw.length >= threshold) {
                writeVarInt(inner, raw.length);
                Deflater d = new Deflater();
                d.setInput(raw);
                d.finish();
                byte[] tmp = new byte[raw.length + 64];
                while (!d.finished()) {
                    int n = d.deflate(tmp);
                    inner.writeBytes(tmp, 0, n);
                }
                d.end();
            } else {
                writeVarInt(inner, 0);
                inner.writeBytes(raw);
            }
            writeVarInt(frame, inner.readableBytes());
            frame.writeBytes(inner);
        } else {
            writeVarInt(frame, raw.length);
            frame.writeBytes(raw);
        }
        byte[] bytes = new byte[frame.readableBytes()];
        frame.readBytes(bytes);
        out.write(bytes);
        out.flush();
    }

    /** The next packet's raw payload, decompressed as CompressionDecoder does (validating). */
    private byte[] readFrame() throws Exception {
        int length = readVarInt(in);
        byte[] frame = new byte[length];
        in.readFully(frame);
        if (threshold < 0) return frame;
        ByteBuf buf = Unpooled.wrappedBuffer(frame);
        int dataLength = readVarInt(new java.io.ByteArrayInputStream(frame));
        int header = varIntSize(dataLength);
        buf.skipBytes(header);
        if (dataLength == 0) {
            byte[] raw = new byte[buf.readableBytes()];
            buf.readBytes(raw);
            return raw;
        }
        if (dataLength < threshold) {
            throw new IllegalStateException("Badly compressed packet - size of " + dataLength + " is below server threshold of " + threshold);
        }
        if (dataLength > 8388608) {
            throw new IllegalStateException("Badly compressed packet - size of " + dataLength + " is larger than protocol maximum of 8388608");
        }
        Inflater inflater = new Inflater();
        inflater.setInput(frame, header, frame.length - header);
        byte[] raw = new byte[dataLength];
        int n;
        try {
            n = inflater.inflate(raw);
        } catch (DataFormatException e) {
            throw new IllegalStateException("bad zlib data", e);
        }
        boolean finished = inflater.finished();
        inflater.end();
        if (n != dataLength || !finished) throw new IllegalStateException("decompressed " + n + " of " + dataLength + " bytes");
        return raw;
    }

    private static int varIntSize(int v) {
        int n = 1;
        while ((v & ~0x7F) != 0) {
            v >>>= 7;
            n++;
        }
        return n;
    }

    private record Received(Packet<?> packet, byte[] payload, String type) {}

    /** The next packet, decoded with the inbound protocol; leftover bytes are an error (PacketDecoder). */
    private Received read() throws Exception {
        byte[] payload = readFrame();
        ByteBuf buf = Unpooled.wrappedBuffer(payload);
        Packet<?> packet = inbound.codec().decode(buf);
        if (buf.readableBytes() > 0) {
            throw new IllegalStateException("Packet " + inbound.id().id() + "/" + packet.type().id() + " was larger than I expected, found "
                + buf.readableBytes() + " bytes extra whilst reading");
        }
        return new Received(packet, payload, packet.type().id().toString());
    }

    // ---- summaries ----------------------------------------------------

    private static String sha(byte[] bytes) {
        try {
            return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes)).substring(0, 16);
        } catch (Exception e) {
            throw new RuntimeException(e);
        }
    }

    private static String json(Component c) {
        JsonElement e = ComponentSerialization.CODEC.encodeStart(ParityData.access.createSerializationContext(JsonOps.INSTANCE), c).getOrThrow();
        return e.toString();
    }

    /** A record's text without object identities; other objects by their fields. */
    static String text(Object o) {
        String s = String.valueOf(o);
        if (o != null && s.matches("[\\w.$]+@[0-9a-f]+")) {
            StringBuilder b = new StringBuilder(o.getClass().getSimpleName()).append('{');
            for (Class<?> c = o.getClass(); c != null && c != Object.class; c = c.getSuperclass()) {
                for (java.lang.reflect.Field f : c.getDeclaredFields()) {
                    if (java.lang.reflect.Modifier.isStatic(f.getModifiers())) continue;
                    try {
                        f.setAccessible(true);
                        b.append(f.getName()).append('=').append(text(f.get(o))).append(", ");
                    } catch (Exception e) {
                        b.append(f.getName()).append("=?, ");
                    }
                }
            }
            return b.append('}').toString();
        }
        return s.replaceAll("\\$\\$Lambda/0x[0-9a-f]+@", "\\$\\$Lambda@").replaceAll("@[0-9a-f]{4,}", "@");
    }

    private void record(String section, String line) {
        lines.add(section + " " + line);
    }

    // ---- status -------------------------------------------------------

    private void status(String host, int port) throws Exception {
        state = "status";
        open(host, port);
        outbound = HandshakeProtocols.SERVERBOUND;
        send(new ClientIntentionPacket(SharedConstants.getProtocolVersion(), host, port, ClientIntent.STATUS));
        outbound = StatusProtocols.SERVERBOUND;
        inbound = StatusProtocols.CLIENTBOUND;
        send(ServerboundStatusRequestPacket.INSTANCE);
        Received r = read();
        if (r.packet instanceof ClientboundStatusResponsePacket p) {
            JsonElement e = ServerStatus.CODEC.encodeStart(JsonOps.INSTANCE, p.status()).getOrThrow();
            // The favicon is compared by its bytes' hash.
            String s = e.toString().replaceAll("\"favicon\":\"[^\"]*\"", "\"favicon\":\"" + p.status().favicon().map(f -> sha(f.iconBytes())).orElse("") + "\"");
            record("status", r.type + " " + s);
        } else {
            record("status", r.type + " " + text(r.packet));
        }
        send(new ServerboundPingRequestPacket(1234567890123L));
        Received pong = read();
        record("status", pong.type + " " + (pong.packet instanceof ClientboundPongResponsePacket p ? p.time() : text(pong.packet)));
        int after = in.read();
        record("status", "closed-after-pong " + (after < 0));
        socket.close();
    }

    // ---- join ---------------------------------------------------------

    private void join(String host, int port, String name) throws Exception {
        open(host, port);
        state = "login";
        outbound = HandshakeProtocols.SERVERBOUND;
        send(new ClientIntentionPacket(SharedConstants.getProtocolVersion(), host, port, ClientIntent.LOGIN));
        outbound = LoginProtocols.SERVERBOUND;
        inbound = LoginProtocols.CLIENTBOUND;
        UUID uuid = UUIDUtil.createOfflinePlayerUUID(name);
        send(new ServerboundHelloPacket(name, uuid));
        while (true) {
            Received r = read();
            if (r.packet instanceof ClientboundLoginCompressionPacket p) {
                threshold = p.getCompressionThreshold();
                record("login", r.type + " " + threshold);
            } else if (r.packet instanceof ClientboundLoginFinishedPacket p) {
                record("login", r.type + " " + p.gameProfile().id() + " " + p.gameProfile().name() + " " + p.gameProfile().properties().size());
                send(ServerboundLoginAcknowledgedPacket.INSTANCE);
                break;
            } else if (r.packet instanceof ClientboundLoginDisconnectPacket p) {
                record("login", r.type + " " + json(p.reason()));
                return;
            } else {
                record("login", r.type + " " + text(r.packet));
            }
        }
        state = "configuration";
        outbound = ConfigurationProtocols.SERVERBOUND;
        inbound = ConfigurationProtocols.CLIENTBOUND;
        // ClientHandshakePacketListenerImpl.handleLoginFinished: brand, then the options.
        send(new ServerboundCustomPayloadPacket(new BrandPayload("vanilla")));
        send(new ServerboundClientInformationPacket(ClientInformation.createDefault()));
        while (true) {
            Received r = read();
            if (r.packet instanceof ClientboundSelectKnownPacks p) {
                record("config", r.type + " " + text(p.knownPacks()));
                send(new ServerboundSelectKnownPacks(p.knownPacks()));
            } else if (r.packet instanceof ClientboundKeepAlivePacket p) {
                record("config", r.type);
                send(new ServerboundKeepAlivePacket(p.getId()));
            } else if (r.packet instanceof ClientboundPingPacket p) {
                record("config", r.type);
                send(new ServerboundPongPacket(p.getId()));
            } else if (r.packet instanceof ClientboundCustomPayloadPacket p) {
                record("config", r.type + " " + text(p.payload()));
            } else if (r.packet instanceof net.minecraft.network.protocol.common.ClientboundUpdateTagsPacket p) {
                // Each registry's tags and what they hold. Vanilla sends the
                // registries and tags in the order of maps keyed by interned
                // ResourceKeys (identity hashes), which changes between runs.
                var registries = new TreeMap<String, String>();
                for (var e : p.tags().entrySet()) {
                    var tags = new TreeMap<String, String>();
                    for (var t : e.getValue().tags().entrySet()) {
                        tags.put(t.getKey().toString(), t.getValue().toString());
                    }
                    registries.put(e.getKey().identifier().toString(), tags.size() + " " + sha(tags.toString().getBytes()));
                }
                record("config", r.type + " " + registries.size() + " registries");
                registries.forEach((k, v) -> record("config-tags", k + " " + v));
            } else if (r.packet instanceof ClientboundFinishConfigurationPacket) {
                record("config", r.type);
                send(ServerboundFinishConfigurationPacket.INSTANCE);
                break;
            } else if (r.packet instanceof ClientboundDisconnectPacket p) {
                record("config", r.type + " " + json(p.reason()));
                return;
            } else {
                record("config", r.type + " " + (r.payload.length > 200 ? "sha=" + sha(r.payload) : text(r.packet)));
            }
        }
        play();
    }

    /** The entity mode's step, and how long it records the entities for (ms). */
    private static final int ENTITIES = 1000;
    private static final long ENTITY_WAIT = 20_000;
    /** The ride mode records until the console says "RIDE END" in chat, at most this long (ms). */
    private static final long RIDE_WAIT = 150_000;
    /**
     * How long a step waits at most for what it waits on (a villager to trade with, the ingot, a thrown
     * snowball, the pearl's teleport and hurt): a busy machine can hold a server's ticks back for seconds,
     * so the steps wait for those, not for a fixed time.
     */
    private static final long PATIENCE = 30_000;

    static int moveEntityId(net.minecraft.network.protocol.game.ClientboundMoveEntityPacket p) {
        return (int) field(p, net.minecraft.network.protocol.game.ClientboundMoveEntityPacket.class, "entityId");
    }

    static int rotateHeadId(net.minecraft.network.protocol.game.ClientboundRotateHeadPacket p) {
        return (int) field(p, p.getClass(), "entityId");
    }

    private static final String[] REPLIES = {"random value 7..7", "random value x", "trigger nope"};

    private void play() throws Exception {
        state = "play";
        GameProtocols.Context context = new GameProtocols.Context() {
            @Override
            public boolean hasInfiniteMaterials() {
                return false;
            }

            @Override
            public boolean canUseCommandBlocks() {
                return false;
            }
        };
        outbound = GameProtocols.SERVERBOUND_TEMPLATE.bind(RegistryFriendlyByteBuf.decorator(ParityData.access), context);
        inbound = GameProtocols.CLIENTBOUND_TEMPLATE.bind(RegistryFriendlyByteBuf.decorator(ParityData.access));
        socket.setSoTimeout(250);
        Map<String, String> chunks = new TreeMap<>();
        Map<String, String> layers = new TreeMap<>();
        Map<String, Integer> counts = new TreeMap<>();
        boolean ordered = true;
        boolean loaded = false;
        PositionMoveRotation position = new PositionMoveRotation(Vec3.ZERO, Vec3.ZERO, 0, 0);
        long lastChunk = 0;
        long deadline = System.currentTimeMillis() + (goatVoices == null ? 150_000 : 600_000);
        int step = 0;
        long stepAt = 0;
        // The death the console deals and the respawn after it (PlayerList.respawn): from the
        // client's PERFORM_RESPAWN until its level is loaded again (LevelLoadTracker: the level
        // info's "level chunks load start" event, then the chunk under the player).
        boolean respawning = false;
        boolean respawned = false;
        boolean loadStart = false;
        boolean respawnChunk = false;
        String runType = null;
        int run = 0;
        PositionMoveRotation respawnAt = null;
        // The furnace set beside the respawn point: its window and the menu's state id.
        net.minecraft.core.BlockPos furnace = null;
        int furnaceWindow = -1;
        int furnaceState = 0;
        // Snowballs the server spawned since the last right click.
        int snowballs = 0;
        // When the first of them came, and when the furnace's ingot did.
        long snowballAt = 0;
        long smeltedAt = 0;
        // The player's own id, and what the ender pearl it throws does to it.
        int playerId = -1;
        boolean pearl = false;
        int pearlTeleports = 0;
        float pearlHealth = -1;
        // The villager the server's console summoned beside the spawn.
        int villager = -1;
        net.minecraft.world.phys.Vec3 villagerAt = null;
        // When the dead player sent a command (ServerGamePacketListenerImpl.handleChatCommand runs it
        // whether the player is alive or not); it respawns once the reply came, or 3 s later.
        long deadCommandAt = 0;
        // When the client last sent where it is.
        long positionAt = 0;
        // Chat lines and furnace slot changes received, and how many there were when the current step sent
        // its command or click: a step goes on once the server answered, not after a fixed time (a busy
        // server answers seconds late, and a click sent before the last one's reply carries a stale state id).
        int replies = 0;
        int repliesBefore = 0;
        // A shift-click changes two slots, each its own packet, which a busy server can send ticks apart: a furnace
        // step waits for both. A full resend of the menu counts as both.
        int furnaceReplies = 0;
        int furnaceBefore = 0;
        // The furnace's lit and cooking totals received: the server sends them a tick after what changed them.
        int furnaceData = 0;
        while (System.currentTimeMillis() < deadline) {
            Received r;
            try {
                r = read();
            } catch (SocketTimeoutException e) {
                r = null;
            }
            long now = System.currentTimeMillis();
            if (r != null) {
                Packet<?> packet = r.packet;
                counts.merge(r.type, 1, Integer::sum);
                if (entities != null && !entities.acceptSelf(packet) && step >= ENTITIES) entities.accept(packet);
                if (ride != null && step >= ENTITIES) ride.accept(packet);
                if (ride != null && ride.takeBoard() >= 0) {
                    // The player right-clicks the vehicle (Entity.interact: it takes the player on).
                    send(new net.minecraft.network.protocol.game.ServerboundInteractPacket(ride.boarded(),
                        net.minecraft.world.InteractionHand.MAIN_HAND, new net.minecraft.world.phys.Vec3(0.0, 0.5, 0.0), false));
                }
                if (ride != null) ride.self(packet);
                if (goatVoices != null) goatVoices.accept(packet);
                if (r.type.equals("minecraft:chunk_batch_start")) ordered = false;
                if (respawning && !respawned) {
                    String line = respawnLine(r, playerId);
                    // PlayerList.respawn ends with initInventoryMenu: what follows is the
                    // next ticks' (health, entity data, chunks), in each server's own order.
                    respawned = r.type.equals("minecraft:container_set_content") && respawnAt != null;
                    if (line != null && !line.equals(runType)) {
                        if (runType != null) record("play-respawn", runLine(runType, run));
                        runType = line;
                        run = 0;
                    }
                    if (line != null) run++;
                    if (packet instanceof net.minecraft.network.protocol.game.ClientboundGameEventPacket p
                        && p.getEvent() == net.minecraft.network.protocol.game.ClientboundGameEventPacket.LEVEL_CHUNKS_LOAD_START) {
                        loadStart = true;
                    }
                }
                if (packet instanceof net.minecraft.network.protocol.game.ClientboundLoginPacket p) playerId = p.playerId();
                // The first health after the pearl: the hurt (regeneration follows each server's ticks).
                if (pearl && pearlHealth < 0 && packet instanceof net.minecraft.network.protocol.game.ClientboundSetHealthPacket p) {
                    pearlHealth = p.getHealth();
                }
                if (packet instanceof ClientboundKeepAlivePacket p) {
                    send(new ServerboundKeepAlivePacket(p.getId()));
                } else if (packet instanceof ClientboundPingPacket p) {
                    send(new ServerboundPongPacket(p.getId()));
                } else if (packet instanceof ClientboundPlayerPositionPacket p) {
                    position = PositionMoveRotation.calculateAbsolute(position, p.change(), p.relatives());
                    // Where a pearl lands follows its random inaccuracy: only counted.
                    if (pearl) pearlTeleports++;
                    else record("play-move", text(position.position()) + " " + position.yRot() + " " + position.xRot());
                    if (respawning) respawnAt = position;
                    send(new ServerboundAcceptTeleportationPacket(p.id(), position.position().x, position.position().y, position.position().z,
                        position.yRot(), position.xRot()));
                    // The 26.3 client answers with the acceptance alone; a second position in the
                    // same tick is an invalid move to vanilla's server once the player is loaded.
                    // The entity mode's scene may teleport the player (to another level): the
                    // acceptance alone, as the 26.3 client sends.
                    if (!respawning && step < ENTITIES) {
                        send(new ServerboundMovePlayerPacket.PosRot(position.position(), position.yRot(), position.xRot(), false, false));
                        positionAt = now;
                    }
                    if (!loaded) {
                        send(new ServerboundPlayerLoadedPacket());
                        loaded = true;
                    }
                } else if (packet instanceof ClientboundChunkBatchFinishedPacket) {
                    send(new ServerboundChunkBatchReceivedPacket(64.0F));
                } else if (packet instanceof ClientboundLevelChunkWithLightPacket p) {
                    chunks.put(p.x() + "," + p.z(), chunkHash(p));
                    layers.put(p.x() + "," + p.z(), lightLayers(p));
                    lastChunk = now;
                    int px = (int) Math.floor(position.position().x) >> 4;
                    int pz = (int) Math.floor(position.position().z) >> 4;
                    if (p.x() == px && p.z() == pz && !respawning) chunkParts(p);
                    if (p.x() == px && p.z() == pz && respawnAt != null) respawnChunk = true;
                } else if (packet instanceof net.minecraft.network.protocol.game.ClientboundContainerSetContentPacket p
                    && furnaceWindow > 0 && p.containerId() == furnaceWindow) {
                    furnaceState = p.stateId();
                    furnaceReplies += 2;
                    record("play-furnace", "content " + p.items() + " carried " + p.carriedItem());
                } else if (packet instanceof net.minecraft.network.protocol.game.ClientboundContainerSetSlotPacket p
                    && furnaceWindow > 0 && p.getContainerId() == furnaceWindow) {
                    furnaceState = p.getStateId();
                    furnaceReplies++;
                    record("play-furnace", "slot " + p.getSlot() + " " + p.getItem());
                    if (p.getSlot() == 2 && !p.getItem().isEmpty() && smeltedAt == 0) smeltedAt = now;
                } else if (packet instanceof net.minecraft.network.protocol.game.ClientboundContainerSetDataPacket p
                    && furnaceWindow > 0 && p.getContainerId() == furnaceWindow && (p.getId() == 1 || p.getId() == 3)) {
                    // Lit total and cooking total; the time left and the cooking timer follow the ticks.
                    furnaceData++;
                    record("play-furnace", "data " + p.getId() + " " + p.getValue());
                } else if (packet instanceof net.minecraft.network.protocol.game.ClientboundEntityEventPacket p
                    && step == 5 + REPLIES.length && (int) (byte) field(p, "eventId") == 28) {
                    // Operator (level 4, as the console made it): it may run /kill on itself.
                    send(new ServerboundChatCommandPacket("kill"));
                    step++;
                } else if (packet instanceof net.minecraft.network.protocol.game.ClientboundPlayerCombatKillPacket p && step > 0) {
                    // The death message is left out: guests' death messages aren't Horizon's yet.
                    record("play-death", "death screen");
                    send(new ServerboundChatCommandPacket("trigger nope"));
                    deadCommandAt = now;
                } else if (packet instanceof ClientboundSystemChatPacket p && deadCommandAt > 0 && !respawning
                    && json(p.content()).contains("nope")) {
                    record("play-death", "command reply " + json(p.content()));
                    respawning = respawn();
                } else if (packet instanceof ClientboundSystemChatPacket p && step > 0 && step < 5 + REPLIES.length) {
                    replies++;
                    record("play-chat", "system " + p.overlay() + " " + json(p.content()));
                } else if (packet instanceof ClientboundPlayerChatPacket p && step > 0 && step < 5 + REPLIES.length) {
                    replies++;
                    record("play-chat", "player " + p.sender() + " " + p.index() + " " + p.body().content() + " " + p.signature().isPresent()
                        + " " + p.unsignedContent().map(VanillaProtocolClient::json) + " " + text(p.filterMask()) + " "
                        + p.chatType().chatType().getRegisteredName() + " " + json(p.chatType().name()) + " "
                        + p.chatType().targetName().map(VanillaProtocolClient::json));
                } else if (packet instanceof ClientboundDisguisedChatPacket p && step > 0 && step < 5 + REPLIES.length) {
                    replies++;
                    record("play-chat", "disguised " + json(p.message()) + " " + p.chatType().chatType().getRegisteredName());
                } else if (packet instanceof net.minecraft.network.protocol.game.ClientboundAddEntityPacket p
                    && p.getType() == net.minecraft.world.entity.EntityTypes.SNOWBALL) {
                    snowballs++;
                    if (snowballAt == 0) snowballAt = now;
                    // Projectile.getAddEntityPacket: its owner's id.
                    record("play-use", "snowball owner is the player: " + (p.getData() == playerId));
                } else if (packet instanceof net.minecraft.network.protocol.game.ClientboundAddEntityPacket p
                    && p.getType() == net.minecraft.world.entity.EntityTypes.VILLAGER) {
                    villager = p.getId();
                    villagerAt = new net.minecraft.world.phys.Vec3(p.getX(), p.getY(), p.getZ());
                } else if (packet instanceof net.minecraft.network.protocol.game.ClientboundOpenScreenPacket p && step > 6 + REPLIES.length) {
                    furnaceWindow = p.getContainerId();
                    record("play-furnace", "open " + net.minecraft.core.registries.BuiltInRegistries.MENU.getKey(p.getType()) + " "
                        + json(p.getTitle()));
                } else if (packet instanceof net.minecraft.network.protocol.game.ClientboundOpenScreenPacket p && step > 0) {
                    record("play-trade", "open " + p.getContainerId() + " "
                        + net.minecraft.core.registries.BuiltInRegistries.MENU.getKey(p.getType()) + " "
                        // The villager's UUID is each server's own.
                        + json(p.getTitle()).replaceAll("\"uuid\":\\[[^\\]]*\\]", "\"uuid\":\"<uuid>\"")
                            .replaceAll("\"insertion\":\"[^\"]*\"", "\"insertion\":\"<uuid>\""));
                } else if (packet instanceof net.minecraft.network.protocol.game.ClientboundMerchantOffersPacket p && step > 0) {
                    StringBuilder sb = new StringBuilder("offers " + p.getContainerId() + " " + p.getVillagerLevel() + " "
                        + p.getVillagerXp() + " " + p.showProgress() + " " + p.canRestock());
                    for (var o : p.getOffers()) {
                        sb.append(" | ").append(o.getItemCostA()).append(' ').append(o.getItemCostB()).append(' ')
                            .append(net.minecraft.world.item.ItemStack.CODEC.encodeStart(
                                ParityData.access.createSerializationContext(com.mojang.serialization.JsonOps.INSTANCE), o.getResult()).getOrThrow())
                            .append(' ').append(o.isOutOfStock()).append(' ').append(o.getUses()).append(' ').append(o.getMaxUses())
                            .append(' ').append(o.getXp()).append(' ').append(o.getSpecialPriceDiff()).append(' ')
                            .append(Float.floatToRawIntBits(o.getPriceMultiplier())).append(' ').append(o.getDemand());
                    }
                    record("play-trade", sb.toString());
                } else if (packet instanceof ClientboundDisconnectPacket p) {
                    record("play", r.type + " " + json(p.reason()));
                    break;
                }
                // MinecraftServer.tickServer syncs the game time every 20 ticks
                // (no clocks): whether one lands before the first chunk batch
                // depends on when the client joined, so it isn't compared.
                boolean periodicTime = packet instanceof net.minecraft.network.protocol.game.ClientboundSetTimePacket p
                    && p.clockUpdates().isEmpty();
                if (ordered && !periodicTime) {
                    record("play-start", r.type + " " + summary(r));
                }
                if (packet instanceof net.minecraft.network.protocol.game.ClientboundCommandsPacket p && counts.get(r.type) == 1) {
                    commandTree(p);
                }
            }
            if (entities == null && loaded && deadCommandAt == 0 && now - positionAt >= 1000) {
                // LocalPlayer.sendPosition: once its level is loaded the 26.3 client sends where it is
                // every 20 ticks even standing still, and the server then checks again which entities the
                // player sees (ChunkMap.move). An entity whose chunk was sent after the player's last move,
                // such as the villager when the first chunks come late, is paired at the next one. The tick
                // end first (Minecraft.tick sends one every tick): a server takes one position per client tick.
                // Until the death: from then on the steps wait only on entities spawned for the player.
                send(ServerboundClientTickEndPacket.INSTANCE);
                send(new ServerboundMovePlayerPacket.Pos(position.position(), false, false));
                positionAt = now;
            }
            if (entities != null && loaded && lastChunk > 0 && now - lastChunk > 3000 && step == 0) {
                // Entity mode: the harness's console summons the scene once it reads this
                // line; everything tracked from then on is recorded until the client leaves.
                record("play-chunks", String.valueOf(chunks.size()));
                send(new ServerboundChatPacket(goatVoices == null ? "summon the entities" : "summon the goats", Instant.now(), 0L, Optional.empty(),
                    new LastSeenMessages.Update(0, new BitSet(), (byte) 0)));
                step = ENTITIES;
                stepAt = now;
            } else if (step == ENTITIES && ride != null && (ride.done() || now - stepAt > RIDE_WAIT)) {
                for (String line : ride.lines()) record("play-ride", line);
                return;
            } else if (step == ENTITIES && ride == null && (goatVoices == null ? now - stepAt > ENTITY_WAIT : goatVoices.done())) {
                if (goatVoices == null) for (String line : entities.lines()) record("play-entities", line);
                else for (String line : goatVoices.lines()) record("play-goat-voice", line);
                return;
            }
            if (goatVoices != null) {
                String ack = goatVoices.acknowledgement();
                if (ack != null) send(new ServerboundChatPacket(ack, Instant.now(), 0L, Optional.empty(),
                    new LastSeenMessages.Update(0, new BitSet(), (byte) 0)));
            }
            // Once the chunks stop coming: chat, then /list, then leave.
            if (entities == null && loaded && lastChunk > 0 && now - lastChunk > 3000 && step == 0) {
                record("play-chunks", chunks.size() + " " + chunks);
                BLOCK_ENTITIES.forEach((k, v) -> record("play-block-entities", k + " " + v));
                // Which light layers exist depends on the neighbouring
                // columns: compared where all eight were sent.
                Map<String, String> inner = new TreeMap<>();
                layers.forEach((k, v) -> {
                    String[] xz = k.split(",");
                    int x = Integer.parseInt(xz[0]), z = Integer.parseInt(xz[1]);
                    boolean all = true;
                    for (int dz = -1; dz <= 1; dz++) for (int dx = -1; dx <= 1; dx++) all &= chunks.containsKey((x + dx) + "," + (z + dz));
                    if (all) inner.put(k, v);
                });
                record("play-light-layers", inner.size() + " " + inner);
                step = 1;
                stepAt = now;
                repliesBefore = replies;
                send(new ServerboundChatPacket("hello from the parity client", Instant.now(), 0L, Optional.empty(),
                    new LastSeenMessages.Update(0, new BitSet(), (byte) 0)));
            } else if (step == 1 && answered(replies, repliesBefore, stepAt, now)) {
                step = 2;
                stepAt = now;
                repliesBefore = replies;
                send(new ServerboundChatCommandPacket("list"));
            } else if (step >= 2 && step < 2 + REPLIES.length && answered(replies, repliesBefore, stepAt, now)) {
                // Commands a player without operator rights may run, whose
                // replies (a failure, a syntax error with its context, a
                // failure with an argument) are compared component by component.
                send(new ServerboundChatCommandPacket(REPLIES[step - 2]));
                step++;
                stepAt = now;
                repliesBefore = replies;
            } else if (step == 2 + REPLIES.length && answered(replies, repliesBefore, stepAt, now)
                && (villager >= 0 || now - stepAt > PATIENCE)) {
                // Right-click the villager (once the server sent it): the trading screen and its offers.
                record("play-trade", "villager " + (villager >= 0));
                if (villager >= 0) {
                    send(new net.minecraft.network.protocol.game.ServerboundInteractPacket(villager,
                        net.minecraft.world.InteractionHand.MAIN_HAND, new net.minecraft.world.phys.Vec3(0.0, 1.0, 0.0), false));
                }
                step++;
                stepAt = now;
            } else if (step == 3 + REPLIES.length && now - stepAt > 1500) {
                send(new net.minecraft.network.protocol.game.ServerboundSelectTradePacket(0));
                step++;
                stepAt = now;
            } else if (step == 4 + REPLIES.length && now - stepAt > 1500) {
                send(new net.minecraft.network.protocol.game.ServerboundContainerClosePacket(1));
                record("play-counts", "chunks " + counts.getOrDefault("minecraft:level_chunk_with_light", 0));
                // The harness's console makes the player an operator once it reads this line.
                send(new ServerboundChatPacket("ready to respawn", Instant.now(), 0L, Optional.empty(),
                    new LastSeenMessages.Update(0, new BitSet(), (byte) 0)));
                step++;
                stepAt = now;
            } else if (deadCommandAt > 0 && !respawning && now - deadCommandAt > 3000) {
                record("play-death", "no command reply");
                respawning = respawn();
            } else if (step == 6 + REPLIES.length && loadStart && respawnChunk) {
                // LevelLoadTracker is done: the vanilla client leaves the loading screen.
                if (runType != null) record("play-respawn", runLine(runType, run));
                record("play-respawn", "level loaded at " + text(respawnAt.position()));
                send(new ServerboundPlayerLoadedPacket());
                // A furnace two blocks south, an ore and its fuel.
                furnace = net.minecraft.core.BlockPos.containing(respawnAt.position()).south(2);
                send(new ServerboundChatCommandPacket("setblock " + furnace.getX() + " " + furnace.getY() + " " + furnace.getZ()
                    + " minecraft:furnace"));
                send(new ServerboundChatCommandPacket("give @s minecraft:raw_iron"));
                send(new ServerboundChatCommandPacket("give @s minecraft:coal"));
                // A score on @s is the player's own (its feedback reaches the console: broadcast);
                // a query's reply doesn't.
                send(new ServerboundChatCommandPacket("scoreboard objectives add parity dummy"));
                send(new ServerboundChatCommandPacket("scoreboard players set @s parity 3"));
                send(new ServerboundChatCommandPacket("scoreboard players get @s parity"));
                step++;
                stepAt = now;
            } else if (step == 7 + REPLIES.length && now - stepAt > 1500) {
                // Right-click it with an empty hand (AbstractFurnaceBlock.useWithoutItem).
                send(new net.minecraft.network.protocol.game.ServerboundUseItemOnPacket(net.minecraft.world.InteractionHand.MAIN_HAND,
                    new net.minecraft.world.phys.BlockHitResult(net.minecraft.world.phys.Vec3.atCenterOf(furnace),
                        net.minecraft.core.Direction.NORTH, furnace, false), 1));
                furnaceBefore = furnaceReplies;
                step++;
                stepAt = now;
            } else if (step >= 8 + REPLIES.length && step <= 10 + REPLIES.length && furnaceWindow > 0
                && answered(furnaceReplies, furnaceBefore + 1, stepAt, now)
                // Both totals with the window, then the cooking total the ore set, then the lit total the coal set.
                && (furnaceData >= step - 6 - REPLIES.length || now - stepAt > PATIENCE)
                // The ingot out once the furnace made it (200 ticks), and its tick's other changes came.
                && (step < 10 + REPLIES.length || smeltedAt > 0 && now - smeltedAt > 1000 || now - stepAt > 10_000 + PATIENCE)) {
                // Shift-click the ore (hotbar 0) into the input, the coal (hotbar 1) into the
                // fuel slot, and the smelted ingot out of the result; nothing claimed, so the
                // server sends every slot the click changed.
                int slot = step == 8 + REPLIES.length ? 30 : step == 9 + REPLIES.length ? 31 : 2;
                record("play-furnace", "quick move " + slot);
                send(new net.minecraft.network.protocol.game.ServerboundContainerClickPacket(furnaceWindow, furnaceState, (short) slot,
                    (byte) 0, net.minecraft.world.inventory.ContainerInput.QUICK_MOVE,
                    new it.unimi.dsi.fastutil.ints.Int2ObjectOpenHashMap<>(), net.minecraft.network.HashedStack.EMPTY));
                furnaceBefore = furnaceReplies;
                step++;
                stepAt = now;
            } else if (step == 11 + REPLIES.length && answered(furnaceReplies, furnaceBefore + 1, stepAt, now)) {
                send(new net.minecraft.network.protocol.game.ServerboundContainerClosePacket(furnaceWindow));
                record("play-furnace", "closed");
                step++;
                stepAt = now;
            } else if (step == 12 + REPLIES.length && now - stepAt > 1500) {
                // Once the furnace is closed on the server too: the slot change isn't the furnace menu's.
                send(new ServerboundChatCommandPacket("item replace entity @s weapon.mainhand with minecraft:snowball 16"));
                step++;
                stepAt = now;
            } else if (step == 13 + REPLIES.length && now - stepAt > 1500) {
                // A right click on the ground with a snowball, as the 26.3 client sends it
                // (Minecraft.startUseItem): the block's part, which passes, then the item's.
                useOn(respawnAt, net.minecraft.world.InteractionHand.MAIN_HAND, 2);
                use(position, net.minecraft.world.InteractionHand.MAIN_HAND, 3);
                snowballs = 0;
                snowballAt = 0;
                step++;
                stepAt = now;
            } else if (step == 14 + REPLIES.length && thrown(snowballAt, stepAt, now)) {
                record("play-use", "main hand on a block: " + snowballs + " snowball(s)");
                send(new ServerboundChatCommandPacket("item replace entity @s weapon.mainhand with minecraft:air"));
                send(new ServerboundChatCommandPacket("item replace entity @s weapon.offhand with minecraft:snowball 16"));
                step++;
                stepAt = now;
            } else if (step == 15 + REPLIES.length && now - stepAt > 1500) {
                // An empty main hand: its block part only (no item to use), then the off hand's two.
                useOn(respawnAt, net.minecraft.world.InteractionHand.MAIN_HAND, 4);
                useOn(respawnAt, net.minecraft.world.InteractionHand.OFF_HAND, 5);
                use(position, net.minecraft.world.InteractionHand.OFF_HAND, 6);
                snowballs = 0;
                snowballAt = 0;
                step++;
                stepAt = now;
            } else if (step == 16 + REPLIES.length && thrown(snowballAt, stepAt, now)) {
                record("play-use", "off hand on a block: " + snowballs + " snowball(s)");
                // An ender pearl thrown at its feet (no endermite: ThrownEnderpearl spawns one 5% of the time).
                send(new ServerboundChatCommandPacket("gamerule spawn_monsters false"));
                send(new ServerboundChatCommandPacket("item replace entity @s weapon.offhand with minecraft:air"));
                send(new ServerboundChatCommandPacket("item replace entity @s weapon.mainhand with minecraft:ender_pearl"));
                step++;
                stepAt = now;
            } else if (step == 17 + REPLIES.length && now - stepAt > 1500) {
                pearl = true;
                send(new net.minecraft.network.protocol.game.ServerboundUseItemPacket(net.minecraft.world.InteractionHand.MAIN_HAND, 7,
                    position.yRot(), 90.0F));
                step++;
                stepAt = now;
            } else if (step == 18 + REPLIES.length && now - stepAt > 2500
                && (pearlTeleports > 0 && pearlHealth >= 0 || now - stepAt > PATIENCE)) {
                // ThrownEnderpearl.onHit: the thrower is teleported and takes 5 damage.
                record("play-pearl", "teleported " + pearlTeleports + " time(s), health " + pearlHealth);
                return;
            }
        }
        if (respawning) {
            if (runType != null) record("play-respawn", runLine(runType, run));
            record("play-respawn", "level never loaded: load start " + loadStart + ", chunk under the player " + respawnChunk);
        }
        record("error", "timed out in play, step " + step + ", chunks " + chunks.size());
    }

    /** The server answered the step's command or click (at least 1.5 s after it, as before), or had all the time. */
    private static boolean answered(int count, int before, long sent, long now) {
        return now - sent > 1500 && (count > before || now - sent > PATIENCE);
    }

    /**
     * A right click's snowballs are counted: once the first came and a second could have come with it
     * (the server spawns each in the tick it handles the click), or once the server had all the time.
     */
    private static boolean thrown(long first, long clicked, long now) {
        return now - clicked > 2000 && (first > 0 && now - first > 1000 || now - clicked > PATIENCE);
    }

    /** The death screen's Respawn button: true, the respawn being under way. */
    private boolean respawn() throws Exception {
        send(new net.minecraft.network.protocol.game.ServerboundClientCommandPacket(
            net.minecraft.network.protocol.game.ServerboundClientCommandPacket.Action.PERFORM_RESPAWN));
        return true;
    }

    /**
     * A run of the same packet: counted, but for the entities removed with the dead player
     * (which mobs are tracked near the spawn at that moment varies).
     */
    private static String runLine(String type, int run) {
        return type + (run > 1 && !type.equals("minecraft:remove_entities") ? " x" + run : "");
    }

    /** ServerboundUseItemOnPacket on the top of the block under the player. */
    private void useOn(PositionMoveRotation at, net.minecraft.world.InteractionHand hand, int sequence) throws Exception {
        net.minecraft.core.BlockPos ground = net.minecraft.core.BlockPos.containing(at.position()).below();
        send(new net.minecraft.network.protocol.game.ServerboundUseItemOnPacket(hand,
            new net.minecraft.world.phys.BlockHitResult(net.minecraft.world.phys.Vec3.atCenterOf(ground).add(0.0, 0.5, 0.0),
                net.minecraft.core.Direction.UP, ground, false), sequence));
    }

    /** ServerboundUseItemPacket, looking where the player looks. */
    private void use(PositionMoveRotation at, net.minecraft.world.InteractionHand hand, int sequence) throws Exception {
        send(new net.minecraft.network.protocol.game.ServerboundUseItemPacket(hand, sequence, at.yRot(), at.xRot()));
    }

    /**
     * A packet after the respawn request, as compared: entity ids and periodic or streamed
     * packets (time, keep-alive, chunk data and light, whose batching follows each server's
     * timing) left out; null to skip it.
     */
    private String respawnLine(Received r, int playerId) {
        String t = r.type;
        // Other entities (a squid's swim event, a mob coming into view) land in the window as each server's
        // ticks fall: only the player's own are PlayerList.respawn's.
        Integer entity = entityOf(r.packet);
        if (entity != null && entity != playerId) return null;
        // Entities moving, blocks the world's ticks change (a lava fall flows again once the game
        // runs) and the dead player's health land in the window between the death and the respawn
        // as each server's ticks fall: left out.
        if (t.startsWith("minecraft:move_entity") || t.equals("minecraft:rotate_head") || t.equals("minecraft:set_entity_motion")
            || t.equals("minecraft:set_entity_data") || t.equals("minecraft:entity_position_sync") || t.equals("minecraft:set_health")
            || t.equals("minecraft:block_update") || t.equals("minecraft:section_blocks_update")) {
            return null;
        }
        if (t.equals("minecraft:keep_alive") || t.equals("minecraft:ping") || t.equals("minecraft:level_chunk_with_light")
            || t.equals("minecraft:chunk_batch_start") || t.equals("minecraft:chunk_batch_finished")
            || t.equals("minecraft:light_update") || t.equals("minecraft:bundle_delimiter")
            // Chat (the death message and /kill's feedback) is compared before the death.
            || t.equals("minecraft:system_chat") || t.equals("minecraft:player_chat") || t.equals("minecraft:disguised_chat")) {
            return null;
        }
        if (r.packet instanceof net.minecraft.network.protocol.game.ClientboundSetTimePacket p && p.clockUpdates().isEmpty()) {
            return null;
        }
        if (r.packet instanceof net.minecraft.network.protocol.game.ClientboundEntityEventPacket p) {
            return t + " " + field(p, "eventId");
        }
        if (r.packet instanceof net.minecraft.network.protocol.game.ClientboundRemoveEntitiesPacket p) {
            return t;
        }
        if (t.equals("minecraft:respawn") || t.equals("minecraft:game_event") || t.equals("minecraft:set_default_spawn_position")
            || t.equals("minecraft:change_difficulty") || t.equals("minecraft:set_experience") || t.equals("minecraft:set_health")
            || t.equals("minecraft:set_chunk_cache_center") || t.equals("minecraft:initialize_border")) {
            return t + " " + text(r.packet);
        }
        return t;
    }

    /** The entity a packet is about, for the packets that name one; null for the rest. */
    private static Integer entityOf(Packet<?> packet) {
        if (packet instanceof net.minecraft.network.protocol.game.ClientboundAddEntityPacket p) return p.getId();
        if (packet instanceof net.minecraft.network.protocol.game.ClientboundUpdateAttributesPacket p) return p.getEntityId();
        if (packet instanceof net.minecraft.network.protocol.game.ClientboundEntityEventPacket p) return (int) field(p, "entityId");
        if (packet instanceof net.minecraft.network.protocol.game.ClientboundSetEquipmentPacket p) return p.getEntity();
        if (packet instanceof net.minecraft.network.protocol.game.ClientboundAnimatePacket p) return p.getId();
        if (packet instanceof net.minecraft.network.protocol.game.ClientboundHurtAnimationPacket p) return p.id();
        if (packet instanceof net.minecraft.network.protocol.game.ClientboundDamageEventPacket p) return p.entityId();
        if (packet instanceof net.minecraft.network.protocol.game.ClientboundUpdateMobEffectPacket p) return p.getEntityId();
        if (packet instanceof net.minecraft.network.protocol.game.ClientboundRemoveMobEffectPacket p) return p.entityId();
        if (packet instanceof net.minecraft.network.protocol.game.ClientboundSoundEntityPacket p) return p.getId();
        return null;
    }

    /**
     * A chunk's blocks, biomes, heightmaps and block entities (its
     * ClientboundLevelChunkPacketData bytes). The light is left out: both servers relight around chunks
     * loaded with their saved light, which test_light_parity covers.
     */
    private static final Map<String, String> BLOCK_ENTITIES = new TreeMap<>();

    private static String chunkHash(ClientboundLevelChunkWithLightPacket p) {
        var data = p.chunkData();
        var heightmaps = new TreeMap<String, String>();
        data.getHeightmaps().forEach((k, v) -> heightmaps.put(k.getSerializedName(), java.util.Arrays.toString(v)));
        FriendlyByteBuf buf = data.getReadBuffer();
        byte[] sections = new byte[buf.readableBytes()];
        buf.getBytes(buf.readerIndex(), sections);
        // What the sections hold, apart from how their palettes encode it.
        FriendlyByteBuf copy = new FriendlyByteBuf(Unpooled.wrappedBuffer(sections));
        var factory = PalettedContainerFactory.create(ParityData.access);
        StringBuilder content = new StringBuilder();
        while (copy.isReadable()) {
            LevelChunkSection section = new LevelChunkSection(factory);
            section.read(copy);
            for (int y = 0; y < 16; y++) for (int z = 0; z < 16; z++) for (int x = 0; x < 16; x++) {
                content.append(net.minecraft.world.level.block.Block.getId(section.getBlockState(x, y, z))).append(',');
            }
            for (int y = 0; y < 4; y++) for (int z = 0; z < 4; z++) for (int x = 0; x < 4; x++) {
                content.append(section.getNoiseBiome(x, y, z).getRegisteredName()).append(',');
            }
        }
        StringBuilder entities = new StringBuilder();
        data.forEachBlockEntityTag(p.x(), p.z(), (pos, type, tag) -> entities.append(pos.toShortString()).append(' ')
            .append(net.minecraft.core.registries.BuiltInRegistries.BLOCK_ENTITY_TYPE.getKey(type)).append(' ').append(tag).append(';'));
        if (entities.length() > 0) BLOCK_ENTITIES.put(p.x() + "," + p.z(), entities.toString());
        return "c" + sha(content.toString().getBytes()).substring(0, 8) + "s" + sha(sections).substring(0, 8) + "h" + sha(heightmaps.toString().getBytes()).substring(0, 8) + "e"
            + sha(entities.toString().getBytes()).substring(0, 8);
    }

    /** The light layers a chunk has, sky then block (with data or empty). */
    private static String lightLayers(ClientboundLevelChunkWithLightPacket p) {
        var light = p.lightData();
        BitSet sky = (BitSet) light.skyYMask().clone();
        sky.or(light.emptySkyYMask());
        BitSet block = (BitSet) light.blockYMask().clone();
        block.or(light.emptyBlockYMask());
        return sky + "/" + block;
    }

    /** The player's chunk part by part, so a difference in its bytes can be told apart. */
    private void chunkParts(ClientboundLevelChunkWithLightPacket p) {
        var data = p.chunkData();
        for (var e : new TreeMap<>(data.getHeightmaps().entrySet().stream()
            .collect(java.util.stream.Collectors.toMap(x -> x.getKey().getSerializedName(), x -> x.getValue()))).entrySet()) {
            record("chunk-heightmap", e.getKey() + " " + java.util.Arrays.toString(e.getValue()).hashCode());
        }
        FriendlyByteBuf buf = data.getReadBuffer();
        byte[] all = new byte[buf.readableBytes()];
        buf.getBytes(buf.readerIndex(), all);
        var factory = PalettedContainerFactory.create(ParityData.access);
        int index = 0;
        while (buf.isReadable()) {
            int start = buf.readerIndex();
            LevelChunkSection section = new LevelChunkSection(factory);
            section.read(buf);
            StringBuilder states = new StringBuilder();
            for (int y = 0; y < 16; y++) for (int z = 0; z < 16; z++) for (int x = 0; x < 16; x++) {
                states.append(net.minecraft.world.level.block.Block.getId(section.getBlockState(x, y, z))).append(',');
            }
            StringBuilder biomes = new StringBuilder();
            for (int y = 0; y < 4; y++) for (int z = 0; z < 4; z++) for (int x = 0; x < 4; x++) {
                biomes.append(section.getNoiseBiome(x, y, z).getRegisteredName()).append(',');
            }
            byte[] bytes = new byte[buf.readerIndex() - start];
            buf.getBytes(start, bytes);
            record("chunk-section", index++ + " states=" + sha(states.toString().getBytes()) + " biomes=" + sha(biomes.toString().getBytes())
                + " count=" + (((bytes[0] & 0xFF) << 8) | (bytes[1] & 0xFF)) + " bytes=" + sha(bytes));
        }
        StringBuilder entities = new StringBuilder();
        data.forEachBlockEntityTag(p.x(), p.z(), (pos, type, tag) -> entities.append(pos.toShortString()).append(' ')
            .append(net.minecraft.core.registries.BuiltInRegistries.BLOCK_ENTITY_TYPE.getKey(type)).append(' ').append(tag).append(';'));
        record("chunk-block-entities", entities.toString());
    }

    /** The command tree, one line per top-level command with its subtree. */
    private void commandTree(net.minecraft.network.protocol.game.ClientboundCommandsPacket p) {
        var context = net.minecraft.commands.CommandBuildContext.simple(ParityData.access, net.minecraft.world.flag.FeatureFlags.VANILLA_SET);
        var root = p.getRoot(context, new net.minecraft.network.protocol.game.ClientboundCommandsPacket.NodeBuilder<Object>() {
            @Override
            public com.mojang.brigadier.builder.ArgumentBuilder<Object, ?> createLiteral(String id) {
                return com.mojang.brigadier.builder.LiteralArgumentBuilder.literal(id);
            }

            @Override
            public com.mojang.brigadier.builder.ArgumentBuilder<Object, ?> createArgument(String id, com.mojang.brigadier.arguments.ArgumentType<?> type,
                net.minecraft.resources.Identifier suggestionId) {
                return com.mojang.brigadier.builder.RequiredArgumentBuilder.argument(id + ":" + type.getClass().getSimpleName() + ":" + suggestionId, type);
            }

            @Override
            public com.mojang.brigadier.builder.ArgumentBuilder<Object, ?> configure(com.mojang.brigadier.builder.ArgumentBuilder<Object, ?> input,
                boolean executable, boolean restricted) {
                if (executable) input.executes(c -> 0);
                return input;
            }
        });
        // Children in vanilla's registration order, but the game rules', which
        // GameRuleCommand registers in an identity-hashed order: sorted.
        var names = new java.util.ArrayList<String>();
        for (var child : root.getChildren()) {
            names.add(child.getName());
            var paths = new java.util.ArrayList<String>();
            describe(child, "", paths, new java.util.IdentityHashMap<>());
            record("play-command", child.getName() + " " + paths.size() + " " + sha(String.join("\n", paths).getBytes()));
        }
        record("play-command-order", String.join(" ", names));
    }

    private static void describe(com.mojang.brigadier.tree.CommandNode<Object> node, String path, java.util.List<String> out,
        java.util.IdentityHashMap<Object, Boolean> seen) {
        String here = path + " " + node.getName() + (node.getCommand() != null ? "*" : "");
        out.add(here + (node.getRedirect() != null ? " -> " + node.getRedirect().getName() : ""));
        if (seen.put(node, true) != null) return;
        var children = new java.util.ArrayList<>(node.getChildren());
        if (here.equals(" gamerule")) children.sort(java.util.Comparator.comparing(com.mojang.brigadier.tree.CommandNode::getName));
        for (var child : children) describe(child, here, out, seen);
    }

    /** The first play packets: entity ids, seeds and times aside. */
    private String summary(Received r) {
        String t = r.type;
        if (t.equals("minecraft:login")) {
            // levels: the server's level keys, in an identity-hashed order after the overworld.
            var login = (net.minecraft.network.protocol.game.ClientboundLoginPacket) r.packet;
            var levels = new java.util.TreeSet<String>();
            login.levels().forEach(l -> levels.add(l.identifier().toString()));
            return text(r.packet).replaceAll("playerId=-?\\d+", "playerId=_").replaceAll("levels=\\[(ResourceKey\\[[^\\]]*\\](, )?)*\\]", "levels=" + levels);
        }
        if (t.equals("minecraft:set_time") || t.equals("minecraft:keep_alive") || t.equals("minecraft:ping")) return "";
        // The tree is compared below: its node numbering follows the game rules' identity-hashed order.
        if (t.equals("minecraft:commands")) return "";
        // Vanilla's hash maps and sets order these per run: compared sorted.
        if (t.equals("minecraft:update_recipes")) {
            var p = (net.minecraft.network.protocol.game.ClientboundUpdateRecipesPacket) r.packet;
            var sets = new TreeMap<String, String>();
            p.itemSets().forEach((k, v) -> {
                var items = new java.util.TreeSet<String>();
                ((java.util.Set<?>) field(v, "items")).forEach(h -> items.add(((net.minecraft.core.Holder<?>) h).getRegisteredName()));
                sets.put(k.identifier().toString(), items.toString());
            });
            p.stonecutterRecipes().entries().forEach(e -> record("play-stonecutter", field(e.input(), "values") + " -> " + e.recipe().optionDisplay()));
            sets.forEach((k, v) -> record("play-property-set", k + " " + v));
            return "sets=" + sets.size() + " stonecutter=" + p.stonecutterRecipes().entries().size();
        }
        if (t.equals("minecraft:update_advancements")) {
            var p = (net.minecraft.network.protocol.game.ClientboundUpdateAdvancementsPacket) r.packet;
            var added = new TreeMap<String, String>();
            for (var a : p.added()) {
                var adv = a.advancement().value();
                added.put(a.advancement().id().toString(), adv.parent() + " " + adv.display().map(d -> json(d.title()) + " " + json(d.description())
                    + " " + d.icon() + " " + d.type() + " " + d.background() + " " + d.showToast() + " " + d.hidden()).orElse("-")
                    + " " + a.x() + "," + a.y() + " " + adv.requirements().requirements() + " " + adv.sendsTelemetryEvent());
            }
            var progress = new TreeMap<String, String>();
            p.progress().forEach((k, v) -> {
                var criteria = new TreeMap<String, Boolean>();
                ((Map<?, ?>) field(v, "criteria")).forEach((c, cp) -> criteria.put((String) c,
                    ((net.minecraft.advancements.CriterionProgress) cp).isDone()));
                progress.put(k.toString(), criteria.toString());
            });
            added.forEach((k, v) -> record("play-advancement", k + " " + v));
            return "reset=" + p.shouldReset() + " show=" + p.showAdvancements() + " added=" + added.size()
                + " removed=" + new java.util.TreeSet<>(p.removed()) + " progress=" + progress;
        }
        if (t.equals("minecraft:recipe_book_add")) return text(r.packet) + " sha=" + sha(r.payload);
        if (t.equals("minecraft:set_entity_data")) {
            return text(((net.minecraft.network.protocol.game.ClientboundSetEntityDataPacket) r.packet).packedItems());
        }
        if (t.contains("entity") || t.equals("minecraft:add_entity") || t.equals("minecraft:update_attributes")
            || t.equals("minecraft:player_info_update") || t.equals("minecraft:player_position")) {
            return "";
        }
        return r.payload.length > 200 ? "sha=" + sha(r.payload) : text(r.packet);
    }

    private static Object field(Object owner, String name) {
        return field(owner, owner.getClass(), name);
    }

    private static Object field(Object owner, Class<?> declaring, String name) {
        try {
            var f = declaring.getDeclaredField(name);
            f.setAccessible(true);
            return f.get(owner);
        } catch (ReflectiveOperationException e) {
            throw new RuntimeException(e);
        }
    }
}
