package dev.horizonlab.parity;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.List;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.world.entity.EntitySpawnReason;
import net.minecraft.world.entity.boss.enderdragon.EnderDragon;
import net.minecraft.world.entity.boss.enderdragon.EnderDragonPart;
import net.minecraft.world.entity.boss.enderdragon.phases.EnderDragonPhase;
import net.minecraft.world.entity.monster.Enderman;

/** Calls the real 26.3 dragon contact methods; no entity or physics ticks intervene. */
public final class VanillaDragonContactsOracle {
    static JsonArray run(ServerLevel level) {
        try {
            Method wing = EnderDragon.class.getDeclaredMethod("knockBack", ServerLevel.class, List.class);
            Method head = EnderDragon.class.getDeclaredMethod("hurt", ServerLevel.class, List.class);
            Field body = EnderDragon.class.getDeclaredField("body");
            wing.setAccessible(true);
            head.setAccessible(true);
            body.setAccessible(true);
            JsonArray rows = new JsonArray();
            String[] names = {"flying", "sitting", "two_wings", "team", "head", "hurt_dragon", "recent_victim"};
            for (String name : names) {
                EnderDragon dragon = (EnderDragon) BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse("minecraft:ender_dragon")).create(level, EntitySpawnReason.COMMAND);
                dragon.setPos(0, 0, 0);
                ((EnderDragonPart) body.get(dragon)).setPos(0, 0, 0);
                dragon.getPhaseManager().setPhase(name.equals("sitting") ? EnderDragonPhase.HOVERING : EnderDragonPhase.HOLDING_PATTERN);
                Enderman mob = (Enderman) BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse("minecraft:enderman")).create(level, EntitySpawnReason.COMMAND);
                mob.setPos(1, 0, 0);
                mob.setNoAi(true);
                mob.tickCount = 100;
                if (name.equals("recent_victim")) {
                    mob.tickCount = 99;
                    mob.setLastHurtByMob(dragon);
                    mob.tickCount = 100;
                }
                if (name.equals("team")) {
                    var board = level.getScoreboard();
                    var team = board.addPlayerTeam("contact");
                    team.setAllowFriendlyFire(false);
                    board.addPlayerToTeam(dragon.getScoreboardName(), team);
                    board.addPlayerToTeam(mob.getScoreboardName(), team);
                }
                if (!name.equals("hurt_dragon")) {
                    if (name.equals("head")) head.invoke(dragon, level, List.of(mob));
                    else {
                        wing.invoke(dragon, level, List.of(mob));
                        if (name.equals("two_wings")) wing.invoke(dragon, level, List.of(mob));
                    }
                }
                JsonObject row = new JsonObject();
                row.addProperty("case", name);
                row.addProperty("health", mob.getHealth());
                JsonArray velocity = new JsonArray();
                velocity.add(mob.getDeltaMovement().x);
                velocity.add(mob.getDeltaMovement().y);
                velocity.add(mob.getDeltaMovement().z);
                row.add("velocity", velocity);
                row.addProperty("revenge", mob.getLastHurtByMob() == dragon);
                rows.add(row);
                if (name.equals("team")) level.getScoreboard().removePlayerTeam(level.getScoreboard().getPlayerTeam("contact"));
                mob.discard();
                dragon.discard();
            }
            return rows;
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException(e);
        }
    }
}
