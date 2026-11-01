package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.BlockPos;
import net.minecraft.core.NonNullList;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.Container;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.item.PrimedTnt;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.CrafterBlock;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.CrafterBlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.gamerules.GameRules;
import net.minecraft.world.level.levelgen.LegacyRandomSource;

/**
 * Drives vanilla `DispenserBlock.dispenseFrom` (and the dropper's) over the
 * headless level of {@link VanillaBlockUpdateOracle}, with entities: the
 * dispenser holds the case's items, target entities stand in front, and the
 * result lists block changes, container contents, spawned entities with their
 * position and motion, the targets' state afterwards, level events, sounds
 * (those the client plays for level events included), game events and the
 * level random. Every case runs three times: numbers that differ between the
 * runs come from an entity's own unseeded random and are marked with "~".
 * Used by tools/test_dispenser_parity.py.
 */
public final class VanillaDispenserOracle {
    /** The sound LevelEventHandler plays for a level event a dispenser can cause, or null. */
    static String eventSound(int id, int data) {
        return switch (id) {
            case 1000 -> "minecraft:block.dispenser.dispense";
            case 1001 -> "minecraft:block.dispenser.fail";
            case 1002 -> "minecraft:block.dispenser.launch";
            case 1004 -> "minecraft:entity.firework_rocket.shoot";
            case 1018 -> "minecraft:entity.blaze.shoot";
            case 1049 -> "minecraft:block.crafter.craft";
            case 1050 -> "minecraft:block.crafter.fail";
            case 1051 -> "minecraft:entity.wind_charge.throw";
            case 1505 -> "minecraft:item.bone_meal.use";
            case 1501 -> "minecraft:block.lava.extinguish";
            case 1500 -> data == 1 ? "minecraft:block.composter.fill_success" : "minecraft:block.composter.fill";
            case 1009 -> data == 0 ? "minecraft:block.fire.extinguish" : data == 1 ? "minecraft:block.generic.extinguish_fire" : null;
            case 2001 -> {
                BlockState state = Block.stateById(data);
                yield state.isAir() ? null : state.getSoundType().getBreakSound().location().toString();
            }
            default -> null;
        };
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        VanillaBlockUpdateOracle.AIR = Blocks.AIR.defaultBlockState();
        Path resources = Path.of(args[0]);
        VanillaLevelShapeOracle.bindFluidTags(resources.resolve("data/minecraft/tags/fluid"));
        VanillaBlockUpdateOracle.OVERWORLD_TYPE = VanillaBlockUpdateOracle.loadOverworld();
        var allocator = Class.forName("sun.misc.Unsafe");
        var field = allocator.getDeclaredField("theUnsafe");
        field.setAccessible(true);
        VanillaBlockUpdateOracle.unsafe = field.get(null);
        VanillaBlockUpdateOracle.allocate = allocator.getMethod("allocateInstance", Class.class);
        var recipes = new net.minecraft.world.item.crafting.RecipeManager(ParityData.lookup);
        recipes.finalizeRecipeLoading(net.minecraft.world.flag.FeatureFlags.VANILLA_SET);
        VanillaBlockUpdateOracle.Server.recipes = recipes;
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[2]))); var lines = Files.lines(Path.of(args[1]))) {
            int index = 0;
            for (var line : (Iterable<String>) lines::iterator) {
                var f = JsonParser.parseString(line).getAsJsonObject();
                // Three runs: a value from an unseeded random rarely repeats thrice.
                List<String> a = attempt(f), b = attempt(f), c = attempt(f);
                out.print("case " + index++ + "\n");
                for (String s : merge(merge(a, b), c)) out.print(s + "\n");
            }
        }
    }

    static List<String> attempt(JsonObject f) {
        List<String> result = new ArrayList<>();
        try {
            run(f, result);
        } catch (Throwable e) {
            Throwable c = e;
            while (c.getCause() != null) c = c.getCause();
            if (System.getenv("ORACLE_TRACE") != null) c.printStackTrace();
            StackTraceElement[] st = c.getStackTrace();
            result.clear();
            result.add("error " + c.getClass().getSimpleName() + " " + (st.length > 0 ? st[0].toString() : "") + " " + c.getMessage());
        }
        return result;
    }

    /** Lines equal in both runs stay; numbers that differ are marked "~" (from an unseeded entity random). */
    static List<String> merge(List<String> a, List<String> b) {
        if (a.size() != b.size()) return a;
        List<String> out = new ArrayList<>();
        for (int i = 0; i < a.size(); i++) {
            String x = a.get(i), y = b.get(i);
            if (x.equals(y)) {
                out.add(x);
                continue;
            }
            String[] p = x.split(" "), q = y.split(" ");
            if (p.length != q.length) {
                out.add(x);
                continue;
            }
            // A token already marked in an earlier merge stays marked.
            for (int j = 0; j < p.length; j++) if (!p[j].equals(q[j]) && !p[j].startsWith("~")) p[j] = "~" + p[j];
            out.add(String.join(" ", p));
        }
        return out;
    }

    static String num(double v) {
        return String.format(Locale.ROOT, "%.6f", v);
    }

    static ItemStack stack(String snbt) throws Exception {
        return ParityData.stack(snbt);
    }

    /** The item list of a container block entity (dispensers, droppers, chests, hoppers, barrels...). */
    @SuppressWarnings("unchecked")
    static NonNullList<ItemStack> slots(BlockEntity be) throws Exception {
        for (Class<?> c = be.getClass(); c != null; c = c.getSuperclass()) {
            for (Field fd : c.getDeclaredFields()) {
                if (fd.getType() == NonNullList.class && !java.lang.reflect.Modifier.isStatic(fd.getModifiers())) {
                    fd.setAccessible(true);
                    return (NonNullList<ItemStack>) fd.get(be);
                }
            }
        }
        throw new IllegalStateException("no slots in " + be);
    }

    /** Put a stack in a slot: the item list of a container, else the one item a block entity holds (jukebox, lectern, decorated pot). */
    static void setSlot(BlockEntity be, int slot, ItemStack stack) throws Exception {
        for (Class<?> c = be.getClass(); c != null; c = c.getSuperclass()) {
            for (Field fd : c.getDeclaredFields()) {
                if (fd.getType() == NonNullList.class && !java.lang.reflect.Modifier.isStatic(fd.getModifiers())) {
                    fd.setAccessible(true);
                    @SuppressWarnings("unchecked") NonNullList<ItemStack> list = (NonNullList<ItemStack>) fd.get(be);
                    list.set(slot, stack);
                    return;
                }
            }
        }
        for (Class<?> c = be.getClass(); c != null; c = c.getSuperclass()) {
            for (Field fd : c.getDeclaredFields()) {
                if (fd.getType() == ItemStack.class && !java.lang.reflect.Modifier.isStatic(fd.getModifiers())) {
                    fd.setAccessible(true);
                    fd.set(be, stack);
                    return;
                }
            }
        }
        throw new IllegalStateException("no slots in " + be);
    }

    /** The removal a "remove" case runs on the block at its origin; a player's mining prints its statistics and held stack. */
    static List<String> remove(VanillaBlockUpdateOracle.World w, BlockPos origin, JsonObject op) throws Exception {
        switch (op.get("op").getAsString()) {
            case "set" -> w.setBlock(origin, Block.stateById(op.get("state").getAsInt()), op.get("flags").getAsInt());
            case "destroy" -> w.destroyBlock(origin, op.get("drop").getAsBoolean());
            case "remove" -> w.removeBlock(origin, false);
            case "mine" -> {
                return mine(w, origin, op);
            }
            default -> throw new IllegalStateException(op.toString());
        }
        return new ArrayList<>();
    }

    /**
     * A survival player three blocks south of `pos` mines it holding the case's
     * item: the real `ServerPlayerGameMode.destroyBlock` (playerWillDestroy,
     * removeBlock with the block entity's side effects, mineBlock, playerDestroy's
     * statistic, exhaustion and drops). The statistics and the held stack follow.
     */
    static List<String> mine(VanillaBlockUpdateOracle.World w, BlockPos pos, JsonObject op) throws Exception {
        Miner player = (Miner) VanillaBlockUpdateOracle.allocate.invoke(VanillaBlockUpdateOracle.unsafe, Miner.class);
        player.world = w;
        player.hand = stack(op.get("item").getAsString());
        player.offhand = ItemStack.EMPTY;
        player.stats = new ArrayList<>();
        VanillaBlockUpdateOracle.set(net.minecraft.world.entity.player.Player.class, player, "abilities", new net.minecraft.world.entity.player.Abilities());
        VanillaBlockUpdateOracle.set(net.minecraft.world.entity.player.Player.class, player, "foodData", new net.minecraft.world.food.FoodData());
        var advancements = (net.minecraft.server.PlayerAdvancements) VanillaBlockUpdateOracle.allocate.invoke(VanillaBlockUpdateOracle.unsafe, net.minecraft.server.PlayerAdvancements.class);
        VanillaBlockUpdateOracle.set(net.minecraft.server.PlayerAdvancements.class, advancements, "activeTriggers", new HashMap<>());
        VanillaBlockUpdateOracle.set(net.minecraft.server.level.ServerPlayer.class, player, "advancements", advancements);
        var at = net.minecraft.world.phys.Vec3.atBottomCenterOf(pos).add(0, 1, 3);
        VanillaBlockUpdateOracle.set(Entity.class, player, "position", at);
        VanillaBlockUpdateOracle.set(Entity.class, player, "blockPosition", BlockPos.containing(at));
        // PiglinAi.angerNearbyPiglins looks around the player's box (gold, chests, barrels...).
        player.setBoundingBox(EntityTypes.PLAYER.getDimensions().makeBoundingBox(at));
        var gameMode = new net.minecraft.server.level.ServerPlayerGameMode(player);
        VanillaBlockUpdateOracle.set(net.minecraft.server.level.ServerPlayerGameMode.class, gameMode, "gameModeForPlayer",
            net.minecraft.world.level.GameType.SURVIVAL);
        gameMode.destroyBlock(pos);
        List<String> out = new ArrayList<>(player.stats);
        out.add("mine held " + ParityData.canon(player.hand));
        return out;
    }

    /** The player of a "mine" case: a survival `Clicker` whose selected inventory slot is its main hand. */
    static final class Miner extends net.minecraft.server.level.ServerPlayer {
        VanillaBlockUpdateOracle.World world;
        ItemStack hand, offhand;
        List<String> stats;
        Miner() {
            super(null, null, null, null);
        }
        @Override public ServerLevel level() { return world; }
        @Override public boolean hasInfiniteMaterials() { return false; }
        @Override public boolean isCreative() { return false; }
        @Override public boolean isSpectator() { return false; }
        @Override public EntityType<?> getType() { return EntityTypes.PLAYER; }
        @Override public net.minecraft.core.Holder<EntityType<?>> typeHolder() { return EntityTypes.PLAYER.builtInRegistryHolder(); }
        @Override public ItemStack getItemBySlot(EquipmentSlot slot) {
            return slot == EquipmentSlot.MAINHAND ? hand : slot == EquipmentSlot.OFFHAND ? offhand : ItemStack.EMPTY;
        }
        @Override public void setItemSlot(EquipmentSlot slot, ItemStack stack) {
            if (slot == EquipmentSlot.MAINHAND) hand = stack; else if (slot == EquipmentSlot.OFFHAND) offhand = stack;
        }
        @Override public ItemStack getItemInHand(net.minecraft.world.InteractionHand h) {
            return h == net.minecraft.world.InteractionHand.MAIN_HAND ? hand : offhand;
        }
        // Player.hasCorrectToolForDrops asks the inventory's selected slot: the main hand.
        @Override public boolean hasCorrectToolForDrops(BlockState state) {
            return !state.requiresCorrectToolForDrops() || hand.isCorrectToolForDrops(state);
        }
        @Override public void awardStat(net.minecraft.stats.Stat<?> stat, int count) {
            stats.add("stat " + User.key(stat) + " " + BuiltInRegistries.STAT_TYPE.getKey(stat.getType()) + " " + count);
        }
        @Override public net.minecraft.server.players.NameAndId nameAndId() { return new net.minecraft.server.players.NameAndId(new UUID(0L, 0L), "Player"); }
    }

    static boolean hasList(Object be) {
        for (Class<?> c = be.getClass(); c != null; c = c.getSuperclass()) {
            for (Field fd : c.getDeclaredFields()) {
                if (fd.getType() == NonNullList.class && !java.lang.reflect.Modifier.isStatic(fd.getModifiers())) return true;
            }
        }
        return false;
    }

    static Method dispenseFrom(Class<?> type) throws Exception {
        for (Class<?> c = type; c != null; c = c.getSuperclass()) {
            try {
                Method m = c.getDeclaredMethod("dispenseFrom", ServerLevel.class, BlockState.class, BlockPos.class);
                m.setAccessible(true);
                return m;
            } catch (NoSuchMethodException ignored) {
            }
        }
        throw new IllegalStateException("no dispenseFrom on " + type);
    }

    static Entity target(VanillaBlockUpdateOracle.World w, JsonObject e) throws Exception {
        EntityType<?> type = BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse(e.get("type").getAsString()));
        Entity ent = type.create(w, EntitySpawnReason.COMMAND);
        var p = e.getAsJsonArray("pos");
        ent.snapTo(p.get(0).getAsDouble(), p.get(1).getAsDouble(), p.get(2).getAsDouble(), e.has("yaw") ? e.get("yaw").getAsFloat() : 0f, 0f);
        if (e.has("baby") && ent instanceof AgeableMob ageable) ageable.setAge(-24000);
        if (e.has("tame")) {
            if (ent instanceof net.minecraft.world.entity.animal.equine.AbstractHorse horse) horse.setTamed(true);
            if (ent instanceof TamableAnimal tamable) tamable.setTame(true, false);
        }
        if (e.has("sheared")) {
            if (ent instanceof net.minecraft.world.entity.animal.sheep.Sheep sheep) sheep.setSheared(true);
            if (ent instanceof net.minecraft.world.entity.monster.skeleton.Bogged bogged) bogged.setSheared(true);
            if (ent instanceof net.minecraft.world.entity.animal.golem.SnowGolem golem) golem.setPumpkin(false);
        }
        if (e.has("chest") && ent instanceof net.minecraft.world.entity.animal.equine.AbstractChestedHorse chested) chested.setChest(true);
        if (e.has("loot") && ent instanceof Mob mob) mob.setCanPickUpLoot(true);
        if (e.has("ground")) ent.setOnGround(true);
        if (e.has("health") && ent instanceof LivingEntity living) {
            living.getAttribute(net.minecraft.world.entity.ai.attributes.Attributes.MAX_HEALTH).setBaseValue(e.get("health").getAsDouble());
            living.setHealth(e.get("health").getAsFloat());
        }
        if (e.has("knockback_resistance") && ent instanceof LivingEntity living) {
            living.getAttribute(net.minecraft.world.entity.ai.attributes.Attributes.KNOCKBACK_RESISTANCE).setBaseValue(e.get("knockback_resistance").getAsDouble());
        }
        if (e.has("marker") && ent instanceof net.minecraft.world.entity.decoration.ArmorStand stand) {
            Method marker = net.minecraft.world.entity.decoration.ArmorStand.class.getDeclaredMethod("setMarker", boolean.class);
            marker.setAccessible(true);
            marker.invoke(stand, true);
        }
        // ArmorStand.lastHit: the game tick of the stand's previous punch.
        if (e.has("last_hit") && ent instanceof net.minecraft.world.entity.decoration.ArmorStand stand) {
            stand.lastHit = e.get("last_hit").getAsLong();
        }
        if (e.has("brown") && ent instanceof net.minecraft.world.entity.animal.cow.MushroomCow cow) {
            Method variant = net.minecraft.world.entity.animal.cow.MushroomCow.class.getDeclaredMethod("setVariant", net.minecraft.world.entity.animal.cow.MushroomCow.Variant.class);
            variant.setAccessible(true);
            variant.invoke(cow, net.minecraft.world.entity.animal.cow.MushroomCow.Variant.BROWN);
        }
        // A target has lived in the level for a while: past its first tick, so
        // equipping it plays the equip sound.
        VanillaBlockUpdateOracle.set(Entity.class, ent, "firstTick", false);
        if (e.has("equip") && ent instanceof LivingEntity living) {
            for (var slot : e.getAsJsonObject("equip").entrySet()) {
                living.setItemSlot(EquipmentSlot.byName(slot.getKey()), stack(slot.getValue().getAsString()));
            }
            // A target that has ticked since it was equipped has run
            // collectEquipmentChanges (a sulfur cube takes its archetype then).
            Map<EquipmentSlot, ItemStack> last = new EnumMap<>(EquipmentSlot.class);
            for (EquipmentSlot slot : EquipmentSlot.values()) last.put(slot, ItemStack.EMPTY);
            Method collect = LivingEntity.class.getDeclaredMethod("collectEquipmentChanges", Map.class);
            collect.setAccessible(true);
            collect.invoke(ent, last);
        }
        return ent;
    }

    static void run(JsonObject f, List<String> result) throws Exception {
        VanillaBlockUpdateOracle.World w = VanillaBlockUpdateOracle.build(f);
        w.entities = new ArrayList<>();
        w.registries = ParityData.access;
        w.server.loot.delegate = ParityData.loot;
        VanillaBlockUpdateOracle.set(Level.class, w, "damageSources", new net.minecraft.world.damagesource.DamageSources(ParityData.access));
        if (f.has("dim") && f.get("dim").getAsString().equals("nether")) {
            // The Nether's dimension type and attributes (respawn anchors work, beds explode).
            if (NETHER == null) NETHER = VanillaBlockUpdateOracle.loadDimensionType(net.minecraft.world.level.dimension.BuiltinDimensionTypes.NETHER);
            w.type = NETHER;
            w.dimensionKey = Level.NETHER;
            w.attributes = net.minecraft.world.attribute.EnvironmentAttributeSystem.builder().addConstantLayer(NETHER.attributes()).build();
        } else if (f.has("dim") && f.get("dim").getAsString().equals("end")) {
            // The End's (beds explode, anchors do too).
            if (END == null) END = VanillaBlockUpdateOracle.loadDimensionType(net.minecraft.world.level.dimension.BuiltinDimensionTypes.END);
            w.type = END;
            w.dimensionKey = Level.END;
            w.attributes = net.minecraft.world.attribute.EnvironmentAttributeSystem.builder().addConstantLayer(END.attributes()).build();
        }
        if (f.has("rules")) {
            for (var rule : f.getAsJsonObject("rules").entrySet()) {
                if (rule.getKey().equals("tnt_explodes")) w.rules.set(GameRules.TNT_EXPLODES, rule.getValue().getAsBoolean(), null);
                if (rule.getKey().equals("spawner_blocks_work")) w.rules.set(GameRules.SPAWNER_BLOCKS_WORK, rule.getValue().getAsBoolean(), null);
                if (rule.getKey().equals("block_explosion_drop_decay")) w.rules.set(GameRules.BLOCK_EXPLOSION_DROP_DECAY, rule.getValue().getAsBoolean(), null);
            }
        }
        if (f.has("items")) {
            for (var item : f.getAsJsonArray("items")) {
                var it = item.getAsJsonArray();
                BlockPos at = new BlockPos(it.get(0).getAsInt(), it.get(1).getAsInt(), it.get(2).getAsInt());
                setSlot(w.getBlockEntity(at), it.get(3).getAsInt(), stack(it.get(4).getAsString()));
            }
        }
        if (f.has("disabled")) {
            for (var d : f.getAsJsonArray("disabled")) {
                var it = d.getAsJsonArray();
                BlockPos at = new BlockPos(it.get(0).getAsInt(), it.get(1).getAsInt(), it.get(2).getAsInt());
                ((CrafterBlockEntity) w.getBlockEntity(at)).setSlotState(it.get(3).getAsInt(), false);
            }
        }
        if (f.has("entities")) {
            List<Entity> created = new ArrayList<>();
            for (var e : f.getAsJsonArray("entities")) {
                Entity ent = target(w, e.getAsJsonObject());
                created.add(ent);
                w.entities.add(ent);
            }
            var list = f.getAsJsonArray("entities");
            for (int i = 0; i < list.size(); i++) {
                var e = list.get(i).getAsJsonObject();
                if (e.has("leash") && created.get(i) instanceof Leashable leashable) {
                    leashable.setLeashedTo(created.get(e.get("leash").getAsInt()), true);
                }
            }
        }
        // Creating the targets must not move the stream the dispenser draws from.
        w.rng = new LegacyRandomSource(f.get("seed").getAsLong());
        VanillaBlockUpdateOracle.set(Level.class, w, "random", w.rng);
        // Spawners have their block entity from the start (the oracle level makes them on demand).
        for (var cell : List.copyOf(w.cells.entrySet())) {
            if (cell.getValue().getBlock() instanceof net.minecraft.world.level.block.SpawnerBlock) w.getBlockEntity(cell.getKey());
        }
        w.log = new StringBuilder();
        w.blockEvents.clear();
        Map<BlockPos, BlockState> before = new HashMap<>(w.cells);
        List<Entity> targets = List.copyOf(w.entities);
        BlockPos origin = VanillaBlockUpdateOracle.readPos(f.getAsJsonArray("pos"));
        BlockState state = w.getBlockState(origin);
        List<String> used = new ArrayList<>();
        if (f.has("remove")) {
            used = remove(w, origin, f.getAsJsonObject("remove"));
        } else if (f.has("use")) {
            used = use(w, origin, f.getAsJsonObject("use"));
        } else if (f.has("use_item")) {
            used = useItem(w, origin, f.getAsJsonObject("use_item"));
        } else if (f.has("respawn_at")) {
            used = respawnAt(w, origin, f.getAsJsonObject("respawn_at"));
        } else if (f.has("attack")) {
            used = attack(w, f.getAsJsonObject("attack"), targets);
        } else if (f.has("hopper")) {
            // HopperBlockEntity.pushItemsTick, once.
            var hopper = (net.minecraft.world.level.block.entity.HopperBlockEntity) w.getBlockEntity(origin);
            net.minecraft.world.level.block.entity.HopperBlockEntity.pushItemsTick(w, origin, state, hopper);
        } else if (state.getBlock() instanceof CrafterBlock) {
            Method craft = CrafterBlock.class.getDeclaredMethod("dispenseFrom", BlockState.class, ServerLevel.class, BlockPos.class);
            craft.setAccessible(true);
            craft.invoke(state.getBlock(), state, w, origin);
            // CrafterBlockEntity.serverTick for every crafter, the animation running down.
            int ticks = f.has("ticks") ? f.get("ticks").getAsInt() : 0;
            for (int t = 0; t < ticks; t++) {
                for (BlockPos at : sortedEntities(w)) {
                    if (w.getBlockEntity(at) instanceof CrafterBlockEntity crafter) {
                        CrafterBlockEntity.serverTick(w, at, w.getBlockState(at), crafter);
                    }
                }
            }
        } else {
            dispenseFrom(state.getBlock().getClass()).invoke(state.getBlock(), w, state, origin);
        }

        result.addAll(VanillaBlockUpdateOracle.summary(w, before));
        List<BlockPos> containers = sortedEntities(w);
        for (BlockPos at : containers) {
            if (w.blockEntities.get(at) instanceof CrafterBlockEntity crafter && w.getBlockEntity(at) == crafter) {
                Field ticksField = CrafterBlockEntity.class.getDeclaredField("craftingTicksRemaining");
                ticksField.setAccessible(true);
                StringBuilder disabled = new StringBuilder();
                for (int i = 0; i < 9; i++) if (crafter.isSlotDisabled(i)) disabled.append(disabled.length() > 0 ? "," : "").append(i);
                result.add("crafter " + VanillaBlockUpdateOracle.pos(at) + " ticks=" + ticksField.getInt(crafter) + " triggered=" + crafter.isTriggered()
                    + " signal=" + crafter.getRedstoneSignal() + " disabled=" + disabled);
            }
        }
        for (BlockPos at : containers) {
            // A spawner's next spawn data, potentials and delay (BaseSpawner.save).
            if (w.blockEntities.get(at) instanceof net.minecraft.world.level.block.entity.SpawnerBlockEntity spawner) {
                var out = net.minecraft.world.level.storage.TagValueOutput.createWithContext(net.minecraft.util.ProblemReporter.DISCARDING, ParityData.access);
                spawner.getSpawner().save(out);
                var saved = out.buildResult();
                var entity = saved.getCompound("SpawnData").flatMap(d -> d.getCompound("entity"));
                String next = entity.map(e -> e.getString("id").orElse("?") + "/" + e.size()).orElse("-");
                result.add("spawner " + VanillaBlockUpdateOracle.pos(at) + " next=" + next + " potentials=" + saved.getListOrEmpty("SpawnPotentials").size()
                    + " delay=" + saved.getShortOr("Delay", (short) 0));
            }
        }
        for (BlockPos at : containers) {
            if (!(w.blockEntities.get(at) instanceof Container) || w.getBlockEntity(at) != w.blockEntities.get(at)) continue;
            List<ItemStack> items = w.blockEntities.get(at) instanceof Container c && !hasList(c) ? List.of(c.getItem(0)) : slots(w.blockEntities.get(at));
            for (int i = 0; i < items.size(); i++) {
                if (!items.get(i).isEmpty()) result.add("inv " + VanillaBlockUpdateOracle.pos(at) + " " + i + " " + ParityData.canon(items.get(i)));
            }
        }
        for (Entity e : w.entities) {
            if (targets.contains(e)) continue;
            StringBuilder line = new StringBuilder("N ").append(BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()));
            line.append(' ').append(num(e.getX())).append(' ').append(num(e.getY())).append(' ').append(num(e.getZ()));
            var v = e.getDeltaMovement();
            line.append(' ').append(num(v.x)).append(' ').append(num(v.y)).append(' ').append(num(v.z));
            if (e instanceof ItemEntity item) line.append(' ').append(ParityData.canon(item.getItem()));
            if (e instanceof PrimedTnt tnt) line.append(" fuse=").append(tnt.getFuse());
            // An orb that took others in (ExperienceOrb.tryMergeToExisting) prints once per
            // count: whether orbs merge depends on entity ids, which Horizon doesn't share.
            for (int n = e instanceof ExperienceOrb orb ? orbCount(orb) : 1; n > 0; n--) result.add(line.toString());
        }
        for (int i = 0; i < targets.size(); i++) result.add(describe(i, targets.get(i)));
        for (String s : w.log.toString().split("\n")) {
            if (s.isEmpty() || s.startsWith("E ")) continue;
            result.add(s);
            if (s.startsWith("L ")) {
                String[] p = s.split(" ");
                String sound = eventSound(Integer.parseInt(p[1]), Integer.parseInt(p[3]));
                if (sound != null) result.add("S " + p[2] + " " + sound);
            }
        }
        result.addAll(w.blockEvents);
        result.addAll(used);
        result.add("rng " + w.rng.nextLong());
    }

    static net.minecraft.world.level.dimension.DimensionType NETHER, END;

    /** The player of a "use" case: its main hand, game mode and the statistics it is awarded. */
    static final class User extends net.minecraft.server.level.ServerPlayer {
        VanillaBlockUpdateOracle.World world;
        boolean creative;
        ItemStack hand;
        List<String> stats;
        User() {
            super(null, null, null, null);
        }
        @Override public ServerLevel level() { return world; }
        @Override public boolean hasInfiniteMaterials() { return creative; }
        @Override public boolean isCreative() { return creative; }
        @Override public boolean isSpectator() { return false; }
        @Override public EntityType<?> getType() { return EntityTypes.PLAYER; }
        @Override public net.minecraft.core.Holder<EntityType<?>> typeHolder() { return EntityTypes.PLAYER.builtInRegistryHolder(); }
        @Override public ItemStack getItemInHand(net.minecraft.world.InteractionHand h) {
            return h == net.minecraft.world.InteractionHand.MAIN_HAND ? hand : ItemStack.EMPTY;
        }
        @Override public void awardStat(net.minecraft.stats.Stat<?> stat, int count) {
            stats.add("stat " + key(stat) + " " + BuiltInRegistries.STAT_TYPE.getKey(stat.getType()) + " " + count);
        }
        static <T> String key(net.minecraft.stats.Stat<T> stat) {
            return String.valueOf(stat.getType().getRegistry().getKey(stat.getValue()));
        }
    }

    /**
     * ServerPlayerGameMode.useItemOn by a player holding the case's item on the
     * block at `pos` (not sneaking): `useItemOn`, then `useWithoutItem` when the
     * block asks to try with an empty hand. The held stack and statistics follow.
     */
    static List<String> use(VanillaBlockUpdateOracle.World w, BlockPos pos, JsonObject use) throws Exception {
        User player = (User) VanillaBlockUpdateOracle.allocate.invoke(VanillaBlockUpdateOracle.unsafe, User.class);
        player.world = w;
        player.creative = use.has("creative") && use.get("creative").getAsBoolean();
        player.hand = stack(use.get("item").getAsString());
        player.stats = new ArrayList<>();
        var hit = new net.minecraft.world.phys.BlockHitResult(net.minecraft.world.phys.Vec3.atCenterOf(pos).add(0, 0.5, 0),
            net.minecraft.core.Direction.UP, pos, false);
        BlockState state = w.getBlockState(pos);
        var hand = net.minecraft.world.InteractionHand.MAIN_HAND;
        var result = state.useItemOn(player.getItemInHand(hand), w, player, hand, hit);
        if (result instanceof net.minecraft.world.InteractionResult.TryEmptyHandInteraction) {
            result = state.useWithoutItem(w, player, hit);
        }
        List<String> out = new ArrayList<>(player.stats);
        out.add("use " + (result.consumesAction() ? "consumed" : "pass") + " held " + ParityData.canon(player.hand));
        return out;
    }

    /**
     * The player of a "use_item" case: a `User` that also has abilities, food,
     * item cooldowns and a place (away from the clicked block), and records
     * starting to use its item instead of syncing it.
     */
    static final class Clicker extends net.minecraft.server.level.ServerPlayer {
        VanillaBlockUpdateOracle.World world;
        boolean creative, sneaking;
        ItemStack hand, offhand;
        List<String> stats;
        String using;
        Clicker() {
            super(null, null, null, null);
        }
        @Override public ServerLevel level() { return world; }
        @Override public boolean hasInfiniteMaterials() { return creative; }
        @Override public boolean isCreative() { return creative; }
        @Override public boolean isSpectator() { return false; }
        @Override public boolean isSecondaryUseActive() { return sneaking; }
        @Override public boolean isShiftKeyDown() { return sneaking; }
        @Override public boolean isDescending() { return sneaking; }
        @Override public EntityType<?> getType() { return EntityTypes.PLAYER; }
        @Override public net.minecraft.core.Holder<EntityType<?>> typeHolder() { return EntityTypes.PLAYER.builtInRegistryHolder(); }
        @Override public ItemStack getItemBySlot(EquipmentSlot slot) {
            return slot == EquipmentSlot.MAINHAND ? hand : slot == EquipmentSlot.OFFHAND ? offhand : ItemStack.EMPTY;
        }
        @Override public ItemStack getItemInHand(net.minecraft.world.InteractionHand h) {
            return h == net.minecraft.world.InteractionHand.MAIN_HAND ? hand : offhand;
        }
        @Override public void setItemInHand(net.minecraft.world.InteractionHand h, ItemStack stack) {
            if (h == net.minecraft.world.InteractionHand.MAIN_HAND) hand = stack; else offhand = stack;
        }
        @Override public void startUsingItem(net.minecraft.world.InteractionHand h) {
            using = BuiltInRegistries.ITEM.getKey(getItemInHand(h).getItem()).toString();
        }
        @Override public boolean isUsingItem() { return using != null; }
        @Override public void awardStat(net.minecraft.stats.Stat<?> stat, int count) {
            stats.add("stat " + User.key(stat) + " " + BuiltInRegistries.STAT_TYPE.getKey(stat.getType()) + " " + count);
        }
        @Override public net.minecraft.server.players.NameAndId nameAndId() { return new net.minecraft.server.players.NameAndId(new UUID(0L, 0L), "Player"); }
        @Override public void sendSystemMessage(net.minecraft.network.chat.Component message) {
            stats.add("msg " + (message.getContents() instanceof net.minecraft.network.chat.contents.TranslatableContents t ? t.getKey() : message.getString()));
        }
    }

    /** The single player's list: the player is an operator (cheats are on). */
    static final class Ops extends net.minecraft.server.players.PlayerList {
        Ops() { super(null, null, null, null); }
        @Override public boolean isOp(net.minecraft.server.players.NameAndId nameAndId) { return true; }
    }

    /**
     * A right click on the top of the block at `pos`, or on its `face`, by a
     * player turned to `yaw` (`ServerPlayerGameMode.useItemOn`
     * of the real game mode, survival or creative): the block's `useItemOn` and
     * `useWithoutItem`, then the item's `useOn` (`BlockItem.place`, falling back
     * to eating a consumable), and `useItem` when that passes, for the main
     * hand and then the off hand, as `Minecraft.startUseItem` goes on to. The
     * statistics, the held stacks and the item the player started using follow.
     */
    static List<String> useItem(VanillaBlockUpdateOracle.World w, BlockPos pos, JsonObject use) throws Exception {
        if (VanillaBlockUpdateOracle.Server.players == null) {
            VanillaBlockUpdateOracle.Server.players = (Ops) VanillaBlockUpdateOracle.allocate.invoke(VanillaBlockUpdateOracle.unsafe, Ops.class);
        }
        Clicker player = (Clicker) VanillaBlockUpdateOracle.allocate.invoke(VanillaBlockUpdateOracle.unsafe, Clicker.class);
        player.world = w;
        player.creative = use.get("creative").getAsBoolean();
        player.sneaking = use.get("sneaking").getAsBoolean();
        player.hand = stack(use.get("item").getAsString());
        player.offhand = use.has("offhand") ? stack(use.get("offhand").getAsString()) : ItemStack.EMPTY;
        player.stats = new ArrayList<>();
        var abilities = new net.minecraft.world.entity.player.Abilities();
        abilities.invulnerable = abilities.instabuild = abilities.mayfly = player.creative;
        VanillaBlockUpdateOracle.set(net.minecraft.world.entity.player.Player.class, player, "abilities", abilities);
        var food = new net.minecraft.world.food.FoodData();
        food.setFoodLevel(use.get("food").getAsInt());
        VanillaBlockUpdateOracle.set(net.minecraft.world.entity.player.Player.class, player, "foodData", food);
        VanillaBlockUpdateOracle.set(net.minecraft.world.entity.player.Player.class, player, "cooldowns", new net.minecraft.world.item.ItemCooldowns());
        var advancements = (net.minecraft.server.PlayerAdvancements) VanillaBlockUpdateOracle.allocate.invoke(VanillaBlockUpdateOracle.unsafe, net.minecraft.server.PlayerAdvancements.class);
        VanillaBlockUpdateOracle.set(net.minecraft.server.PlayerAdvancements.class, advancements, "activeTriggers", new HashMap<>());
        VanillaBlockUpdateOracle.set(net.minecraft.server.level.ServerPlayer.class, player, "advancements", advancements);
        // Standing three blocks (or `distance`) south of the clicked block, looking down at it.
        double distance = use.has("distance") ? use.get("distance").getAsDouble() : 3;
        var at = net.minecraft.world.phys.Vec3.atBottomCenterOf(pos).add(0, 1, distance);
        if (use.has("respawn")) {
            var r = use.getAsJsonArray("respawn");
            BlockPos point = new BlockPos(r.get(0).getAsInt(), r.get(1).getAsInt(), r.get(2).getAsInt());
            player.setRespawnPosition(new net.minecraft.server.level.ServerPlayer.RespawnConfig(
                net.minecraft.world.level.storage.LevelData.RespawnData.of(w.dimension(), point, 0.0F, 0.0F), false), false);
        }
        VanillaBlockUpdateOracle.set(Entity.class, player, "position", at);
        VanillaBlockUpdateOracle.set(Entity.class, player, "blockPosition", BlockPos.containing(at));
        VanillaBlockUpdateOracle.set(Entity.class, player, "xRot", 90f);
        if (use.has("yaw")) VanillaBlockUpdateOracle.set(Entity.class, player, "yRot", use.get("yaw").getAsFloat());
        var gameMode = new net.minecraft.server.level.ServerPlayerGameMode(player);
        VanillaBlockUpdateOracle.set(net.minecraft.server.level.ServerPlayerGameMode.class, gameMode, "gameModeForPlayer",
            player.creative ? net.minecraft.world.level.GameType.CREATIVE : net.minecraft.world.level.GameType.SURVIVAL);
        // The top face, or the case's `face`, clicked at its centre.
        var face = use.has("face") ? net.minecraft.core.Direction.byName(use.get("face").getAsString()) : net.minecraft.core.Direction.UP;
        var hit = new net.minecraft.world.phys.BlockHitResult(net.minecraft.world.phys.Vec3.atCenterOf(pos)
            .add(face.getStepX() * 0.5, face.getStepY() * 0.5, face.getStepZ() * 0.5), face, pos, false);
        // Minecraft.startUseItem: each hand in turn clicks the block, then uses
        // its item unless the click succeeded or failed; a success ends it.
        for (var hand : net.minecraft.world.InteractionHand.values()) {
            ItemStack held = player.getItemInHand(hand);
            var result = gameMode.useItemOn(player, w, held, hand, hit);
            if (result instanceof net.minecraft.world.InteractionResult.Success
                || result instanceof net.minecraft.world.InteractionResult.Fail) break;
            if (!held.isEmpty() && gameMode.useItem(player, w, held, hand) instanceof net.minecraft.world.InteractionResult.Success) break;
        }
        List<String> out = new ArrayList<>(player.stats);
        out.add("held " + ParityData.canon(player.hand) + " offhand " + ParityData.canon(player.offhand)
            + " using " + (player.using == null ? "-" : player.using));
        var respawn = player.getRespawnConfig();
        out.add("respawn " + (respawn == null ? "-" : respawn.respawnData().dimension().identifier() + " " + VanillaBlockUpdateOracle.pos(respawn.respawnData().pos())));
        return out;
    }

    /**
     * ServerPlayer.findRespawnAndUseSpawnBlock for a respawn point at `pos`
     * (a respawn anchor, in the Nether or not): the position, the yaw that
     * looks at the block, and the charge used (a block change).
     */
    static List<String> respawnAt(VanillaBlockUpdateOracle.World w, BlockPos pos, JsonObject op) throws Exception {
        var config = new net.minecraft.server.level.ServerPlayer.RespawnConfig(
            net.minecraft.world.level.storage.LevelData.RespawnData.of(w.dimension(), pos,
                op.has("yaw") ? op.get("yaw").getAsFloat() : 0.0F, 0.0F), op.get("forced").getAsBoolean());
        Method find = net.minecraft.server.level.ServerPlayer.class.getDeclaredMethod("findRespawnAndUseSpawnBlock",
            ServerLevel.class, net.minecraft.server.level.ServerPlayer.RespawnConfig.class, boolean.class);
        find.setAccessible(true);
        var found = (Optional<?>) find.invoke(null, w, config, op.get("consume").getAsBoolean());
        if (found.isEmpty()) return List.of("respawn none");
        Object angle = found.get();
        Method position = angle.getClass().getDeclaredMethod("position");
        Method yaw = angle.getClass().getDeclaredMethod("yaw");
        position.setAccessible(true);
        yaw.setAccessible(true);
        var p = (net.minecraft.world.phys.Vec3) position.invoke(angle);
        return List.of("respawn " + num(p.x) + " " + num(p.y) + " " + num(p.z) + " " + Float.floatToRawIntBits((float) yaw.invoke(angle)));
    }

    /**
     * The player of an "attack" case: a `ServerPlayer` with a place, a fall
     * and a velocity, a stand-in connection and abilities.
     */
    static final class Attacker extends net.minecraft.server.level.ServerPlayer {
        VanillaBlockUpdateOracle.World world;
        boolean creative, fallFlying;
        ItemStack hand;
        List<String> stats;
        Attacker() {
            super(null, null, null, null);
        }
        @Override public ServerLevel level() { return world; }
        @Override public boolean hasInfiniteMaterials() { return creative; }
        @Override public boolean isCreative() { return creative; }
        @Override public boolean isSpectator() { return false; }
        @Override public boolean isFallFlying() { return fallFlying; }
        @Override public net.minecraft.world.phys.Vec3 getKnownMovement() { return getDeltaMovement(); }
        @Override public EntityType<?> getType() { return EntityTypes.PLAYER; }
        @Override public net.minecraft.core.Holder<EntityType<?>> typeHolder() { return EntityTypes.PLAYER.builtInRegistryHolder(); }
        @Override public ItemStack getItemBySlot(EquipmentSlot slot) { return slot == EquipmentSlot.MAINHAND ? hand : ItemStack.EMPTY; }
        @Override public ItemStack getItemInHand(net.minecraft.world.InteractionHand h) {
            return h == net.minecraft.world.InteractionHand.MAIN_HAND ? hand : ItemStack.EMPTY;
        }
        @Override public void setItemInHand(net.minecraft.world.InteractionHand h, ItemStack stack) {
            if (h == net.minecraft.world.InteractionHand.MAIN_HAND) hand = stack;
        }
        @Override public void awardStat(net.minecraft.stats.Stat<?> stat, int count) {
            stats.add("stat " + User.key(stat) + " " + BuiltInRegistries.STAT_TYPE.getKey(stat.getType()) + " " + count);
        }
    }

    /** The synched data `Entity`'s constructor defines (a player's air supply is 300). */
    @SuppressWarnings({"unchecked", "rawtypes"})
    static void defineData(Entity e) throws Exception {
        var builder = new net.minecraft.network.syncher.SynchedEntityData.Builder(e);
        Object[][] base = {{"DATA_SHARED_FLAGS_ID", (byte) 0}, {"DATA_AIR_SUPPLY_ID", 300}, {"DATA_CUSTOM_NAME_VISIBLE", false},
            {"DATA_CUSTOM_NAME", Optional.empty()}, {"DATA_SILENT", false}, {"DATA_NO_GRAVITY", false}, {"DATA_POSE", Pose.STANDING},
            {"DATA_TICKS_FROZEN", 0}};
        for (Object[] d : base) {
            Field f = Entity.class.getDeclaredField((String) d[0]);
            f.setAccessible(true);
            builder.define((net.minecraft.network.syncher.EntityDataAccessor) f.get(null), d[1]);
        }
        Method define = Entity.class.getDeclaredMethod("defineSynchedData", net.minecraft.network.syncher.SynchedEntityData.Builder.class);
        define.setAccessible(true);
        define.invoke(e, builder);
        VanillaBlockUpdateOracle.set(Entity.class, e, "entityData", builder.build());
    }

    /** A connection to no client: the motion packets a smash sends go nowhere. */
    static final class Silent extends net.minecraft.server.network.ServerGamePacketListenerImpl {
        Silent() { super(null, null, null, null); }
        @Override public void send(net.minecraft.network.protocol.Packet<?> packet) { }
    }

    /**
     * A player hit on the first target with the case's item, as
     * `Player.attack` and `itemAttackInteraction` run it after the attack
     * strength scaled the base damage to `base`: the item's damage bonus
     * (`MaceItem.getAttackDamageBonus`) and damage source, the target's real
     * `hurtServer` (armor with the weapon's Breach), then `hurtEnemy` (the
     * mace's smash), the post-attack enchantments (Wind Burst) and
     * `postHurtEnemy` (the fall reset and the weapon's wear). Every target's
     * health and velocity, the player's velocity, fall, impulse and the
     * statistics follow.
     */
    static List<String> attack(VanillaBlockUpdateOracle.World w, JsonObject op, List<Entity> targets) throws Exception {
        if (VanillaBlockUpdateOracle.Server.players == null) {
            VanillaBlockUpdateOracle.Server.players = (Ops) VanillaBlockUpdateOracle.allocate.invoke(VanillaBlockUpdateOracle.unsafe, Ops.class);
        }
        Attacker player = (Attacker) VanillaBlockUpdateOracle.allocate.invoke(VanillaBlockUpdateOracle.unsafe, Attacker.class);
        player.world = w;
        player.creative = op.get("creative").getAsBoolean();
        player.fallFlying = op.get("fall_flying").getAsBoolean();
        player.hand = stack(op.get("item").getAsString());
        player.stats = new ArrayList<>();
        var abilities = new net.minecraft.world.entity.player.Abilities();
        abilities.invulnerable = abilities.instabuild = abilities.mayfly = player.creative;
        abilities.flying = op.get("flying").getAsBoolean();
        VanillaBlockUpdateOracle.set(net.minecraft.world.entity.player.Player.class, player, "abilities", abilities);
        var advancements = (net.minecraft.server.PlayerAdvancements) VanillaBlockUpdateOracle.allocate.invoke(VanillaBlockUpdateOracle.unsafe, net.minecraft.server.PlayerAdvancements.class);
        VanillaBlockUpdateOracle.set(net.minecraft.server.PlayerAdvancements.class, advancements, "activeTriggers", new HashMap<>());
        VanillaBlockUpdateOracle.set(net.minecraft.server.level.ServerPlayer.class, player, "advancements", advancements);
        player.connection = (Silent) VanillaBlockUpdateOracle.allocate.invoke(VanillaBlockUpdateOracle.unsafe, Silent.class);
        defineData(player);
        player.setId(w.getNextEntityId());
        VanillaBlockUpdateOracle.set(LivingEntity.class, player, "attributes",
            new net.minecraft.world.entity.ai.attributes.AttributeMap(net.minecraft.world.entity.player.Player.createAttributes().build()));
        player.setUUID(new UUID(0L, 1L));
        VanillaBlockUpdateOracle.set(net.minecraft.world.entity.player.Player.class, player, "gameProfile",
            new com.mojang.authlib.GameProfile(new UUID(0L, 1L), "Player"));
        var p = op.getAsJsonArray("player");
        var at = new net.minecraft.world.phys.Vec3(p.get(0).getAsDouble(), p.get(1).getAsDouble(), p.get(2).getAsDouble());
        VanillaBlockUpdateOracle.set(Entity.class, player, "position", at);
        VanillaBlockUpdateOracle.set(Entity.class, player, "blockPosition", BlockPos.containing(at));
        var standing = EntityTypes.PLAYER.getDimensions();
        VanillaBlockUpdateOracle.set(Entity.class, player, "dimensions", standing);
        VanillaBlockUpdateOracle.set(Entity.class, player, "eyeHeight", standing.eyeHeight());
        player.setBoundingBox(standing.makeBoundingBox(at));
        var m = op.getAsJsonArray("motion");
        player.setDeltaMovement(m.get(0).getAsDouble(), m.get(1).getAsDouble(), m.get(2).getAsDouble());
        player.fallDistance = op.get("fall").getAsDouble();
        if (op.has("impact")) {
            var i = op.getAsJsonArray("impact");
            player.currentImpulseImpactPos = new net.minecraft.world.phys.Vec3(i.get(0).getAsDouble(), i.get(1).getAsDouble(), i.get(2).getAsDouble());
        }
        // The player stands in the level, for the bursts that push it.
        w.entities.add(player);
        LivingEntity target = (LivingEntity) targets.get(0);
        ItemStack weapon = player.hand;
        var source = weapon.getDamageSource(player);
        float base = op.get("base").getAsFloat();
        float bonus = weapon.getItem().getAttackDamageBonus(target, base, source);
        boolean hurt = target.hurtServer(w, source, base + bonus);
        if (hurt) {
            boolean weaponHit = weapon.hurtEnemy(target, player);
            net.minecraft.world.item.enchantment.EnchantmentHelper.doPostAttackEffectsWithItemSource(w, target, source, weapon);
            if (weaponHit) weapon.postHurtEnemy(target, player);
        }
        w.entities.remove(player);
        List<String> out = new ArrayList<>(player.stats);
        out.add("attack bonus=" + num(bonus) + " source=" + source.typeHolder().getRegisteredName() + " hurt=" + hurt);
        for (int i = 0; i < targets.size(); i++) {
            Entity e = targets.get(i);
            var v = e.getDeltaMovement();
            String health = e instanceof LivingEntity living && !(e instanceof net.minecraft.world.entity.decoration.ArmorStand) ? num(living.getHealth()) : "-";
            // A stand the hit broke has left the level.
            if (e.isRemoved()) {
                out.add("V " + i + " gone");
                continue;
            }
            out.add("V " + i + " " + health + " " + num(v.x) + " " + num(v.y) + " " + num(v.z));
        }
        var v = player.getDeltaMovement();
        var impact = player.currentImpulseImpactPos;
        Field grace = LivingEntity.class.getDeclaredField("currentImpulseContextResetGraceTime");
        grace.setAccessible(true);
        Field extra = net.minecraft.server.level.ServerPlayer.class.getDeclaredField("spawnExtraParticlesOnFall");
        extra.setAccessible(true);
        out.add("P " + num(v.x) + " " + num(v.y) + " " + num(v.z) + " fall=" + num(player.fallDistance)
            + " impact=" + (impact == null ? "-" : num(impact.x) + "," + num(impact.y) + "," + num(impact.z))
            + " grace=" + grace.getInt(player) + " extra=" + extra.getBoolean(player));
        out.add("held " + ParityData.canon(player.hand));
        return out;
    }

    /** The level's block entity positions by x, y, z. */
    static List<BlockPos> sortedEntities(VanillaBlockUpdateOracle.World w) {
        List<BlockPos> out = new ArrayList<>(w.blockEntities == null ? List.of() : w.blockEntities.keySet());
        out.sort(Comparator.<BlockPos>comparingInt(q -> q.getX()).thenComparingInt(q -> q.getY()).thenComparingInt(q -> q.getZ()));
        return out;
    }

    /** A target's state after dispensing: removal, equipment and readiness for shearing. */
    static int orbCount(ExperienceOrb orb) {
        try {
            var f = ExperienceOrb.class.getDeclaredField("count");
            f.setAccessible(true);
            return f.getInt(orb);
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException(e);
        }
    }

    static String describe(int index, Entity e) {
        StringBuilder line = new StringBuilder("T " + index + " " + BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()));
        if (e.isRemoved()) line.append(" removed");
        if (e instanceof LivingEntity living) {
            for (EquipmentSlot slot : EquipmentSlot.values()) {
                ItemStack s = living.getItemBySlot(slot);
                if (!s.isEmpty()) line.append(' ').append(slot.getName()).append('=').append(BuiltInRegistries.ITEM.getKey(s.getItem())).append('x').append(s.getCount());
            }
        }
        boolean listed = e instanceof net.minecraft.world.entity.animal.sheep.Sheep || e instanceof net.minecraft.world.entity.animal.cow.MushroomCow
            || e instanceof net.minecraft.world.entity.monster.skeleton.Bogged || e instanceof net.minecraft.world.entity.animal.golem.SnowGolem
            || e instanceof net.minecraft.world.entity.animal.golem.CopperGolem;
        if (listed && e instanceof Shearable shearable) line.append(shearable.readyForShearing() ? " ready" : " unready");
        if (e instanceof net.minecraft.world.entity.animal.equine.AbstractChestedHorse chested && chested.hasChest()) line.append(" chest");
        if (e instanceof Leashable leashable && leashable.isLeashed()) line.append(" leashed");
        return line.toString();
    }
}
