package dev.horizonlab.parity;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import net.minecraft.core.component.DataComponentType;
import net.minecraft.core.component.DataComponents;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.item.TooltipFlag;
import net.minecraft.world.item.component.TooltipDisplay;

/** The name and attribute lines returned for a diamond sword's tooltip. */
public final class VanillaTooltipDisplayOracle {
    public static void main(String[] args) throws Exception {
        ParityData.init();
        var context = Item.TooltipContext.of(ParityData.lookup);
        var output = new ArrayList<String>();
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            if (line.startsWith("name\t")) {
                var stack = ParityData.stack("{id:\"minecraft:stick\",count:1,components:{\"minecraft:custom_name\":" + line.substring(5) + "}}");
                var result = new StringBuilder("name\t");
                stack.getStyledHoverName().visit((style, text) -> {
                    text.codePoints().forEach(c -> result.append(c).append(',')
                        .append(style.getColor() == null ? 0xffffff : style.getColor().getValue()).append(',')
                        .append(style.isBold()).append(',').append(style.isItalic()).append(',')
                        .append(style.isUnderlined()).append(',').append(style.isStrikethrough()).append(',')
                        .append(style.isObfuscated()).append('|'));
                    return java.util.Optional.empty();
                }, net.minecraft.network.chat.Style.EMPTY);
                output.add(result.toString());
                continue;
            }
            String[] fields = line.split("\t");
            boolean hideTooltip = Boolean.parseBoolean(fields[0]);
            boolean creative = Boolean.parseBoolean(fields[1]);
            boolean hideAttributes = Boolean.parseBoolean(fields[2]);
            var hidden = new LinkedHashSet<DataComponentType<?>>();
            if (hideAttributes) hidden.add(DataComponents.ATTRIBUTE_MODIFIERS);
            var stack = new ItemStack(Items.DIAMOND_SWORD, 1);
            stack.set(DataComponents.TOOLTIP_DISPLAY, new TooltipDisplay(hideTooltip, hidden));
            TooltipFlag flag = creative ? TooltipFlag.Default.NORMAL.asCreative() : TooltipFlag.Default.NORMAL;
            var tooltip = stack.getTooltipLines(context, null, flag);
            output.add((!tooltip.isEmpty()) + "\t" + (tooltip.size() > 1));
        }
        Files.write(Path.of(args[1]), output);
    }
}
