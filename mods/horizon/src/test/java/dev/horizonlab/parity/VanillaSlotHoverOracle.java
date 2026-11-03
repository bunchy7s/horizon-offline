package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import net.minecraft.client.gui.screens.inventory.AbstractContainerScreen;
import net.minecraft.core.NonNullList;
import net.minecraft.world.SimpleContainer;
import net.minecraft.world.inventory.*;

/** The actual AbstractContainerScreen.getHoveredSlot, including its padded boundaries. */
public final class VanillaSlotHoverOracle {
    public static void main(String[] args) throws Exception {
        ParityData.init();
        Class<?> u = Class.forName("sun.misc.Unsafe");
        Field field = u.getDeclaredField("theUnsafe");
        field.setAccessible(true);
        VanillaContainerScreenOracle.unsafe = field.get(null);
        VanillaContainerScreenOracle.allocate = u.getMethod("allocateInstance", Class.class);
        Method hovered = AbstractContainerScreen.class.getDeclaredMethod("getHoveredSlot", double.class, double.class);
        hovered.setAccessible(true);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1])))) {
            for (String line : Files.readAllLines(Path.of(args[0]))) {
                JsonObject c = JsonParser.parseString(line).getAsJsonObject();
                var screen = VanillaContainerScreenOracle.alloc(VanillaContainerScreenOracle.Chest.class);
                var menu = VanillaContainerScreenOracle.alloc(ChestMenu.class);
                NonNullList<Slot> slots = NonNullList.create();
                for (var entry : c.getAsJsonArray("slots")) {
                    var s = entry.getAsJsonArray();
                    Slot slot = new Slot(new SimpleContainer(1), 0, s.get(1).getAsInt(), s.get(2).getAsInt());
                    slot.index = s.get(0).getAsInt();
                    slots.add(slot);
                }
                VanillaContainerScreenOracle.put(menu, AbstractContainerMenu.class, "slots", slots);
                VanillaContainerScreenOracle.put(screen, AbstractContainerScreen.class, "menu", menu);
                for (var point : c.getAsJsonArray("points")) {
                    var p = point.getAsJsonArray();
                    Slot slot = (Slot) hovered.invoke(screen, p.get(0).getAsDouble(), p.get(1).getAsDouble());
                    out.println(slot == null ? -1 : slot.index);
                }
            }
        }
    }
}
