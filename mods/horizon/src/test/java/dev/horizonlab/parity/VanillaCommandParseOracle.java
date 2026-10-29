package dev.horizonlab.parity;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.mojang.brigadier.CommandDispatcher;
import com.mojang.brigadier.ParseResults;
import com.mojang.brigadier.StringReader;
import com.mojang.brigadier.exceptions.CommandSyntaxException;
import com.mojang.brigadier.context.CommandContextBuilder;
import com.mojang.brigadier.context.ParsedCommandNode;
import com.mojang.brigadier.context.ContextChain;
import com.mojang.brigadier.suggestion.Suggestion;
import com.mojang.brigadier.suggestion.Suggestions;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collection;
import java.util.List;
import java.util.Set;
import java.util.TreeSet;
import net.minecraft.SharedConstants;
import net.minecraft.commands.CommandBuildContext;
import net.minecraft.commands.CommandSource;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
import net.minecraft.commands.SharedSuggestionProvider;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.chat.Component;
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
import net.minecraft.world.flag.FeatureFlagSet;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.phys.Vec2;
import net.minecraft.world.phys.Vec3;

/**
 * Runs a corpus of command strings through vanilla's real Brigadier command
 * tree and prints, per input, whether it parses, the error (message and
 * cursor), the parsed nodes and the completion suggestions at the end of the
 * input. Also exports the registry contents the Rust side needs.
 *
 * usage: registries|tree|run OUT_DIR [CORPUS PERMISSION_LEVEL | SELECTION]
 */
public final class VanillaCommandParseOracle {
    static sun.misc.Unsafe unsafe;
    static final String[] PLAYERS = {"Player", "Steve", "Alex"};
    /** ClientSuggestionProvider.getSelectedEntities: the crosshair entity's UUID. */
    static final String[] SELECTED = {"0b9c6a7e-3d1f-4a2b-9c8d-7e6f5a4b3c2d"};
    static final String[] TEAMS = {"red", "blue"};
    static final String[] OBJECTIVES = {"kills", "health", "obj"};
    static final String[] TRACKED = {"#total", "Steve", "fake"};

    static final class Src extends CommandSourceStack {
        final RegistryAccess access;
        final HolderLookup.Provider lookup;
        Src(MinecraftServer server, PermissionSet perms, RegistryAccess access, HolderLookup.Provider lookup) {
            this(server, null, perms, access, lookup);
        }
        Src(MinecraftServer server, net.minecraft.server.level.ServerLevel level, PermissionSet perms, RegistryAccess access, HolderLookup.Provider lookup) {
            super(CommandSource.NULL, Vec3.ZERO, Vec2.ZERO, level, perms, Component.literal("Player"), server);
            this.access = access;
            this.lookup = lookup;
        }
        @Override public Collection<String> getOnlinePlayerNames() { return new ArrayList<>(List.of(PLAYERS)); }
        @Override public Collection<String> getSelectedEntities() { return new ArrayList<>(List.of(SELECTED)); }
        @Override public Collection<String> getAllTeams() { return new ArrayList<>(List.of(TEAMS)); }
        @Override public RegistryAccess registryAccess() { return access; }
        @Override public Set<ResourceKey<Level>> levels() { return Set.of(Level.OVERWORLD, Level.NETHER, Level.END); }
        @Override public FeatureFlagSet enabledFeatures() { return FeatureFlags.VANILLA_SET; }
        @Override public <E> java.util.concurrent.CompletableFuture<Suggestions> suggestRegistryElements(ResourceKey<? extends Registry<E>> key, SharedSuggestionProvider.ElementSuggestionType elements, com.mojang.brigadier.suggestion.SuggestionsBuilder builder, com.mojang.brigadier.context.CommandContext<?> context, java.util.function.Predicate<E> filter) {
            java.util.Optional<? extends HolderLookup<E>> registry = access.lookup(key);
            if (registry.isEmpty()) registry = lookup.lookup(key);
            if (registry.isEmpty()) return Suggestions.empty();
            this.suggestRegistryElements(registry.get(), elements, builder, filter);
            return builder.buildFuture();
        }
    }

    static void put(Object target, Class<?> owner, String name, Object value) throws Exception {
        Field f = owner.getDeclaredField(name);
        long offset = unsafe.objectFieldOffset(f);
        unsafe.putObject(target, offset, value);
    }

    static String message(CommandSyntaxException e) {
        com.mojang.brigadier.Message m = e.getRawMessage();
        return m.getString();
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        var unsafeField = sun.misc.Unsafe.class.getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        unsafe = (sun.misc.Unsafe) unsafeField.get(null);
        Path out = Path.of(args[1]);
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
        // The data pack managers (advancements, functions, recipes) a running server holds.
        var managers = net.minecraft.server.ReloadableServerResources.loadResources(
            resources, worldLayers, staticTags, FeatureFlags.VANILLA_SET, Commands.CommandSelection.INTEGRATED,
            LevelBasedPermissionSet.GAMEMASTER, Runnable::run, Runnable::run).join();
        HolderLookup.Provider lookup = loaded.lookupWithUpdatedTags();
        var components = BuiltInRegistries.DATA_COMPONENT_INITIALIZERS.build(lookup);
        staticTags.forEach(Registry.PendingTags::apply);
        components.forEach(net.minecraft.core.component.DataComponentInitializers.PendingComponents::apply);
        RegistryAccess access = loaded.layers().compositeAccess();

        if (args[0].equals("registries")) {
            exportRegistries(out, access, lookup, repo, resources, managers);
            return;
        }
        FeatureFlagSet features = FeatureFlags.VANILLA_SET;
        CommandBuildContext buildContext = CommandBuildContext.simple(lookup, features);
        // `tree OUT_DIR ALL` exports the tree of every command (dedicated and integrated).
        var selection = args[0].equals("tree") && args.length > 2 ? Commands.CommandSelection.valueOf(args[2]) : Commands.CommandSelection.INTEGRATED;
        Commands commands = new Commands(selection, buildContext);
        CommandDispatcher<CommandSourceStack> dispatcher = commands.getDispatcher();

        if (args[0].equals("tree")) {
            Files.writeString(out.resolve("tree.json"), net.minecraft.commands.synchronization.ArgumentUtils.serializeNodeToJson(dispatcher, dispatcher.getRoot()).toString());
            return;
        }
        MinecraftServer server = (MinecraftServer) unsafe.allocateInstance(DedicatedServer.class);
        // The minimal state of a freshly created world that suggestion providers read:
        // empty command storage, stopwatches, boss bars, random sequences and
        // scheduled functions, the world clocks, the vanilla data pack (repository,
        // advancements, functions, structure templates) and an overworld level.
        put(server, MinecraftServer.class, "registries", loaded.layers());
        Class<?> reloadable = Class.forName("net.minecraft.server.MinecraftServer$ReloadableResources");
        var reloadableCtor = reloadable.getDeclaredConstructor(net.minecraft.server.packs.resources.CloseableResourceManager.class, net.minecraft.server.ReloadableServerResources.class);
        reloadableCtor.setAccessible(true);
        put(server, MinecraftServer.class, "resources", reloadableCtor.newInstance(resources, managers));
        put(server, MinecraftServer.class, "functionManager", new net.minecraft.server.ServerFunctionManager(server, managers.getFunctionLibrary()));
        put(server, MinecraftServer.class, "packRepository", repo);
        put(server, MinecraftServer.class, "commandStorage", new net.minecraft.world.level.storage.CommandStorage(null));
        var stopwatchesCtor = net.minecraft.world.Stopwatches.class.getDeclaredConstructor();
        stopwatchesCtor.setAccessible(true);
        put(server, MinecraftServer.class, "stopwatches", stopwatchesCtor.newInstance());
        put(server, MinecraftServer.class, "customBossEvents", new net.minecraft.server.bossevents.CustomBossEvents());
        put(server, MinecraftServer.class, "randomSequences", new net.minecraft.world.RandomSequences());
        put(server, MinecraftServer.class, "scheduledEvents", new net.minecraft.world.level.timers.TimerQueue<MinecraftServer>());
        var clocks = net.minecraft.world.clock.ServerClockManager.TYPE.constructor().get();
        clocks.init(server);
        put(server, MinecraftServer.class, "clockManager", clocks);
        var templates = (net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplateManager) unsafe.allocateInstance(
            net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplateManager.class);
        put(templates, net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplateManager.class, "sources", List.of(templateSource(lookup, resources)));
        put(server, MinecraftServer.class, "structureTemplateManager", templates);
        var level = (net.minecraft.server.level.ServerLevel) unsafe.allocateInstance(net.minecraft.server.level.ServerLevel.class);
        put(level, Level.class, "dimensionTypeRegistration",
            access.lookupOrThrow(Registries.DIMENSION_TYPE).getOrThrow(net.minecraft.world.level.dimension.BuiltinDimensionTypes.OVERWORLD));
        put(level, net.minecraft.server.level.ServerLevel.class, "server", server);
        PlayerList playerList = (PlayerList) unsafe.allocateInstance(net.minecraft.server.dedicated.DedicatedPlayerList.class);
        put(playerList, PlayerList.class, "players", new ArrayList<>());
        put(server, MinecraftServer.class, "playerList", playerList);
        ServerScoreboard scoreboard = new ServerScoreboard(server);
        put(server, MinecraftServer.class, "scoreboard", scoreboard);
        for (String team : TEAMS) scoreboard.addPlayerTeam(team);
        for (String objective : OBJECTIVES)
            scoreboard.addObjective(objective, net.minecraft.world.scores.criteria.ObjectiveCriteria.DUMMY, Component.literal(objective),
                net.minecraft.world.scores.criteria.ObjectiveCriteria.RenderType.INTEGER, true, null);
        Object holder = null;
        for (String tracked : TRACKED)
            scoreboard.getOrCreatePlayerScore(net.minecraft.world.scores.ScoreHolder.forNameOnly(tracked), scoreboard.getObjective("kills"));
        PermissionSet perms = LevelBasedPermissionSet.forLevel(PermissionLevel.byId(Integer.parseInt(args[3])));
        Src source = new Src(server, level, perms, access, lookup);

        List<String> lines = Files.readAllLines(Path.of(args[2]), StandardCharsets.UTF_8);
        java.io.BufferedWriter sb = Files.newBufferedWriter(out.resolve("vanilla.jsonl"));
        for (String raw : lines) {
            String input = raw;
            JsonObject row = new JsonObject();
            row.addProperty("in", input);
            try {
                ParseResults<CommandSourceStack> parse = dispatcher.parse(new StringReader(input), source);
                JsonObject result = new JsonObject();
                String error = null;
                int cursor = 0;
                if (parse.getReader().canRead()) {
                    if (parse.getExceptions().size() == 1) {
                        CommandSyntaxException e = parse.getExceptions().values().iterator().next();
                        error = message(e);
                        cursor = e.getCursor();
                    } else if (parse.getContext().getRange().isEmpty()) {
                        error = Component.translatable("command.unknown.command").getString();
                        cursor = parse.getReader().getCursor();
                    } else {
                        error = Component.translatable("command.unknown.argument").getString();
                        cursor = parse.getReader().getCursor();
                    }
                } else {
                    var ctx = parse.getContext().build(input);
                    if (ContextChain.tryFlatten(ctx).isEmpty()) {
                        error = Component.translatable("command.unknown.command").getString();
                        cursor = parse.getReader().getCursor();
                    }
                }
                if (error == null) {
                    result.addProperty("ok", true);
                    JsonArray nodes = new JsonArray();
                    CommandContextBuilder<CommandSourceStack> b = parse.getContext();
                    while (b != null) {
                        for (ParsedCommandNode<CommandSourceStack> n : b.getNodes()) nodes.add(n.getNode().getName());
                        b = b.getChild();
                    }
                    result.add("nodes", nodes);
                } else {
                    result.addProperty("ok", false);
                    result.addProperty("error", error);
                    result.addProperty("cursor", cursor);
                }
                row.add("parse", result);
                Suggestions suggestions = dispatcher.getCompletionSuggestions(parse, input.length()).get();
                JsonObject sg = new JsonObject();
                sg.addProperty("start", suggestions.getRange().getStart());
                sg.addProperty("end", suggestions.getRange().getEnd());
                JsonArray list = new JsonArray();
                for (Suggestion s : suggestions.getList()) {
                    String text = s.getText();
                    if (s.getTooltip() != null) text += "\t" + s.getTooltip().getString();
                    list.add(text);
                }
                sg.add("list", list);
                row.add("sugg", sg);
            } catch (Throwable t) {
                Throwable root = t;
                while (root.getCause() != null) root = root.getCause();
                StackTraceElement[] st = root.getStackTrace();
                row.addProperty("crash", root + (st.length > 0 ? " @ " + st[0] : ""));
            }
            sb.write(row.toString());
            sb.write("\n");
        }
        sb.close();
    }

    static net.minecraft.world.level.levelgen.structure.templatesystem.loader.ResourceManagerTemplateSource templateSource(HolderLookup.Provider lookup, MultiPackResourceManager resources) {
        return new net.minecraft.world.level.levelgen.structure.templatesystem.loader.ResourceManagerTemplateSource(
            net.minecraft.util.datafix.DataFixers.getDataFixer(), lookup.lookupOrThrow(Registries.BLOCK), resources,
            new net.minecraft.resources.FileToIdConverter("structure", ".nbt"));
    }

    static void exportRegistries(Path out, RegistryAccess access, HolderLookup.Provider lookup, net.minecraft.server.packs.repository.PackRepository repo, MultiPackResourceManager resources, net.minecraft.server.ReloadableServerResources managers) throws Exception {
        JsonObject o = new JsonObject();
        Set<String> learnable = new TreeSet<>();
        lookup.lookupOrThrow(Registries.RECIPE).listElements().filter(h -> !h.value().isSpecial()).forEach(h -> learnable.add(h.key().identifier().toString()));
        JsonArray learnableArr = new JsonArray();
        learnable.forEach(learnableArr::add);
        o.add("learnable_recipes", learnableArr);
        JsonObject criteria = new JsonObject();
        managers.getAdvancements().getAllAdvancements().stream()
            .sorted(java.util.Comparator.comparing(a -> a.id().toString()))
            .forEach(a -> {
                JsonArray names = new JsonArray();
                new TreeSet<>(a.value().criteria().keySet()).forEach(names::add);
                criteria.add(a.id().toString(), names);
            });
        o.add("advancement_criteria", criteria);
        JsonArray blockEntities = new JsonArray();
        for (Block block : BuiltInRegistries.BLOCK) {
            if (FeatureFlags.VANILLA_SET.isSubsetOf(block.requiredFeatures()) && block.defaultBlockState().hasBlockEntity())
                blockEntities.add(BuiltInRegistries.BLOCK.getKey(block).toString());
        }
        o.add("block_entities", blockEntities);
        Set<String> persistent = new TreeSet<>();
        for (var entry : BuiltInRegistries.DATA_COMPONENT_TYPE.entrySet())
            if (!entry.getValue().isTransient()) persistent.add(entry.getKey().identifier().toString());
        JsonArray persistentArr = new JsonArray();
        persistent.forEach(persistentArr::add);
        o.add("persistent_components", persistentArr);
        JsonObject blockTagMembers = new JsonObject();
        lookup.lookupOrThrow(Registries.BLOCK).listTags().sorted(java.util.Comparator.comparing(t -> t.key().location().toString())).forEach(tag -> {
            JsonArray members = new JsonArray();
            tag.forEach(holder -> members.add(holder.unwrapKey().orElseThrow().identifier().toString()));
            blockTagMembers.add(tag.key().location().toString(), members);
        });
        o.add("block_tag_members", blockTagMembers);
        JsonArray selected = new JsonArray();
        repo.getSelectedIds().forEach(selected::add);
        o.add("datapacks_selected", selected);
        JsonArray available = new JsonArray();
        repo.getAvailableIds().forEach(available::add);
        o.add("datapacks_available", available);
        JsonArray enableable = new JsonArray();
        repo.getAvailablePacks().stream().filter(p -> p.getRequestedFeatures().isSubsetOf(FeatureFlags.VANILLA_SET))
            .forEach(p -> enableable.add(p.getId()));
        o.add("datapacks_enableable", enableable);
        // ServerClockManager.commandTimeMarkersForClock, from the timelines' markers.
        java.util.Map<String, Set<String>> markers = new java.util.TreeMap<>();
        access.lookupOrThrow(Registries.WORLD_CLOCK).listElementIds().forEach(k -> markers.put(k.identifier().toString(), new TreeSet<>()));
        access.lookupOrThrow(Registries.TIMELINE).listElements().forEach(t -> t.value().registerTimeMarkers((key, marker) -> {
            if (marker.showInCommands())
                markers.computeIfAbsent(marker.clock().unwrapKey().orElseThrow().identifier().toString(), x -> new TreeSet<>()).add(key.identifier().toString());
        }));
        JsonObject markerObj = new JsonObject();
        markers.forEach((clock, ids) -> { JsonArray a = new JsonArray(); ids.forEach(a::add); markerObj.add(clock, a); });
        o.add("clock_time_markers", markerObj);
        Set<String> templates = new TreeSet<>();
        templateSource(lookup, resources).list().forEach(id -> templates.add(id.toString()));
        JsonArray templateArr = new JsonArray();
        templates.forEach(templateArr::add);
        o.add("templates", templateArr);
        JsonArray items = new JsonArray();
        for (var item : BuiltInRegistries.ITEM) {
            if (FeatureFlags.VANILLA_SET.isSubsetOf(item.requiredFeatures()))
                items.add(BuiltInRegistries.ITEM.getKey(item).toString());
        }
        o.add("items", items);
        JsonObject blocks = new JsonObject();
        for (Block block : BuiltInRegistries.BLOCK) {
            if (!FeatureFlags.VANILLA_SET.isSubsetOf(block.requiredFeatures())) continue;
            JsonArray props = new JsonArray();
            for (Property<?> p : block.getStateDefinition().getProperties()) {
                JsonArray values = new JsonArray();
                for (Object v : p.getPossibleValues()) values.add(((Property) p).getName((Comparable) v));
                JsonArray pair = new JsonArray();
                pair.add(p.getName());
                pair.add(values);
                props.add(pair);
            }
            blocks.add(BuiltInRegistries.BLOCK.getKey(block).toString(), props);
        }
        o.add("blocks", blocks);
        o.add("item_tags", tags(lookup, Registries.ITEM));
        o.add("block_tags", tags(lookup, Registries.BLOCK));
        JsonArray summonable = new JsonArray();
        JsonArray types = new JsonArray();
        for (var type : BuiltInRegistries.ENTITY_TYPE) {
            if (!FeatureFlags.VANILLA_SET.isSubsetOf(type.requiredFeatures())) continue;
            String id = BuiltInRegistries.ENTITY_TYPE.getKey(type).toString();
            types.add(id);
            if (type.canSummon()) summonable.add(id);
        }
        o.add("entity_types", types);
        o.add("summonable", summonable);
        o.add("effects", ids(BuiltInRegistries.MOB_EFFECT));
        o.add("attributes", ids(BuiltInRegistries.ATTRIBUTE));
        o.add("sounds", ids(BuiltInRegistries.SOUND_EVENT));
        o.add("enchantments", dynamic(access, Registries.ENCHANTMENT));
        o.add("structures", dynamic(access, Registries.STRUCTURE));
        o.add("biomes", dynamic(access, Registries.BIOME));
        o.add("damage_types", dynamic(access, Registries.DAMAGE_TYPE));
        o.add("clocks", dynamic(access, Registries.WORLD_CLOCK));
        o.add("timelines", dynamic(access, Registries.TIMELINE));
        o.add("dimensions", arr("minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"));
        o.add("players", arr(PLAYERS));
        o.add("selected_entities", arr(SELECTED));
        o.add("teams", arr(TEAMS));
        o.add("objectives", arr(OBJECTIVES));
        o.add("tracked", arr(TRACKED));
        JsonObject by = new JsonObject();
        lookup.listRegistryKeys().sorted(java.util.Comparator.comparing(k -> k.identifier().toString())).forEach(key -> {
            var reg = lookup.lookupOrThrow(key);
            JsonObject entry = new JsonObject();
            java.util.List<String> ids = reg.listElementIds().map(k -> k.identifier().toString()).sorted().toList();
            JsonArray idArr = new JsonArray();
            ids.forEach(idArr::add);
            entry.add("ids", idArr);
            JsonArray tagArr = new JsonArray();
            reg.listTagIds().map(t -> t.location().toString()).sorted().forEach(tagArr::add);
            entry.add("tags", tagArr);
            by.add(key.identifier().toString(), entry);
        });
        o.add("by_registry", by);
        Files.writeString(out.resolve("registries.json"), o.toString());
    }

    static JsonArray arr(String... values) {
        JsonArray a = new JsonArray();
        for (String v : values) a.add(v);
        return a;
    }

    static <T> JsonArray ids(Registry<T> registry) {
        Set<String> sorted = new TreeSet<>();
        for (var key : registry.keySet()) sorted.add(key.toString());
        JsonArray a = new JsonArray();
        sorted.forEach(a::add);
        return a;
    }

    static <T> JsonArray dynamic(RegistryAccess access, ResourceKey<? extends Registry<T>> key) {
        return ids(access.lookupOrThrow(key));
    }

    static <T> JsonArray tags(HolderLookup.Provider lookup, ResourceKey<? extends Registry<T>> key) {
        Set<String> sorted = new TreeSet<>();
        lookup.lookupOrThrow(key).listTagIds().forEach(t -> sorted.add(t.location().toString()));
        JsonArray a = new JsonArray();
        sorted.forEach(a::add);
        return a;
    }
}
