package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import com.mojang.serialization.JsonOps;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.InputType;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.ActiveTextCollector;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.Gui;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.TextAlignment;
import net.minecraft.client.gui.components.AbstractWidget;
import net.minecraft.client.gui.components.EditBox;
import net.minecraft.client.gui.components.WidgetTooltipHolder;
import net.minecraft.client.gui.screens.ConfirmScreen;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.inventory.tooltip.ClientTooltipPositioner;
import net.minecraft.client.gui.screens.packs.PackSelectionScreen;
import net.minecraft.client.input.MouseButtonEvent;
import net.minecraft.client.input.MouseButtonInfo;
import net.minecraft.client.sounds.SoundManager;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.ComponentSerialization;
import net.minecraft.resources.Identifier;
import net.minecraft.server.packs.PackLocationInfo;
import net.minecraft.server.packs.PackSelectionConfig;
import net.minecraft.server.packs.repository.Pack;
import net.minecraft.server.packs.repository.PackCompatibility;
import net.minecraft.server.packs.repository.PackRepository;
import net.minecraft.server.packs.repository.PackSource;
import net.minecraft.util.FormattedCharSequence;
import net.minecraft.util.Util;
import net.minecraft.world.flag.FeatureFlagSet;
import net.minecraft.world.inventory.tooltip.TooltipComponent;
import org.joml.Matrix3x2f;
import org.joml.Matrix3x2fStack;

/**
 * Original oracle for the resource pack screen's drawing: a real
 * PackSelectionScreen over a PackRepository of the case's packs (titles,
 * descriptions, sources, compatibility, required, positions, icons), driven
 * by the case's clicks, typing in the search box and mouse wheel, with the
 * ConfirmScreen an incompatible pack opens (a stand-in Gui switches screens
 * as Gui.setScreen does: the old screen loses focus, the new one is
 * initialised or repositioned). The screen shown at the end is drawn
 * (Screen.extractRenderState: its widgets, then the tooltip) and every
 * GuiGraphicsExtractor call is recorded.
 *
 * Input lines: case JSON. Output lines: {screen, draws} or {error}.
 */
public final class VanillaPackScreenOracle {
    static Object unsafe;
    static Method allocate;
    static Font font;

    @SuppressWarnings("unchecked")
    static <T> T alloc(Class<T> type) throws Exception {
        return (T) allocate.invoke(unsafe, type);
    }

    static Field field(Class<?> owner, String name) throws Exception {
        Field f = owner.getDeclaredField(name);
        f.setAccessible(true);
        return f;
    }

    static void put(Object target, Class<?> owner, String name, Object value) throws Exception {
        Field f = owner.getDeclaredField(name);
        long offset = (long) unsafe.getClass().getMethod("objectFieldOffset", Field.class).invoke(unsafe, f);
        Class<?> t = f.getType();
        String setter = t == int.class ? "putInt" : t == boolean.class ? "putBoolean" : t == float.class ? "putFloat" : t == long.class ? "putLong" : "putObject";
        Class<?> param = t.isPrimitive() ? t : Object.class;
        unsafe.getClass().getMethod(setter, Object.class, long.class, param).invoke(unsafe, target, offset, value);
    }

    static Object call(Object target, Class<?> owner, String name) throws Exception {
        Method m = owner.getDeclaredMethod(name);
        m.setAccessible(true);
        return m.invoke(target);
    }

    /** Styled runs: [text, rgb or null, bold, italic, underlined, strikethrough, obfuscated]. */
    static JsonArray runs(FormattedCharSequence s) {
        var runs = new JsonArray();
        s.accept((index, style, cp) -> {
            Integer rgb = style.getColor() == null ? null : style.getColor().getValue();
            boolean[] flags = {style.isBold(), style.isItalic(), style.isUnderlined(), style.isStrikethrough(), style.isObfuscated()};
            JsonArray last = runs.isEmpty() ? null : runs.get(runs.size() - 1).getAsJsonArray();
            boolean same = last != null && Objects.equals(last.get(1).isJsonNull() ? null : last.get(1).getAsInt(), rgb);
            for (int i = 0; same && i < 5; i++) same = last.get(2 + i).getAsBoolean() == flags[i];
            if (same) {
                last.set(0, new JsonPrimitive(last.get(0).getAsString() + new String(Character.toChars(cp))));
            } else {
                var run = new JsonArray();
                run.add(new String(Character.toChars(cp)));
                if (rgb == null) run.add(JsonNull.INSTANCE); else run.add(rgb);
                for (boolean f : flags) run.add(f);
                runs.add(run);
            }
            return true;
        });
        return runs;
    }

    /** Records every GuiGraphicsExtractor call. */
    public static final class Capture extends GuiGraphicsExtractor {
        JsonArray draws;
        Matrix3x2fStack stack;
        int width, height;
        private Capture() { super(null, null, 0, 0); }
        @Override public int guiWidth() { return width; }
        @Override public int guiHeight() { return height; }
        @Override public Matrix3x2fStack pose() { return stack; }
        @Override public void nextStratum() { add("stratum"); }
        @Override public void requestCursor(com.mojang.blaze3d.platform.cursor.CursorType type) { }
        @Override public boolean containsPointInScissor(int x, int y) { return true; }
        JsonObject add(String type) {
            var o = new JsonObject();
            o.addProperty("type", type);
            if (!stack.equals(new Matrix3x2f())) {
                var a = new JsonArray();
                for (float v : new float[] {stack.m00(), stack.m01(), stack.m10(), stack.m11(), stack.m20(), stack.m21()}) a.add(v);
                o.add("matrix", a);
            }
            draws.add(o);
            return o;
        }
        @Override public void enableScissor(int x0, int y0, int x1, int y1) {
            var o = add("scissor");
            o.addProperty("x0", x0); o.addProperty("y0", y0); o.addProperty("x1", x1); o.addProperty("y1", y1);
        }
        @Override public void disableScissor() { add("noscissor"); }
        @Override public void fill(RenderPipeline p, int x0, int y0, int x1, int y1, int color) {
            var o = add("fill");
            o.addProperty("x0", Math.min(x0, x1)); o.addProperty("y0", Math.min(y0, y1));
            o.addProperty("x1", Math.max(x0, x1)); o.addProperty("y1", Math.max(y0, y1));
            o.addProperty("color", color);
        }
        @Override public void textHighlight(int x0, int y0, int x1, int y1, boolean invert) {
            var o = add("highlight");
            o.addProperty("x0", x0); o.addProperty("y0", y0); o.addProperty("x1", x1); o.addProperty("y1", y1);
        }
        @Override public void blitSprite(RenderPipeline p, Identifier id, int x, int y, int w, int h, int color) {
            var o = add("sprite");
            o.addProperty("sprite", id.getPath());
            o.addProperty("x", x); o.addProperty("y", y); o.addProperty("w", w); o.addProperty("h", h);
            o.addProperty("color", color);
        }
        @Override public void blit(RenderPipeline p, Identifier texture, int x, int y, float u, float v, int w, int h, int srcW, int srcH, int texW, int texH, int color) {
            var o = add("blit");
            o.addProperty("texture", texture.getPath());
            o.addProperty("x", x); o.addProperty("y", y); o.addProperty("w", w); o.addProperty("h", h);
            o.addProperty("u", u); o.addProperty("v", v);
            o.addProperty("srcW", srcW); o.addProperty("srcH", srcH); o.addProperty("texW", texW); o.addProperty("texH", texH);
            o.addProperty("color", color);
        }
        void text(FormattedCharSequence str, int x, int y, int color, boolean shadow) {
            var o = add("text");
            o.add("runs", runs(str));
            o.addProperty("x", x); o.addProperty("y", y); o.addProperty("color", color); o.addProperty("shadow", shadow);
        }
        @Override public void text(Font font, FormattedCharSequence str, int x, int y, int color, boolean shadow) {
            if (net.minecraft.util.ARGB.alpha(color) != 0) text(str, x, y, color, shadow);
        }
        ActiveTextCollector collector(float opacity) {
            var self = this;
            return new ActiveTextCollector() {
                Parameters parameters = new Parameters(new Matrix3x2f(stack), opacity, null);
                @Override public Parameters defaultParameters() { return parameters; }
                @Override public void defaultParameters(Parameters p) { parameters = p; }
                @Override public void accept(TextAlignment alignment, int anchorX, int y, Parameters p, FormattedCharSequence text) {
                    int left = alignment.calculateLeft(anchorX, font, text);
                    if (net.minecraft.util.ARGB.as8BitChannel(p.opacity()) != 0) self.text(text, left, y, net.minecraft.util.ARGB.white(p.opacity()), true);
                }
                @Override public void acceptScrolling(Component m, int cx, int l, int r, int t, int b, Parameters p) {
                    defaultScrollingHelper(m, cx, l, r, t, b, font.width(m), 9, p);
                }
            };
        }
        @Override public ActiveTextCollector textRendererForWidget(AbstractWidget owner, GuiGraphicsExtractor.HoveredTextEffects effects) {
            return collector(owner.getAlpha());
        }
        @Override public ActiveTextCollector textRenderer(GuiGraphicsExtractor.HoveredTextEffects effects, java.util.function.Consumer<net.minecraft.network.chat.Style> consumer) {
            return collector(1.0F);
        }
        @Override public void setTooltipForNextFrame(Font font, List<FormattedCharSequence> lines, Optional<TooltipComponent> image,
                                                     ClientTooltipPositioner positioner, int x, int y, boolean replace, Identifier style) {
            var o = add("tooltip");
            var a = new JsonArray();
            for (var l : lines) a.add(runs(l));
            o.add("lines", a);
            o.addProperty("x", x); o.addProperty("y", y);
        }
        @Override public void setTooltipForNextFrame(Font font, List<? extends FormattedCharSequence> lines, int x, int y, Identifier style) {
            var o = add("tooltip");
            var a = new JsonArray();
            for (var l : lines) a.add(runs(l));
            o.add("lines", a);
            o.addProperty("x", x); o.addProperty("y", y);
        }
    }

    /** The window's text input area: not tracked. */
    static final class NoTextInput extends com.mojang.blaze3d.platform.TextInputManager {
        NoTextInput() { super(null); }
        @Override public void setTextInputArea(int x0, int y0, int x1, int y1) { }
        @Override public void onTextInputFocusChange(Object owner, boolean focused) { }
        @Override public void startTextInput(Object owner) { }
        @Override public void stopTextInput(Object owner) { }
        @Override public void stopTextInput() { }
    }

    /** Sounds are not played. */
    public static final class NoSound extends SoundManager {
        private NoSound() { super(null); }
        @Override public net.minecraft.client.sounds.SoundEngine.PlayResult play(net.minecraft.client.resources.sounds.SoundInstance instance) { return null; }
    }

    /** No input method editor. */
    public static final class NoKeyboard extends net.minecraft.client.KeyboardHandler {
        private NoKeyboard() { super(null); }
        @Override public void resubmitLastPreeditEvent(net.minecraft.client.gui.components.events.GuiEventListener e) { }
    }

    /** Gui.setScreen without the window: the screen shown, focus cleared on the old one. */
    public static final class StubGui extends Gui {
        Screen current;
        int width, height;
        private StubGui() { super(null, null, null); }
        @Override public void setScreen(Screen screen) {
            try {
                if (current != null) {
                    current.clearFocus();
                    current.removed();
                }
                current = screen;
                if (screen == null) return;
                screen.added();
                show(screen, width, height);
            } catch (Exception e) {
                throw new RuntimeException(e);
            }
        }
        @Override public Screen screen() { return current; }
    }

    /** Screen.init(width, height) without narration. */
    static void show(Screen screen, int width, int height) throws Exception {
        screen.width = width;
        screen.height = height;
        if (!(boolean) field(Screen.class, "initialized").get(screen)) {
            call(screen, Screen.class, "init");
            Method m = Screen.class.getDeclaredMethod("setInitialFocus");
            m.setAccessible(true);
            m.invoke(screen);
        } else {
            Method m = screen.getClass().getDeclaredMethod("repositionElements");
            m.setAccessible(true);
            m.invoke(screen);
        }
        put(screen, Screen.class, "initialized", true);
    }

    static Component component(JsonElement e) {
        return ComponentSerialization.CODEC.parse(JsonOps.INSTANCE, e).getOrThrow();
    }

    static Pack pack(JsonObject o) {
        String id = o.get("id").getAsString();
        PackSource source = switch (o.get("source").getAsString()) {
            case "builtin" -> PackSource.BUILT_IN;
            case "world" -> PackSource.WORLD;
            case "server" -> PackSource.SERVER;
            case "feature" -> PackSource.FEATURE;
            default -> PackSource.DEFAULT;
        };
        var location = new PackLocationInfo(id, component(o.get("title")), source, Optional.empty());
        var compatibility = switch (o.get("compat").getAsString()) {
            case "old" -> PackCompatibility.TOO_OLD;
            case "new" -> PackCompatibility.TOO_NEW;
            case "unknown" -> PackCompatibility.UNKNOWN;
            default -> PackCompatibility.COMPATIBLE;
        };
        var metadata = new Pack.Metadata(component(o.get("description")), compatibility, FeatureFlagSet.of(), List.of());
        var config = new PackSelectionConfig(o.get("required").getAsBoolean(),
            o.get("position").getAsString().equals("top") ? Pack.Position.TOP : Pack.Position.BOTTOM, o.get("fixed").getAsBoolean());
        return new Pack(location, null, metadata, config);
    }

    static JsonObject run(JsonObject c, Minecraft mc, StubGui gui) throws Exception {
        int width = c.get("width").getAsInt(), height = c.get("height").getAsInt();
        gui.width = width;
        gui.height = height;
        gui.current = null;
        put(mc, Minecraft.class, "level", c.get("inWorld").getAsBoolean() ? alloc(net.minecraft.client.multiplayer.ClientLevel.class) : null);
        List<Pack> packs = new ArrayList<>();
        Map<String, Boolean> icons = new HashMap<>();
        for (var p : c.getAsJsonArray("packs")) {
            packs.add(pack(p.getAsJsonObject()));
            icons.put(p.getAsJsonObject().get("id").getAsString(), p.getAsJsonObject().get("icon").getAsBoolean());
        }
        var repository = new PackRepository(consumer -> packs.forEach(consumer));
        repository.reload();
        List<String> initial = new ArrayList<>();
        for (var s : c.getAsJsonArray("selected")) initial.add(s.getAsString());
        repository.setSelected(initial);
        var screen = new PackSelectionScreen(repository, r -> { }, Path.of(c.get("dir").getAsString()), Component.translatable(c.get("title").getAsString()));
        gui.setScreen(screen);
        // PackSelectionScreen.loadPackIcon: a pack with a pack.png gets its own texture.
        @SuppressWarnings("unchecked")
        var iconMap = (Map<String, Identifier>) field(PackSelectionScreen.class, "packIcons").get(screen);
        iconMap.clear();
        for (var e : icons.entrySet()) {
            String id = e.getKey();
            iconMap.put(id, e.getValue()
                ? Identifier.withDefaultNamespace("pack/" + Util.sanitizeName(id, Identifier::validPathChar) + "/" + com.google.common.hash.Hashing.sha1().hashUnencodedChars(id) + "/icon")
                : Identifier.withDefaultNamespace("textures/misc/unknown_pack.png"));
        }
        var search = (EditBox) field(PackSelectionScreen.class, "search").get(screen);
        var info = new MouseButtonInfo(1, 0);
        for (var a : c.getAsJsonArray("actions")) {
            var o = a.getAsJsonObject();
            Screen s = gui.current;
            switch (o.get("type").getAsString()) {
                case "click" -> {
                    var event = new MouseButtonEvent(o.get("x").getAsDouble(), o.get("y").getAsDouble(), info);
                    // Never press Open Pack Folder: it opens a file manager.
                    if (s.getChildAt(event.x(), event.y()).orElse(null) instanceof net.minecraft.client.gui.components.Button b
                        && b.getMessage().getString().equals(Component.translatable("pack.openFolder").getString())) {
                        throw new IllegalArgumentException("click on Open Pack Folder");
                    }
                    s.mouseClicked(event, false);
                    if (gui.current == s) s.mouseReleased(event);
                }
                case "scroll" -> s.mouseScrolled(o.get("x").getAsDouble(), o.get("y").getAsDouble(), 0.0, o.get("amount").getAsDouble());
                case "type" -> { if (s == screen && screen.getFocused() == search) search.insertText(o.get("text").getAsString()); }
                case "erase" -> { if (s == screen && screen.getFocused() == search) search.deleteChars(-o.get("count").getAsInt()); }
                default -> throw new IllegalArgumentException(o.toString());
            }
        }
        Screen shown = gui.current;
        for (var w : shown.children()) {
            if (w instanceof AbstractWidget widget) {
                var holder = (WidgetTooltipHolder) field(AbstractWidget.class, "tooltip").get(widget);
                put(holder, WidgetTooltipHolder.class, "wasDisplayed", widget.isMouseOver(c.getAsJsonArray("mouse").get(0).getAsInt(), c.getAsJsonArray("mouse").get(1).getAsInt()));
                put(holder, WidgetTooltipHolder.class, "displayStartTime", Util.getMillis() - 1000L);
            }
        }
        if (shown == screen) put(search, EditBox.class, "focusedTime", Util.getMillis() - c.get("blink").getAsLong());
        var g = alloc(Capture.class);
        g.draws = new JsonArray();
        g.stack = new Matrix3x2fStack(16);
        g.width = width;
        g.height = height;
        shown.extractRenderState(g, c.getAsJsonArray("mouse").get(0).getAsInt(), c.getAsJsonArray("mouse").get(1).getAsInt(), 1.0F);
        var result = new JsonObject();
        result.addProperty("screen", shown == screen ? "packs" : shown instanceof ConfirmScreen ? "confirm" : shown.getClass().getSimpleName());
        result.add("draws", g.draws);
        return result;
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        unsafe = unsafeField.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        font = ParityFont.createDefault(ParityFont.resources(Path.of(args[2])));
        Minecraft mc = alloc(Minecraft.class);
        put(mc, Minecraft.class, "font", font);
        put(mc, Minecraft.class, "lastInputType", InputType.MOUSE);
        put(mc, Minecraft.class, "textInputManager", new NoTextInput());
        put(mc, Minecraft.class, "soundManager", alloc(NoSound.class));
        put(mc, Minecraft.class, "keyboardHandler", alloc(NoKeyboard.class));
        var gui = alloc(StubGui.class);
        put(mc, Minecraft.class, "gui", gui);
        field(Minecraft.class, "instance").set(null, mc);
        var gson = new Gson();
        var out = new StringBuilder();
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            if (line.isBlank()) continue;
            var c = JsonParser.parseString(line).getAsJsonObject();
            JsonObject result;
            try {
                result = run(c, mc, gui);
            } catch (Throwable error) {
                Throwable cause = error;
                while (cause.getCause() != null) cause = cause.getCause();
                var sw = new java.io.StringWriter();
                cause.printStackTrace(new java.io.PrintWriter(sw));
                result = new JsonObject();
                result.addProperty("error", sw.toString().lines().limit(10).reduce("", (a, b) -> a + b + " | "));
            }
            out.append(gson.toJson(result)).append('\n');
        }
        Files.writeString(Path.of(args[1]), out.toString(), StandardCharsets.UTF_8);
    }
}
