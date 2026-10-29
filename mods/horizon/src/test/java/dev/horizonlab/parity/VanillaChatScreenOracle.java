package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.brigadier.CommandDispatcher;
import com.mojang.brigadier.StringReader;
import com.mojang.brigadier.context.CommandContext;
import com.mojang.brigadier.suggestion.Suggestions;
import com.mojang.brigadier.suggestion.SuggestionsBuilder;
import com.mojang.brigadier.tree.RootCommandNode;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.CompletableFuture;
import net.minecraft.SharedConstants;
import net.minecraft.client.InputType;
import net.minecraft.client.Minecraft;
import net.minecraft.client.OptionInstance;
import net.minecraft.client.Options;
import net.minecraft.client.gui.Hud;
import net.minecraft.client.gui.components.CommandSuggestions;
import net.minecraft.client.gui.components.EditBox;
import net.minecraft.client.gui.components.Renderable;
import net.minecraft.client.gui.components.ChatComponent;
import net.minecraft.client.gui.screens.ChatScreen;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.input.CharacterEvent;
import net.minecraft.client.input.KeyEvent;
import net.minecraft.client.input.MouseButtonEvent;
import net.minecraft.client.input.MouseButtonInfo;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.multiplayer.ClientSuggestionProvider;
import net.minecraft.client.multiplayer.chat.ChatAbilities;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.commands.CommandBuildContext;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
import net.minecraft.commands.SharedSuggestionProvider;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.chat.Component;
import net.minecraft.network.protocol.game.ClientboundCommandsPacket;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.ReloadableServerRegistries;
import net.minecraft.server.ServerScoreboard;
import net.minecraft.server.dedicated.DedicatedServer;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.server.permissions.LevelBasedPermissionSet;
import net.minecraft.server.permissions.PermissionLevel;
import net.minecraft.server.permissions.PermissionSet;
import net.minecraft.server.players.PlayerList;
import net.minecraft.tags.TagLoader;
import net.minecraft.util.ArrayListDeque;
import net.minecraft.util.Util;
import net.minecraft.world.flag.FeatureFlagSet;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.level.Level;
import org.joml.Matrix3x2fStack;

/**
 * Original oracle for the chat screen's input: a real ChatScreen (its
 * EditBox and CommandSuggestions) on a client command tree made as the game
 * makes it (the server's tree filtered for an operator, sent through
 * ClientboundCommandsPacket), with suggestions a node asks the server for
 * answered by the server's dispatcher (as
 * ServerGamePacketListenerImpl.handleCustomCommandSuggestions does). The
 * case's sent-message history, typed characters, keys and clicks drive it;
 * then ChatScreen.extractRenderState without the chat lines (the input's
 * background, the EditBox, the suggestion list or usage) is recorded.
 *
 * Input lines: case JSON. Output lines: {value, cursor, highlight, display, draws} or {closed} or {error}.
 */
public final class VanillaChatScreenOracle {
    static Object unsafe;
    static Method allocate;
    static CommandDispatcher<CommandSourceStack> server;
    static CommandSourceStack source;
    static RegistryAccess access;
    static HolderLookup.Provider lookup;

    @SuppressWarnings("unchecked")
    static <T> T alloc(Class<T> type) throws Exception {
        return (T) allocate.invoke(unsafe, type);
    }

    static void put(Object target, Class<?> owner, String name, Object value) throws Exception {
        VanillaPackScreenOracle.put(target, owner, name, value);
    }

    static Object get(Object target, Class<?> owner, String name) throws Exception {
        Field f = owner.getDeclaredField(name);
        f.setAccessible(true);
        return f.get(target);
    }

    static <T> OptionInstance<T> option(T value) throws Exception {
        @SuppressWarnings("unchecked") OptionInstance<T> o = alloc(OptionInstance.class);
        put(o, OptionInstance.class, "value", value);
        return o;
    }

    /** The client's suggestion provider; what it asks the server is answered at once. */
    public static final class Provider extends ClientSuggestionProvider {
        private Provider() { super(null, null, null); }
        @Override public Collection<String> getOnlinePlayerNames() { return new ArrayList<>(List.of(VanillaCommandParseOracle.PLAYERS)); }
        @Override public Collection<String> getCustomTabSuggestions() { return getOnlinePlayerNames(); }
        @Override public Collection<String> getSelectedEntities() { return new ArrayList<>(List.of(VanillaCommandParseOracle.SELECTED)); }
        @Override public Collection<String> getAllTeams() { return new ArrayList<>(List.of(VanillaCommandParseOracle.TEAMS)); }
        @Override public java.util.stream.Stream<Identifier> getAvailableSounds() { return BuiltInRegistries.SOUND_EVENT.keySet().stream(); }
        @Override public java.util.stream.Stream<Identifier> getAvailablePostEffects() { return source.getAvailablePostEffects(); }
        /** ClientPacketListener's: the player's (an operator's) and ALLOW_RESTRICTED_COMMANDS. */
        @Override public PermissionSet permissions() {
            try {
                Field allow = ClientPacketListener.class.getDeclaredField("ALLOW_RESTRICTED_COMMANDS");
                allow.setAccessible(true);
                return LevelBasedPermissionSet.forLevel(PermissionLevel.byId(4)).union((PermissionSet) allow.get(null));
            } catch (ReflectiveOperationException e) {
                throw new IllegalStateException(e);
            }
        }
        @Override public Collection<SharedSuggestionProvider.TextCoordinates> getRelevantCoordinates() { return source.getRelevantCoordinates(); }
        @Override public Collection<SharedSuggestionProvider.TextCoordinates> getAbsoluteCoordinates() { return source.getAbsoluteCoordinates(); }
        @Override public Set<ResourceKey<Level>> levels() { return source.levels(); }
        @Override public RegistryAccess registryAccess() { return access; }
        @Override public FeatureFlagSet enabledFeatures() { return FeatureFlags.VANILLA_SET; }
        @Override public <E> CompletableFuture<Suggestions> suggestRegistryElements(ResourceKey<? extends Registry<E>> key,
            SharedSuggestionProvider.ElementSuggestionType elements, SuggestionsBuilder builder, CommandContext<?> context, java.util.function.Predicate<E> filter) {
            return source.suggestRegistryElements(key, elements, builder, context, filter);
        }
        @Override public CompletableFuture<Suggestions> customSuggestion(CommandContext<?> context) {
            StringReader reader = new StringReader(context.getInput());
            if (reader.canRead() && reader.peek() == '/') reader.skip();
            return CompletableFuture.completedFuture(server.getCompletionSuggestions(server.parse(reader, source)).join());
        }
    }

    /** Recent messages only; the chat lines are compared by the HUD harness. */
    public static final class Chat extends ChatComponent {
        private Chat() { super(null); }
        @Override public void resetChatScroll() { }
        @Override public void discardDraft() { }
        @Override public void saveAsDraft(String text) { }
        @Override public void captureClickableText(net.minecraft.client.gui.ActiveTextCollector output, int screenHeight, int ticks,
                                                   ChatComponent.DisplayMode mode) { }
    }

    public static final class NoNarrator extends net.minecraft.client.GameNarrator {
        private NoNarrator() { super(null); }
        @Override public void saySystemNow(Component message) { }
        @Override public void saySystemNow(String message) { }
    }

    /** VanillaCommandParseOracle's server state: the dispatcher and an operator's source. */
    static void server() throws Exception {
        var repo = ServerPacksSource.createVanillaTrustedRepository();
        repo.reload();
        repo.setSelected(List.of("vanilla"));
        var resources = new MultiPackResourceManager(PackType.SERVER_DATA, repo.openAllSelected());
        var layers = RegistryLayer.createRegistryAccess();
        var staticTags = TagLoader.loadTagsForExistingRegistries(resources, layers.getLayer(RegistryLayer.STATIC));
        var context = TagLoader.buildUpdatedLookups(layers.getAccessForLoading(RegistryLayer.WORLD), staticTags);
        RegistryAccess.Frozen world = RegistryDataLoader.load(resources, context, RegistryDataLoader.WORLD_REGISTRIES, Runnable::run).join();
        var dims = RegistryDataLoader.load(resources, java.util.stream.Stream.concat(context.stream(), world.listRegistries()).toList(), RegistryDataLoader.DIMENSION_REGISTRIES, Runnable::run).join();
        var worldLayers = layers.replaceFrom(RegistryLayer.WORLD, world, dims);
        var loaded = ReloadableServerRegistries.reload(worldLayers, staticTags, resources, Runnable::run).join();
        var managers = net.minecraft.server.ReloadableServerResources.loadResources(
            resources, worldLayers, staticTags, FeatureFlags.VANILLA_SET, Commands.CommandSelection.INTEGRATED,
            LevelBasedPermissionSet.GAMEMASTER, Runnable::run, Runnable::run).join();
        lookup = loaded.lookupWithUpdatedTags();
        var components = BuiltInRegistries.DATA_COMPONENT_INITIALIZERS.build(lookup);
        staticTags.forEach(Registry.PendingTags::apply);
        components.forEach(net.minecraft.core.component.DataComponentInitializers.PendingComponents::apply);
        access = loaded.layers().compositeAccess();
        Commands commands = new Commands(Commands.CommandSelection.INTEGRATED, CommandBuildContext.simple(lookup, FeatureFlags.VANILLA_SET));
        server = commands.getDispatcher();
        MinecraftServer mcs = alloc(DedicatedServer.class);
        put(mcs, MinecraftServer.class, "registries", loaded.layers());
        Class<?> reloadable = Class.forName("net.minecraft.server.MinecraftServer$ReloadableResources");
        var reloadableCtor = reloadable.getDeclaredConstructor(net.minecraft.server.packs.resources.CloseableResourceManager.class, net.minecraft.server.ReloadableServerResources.class);
        reloadableCtor.setAccessible(true);
        put(mcs, MinecraftServer.class, "resources", reloadableCtor.newInstance(resources, managers));
        put(mcs, MinecraftServer.class, "functionManager", new net.minecraft.server.ServerFunctionManager(mcs, managers.getFunctionLibrary()));
        put(mcs, MinecraftServer.class, "packRepository", repo);
        put(mcs, MinecraftServer.class, "commandStorage", new net.minecraft.world.level.storage.CommandStorage(null));
        var stopwatchesCtor = net.minecraft.world.Stopwatches.class.getDeclaredConstructor();
        stopwatchesCtor.setAccessible(true);
        put(mcs, MinecraftServer.class, "stopwatches", stopwatchesCtor.newInstance());
        put(mcs, MinecraftServer.class, "customBossEvents", new net.minecraft.server.bossevents.CustomBossEvents());
        put(mcs, MinecraftServer.class, "randomSequences", new net.minecraft.world.RandomSequences());
        put(mcs, MinecraftServer.class, "scheduledEvents", new net.minecraft.world.level.timers.TimerQueue<MinecraftServer>());
        var clocks = net.minecraft.world.clock.ServerClockManager.TYPE.constructor().get();
        clocks.init(mcs);
        put(mcs, MinecraftServer.class, "clockManager", clocks);
        var templates = alloc(net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplateManager.class);
        put(templates, net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplateManager.class, "sources",
            List.of(VanillaCommandParseOracle.templateSource(lookup, resources)));
        put(mcs, MinecraftServer.class, "structureTemplateManager", templates);
        var level = alloc(net.minecraft.server.level.ServerLevel.class);
        put(level, Level.class, "dimensionTypeRegistration",
            access.lookupOrThrow(Registries.DIMENSION_TYPE).getOrThrow(net.minecraft.world.level.dimension.BuiltinDimensionTypes.OVERWORLD));
        put(level, net.minecraft.server.level.ServerLevel.class, "server", mcs);
        PlayerList playerList = alloc(net.minecraft.server.dedicated.DedicatedPlayerList.class);
        put(playerList, PlayerList.class, "players", new ArrayList<>());
        put(mcs, MinecraftServer.class, "playerList", playerList);
        ServerScoreboard scoreboard = new ServerScoreboard(mcs);
        put(mcs, MinecraftServer.class, "scoreboard", scoreboard);
        for (String team : VanillaCommandParseOracle.TEAMS) scoreboard.addPlayerTeam(team);
        for (String objective : VanillaCommandParseOracle.OBJECTIVES)
            scoreboard.addObjective(objective, net.minecraft.world.scores.criteria.ObjectiveCriteria.DUMMY, Component.literal(objective),
                net.minecraft.world.scores.criteria.ObjectiveCriteria.RenderType.INTEGER, true, null);
        for (String tracked : VanillaCommandParseOracle.TRACKED)
            scoreboard.getOrCreatePlayerScore(net.minecraft.world.scores.ScoreHolder.forNameOnly(tracked), scoreboard.getObjective("kills"));
        source = new VanillaCommandParseOracle.Src(mcs, level, LevelBasedPermissionSet.forLevel(PermissionLevel.byId(4)), access, lookup);
    }

    /** Commands.sendCommands for an operator, then ClientPacketListener.handleCommands. */
    @SuppressWarnings("unchecked")
    static CommandDispatcher<ClientSuggestionProvider> clientTree() throws Exception {
        RootCommandNode<CommandSourceStack> root = new RootCommandNode<>();
        Map<com.mojang.brigadier.tree.CommandNode<CommandSourceStack>, com.mojang.brigadier.tree.CommandNode<CommandSourceStack>> converted = new HashMap<>();
        converted.put(server.getRoot(), root);
        Method fill = Commands.class.getDeclaredMethod("fillUsableCommands", com.mojang.brigadier.tree.CommandNode.class, com.mojang.brigadier.tree.CommandNode.class, Object.class, Map.class);
        fill.setAccessible(true);
        fill.invoke(null, server.getRoot(), root, source, converted);
        Field inspector = Commands.class.getDeclaredField("COMMAND_NODE_INSPECTOR");
        inspector.setAccessible(true);
        var packet = new ClientboundCommandsPacket(root, (ClientboundCommandsPacket.NodeInspector<CommandSourceStack>) inspector.get(null));
        Field builder = ClientPacketListener.class.getDeclaredField("COMMAND_NODE_BUILDER");
        builder.setAccessible(true);
        return new CommandDispatcher<>(packet.getRoot(CommandBuildContext.simple(access, FeatureFlags.VANILLA_SET),
            (ClientboundCommandsPacket.NodeBuilder<ClientSuggestionProvider>) builder.get(null)));
    }

    /** A key by name: [SDL scancode, keycode], with "shift+" and "ctrl+" modifiers. */
    static KeyEvent key(String name) {
        int mods = 0;
        while (true) {
            if (name.startsWith("shift+")) { mods |= 1; name = name.substring(6); }
            else if (name.startsWith("ctrl+")) { mods |= 64; name = name.substring(5); }
            else break;
        }
        int[] k = switch (name) {
            case "backspace" -> new int[] {42, 8};
            case "delete" -> new int[] {76, 127};
            case "home" -> new int[] {74, 1073741898};
            case "end" -> new int[] {77, 1073741901};
            case "right" -> new int[] {79, 1073741903};
            case "left" -> new int[] {80, 1073741904};
            case "down" -> new int[] {81, 1073741905};
            case "up" -> new int[] {82, 1073741906};
            case "tab" -> new int[] {43, 9};
            case "escape" -> new int[] {41, 27};
            case "a" -> new int[] {4, 97};
            default -> throw new IllegalArgumentException(name);
        };
        return new KeyEvent(k[0], k[1], mods);
    }

    static JsonObject run(JsonObject c, Minecraft mc, VanillaPackScreenOracle.StubGui gui) throws Exception {
        int width = c.get("width").getAsInt(), height = c.get("height").getAsInt();
        gui.width = width;
        gui.height = height;
        gui.current = null;
        var chat = alloc(Chat.class);
        var recent = new ArrayListDeque<String>(100);
        for (var s : c.getAsJsonArray("history")) recent.add(s.getAsString());
        put(chat, ChatComponent.class, "recentChat", recent);
        var hud = alloc(Hud.class);
        put(hud, Hud.class, "chat", chat);
        put(gui, net.minecraft.client.gui.Gui.class, "hud", hud);
        var options = alloc(Options.class);
        put(options, Options.class, "autoSuggestions", option(c.get("autoSuggest").getAsBoolean()));
        put(options, Options.class, "backgroundForChatOnly", option(c.get("chatOnly").getAsBoolean()));
        put(options, Options.class, "textBackgroundOpacity", option(c.get("opacity").getAsDouble()));
        put(options, Options.class, "saveChatDrafts", option(false));
        var window = alloc(com.mojang.blaze3d.platform.Window.class);
        put(window, com.mojang.blaze3d.platform.Window.class, "guiScaledWidth", width);
        put(window, com.mojang.blaze3d.platform.Window.class, "guiScaledHeight", height);
        put(mc, Minecraft.class, "window", window);
        put(mc, Minecraft.class, "options", options);
        var screen = new ChatScreen(c.get("initial").getAsString(), false);
        gui.setScreen(screen);
        var info = new MouseButtonInfo(1, 0);
        for (var a : c.getAsJsonArray("actions")) {
            var o = a.getAsJsonObject();
            Screen s = gui.current;
            if (s != screen) break;
            switch (o.get("type").getAsString()) {
                case "type" -> o.get("text").getAsString().codePoints().forEach(cp -> screen.charTyped(new CharacterEvent(cp)));
                case "key" -> screen.keyPressed(key(o.get("key").getAsString()));
                case "click" -> {
                    var event = new MouseButtonEvent(o.get("x").getAsDouble(), o.get("y").getAsDouble(), info);
                    screen.mouseClicked(event, false);
                    screen.mouseReleased(event);
                }
                default -> throw new IllegalArgumentException(o.toString());
            }
        }
        var result = new JsonObject();
        if (gui.current != screen) {
            result.addProperty("closed", true);
            return result;
        }
        var input = (EditBox) get(screen, ChatScreen.class, "input");
        put(input, EditBox.class, "focusedTime", Util.getMillis() - c.get("blink").getAsLong());
        var g = alloc(VanillaPackScreenOracle.Capture.class);
        g.draws = new JsonArray();
        g.stack = new Matrix3x2fStack(16);
        g.width = width;
        g.height = height;
        int mx = c.getAsJsonArray("mouse").get(0).getAsInt(), my = c.getAsJsonArray("mouse").get(1).getAsInt();
        // ChatScreen.extractRenderState without ChatComponent's lines.
        g.fill(2, height - 14, width - 2, height - 2, options.getBackgroundColor(Integer.MIN_VALUE));
        @SuppressWarnings("unchecked")
        var renderables = (List<Renderable>) get(screen, Screen.class, "renderables");
        for (var r : renderables) r.extractRenderState(g, mx, my, 1.0F);
        ((CommandSuggestions) get(screen, ChatScreen.class, "commandSuggestions")).extractRenderState(g, mx, my);
        result.addProperty("value", input.getValue());
        result.addProperty("cursor", input.getCursorPosition());
        result.addProperty("highlight", (int) get(input, EditBox.class, "highlightPos"));
        result.addProperty("display", (int) get(input, EditBox.class, "displayPos"));
        result.add("draws", g.draws);
        return result;
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        unsafe = unsafeField.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        VanillaPackScreenOracle.unsafe = unsafe;
        VanillaPackScreenOracle.allocate = allocate;
        server();
        var font = ParityFont.create(ParityFont.resources());
        VanillaPackScreenOracle.font = font;
        Minecraft mc = alloc(Minecraft.class);
        put(mc, Minecraft.class, "font", font);
        put(mc, Minecraft.class, "fontFilterFishy", font);
        put(mc, Minecraft.class, "lastInputType", InputType.MOUSE);
        put(mc, Minecraft.class, "textInputManager", new VanillaPackScreenOracle.NoTextInput());
        put(mc, Minecraft.class, "soundManager", alloc(VanillaPackScreenOracle.NoSound.class));
        put(mc, Minecraft.class, "keyboardHandler", alloc(VanillaPackScreenOracle.NoKeyboard.class));
        put(mc, Minecraft.class, "narrator", alloc(NoNarrator.class));
        var gui = alloc(VanillaPackScreenOracle.StubGui.class);
        put(mc, Minecraft.class, "gui", gui);
        var connection = alloc(ClientPacketListener.class);
        put(connection, ClientPacketListener.class, "commands", clientTree());
        var provider = alloc(Provider.class);
        put(provider, ClientSuggestionProvider.class, "minecraft", mc);
        put(connection, ClientPacketListener.class, "suggestionsProvider", provider);
        var player = alloc(LocalPlayer.class);
        put(player, LocalPlayer.class, "connection", connection);
        put(player, LocalPlayer.class, "chatAbilities", ChatAbilities.NO_RESTRICTIONS);
        put(mc, Minecraft.class, "player", player);
        Field instance = Minecraft.class.getDeclaredField("instance");
        instance.setAccessible(true);
        instance.set(null, mc);
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
