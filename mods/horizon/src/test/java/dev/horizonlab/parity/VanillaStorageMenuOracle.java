package dev.horizonlab.parity;

import java.io.PrintWriter;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.inventory.AbstractContainerMenu;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.block.EntityBlock;
import net.minecraft.world.level.block.entity.BaseContainerBlockEntity;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;

/**
 * Original oracle: the menu a storage block's block entity opens. Each input
 * line is a block id; its default state's block entity
 * (EntityBlock.newBlockEntity, marked removed so that opening counts no
 * opener and needs no level) builds its menu with
 * BaseContainerBlockEntity.createMenu(id, inventory), the menu a server sends
 * in ClientboundOpenScreenPacket. Prints the menu's MenuType id and whether
 * its first slot accepts a shulker box (Slot.mayPlace).
 */
public final class VanillaStorageMenuOracle {
    static String menu(String id, Method createMenu) throws Exception {
        BlockState state = BuiltInRegistries.BLOCK.getValue(Identifier.parse(id)).defaultBlockState();
        BlockEntity entity = ((EntityBlock) state.getBlock()).newBlockEntity(BlockPos.ZERO, state);
        entity.setRemoved();
        Inventory inventory = new Inventory(null, null);
        var menu = (AbstractContainerMenu) createMenu.invoke((BaseContainerBlockEntity) entity, 0, inventory);
        boolean accepts = menu.getSlot(0).mayPlace(new ItemStack(Items.SHULKER_BOX));
        return BuiltInRegistries.MENU.getKey(menu.getType()) + (accepts ? " accepts" : " refuses") + " shulker_box";
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        Method createMenu = BaseContainerBlockEntity.class.getDeclaredMethod("createMenu", int.class, Inventory.class);
        createMenu.setAccessible(true);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            for (String line : (Iterable<String>) lines::iterator) {
                try {
                    out.println(menu(line.strip(), createMenu));
                } catch (Exception e) {
                    System.err.println(line + ": " + e);
                    out.println("error");
                }
            }
        }
    }
}
