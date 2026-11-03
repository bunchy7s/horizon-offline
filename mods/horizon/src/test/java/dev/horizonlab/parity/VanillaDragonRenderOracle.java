package dev.horizonlab.parity;

import com.google.gson.Gson;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import net.minecraft.client.renderer.entity.EnderDragonRenderer;

/** Capture the real dragon death-ray triangles, without a window or GPU. */
public final class VanillaDragonRenderOracle {
    private static final class Capture implements VertexConsumer {
        final List<double[]> vertices = new ArrayList<>();
        @Override public VertexConsumer addVertex(float x, float y, float z) {
            vertices.add(new double[] {x, y, z, 0});
            return this;
        }
        @Override public VertexConsumer setColor(int color) {
            vertices.getLast()[3] = Integer.toUnsignedLong(color);
            return this;
        }
        @Override public VertexConsumer setColor(int r, int g, int b, int a) {
            return setColor(a << 24 | r << 16 | g << 8 | b);
        }
        @Override public VertexConsumer setUv(float u, float v) { return this; }
        @Override public VertexConsumer setUv1(int u, int v) { return this; }
        @Override public VertexConsumer setUv2(int u, int v) { return this; }
        @Override public VertexConsumer setUv3(float u, float v) { return this; }
        @Override public VertexConsumer setNormal(float x, float y, float z) { return this; }
        @Override public VertexConsumer setLineWidth(float width) { return this; }
    }

    public static void main(String[] args) throws Exception {
        Method rays = EnderDragonRenderer.class.getDeclaredMethod(
            "lambda$submitRays$0", float.class, PoseStack.Pose.class, VertexConsumer.class);
        rays.setAccessible(true);
        var cases = new ArrayList<List<double[]>>();
        for (float t : new float[] {0, .05F, .25F, .5F, .8F, .9F, .95F, 1}) {
            var capture = new Capture();
            rays.invoke(null, t, new PoseStack().last(), capture);
            cases.add(capture.vertices);
        }
        Files.writeString(Path.of(args[0]), new Gson().toJson(cases));
    }
}
