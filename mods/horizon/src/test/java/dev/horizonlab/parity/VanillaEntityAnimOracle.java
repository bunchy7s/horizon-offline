package dev.horizonlab.parity;

import com.google.gson.*;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import com.mojang.blaze3d.vertex.BufferBuilder;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import com.mojang.math.Axis;
import net.minecraft.SharedConstants;
import net.minecraft.client.model.EntityModel;
import net.minecraft.client.model.geom.LayerDefinitions;
import net.minecraft.client.model.geom.ModelLayerLocation;
import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.renderer.entity.state.EntityRenderState;
import net.minecraft.server.Bootstrap;

/**
 * Original oracle for the client's entity model animation: bakes each vanilla model layer, builds
 * its own render state class with the given walk/age/head/attack values plus optional extra
 * fields, runs the real setupAnim and writes every part's pose.
 *
 * Input lines: "layer-key ModelClass|case-json" where case-json is an object of state field
 * name to value. Output lines: {layer, case, parts:{path:[x,y,z,xRot,yRot,zRot,xScale,yScale,zScale,visible]},
 * vertices:{path:{n, f, p, d}}}.
 *
 * `vertices` renders the posed model the way ModelPart.render does (translateAndRotate on a
 * PoseStack, then Cube.compile: PoseStack.Pose.transformNormal once per polygon and
 * VertexConsumer.addVertex/setNormal per vertex) under LivingEntityRenderer's root transform:
 * rotateDegrees(YP, 180 - "@rootYaw"), scale(-1, -1, 1), scale("@rootScale"),
 * translate(0, -1.501, 0). Per drawn part: n = each vertex's normal as the signed bytes
 * BufferBuilder.setNormal writes (its own normalIntValue), f = each polygon's float normal before
 * packing, p = sum of x + 2y + 3z over the posed positions, d = all its vertices are one point.
 */
public final class VanillaEntityAnimOracle {
    private static Field cubes;
    private static Field children;

    @SuppressWarnings("unchecked")
    private static void dump(String path, ModelPart part, JsonObject out) throws Exception {
        var pose = new JsonArray();
        for (float f : new float[] {part.x, part.y, part.z, part.xRot, part.yRot, part.zRot, part.xScale, part.yScale, part.zScale})
            pose.add(f);
        pose.add(part.visible && !part.skipDraw ? 1 : 0);
        int polys = 0;
        double sum = 0;
        for (ModelPart.Cube cube : (List<ModelPart.Cube>) cubes.get(part)) {
            for (ModelPart.Polygon polygon : cube.polygons) {
                polys++;
                for (ModelPart.Vertex v : polygon.vertices()) {
                    float u = Float.isFinite(v.u()) ? v.u() : 0.0F, w = Float.isFinite(v.v()) ? v.v() : 0.0F;
                    sum += v.x() + 2.0 * v.y() + 3.0 * v.z() + 5.0 * u + 7.0 * w;
                }
            }
        }
        pose.add(polys);
        pose.add(sum);
        out.add(path, pose);
        for (var entry : ((Map<String, ModelPart>) children.get(part)).entrySet())
            dump(path.isEmpty() ? entry.getKey() : path + "/" + entry.getKey(), entry.getValue(), out);
    }

    private static Class<?> findClass(String simple) throws Exception {
        for (String pkg : new String[] {"net.minecraft.client.model"}) {
            // Search subpackages by trying every known class location through the jar index.
            for (String name : CLASS_NAMES) {
                if (name.endsWith("." + simple) || name.equals(simple)) return Class.forName(name);
            }
        }
        throw new ClassNotFoundException(simple);
    }

    private static final List<String> CLASS_NAMES = new ArrayList<>();

    private static Method normalIntValue;

    /** Records what Cube.compile hands its VertexConsumer: each vertex's position and normal. */
    private static final class Capture implements VertexConsumer {
        final List<float[]> vertices = new ArrayList<>();
        private float[] current;

        @Override
        public VertexConsumer addVertex(float x, float y, float z) {
            current = new float[] {x, y, z, Float.NaN, Float.NaN, Float.NaN};
            vertices.add(current);
            return this;
        }

        @Override
        public VertexConsumer setNormal(float x, float y, float z) {
            current[3] = x;
            current[4] = y;
            current[5] = z;
            return this;
        }

        @Override public VertexConsumer setColor(int r, int g, int b, int a) { return this; }
        @Override public VertexConsumer setColor(int color) { return this; }
        @Override public VertexConsumer setUv(float u, float v) { return this; }
        @Override public VertexConsumer setUv1(int u, int v) { return this; }
        @Override public VertexConsumer setUv2(int u, int v) { return this; }
        @Override public VertexConsumer setUv3(float u, float v) { return this; }
        @Override public VertexConsumer setLineWidth(float width) { return this; }

        JsonObject json() throws Exception {
            var packed = new JsonArray();
            var floats = new JsonArray();
            double sum = 0;
            boolean degenerate = true;
            float[] first = vertices.get(0);
            for (int i = 0; i < vertices.size(); i++) {
                float[] v = vertices.get(i), corner = vertices.get(i - i % 4);
                for (int c = 3; c < 6; c++) packed.add((byte) normalIntValue.invoke(null, v[c]));
                if (i % 4 == 0) {
                    for (int c = 3; c < 6; c++) floats.add(Float.isFinite(v[c]) ? v[c] : 0.0F);
                } else if (!Arrays.equals(v, 3, 6, corner, 3, 6)) {
                    throw new IllegalStateException("normal changes inside a polygon");
                }
                sum += v[0] + 2.0 * v[1] + 3.0 * v[2];
                degenerate &= v[0] == first[0] && v[1] == first[1] && v[2] == first[2];
            }
            var o = new JsonObject();
            o.add("n", packed);
            o.add("f", floats);
            o.addProperty("p", sum);
            o.addProperty("d", degenerate);
            return o;
        }
    }

    /** ModelPart.render, keeping each part's vertices apart under its path. */
    @SuppressWarnings("unchecked")
    private static void render(String path, ModelPart part, PoseStack poseStack, JsonObject out) throws Exception {
        if (!part.visible) return;
        var cubeList = (List<ModelPart.Cube>) cubes.get(part);
        var kids = (Map<String, ModelPart>) children.get(part);
        if (cubeList.isEmpty() && kids.isEmpty()) return;
        poseStack.pushPose();
        part.translateAndRotate(poseStack);
        if (!part.skipDraw) {
            var capture = new Capture();
            for (ModelPart.Cube cube : cubeList)
                cube.compile(poseStack.last(), capture, 0xF000F0, net.minecraft.client.renderer.texture.OverlayTexture.NO_OVERLAY, -1);
            if (!capture.vertices.isEmpty()) out.add(path, capture.json());
        }
        for (var entry : kids.entrySet())
            render(path.isEmpty() ? entry.getKey() : path + "/" + entry.getKey(), entry.getValue(), poseStack, out);
        poseStack.popPose();
    }

    private static Class<?> raw(java.lang.reflect.Type t, Map<java.lang.reflect.TypeVariable<?>, java.lang.reflect.Type> env) {
        if (t instanceof Class<?> c) return c;
        if (t instanceof java.lang.reflect.ParameterizedType p) return raw(p.getRawType(), env);
        if (t instanceof java.lang.reflect.TypeVariable<?> v) {
            var bound = env.get(v);
            return raw(bound != null ? bound : v.getBounds()[0], env);
        }
        throw new IllegalArgumentException("cannot resolve " + t);
    }

    /** The render state class: the model's `S`, resolved through its generic superclasses. */
    private static Class<?> stateClass(Class<?> model) {
        var env = new HashMap<java.lang.reflect.TypeVariable<?>, java.lang.reflect.Type>();
        Class<?> current = model;
        for (var v : current.getTypeParameters()) env.put(v, raw(v.getBounds()[0], env));
        while (current != net.minecraft.client.model.Model.class) {
            var sup = current.getGenericSuperclass();
            if (sup instanceof java.lang.reflect.ParameterizedType p) {
                var params = ((Class<?>) p.getRawType()).getTypeParameters();
                for (int i = 0; i < params.length; i++) env.put(params[i], raw(p.getActualTypeArguments()[i], env));
                current = (Class<?>) p.getRawType();
            } else {
                current = (Class<?>) sup;
            }
        }
        return raw(net.minecraft.client.model.Model.class.getTypeParameters()[0], env);
    }

    private static Field field(Class<?> type, String name) {
        for (Class<?> c = type; c != null; c = c.getSuperclass()) {
            try {
                return c.getDeclaredField(name);
            } catch (NoSuchFieldException e) {
            }
        }
        return null;
    }

    private static void set(Object state, String name, JsonElement value) throws Exception {
        boolean optional = name.startsWith("?") || Set.of("walkAnimationPos", "walkAnimationSpeed", "yRot", "xRot", "isBaby", "swingAnimation", "ageScale", "ageInTicks", "entityId", "yHeadRot", "yBodyRot", "eatTick", "flapOffset", "interest").contains(name);
        if (name.startsWith("?")) name = name.substring(1);
        Field f = field(state.getClass(), name);
        if (f == null && optional) return;
        if (f == null) throw new IllegalArgumentException("no field " + name + " on " + state.getClass());
        f.setAccessible(true);
        Class<?> t = f.getType();
        if (t == float.class) f.setFloat(state, value.getAsFloat());
        else if (t == int.class) f.setInt(state, value.getAsInt());
        else if (t == double.class) f.setDouble(state, value.getAsDouble());
        else if (t == boolean.class) f.setBoolean(state, value.getAsBoolean());
        else if (t.isEnum()) {
            for (Object c : t.getEnumConstants()) if (((Enum<?>) c).name().equals(value.getAsString())) f.set(state, c);
        } else if (t == net.minecraft.world.item.ItemStack.class) {
            // value = an item id. The item components are not bound outside a level, so the stack
            // carries none: models only test that it is not empty.
            var item = net.minecraft.core.registries.BuiltInRegistries.ITEM.wrapAsHolder(net.minecraft.core.registries.BuiltInRegistries.ITEM.getValue(
                net.minecraft.resources.Identifier.parse(value.getAsString())));
            var ctor = net.minecraft.world.item.ItemStack.class.getDeclaredConstructor(net.minecraft.core.Holder.class, int.class,
                net.minecraft.core.component.PatchedDataComponentMap.class);
            ctor.setAccessible(true);
            f.set(state, ctor.newInstance(item, 1, new net.minecraft.core.component.PatchedDataComponentMap(
                net.minecraft.core.component.DataComponentMap.EMPTY)));
        } else if (t.getSimpleName().equals("AnimationState")) {
            // value = age in ticks since start (negative = stopped)
            var a = (net.minecraft.world.entity.AnimationState) f.get(state);
            a.stop();
            if (value.getAsJsonPrimitive().isNumber()) a.start(value.getAsInt());
        } else throw new IllegalArgumentException("unsupported type " + t + " for " + name);
    }

    /** Render-state values vanilla derives from entity state; the case gives the entity-side input. */
    private static void hooks(EntityRenderState state, JsonObject kase) throws Exception {
        if (state instanceof net.minecraft.client.renderer.entity.state.BatRenderState bat && kase.has("@batTicks"))
            batHistory(bat, kase.getAsJsonArray("@batTicks"));
        if (state instanceof net.minecraft.client.renderer.entity.state.SheepRenderState sheep && kase.has("eatTick")) {
            // Sheep.getHeadEatPositionScale / getHeadEatAngleScale at partial tick 0.
            int t = kase.get("eatTick").getAsInt();
            float a = 0.0F;
            sheep.headEatPositionScale = t <= 0 ? 0.0F : t >= 4 && t <= 36 ? 1.0F : t < 4 ? (t - a) / 4.0F : -(t - 40 - a) / 4.0F;
            if (t > 4 && t <= 36) {
                float scale = (t - 4 - a) / 32.0F;
                sheep.headEatAngleScale = (float) (Math.PI / 5) + 0.21991149F * net.minecraft.util.Mth.sin(scale * 28.7F);
            } else {
                sheep.headEatAngleScale = t > 0 ? (float) (Math.PI / 5) : sheep.xRot * (float) (Math.PI / 180.0);
            }
        }
        if (state instanceof net.minecraft.client.renderer.entity.state.WolfRenderState wolf && kase.has("interest"))
            wolf.headRollAngle = kase.get("interest").getAsFloat() * 0.15F * (float) Math.PI;
        if (state instanceof net.minecraft.client.renderer.entity.state.PhantomRenderState phantom && kase.has("flapOffset"))
            phantom.flapTime = kase.get("flapOffset").getAsFloat() + phantom.ageInTicks;
    }

    /** Only the server's flags are supplied; the shipped Bat starts/stops its own clocks. */
    private static final class AnimationBat extends net.minecraft.world.entity.ambient.Bat {
        int flags;
        AnimationBat() { super(null, null); }
        @Override public boolean isResting() { return (flags & 1) != 0; }
    }

    private static void batHistory(net.minecraft.client.renderer.entity.state.BatRenderState state, JsonArray ticks) throws Exception {
        // Allocate without a world, as the other controlled entity oracles do.
        var allocator = Class.forName("sun.misc.Unsafe");
        var field = allocator.getDeclaredField("theUnsafe");
        field.setAccessible(true);
        var bat = (AnimationBat) allocator.getMethod("allocateInstance", Class.class).invoke(field.get(null), AnimationBat.class);
        var type = net.minecraft.world.entity.ambient.Bat.class;
        // The animation states are final fields that allocateInstance left null: only an accessible Field may set them.
        for (var name : new String[] {"flyAnimationState", "restAnimationState"}) {
            var animation = type.getDeclaredField(name);
            animation.setAccessible(true);
            animation.set(bat, new net.minecraft.world.entity.AnimationState());
        }
        var setup = type.getDeclaredMethod("setupAnimationStates");
        setup.setAccessible(true);
        for (var flags : ticks) {
            bat.tickCount++;
            bat.flags = flags.getAsInt();
            setup.invoke(bat);
        }
        state.isResting = bat.isResting();
        state.flyAnimationState.copyFrom(bat.flyAnimationState);
        state.restAnimationState.copyFrom(bat.restAnimationState);
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        cubes = ModelPart.class.getDeclaredField("cubes");
        cubes.setAccessible(true);
        children = ModelPart.class.getDeclaredField("children");
        children.setAccessible(true);
        normalIntValue = BufferBuilder.class.getDeclaredMethod("normalIntValue", float.class);
        normalIntValue.setAccessible(true);
        try (var zip = new java.util.zip.ZipFile(args[2])) {
            zip.stream().map(e -> e.getName()).filter(n -> n.startsWith("net/minecraft/client/model/") && n.endsWith(".class"))
                .forEach(n -> CLASS_NAMES.add(n.substring(0, n.length() - 6).replace('/', '.')));
        }
        var roots = LayerDefinitions.createRoots();
        var byName = new HashMap<String, ModelLayerLocation>();
        for (var key : roots.keySet()) byName.put(key.toString(), key);
        var gson = new Gson();
        var out = new StringBuilder();
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            if (line.isBlank()) continue;
            var halves = line.split("\\|", 2);
            var head = halves[0].split(" ");
            var layer = head[0];
            var kase = JsonParser.parseString(halves[1]).getAsJsonObject();
            var result = new JsonObject();
            result.addProperty("layer", layer);
            result.add("case", kase);
            try {
                var location = byName.get(layer);
                if (location == null) throw new IllegalArgumentException("no layer " + layer);
                var cls = findClass(head[1]);
                var baked = roots.get(location).bakeRoot();
                var model = (net.minecraft.client.model.Model<EntityRenderState>) cls.getConstructor(ModelPart.class).newInstance(baked);
                // IllusionerRenderer shows the hat IllagerModel hides.
                if (layer.equals("minecraft:illusioner#main")) ((net.minecraft.client.model.monster.illager.IllagerModel<?>) (Object) model).getHat().visible = true;
                var type = stateClass(cls);
                var state = (EntityRenderState) type.getDeclaredConstructor().newInstance();
                state.getClass();
                for (var entry : kase.entrySet())
                    if (!entry.getKey().startsWith("@")) set(state, entry.getKey(), entry.getValue());
                if (kase.has("@walkTicks")) {
                    var walk = new net.minecraft.world.entity.WalkAnimationState();
                    for (var speed : kase.getAsJsonArray("@walkTicks"))
                        walk.update(speed.getAsFloat(), 0.4F, 1.0F);
                    // LivingEntity.calculateEntityAnimation stops passengers/dead entities;
                    // LivingEntityRenderer.extractRenderState also zeros both fields.
                    if (kase.get("isPassenger").getAsBoolean() || !kase.get("@alive").getAsBoolean())
                        walk.stop();
                    float partial = kase.get("@partial").getAsFloat();
                    set(state, "walkAnimationPos", gson.toJsonTree(walk.position(partial)));
                    set(state, "walkAnimationSpeed", gson.toJsonTree(walk.speed(partial)));
                    result.add("walk", gson.toJsonTree(new float[]{walk.position(partial), walk.speed(partial)}));
                }
                hooks(state, kase);
                if (state instanceof net.minecraft.client.renderer.entity.state.ArmedEntityRenderState armed && armed.swingAnimation > 0)
                    armed.currentSwing = new net.minecraft.world.entity.LivingEntity.SwingDescription(net.minecraft.world.InteractionHand.MAIN_HAND, net.minecraft.world.item.component.SwingAnimation.DEFAULT, 6);
                model.setupAnim(state);
                if (kase.has("@partScale")) {
                    var override = kase.getAsJsonArray("@partScale");
                    var part = baked.createPartLookup().apply(override.get(0).getAsString());
                    if (part == null) throw new IllegalArgumentException("no part " + override.get(0));
                    part.xScale = override.get(1).getAsFloat();
                    part.yScale = override.get(2).getAsFloat();
                    part.zScale = override.get(3).getAsFloat();
                }
                var parts = new JsonObject();
                dump("", baked, parts);
                result.add("parts", parts);
                // LivingEntityRenderer.render/setupRotations, without the entity's position.
                var poseStack = new PoseStack();
                float yaw = kase.has("@rootYaw") ? kase.get("@rootYaw").getAsFloat() : 0.0F;
                poseStack.rotateDegrees(Axis.YP, 180.0F - yaw);
                poseStack.scale(-1.0F, -1.0F, 1.0F);
                if (kase.has("@rootScale")) {
                    var scale = kase.getAsJsonArray("@rootScale");
                    poseStack.scale(scale.get(0).getAsFloat(), scale.get(1).getAsFloat(), scale.get(2).getAsFloat());
                }
                poseStack.translate(0.0F, -1.501F, 0.0F);
                var vertices = new JsonObject();
                render("", baked, poseStack, vertices);
                result.add("vertices", vertices);
            } catch (Throwable error) {
                var cause = error.getCause() != null ? error.getCause() : error;
                result.addProperty("error", cause.toString());
            }
            out.append(gson.toJson(result)).append('\n');
        }
        Files.writeString(Path.of(args[1]), out.toString(), StandardCharsets.UTF_8);
    }
}
