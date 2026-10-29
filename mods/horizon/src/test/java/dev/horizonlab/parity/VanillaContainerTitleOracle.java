package dev.horizonlab.parity;

import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Optional;
import net.minecraft.core.BlockPos;
import net.minecraft.core.component.DataComponentPatch;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.nbt.TagParser;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.ComponentSerialization;
import net.minecraft.resources.Identifier;
import net.minecraft.world.MenuProvider;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.ChestBlock;
import net.minecraft.world.level.block.DoubleBlockCombiner;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.ChestBlockEntity;
import net.minecraft.world.level.block.state.BlockState;

/**
 * Original oracle: the title a container block's menu opens with
 * (ClientboundOpenScreenPacket.title). Each case line is JSON: a block and the
 * saved NBT of its block entity ("nbt"), with an optional item component patch
 * ("patch", SNBT) the block was placed from (BlockEntity.applyComponentsFromItemStack,
 * as BlockItem.updateBlockEntityComponents runs it); or, for a double chest, the
 * saved NBT of its first (right) and second (left) halves ("first", "second").
 * Prints the menu provider's getDisplayName as canonical NBT: a single block
 * entity's own (BaseContainerBlockEntity.getName), or ChestBlock's
 * MENU_PROVIDER_COMBINER's for the two halves.
 */
public final class VanillaContainerTitleOracle {
    static final BlockPos POS = new BlockPos(0, 64, 0);

    static BlockEntity load(BlockState state, BlockPos pos, String snbt) throws Exception {
        BlockEntity entity = BlockEntity.loadStatic(pos, state, TagParser.parseCompoundFully(snbt), ParityData.access);
        if (entity == null) throw new IllegalStateException("no block entity: " + snbt);
        return entity;
    }

    @SuppressWarnings("unchecked")
    static Component doubleChest(ChestBlockEntity first, ChestBlockEntity second) throws Exception {
        Field field = ChestBlock.class.getDeclaredField("MENU_PROVIDER_COMBINER");
        field.setAccessible(true);
        var combiner = (DoubleBlockCombiner.Combiner<ChestBlockEntity, Optional<MenuProvider>>) field.get(null);
        return combiner.acceptDouble(first, second).orElseThrow().getDisplayName();
    }

    static Component title(JsonObject c) throws Exception {
        BlockState state = BuiltInRegistries.BLOCK.getValue(Identifier.parse(c.get("block").getAsString())).defaultBlockState();
        if (c.has("first")) {
            var first = (ChestBlockEntity) load(state, POS, c.get("first").getAsString());
            var second = (ChestBlockEntity) load(state, POS.west(), c.get("second").getAsString());
            return doubleChest(first, second);
        }
        BlockEntity entity = load(state, POS, c.get("nbt").getAsString());
        if (c.has("patch")) {
            var patch = DataComponentPatch.CODEC.parse(ParityData.ops, TagParser.parseCompoundFully(c.get("patch").getAsString())).getOrThrow();
            ItemStack item = new ItemStack(state.getBlock().asItem());
            item.applyComponents(patch);
            entity.applyComponentsFromItemStack(item);
        }
        return ((MenuProvider) entity).getDisplayName();
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            for (String line : (Iterable<String>) lines::iterator) {
                try {
                    Component title = title(JsonParser.parseString(line).getAsJsonObject());
                    out.println(ParityData.canon(ComponentSerialization.CODEC.encodeStart(ParityData.ops, title).getOrThrow()));
                } catch (Exception e) {
                    System.err.println(line + ": " + e);
                    out.println("error");
                }
            }
        }
    }
}
