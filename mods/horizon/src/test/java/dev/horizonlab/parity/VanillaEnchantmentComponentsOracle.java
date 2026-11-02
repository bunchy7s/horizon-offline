package dev.horizonlab.parity;

import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import net.minecraft.core.component.DataComponents;
import net.minecraft.nbt.TagParser;
import net.minecraft.util.ProblemReporter;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.enchantment.Enchantment;
import net.minecraft.world.item.enchantment.ItemEnchantments;
import net.minecraft.world.level.storage.TagValueInput;

/** Original oracle for persistent enchantment decoding and translated level names. */
public final class VanillaEnchantmentComponentsOracle {
    private static String describe(ItemEnchantments enchantments) {
        var rows = new ArrayList<String>();
        for (var entry : enchantments.entrySet()) {
            var holder = entry.getKey();
            int level = entry.getIntValue();
            rows.add(holder.unwrapKey().orElseThrow().identifier() + "=" + level + "="
                + Enchantment.getFullname(holder, level).getString());
        }
        Collections.sort(rows);
        return String.join("|", rows);
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8))) {
            for (String line : Files.readAllLines(Path.of(args[0]), StandardCharsets.UTF_8)) {
                String[] fields = line.split("\t", 2);
                var tag = TagParser.parseCompoundFully(
                    "{id:'minecraft:diamond_sword',count:1,components:" + fields[1] + "}");
                // Item loading keeps a codec's partial value: invalid entries disappear,
                // while the valid enchantments and the other components survive.
                var stack = TagValueInput.create(ProblemReporter.DISCARDING, ParityData.lookup, tag)
                    .read(ItemStack.MAP_CODEC).orElseThrow();
                out.println(fields[0] + "\t" + stack.getDamageValue() + "\t"
                    + describe(stack.getOrDefault(DataComponents.ENCHANTMENTS, ItemEnchantments.EMPTY)) + "\t"
                    + describe(stack.getOrDefault(DataComponents.STORED_ENCHANTMENTS, ItemEnchantments.EMPTY)));
            }
        }
    }
}
