package dev.horizonlab.parity;

import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.io.PrintWriter;
import java.lang.reflect.Proxy;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.Map;
import net.minecraft.commands.arguments.blocks.BlockStateParser;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.level.LevelAccessor;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.ChestBlock;
import net.minecraft.world.level.block.DoubleBlockCombiner;
import net.minecraft.world.level.block.EntityBlock;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.ChestBlockEntity;
import net.minecraft.world.level.block.state.BlockState;

/**
 * Original oracle: which chests join into a double chest. Each case line is
 * JSON: a chest's block state ("state") at the origin and another block state
 * ("other") at the horizontal offset "at" ([dx, dz]); each holds the block
 * entity its block makes (EntityBlock.newBlockEntity). Runs
 * DoubleBlockCombiner.combineWithNeigbour as ChestBlock.combine(state, level,
 * pos, true) calls it (the block's blockEntityType, ChestBlock.getBlockType,
 * ChestBlock.getConnectedDirection, FACING; nothing blocked) on a stand-in
 * level, and prints "double" with the first (right) half's position and the
 * second's, or "single".
 */
public final class VanillaChestPairOracle {
    static BlockState parse(String s) throws Exception {
        return BlockStateParser.parseForBlock(BuiltInRegistries.BLOCK, s, false).blockState();
    }

    static String xz(BlockPos p) {
        return p.getX() + "," + p.getZ();
    }

    static String pair(JsonObject c) throws Exception {
        BlockState state = parse(c.get("state").getAsString());
        BlockState other = parse(c.get("other").getAsString());
        var at = c.getAsJsonArray("at");
        BlockPos pos = BlockPos.ZERO;
        BlockPos otherPos = pos.offset(at.get(0).getAsInt(), 0, at.get(1).getAsInt());
        Map<BlockPos, BlockState> blocks = new HashMap<>();
        Map<BlockPos, BlockEntity> entities = new HashMap<>();
        blocks.put(pos, state);
        blocks.put(otherPos, other);
        for (var e : blocks.entrySet()) {
            if (e.getValue().getBlock() instanceof EntityBlock block) {
                BlockEntity entity = block.newBlockEntity(e.getKey(), e.getValue());
                if (entity != null) entities.put(e.getKey(), entity);
            }
        }
        LevelAccessor level = (LevelAccessor) Proxy.newProxyInstance(
            LevelAccessor.class.getClassLoader(), new Class<?>[] {LevelAccessor.class}, (proxy, method, args) -> {
                switch (method.getName()) {
                    case "getBlockState": return blocks.getOrDefault(((BlockPos) args[0]).immutable(), Blocks.AIR.defaultBlockState());
                    case "getBlockEntity":
                        if (args.length == 1) return entities.get(((BlockPos) args[0]).immutable());
                        break;
                    default: break;
                }
                throw new UnsupportedOperationException(method.toString());
            });
        ChestBlock chest = (ChestBlock) state.getBlock();
        var result = DoubleBlockCombiner.combineWithNeigbour(chest.blockEntityType(), ChestBlock::getBlockType,
            ChestBlock::getConnectedDirection, ChestBlock.FACING, state, level, pos, (l, p) -> false);
        return result.apply(new DoubleBlockCombiner.Combiner<ChestBlockEntity, String>() {
            @Override public String acceptDouble(ChestBlockEntity first, ChestBlockEntity second) {
                return "double " + xz(first.getBlockPos()) + " " + xz(second.getBlockPos());
            }
            @Override public String acceptSingle(ChestBlockEntity single) { return "single"; }
            @Override public String acceptNone() { return "none"; }
        });
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            for (String line : (Iterable<String>) lines::iterator) {
                try {
                    out.println(pair(JsonParser.parseString(line).getAsJsonObject()));
                } catch (Exception e) {
                    System.err.println(line + ": " + e);
                    out.println("error");
                }
            }
        }
    }
}
