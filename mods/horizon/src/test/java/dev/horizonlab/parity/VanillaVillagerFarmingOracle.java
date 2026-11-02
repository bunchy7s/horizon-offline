package dev.horizonlab.parity;

import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.SharedConstants;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.CropBlock;
import net.minecraft.world.level.block.FarmlandBlock;

/** HarvestFarmland's class and maturity checks, including non-CropBlock age properties. */
public final class VanillaVillagerFarmingOracle {
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        try (var out = Files.newBufferedWriter(Path.of(args[0]))) {
            for (String name : new String[] {"wheat", "carrots", "potatoes", "beetroots",
                    "torchflower_crop", "pitcher_crop", "cocoa", "farmland", "stone"}) {
                var block = BuiltInRegistries.BLOCK.getValue(Identifier.parse("minecraft:" + name));
                for (var state : block.getStateDefinition().getPossibleStates()) {
                    boolean crop = block instanceof CropBlock;
                    boolean ripe = block instanceof CropBlock c && c.isMaxAge(state);
                    out.write(Block.getId(state) + " " + crop + " " + ripe + " " + (block instanceof FarmlandBlock));
                    out.newLine();
                }
            }
        }
    }
}
