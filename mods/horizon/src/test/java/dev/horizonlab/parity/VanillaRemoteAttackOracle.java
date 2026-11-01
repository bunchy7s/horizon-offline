package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.function.Predicate;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.player.RemotePlayer;
import net.minecraft.core.RegistryAccess;
import net.minecraft.sounds.SoundEvent;
import net.minecraft.sounds.SoundSource;
import net.minecraft.world.damagesource.DamageSource;
import net.minecraft.world.damagesource.DamageSources;
import net.minecraft.world.effect.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.ai.attributes.AttributeMap;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.entity.monster.zombie.Zombie;
import net.minecraft.world.entity.player.Abilities;
import net.minecraft.world.entity.player.Input;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;

/**
 * Runs the real Player.attack on a LocalPlayer shell in a ClientLevel shell, as
 * MultiPlayerGameMode.attack does on the attacking client, and prints what it
 * leaves on the attacker: its velocity and sprint flag. The target is a
 * RemotePlayer (hurtClient is true) or a zombie (Entity.hurtClient is false).
 * Effects go through the real forceAddEffect, as ClientPacketListener adds them;
 * the attribute values are what the client's map holds. The setSprinting call
 * is recorded rather than run: its attribute and entity-data side is not what
 * this compares.
 */
public final class VanillaRemoteAttackOracle {
    static Object unsafe;
    static Method allocate;

    @SuppressWarnings("unchecked")
    static <T> T make(Class<T> type) throws Exception {
        return (T) allocate.invoke(unsafe, type);
    }

    static Field field(Class<?> type, String name) throws Exception {
        Field f = type.getDeclaredField(name);
        f.setAccessible(true);
        return f;
    }

    static double d(JsonElement v) {
        return Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(), 16));
    }

    static final class Client extends ClientLevel {
        DamageSources sources;
        RegistryAccess registries;

        Client() {
            super(null, null, null, null, 0, 0, null, false, 0L, 0);
        }

        @Override public boolean isClientSide() { return true; }
        @Override public DamageSources damageSources() { return sources; }
        @Override public RegistryAccess registryAccess() { return registries; }
        @Override public void playSound(Entity except, double x, double y, double z, SoundEvent sound, SoundSource source, float volume, float pitch) {}
        @Override public <T extends Entity> List<T> getEntitiesOfClass(Class<T> type, AABB bounds, Predicate<? super T> filter) { return List.of(); }
    }

    static final class Attacker extends LocalPlayer {
        AttributeMap map;
        ItemStack hand;
        boolean sprint;
        boolean spin;

        Attacker() {
            super(null, null, null, null, null, Input.EMPTY, false, null, null);
        }

        @Override public AttributeMap getAttributes() { return map; }
        @Override public ItemStack getMainHandItem() { return hand; }
        @Override public ItemStack getItemBySlot(EquipmentSlot slot) { return slot == EquipmentSlot.MAINHAND ? hand : ItemStack.EMPTY; }
        @Override public boolean isSprinting() { return sprint; }
        @Override public void setSprinting(boolean value) { sprint = value; }
        @Override public boolean isAutoSpinAttack() { return spin; }
        @Override public boolean onGround() { return true; }
    }

    static final class PlayerTarget extends RemotePlayer {
        PlayerTarget() {
            super(null, null);
        }

        @Override public float getHealth() { return 20.0F; }
        @Override public void knockback(double power, double x, double z, DamageSource source, float scale, boolean flag) {}
        @Override public void knockback(double power, double x, double z, DamageSource source, float scale) {}
    }

    static final class MobTarget extends Zombie {
        MobTarget() {
            super((Level) null);
        }

        @Override public float getHealth() { return 20.0F; }
        @Override public void knockback(double power, double x, double z, DamageSource source, float scale, boolean flag) {}
        @Override public void knockback(double power, double x, double z, DamageSource source, float scale) {}
    }

    static <T extends Entity> T entity(Class<T> type, EntityType<?> kind, Client level) throws Exception {
        T e = make(type);
        field(Entity.class, "type").set(e, kind);
        field(Entity.class, "level").set(e, level);
        field(Entity.class, "deltaMovement").set(e, Vec3.ZERO);
        field(Entity.class, "position").set(e, new Vec3(2.0, 64.0, 0.0));
        return e;
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var u = Class.forName("sun.misc.Unsafe");
        unsafe = field(u, "theUnsafe").get(null);
        allocate = u.getMethod("allocateInstance", Class.class);
        Client level = make(Client.class);
        level.registries = ParityData.access;
        level.sources = new DamageSources(ParityData.access);
        Map<String, ItemStack> items = new HashMap<>();
        var ticker = field(LivingEntity.class, "attackStrengthTicker");
        var spinDamage = field(LivingEntity.class, "autoSpinAttackDmg");
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            for (var line : (Iterable<String>) lines::iterator) {
                var c = JsonParser.parseString(line).getAsJsonObject();
                Attacker player = entity(Attacker.class, EntityTypes.PLAYER, level);
                field(Player.class, "abilities").set(player, new Abilities());
                field(LivingEntity.class, "activeEffects").set(player, new HashMap<>());
                player.map = new AttributeMap(Player.createAttributes().build());
                player.map.getInstance(Attributes.ATTACK_SPEED).setBaseValue(d(c.get("speed")));
                player.map.getInstance(Attributes.ATTACK_DAMAGE).setBaseValue(d(c.get("damage")));
                player.map.getInstance(Attributes.ATTACK_KNOCKBACK).setBaseValue(d(c.get("knockback")));
                String item = c.get("item").getAsString();
                if (!items.containsKey(item)) items.put(item, ParityData.stack(item));
                player.hand = items.get(item).copy();
                player.sprint = c.get("sprint").getAsBoolean();
                player.spin = !c.get("spin").isJsonNull();
                if (player.spin) spinDamage.setFloat(player, c.get("spin").getAsFloat());
                ticker.setInt(player, c.get("ticker").getAsInt());
                if (!c.get("effect").isJsonNull()) {
                    boolean weakness = c.get("effect").getAsString().equals("weakness");
                    player.forceAddEffect(new MobEffectInstance(weakness ? MobEffects.WEAKNESS : MobEffects.STRENGTH, 1000, 0), null);
                }
                var v = c.getAsJsonArray("vel");
                player.setDeltaMovement(new Vec3(d(v.get(0)), d(v.get(1)), d(v.get(2))));
                float strength = player.getAttackStrengthScale(0.5F);
                Entity target = c.get("target").getAsString().equals("player")
                    ? entity(PlayerTarget.class, EntityTypes.PLAYER, level)
                    : entity(MobTarget.class, EntityTypes.ZOMBIE, level);
                player.attack(target);
                Vec3 after = player.getDeltaMovement();
                out.printf("%08x %016x %016x %016x %s%n", Float.floatToRawIntBits(strength), Double.doubleToRawLongBits(after.x),
                    Double.doubleToRawLongBits(after.y), Double.doubleToRawLongBits(after.z), player.sprint);
            }
        }
        System.exit(0);
    }
}
