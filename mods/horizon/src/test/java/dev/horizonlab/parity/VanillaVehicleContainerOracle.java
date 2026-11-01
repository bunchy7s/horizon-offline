package dev.horizonlab.parity;

import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.UUID;
import com.mojang.authlib.GameProfile;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.MenuProvider;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntitySpawnReason;
import net.minecraft.world.entity.EntitySpawnRequest;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.inventory.AbstractContainerMenu;
import net.minecraft.world.item.component.ResolvableProfile;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.Level;

/**
 * Original oracle: the menu a container vehicle opens. Each input line is an
 * entity type id; EntityType.create makes it on the stand-in ServerLevel (which
 * only hands out entity ids), its MenuProvider.createMenu(id, inventory,
 * player) - the menu ServerPlayer.openMenu sends - gives the menu, and
 * getDisplayName (the vehicle's custom name or its type name) the title.
 * Prints the menu type, the container's slot count (without the player's 36)
 * and the title's name key.
 */
public final class VanillaVehicleContainerOracle {
    /** A stand-in server level: only the entity id counter is real. */
    static final class Lvl extends ServerLevel {
        int next = 1;

        Lvl() {
            super(null, null, null, null, null, null, false, 0L, List.of(), false);
        }

        @Override
        public int getNextEntityId() {
            return next++;
        }

        @Override
        public RegistryAccess registryAccess() {
            return ParityData.access;
        }

        @Override
        public MinecraftServer getServer() {
            return null;
        }

        @Override
        public net.minecraft.world.flag.FeatureFlagSet enabledFeatures() {
            return net.minecraft.world.flag.FeatureFlags.REGISTRY.allFlags();
        }
    }

    /** A player the menu constructor may ask isSpectator() (a loot roll). */
    static final class FakePlayer extends Player {
        FakePlayer(Lvl level) {
            super(level, new GameProfile(new UUID(1L, 2L), "oracle"));
        }

        @Override
        public GameType gameMode() {
            return GameType.SURVIVAL;
        }

        @Override
        public ResolvableProfile getProfile() {
            return ResolvableProfile.createResolved(this.getGameProfile());
        }

        @Override
        public boolean isSpectator() {
            return false;
        }
    }

    static Object unsafe;
    static Method allocate;
    static Method putObject;
    static Method objectFieldOffset;

    static void reflect() throws Exception {
        var unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        unsafe = unsafeField.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        putObject = unsafe.getClass().getMethod("putObject", Object.class, long.class, Object.class);
        objectFieldOffset = unsafe.getClass().getMethod("objectFieldOffset", Field.class);
    }

    static Lvl level() throws Exception {
        Lvl level = (Lvl) allocate.invoke(unsafe, Lvl.class);
        Field registryAccess = net.minecraft.world.level.Level.class.getDeclaredField("registryAccess");
        registryAccess.setAccessible(true);
        long offset = (long) objectFieldOffset.invoke(unsafe, registryAccess);
        putObject.invoke(unsafe, level, offset, ParityData.access);
        return level;
    }

    static String menu(Lvl level, String id) throws Exception {
        EntityType<?> type = BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse(id));
        if (type == null) {
            return "error";
        }
        Entity entity = type.create(level, new EntitySpawnRequest(EntitySpawnReason.LOAD, true));
        if (!(entity instanceof MenuProvider provider)) {
            return "error";
        }
        Inventory inventory = new Inventory(null, null);
        AbstractContainerMenu menu = provider.createMenu(0, inventory, new FakePlayer(level));
        if (menu == null) {
            return "error";
        }
        String name = entity.getCustomName() == null ? entity.getType().getDescriptionId() : "custom";
        return BuiltInRegistries.MENU.getKey(menu.getType()) + " " + (menu.slots.size() - 36) + " " + name;
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        reflect();
        Lvl level = level();
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            for (String line : (Iterable<String>) lines::iterator) {
                try {
                    out.println(menu(level, line.strip()));
                } catch (Exception e) {
                    System.err.println(line + ": " + e);
                    e.printStackTrace();
                    out.println("error");
                }
            }
        }
    }
}
