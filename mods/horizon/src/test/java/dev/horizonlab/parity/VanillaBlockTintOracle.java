package dev.horizonlab.parity;

import com.google.gson.JsonParser;
import java.nio.file.Files;
import java.nio.file.Path;
import javax.imageio.ImageIO;
import net.minecraft.SharedConstants;
import net.minecraft.client.color.block.BlockColors;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.GrassColor;
import net.minecraft.world.level.block.Block;

/** Original adapter calling the pinned client's BlockTintSource.color(state). */
public final class VanillaBlockTintOracle {
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        var input = JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonObject();
        var root = Path.of(input.get("resources").getAsString());
        var image = ImageIO.read(root.resolve("assets/minecraft/textures/colormap/grass.png").toFile());
        GrassColor.init(image.getRGB(0, 0, 256, 256, null, 0, 256));
        var colors = BlockColors.createDefault();
        try (var output = Files.newBufferedWriter(Path.of(args[1]))) {
            for (var value : input.getAsJsonArray("cases")) {
                var row = value.getAsJsonArray();
                int stateId = row.get(0).getAsInt(), index = row.get(1).getAsInt();
                var state = Block.stateById(stateId);
                var source = index < 0 ? null : colors.getTintSource(state, index);
                int color = source == null ? -1 : source.color(state);
                output.write(stateId + " " + index + " " + color + "\n");
            }
        }
    }
}
