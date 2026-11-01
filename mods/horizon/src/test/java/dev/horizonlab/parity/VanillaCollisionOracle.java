package dev.horizonlab.parity;

import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.nbt.*;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.*;
import net.minecraft.world.phys.shapes.*;

/** Exports private runtime data locally and invokes actual vanilla cached-shape queries. */
public final class VanillaCollisionOracle {
    static { SharedConstants.tryDetectVersion(); Bootstrap.bootStrap(); }
    private static final boolean REUSE = Boolean.getBoolean("horizon.prepare.reuse");
    private static Map<String,byte[]> generated;
    private static final Map<BlockState,Tag> encodedStates = new IdentityHashMap<>();
    private static final Map<VoxelShape,CompoundTag> encodedGrids = new IdentityHashMap<>();
    private static Tag encode(BlockState state) {
        if (!REUSE) return BlockState.FULL_CODEC.encodeStart(NbtOps.INSTANCE,state).getOrThrow();
        return encodedStates.computeIfAbsent(state, s -> BlockState.FULL_CODEC.encodeStart(NbtOps.INSTANCE,s).getOrThrow()).copy();
    }
    private static final Field CACHE, COLLISION, DISCRETE;
    private static final Method INDEX;
    static {
        try {
            CACHE = BlockBehaviour.BlockStateBase.class.getDeclaredField("cache"); CACHE.setAccessible(true);
            COLLISION = CACHE.getType().getDeclaredField("collisionShape"); COLLISION.setAccessible(true);
            DISCRETE = VoxelShape.class.getDeclaredField("shape"); DISCRETE.setAccessible(true);
            INDEX = VoxelShape.class.getDeclaredMethod("findIndex", Direction.Axis.class, double.class); INDEX.setAccessible(true);
        } catch (ReflectiveOperationException e) { throw new ExceptionInInitializerError(e); }
    }
    private static VoxelShape cached(BlockState state) throws ReflectiveOperationException {
        Object cache = CACHE.get(state);
        if (cache == null) return null;
        VoxelShape shape = (VoxelShape) COLLISION.get(cache);
        if (state.getCollisionShape(EmptyBlockGetter.INSTANCE, BlockPos.ZERO) != shape) throw new AssertionError("cached dispatch differs");
        return shape;
    }
    private static CompoundTag grid(VoxelShape shape) throws ReflectiveOperationException {
        if (REUSE && encodedGrids.containsKey(shape)) return encodedGrids.get(shape).copy();
        CompoundTag tag = new CompoundTag();
        tag.putByte("canonical",(byte)(shape==Shapes.empty()?1:shape==Shapes.block()?2:0));
        tag.putByte("cube", (byte)(shape instanceof CubeVoxelShape ? 1 : 0));
        int cubeAxes=0;for(Direction.Axis axis:Direction.Axis.values())if(shape.getCoords(axis) instanceof CubePointRange)cubeAxes|=1<<axis.ordinal();tag.putByte("cube_axes",(byte)cubeAxes);
        for (Direction.Axis axis : Direction.Axis.values()) {
            var coords = shape.getCoords(axis); long[] bits = new long[coords.size()];
            for (int i = 0; i < bits.length; i++) bits[i] = Double.doubleToRawLongBits(coords.getDouble(i));
            tag.putLongArray(axis.getName(), bits);
        }
        DiscreteVoxelShape discrete = (DiscreteVoxelShape) DISCRETE.get(shape);
        tag.putIntArray("discrete_min", Arrays.stream(Direction.Axis.values()).mapToInt(discrete::firstFull).toArray());
        tag.putIntArray("discrete_max", Arrays.stream(Direction.Axis.values()).mapToInt(discrete::lastFull).toArray());
        byte[] bits = new byte[discrete.getXSize() * discrete.getYSize() * discrete.getZSize()];
        for (int x = 0; x < discrete.getXSize(); x++) for (int y = 0; y < discrete.getYSize(); y++) for (int z = 0; z < discrete.getZSize(); z++) bits[(x * discrete.getYSize() + y) * discrete.getZSize() + z] = (byte)(discrete.isFull(x, y, z) ? 1 : 0);
        tag.putByteArray("occupied", bits);
        if (REUSE) encodedGrids.put(shape, tag.copy());
        return tag;
    }
    public static CompoundTag occlusionDefinitions() throws Exception {
        var method=BlockBehaviour.class.getDeclaredMethod("getOcclusionShape",BlockState.class);method.setAccessible(true);
        var states=new ListTag();var shapes=new ListTag();var ids=new IdentityHashMap<VoxelShape,Integer>();int[] mapping=new int[Block.BLOCK_STATE_REGISTRY.size()];int index=0;
        for(var state:Block.BLOCK_STATE_REGISTRY) {
            states.add(encode(state));
            var shape=(VoxelShape)method.invoke(state.getBlock(),state);Integer id=ids.get(shape);
            if(id==null){id=shapes.size();ids.put(shape,id);shapes.add(grid(shape));}
            mapping[index++]=id;
        }
        var data=new CompoundTag();data.putInt("DataVersion",SharedConstants.getCurrentVersion().dataVersion().version());data.put("states",states);data.put("shapes",shapes);data.putIntArray("cached_shapes",mapping);return data;
    }
    private static void write(CompoundTag tag, Path path) throws IOException {
        try (DataOutputStream out = new DataOutputStream(Files.newOutputStream(path))) { NbtIo.write(tag, out); }
    }
    private static void inputValues(PrintWriter out, double... values) { for (double value : values) out.printf(" %016x", Double.doubleToRawLongBits(value)); }
    private static void generate(Path base) throws Exception {
        if (REUSE && generated != null) {
            for (var file : generated.entrySet()) Files.write(base.resolve(file.getKey()), file.getValue());
            return;
        }
        ListTag states = new ListTag(), shapes = new ListTag(); CompoundTag defaults = new CompoundTag();
        Map<VoxelShape,Integer> ids = new IdentityHashMap<>();Set<CompoundTag> grids=new HashSet<>(); int[] mapping = new int[Block.BLOCK_STATE_REGISTRY.size()]; int cachedCount = 0;
        for (BlockState state : Block.BLOCK_STATE_REGISTRY) {
            int id = states.size();
            if (Block.BLOCK_STATE_REGISTRY.getId(state) != id) throw new AssertionError("non-dense states");
            Tag encoded = encode(state); states.add(encoded);
            if (state == state.getBlock().defaultBlockState()) defaults.putInt(((CompoundTag)encoded).getStringOr("id", ""), id);
            VoxelShape shape = cached(state);
            if (shape == null) mapping[id] = -1;
            else {
                cachedCount++; CompoundTag data = grid(shape);var geometry=data.copy();geometry.remove("canonical");grids.add(geometry); Integer shapeId = ids.get(shape);
                if (shapeId == null) { shapeId = shapes.size(); ids.put(shape, shapeId); shapes.add(data); }
                mapping[id] = shapeId;
            }
        }
        CompoundTag registry = new CompoundTag(); registry.putInt("DataVersion", SharedConstants.getCurrentVersion().dataVersion().version()); registry.put("states", states); registry.put("defaults", defaults);
        write(registry, base.resolve("block-registry.nbt"));
        CompoundTag collision = new CompoundTag(); collision.putInt("DataVersion", SharedConstants.getCurrentVersion().dataVersion().version()); collision.put("states", states); collision.put("shapes", shapes); collision.putIntArray("cached_shapes", mapping);
        write(collision, base.resolve("cached-collision.nbt"));
        // The verification inputs (62 MB, printf'd) are the parity harness's; preparing reads none.
        if (!REUSE) writeInputs(base, mapping);
        else {
            // Consumers can overwrite block-registry.nbt; preserve the original bytes.
            generated = new HashMap<>();
            for (String file : List.of("block-registry.nbt", "cached-collision.nbt"))
                generated.put(file, Files.readAllBytes(base.resolve(file)));
        }
        System.out.printf("HORIZON_COLLISION_RESULT {\"registry_states\":%d,\"cached_states\":%d,\"uncached_states\":%d,\"unique_shapes\":%d,\"unique_grids\":%d,\"queries\":%d}%n", mapping.length,cachedCount,mapping.length-cachedCount,shapes.size(),grids.size(),cachedCount*6);
    }
    private static void writeInputs(Path base, int[] mapping) throws IOException {
        Random random = new Random(0x484f52495a4f4eL);
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(base.resolve("inputs.txt")))) {
            for (int id = 0; id < mapping.length; id++) {
                if (mapping[id] < 0) continue;
                for (int scenario = 0; scenario < 6; scenario++) {
                    BlockPos offset = switch(scenario % 3) { case 0 -> BlockPos.ZERO; case 1 -> new BlockPos(-17,-64,-1); default -> new BlockPos(30_000_000,64,-30_000_000); };
                    double[] origin = {offset.getX(),offset.getY(),offset.getZ()}; double[] lower = new double[3], upper = new double[3], start = new double[3], end = new double[3], point = new double[3];
                    for (int axis = 0; axis < 3; axis++) {
                        lower[axis] = origin[axis] + (scenario < 3 ? -.6 + axis * .4 : random.nextDouble() * 4 - 2);
                        upper[axis] = lower[axis] + (axis == 1 ? 1.8 : .6);
                        start[axis] = origin[axis] + (scenario < 3 ? (axis == 0 ? -1 : .5) : random.nextDouble() * 3 - 1);
                        end[axis] = origin[axis] + (scenario < 3 ? (axis == 0 ? 2 : .5) : random.nextDouble() * 3 - 1);
                        point[axis] = origin[axis] + random.nextDouble() * 3 - 1;
                    }
                    out.print(id); out.printf(" %d %d %d",offset.getX(),offset.getY(),offset.getZ());
                    inputValues(out, lower); inputValues(out, upper); inputValues(out, start); inputValues(out, end); inputValues(out, point);
                    inputValues(out, scenario == 4 ? 1e-8 : scenario == 5 ? -1e-7 : random.nextDouble() * 4 - 2); out.println();
                }
            }
        }
    }
    private static void values(PrintWriter out, String label, double... values) { out.print(label); for (double value : values) out.printf(" %016x",Double.doubleToLongBits(value)); out.println(); }
    private static void box(PrintWriter out, String label, AABB b) { values(out,label,b.minX,b.minY,b.minZ,b.maxX,b.maxY,b.maxZ); }
    private static void hit(PrintWriter out, BlockHitResult h) {
        if (h == null) out.println("hit none"); else {Vec3 v=h.getLocation(); out.printf("hit %d %d %016x %016x %016x%n",h.getDirection().ordinal(),h.isInside()?1:0,Double.doubleToLongBits(v.x),Double.doubleToLongBits(v.y),Double.doubleToLongBits(v.z));}
    }
    private static final class Input {
        String[] words; int index;
        Input(String line) {words=line.split(" ");}
        int integer() {return Integer.parseInt(words[index++]);}
        double number() {return Double.longBitsToDouble(Long.parseUnsignedLong(words[index++],16));}
        Vec3 vector() {return new Vec3(number(),number(),number());}
    }
    private static void verify(Path base, String side) throws Exception {
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(base.resolve("vanilla-"+side+".txt")))) {
            IdentityHashMap<VoxelShape,Integer> identities=new IdentityHashMap<>();
            for (BlockState state : Block.BLOCK_STATE_REGISTRY) {
                VoxelShape shape=cached(state); int id=Block.BLOCK_STATE_REGISTRY.getId(state);
                if (shape==null) {out.printf("state %d uncached%n",id); continue;}
                List<AABB> boxes=shape.toAabbs(); out.printf("state %d cached %d%n",id,boxes.size());
                out.printf("identity %d%n",identities.computeIfAbsent(shape,key->identities.size()));
                out.printf("canonical %b %b%n",shape==Shapes.empty(),shape==Shapes.block());
                int cubeAxes=0;for(Direction.Axis axis:Direction.Axis.values())if(shape.getCoords(axis) instanceof CubePointRange)cubeAxes|=1<<axis.ordinal();out.printf("cube %d %d%n",shape instanceof CubeVoxelShape?1:0,cubeAxes);
                values(out,"axis-extrema",shape.min(Direction.Axis.X),shape.min(Direction.Axis.Y),shape.min(Direction.Axis.Z),shape.max(Direction.Axis.X),shape.max(Direction.Axis.Y),shape.max(Direction.Axis.Z));
                if (shape.isEmpty()) out.println("bounds none"); else box(out,"bounds",shape.bounds());
                for (AABB b:boxes) box(out,"part",b);
            }
            try (BufferedReader input=Files.newBufferedReader(base.resolve("inputs.txt"))) {
                for (String line;(line=input.readLine())!=null;) {
                    Input in=new Input(line); int id=in.integer(); BlockPos offset=new BlockPos(in.integer(),in.integer(),in.integer());
                    AABB box=new AABB(in.vector(),in.vector()); Vec3 start=in.vector(),end=in.vector(),point=in.vector(); double amount=in.number();
                    VoxelShape local=cached(Block.BLOCK_STATE_REGISTRY.byId(id)); VoxelShape moved=local.move(offset);
                    values(out,"collide",moved.collide(Direction.Axis.X,box,amount),moved.collide(Direction.Axis.Y,box,amount),moved.collide(Direction.Axis.Z,box,amount));
                    hit(out,local.clip(start,end,offset));
                    Optional<Vec3> nearest=moved.closestPointTo(point); if(nearest.isEmpty()) out.println("closest none"); else values(out,"closest",nearest.get().x,nearest.get().y,nearest.get().z);
                    out.printf("index %d %d %d%n",INDEX.invoke(moved,Direction.Axis.X,point.x),INDEX.invoke(moved,Direction.Axis.Y,point.y),INDEX.invoke(moved,Direction.Axis.Z,point.z));
                    if(in.index!=in.words.length) throw new AssertionError("trailing input");
                }
            }
        }
    }
    public static void main(String[] args) throws Exception { Path base=Path.of(args[1]); if(args[0].equals("generate"))generate(base);else verify(base,args[0]); }
}
