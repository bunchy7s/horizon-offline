package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Options;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.gui.screens.inventory.*;
import net.minecraft.client.gui.screens.recipebook.*;
import net.minecraft.client.input.MouseButtonEvent;
import net.minecraft.client.input.MouseButtonInfo;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.core.NonNullList;
import net.minecraft.world.SimpleContainer;
import net.minecraft.world.entity.player.Abilities;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.inventory.*;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;

/**
 * Original oracle for the mouse handling of the survival and container
 * screens: the real AbstractContainerScreen.mouseClicked/mouseReleased of
 * ContainerScreen, HopperScreen, ShulkerBoxScreen and BeaconScreen (one slot
 * in the menu, the carried stack, the event's Shift modifiers and whether the
 * player has infinite materials from the case) recording every slotClicked
 * call they make, and the real hasClickedOutside of those screens,
 * MerchantScreen and the recipe book screens (InventoryScreen,
 * CraftingScreen, FurnaceScreen) with their RecipeBookComponent's visibility
 * and selected tab hover from the case.
 */
public final class VanillaContainerScreenOracle {
    static Object unsafe;
    static Method allocate;
    static final StringBuilder clicks = new StringBuilder();

    @SuppressWarnings("unchecked")
    static <T> T alloc(Class<T> type) throws Exception {
        return (T) allocate.invoke(unsafe, type);
    }

    static void put(Object target, Class<?> owner, String name, Object value) throws Exception {
        Field f = owner.getDeclaredField(name);
        long offset = (long) unsafe.getClass().getMethod("objectFieldOffset", Field.class).invoke(unsafe, f);
        Class<?> t = f.getType();
        String setter = t == int.class ? "putInt" : t == boolean.class ? "putBoolean" : "putObject";
        Class<?> param = t.isPrimitive() ? t : Object.class;
        unsafe.getClass().getMethod(setter, Object.class, long.class, param).invoke(unsafe, target, offset, value);
    }

    static void record(int slotId, int button, ContainerInput input) {
        clicks.append(" click ").append(slotId).append(' ').append(button).append(' ').append(input.name());
    }

    // The screens with slotClicked recording what the real mouse handling sends.
    public static final class Chest extends ContainerScreen {
        Chest() { super(null, null, null); }
        @Override protected void slotClicked(Slot slot, int slotId, int button, ContainerInput input) { record(slotId, button, input); }
    }

    public static final class Hopper extends HopperScreen {
        Hopper() { super(null, null, null); }
        @Override protected void slotClicked(Slot slot, int slotId, int button, ContainerInput input) { record(slotId, button, input); }
    }

    public static final class Shulker extends ShulkerBoxScreen {
        Shulker() { super(null, null, null); }
        @Override protected void slotClicked(Slot slot, int slotId, int button, ContainerInput input) { record(slotId, button, input); }
    }

    public static final class Beacon extends BeaconScreen {
        Beacon() { super(null, null, null); }
        @Override protected void slotClicked(Slot slot, int slotId, int button, ContainerInput input) { record(slotId, button, input); }
    }

    static AbstractContainerScreen<?> screen(String kind) throws Exception {
        return switch (kind) {
            case "chest" -> alloc(Chest.class);
            case "hopper" -> alloc(Hopper.class);
            case "shulker" -> alloc(Shulker.class);
            case "beacon" -> alloc(Beacon.class);
            case "merchant" -> alloc(MerchantScreen.class);
            case "inventory" -> alloc(InventoryScreen.class);
            case "crafting" -> alloc(CraftingScreen.class);
            case "furnace" -> alloc(FurnaceScreen.class);
            default -> throw new IllegalArgumentException(kind);
        };
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var u = Class.forName("sun.misc.Unsafe");
        Field theUnsafe = u.getDeclaredField("theUnsafe");
        theUnsafe.setAccessible(true);
        unsafe = theUnsafe.get(null);
        allocate = u.getMethod("allocateInstance", Class.class);
        Minecraft shell = alloc(Minecraft.class);
        put(shell, Minecraft.class, "fixerUpper", net.minecraft.util.datafix.DataFixers.getDataFixer());
        Field instance = Minecraft.class.getDeclaredField("instance");
        instance.setAccessible(true);
        instance.set(null, shell);
        Options options = new Options(shell, Files.createTempDirectory("opts").toFile());
        put(shell, Minecraft.class, "options", options);
        Method outside = AbstractContainerScreen.class.getDeclaredMethod("hasClickedOutside", double.class, double.class, int.class, int.class);
        outside.setAccessible(true);
        Method hovered = AbstractContainerScreen.class.getDeclaredMethod("getHoveredSlot", double.class, double.class);
        hovered.setAccessible(true);
        Field carried = AbstractContainerMenu.class.getDeclaredField("carried");
        carried.setAccessible(true);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            int n = 0;
            for (var line : (Iterable<String>) lines::iterator) {
                var c = JsonParser.parseString(line).getAsJsonObject();
                out.println("== " + n++);
                // mouseClicked's keyPickItem test asks the player whether it has
                // infinite materials (Player.hasInfiniteMaterials: the abilities'
                // instabuild), so the shell gets a player with just those.
                var player = alloc(LocalPlayer.class);
                var abilities = new Abilities();
                abilities.instabuild = c.has("creative") && c.get("creative").getAsBoolean();
                put(player, Player.class, "abilities", abilities);
                put(shell, Minecraft.class, "player", player);
                String kind = c.get("kind").getAsString();
                var screen = screen(kind);
                var image = c.getAsJsonArray("image");
                put(screen, AbstractContainerScreen.class, "leftPos", image.get(0).getAsInt());
                put(screen, AbstractContainerScreen.class, "topPos", image.get(1).getAsInt());
                put(screen, AbstractContainerScreen.class, "imageWidth", image.get(2).getAsInt());
                put(screen, AbstractContainerScreen.class, "imageHeight", image.get(3).getAsInt());
                put(screen, Screen.class, "minecraft", shell);
                put(screen, Screen.class, "children", new ArrayList<>());
                put(screen, AbstractContainerScreen.class, "quickCraftSlots", new HashSet<Slot>());
                put(screen, AbstractContainerScreen.class, "lastQuickMoved", ItemStack.EMPTY);
                if (c.has("book")) {
                    var book = c.getAsJsonArray("book");
                    var component = alloc(CraftingRecipeBookComponent.class);
                    put(component, RecipeBookComponent.class, "visible", book.get(0).getAsBoolean());
                    var tab = alloc(RecipeBookTabButton.class);
                    put(tab, net.minecraft.client.gui.components.AbstractWidget.class, "isHovered", book.get(1).getAsBoolean());
                    put(component, RecipeBookComponent.class, "selectedTab", tab);
                    put(screen, AbstractRecipeBookScreen.class, "recipeBookComponent", component);
                }
                var menu = alloc(ChestMenu.class);
                NonNullList<Slot> slots = NonNullList.create();
                var pos = c.getAsJsonArray("slot");
                var slot = new Slot(new SimpleContainer(1), 0, pos.get(0).getAsInt(), pos.get(1).getAsInt());
                slot.index = 7;
                slots.add(slot);
                put(menu, AbstractContainerMenu.class, "slots", slots);
                carried.set(menu, ItemStack.EMPTY);
                put(screen, AbstractContainerScreen.class, "menu", menu);
                int left = image.get(0).getAsInt(), top = image.get(1).getAsInt();
                for (var e : c.getAsJsonArray("events")) {
                    var ev = e.getAsJsonArray();
                    String name = ev.get(0).getAsString();
                    double x = ev.get(1).getAsDouble(), y = ev.get(2).getAsDouble();
                    var sb = new StringBuilder(name);
                    sb.append(' ').append(outside.invoke(screen, x, y, left, top));
                    if (!name.equals("point")) {
                        sb.append(' ').append(hovered.invoke(screen, x, y) != null);
                        carried.set(menu, ev.get(5).getAsBoolean() ? new ItemStack(Items.STONE, 5) : ItemStack.EMPTY);
                        var event = new MouseButtonEvent(x, y, new MouseButtonInfo(ev.get(3).getAsInt(), ev.get(4).getAsInt()));
                        clicks.setLength(0);
                        if (name.equals("press")) {
                            screen.mouseClicked(event, false);
                        } else {
                            screen.mouseReleased(event);
                        }
                        sb.append(clicks);
                    }
                    out.println(sb);
                }
            }
        }
        System.exit(0);
    }
}
