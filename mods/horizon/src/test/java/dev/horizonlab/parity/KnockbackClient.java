package dev.horizonlab.parity;

import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import java.io.BufferedReader;
import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.DataInputStream;
import java.io.FileDescriptor;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.io.OutputStream;
import java.io.PrintStream;
import java.io.PrintWriter;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.zip.Deflater;
import java.util.zip.Inflater;
import net.minecraft.SharedConstants;
import net.minecraft.core.UUIDUtil;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.ProtocolInfo;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.protocol.Packet;
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
import net.minecraft.network.protocol.game.ClientboundExplodePacket;
import net.minecraft.network.protocol.game.ClientboundPlayerPositionPacket;
import net.minecraft.network.protocol.game.ClientboundSystemChatPacket;
import net.minecraft.network.protocol.game.GameProtocols;
import net.minecraft.network.protocol.game.ClientboundSetEntityMotionPacket;
import net.minecraft.network.protocol.game.ServerboundAttackPacket;
import net.minecraft.network.protocol.game.ServerboundPlayerCommandPacket;
import net.minecraft.network.protocol.game.ServerboundPunchPacket;
import net.minecraft.network.protocol.game.ServerboundUseItemPacket;
import net.minecraft.world.InteractionHand;
import net.minecraft.network.protocol.game.ServerboundAcceptTeleportationPacket;
import net.minecraft.network.protocol.game.ServerboundChatCommandPacket;
import net.minecraft.network.protocol.game.ServerboundChunkBatchReceivedPacket;
import net.minecraft.network.protocol.game.ServerboundMovePlayerPacket;
import net.minecraft.network.protocol.game.ServerboundPlayerInputPacket;
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
import net.minecraft.server.level.ClientInformation;
import net.minecraft.world.entity.PositionMoveRotation;
import net.minecraft.world.entity.player.Input;
import net.minecraft.world.phys.Vec3;

/**
 * Players made of vanilla's own protocol code that hit and get hit, for
 * knockback (tools/test_server_knockback_parity.py). Script on stdin:
 * {@code step LABEL}, {@code join NAME}, {@code cmd NAME COMMAND...} (waits
 * for its feedback), {@code pos NAME X Y Z YAW PITCH GROUND} (one client tick
 * at that place, on the ground or not), {@code use-pos NAME X Y Z YAW PITCH GROUND}
 * (use the item, then move, in one network write), {@code sprint NAME on|off}, {@code
 * sync NAME} (a round trip through the server thread; "synced NAME" on stdout when
 * it is back), {@code attack NAME TARGET} (ServerboundAttackPacket then the swing; TARGET is a
 * player's name or an entity type's path, the last one added), {@code use
 * NAME} (ServerboundUseItemPacket, main hand) and {@code end}. Each player
 * records the ClientboundSetEntityMotionPacket it receives, the push each
 * ClientboundExplodePacket carries for it, its own sprint flag as the server
 * sets it, teleports, and the system chat (command feedback).
 */
public final class KnockbackClient {
    private static volatile String step = "start";
    private static final Map<UUID, String> NAMES = new LinkedHashMap<>();

    public static void main(String[] args) throws Exception {
        // Bootstrap sends System.out to the logger: `synced` lines go to the real stdout.
        PrintStream stdout = new PrintStream(new FileOutputStream(FileDescriptor.out), true, StandardCharsets.UTF_8);
        ParityData.init();
        String host = args[0];
        int port = Integer.parseInt(args[1]);
        Path output = Path.of(args[2]);
        Map<String, Player> players = new LinkedHashMap<>();
        BufferedReader script = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        String line;
        while ((line = script.readLine()) != null) {
            String[] w = line.trim().split(" ", 3);
            try {
                switch (w[0]) {
                    case "step" -> step = w[1];
                    case "join" -> {
                        Player p = new Player(w[1]);
                        players.put(w[1], p);
                        NAMES.put(UUIDUtil.createOfflinePlayerUUID(w[1]), w[1]);
                        p.join(host, port);
                    }
                    case "cmd" -> players.get(w[1]).command(w[2]);
                    case "pos" -> {
                        String[] a = w[2].split(" ");
                        players.get(w[1]).at(new Vec3(Double.parseDouble(a[0]), Double.parseDouble(a[1]), Double.parseDouble(a[2])),
                            Float.parseFloat(a[3]), Float.parseFloat(a[4]), a[5].equals("1"));
                    }
                    case "use-pos" -> {
                        String[] a = w[2].split(" ");
                        players.get(w[1]).useThenMove(new Vec3(Double.parseDouble(a[0]), Double.parseDouble(a[1]), Double.parseDouble(a[2])),
                            Float.parseFloat(a[3]), Float.parseFloat(a[4]), a[5].equals("1"));
                    }
                    case "glide" -> players.get(w[1]).playerCommand(ServerboundPlayerCommandPacket.Action.START_FALL_FLYING);
                    case "sprint" -> players.get(w[1]).sprint(w[2].equals("on"));
                    case "attack" -> players.get(w[1]).attack(w[2]);
                    case "use" -> players.get(w[1]).send(new ServerboundUseItemPacket(InteractionHand.MAIN_HAND, 0, 0, 0));
                    case "punch" -> players.get(w[1]).send(net.minecraft.network.protocol.game.ServerboundPunchPacket.INSTANCE);
                    case "sync" -> {
                        try {
                            players.get(w[1]).sync();
                        } finally {
                            stdout.println("synced " + w[1]);
                        }
                    }
                    case "mark" -> players.get(w[1]).mark();
                    case "await" -> players.get(w[1]).await();
                    case "end" -> {
                        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(output, StandardCharsets.UTF_8))) {
                            for (Player p : players.values()) {
                                synchronized (p.lines) {
                                    for (String l : p.lines) out.println(p.name + " " + l);
                                }
                            }
                        }
                        for (Player p : players.values()) p.close();
                        System.exit(0);
                    }
                    default -> throw new IllegalArgumentException("unknown script line " + line);
                }
            } catch (Exception e) {
                Player p = w.length > 1 ? players.get(w[1]) : null;
                if (p != null) p.record("error " + w[0] + " " + e);
                else System.err.println("error " + line + ": " + e);
            }
        }
        System.exit(1);
    }

    private static final class Player {
        final String name;
        final List<String> lines = new ArrayList<>();
        private Socket socket;
        private DataInputStream in;
        private OutputStream out;
        private int threshold = -1;
        private ProtocolInfo<?> inbound;
        private ProtocolInfo<?> outbound;
        private volatile PositionMoveRotation position = new PositionMoveRotation(Vec3.ZERO, Vec3.ZERO, 0, 0);
        private boolean loaded;
        private volatile int id = -1;
        /** Every entity this player was sent: its id to a name (players) or type path. */
        private final Map<Integer, String> seen = new java.util.concurrent.ConcurrentHashMap<>();
        private final Map<String, Integer> last = new java.util.concurrent.ConcurrentHashMap<>();
        private final Object replyLock = new Object();
        private int replies;
        private int syncs;
        /** The motion packets about this player it was sent, and how many there were at the last `mark`. */
        private int pushes;
        private int marked;

        Player(String name) {
            this.name = name;
        }

        void command(String command) throws Exception {
            synchronized (replyLock) {
                int before = replies;
                send(new ServerboundChatCommandPacket(command));
                long until = System.nanoTime() + 30_000_000_000L;
                while (replies == before) {
                    long left = (until - System.nanoTime()) / 1_000_000L;
                    if (left <= 0) {
                        record("error no reply to /" + command);
                        return;
                    }
                    replyLock.wait(left);
                }
            }
        }

        /** A round trip through the server thread, its reply left out of the transcript: every packet this player
         * was sent before the reply has been recorded when it returns. */
        void sync() throws Exception {
            synchronized (replyLock) {
                int before = syncs;
                send(new ServerboundChatCommandPacket("time query gametime"));
                long until = System.nanoTime() + 30_000_000_000L;
                while (syncs == before) {
                    long left = (until - System.nanoTime()) / 1_000_000L;
                    if (left <= 0) {
                        record("error no reply to sync");
                        return;
                    }
                    replyLock.wait(left);
                }
            }
        }

        void mark() {
            synchronized (replyLock) {
                marked = pushes;
            }
        }

        /** Waits for a motion packet about this player since the last `mark` (a mob's hit, which comes when its AI
         * strikes). */
        void await() throws Exception {
            synchronized (replyLock) {
                long until = System.nanoTime() + 20_000_000_000L;
                while (pushes == marked) {
                    long left = (until - System.nanoTime()) / 1_000_000L;
                    if (left <= 0) {
                        record("error no motion packet since the mark");
                        return;
                    }
                    replyLock.wait(left);
                }
            }
        }

        void record(String line) {
            synchronized (lines) {
                lines.add("step=" + step + " " + line);
            }
        }

        void at(Vec3 to, float yaw, float pitch, boolean ground) throws Exception {
            position = new PositionMoveRotation(to, Vec3.ZERO, yaw, pitch);
            // A tick of its own: the server takes one position per client tick (receivedPositionThisTick).
            send(net.minecraft.network.protocol.game.ServerboundClientTickEndPacket.INSTANCE);
            send(new ServerboundMovePlayerPacket.PosRot(to, yaw, pitch, ground, false));
            send(net.minecraft.network.protocol.game.ServerboundClientTickEndPacket.INSTANCE);
        }

        /** One write: the item use belongs to the old position, before the next client's tick moves. */
        void useThenMove(Vec3 to, float yaw, float pitch, boolean ground) throws Exception {
            position = new PositionMoveRotation(to, Vec3.ZERO, yaw, pitch);
            send(new ServerboundUseItemPacket(InteractionHand.MAIN_HAND, 0, yaw, pitch),
                net.minecraft.network.protocol.game.ServerboundClientTickEndPacket.INSTANCE,
                new ServerboundMovePlayerPacket.PosRot(to, yaw, pitch, ground, false),
                net.minecraft.network.protocol.game.ServerboundClientTickEndPacket.INSTANCE);
        }

        void sprint(boolean on) throws Exception {
            playerCommand(on ? ServerboundPlayerCommandPacket.Action.START_SPRINTING : ServerboundPlayerCommandPacket.Action.STOP_SPRINTING);
        }

        void playerCommand(ServerboundPlayerCommandPacket.Action action) throws Exception {
            // ServerboundPlayerCommandPacket's only public constructors take an Entity: read it from its bytes.
            FriendlyByteBuf b = new FriendlyByteBuf(Unpooled.buffer());
            b.writeVarInt(id);
            b.writeVarInt(action.ordinal());
            b.writeVarInt(0);
            send(ServerboundPlayerCommandPacket.STREAM_CODEC.decode(b));
        }

        void attack(String target) throws Exception {
            Integer entity = last.get(target);
            if (entity == null) {
                record("error no " + target + " to attack");
                return;
            }
            send(new ServerboundAttackPacket(entity));
            send(ServerboundPunchPacket.INSTANCE);
        }

        void close() throws Exception {
            if (socket != null) socket.close();
        }

        void join(String host, int port) throws Exception {
            socket = new Socket();
            socket.connect(new InetSocketAddress(host, port), 10_000);
            socket.setTcpNoDelay(true);
            in = new DataInputStream(socket.getInputStream());
            out = socket.getOutputStream();
            outbound = HandshakeProtocols.SERVERBOUND;
            send(new ClientIntentionPacket(SharedConstants.getProtocolVersion(), host, port, ClientIntent.LOGIN));
            outbound = LoginProtocols.SERVERBOUND;
            inbound = LoginProtocols.CLIENTBOUND;
            send(new ServerboundHelloPacket(name, UUIDUtil.createOfflinePlayerUUID(name)));
            while (true) {
                Packet<?> p = read().packet;
                if (p instanceof ClientboundLoginCompressionPacket c) {
                    threshold = c.getCompressionThreshold();
                } else if (p instanceof ClientboundLoginFinishedPacket) {
                    send(ServerboundLoginAcknowledgedPacket.INSTANCE);
                    break;
                } else if (p instanceof ClientboundLoginDisconnectPacket d) {
                    throw new IllegalStateException("login refused: " + d.reason().getString());
                }
            }
            outbound = ConfigurationProtocols.SERVERBOUND;
            inbound = ConfigurationProtocols.CLIENTBOUND;
            send(new ServerboundCustomPayloadPacket(new BrandPayload("vanilla")));
            send(new ServerboundClientInformationPacket(ClientInformation.createDefault()));
            while (true) {
                Packet<?> p = read().packet;
                if (p instanceof ClientboundSelectKnownPacks k) {
                    send(new ServerboundSelectKnownPacks(k.knownPacks()));
                } else if (p instanceof ClientboundKeepAlivePacket k) {
                    send(new ServerboundKeepAlivePacket(k.getId()));
                } else if (p instanceof ClientboundPingPacket k) {
                    send(new ServerboundPongPacket(k.getId()));
                } else if (p instanceof ClientboundFinishConfigurationPacket) {
                    send(ServerboundFinishConfigurationPacket.INSTANCE);
                    break;
                } else if (p instanceof ClientboundDisconnectPacket d) {
                    throw new IllegalStateException("disconnected: " + d.reason().getString());
                }
            }
            GameProtocols.Context context = new GameProtocols.Context() {
                @Override
                public boolean hasInfiniteMaterials() {
                    return true;
                }

                @Override
                public boolean canUseCommandBlocks() {
                    return true;
                }
            };
            outbound = GameProtocols.SERVERBOUND_TEMPLATE.bind(RegistryFriendlyByteBuf.decorator(ParityData.access), context);
            inbound = GameProtocols.CLIENTBOUND_TEMPLATE.bind(RegistryFriendlyByteBuf.decorator(ParityData.access));
            Thread reader = new Thread(this::play, name);
            reader.setDaemon(true);
            reader.start();
        }

        private void play() {
            try {
                while (true) {
                    Received r = read();
                    Packet<?> p = r.packet;
                    if (p instanceof net.minecraft.network.protocol.game.ClientboundLoginPacket l) {
                        id = l.playerId();
                        seen.put(id, "self");
                    } else if (p instanceof net.minecraft.network.protocol.game.ClientboundAddEntityPacket a) {
                        String who = a.getType() == net.minecraft.world.entity.EntityTypes.PLAYER
                            ? NAMES.getOrDefault(a.getUUID(), "uuid?")
                            : net.minecraft.core.registries.BuiltInRegistries.ENTITY_TYPE.getKey(a.getType()).getPath();
                        if (who.equals("wind_charge")) {
                            // Inaccuracy is random. These buckets distinguish the old source from
                            // the later position eight blocks away (and its eight-block velocity).
                            record("wind source x=" + (int) Math.floor(a.getX()) + " vx=" + Math.round(a.getMovement().x));
                        }
                        seen.put(a.getId(), who);
                        last.put(who, a.getId());
                    } else if (p instanceof ClientboundSetEntityMotionPacket m) {
                        Vec3 v = m.movement();
                        record(String.format(java.util.Locale.ROOT, "motion %s %.5f %.5f %.5f",
                            seen.getOrDefault(m.id(), "id" + m.id()), v.x, v.y, v.z));
                        if (m.id() == id) {
                            synchronized (replyLock) {
                                pushes++;
                                replyLock.notifyAll();
                            }
                        }
                    } else if (p instanceof ClientboundExplodePacket e) {
                        record(e.playerKnockback().map(v -> String.format(java.util.Locale.ROOT, "explode push %.5f %.5f %.5f",
                            v.x, v.y, v.z)).orElse("explode push none"));
                    } else if (p instanceof net.minecraft.network.protocol.game.ClientboundSetEntityDataPacket d && d.id() == id) {
                        for (var value : d.packedItems()) {
                            if (value.id() == 0) {
                                record("sprinting " + ((((Byte) value.value()) & 8) != 0));
                                record("gliding " + ((((Byte) value.value()) & 128) != 0));
                            }
                        }
                    } else if (p instanceof ClientboundSystemChatPacket c
                        && c.content().getContents() instanceof net.minecraft.network.chat.contents.TranslatableContents t
                        && t.getKey().equals("commands.time.query.gametime")) {
                        synchronized (replyLock) {
                            syncs++;
                            replyLock.notifyAll();
                        }
                    } else if (p instanceof ClientboundSystemChatPacket c) {
                        record("chat " + c.content().getString());
                        synchronized (replyLock) {
                            replies++;
                            replyLock.notifyAll();
                        }
                    } else if (p instanceof ClientboundKeepAlivePacket k) {
                        send(new ServerboundKeepAlivePacket(k.getId()));
                    } else if (p instanceof ClientboundPingPacket k) {
                        send(new ServerboundPongPacket(k.getId()));
                    } else if (p instanceof ClientboundPlayerPositionPacket t) {
                        position = PositionMoveRotation.calculateAbsolute(position, t.change(), t.relatives());
                        Vec3 at = position.position();
                        if (loaded) {
                            record(String.format(java.util.Locale.ROOT, "teleport %.3f %.3f %.3f", at.x, at.y, at.z));
                        }
                        send(new ServerboundAcceptTeleportationPacket(t.id(), at.x, at.y, at.z, position.yRot(), position.xRot()));
                        if (!loaded) {
                            send(new ServerboundMovePlayerPacket.PosRot(at, position.yRot(), position.xRot(), false, false));
                            send(new ServerboundPlayerLoadedPacket());
                            loaded = true;
                        }
                    } else if (p instanceof ClientboundChunkBatchFinishedPacket) {
                        send(new ServerboundChunkBatchReceivedPacket(64.0F));
                    } else if (p instanceof ClientboundDisconnectPacket d) {
                        record("disconnected " + d.reason().getString());
                        return;
                    }
                }
            } catch (Exception e) {
                if (!socket.isClosed()) record("error play " + e);
            }
        }

        private record Received(Packet<?> packet, byte[] payload) {}

        private Received read() throws Exception {
            byte[] payload = readFrame();
            ByteBuf buf = Unpooled.wrappedBuffer(payload);
            Packet<?> packet = inbound.codec().decode(buf);
            if (buf.readableBytes() > 0) {
                throw new IllegalStateException(packet.type().id() + " was larger than expected: " + buf.readableBytes() + " bytes left");
            }
            return new Received(packet, payload);
        }

        synchronized void send(Packet<?>... packets) throws Exception {
            ByteArrayOutputStream batch = new ByteArrayOutputStream();
            for (Packet<?> packet : packets) batch.write(frame(packet));
            out.write(batch.toByteArray());
            out.flush();
        }

        private byte[] frame(Packet<?> packet) throws Exception {
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
                    while (!d.finished()) inner.writeBytes(tmp, 0, d.deflate(tmp));
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
            return bytes;
        }

        private byte[] readFrame() throws Exception {
            byte[] frame = new byte[readVarInt(in)];
            in.readFully(frame);
            if (threshold < 0) return frame;
            ByteArrayInputStream header = new ByteArrayInputStream(frame);
            int dataLength = readVarInt(header);
            int skip = frame.length - header.available();
            if (dataLength == 0) return java.util.Arrays.copyOfRange(frame, skip, frame.length);
            Inflater inflater = new Inflater();
            inflater.setInput(frame, skip, frame.length - skip);
            byte[] raw = new byte[dataLength];
            int n = inflater.inflate(raw);
            inflater.end();
            if (n != dataLength) throw new IllegalStateException("decompressed " + n + " of " + dataLength + " bytes");
            return raw;
        }
    }

    private static int readVarInt(InputStream in) throws Exception {
        int value = 0;
        for (int shift = 0; shift < 35; shift += 7) {
            int b = in.read();
            if (b < 0) throw new java.io.EOFException();
            value |= (b & 0x7F) << shift;
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
}
