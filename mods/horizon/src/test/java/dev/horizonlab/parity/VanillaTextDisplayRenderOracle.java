package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.blaze3d.font.GlyphInfo;
import com.mojang.blaze3d.font.GlyphProvider;
import com.mojang.blaze3d.font.UnbakedGlyph;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import com.mojang.serialization.JsonOps;
import io.netty.buffer.Unpooled;
import java.io.InputStreamReader;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.GlyphSource;
import net.minecraft.client.gui.font.TextRenderable;
import net.minecraft.client.gui.font.glyphs.BakedGlyph;
import net.minecraft.client.gui.font.glyphs.BakedSheetGlyph;
import net.minecraft.client.gui.font.providers.GlyphProviderDefinition;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.entity.DisplayRenderer;
import net.minecraft.client.renderer.entity.state.TextDisplayEntityRenderState;
import net.minecraft.client.renderer.state.GameRenderState;
import net.minecraft.nbt.TagParser;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.chat.*;
import net.minecraft.network.protocol.game.ClientboundSetEntityDataPacket;
import net.minecraft.network.syncher.EntityDataAccessor;
import net.minecraft.resources.Identifier;
import net.minecraft.server.packs.resources.ResourceManager;
import net.minecraft.util.FormattedCharSequence;
import net.minecraft.util.RandomSource;
import net.minecraft.world.entity.Display;
import org.joml.Matrix4f;

/**
 * A text display's text as vanilla draws it: the real TextDisplayRenderer.submitInner
 * submits to a recording collector, then each submit becomes vertices the way
 * TextFeatureRenderer.buildGroup makes them (Font.prepareText / prepareBackground
 * visited, TextRenderable.render with flat false), from glyphs baked by the real
 * providers through a stitcher without a GPU. Out: the submit pose and every quad
 * in text units [x, y, z, ARGB], so the depths of background, shadow, glyph, bold
 * copy and lines are compared too.
 */
public final class VanillaTextDisplayRenderOracle {
    static final class Level extends net.minecraft.server.level.ServerLevel {
        Level() { super(null, null, null, null, null, null, false, 0L, List.of(), false); }
        @Override public int getNextEntityId() { return 7; }
        @Override public net.minecraft.core.RegistryAccess registryAccess() { return ParityData.access; }
    }

    static Object unsafe;
    static Method allocate;

    @SuppressWarnings("unchecked")
    static <T> T allocate(Class<T> type) throws Exception {
        return (T) allocate.invoke(unsafe, type);
    }

    static void set(Object target, Class<?> owner, String name, Object value) throws Exception {
        Field field = owner.getDeclaredField(name);
        field.setAccessible(true);
        field.set(target, value);
    }

    /** FontTexture.add without the texture: the bitmap's own bounds. */
    static final UnbakedGlyph.Stitcher STITCHER = new UnbakedGlyph.Stitcher() {
        @Override
        public BakedGlyph stitch(GlyphInfo info, com.mojang.blaze3d.font.GlyphBitmap bitmap) {
            return new BakedSheetGlyph(info, null, null, 0, 0, 0, 0, bitmap.getLeft(), bitmap.getRight(), bitmap.getTop(), bitmap.getBottom());
        }

        @Override
        public BakedGlyph getMissing() {
            throw new IllegalStateException("missing glyph");
        }
    };

    /** The `space` and `include/default` providers (ParityFont.create), baked. */
    static Font font(ResourceManager resources) throws Exception {
        List<GlyphProvider> providers = new ArrayList<>();
        for (String include : new String[] {"space", "default"}) {
            var id = Identifier.withDefaultNamespace("font/include/" + include + ".json");
            try (var reader = new InputStreamReader(resources.open(id), StandardCharsets.UTF_8)) {
                for (var element : JsonParser.parseReader(reader).getAsJsonObject().getAsJsonArray("providers")) {
                    var conditional = GlyphProviderDefinition.Conditional.CODEC.parse(JsonOps.INSTANCE, element).getOrThrow();
                    providers.add(conditional.definition().unpack().left().orElseThrow().load(resources));
                }
            }
        }
        Map<Integer, BakedGlyph> baked = new HashMap<>();
        GlyphSource source = new GlyphSource() {
            @Override
            public BakedGlyph getGlyph(int codepoint) {
                return baked.computeIfAbsent(codepoint, c -> {
                    for (GlyphProvider provider : providers) {
                        UnbakedGlyph glyph = provider.getGlyph(c);
                        if (glyph != null) return glyph.bake(STITCHER);
                    }
                    throw new IllegalArgumentException("no glyph for " + c);
                });
            }

            @Override
            public BakedGlyph getRandomGlyph(RandomSource random, int width) {
                throw new UnsupportedOperationException();
            }
        };
        // SpecialGlyphs.WHITE: a 5x8 white glyph; effects only use their own corners.
        var white = new BakedSheetGlyph(GlyphInfo.simple(6.0F), null, null, 0, 0, 0, 0, 0, 5, 0, 8);
        return new Font(new Font.Provider() {
            @Override
            public GlyphSource glyphs(net.minecraft.network.chat.FontDescription description) {
                return source;
            }

            @Override
            public net.minecraft.client.gui.font.glyphs.EffectGlyph effect() {
                return white;
            }
        });
    }

    static VertexConsumer recorder(List<float[]> out) {
        InvocationHandler handler = (proxy, method, args) -> {
            if (method.isDefault()) return InvocationHandler.invokeDefault(proxy, method, args);
            switch (method.getName()) {
                case "addVertex" -> out.add(new float[] {(float) args[0], (float) args[1], (float) args[2], 0});
                case "setColor" -> out.get(out.size() - 1)[3] = Float.intBitsToFloat(args.length == 1 ? (int) args[0]
                    : ((int) args[3] << 24) | ((int) args[0] << 16) | ((int) args[1] << 8) | (int) args[2]);
                default -> { }
            }
            return proxy;
        };
        return (VertexConsumer) Proxy.newProxyInstance(VertexConsumer.class.getClassLoader(), new Class<?>[] {VertexConsumer.class}, handler);
    }

    @SuppressWarnings({"rawtypes", "unchecked"})
    public static void main(String[] args) throws Exception {
        ParityData.init();
        var unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        unsafe = unsafeField.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        ResourceManager resources = ParityFont.resources();
        Font font = font(resources);
        // Font.split's Language.getVisualOrder is the client's (FormattedBidiReorder: one
        // sequence per run of a style), not the server's default.
        net.minecraft.locale.Language.inject(
            net.minecraft.client.resources.language.ClientLanguage.loadFrom(resources, List.of("en_us"), false));
        // Minecraft.getInstance().gameRenderer.gameRenderState().optionsRenderState, for the default background.
        var options = new GameRenderState();
        var gameRenderer = allocate(GameRenderer.class);
        set(gameRenderer, GameRenderer.class, "gameRenderState", options);
        var minecraft = allocate(Minecraft.class);
        set(minecraft, Minecraft.class, "gameRenderer", gameRenderer);
        set(null, Minecraft.class, "instance", minecraft);
        var renderer = allocate(DisplayRenderer.TextDisplayRenderer.class);
        set(renderer, DisplayRenderer.TextDisplayRenderer.class, "font", font);
        Method split = DisplayRenderer.TextDisplayRenderer.class.getDeclaredMethod("splitLines", Component.class, int.class);
        split.setAccessible(true);
        Method submitInner = DisplayRenderer.TextDisplayRenderer.class.getDeclaredMethod("submitInner",
            TextDisplayEntityRenderState.class, PoseStack.class, SubmitNodeCollector.class, int.class, float.class);
        submitInner.setAccessible(true);
        Method fresh = Display.TextDisplay.class.getDeclaredMethod("createFreshTextRenderState");
        fresh.setAccessible(true);
        var level = (Level) allocate(Level.class);
        var type = net.minecraft.core.registries.BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse("minecraft:text_display"));
        String[][] keys = {{"text", "DATA_TEXT_ID"}, {"line_width", "DATA_LINE_WIDTH_ID"},
            {"background", "DATA_BACKGROUND_COLOR_ID"}, {"opacity", "DATA_TEXT_OPACITY_ID"}, {"flags", "DATA_STYLE_FLAGS_ID"}};
        var out = new ArrayList<String>();
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            JsonObject input = JsonParser.parseString(line).getAsJsonObject();
            var result = new JsonObject();
            try {
                var display = new Display.TextDisplay(type, level);
                for (String[] key : keys) {
                    if (!input.has(key[0])) continue;
                    Field field = Display.TextDisplay.class.getDeclaredField(key[1]);
                    field.setAccessible(true);
                    EntityDataAccessor accessor = (EntityDataAccessor) field.get(null);
                    JsonElement in = input.get(key[0]);
                    Object value = switch (key[0]) {
                        case "text" -> ComponentSerialization.CODEC.parse(ParityData.ops,
                            TagParser.parseCompoundFully("{v:" + in + "}").get("v")).getOrThrow();
                        case "opacity", "flags" -> in.getAsByte();
                        default -> in.getAsInt();
                    };
                    display.getEntityData().set(accessor, value);
                }
                var values = display.getEntityData().getNonDefaultValues();
                var buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), ParityData.access);
                ClientboundSetEntityDataPacket.STREAM_CODEC.encode(buf, new ClientboundSetEntityDataPacket(7, values == null ? List.of() : values));
                byte[] bytes = new byte[buf.readableBytes()];
                buf.readBytes(bytes);
                buf.release();
                result.addProperty("wire", HexFormat.of().formatHex(bytes));

                options.optionsRenderState.backgroundForChatOnly = input.get("chat_only").getAsBoolean();
                options.optionsRenderState.textBackgroundOpacity = input.get("background_opacity").getAsFloat();
                var state = new TextDisplayEntityRenderState();
                state.textRenderState = (Display.TextDisplay.TextRenderState) fresh.invoke(display);
                state.cachedInfo = (Display.TextDisplay.CachedInfo) split.invoke(renderer,
                    state.textRenderState.text(), state.textRenderState.lineWidth());
                List<Object[]> submits = new ArrayList<>();
                var collector = (SubmitNodeCollector) Proxy.newProxyInstance(SubmitNodeCollector.class.getClassLoader(),
                    new Class<?>[] {SubmitNodeCollector.class}, (proxy, method, a) -> {
                        switch (method.getName()) {
                            case "submitText", "submitTextBackground" -> submits.add(new Object[] {method.getName(),
                                new Matrix4f(((PoseStack) a[0]).last().pose()), a});
                            default -> throw new UnsupportedOperationException(method.getName());
                        }
                        return null;
                    });
                submitInner.invoke(renderer, state, new PoseStack(), collector, 0xF000F0, 1.0F);
                // TextFeatureRenderer.buildGroup, in text units: the pose is reported once.
                List<float[]> vertices = new ArrayList<>();
                VertexConsumer consumer = recorder(vertices);
                Matrix4f identity = new Matrix4f();
                Font.GlyphVisitor visitor = new Font.GlyphVisitor() {
                    @Override
                    public void acceptRenderable(TextRenderable renderable) {
                        renderable.render(identity, consumer, 0xF000F0, false);
                    }
                };
                var poses = new JsonArray();
                var modes = new JsonArray();
                for (Object[] submit : submits) {
                    Object[] a = (Object[]) submit[2];
                    float[] m = new float[16];
                    ((Matrix4f) submit[1]).get(m);
                    if (poses.isEmpty()) for (float v : m) poses.add(v);
                    if (submit[0].equals("submitText")) {
                        modes.add(a[5].toString());
                        font.prepareText((FormattedCharSequence) a[3], (float) a[1], (float) a[2], (int) a[7], (boolean) a[4], false, (int) a[8])
                            .visit(visitor);
                    } else {
                        modes.add(a[6].toString());
                        visitor.acceptRenderable(font.prepareBackground((float) a[1], (float) a[2], (float) a[3], (float) a[4], (int) a[5]));
                    }
                }
                var quads = new JsonArray();
                for (int i = 0; i + 3 < vertices.size(); i += 4) {
                    var quad = new JsonArray();
                    for (int j = 0; j < 4; j++) {
                        float[] v = vertices.get(i + j);
                        var vertex = new JsonArray();
                        vertex.add(v[0]);
                        vertex.add(v[1]);
                        vertex.add(v[2]);
                        vertex.add(Float.floatToRawIntBits(v[3]));
                        quad.add(vertex);
                    }
                    quads.add(quad);
                }
                result.add("pose", poses);
                result.add("modes", modes);
                result.add("quads", quads);
            } catch (Throwable e) {
                Throwable cause = e instanceof InvocationTargetException t ? t.getCause() : e;
                result.addProperty("error", String.valueOf(cause));
            }
            out.add(result.toString());
        }
        Files.write(Path.of(args[1]), out);
    }
}
