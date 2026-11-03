package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.blaze3d.font.GlyphInfo;
import com.mojang.blaze3d.vertex.VertexConsumer;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.GlyphSource;
import net.minecraft.client.gui.font.TextRenderable;
import net.minecraft.client.gui.font.glyphs.BakedGlyph;
import net.minecraft.client.gui.font.glyphs.EffectGlyph;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import net.minecraft.network.chat.*;
import net.minecraft.util.FormattedCharSequence;
import net.minecraft.util.RandomSource;
import org.joml.Matrix4f;

/** Real font colour selection and atlas glyph rendering, with GPU handles left unused. */
public final class VanillaInlineObjectOracle {
    public static void main(String[] args) throws Exception {
        ParityData.init();
        var field = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        field.setAccessible(true);
        Object unsafe = field.get(null);
        var allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        var sprite = (TextureAtlasSprite) allocate.invoke(unsafe, TextureAtlasSprite.class);
        for (var entry : Map.of("u0", .125f, "v0", .25f, "u1", .375f, "v1", .5f).entrySet()) {
            var f = TextureAtlasSprite.class.getDeclaredField(entry.getKey());
            f.setAccessible(true);
            f.setFloat(sprite, entry.getValue());
        }
        var constructor = Class.forName("net.minecraft.client.gui.font.AtlasGlyphProvider$Instance")
            .getDeclaredConstructors()[0];
        constructor.setAccessible(true);
        BakedGlyph glyph = new BakedGlyph() {
            public GlyphInfo info() { return GlyphInfo.simple(8); }
            public TextRenderable.Styled createGlyph(float x, float y, int color, int shadow,
                                                     Style style, float bold, float offset) {
                try {
                    return (TextRenderable.Styled) constructor.newInstance(
                        null, null, sprite, x, y, color, shadow, offset, style);
                } catch (ReflectiveOperationException e) { throw new RuntimeException(e); }
            }
        };
        GlyphSource source = new GlyphSource() {
            public BakedGlyph getGlyph(int c) { return glyph; }
            public BakedGlyph getRandomGlyph(RandomSource random, int width) { return glyph; }
        };
        Font font = new Font(new Font.Provider() {
            public GlyphSource glyphs(FontDescription description) { return source; }
            public EffectGlyph effect() { throw new UnsupportedOperationException(); }
        });
        var output = new StringBuilder();
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            var row = JsonParser.parseString(line).getAsJsonObject();
            Style style = Style.EMPTY.withBold(true).withItalic(true);
            if (row.has("color")) style = style.withColor(TextColor.parseColor(row.get("color").getAsString()).getOrThrow());
            if (row.has("shadow_color")) style = style.withShadowColor(row.get("shadow_color").getAsInt());
            var vertices = new ArrayList<float[]>();
            var colors = new ArrayList<Integer>();
            VertexConsumer recorder = (VertexConsumer) Proxy.newProxyInstance(
                VertexConsumer.class.getClassLoader(), new Class<?>[] {VertexConsumer.class},
                (proxy, method, values) -> {
                    if (method.isDefault()) return InvocationHandler.invokeDefault(proxy, method, values);
                    if (method.getName().equals("addVertex"))
                        vertices.add(new float[] {(float) values[0], (float) values[1], (float) values[2]});
                    if (method.getName().equals("setColor")) colors.add((int) values[0]);
                    return proxy;
                });
            boolean world = row.get("world").getAsBoolean();
            var prepared = font.prepareText(FormattedCharSequence.forward("\ufffc", style),
                row.get("x").getAsFloat(), row.get("y").getAsFloat(),
                (int) row.get("base").getAsLong(), row.get("shadow").getAsBoolean(), false, 0);
            prepared.visit(new Font.GlyphVisitor() {
                public void acceptGlyph(TextRenderable.Styled renderable) {
                    renderable.render(new Matrix4f(), recorder, 0, !world);
                }
            });
            output.append(vertices.size() / 4).append('\n');
            for (int i = 0; i < vertices.size(); i += 4) {
                var a = vertices.get(i);
                var b = vertices.get(i + 2);
                output.append(String.format(Locale.ROOT, "%.3f %.3f %.3f %.3f %.3f %08x%n",
                    a[0], a[1], b[0], b[1], a[2], colors.get(i)));
            }
        }
        Files.writeString(Path.of(args[1]), output);
    }
}
