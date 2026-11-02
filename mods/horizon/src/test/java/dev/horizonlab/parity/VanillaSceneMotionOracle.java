package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.lang.invoke.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.world.effect.*;
import net.minecraft.world.damagesource.*;
import net.minecraft.sounds.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.ai.attributes.*;
import net.minecraft.world.entity.player.*;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.border.WorldBorder;
import net.minecraft.world.level.gameevent.GameEvent;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.phys.*;
import net.minecraft.network.syncher.*;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.food.FoodData;
import net.minecraft.world.phys.shapes.*;
import static dev.horizonlab.parity.VanillaRegistryCollisionScanOracle.*;

/** Real server-player movement, landing and hurt over stored attributes, health
 * and chunks. Audio/network delivery, statistics/tracker/criterion listeners and
 * full death services remain controlled boundaries. Clock operations set up the
 * two damage-clock fields, not the complete enclosing native tick methods. */
public final class VanillaSceneMotionOracle {
    static Field blockPositionField,position,history,support,noBlocks,last,stuck,delay,trigger,attributeStorage,attributeDirty,attributeCached,impact,grace;
    static MethodHandle aiStep;
    static Method refreshAttributes;
    static EntityDataAccessor<Float> healthKey,absorptionKey;
    static Field healthDead,cooldown,lastHurt,noAction,lastDamage,lastStamp,foodExhaustion,foodTimer,storedSpeed;
    static Map<String,DamageSource> damageSources;
    static final class TrackedAttributes extends AttributeMap {
        final List<AttributeInstance> instances=new ArrayList<>();
        TrackedAttributes(AttributeSupplier supplier){super(supplier);}
        @Override public AttributeInstance getInstance(Holder<Attribute> key){
            var instance=super.getInstance(key);
            if(instance!=null&&!instances.contains(instance))instances.add(instance);
            return instance;
        }
    }
    static final class Waypoints extends net.minecraft.server.waypoints.ServerWaypointManager {
        Level level;
        @Override public void trackWaypoint(net.minecraft.world.waypoints.WaypointTransmitter actor){level.actor.events.add("waypoint transmit true");}
        @Override public void untrackWaypoint(net.minecraft.world.waypoints.WaypointTransmitter actor){level.actor.events.add("waypoint transmit false");}
        @Override public void addPlayer(ServerPlayer actor){level.actor.events.add("waypoint receive true");}
        @Override public void removePlayer(ServerPlayer actor){level.actor.events.add("waypoint receive false");}
    }
    static String sourceName(DamageSource source){return source==null?"none":source.typeHolder().getRegisteredName();}
    static final class Connection extends net.minecraft.server.network.ServerGamePacketListenerImpl {
        Connection(){super(null,null,null,null);}
        @Override public boolean hasClientLoaded(){return true;}
    }
    static final class Tracker extends CombatTracker {
        Actor actor;Tracker(){super(null);}
        @Override public void recordDamage(DamageSource source,float amount){actor.events.add(String.format("record-damage %08x",Float.floatToIntBits(amount)));}
    }
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static void write(Field f,Object o,Object value){try{f.set(o,value);}catch(Exception e){throw new AssertionError(e);}}
    static Field field(Class<?> c,String n)throws Exception{var f=c.getDeclaredField(n);f.setAccessible(true);return f;}
    static String blockPos(BlockPos p){return p.getX()+","+p.getY()+","+p.getZ();}
    static String vectorBits(Vec3 p){return String.format("%016x,%016x,%016x",Double.doubleToLongBits(p.x),Double.doubleToLongBits(p.y),Double.doubleToLongBits(p.z));}
    static String soundName(SoundEvent s){
        if(s==SoundEvents.PLAYER_SMALL_FALL)return "small";if(s==SoundEvents.PLAYER_BIG_FALL)return "big";
        if(s==SoundEvents.PLAYER_SPLASH)return "splash";if(s==SoundEvents.PLAYER_SPLASH_HIGH_SPEED)return "splash-high";if(s==SoundEvents.PLAYER_SWIM)return "swim";
        return VanillaBlockLandingOracle.sound(s);
    }
    static final class Sources extends DamageSources {
        DamageSource fall,stalagmite,starve,freeze,sweetBerryBush,drown;Sources(){super(null);}
        @Override public DamageSource drown(){return drown;}
        @Override public DamageSource freeze(){return freeze;}
        @Override public DamageSource sweetBerryBush(){return sweetBerryBush;}
        @Override public DamageSource starve(){return starve;}
        @Override public DamageSource fall(){return fall;}
        @Override public DamageSource stalagmite(){return stalagmite;}
    }
    static final class Rules extends net.minecraft.world.level.gamerules.GameRules {
        boolean regenerate=true;
        Rules(){super(net.minecraft.world.flag.FeatureFlagSet.of());}
        @SuppressWarnings("unchecked") @Override public <T> T get(net.minecraft.world.level.gamerules.GameRule<T> rule){
            return rule==net.minecraft.world.level.gamerules.GameRules.NATURAL_HEALTH_REGENERATION?(T)Boolean.valueOf(regenerate):super.get(rule);
        }
    }
    static final class Border extends WorldBorder {
        @Override public boolean isInsideCloseToBorder(Entity e,AABB b){return false;}
    }
    static final class Level extends net.minecraft.server.level.ServerLevel {
        Map<Long,Chunk> chunks;WorldBorder border;Actor actor;Sources sources;net.minecraft.util.RandomSource rng;
        net.minecraft.server.MinecraftServer server;
        Rules rules;net.minecraft.world.Difficulty difficulty;long time;Waypoints waypoints;
        @Override public net.minecraft.server.waypoints.ServerWaypointManager getWaypointManager(){return waypoints;}
        @Override public net.minecraft.world.level.gamerules.GameRules getGameRules(){return rules;}
        @Override public net.minecraft.world.Difficulty getDifficulty(){return difficulty;}
        static final net.minecraft.world.TickRateManager TICK_RATE=new net.minecraft.world.TickRateManager();
        @Override public net.minecraft.world.TickRateManager tickRateManager(){return TICK_RATE;}
        @Override public long getGameTime(){return time;}
        @Override public void broadcastDamageEvent(Entity actor,DamageSource source){this.actor.events.add("broadcast-damage");}
        Level(){super(null,null,null,null,null,null,false,0L,List.of(),false);}
        // LevelReader.containsAnyLiquid over the scene chunks (its section scan
        // needs level chunks): any fluid from the box's minimum block to its
        // maximum block, both included.
        @Override public boolean containsAnyLiquid(AABB b){
            for(var p:BlockPos.betweenClosed(BlockPos.containing(b.minX,b.minY,b.minZ),BlockPos.containing(b.maxX,b.maxY,b.maxZ)))if(!getBlockState(p).getFluidState().isEmpty())return true;
            return false;
        }
        @Override public boolean isClientSide(){return false;}
        @Override public BlockGetter getChunkForCollisions(int x,int z){return chunks.get(key(x,z));}
        @Override public BlockState getBlockState(BlockPos p){var c=chunks.get(key(p.getX()>>4,p.getZ()>>4));return c==null?Blocks.AIR.defaultBlockState():c.getBlockState(p);}
        @Override public FluidState getFluidState(BlockPos p){return getBlockState(p).getFluidState();}
        // No dimension layers: FAST_LAVA keeps its default (the Overworld's slow lava).
        static final net.minecraft.world.attribute.EnvironmentAttributeSystem ATTRIBUTES=net.minecraft.world.attribute.EnvironmentAttributeSystem.builder().build();
        @Override public net.minecraft.world.attribute.EnvironmentAttributeSystem environmentAttributes(){return ATTRIBUTES;}
        // EntityFluidInteraction.hasFluidAndLoaded: resident chunks only.
        @Override public net.minecraft.world.level.chunk.ChunkAccess getChunk(int x,int z,net.minecraft.world.level.chunk.status.ChunkStatus status,boolean load){return chunks.get(key(x,z));}
        // Entity.doWaterSplashEffect's particles (a client level adds them).
        @Override public void addParticle(net.minecraft.core.particles.ParticleOptions particle,double x,double y,double z,double xd,double yd,double zd){
            actor.events.add("splash-particle "+(particle==net.minecraft.core.particles.ParticleTypes.BUBBLE?"bubble":particle==net.minecraft.core.particles.ParticleTypes.SPLASH?"splash":"unknown")+" "+vectorBits(new Vec3(x,y,z))+" "+vectorBits(new Vec3(xd,yd,zd)));
        }
        @Override public List<VoxelShape> getEntityCollisions(Entity e,AABB b){return List.of();}
        @Override public List<Entity> getPushableEntities(Entity e,AABB b){return List.of();}
        @Override public List<Entity> getEntities(Entity e,AABB b){return e!=actor&&b.intersects(actor.getBoundingBox())?List.of(actor):List.of();}
        @Override public WorldBorder getWorldBorder(){return border;}
        @Override public DamageSources damageSources(){return sources;}
        @Override public net.minecraft.util.RandomSource getRandom(){return rng;}
        @Override public boolean isRainingAt(BlockPos p){return false;}
        @Override public net.minecraft.server.MinecraftServer getServer(){return server;}
        @Override public void gameEvent(Holder<GameEvent> event,Vec3 pos,GameEvent.Context context){if(event==GameEvent.HIT_GROUND)actor.events.add("hit "+vectorBits(pos)+" "+Block.getId(context.affectedState()));}
        @Override public void gameEvent(Holder<GameEvent> event,BlockPos pos,GameEvent.Context context){actor.events.add("event "+(event==GameEvent.BLOCK_CHANGE?"change":event==GameEvent.BLOCK_DESTROY?"destroy":"unknown")+" "+blockPos(pos)+" "+Block.getId(context.affectedState())+" "+(context.sourceEntity()!=null));}
        @Override public <T extends net.minecraft.core.particles.ParticleOptions> int sendParticles(T particle,double x,double y,double z,int count,double dx,double dy,double dz,double xs,double ys,double zs,net.minecraft.network.protocol.game.ClientboundLevelParticlesPacket.RandomizationType type){
            if(particle!=net.minecraft.core.particles.ParticleTypes.SNOWFLAKE||count!=1||dx!=0||dy!=0||dz!=0||type!=net.minecraft.network.protocol.game.ClientboundLevelParticlesPacket.RandomizationType.ALTERNATIVE)throw new AssertionError();
            actor.events.add("snowflake "+vectorBits(new Vec3(x,y,z))+" "+vectorBits(new Vec3(xs,ys,zs)));return 0;
        }
        @Override public <T extends net.minecraft.core.particles.ParticleOptions> int sendParticles(T particle,double x,double y,double z,int count,double dx,double dy,double dz,double speed){
            // Entity.sendBubbleColumnParticles.
            if(particle==net.minecraft.core.particles.ParticleTypes.SPLASH||particle==net.minecraft.core.particles.ParticleTypes.BUBBLE){
                actor.events.add(String.format("column-particle %s %s %d %s %016x",particle==net.minecraft.core.particles.ParticleTypes.BUBBLE?"bubble":"splash",vectorBits(new Vec3(x,y,z)),count,vectorBits(new Vec3(dx,dy,dz)),Double.doubleToLongBits(speed)));return 0;
            }
            var b=(net.minecraft.core.particles.BlockParticleOption)particle;if(dx!=0||dy!=0||dz!=0)throw new AssertionError();
            actor.events.add(String.format("particles %d %s %d %016x",Block.getId(b.getState()),vectorBits(new Vec3(x,y,z)),count,Double.doubleToLongBits(speed)));return 0;
        }
        @Override public void playSound(Entity e,BlockPos p,SoundEvent s,SoundSource source,float volume,float pitch){if(e!=null||source!=SoundSource.BLOCKS)throw new AssertionError();actor.events.add(String.format("sound %s %s %08x %08x",soundName(s),blockPos(p),Float.floatToIntBits(volume),Float.floatToIntBits(pitch)));}
        @Override public void broadcastEntityEvent(Entity e,byte event){actor.events.add("broadcast "+Byte.toUnsignedInt(event));}
        @Override public boolean setBlock(BlockPos p,BlockState state,int flags){chunks.get(key(p.getX()>>4,p.getZ()>>4)).write(p,Block.getId(state));actor.events.add("write "+blockPos(p)+" "+Block.getId(state)+" "+flags);return true;}
        @Override public boolean destroyBlock(BlockPos p,boolean drops){chunks.get(key(p.getX()>>4,p.getZ()>>4)).write(p,Block.getId(Blocks.AIR.defaultBlockState()));actor.events.add("destroy "+blockPos(p)+" "+drops);return true;}
        @Override public void levelEvent(int event,BlockPos p,int data){actor.events.add("level-event "+event+" "+blockPos(p)+" "+data);}
    }
    static final class Actor extends ServerPlayer {
        AttributeMap attrs;byte flags;boolean preparedInput;List<String> events;
        // Query overrides only while directly exercising the complete statistics
        // method; regular resident movement keeps its actual dry pose state.
        Integer statisticsFlags;
        Object sprintController;
        Pose scenePose=Pose.STANDING;
        @Override public Pose getPose(){return scenePose;}
        @Override public void setPose(Pose pose){if(scenePose!=pose){scenePose=pose;super.refreshDimensions();}}
        public boolean fitPose(Pose pose){return super.canPlayerFitWithinBlocksAndEntitiesWhen(pose);}
        public void poseTick(){updatePlayerPose();}
        public void attributeTick()throws Exception{refreshAttributes.invoke(this);}
        boolean statFlag(int bit){return (statisticsFlags&(1<<bit))!=0;}
        SynchedEntityData.DataItem<Float> healthItem,absorptionItem;Level sceneLevel;Tracker tracker;net.minecraft.server.PlayerAdvancements advancements;
        Actor(){super(null,null,null,null);}
        void configure(Level l){sceneLevel=l;setLevel(l);}
        @Override public Level level(){return sceneLevel;}
        @Override public boolean isChangingDimension(){return false;}
        @Override public boolean isPermanentlyInvulnerable(){return false;}
        @Override public boolean isTemporarilyInvulnerable(){return false;}
        @Override public boolean fireImmune(){return false;}
        @Override public boolean is(net.minecraft.tags.TagKey<EntityType<?>> tag){return false;} // player: neither fall immune nor freeze-extra
        @Override public boolean isSleeping(){return false;}
        @Override public ItemStack getUseItem(){return ItemStack.EMPTY;}
        @Override public boolean isUsingItem(){return false;}
        @Override public ItemStack getItemInHand(net.minecraft.world.InteractionHand hand){return ItemStack.EMPTY;}
        @Override protected void removeEntitiesOnShoulder(){}
        @Override public Collection<MobEffectInstance> getActiveEffects(){return List.of();}
        @Override public CombatTracker getCombatTracker(){return tracker;}
        @Override public net.minecraft.server.PlayerAdvancements getAdvancements(){events.add("hurt-criterion");return advancements;}
        @Override public void onSyncedDataUpdated(EntityDataAccessor<?> key){if(key!=healthKey&&key!=absorptionKey)throw new AssertionError(key);events.add(String.format("updated %d %08x %s",key.id(),Float.floatToIntBits(getEntityData().get(key==healthKey?healthKey:absorptionKey)),getEntityData().isDirty()));}
        @Override public void gameEvent(Holder<GameEvent> event){if(event==GameEvent.ENTITY_DAMAGE)events.add("entity-damage");else if(event==GameEvent.SPLASH)events.add("splash-event");else super.gameEvent(event);}
        @Override public void die(DamageSource source){dead=true;events.add("death-request");}
        @Override protected void playHurtSound(DamageSource source){events.add("hurt-sound");}
        @Override protected SoundEvent getDeathSound(){return SoundEvents.PLAYER_DEATH;}
        @Override public void makeSound(SoundEvent sound){events.add("death-sound");}
        @Override public GameType gameMode(){return GameType.SURVIVAL;}
        @Override public AttributeMap getAttributes(){return attrs;}
        @Override protected boolean getSharedFlag(int bit){return (flags&(1<<bit))!=0;}
        @Override protected void setSharedFlag(int bit,boolean v){flags=(byte)(v?flags|(1<<bit):flags&~(1<<bit));}
        @Override public void setPos(double x,double y,double z){write(position,this,new Vec3(x,y,z));write(blockPositionField,this,BlockPos.containing(x,y,z));setBoundingBox(EntityDimensions.scalable(getBbWidth(),getBbHeight()).makeBoundingBox(position()));}
        @Override public BlockPos blockPosition(){return BlockPos.containing(position());}
        @Override public BlockState getInBlockState(){return level().getBlockState(blockPosition());}
        @Override public ItemStack getMainHandItem(){return ItemStack.EMPTY;}
        @Override public boolean isPassenger(){return statisticsFlags!=null&&statFlag(0);}
        @Override public boolean isSwimming(){return statisticsFlags!=null?statFlag(1):super.isSwimming();}
        // LocalPlayer.isUnderWater: the resident player is the local one.
        @Override public boolean isUnderWater(){return wasUnderwater;}
        // The synchronized air supply as a plain field (no data item here).
        int air;
        @Override public int getAirSupply(){return air;}
        @Override public void setAirSupply(int value){air=value;}
        String swimState(){return "swim "+getSharedFlag(4)+" "+wasUnderwater+" air "+air;}
        @Override public boolean isEyeInFluid(net.minecraft.tags.TagKey<net.minecraft.world.level.material.Fluid> tag){return statisticsFlags!=null?statFlag(2):super.isEyeInFluid(tag);}
        @Override public boolean isInWater(){return statisticsFlags!=null?statFlag(3):super.isInWater();}
        @Override public boolean onClimbable(){return statisticsFlags!=null?statFlag(4):super.onClimbable();}
        @Override public boolean onGround(){return statisticsFlags!=null?statFlag(5):super.onGround();}
        @Override public boolean isSprinting(){return statisticsFlags!=null?statFlag(6):super.isSprinting();}
        @Override public boolean isCrouching(){return statisticsFlags!=null?statFlag(7):super.isCrouching();}
        @Override public boolean isFallFlying(){return statisticsFlags!=null&&statFlag(8);}
        @Override public boolean isAutoSpinAttack(){return false;}
        @Override public boolean isSpectator(){return false;}
        @Override public boolean isNoGravity(){return false;}
        // Effects as flags (Weaving for WebBlock.entityInside, water effects for
        // travel and breathing), set without the effects' callbacks.
        boolean weaving;Set<Holder<MobEffect>> effectFlags;
        @Override public boolean hasEffect(Holder<MobEffect> effect){return weaving&&effect==net.minecraft.world.effect.MobEffects.WEAVING||effectFlags.contains(effect);}
        @Override public MobEffectInstance getEffect(Holder<MobEffect> effect){return null;}
        @Override public MoveSimulationType getMoveSimulationType(){return MoveSimulationType.SERVER_AND_CLIENT;}
        @Override public boolean isClientAuthoritative(){return false;}
        @Override public void recordMovement(MoverType mover,Vec3 movement){}
        @Override public void playSound(SoundEvent sound,float volume,float pitch){if(stepping!=null&&sound!=SoundEvents.PLAYER_SWIM){events.add("muffled-step "+Block.getId(stepping));return;}events.add(String.format("sound %s entity %08x %08x",soundName(sound),Float.floatToIntBits(volume),Float.floatToIntBits(pitch)));}
        @Override protected void playBlockFallSound(){events.add("block-fall-sound");}
        @Override public void awardStat(net.minecraft.stats.Stat<?> stat,int value){
            if(stat.getValue()!=net.minecraft.stats.Stats.JUMP)throw new AssertionError(stat);
            events.add("health-stat jump "+value);
        }
        @Override public void awardStat(Identifier stat,int value){
            if(stat==net.minecraft.stats.Stats.FALL_ONE_CM){events.add("stat "+value);return;}
            String name=stat==net.minecraft.stats.Stats.DAMAGE_ABSORBED?"absorbed":stat==net.minecraft.stats.Stats.DAMAGE_TAKEN?"taken":stat==net.minecraft.stats.Stats.DAMAGE_RESISTED?"resisted":null;
            if(name==null){name=stat.getPath();if(!Set.of("swim_one_cm","walk_under_water_one_cm","walk_on_water_one_cm","climb_one_cm","sprint_one_cm","crouch_one_cm","walk_one_cm","aviate_one_cm","fly_one_cm").contains(name))throw new AssertionError(stat);}events.add("health-stat "+name+" "+value);
        }
        @Override public boolean hurtServer(net.minecraft.server.level.ServerLevel level,DamageSource source,float damage){var sources=((Level)level).sources;events.add(String.format("damage %s %08x",source==sources.fall?"fall":source==sources.stalagmite?"stalagmite":source==sources.starve?"starve":source==sources.freeze?"freeze":source==sources.sweetBerryBush?"sweet_berry_bush":source==sources.drown?"drown":"unknown",Float.floatToIntBits(damage)));return super.hurtServer(level,source,damage);}
        @Override public void teleportRelative(double x,double y,double z){setPos(getX()+x,getY()+y,getZ()+z);}
        // Footsteps stay a controlled consumer; in water Player.playStepSound
        // swims (its sound is compared) and muffles the step (marked here).
        BlockState stepping;
        @Override protected void playStepSound(BlockPos p,BlockState state){if(isInWater()){stepping=state;try{super.playStepSound(p,state);}finally{stepping=null;}}}
        @Override public void gameEvent(Holder<GameEvent> event,Entity source){}
        @Override public LivingEntity getControllingPassenger(){return null;}
        // Block-inside effects and frozen ticks run only in "snow_tick" operations.
        boolean effects;int frozen;Vec3 tickStart;
        @Override public int getTicksFrozen(){return frozen;}
        @Override public void setTicksFrozen(int value){if(!effects&&value!=0)throw new AssertionError(value);frozen=value;}
        @Override protected void applyEffectsFromBlocks(){if(effects)super.applyEffectsFromBlocks();}
        // LivingEntity.canFreeze with #freeze_immune_wearables, whose only
        // member a scene wears is leather boots (item tags aren't bound here).
        @Override public boolean canFreeze(){return !isSpectator()&&!getItemBySlot(net.minecraft.world.entity.EquipmentSlot.FEET).is(net.minecraft.world.item.Items.LEATHER_BOOTS);}
        // ServerPlayer.lastKnownClientMovement: this tick's movement.
        @Override public Vec3 getKnownMovement(){return tickStart==null?Vec3.ZERO:position().subtract(tickStart);}
        @Override protected void applyInput(){if(!preparedInput)super.applyInput();}
        void peaceful(){tickRegeneration();}
        // Entity.handlePortal without the destination (none: one level here),
        // remembering the entry of a portal it would go through.
        String travel;
        // The creative delay applies with invulnerable abilities, set only for
        // the transition time (the scene's food code runs without abilities).
        void portalTick(boolean creative){
            processPortalCooldown();
            if(portalProcess!=null){
                var entry=portalProcess.getEntryPosition();
                boolean was=getAbilities().invulnerable;getAbilities().invulnerable=creative;
                boolean through;try{through=portalProcess.processPortalTeleportation((net.minecraft.server.level.ServerLevel)sceneLevel,this,canUsePortal(false));}finally{getAbilities().invulnerable=was;}
                if(through){travel=entry.getX()+","+entry.getY()+","+entry.getZ();setPortalCooldown();}
                else if(portalProcess.hasExpired())portalProcess=null;
            }
        }
        void clock()throws Exception{sceneLevel.time++;tickCount++;if(cooldown.getInt(this)>0)cooldown.setInt(this,cooldown.getInt(this)-1);if(hurtTime>0)hurtTime--;}
        // Entity.baseTick's fluid phase: wasEyeInWater, the real
        // updateFluidInteraction, lava's fall halving and the first tick's end.
        // Player.tick's updateIsUnderwater, then Entity.baseTick's fluid phase:
        // wasEyeInWater, the real updateFluidInteraction and updateSwimming,
        // lava's fall halving and the first tick's end.
        void fluidTick(){updateIsUnderwater();wasEyeInWater=isEyeInFluid(net.minecraft.tags.FluidTags.WATER);updateFluidInteraction();updateSwimming();if(isInLava())fallDistance*=0.5;firstTick=false;}
        // LivingEntity.baseTick's air supply (server): the real decrease /
        // increase / drowning methods in its order.
        void airTick(){
            if(!isAlive())return;
            if(isEyeInFluid(net.minecraft.tags.FluidTags.WATER)&&!level().getBlockState(BlockPos.containing(getX(),getEyeY(),getZ())).is(Blocks.BUBBLE_COLUMN)){
                boolean canDrown=!canBreatheUnderwater()&&!MobEffectUtil.hasWaterBreathing(this)&&!getAbilities().invulnerable;
                if(canDrown){
                    setAirSupply(decreaseAirSupply(getAirSupply()));
                    if(shouldTakeDrowningDamage()){setAirSupply(0);level().broadcastEntityEvent(this,(byte)67);hurtServer((net.minecraft.server.level.ServerLevel)level(),damageSources().drown(),2.0F);}
                }else if(getAirSupply()<getMaxAirSupply()&&MobEffectUtil.shouldEffectsRefillAirsupply(this))setAirSupply(increaseAirSupply(getAirSupply()));
            }else if(getAirSupply()<getMaxAirSupply())setAirSupply(increaseAirSupply(getAirSupply()));
        }
        String fluidState(){return wasTouchingWater+" "+wasEyeInWater+" "+isEyeInFluid(net.minecraft.tags.FluidTags.WATER)+" "+firstTick+String.format(" %016x %016x",Double.doubleToLongBits(getFluidHeight(net.minecraft.tags.FluidTags.WATER)),Double.doubleToLongBits(getFluidHeight(net.minecraft.tags.FluidTags.LAVA)));}
        void playerMovementTick(Vec3 input,boolean jumping){
            fluidTick();
            aiStepWith(input,jumping);
        }
        void aiStepWith(Vec3 input,boolean jumping){
            preparedInput=true;xxa=(float)input.x;yya=0;zza=(float)input.z;setJumping(jumping);
            try{aiStep();}finally{preparedInput=false;}
        }
        void movementTick(Vec3 input)throws Exception {
            xxa=(float)input.x;yya=(float)input.y;zza=(float)input.z;
            try{aiStep.invoke((LivingEntity)this);}catch(Throwable t){if(t instanceof Exception e)throw e;throw new AssertionError(t);}
        }
        void land(double dy,boolean ground,BlockPos pos){checkFallDamage(dy,ground,level().getBlockState(pos),pos);}
    }
    static String pos(Object v){var o=(Optional<?>)v;return o.isEmpty()?"none":((BlockPos)o.get()).getX()+","+((BlockPos)o.get()).getY()+","+((BlockPos)o.get()).getZ();}
    @SuppressWarnings("unchecked")
    static void snapshot(PrintWriter out,Actor p)throws Exception{
        var attributes=new ArrayList<String>();
        var stored=(Map<Holder<Attribute>,AttributeInstance>)attributeStorage.get(p.attrs);
        for(var entry:stored.entrySet()){
            var a=entry.getValue();var modifiers=new ArrayList<String>();
            for(var m:a.getModifiers())modifiers.add(String.format("%s:%016x:%d:%s",m.id(),Double.doubleToLongBits(m.amount()),m.operation().ordinal(),a.getPermanentModifiers().contains(m)));
            Collections.sort(modifiers);
            attributes.add(String.format("%s:%016x:%s:%016x:%s:%s:[%s]",entry.getKey().getRegisteredName(),Double.doubleToLongBits(a.getBaseValue()),attributeDirty.getBoolean(a),Double.doubleToLongBits(attributeCached.getDouble(a)),p.attrs.getAttributesToUpdate().contains(a),p.attrs.getAttributesToSync().contains(a),String.join(",",modifiers)));
        }
        Collections.sort(attributes);
        AABB b=p.getBoundingBox();
        for(var v:List.of(p.position(),p.getDeltaMovement(),new Vec3(b.minX,b.minY,b.minZ),new Vec3(b.maxX,b.maxY,b.maxZ)))
            out.printf("%016x %016x %016x ",Double.doubleToLongBits(v.x),Double.doubleToLongBits(v.y),Double.doubleToLongBits(v.z));
        out.printf("%s %s %s %s %s %s %s %016x %s %s %s %08x %d %08x %d %d %s | %s%n",p.horizontalCollision,p.verticalCollision,p.verticalCollisionBelow,p.minorHorizontalCollision,
            p.onGround(),noBlocks.getBoolean(p),pos(support.get(p)),Double.doubleToLongBits(p.fallDistance),pos(last.get(p)),p.needsSync,p.syncPosition,Float.floatToIntBits(storedSpeed.getFloat(p)),delay.getInt(p),Float.floatToIntBits(p.yHeadRot),trigger.getInt(p),grace.getInt(p),impact.get(p)==null?"none":vectorBits((Vec3)impact.get(p)),String.join(";",attributes));
        out.println("events "+String.join(" | ",p.events));p.events.clear();
        out.println("block "+Block.getId(p.level().getBlockState(new BlockPos(25,3,25)))+" frozen "+p.frozen+" "+p.isInPowderSnow+" "+p.wasInPowderSnow+" "+vectorBits((Vec3)stuck.get(p))+" fluid "+p.fluidState()+" "+p.swimState());
        var pp=p.portalProcess;
        out.println("portal "+p.getPortalCooldown()+" "+(pp==null?"none":pp.getEntryPosition().getX()+","+pp.getEntryPosition().getY()+","+pp.getEntryPosition().getZ()+" "+pp.getPortalTime()+" "+pp.isInsidePortalThisTick())+" "+(p.travel==null?"-":p.travel));p.travel=null;
        out.printf("health %08x %08x %08x %d %d %08x %d %d %s %d %s %s %s %s %s %d%n",
            Float.floatToIntBits(p.getHealth()),Float.floatToIntBits(p.getAbsorptionAmount()),Float.floatToIntBits(foodExhaustion.getFloat(p.getFoodData())),
            noAction.getInt(p),cooldown.getInt(p),Float.floatToIntBits(lastHurt.getFloat(p)),p.hurtDuration,p.hurtTime,sourceName((DamageSource)lastDamage.get(p)),lastStamp.getLong(p),healthDead.getBoolean(p),p.syncVelocity,
            p.getEntityData().isDirty(),p.healthItem.isDirty(),p.absorptionItem.isDirty(),p.sceneLevel.time);
        var food=p.getFoodData();out.printf("food %d %08x %d %d %d %s%n",food.getFoodLevel(),Float.floatToIntBits(food.getSaturationLevel()),foodTimer.getInt(food),p.tickCount,p.sceneLevel.difficulty.ordinal(),p.sceneLevel.rules.regenerate);
        out.println("input "+p.isSprinting()+" "+(p.sprintController==null?"0 false false":p.sprintController.getClass().getMethod("state").invoke(p.sprintController)));
        out.printf("pose %s %s %s %08x %08x %08x%n",p.getPose(),p.isShiftKeyDown(),p.sprintController!=null&&((Boolean)p.sprintController.getClass().getMethod("crouching").invoke(p.sprintController)),Float.floatToIntBits(p.getBbWidth()),Float.floatToIntBits(p.getBbHeight()),Float.floatToIntBits(p.getEyeHeight()));
    }
    @SuppressWarnings("unchecked") public static void main(String[] args)throws Exception{
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);
        VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(Path.of(args[2]).resolve("block-tags.json"))).getAsJsonObject());
        // The vanilla data pack's #water and #lava fluid tags.
        net.minecraft.core.registries.BuiltInRegistries.FLUID.prepareTagReload(new net.minecraft.tags.TagLoader.LoadResult<>(net.minecraft.core.registries.Registries.FLUID,Map.of(
            net.minecraft.tags.FluidTags.WATER,List.<Holder<net.minecraft.world.level.material.Fluid>>of(net.minecraft.world.level.material.Fluids.FLOWING_WATER.builtInRegistryHolder(),net.minecraft.world.level.material.Fluids.WATER.builtInRegistryHolder()),
            net.minecraft.tags.FluidTags.LAVA,List.<Holder<net.minecraft.world.level.material.Fluid>>of(net.minecraft.world.level.material.Fluids.FLOWING_LAVA.builtInRegistryHolder(),net.minecraft.world.level.material.Fluids.LAVA.builtInRegistryHolder())))).apply();
        var u=Class.forName("sun.misc.Unsafe");var f=u.getDeclaredField("theUnsafe");f.setAccessible(true);var unsafe=f.get(null);var allocate=u.getMethod("allocateInstance",Class.class);
        position=field(Entity.class,"position");blockPositionField=field(Entity.class,"blockPosition");history=field(Entity.class,"movementThisTick");support=field(Entity.class,"mainSupportingBlockPos");noBlocks=field(Entity.class,"onGroundNoBlocks");last=field(LivingEntity.class,"lastClimbablePos");stuck=field(Entity.class,"stuckSpeedMultiplier");
        var abilities=field(Player.class,"abilities");
        healthKey=(EntityDataAccessor<Float>)field(LivingEntity.class,"DATA_HEALTH_ID").get(null);absorptionKey=(EntityDataAccessor<Float>)field(Player.class,"DATA_PLAYER_ABSORPTION_ID").get(null);
        healthDead=field(LivingEntity.class,"dead");cooldown=field(LivingEntity.class,"damageCooldownTime");lastHurt=field(LivingEntity.class,"lastHurt");noAction=field(LivingEntity.class,"noActionTime");lastDamage=field(LivingEntity.class,"lastDamageSource");lastStamp=field(LivingEntity.class,"lastDamageStamp");foodExhaustion=field(FoodData.class,"exhaustionLevel");foodTimer=field(FoodData.class,"tickTimer");
        var dataCtor=SynchedEntityData.class.getDeclaredConstructor(SyncedDataHolder.class,SynchedEntityData.DataItem[].class);dataCtor.setAccessible(true);
        var damageRegistry=VanillaDamageSourceOracle.registry(dir.resolve("damage"));damageSources=new HashMap<>();
        for(var name:damageRegistry.keySet())damageSources.put(name.toString(),new DamageSource(damageRegistry.get(name).orElseThrow()));
        delay=field(LivingEntity.class,"noJumpDelay");
        trigger=field(Player.class,"jumpTriggerTime");storedSpeed=field(LivingEntity.class,"speed");
        impact=field(LivingEntity.class,"currentImpulseImpactPos");grace=field(LivingEntity.class,"currentImpulseContextResetGraceTime");
        attributeStorage=field(AttributeMap.class,"attributes");attributeDirty=field(AttributeInstance.class,"dirty");attributeCached=field(AttributeInstance.class,"cachedValue");
        refreshAttributes=LivingEntity.class.getDeclaredMethod("refreshDirtyAttributes");refreshAttributes.setAccessible(true);
        aiStep=MethodHandles.privateLookupIn(LivingEntity.class,MethodHandles.lookup()).findSpecial(LivingEntity.class,"aiStep",MethodType.methodType(void.class),LivingEntity.class);
        var description=JsonParser.parseString(Files.readString(dir.resolve("scene.json"))).getAsJsonObject();
        var trace=new Trace();trace.out=new PrintWriter(OutputStream.nullOutputStream());trace.fail=-1;
        var level=(Level)allocate.invoke(unsafe,Level.class);level.chunks=VanillaSceneCollisionOracle.scene(description,trace);level.border=new Border();
        level.rng=net.minecraft.util.RandomSource.create(0);level.sources=(Sources)allocate.invoke(unsafe,Sources.class);
        level.sources.fall=damageSources.get("minecraft:fall");level.sources.stalagmite=damageSources.get("minecraft:stalagmite");level.sources.starve=damageSources.get("minecraft:starve");level.sources.freeze=damageSources.get("minecraft:freeze");level.sources.sweetBerryBush=damageSources.get("minecraft:sweet_berry_bush");level.sources.drown=damageSources.get("minecraft:drown");
        level.waypoints=(Waypoints)allocate.invoke(unsafe,Waypoints.class);level.waypoints.level=level;
        level.rules=new Rules();level.difficulty=net.minecraft.world.Difficulty.NORMAL;
        var server=(VanillaBlockLandingOracle.Server)allocate.invoke(unsafe,VanillaBlockLandingOracle.Server.class);
        server.d=new VanillaBlockLandingOracle.Data();server.d.out=new PrintWriter(OutputStream.nullOutputStream());server.d.f=JsonParser.parseString("{\"fail\":-1,\"protected\":false}").getAsJsonObject();level.server=server;write(field(net.minecraft.server.MinecraftServer.class,"debugSubscribers"),server,new net.minecraft.util.debug.ServerDebugSubscribers(server));
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var enriched=new PrintWriter(Files.newBufferedWriter(dir.resolve("inputs-"+args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){
            for(var line:(Iterable<String>)lines::iterator){
                var row=JsonParser.parseString(line).getAsJsonObject();var p=(Actor)allocate.invoke(unsafe,Actor.class);p.configure(level);p.events=new ArrayList<>();level.actor=p;p.effectFlags=new HashSet<>();p.air=300;
                // Entity.nextStep's initializer (allocation skips it): the first footstep or swim sound after a metre.
                field(Entity.class,"nextStep").setFloat(p,1.0F);
                write(field(Entity.class,"type"),p,EntityTypes.PLAYER);
                p.scenePose=Pose.STANDING;
                write(field(Entity.class,"passengers"),p,com.google.common.collect.ImmutableList.of());write(field(Entity.class,"dimensions"),p,p.getDefaultDimensions(Pose.STANDING));write(field(Entity.class,"eyeHeight"),p,1.62f);
                write(field(Entity.class,"interpolationHandler"),p,InterpolationHandler.NO_OP);
                var equipment=new EntityEquipment();write(field(LivingEntity.class,"equipment"),p,equipment);
                write(field(Player.class,"inventory"),p,new Inventory(p,equipment));
                p.attrs=new TrackedAttributes(VanillaEntityAttributeOracle.supplier(EntityTypes.PLAYER));write(field(LivingEntity.class,"attributes"),p,p.attrs);
                write(field(Entity.class,"finalMovementsThisTick"),p,new ArrayList<>());write(field(Entity.class,"visitedBlocks"),p,new it.unimi.dsi.fastutil.longs.LongOpenHashSet());
                write(field(Entity.class,"insideEffectCollector"),p,new net.minecraft.world.entity.InsideBlockEffectApplier.StepBasedCollector());
                write(field(Entity.class,"fluidInteraction"),p,new net.minecraft.world.entity.EntityFluidInteraction(Set.of(net.minecraft.tags.FluidTags.WATER,net.minecraft.tags.FluidTags.LAVA)));
                write(field(Entity.class,"random"),p,level.rng);field(Entity.class,"firstTick").setBoolean(p,true);
                write(history,p,new ArrayDeque<>());write(support,p,Optional.empty());write(last,p,Optional.empty());write(stuck,p,Vec3.ZERO);write(abilities,p,new Abilities());
                write(field(ServerPlayer.class,"shoulderEntityLeft"),p,new net.minecraft.nbt.CompoundTag());write(field(ServerPlayer.class,"shoulderEntityRight"),p,new net.minecraft.nbt.CompoundTag());
                p.connection=(Connection)allocate.invoke(unsafe,Connection.class);
                p.advancements=(net.minecraft.server.PlayerAdvancements)allocate.invoke(unsafe,net.minecraft.server.PlayerAdvancements.class);
                write(field(net.minecraft.server.PlayerAdvancements.class,"activeTriggers"),p.advancements,new HashMap<>());
                p.tracker=(Tracker)allocate.invoke(unsafe,Tracker.class);p.tracker.actor=p;
                write(field(Player.class,"foodData"),p,new FoodData());
                var items=new SynchedEntityData.DataItem<?>[Math.max(healthKey.id(),absorptionKey.id())+1];
                for(int i=0;i<items.length;i++)items[i]=new SynchedEntityData.DataItem<Float>(new EntityDataAccessor<Float>(i,EntityDataSerializers.FLOAT),0f);
                items[healthKey.id()]=p.healthItem=new SynchedEntityData.DataItem<Float>(healthKey,1f);items[absorptionKey.id()]=p.absorptionItem=new SynchedEntityData.DataItem<Float>(absorptionKey,0f);
                write(field(Entity.class,"entityData"),p,dataCtor.newInstance(p,items));p.setHealth(p.getMaxHealth());p.events.clear();level.time=0;level.rules.regenerate=true;level.difficulty=net.minecraft.world.Difficulty.NORMAL;
                p.setDeltaMovement(Vec3.ZERO);p.setPos(VanillaSceneCollisionOracle.vector(row.get("position")));
                for(var value:row.getAsJsonArray("ops")){
                    var op=value.getAsJsonObject();switch(op.get("mode").getAsString()){
                        case "shift"->p.setShiftKeyDown(op.get("value").getAsBoolean());
                        case "pose_tick"->p.poseTick();
                        case "refresh_dimensions"->p.refreshDimensions();
                        case "refresh_attributes"->p.attributeTick();
                        case "pose"->p.setPose(Pose.valueOf(op.get("value").getAsString()));
                        case "controlled_tick"->{
                            // Player.tick: the base tick (fluids, swimming, air supply),
                            // then LocalPlayer.aiStep's prefix and the actor's aiStep.
                            if(p.sprintController==null)p.sprintController=Class.forName("dev.horizonlab.parity.VanillaSprintInputOracle$Controller").getConstructor(ServerPlayer.class).newInstance(p);
                            p.clock();var before=p.position();p.fluidTick();p.airTick();
                            int mask=op.get("value").getAsInt();var direction=(net.minecraft.world.phys.Vec2)p.sprintController.getClass().getMethod("step",int.class).invoke(p.sprintController,mask);
                            p.aiStepWith(new Vec3(direction.x,0,direction.y),(mask&16)!=0);p.attributeTick();p.poseTick();p.checkMovementStatistics(p.getX()-before.x,p.getY()-before.y,p.getZ()-before.z);p.getFoodData().tick(p);
                        }
                        case "health"->p.setHealth(single(op.get("value")));
                        case "absorption"->p.setAbsorptionAmount(single(op.get("value")));
                        case "hurt"->p.hurtServer(level,damageSources.get(op.get("name").getAsString()),single(op.get("value")));
                        case "clock"->p.clock();
                        case "tick_count"->p.tickCount=op.get("value").getAsInt();
                        case "rules"->{level.difficulty=net.minecraft.world.Difficulty.values()[op.get("difficulty").getAsInt()];level.rules.regenerate=op.get("regenerate").getAsBoolean();}
                        case "food"->{var food=p.getFoodData();food.setFoodLevel(op.get("food").getAsInt());food.setSaturation(single(op.get("saturation")));foodExhaustion.setFloat(food,single(op.get("exhaustion")));foodTimer.setInt(food,op.get("timer").getAsInt());}
                        case "exhaust"->p.causeFoodExhaustion(single(op.get("value")));
                        case "eat"->p.getFoodData().eat(op.get("nutrition").getAsInt(),single(op.get("value")));
                        case "food_tick"->p.getFoodData().tick(p);
                        case "peaceful"->p.peaceful();
                        case "simulation_tick"->{p.clock();var before=p.position();p.fluidTick();p.airTick();p.aiStepWith(VanillaSceneCollisionOracle.vector(op.get("value")),op.get("jumping").getAsBoolean());p.attributeTick();p.poseTick();p.checkMovementStatistics(p.getX()-before.x,p.getY()-before.y,p.getZ()-before.z);p.getFoodData().tick(p);}
                        case "movement_statistics"->{var delta=VanillaSceneCollisionOracle.vector(op.get("value"));p.checkMovementStatistics(delta.x,delta.y,delta.z);}
                        case "statistics_context"->{
                            p.statisticsFlags=op.get("flags").getAsInt();p.getAbilities().invulnerable=op.get("invulnerable").getAsBoolean();
                            try{var delta=VanillaSceneCollisionOracle.vector(op.get("value"));p.checkMovementStatistics(delta.x,delta.y,delta.z);}
                            finally{p.statisticsFlags=null;p.getAbilities().invulnerable=false;}
                        }
                        case "seed"->level.rng.setSeed(op.get("value").getAsLong());
                        case "impulse"->p.setIgnoreFallDamageFromCurrentImpulse(op.get("ignore").getAsBoolean(),op.get("value").isJsonNull()?null:VanillaSceneCollisionOracle.vector(op.get("value")));
                        case "block","fill"->{var v=op.getAsJsonArray("position");var to=op.has("to")?op.getAsJsonArray("to"):v;var state=VanillaSceneCollisionOracle.state(op.get("name").getAsString());if(op.has("properties"))for(var property:op.getAsJsonObject("properties").entrySet())state=VanillaSceneCollisionOracle.property(state,state.getBlock().getStateDefinition().getProperty(property.getKey()),property.getValue().getAsString());
                            for(int x=v.get(0).getAsInt();x<=to.get(0).getAsInt();x++)for(int y=v.get(1).getAsInt();y<=to.get(1).getAsInt();y++)for(int z=v.get(2).getAsInt();z<=to.get(2).getAsInt();z++){var pos=new BlockPos(x,y,z);level.chunks.get(key(pos.getX()>>4,pos.getZ()>>4)).write(pos,Block.getId(state));}}
                        case "landing"->{var v=op.getAsJsonArray("position");p.land(number(op.get("value")),op.get("ground").getAsBoolean(),new BlockPos(v.get(0).getAsInt(),v.get(1).getAsInt(),v.get(2).getAsInt()));}
                        case "fluid_tick"->p.fluidTick();
                        case "air_tick"->p.airTick();
                        case "air"->p.air=op.get("value").getAsInt();
                        case "pitch"->p.setXRot(single(op.get("value")));
                        case "swimming"->p.setSwimming(op.get("value").getAsBoolean());
                        case "effect"->{var e=BuiltInRegistries.MOB_EFFECT.wrapAsHolder(BuiltInRegistries.MOB_EFFECT.getValue(Identifier.parse(op.get("name").getAsString())));if(op.get("value").getAsBoolean())p.effectFlags.add(e);else p.effectFlags.remove(e);}
                        case "travel"->p.travel(VanillaSceneCollisionOracle.vector(op.get("value")));
                        case "tick"->p.movementTick(VanillaSceneCollisionOracle.vector(op.get("value")));
                        case "player_tick"->p.playerMovementTick(VanillaSceneCollisionOracle.vector(op.get("value")),op.get("jumping").getAsBoolean());
                        case "snow_tick"->{
                            // The clock, Entity.baseTick's powder snow flags and old
                            // position, then aiStep with its block-inside effects.
                            p.clock();p.wasInPowderSnow=p.isInPowderSnow;p.isInPowderSnow=false;
                            p.xOld=p.getX();p.yOld=p.getY();p.zOld=p.getZ();p.tickStart=p.position();
                            p.effects=true;
                            try{p.playerMovementTick(VanillaSceneCollisionOracle.vector(op.get("value")),op.get("jumping").getAsBoolean());}finally{p.effects=false;}
                        }
                        case "boots"->{
                            // Leather boots without data components (item components aren't
                            // bound here), straight into the equipment: no equip callbacks.
                            var stack=net.minecraft.world.item.ItemStack.EMPTY;
                            if(op.get("value").getAsBoolean()){
                                stack=(net.minecraft.world.item.ItemStack)allocate.invoke(unsafe,net.minecraft.world.item.ItemStack.class);
                                write(field(net.minecraft.world.item.ItemStack.class,"item"),stack,net.minecraft.world.item.Items.LEATHER_BOOTS.builtInRegistryHolder());
                                field(net.minecraft.world.item.ItemStack.class,"count").setInt(stack,1);
                                write(field(net.minecraft.world.item.ItemStack.class,"components"),stack,new net.minecraft.core.component.PatchedDataComponentMap(net.minecraft.core.component.DataComponentMap.EMPTY));
                            }
                            ((EntityEquipment)field(LivingEntity.class,"equipment").get(p)).set(net.minecraft.world.entity.EquipmentSlot.FEET,stack);
                        }
                        case "frozen"->p.frozen=op.get("value").getAsInt();
                        case "portal_tick"->p.portalTick(op.get("creative").getAsBoolean());
                        case "weaving"->p.weaving=op.get("value").getAsBoolean();
                        case "flying"->{var a=p.getAbilities();a.flying=op.get("value").getAsBoolean();a.mayfly=op.has("may_fly")?op.get("may_fly").getAsBoolean():a.flying;if(op.has("speed"))a.setFlyingSpeed(single(op.get("speed")));}
                        case "trigger"->trigger.setInt(p,op.get("value").getAsInt());
                        case "delay"->delay.setInt(p,op.get("value").getAsInt());
                        case "ack_attributes"->{p.attrs.getAttributesToUpdate().clear();p.attrs.getAttributesToSync().clear();}
                        case "modifier"->{
                            var holder=BuiltInRegistries.ATTRIBUTE.get(Identifier.parse(op.get("name").getAsString())).orElseThrow();
                            var modifier=new AttributeModifier(Identifier.parse(op.get("id").getAsString()),number(op.get("value")),AttributeModifier.Operation.values()[op.get("operation").getAsInt()]);
                            if(op.get("permanent").getAsBoolean())p.getAttribute(holder).addPermanentModifier(modifier);else p.getAttribute(holder).addTransientModifier(modifier);
                        }
                        case "move"->p.move(MoverType.SELF,VanillaSceneCollisionOracle.vector(op.get("value")));
                        case "jump"->p.jumpFromGround();
                        case "sprint"->p.setSprinting(op.get("value").getAsBoolean());
                        case "speed"->p.setSpeed(single(op.get("value")));
                        case "yaw"->p.setYRot(single(op.get("value")));
                        case "velocity"->p.setDeltaMovement(VanillaSceneCollisionOracle.vector(op.get("value")));
                        case "stuck"->write(stuck,p,VanillaSceneCollisionOracle.vector(op.get("value")));
                        case "fall"->p.fallDistance=number(op.get("value"));
                        case "jumping"->p.setJumping(op.get("value").getAsBoolean());
                        case "attribute"->{var attr=BuiltInRegistries.ATTRIBUTE.wrapAsHolder(BuiltInRegistries.ATTRIBUTE.getValue(Identifier.parse(op.get("name").getAsString())));p.getAttribute(attr).setBaseValue(number(op.get("value")));}
                        default->throw new AssertionError(op);
                    }
                    snapshot(out,p);
                }
                out.println("end");
                var hashes=new JsonObject();for(var attr:BuiltInRegistries.ATTRIBUTE){var key=BuiltInRegistries.ATTRIBUTE.wrapAsHolder(attr);hashes.addProperty(key.getRegisteredName(),key.hashCode());}row.add("holder_hashes",hashes);
                var instances=new JsonArray();for(var instance:((TrackedAttributes)p.attrs).instances)instances.add(System.identityHashCode(instance));row.add("instance_hashes",instances);
                enriched.println(row);
            }
        }
    }
}
