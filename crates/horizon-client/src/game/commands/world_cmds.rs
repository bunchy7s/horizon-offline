//! `/recipe`, `/damage`, `/rotate`, `/swing`, `/playsound`, `/stopsound`,
//! `/particle`, `/attribute`, `/ride` and `/spreadplayers`.
use super::{Arg, Context, Game, R, Target, err};
use horizon_client::{
    mobs::{Damage, DamageKind, Who},
    scoreboard_cmd::{Holder, Tracker},
    settings::SOUND_SOURCES,
    text::Text,
};
use horizon_core::random::RandomSource;
use horizon_protocol::packets::play::{self, Clientbound as C};
use horizon_world::attribute_map::ValueKind;
use horizon_world::nbt::Tag;

fn num(n: impl std::fmt::Display) -> Text {
    Text::literal(n.to_string())
}
fn holder(text: Text) -> Holder {
    Holder {
        name: String::new(),
        text,
        display: None,
    }
}
/// `ChatFormatting`-free float text as `Component.translatable` prints a `double`.
fn java_double_text(v: f64) -> Text {
    Text::literal(horizon_client::snbt::java_double(v))
}

impl Game<'_> {
    // ---- /recipe -----------------------------------------------------------

    /// Every recipe id of the recipe manager (what `/recipe` resolves and suggests).
    pub(super) fn learnable_recipes(&self) -> Vec<String> {
        self.stations().recipe_index.ids().cloned().collect()
    }
    pub(super) fn cmd_recipe(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "targets", true)?;
        let give = id == "recipe.give";
        let recipes: Vec<String> = match ctx.get("recipe") {
            Some(Arg::Resource(r)) => {
                if !self.learnable_recipes().contains(r) {
                    return Err(err("recipe.notFound", &[r]));
                }
                vec![r.clone()]
            }
            _ => self.learnable_recipes(),
        };
        let mut tracker = Tracker::default();
        for &t in &targets {
            let h = holder(self.target_name(t));
            // ServerPlayer.awardRecipes / resetRecipes on the target's own
            // ServerRecipeBook: a guest's is swapped in for the call, and the
            // packets go to its client.
            let n = self
                .for_player(t, |g| g.change_recipes(give, &recipes))
                .unwrap_or(0);
            tracker.track(&h, n);
        }
        let verb = if give { "give" } else { "take" };
        let (message, total) = tracker.message(
            false,
            Some(err(&format!("commands.recipe.{verb}.failed"), &[])),
            |h, total| {
                Text::translate(
                    &format!("commands.recipe.{verb}.success.single"),
                    &[num(total), h.text.clone()],
                )
            },
            |count, total| {
                Text::translate(
                    &format!("commands.recipe.{verb}.success.multiple"),
                    &[num(total), num(count)],
                )
            },
        )?;
        self.feedback(message);
        Ok(total)
    }

    /// ServerRecipeBook.addRecipes/removeRecipes for the player in place: how
    /// many recipe book displays changed (a transmute recipe has one per
    /// material count).
    fn change_recipes(&mut self, give: bool, recipes: &[String]) -> i32 {
        let index = &world_stations!(self).recipe_index;
        if give {
            let (count, unlocked) = self.level.recipe_book.add_recipes(index, recipes);
            self.recipe_unlocked(&unlocked);
            return count as i32;
        }
        let removed = self.level.recipe_book.remove_recipes(index, recipes);
        let count = removed.len() as i32;
        // A network player's client drops them from its book.
        if self.lan_acting.is_some() && !removed.is_empty() {
            self.guest_packets.push(C::RecipeBookRemove(
                horizon_protocol::packets::recipes::RecipeBookRemove { ids: removed },
            ));
        }
        count
    }

    // ---- /damage ----------------------------------------------------------------

    fn damage_kind(ty: &str) -> DamageKind {
        match ty.trim_start_matches("minecraft:") {
            "player_attack" => DamageKind::PlayerAttack,
            "mace_smash" => DamageKind::MaceSmash,
            "spear" => DamageKind::Spear,
            "mob_attack" | "mob_attack_no_aggro" => DamageKind::MobAttack,
            "arrow" => DamageKind::Arrow,
            "thrown" | "trident" | "mob_projectile" => DamageKind::Thrown,
            "fireball" | "unattributed_fireball" | "wither_skull" => DamageKind::Fireball,
            "explosion" | "player_explosion" | "bad_respawn_point" => DamageKind::Explosion,
            "fall" | "ender_pearl" | "fly_into_wall" => DamageKind::Fall,
            "on_fire" => DamageKind::OnFire,
            "in_fire" | "hot_floor" => DamageKind::InFire,
            "campfire" => DamageKind::Campfire,
            "lava" => DamageKind::Lava,
            "drown" => DamageKind::Drown,
            "dry_out" => DamageKind::DryOut,
            "magic" | "indirect_magic" | "dragon_breath" => DamageKind::Magic,
            "sting" => DamageKind::Sting,
            "cactus" => DamageKind::Cactus,
            "sweet_berry_bush" => DamageKind::SweetBerryBush,
            "freeze" => DamageKind::Freeze,
            "in_wall" => DamageKind::InWall,
            "thorns" => DamageKind::Thorns,
            "cramming" => DamageKind::Cramming,
            "falling_anvil" => DamageKind::FallingAnvil,
            "falling_block" => DamageKind::FallingBlock,
            "fireworks" => DamageKind::Fireworks,
            "generic_kill" => DamageKind::GenericKill,
            "lightning_bolt" => DamageKind::LightningBolt,
            "out_of_world" => DamageKind::OutOfWorld,
            "outside_border" => DamageKind::OutsideBorder,
            "sonic_boom" => DamageKind::SonicBoom,
            "spit" => DamageKind::Spit,
            "stalagmite" => DamageKind::Stalagmite,
            "starve" => DamageKind::Starve,
            "sulfur_cube_hot" => DamageKind::SulfurCubeHot,
            "wind_charge" => DamageKind::WindCharge,
            "wither" => DamageKind::Wither,
            _ => DamageKind::Generic,
        }
    }
    pub(super) fn cmd_damage(&mut self, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "target", false)?;
        let Some(target) = targets.first().copied() else {
            return Ok(0);
        };
        let amount = match ctx.get("amount") {
            Some(Arg::Float(a)) => *a,
            _ => 0.0,
        };
        let ty = match ctx.get("damageType") {
            Some(Arg::Resource(t)) => t.clone(),
            _ => "minecraft:generic".into(),
        };
        let by = match ctx.get("entity") {
            Some(Arg::Entity(_)) => self.entities(ctx, "entity", false)?.first().copied(),
            _ => None,
        };
        let cause = match ctx.get("cause") {
            Some(Arg::Entity(_)) => self.entities(ctx, "cause", false)?.first().copied(),
            _ => None,
        };
        let from = match ctx.get("location") {
            Some(Arg::Coords(_)) => self.position_arg(ctx, "location"),
            _ => by.and_then(|t| self.target_position(t)),
        };
        let hurt = match target {
            // The causing mob is the one the hit is credited to and thorns
            // answer; the source position gives the knockback its direction.
            Target::Player | Target::Guest(_) => {
                // `DamageSource(type, cause, by)`: the death message names the
                // causing entity, else the direct one. A mob is noted by
                // `hurt_player_from`, a player here.
                let named = match cause.or(by) {
                    Some(t @ (Target::Player | Target::Guest(_))) => Some(self.target_name(t)),
                    _ => None,
                };
                let attacker = match cause.or(by) {
                    Some(Target::Mob(i)) => Some(Some(i)),
                    _ => Some(None),
                };
                self.for_player(target, |g| {
                    if let Some(name) = named {
                        g.note_direct_entity(&ty, name);
                    }
                    !g.spectator() && g.hurt_player_from(amount, from, &ty, attacker)
                })
                .unwrap_or(false)
            }
            Target::Mob(id) => {
                let attacker = cause.or(by).and_then(|t| match t {
                    Target::Player => Some(Who::Player),
                    Target::Mob(i) => Some(Who::Mob(i)),
                    _ => None,
                });
                let damage = Damage {
                    kind: Self::damage_kind(&ty),
                    attacker,
                    from,
                };
                self.mobs
                    .iter_mut()
                    .find(|m| m.id == id)
                    .is_some_and(|m| m.damage(amount, damage))
            }
            _ => false,
        };
        if !hurt {
            return Err(err("commands.damage.invulnerable", &[]));
        }
        let name = self.target_name(target);
        self.feedback(Text::translate(
            "commands.damage.success",
            &[
                Text::literal(horizon_client::snbt::java_float(amount)),
                name,
            ],
        ));
        Ok(1)
    }

    // ---- /rotate and /swing -----------------------------------------------------------

    pub(super) fn cmd_rotate(&mut self, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "target", false)?;
        let Some(t) = targets.first().copied() else {
            return Ok(0);
        };
        let Some(pos) = self.target_position(t) else {
            return Ok(0);
        };
        let current = self.target_rotation(t);
        let rot = if let Some(Arg::Coords(c)) = ctx.get("rotation") {
            // Relative parts are offsets from the entity's own rotation.
            c.rotation(current)
        } else {
            let to = if ctx.has("entity") {
                let faced = self.entities(ctx, "facingEntity", false)?;
                let Some(f) = faced.first().copied() else {
                    return Ok(0);
                };
                let mut p = self.target_position(f).unwrap_or(pos);
                if matches!(ctx.get("facingAnchor"), Some(Arg::Anchor(true))) {
                    p[1] += self.target_eye_height(f);
                }
                p
            } else {
                self.position_arg(ctx, "facingLocation").unwrap_or(pos)
            };
            let from = [pos[0], pos[1] + self.target_eye_height(t), pos[2]];
            look_at(from, to)
        };
        self.teleport_to(t, pos, Some(rot));
        if t == Target::Player {
            self.pitch = rot[1].clamp(-90.0, 90.0);
        }
        let name = self.target_name(t);
        self.feedback(Text::translate("commands.rotate.success", &[name]));
        Ok(1)
    }
    pub(super) fn cmd_swing(&mut self, ctx: &Context) -> R<i32> {
        let targets = if ctx.get("targets").is_some() {
            self.entities(ctx, "targets", false)?
        } else {
            vec![
                self.command_source
                    .and_then(|s| s.entity)
                    .unwrap_or(Target::Player),
            ]
        };
        let mut tracker = Tracker::default();
        for t in targets {
            match t {
                Target::Player => self.swing_arm(),
                Target::Mob(id) => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                        m.ai.swinging = true;
                        m.ai.swing_time = -1;
                    }
                }
                // An armor stand is a LivingEntity: it swings (invisibly).
                Target::Stand(_) => {}
                _ => continue,
            }
            let h = holder(self.target_name(t));
            tracker.mark(&h);
        }
        let (message, total) = tracker.message(
            false,
            Some(err("commands.swing.failed.notliving", &[])),
            |h, _| {
                Text::translate(
                    "commands.swing.success.single",
                    std::slice::from_ref(&h.text),
                )
            },
            |c, _| Text::translate("commands.swing.success.multiple", &[num(c)]),
        )?;
        self.feedback(message);
        Ok(total)
    }

    // ---- sounds -----------------------------------------------------------------------

    pub(super) fn cmd_playsound(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Resource(sound)) = ctx.get("sound").cloned() else {
            return Ok(0);
        };
        let source = ctx
            .literals
            .iter()
            .find_map(|l| SOUND_SOURCES.iter().position(|s| s == l))
            .unwrap_or(0);
        let targets = if ctx.get("targets").is_some() {
            self.entities(ctx, "targets", true)?
        } else {
            vec![Target::Player]
        };
        let pos = self
            .position_arg(ctx, "pos")
            .unwrap_or_else(|| self.source_position());
        let float = |name: &str, default: f32| match ctx.get(name) {
            Some(Arg::Float(v)) => *v,
            _ => default,
        };
        let (volume, pitch, min_volume) = (
            float("volume", 1.0),
            float("pitch", 1.0),
            float("minVolume", 0.0),
        );
        let range = if volume > 1.0 { 16.0 * volume } else { 16.0 };
        // PlaySoundCommand: one seed for every player.
        let seed = self.world.random.next_long();
        let here = self.dim.id();
        let mut tracker = Tracker::default();
        for t in targets {
            if !matches!(t, Target::Player | Target::Guest(_)) {
                continue;
            }
            // Only players in the source's level hear it.
            if let Target::Guest(conn) = t
                && self.command_guest(conn).is_none_or(|g| g.dimension != here)
            {
                continue;
            }
            let Some(p) = self.target_position(t) else {
                continue;
            };
            let d = [pos[0] - p[0], pos[1] - p[1], pos[2] - p[2]];
            let dist2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            let (at, v) = if dist2 > f64::from(range) * f64::from(range) {
                if min_volume <= 0.0 {
                    continue;
                }
                let dist = dist2.sqrt();
                (
                    [
                        p[0] + d[0] / dist * 2.0,
                        p[1] + d[1] / dist * 2.0,
                        p[2] + d[2] / dist * 2.0,
                    ],
                    min_volume,
                )
            } else {
                (pos, volume)
            };
            // The connection is sent the sound; the window's own player
            // plays it.
            let guest = match t {
                Target::Guest(conn) => Some(conn),
                _ => self.lan_acting,
            };
            match guest.and_then(|conn| Some((conn, self.lan.as_ref()?))) {
                Some((conn, host)) => {
                    let packet = host
                        .hub
                        .sound_packet(&sound, at, v, pitch, source as i32, seed);
                    if t == Target::Player {
                        self.guest_packets.push(packet);
                    } else {
                        host.hub.send_to(conn, &packet);
                    }
                }
                None => self.audio.play_on(source, &sound, Some(at), v, pitch),
            }
            tracker.mark(&holder(self.target_name(t)));
        }
        let name = Text::literal(sound.clone());
        let (message, total) = tracker.message(
            true,
            Some(err("commands.playsound.failed", &[])),
            |h, _| {
                Text::translate(
                    "commands.playsound.success.single",
                    &[name.clone(), h.text.clone()],
                )
            },
            |c, _| {
                Text::translate(
                    "commands.playsound.success.multiple",
                    &[name.clone(), num(c)],
                )
            },
        )?;
        self.feedback(message);
        Ok(total)
    }
    pub(super) fn cmd_stopsound(&mut self, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "targets", true)?;
        let source = ctx
            .literals
            .iter()
            .find_map(|l| SOUND_SOURCES.iter().position(|s| s == l));
        let sound = match ctx.get("sound") {
            Some(Arg::Resource(s)) => Some(s.clone()),
            _ => None,
        };
        // StopSoundCommand: each target's connection gets the packet; the
        // window's own player stops its sounds.
        let packet = C::StopSound(play::StopSound {
            source: source.map(|s| s as i32),
            sound: sound.clone(),
        });
        for &t in &targets {
            match t {
                Target::Player if self.lan_acting.is_some() => {
                    self.guest_packets.push(packet.clone());
                }
                Target::Player => self.audio.stop_sounds(sound.as_deref(), source),
                Target::Guest(conn) => {
                    if let Some(host) = self.lan.as_ref() {
                        host.hub.send_to(conn, &packet);
                    }
                }
                _ => {}
            }
        }
        let key = match (source, &sound) {
            (Some(_), Some(_)) => "commands.stopsound.success.source.sound",
            (Some(_), None) => "commands.stopsound.success.source.any",
            (None, Some(_)) => "commands.stopsound.success.sourceless.sound",
            (None, None) => "commands.stopsound.success.sourceless.any",
        };
        let mut args = Vec::new();
        if let Some(s) = &sound {
            args.push(Text::literal(s.clone()));
        }
        if let Some(i) = source {
            args.push(Text::literal(SOUND_SOURCES[i]));
        }
        self.feedback(Text::translate(key, &args));
        Ok(targets.len() as i32)
    }

    // ---- /particle ----------------------------------------------------------------------

    pub(super) fn cmd_particle(&mut self, ctx: &Context) -> R<i32> {
        use horizon_client::particles::{ParticleOptions, ParticleType};
        let Some(Arg::Str(spec)) = ctx.get("name").cloned() else {
            return Ok(0);
        };
        // ParticleArgument: `id` then optional SNBT options (`dust{color:[1,0,0],scale:2}`).
        let (name, options_text) = match spec.find('{') {
            Some(i) => (&spec[..i], &spec[i..]),
            None => (spec.as_str(), ""),
        };
        let ty = ParticleType::from_name(name).ok_or_else(|| err("particle.notFound", &[&name]))?;
        let opts = match horizon_client::command::snbt::parse_fully(options_text) {
            Ok(Tag::Compound(c)) if !options_text.is_empty() => Some(c),
            _ => None,
        };
        let field = |key: &str| opts.as_ref().and_then(|c| c.get(&key.into()).cloned());
        let number = |t: &Tag| -> Option<f32> {
            Some(match t {
                Tag::Byte(v) => f32::from(*v),
                Tag::Short(v) => f32::from(*v),
                Tag::Int(v) => *v as f32,
                Tag::Long(v) => *v as f32,
                Tag::Float(v) => f32::from_bits(*v),
                Tag::Double(v) => f64::from_bits(*v) as f32,
                _ => return None,
            })
        };
        let string = |t: &Tag| match t {
            Tag::String(s) => Some(s.to_string_lossy()),
            _ => None,
        };
        // ExtraCodecs.RGB_COLOR_CODEC: an int or a list of three floats.
        let rgb = match field("color") {
            Some(Tag::List(l)) if l.len() >= 3 => [
                number(&l[0]).unwrap_or(1.0),
                number(&l[1]).unwrap_or(0.0),
                number(&l[2]).unwrap_or(0.0),
            ],
            Some(Tag::Int(v)) => [
                ((v >> 16) & 255) as f32 / 255.0,
                ((v >> 8) & 255) as f32 / 255.0,
                (v & 255) as f32 / 255.0,
            ],
            _ => [1.0, 0.0, 0.0],
        };
        // The block state of a block particle, for its packet.
        let mut block_state = None;
        let options = match ty {
            ParticleType::Dust => ParticleOptions::dust(
                rgb,
                field("scale")
                    .as_ref()
                    .and_then(number)
                    .unwrap_or(1.0)
                    .clamp(0.01, 4.0),
            ),
            // ItemParticleOption: an item id, or a stack {id, ...}.
            ParticleType::Item => {
                let id = match field("item") {
                    Some(Tag::String(s)) => s.to_string_lossy(),
                    Some(Tag::Compound(c)) => {
                        c.get(&"id".into()).and_then(string).unwrap_or_default()
                    }
                    _ => String::new(),
                };
                let id = if id.contains(':') {
                    id
                } else {
                    format!("minecraft:{id}")
                };
                self.item_particle(&horizon_client::items::ItemStack::new(&id, 1))
            }
            ParticleType::ItemSlime | ParticleType::ItemSnowball | ParticleType::ItemCobweb => {
                self.ensure_item_particle_sprites();
                ParticleOptions::simple(ty)
            }
            ParticleType::Block
            | ParticleType::FallingDust
            | ParticleType::DustPillar
            | ParticleType::BlockMarker => {
                // BlockParticleOption: a block id, or {Name, Properties}.
                let text = match field("block_state") {
                    Some(Tag::String(s)) => s.to_string_lossy(),
                    Some(Tag::Compound(c)) => {
                        let name = c.get(&"Name".into()).and_then(string).unwrap_or_default();
                        let props: Vec<String> = match c.get(&"Properties".into()) {
                            Some(Tag::Compound(p)) => p
                                .iter()
                                .filter_map(|(k, v)| {
                                    string(v).map(|v| format!("{}={v}", k.to_string_lossy()))
                                })
                                .collect(),
                            _ => Vec::new(),
                        };
                        if props.is_empty() {
                            name
                        } else {
                            format!("{name}[{}]", props.join(","))
                        }
                    }
                    _ => String::new(),
                };
                let mut reader = horizon_client::command::Reader::new(&text);
                let block = horizon_client::command::args::ArgType::BlockState
                    .parse(&mut reader, &self.ui.registries);
                match block {
                    Ok(Arg::Block(b)) => {
                        let state = self.block_state(&b)?;
                        block_state = Some(state);
                        let at = self
                            .position_arg(ctx, "pos")
                            .unwrap_or_else(|| self.source_position())
                            .map(|c| c.floor() as i32);
                        let (sprite, tint) = self
                            .particle_sprite(state, at)
                            .unwrap_or_else(|| ("block/stone".into(), 0xFFFFFF));
                        ParticleOptions::block(ty, sprite, tint)
                    }
                    _ => ParticleOptions::simple(ty),
                }
            }
            _ => ParticleOptions::simple(ty),
        };
        let pos = self
            .position_arg(ctx, "pos")
            .unwrap_or_else(|| self.source_position());
        // Vec3Argument.getVec3: the spread as given (relative parts from the
        // source), not a position to measure from the source.
        let delta = self.position_arg(ctx, "delta").unwrap_or([0.0; 3]);
        let speed = match ctx.get("speed") {
            Some(Arg::Float(v)) => *v,
            _ => 0.0,
        };
        let count = match ctx.get("count") {
            Some(Arg::Int(c)) => *c,
            _ => 0,
        };
        // ParticleCommand.sendParticles: the viewers it reached, each within
        // 32 blocks (512 with `force`) of the particle's block centre.
        // Without viewers, every player (PlayerList.getPlayers).
        let viewers = if ctx.get("viewers").is_some() {
            self.entities(ctx, "viewers", true)?
        } else {
            let acting = self.lan_acting;
            let guests = self.lan.iter().flat_map(|h| h.hub.guests.iter());
            self.player
                .as_ref()
                .map(|_| Target::Player)
                .into_iter()
                .chain(
                    guests
                        .filter(|g| g.placed && Some(g.conn) != acting)
                        .map(|g| Target::Guest(g.conn)),
                )
                .collect()
        };
        let force = ctx.has("force");
        let limit = if force { 512.0 } else { 32.0 };
        let reached: Vec<Target> = viewers
            .into_iter()
            .filter(|&t| {
                self.target_position(t).is_some_and(|p| {
                    let c = p.map(|v| v.floor() + 0.5);
                    (c[0] - pos[0]).powi(2) + (c[1] - pos[1]).powi(2) + (c[2] - pos[2]).powi(2)
                        < limit * limit
                })
            })
            .collect();
        if reached.is_empty() {
            return Err(err("commands.particle.failed", &[]));
        }
        // ServerLevel.sendParticles: each viewer a server serves is sent the
        // particles (the player in place too when it is a guest); the
        // window's own player draws them.
        let packet = self
            .network_particle(&options, block_state)
            .map(|particle| {
                C::LevelParticles(play::LevelParticles {
                    particle,
                    override_limiter: force,
                    always_show: false,
                    pos,
                    dist: delta.map(|d| d as f32),
                    max_speed: [speed; 3],
                    count: count.max(0),
                    randomization: 0,
                })
            });
        let mut local = false;
        for &t in &reached {
            match (t, &packet) {
                (Target::Player, Some(p)) if self.lan_acting.is_some() => {
                    self.guest_packets.push(p.clone());
                }
                (Target::Player, _) => local = true,
                (Target::Guest(conn), Some(p)) => {
                    if let Some(host) = self.lan.as_ref() {
                        host.hub.send_to(conn, p);
                    }
                }
                _ => {}
            }
        }
        let reached = reached.len() as i32;
        if !local {
        } else if count <= 0 {
            // handleParticleEvent, count 0: one particle, the speed times
            // the delta as velocity (both sent as floats, multiplied as floats).
            self.particles
                .create(&options, pos, delta.map(|d| f64::from(speed * d as f32)));
        } else {
            for _ in 0..count {
                let r = self.particles.random();
                let p = [
                    pos[0] + r.next_gaussian() * delta[0],
                    pos[1] + r.next_gaussian() * delta[1],
                    pos[2] + r.next_gaussian() * delta[2],
                ];
                let v = [
                    r.next_gaussian() * f64::from(speed),
                    r.next_gaussian() * f64::from(speed),
                    r.next_gaussian() * f64::from(speed),
                ];
                self.particles.create(&options, p, v);
            }
        }
        self.feedback(Text::translate(
            "commands.particle.success",
            &[Text::literal(name.to_owned())],
        ));
        Ok(reached)
    }

    // ---- /attribute ---------------------------------------------------------------------

    /// The attribute id without `minecraft:`, as the attribute map names it.
    fn attribute_description(attribute: &str) -> Text {
        let short = attribute.trim_start_matches("minecraft:");
        Text::translate(&format!("attribute.name.{short}"), &[])
    }
    pub(super) fn cmd_attribute(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "target", false)?;
        let Some(target) = targets.first().copied() else {
            return Ok(0);
        };
        let Some(Arg::Resource(attribute)) = ctx.get("attribute").cloned() else {
            return Ok(0);
        };
        let name = self.target_name(target);
        let description = Self::attribute_description(&attribute);
        let short = attribute.trim_start_matches("minecraft:").to_owned();
        let no_attribute = |name: &Text, d: &Text| {
            text_error(
                "commands.attribute.failed.no_attribute",
                &[name.clone(), d.clone()],
            )
        };
        let scale = match ctx.get("scale") {
            Some(Arg::Double(s)) => *s,
            _ => 1.0,
        };
        let value_arg = match ctx.get("value") {
            Some(Arg::Double(v)) => *v,
            _ => 0.0,
        };
        let modifier_id = match ctx.get("id") {
            Some(Arg::Resource(m)) => m.clone(),
            _ => String::new(),
        };
        match target {
            Target::Player => {
                let Some(player) = self.player.as_mut() else {
                    return Ok(0);
                };
                let Some(key) = player.attributes.key(&attribute) else {
                    return Err(no_attribute(&name, &description));
                };
                let unavailable = |_| no_attribute(&name, &description);
                match id {
                    "attribute.get" | "attribute.base.get" => {
                        let kind = if id == "attribute.get" {
                            ValueKind::Computed
                        } else {
                            ValueKind::Base
                        };
                        let v = player
                            .attributes
                            .value(Some(key), kind)
                            .map_err(unavailable)?;
                        let k = if id == "attribute.get" {
                            "commands.attribute.value.get.success"
                        } else {
                            "commands.attribute.base_value.get.success"
                        };
                        self.feedback(Text::translate(
                            k,
                            &[description, name, java_double_text(v)],
                        ));
                        Ok((v * scale) as i32)
                    }
                    "attribute.base.set" => {
                        let handle = player
                            .attributes
                            .get_instance(Some(key))
                            .ok()
                            .flatten()
                            .ok_or_else(|| no_attribute(&name, &description))?;
                        let _ = player.attributes.edit(&handle, |value, observer| {
                            value.set_base_value(value_arg, observer)
                        });
                        self.feedback(Text::translate(
                            "commands.attribute.base_value.set.success",
                            &[description, name, java_double_text(value_arg)],
                        ));
                        Ok(1)
                    }
                    "attribute.base.reset" => {
                        if !player
                            .attributes
                            .reset_base(Some(key.clone()))
                            .unwrap_or(false)
                        {
                            return Err(no_attribute(&name, &description));
                        }
                        let v = player
                            .attributes
                            .value(Some(key.clone()), ValueKind::Base)
                            .unwrap_or(0.0);
                        self.feedback(Text::translate(
                            "commands.attribute.base_value.reset.success",
                            &[description, name, java_double_text(v)],
                        ));
                        Ok(1)
                    }
                    "attribute.modifier.add" => {
                        if player
                            .attributes
                            .has_modifier(Some(key), &modifier_id)
                            .unwrap_or(false)
                        {
                            return Err(text_error(
                                "commands.attribute.failed.modifier_already_present",
                                &[
                                    Text::literal(modifier_id.clone()),
                                    description.clone(),
                                    name.clone(),
                                ],
                            ));
                        }
                        let op = ctx
                            .literals
                            .last()
                            .cloned()
                            .unwrap_or_else(|| "add_value".into());
                        player
                            .set_modifier(&attribute, &modifier_id, value_arg, &op, true)
                            .map_err(|_| no_attribute(&name, &description))?;
                        self.feedback(Text::translate(
                            "commands.attribute.modifier.add.success",
                            &[Text::literal(modifier_id), description, name],
                        ));
                        Ok(1)
                    }
                    "attribute.modifier.remove" => {
                        if !player
                            .attributes
                            .has_modifier(Some(key), &modifier_id)
                            .unwrap_or(false)
                        {
                            return Err(text_error(
                                "commands.attribute.failed.no_modifier",
                                &[
                                    description.clone(),
                                    name.clone(),
                                    Text::literal(modifier_id.clone()),
                                ],
                            ));
                        }
                        let _ =
                            player.set_modifier(&attribute, &modifier_id, 0.0, "add_value", false);
                        self.feedback(Text::translate(
                            "commands.attribute.modifier.remove.success",
                            &[Text::literal(modifier_id), description, name],
                        ));
                        Ok(1)
                    }
                    _ => {
                        let Some(handle) = player.attributes.get_instance(Some(key)).ok().flatten()
                        else {
                            return Err(no_attribute(&name, &description));
                        };
                        let found = player
                            .attributes
                            .edit(&handle, |value, _| {
                                Ok(value
                                    .modifiers()
                                    .iter()
                                    .find(|m| m.id() == modifier_id)
                                    .map(|m| m.amount()))
                            })
                            .ok()
                            .flatten();
                        let Some(v) = found else {
                            return Err(text_error(
                                "commands.attribute.failed.no_modifier",
                                &[
                                    description.clone(),
                                    name.clone(),
                                    Text::literal(modifier_id.clone()),
                                ],
                            ));
                        };
                        self.feedback(Text::translate(
                            "commands.attribute.modifier.value.get.success",
                            &[
                                Text::literal(modifier_id),
                                description,
                                name,
                                java_double_text(v),
                            ],
                        ));
                        Ok((v * scale) as i32)
                    }
                }
            }
            Target::Mob(_) | Target::Stand(_)
                if !(matches!(target, Target::Mob(_))
                    && matches!(
                        short.as_str(),
                        "max_health" | "movement_speed" | "attack_damage"
                    )) =>
            {
                let op = ctx
                    .literals
                    .last()
                    .cloned()
                    .unwrap_or_else(|| "add_value".into());
                self.generic_attribute(
                    target,
                    id,
                    &attribute,
                    (&name, &description),
                    (value_arg, scale, &modifier_id, &op),
                )
            }
            Target::Mob(mob_id) => {
                let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob_id) else {
                    return Ok(0);
                };
                // Mobs keep three attributes as plain values.
                let slot: Option<f64> = match short.as_str() {
                    "max_health" => Some(f64::from(m.attrs.max_health)),
                    "movement_speed" => Some(m.attrs.speed),
                    "attack_damage" => Some(f64::from(m.attrs.attack_damage)),
                    _ => None,
                };
                let Some(current) = slot else {
                    return Err(no_attribute(&name, &description));
                };
                match id {
                    "attribute.get" | "attribute.base.get" => {
                        let k = if id == "attribute.get" {
                            "commands.attribute.value.get.success"
                        } else {
                            "commands.attribute.base_value.get.success"
                        };
                        self.feedback(Text::translate(
                            k,
                            &[description, name, java_double_text(current)],
                        ));
                        Ok((current * scale) as i32)
                    }
                    "attribute.base.set" => {
                        match short.as_str() {
                            "max_health" => {
                                m.attrs.max_health = value_arg as f32;
                                m.health = m.health.min(m.attrs.max_health);
                            }
                            "movement_speed" => m.attrs.speed = value_arg,
                            _ => m.attrs.attack_damage = value_arg as f32,
                        }
                        self.feedback(Text::translate(
                            "commands.attribute.base_value.set.success",
                            &[description, name, java_double_text(value_arg)],
                        ));
                        Ok(1)
                    }
                    "attribute.base.reset" => {
                        match short.as_str() {
                            "max_health" => m.attrs.max_health = m.kind.max_health,
                            "movement_speed" => m.attrs.speed = m.kind.speed,
                            _ => m.attrs.attack_damage = m.kind.attack_damage,
                        }
                        let v = match short.as_str() {
                            "max_health" => f64::from(m.attrs.max_health),
                            "movement_speed" => m.attrs.speed,
                            _ => f64::from(m.attrs.attack_damage),
                        };
                        self.feedback(Text::translate(
                            "commands.attribute.base_value.reset.success",
                            &[description, name, java_double_text(v)],
                        ));
                        Ok(1)
                    }
                    _ => Err(text_error(
                        "commands.attribute.failed.no_modifier",
                        &[
                            description.clone(),
                            name.clone(),
                            Text::literal(modifier_id.clone()),
                        ],
                    )),
                }
            }
            _ => Err(text_error(
                "commands.attribute.failed.entity",
                std::slice::from_ref(&name),
            )),
        }
    }

    /// The attribute map of a living entity whose attributes Horizon does
    /// not model, created from its type's defaults on first use.
    pub(in crate::game) fn generic_attributes(
        &mut self,
        t: Target,
    ) -> Option<&mut horizon_world::attributes::entity::EntityAttributes> {
        if !self.cmds.attributes.contains_key(&t) {
            let ty = self.target_type(t);
            let map = self.motion.attributes.create(&ty)?;
            self.cmds.attributes.insert(t, map);
        }
        self.cmds.attributes.get_mut(&t)
    }
    /// AttributeCommand on a living entity through its generic attribute map.
    fn generic_attribute(
        &mut self,
        target: Target,
        id: &str,
        attribute: &str,
        (name, description): (&Text, &Text),
        (value_arg, scale, modifier_id, op): (f64, f64, &str, &str),
    ) -> R<i32> {
        use horizon_world::attributes::{Modifier, Operation};
        let no_attribute = || {
            text_error(
                "commands.attribute.failed.no_attribute",
                &[name.clone(), description.clone()],
            )
        };
        let no_modifier = || {
            text_error(
                "commands.attribute.failed.no_modifier",
                &[
                    description.clone(),
                    name.clone(),
                    Text::literal(modifier_id),
                ],
            )
        };
        let catalog = self.motion.attributes.clone();
        let key = catalog.id(attribute).ok_or_else(no_attribute)?;
        let map = self.generic_attributes(target).ok_or_else(no_attribute)?;
        if !map.has(key) {
            return Err(no_attribute());
        }
        let (message, args, result) = match id {
            "attribute.get" | "attribute.base.get" => {
                let v = if id == "attribute.get" {
                    map.value(key).map_err(|_| no_attribute())?
                } else {
                    map.base_value(key).map_err(|_| no_attribute())?
                };
                let k = if id == "attribute.get" {
                    "commands.attribute.value.get.success"
                } else {
                    "commands.attribute.base_value.get.success"
                };
                (
                    k,
                    vec![description.clone(), name.clone(), java_double_text(v)],
                    (v * scale) as i32,
                )
            }
            "attribute.base.set" => {
                map.edit(key, |a, o| {
                    let _ = a.set_base_value(value_arg, o);
                });
                (
                    "commands.attribute.base_value.set.success",
                    vec![
                        description.clone(),
                        name.clone(),
                        java_double_text(value_arg),
                    ],
                    1,
                )
            }
            "attribute.base.reset" => {
                map.reset_base(key);
                let v = map.base_value(key).unwrap_or(0.0);
                (
                    "commands.attribute.base_value.reset.success",
                    vec![description.clone(), name.clone(), java_double_text(v)],
                    1,
                )
            }
            "attribute.modifier.add" => {
                if map.has_modifier(key, modifier_id) {
                    return Err(text_error(
                        "commands.attribute.failed.modifier_already_present",
                        &[
                            Text::literal(modifier_id),
                            description.clone(),
                            name.clone(),
                        ],
                    ));
                }
                let operation = match op {
                    "add_multiplied_base" => Operation::AddMultipliedBase,
                    "add_multiplied_total" => Operation::AddMultipliedTotal,
                    _ => Operation::AddValue,
                };
                let m =
                    Modifier::new(modifier_id, value_arg, operation).map_err(|_| no_attribute())?;
                map.edit(key, |a, o| {
                    let _ = a.add_permanent(m, o);
                });
                (
                    "commands.attribute.modifier.add.success",
                    vec![
                        Text::literal(modifier_id),
                        description.clone(),
                        name.clone(),
                    ],
                    1,
                )
            }
            "attribute.modifier.remove" => {
                if !map.has_modifier(key, modifier_id) {
                    return Err(no_modifier());
                }
                map.edit(key, |a, o| {
                    let _ = a.remove(modifier_id, o);
                });
                (
                    "commands.attribute.modifier.remove.success",
                    vec![
                        Text::literal(modifier_id),
                        description.clone(),
                        name.clone(),
                    ],
                    1,
                )
            }
            _ => {
                let v = map
                    .modifier_value(key, modifier_id)
                    .map_err(|_| no_modifier())?;
                (
                    "commands.attribute.modifier.value.get.success",
                    vec![
                        Text::literal(modifier_id),
                        description.clone(),
                        name.clone(),
                        java_double_text(v),
                    ],
                    (v * scale) as i32,
                )
            }
        };
        self.feedback(Text::translate(message, &args));
        Ok(result)
    }

    // ---- /ride ----------------------------------------------------------------------------

    pub(super) fn cmd_ride(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "target", false)?;
        let Some(target) = targets.first().copied() else {
            return Ok(0);
        };
        let name = self.target_name(target);
        if id == "ride.dismount" {
            let Some(v) = self.ride_vehicle(target) else {
                return Err(err("commands.ride.not_riding", &[&name.plain()]));
            };
            let vehicle_name = self.target_name(v);
            match target {
                Target::Player => self.dismount_mount(),
                Target::Mob(rider) => self.dismount_mob(rider),
                _ => {}
            }
            self.feedback(Text::translate(
                "commands.ride.dismount.success",
                &[name, vehicle_name],
            ));
            return Ok(1);
        }
        let vehicles = self.entities(ctx, "vehicle", false)?;
        let Some(vehicle) = vehicles.first().copied() else {
            return Ok(0);
        };
        let vehicle_name = self.target_name(vehicle);
        if let Some(current) = self.ride_vehicle(target) {
            let current = self.target_name(current);
            return Err(err(
                "commands.ride.already_riding",
                &[&name.plain(), &current.plain()],
            ));
        }
        if vehicle == Target::Player {
            return Err(err("commands.ride.mount.failure.cant_ride_players", &[]));
        }
        // getSelfAndPassengers: the vehicle is the target or rides it.
        let mut up = Some(vehicle);
        while let Some(v) = up {
            if v == target {
                return Err(err("commands.ride.mount.failure.loop", &[]));
            }
            up = self.ride_vehicle(v);
        }
        // Players and mobs can ride mobs.
        let Target::Mob(mob_id) = vehicle else {
            return Err(err(
                "commands.ride.mount.failure.generic",
                &[&name.plain(), &vehicle_name.plain()],
            ));
        };
        let rider = match target {
            Target::Mob(id) => Some(id),
            _ => None,
        };
        if !(target == Target::Player || rider.is_some())
            || !self.mobs.iter().any(|m| m.id == mob_id && m.alive())
        {
            return Err(err(
                "commands.ride.mount.failure.generic",
                &[&name.plain(), &vehicle_name.plain()],
            ));
        }
        if let Some(rider) = rider {
            // Entity.startRiding.
            self.mount_mob(rider, mob_id);
            self.feedback(Text::translate(
                "commands.ride.mount.success",
                &[name, vehicle_name],
            ));
            return Ok(1);
        }
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob_id) {
            m.extra.flag = false;
            m.extra.counter = 0;
            self.yaw = m.yaw;
        }
        self.riding = Some(mob_id);
        self.mount.jump_ticks = 0;
        self.mount.jump_scale = 0.0;
        self.feedback(Text::translate(
            "commands.ride.mount.success",
            &[name, vehicle_name],
        ));
        Ok(1)
    }

    /// Entity.getVehicle: the player's mount, or a mob's.
    pub(super) fn ride_vehicle(&self, t: Target) -> Option<Target> {
        let id = match t {
            Target::Player => return self.riding.map(Target::Mob),
            Target::Mob(id) => self.mobs.iter().find(|m| m.id == id)?.extra.mount?,
            _ => return None,
        };
        Some(if self.boats.iter().any(|b| b.id == id) {
            Target::Boat(id)
        } else {
            Target::Mob(id)
        })
    }
    /// LivingEntity.stopRiding for a mob: off its vehicle
    /// (Entity.removePassenger), then to where the vehicle lets it out
    /// (LivingEntity.dismountVehicle).
    pub(in crate::game) fn dismount_mob(&mut self, rider: u32) {
        let Some(i) = self.mobs.iter().position(|m| m.id == rider) else {
            return;
        };
        let Some(v) = self.mobs[i].extra.mount.take() else {
            return;
        };
        let (width, yaw) = (f64::from(self.mobs[i].width()), self.mobs[i].yaw);
        let at = if let Some(b) = self.boats.iter().position(|b| b.id == v) {
            self.boat_dismount_target(b, width, yaw)
        } else if let Some(m) = self.mobs.iter_mut().find(|m| m.id == v) {
            m.extra.rider = None;
            // Entity.getDismountLocationForPassenger: on top of the vehicle.
            let p = m.position;
            horizon_core::geometry::Vec3::new(p.x, p.y + f64::from(m.height()), p.z)
        } else {
            self.mobs[i].position
        };
        let m = &mut self.mobs[i];
        m.extra.passenger = false;
        m.extra.boarding_cooldown = super::super::boat_passengers::BOARDING_COOLDOWN;
        m.position = at;
        m.previous = at;
    }

    // ---- /spreadplayers -----------------------------------------------------------------------

    pub(super) fn cmd_spreadplayers(&mut self, ctx: &Context) -> R<i32> {
        let centre = self
            .position_arg(ctx, "center")
            .unwrap_or_else(|| self.source_position());
        let float = |name: &str| match ctx.get(name) {
            Some(Arg::Float(v)) => f64::from(*v),
            _ => 0.0,
        };
        let (spread, range) = (float("spreadDistance"), float("maxRange"));
        let respect = matches!(ctx.get("respectTeams"), Some(Arg::Bool(true)));
        let max_y = match ctx.get("maxHeight") {
            Some(Arg::Int(h)) => *h,
            _ => horizon_client::chunk::MIN_Y + horizon_client::chunk::HEIGHT,
        };
        if max_y < horizon_client::chunk::MIN_Y {
            return Err(err(
                "commands.spreadplayers.failed.invalid.height",
                &[&max_y, &horizon_client::chunk::MIN_Y],
            ));
        }
        let targets = self.entities(ctx, "targets", false)?;
        if targets.is_empty() {
            return Err(err("argument.entity.notfound.entity", &[]));
        }
        // SpreadPlayersCommand: one position per entity, or per team (every
        // non-player and team-less player sharing one) with respectTeams.
        let team_of = |g: &mut Self, t: Target| -> Option<String> {
            if t != Target::Player {
                return None;
            }
            g.holder_name(t)
                .and_then(|h| g.score.board.players_team(&h).map(|x| x.name.clone()))
        };
        let count = if respect {
            let mut seen: Vec<Option<String>> = Vec::new();
            for &t in &targets {
                let team = team_of(self, t);
                if !seen.contains(&team) {
                    seen.push(team);
                }
            }
            seen.len()
        } else {
            targets.len()
        };
        let centre_f = [centre[0] as f32, centre[2] as f32];
        let (cx, cz) = (f64::from(centre_f[0]), f64::from(centre_f[1]));
        let (min_x, max_x) = (cx - range, cx + range);
        let (min_z, max_z) = (cz - range, cz + range);
        // Mth.nextDouble.
        let next = |g: &mut Self, lo: f64, hi: f64| {
            if lo >= hi {
                lo
            } else {
                g.world.random.next_double() * (hi - lo) + lo
            }
        };
        let mut points: Vec<[f64; 2]> = Vec::with_capacity(count);
        for _ in 0..count {
            let x = next(self, min_x, max_x);
            let z = next(self, min_z, max_z);
            points.push([x, z]);
        }
        let mut collisions = true;
        let mut min_distance = f64::from(f32::MAX);
        let mut iteration = 0;
        while iteration < 10_000 && collisions {
            collisions = false;
            min_distance = f64::from(f32::MAX);
            for i in 0..points.len() {
                let mut neighbours = 0;
                let mut avg = [0.0f64, 0.0f64];
                for j in 0..points.len() {
                    if i == j {
                        continue;
                    }
                    let (dx, dz) = (points[i][0] - points[j][0], points[i][1] - points[j][1]);
                    let dist = (dx * dx + dz * dz).sqrt();
                    min_distance = min_distance.min(dist);
                    if dist < spread {
                        neighbours += 1;
                        avg[0] += points[j][0] - points[i][0];
                        avg[1] += points[j][1] - points[i][1];
                    }
                }
                if neighbours > 0 {
                    avg[0] /= f64::from(neighbours);
                    avg[1] /= f64::from(neighbours);
                    let length = (avg[0] * avg[0] + avg[1] * avg[1]).sqrt();
                    if length > 0.0 {
                        points[i][0] -= avg[0] / length;
                        points[i][1] -= avg[1] / length;
                    } else {
                        points[i][0] = next(self, min_x, max_x);
                        points[i][1] = next(self, min_z, max_z);
                    }
                    collisions = true;
                }
                let p = &mut points[i];
                let before = *p;
                p[0] = if p[0] < min_x {
                    min_x
                } else if p[0] > max_x {
                    max_x
                } else {
                    p[0]
                };
                p[1] = if p[1] < min_z {
                    min_z
                } else if p[1] > max_z {
                    max_z
                } else {
                    p[1]
                };
                if before != *p {
                    collisions = true;
                }
            }
            if !collisions {
                for i in 0..points.len() {
                    if !self.spread_safe(points[i], max_y) {
                        points[i][0] = next(self, min_x, max_x);
                        points[i][1] = next(self, min_z, max_z);
                        collisions = true;
                    }
                }
            }
            iteration += 1;
        }
        if min_distance == f64::from(f32::MAX) {
            min_distance = 0.0;
        }
        let float = |v: f32| Text::literal(horizon_client::snbt::java_float(v));
        if iteration >= 10_000 {
            let key = if respect {
                "commands.spreadplayers.failed.teams"
            } else {
                "commands.spreadplayers.failed.entities"
            };
            return Err(text_error(
                key,
                &[
                    num(points.len()),
                    float(centre_f[0]),
                    float(centre_f[1]),
                    Text::literal(format!("{min_distance:.2}")),
                ],
            ));
        }
        // setPlayerPositions.
        let mut total = 0.0;
        let mut index = 0;
        let mut team_points: Vec<(Option<String>, usize)> = Vec::new();
        for &t in &targets {
            let i = if respect {
                let team = team_of(self, t);
                match team_points.iter().find(|(k, _)| *k == team) {
                    Some((_, i)) => *i,
                    None => {
                        team_points.push((team, index));
                        index += 1;
                        index - 1
                    }
                }
            } else {
                index += 1;
                index - 1
            };
            let p = points[i];
            let y = self.spread_spawn_y(p, max_y);
            self.teleport_to(
                t,
                [p[0].floor() + 0.5, f64::from(y), p[1].floor() + 0.5],
                None,
            );
            let closest = points
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, q)| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt())
                .fold(f64::MAX, f64::min);
            total += closest;
        }
        let average = if targets.len() < 2 {
            0.0
        } else {
            total / targets.len() as f64
        };
        let key = if respect {
            "commands.spreadplayers.success.teams"
        } else {
            "commands.spreadplayers.success.entities"
        };
        self.feedback(Text::translate(
            key,
            &[
                num(points.len()),
                float(centre_f[0]),
                float(centre_f[1]),
                Text::literal(format!("{average:.2}")),
            ],
        ));
        Ok(points.len() as i32)
    }
    /// `SpreadPlayersCommand.Position.getSpawnY`.
    fn spread_spawn_y(&self, p: [f64; 2], max_y: i32) -> i32 {
        let (x, z) = (p[0].floor() as i32, p[1].floor() as i32);
        let air = |y: i32| {
            let name: &str = &self.res.facts().get(self.world.block(x, y, z)).name;
            matches!(
                name,
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            )
        };
        let mut y = max_y + 1;
        let mut air2 = air(y);
        y -= 1;
        let mut air1 = air(y);
        while y > horizon_client::chunk::MIN_Y {
            y -= 1;
            let current = air(y);
            if !current && air1 && air2 {
                return y + 1;
            }
            air2 = air1;
            air1 = current;
        }
        max_y + 1
    }
    /// `SpreadPlayersCommand.Position.isSafe`.
    fn spread_safe(&self, p: [f64; 2], max_y: i32) -> bool {
        let y = self.spread_spawn_y(p, max_y) - 1;
        let (x, z) = (p[0].floor() as i32, p[1].floor() as i32);
        let name = self.res.facts().get(self.world.block(x, y, z)).name.clone();
        let liquid = matches!(&*name, "minecraft:water" | "minecraft:lava");
        let teleportable = self
            .res
            .items()
            .block_tags
            .get("entities_can_teleport_to")
            .or_else(|| {
                self.res
                    .items()
                    .block_tags
                    .get("minecraft:entities_can_teleport_to")
            })
            .is_some_and(|set| {
                set.contains(&*name) || set.contains(name.trim_start_matches("minecraft:"))
            });
        y < max_y && !liquid && teleportable
    }
}

/// A `CommandError` whose message is a translation with component arguments.
fn text_error(key: &str, args: &[Text]) -> horizon_client::command::CommandError {
    horizon_client::command::CommandError::new(Text::translate(key, args))
}

/// `Entity.lookAt`: yaw and pitch from `from` towards `to`.
fn look_at(from: [f64; 3], to: [f64; 3]) -> [f32; 2] {
    let (dx, dy, dz) = (to[0] - from[0], to[1] - from[1], to[2] - from[2]);
    let horizontal = (dx * dx + dz * dz).sqrt();
    let pitch = (-(dy.atan2(horizontal).to_degrees())) as f32;
    let yaw = (dz.atan2(dx).to_degrees() - 90.0) as f32;
    [yaw.rem_euclid(360.0), pitch]
}
