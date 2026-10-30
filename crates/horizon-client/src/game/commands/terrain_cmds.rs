//! `/forceload`, `/fillbiome`, `/worldborder` and `/loot`.
use super::{Arg, Context, Game, R, Target, err, items::Owner};
use horizon_client::{
    chunk::{HEIGHT, MIN_Y},
    command::CommandError,
    items::ItemStack,
    scoreboard_cmd::{Holder, Tracker},
    text::Text,
    worldborder::MAX_SIZE,
};
use horizon_core::random::RandomSource;

fn num(n: impl std::fmt::Display) -> Text {
    Text::literal(n.to_string())
}
fn chunk_text(x: i32, z: i32) -> Text {
    Text::literal(format!("[{x}, {z}]"))
}
fn holder(text: Text) -> Holder {
    Holder {
        name: String::new(),
        text,
        display: None,
    }
}
fn ticks_to_seconds(ticks: i64) -> String {
    format!("{:.2}", ticks as f64 / 20.0)
}

impl Game<'_> {
    // ---- /forceload ----------------------------------------------------------

    pub(super) fn column_arg(&self, ctx: &Context, name: &str) -> R<[i32; 2]> {
        let c = self
            .coords(ctx, name)
            .ok_or_else(|| err("argument.pos2d.incomplete", &[]))?;
        let p = self.resolve_coords(&c);
        Ok([p[0].floor() as i32, p[2].floor() as i32])
    }
    pub(super) fn cmd_forceload(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let dim = Text::literal(self.dim.id());
        match id {
            "forceload.query" if ctx.get("pos").is_none() => {
                let mut forced: Vec<(i32, i32)> = self.streaming.forced.iter().copied().collect();
                forced.sort_unstable();
                if forced.is_empty() {
                    self.command_failure(Text::translate("commands.forceload.added.none", &[dim]));
                    return Ok(0);
                }
                let list = forced
                    .iter()
                    .map(|(x, z)| format!("[{x}, {z}]"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let message = if forced.len() == 1 {
                    Text::translate(
                        "commands.forceload.list.single",
                        &[dim, Text::literal(list)],
                    )
                } else {
                    Text::translate(
                        "commands.forceload.list.multiple",
                        &[num(forced.len()), dim, Text::literal(list)],
                    )
                };
                self.feedback(message);
                Ok(forced.len() as i32)
            }
            "forceload.query" => {
                let [x, z] = self.column_arg(ctx, "pos")?;
                let key = (x >> 4, z >> 4);
                if self.streaming.forced.contains(&key) {
                    self.feedback(Text::translate(
                        "commands.forceload.query.success",
                        &[chunk_text(key.0, key.1), dim],
                    ));
                    Ok(1)
                } else {
                    Err(err(
                        "commands.forceload.query.failure",
                        &[&chunk_text(key.0, key.1).plain(), &dim.plain()],
                    ))
                }
            }
            "forceload.remove.all" => {
                self.streaming.forced.clear();
                self.feedback(Text::translate("commands.forceload.removed.all", &[dim]));
                Ok(0)
            }
            _ => {
                let add = id == "forceload.add";
                let from = self.column_arg(ctx, "from")?;
                let to = if ctx.get("to").is_some() {
                    self.column_arg(ctx, "to")?
                } else {
                    from
                };
                let (min_x, max_x) = (from[0].min(to[0]), from[0].max(to[0]));
                let (min_z, max_z) = (from[1].min(to[1]), from[1].max(to[1]));
                if min_x < -30_000_000
                    || min_z < -30_000_000
                    || max_x >= 30_000_000
                    || max_z >= 30_000_000
                {
                    return Err(err("argument.pos.outofworld", &[]));
                }
                let (cx0, cx1, cz0, cz1) = (min_x >> 4, max_x >> 4, min_z >> 4, max_z >> 4);
                let count = i64::from(cx1 - cx0 + 1) * i64::from(cz1 - cz0 + 1);
                if count > 256 {
                    return Err(err("commands.forceload.toobig", &[&256, &count]));
                }
                let mut tracker = Tracker::default();
                for x in cx0..=cx1 {
                    for z in cz0..=cz1 {
                        let changed = if add {
                            self.streaming.forced.insert((x, z))
                        } else {
                            self.streaming.forced.remove(&(x, z))
                        };
                        tracker.track(&holder(chunk_text(x, z)), i32::from(changed));
                    }
                }
                if add {
                    self.streaming.rank_forced(cx0, cz0, cx1, cz1);
                }
                let (lo, hi) = (chunk_text(cx0, cz0), chunk_text(cx1, cz1));
                let verb = if add { "added" } else { "removed" };
                let (message, total) = tracker.message(
                    false,
                    Some(err(&format!("commands.forceload.{verb}.failure"), &[])),
                    |h, _| {
                        Text::translate(
                            &format!("commands.forceload.{verb}.single"),
                            &[h.text.clone(), dim.clone()],
                        )
                    },
                    |c, _| {
                        Text::translate(
                            &format!("commands.forceload.{verb}.multiple"),
                            &[num(c), dim.clone(), lo.clone(), hi.clone()],
                        )
                    },
                )?;
                self.feedback(message);
                Ok(total)
            }
        }
    }

    // ---- /fillbiome ----------------------------------------------------------------

    pub(super) fn cmd_fillbiome(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Resource(biome)) = ctx.get("biome").cloned() else {
            return Ok(0);
        };
        // BlockPosArgument.getLoadedBlockPos on both corners first.
        let (a, b) = (
            self.block_pos_arg(ctx, "from")?,
            self.block_pos_arg(ctx, "to")?,
        );
        // Quantize to biome cells (`QuartPos`), then work with the box.
        let q = |v: i32| (v >> 2) << 2;
        let lo = [
            q(a[0]).min(q(b[0])),
            q(a[1]).min(q(b[1])),
            q(a[2]).min(q(b[2])),
        ];
        let hi = [
            q(a[0]).max(q(b[0])),
            q(a[1]).max(q(b[1])),
            q(a[2]).max(q(b[2])),
        ];
        let volume = i64::from(hi[0] - lo[0] + 1)
            * i64::from(hi[1] - lo[1] + 1)
            * i64::from(hi[2] - lo[2] + 1);
        let limit = self.ui.rules.int("max_block_modifications");
        if volume > i64::from(limit) {
            return Err(err("commands.fillbiome.toobig", &[&limit, &volume]));
        }
        let Some(target) = self.res.biomes().list.iter().position(|b| b.name == biome) else {
            return Err(err(
                "argument.resource.not_found",
                &[&biome, &"minecraft:worldgen/biome"],
            ));
        };
        let filter = match ctx.get("filter") {
            Some(Arg::Resource(f)) => Some(f.clone()),
            _ => None,
        };
        for cz in (lo[2] >> 4)..=(hi[2] >> 4) {
            for cx in (lo[0] >> 4)..=(hi[0] >> 4) {
                if !self.world.chunks.contains_key(&(cx, cz)) {
                    return Err(err("argument.pos.unloaded", &[]));
                }
            }
        }
        let mut changed = 0;
        let mut touched = std::collections::HashSet::new();
        for z in (lo[2]..=hi[2]).step_by(4) {
            for y in (lo[1]..=hi[1]).step_by(4) {
                for x in (lo[0]..=hi[0]).step_by(4) {
                    let key = (x >> 4, z >> 4);
                    let Some(chunk) = self.world.chunks.get(&key) else {
                        continue;
                    };
                    let current = chunk.biome(x, y, z);
                    let current_name = &self.res.biomes().list[usize::from(current)].name;
                    if let Some(f) = &filter
                        && !self.biome_matches(f, current_name)
                    {
                        continue;
                    }
                    if usize::from(current) == target {
                        continue;
                    }
                    let qy = ((y - MIN_Y) >> 2).clamp(0, HEIGHT / 4 - 1);
                    if let Some(chunk) = self.world.chunks.get_mut(&key) {
                        let chunk = std::sync::Arc::make_mut(chunk);
                        chunk.set_biome_cell((x & 15) >> 2, qy, (z & 15) >> 2, target as u16);
                    }
                    touched.insert(key);
                    changed += 1;
                }
            }
        }
        if changed == 0 {
            return Err(err("commands.fillbiome.no_changes", &[]));
        }
        for key in &touched {
            self.world.modified.insert(*key);
            for sy in 0..horizon_client::chunk::SECTIONS as i32 {
                self.world.dirty_sections.insert((key.0, sy, key.1));
            }
        }
        self.feedback(Text::translate(
            "commands.fillbiome.success.count",
            &[
                num(changed),
                num(lo[0]),
                num(lo[1]),
                num(lo[2]),
                num(hi[0]),
                num(hi[1]),
                num(hi[2]),
            ],
        ));
        Ok(changed)
    }

    // ---- /worldborder -----------------------------------------------------------------------

    /// Every tick: the size change, the wall the player cannot pass and
    /// damage outside the border.
    /// `BorderChangeListener.DelegateBorderChangeListener`'s packets: what
    /// changed on the border since the guests were last told of it.
    pub(in crate::game) fn lan_sync_border(&mut self, host: &mut crate::game::lan::LanHost) {
        let now = horizon_client::lan::border::Sent::of(&self.border);
        if let Some(sent) = host.border_sent.replace(now) {
            for packet in sent.changes(&self.border) {
                host.hub.broadcast(&packet);
            }
        }
    }
    pub(in crate::game) fn tick_worldborder(&mut self) {
        self.border.tick();
        self.player_border();
        self.border_damage();
    }
    /// The player's movement collides with the border as it is now
    /// (`CollisionGetter.borderCollision`): from inside, its walls stop it.
    pub(in crate::game) fn player_border(&mut self) {
        if let Some(p) = self.player.as_mut() {
            p.border = Some(self.border.geometry().clone());
        }
    }
    /// `LivingEntity.baseTick`: a player outside the border, further than
    /// its safe zone, is hurt by the blocks beyond it. The game's player, or
    /// a guest's in their turn (`as_guest`).
    pub(in crate::game) fn border_damage(&mut self) {
        let Some((px, pz, half)) = self.player.as_ref().map(|p| {
            (
                p.position.x,
                p.position.z,
                f64::from(p.dimensions.width) / 2.0,
            )
        }) else {
            return;
        };
        let inside = self.border.contains(px - half, pz - half)
            && self.border.contains(px + half, pz + half);
        if inside {
            return;
        }
        let dist = self.border.distance(px, pz) + self.border.safe_zone;
        if dist < 0.0 && self.border.damage_per_block > 0.0 && self.ticks.is_multiple_of(10) {
            let amount = (-dist * self.border.damage_per_block).floor().max(1.0) as f32;
            self.hurt_player_by("minecraft:outside_border", amount);
        }
    }
    pub(super) fn cmd_worldborder(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let float2 = |v: f64| format!("{v:.2}");
        let double = match ctx.get("distance") {
            Some(Arg::Double(d)) => *d,
            _ => 0.0,
        };
        let time = match ctx.get("time") {
            Some(Arg::Int(t)) => i64::from(*t),
            _ => 0,
        };
        match id {
            "worldborder.get" => {
                let size = self.border.size();
                self.feedback(Text::translate(
                    "commands.worldborder.get",
                    &[Text::literal(format!("{size:.0}"))],
                ));
                Ok((size + 0.5).floor() as i32)
            }
            "worldborder.set" | "worldborder.add" => {
                let current = self.border.size();
                let distance = if id == "worldborder.add" {
                    current + double
                } else {
                    double
                };
                if current == distance {
                    return Err(err("commands.worldborder.set.failed.nochange", &[]));
                }
                if distance < 1.0 {
                    return Err(err("commands.worldborder.set.failed.small", &[]));
                }
                if distance > MAX_SIZE {
                    return Err(err("commands.worldborder.set.failed.big", &[&"5.999997E7"]));
                }
                let shown = Text::literal(format!("{distance:.1}"));
                if time > 0 {
                    self.border.lerp_to(distance, time);
                    let key = if distance > current {
                        "commands.worldborder.set.grow"
                    } else {
                        "commands.worldborder.set.shrink"
                    };
                    self.feedback(Text::translate(
                        key,
                        &[shown, Text::literal(ticks_to_seconds(time))],
                    ));
                } else {
                    self.border.set_size(distance);
                    self.feedback(Text::translate(
                        "commands.worldborder.set.immediate",
                        &[shown],
                    ));
                }
                Ok((distance - current) as i32)
            }
            "worldborder.center" => {
                let p = self
                    .position_arg(ctx, "pos")
                    .unwrap_or_else(|| self.source_position());
                let (x, z) = (p[0], p[2]);
                if self.border.center() == [x, z] {
                    return Err(err("commands.worldborder.center.failed", &[]));
                }
                if x.abs() > 2.999_998_4e7 || z.abs() > 2.999_998_4e7 {
                    return Err(err(
                        "commands.worldborder.set.failed.far",
                        &[&"2.9999984E7"],
                    ));
                }
                self.border.set_center([x, z]);
                self.feedback(Text::translate(
                    "commands.worldborder.center.success",
                    &[Text::literal(float2(x)), Text::literal(float2(z))],
                ));
                Ok(0)
            }
            "worldborder.damage.amount" | "worldborder.damage.buffer" => {
                let amount = id.ends_with("amount");
                let value = match ctx.get(if amount { "damagePerBlock" } else { "distance" }) {
                    Some(Arg::Float(v)) => f64::from(*v),
                    _ => 0.0,
                };
                let slot = if amount {
                    &mut self.border.damage_per_block
                } else {
                    &mut self.border.safe_zone
                };
                if *slot == value {
                    return Err(err(
                        if amount {
                            "commands.worldborder.damage.amount.failed"
                        } else {
                            "commands.worldborder.damage.buffer.failed"
                        },
                        &[],
                    ));
                }
                *slot = value;
                self.feedback(Text::translate(
                    if amount {
                        "commands.worldborder.damage.amount.success"
                    } else {
                        "commands.worldborder.damage.buffer.success"
                    },
                    &[Text::literal(float2(value))],
                ));
                Ok(value as i32)
            }
            "worldborder.warning.distance" => {
                let d = match ctx.get("distance") {
                    Some(Arg::Int(v)) => *v,
                    _ => 0,
                };
                if self.border.warning_distance == d {
                    return Err(err("commands.worldborder.warning.distance.failed", &[]));
                }
                self.border.warning_distance = d;
                self.feedback(Text::translate(
                    "commands.worldborder.warning.distance.success",
                    &[num(d)],
                ));
                Ok(d)
            }
            _ => {
                let ticks = match ctx.get("time") {
                    Some(Arg::Int(v)) => *v,
                    _ => 0,
                };
                if self.border.warning_time == ticks {
                    return Err(err("commands.worldborder.warning.time.failed", &[]));
                }
                self.border.warning_time = ticks;
                self.feedback(Text::translate(
                    "commands.worldborder.warning.time.success",
                    &[Text::literal(ticks_to_seconds(i64::from(ticks)))],
                ));
                Ok(ticks)
            }
        }
    }

    // ---- /loot ---------------------------------------------------------------------------------

    fn tool_stack(&mut self, ctx: &Context) -> R<Option<ItemStack>> {
        if let Some(Arg::Item(p)) = ctx.get("tool") {
            let (damage, extra) = match p
                .components
                .as_deref()
                .map(horizon_client::item_data::command_components)
            {
                Some(Ok(tag)) => horizon_client::item_data::decode_components(Some(&tag)),
                Some(Err(e)) => return Err(CommandError::new(Text::literal(e))),
                None => (0, None),
            };
            return Ok(Some(ItemStack {
                item: p.id.as_str().into(),
                count: 1,
                damage,
                extra,
            }));
        }
        let entity = self
            .command_source
            .and_then(|s| s.entity)
            .unwrap_or(Target::Player);
        let slot = if ctx.has("offhand") {
            "weapon.offhand"
        } else {
            "weapon.mainhand"
        };
        Ok(self
            .slots_named(&Owner::Entity(entity), slot)
            .first()
            .and_then(|s| self.slot_get(s)))
    }
    pub(super) fn cmd_loot(&mut self, id: &str, ctx: &Context) -> R<i32> {
        // The drops.
        let (drops, table): (Vec<ItemStack>, Option<String>) = match id {
            "loot.loot" | "loot.fish" => {
                let Some(Arg::Resource(table)) = ctx.get("loot_table").cloned() else {
                    return Ok(0);
                };
                // An inline table (ResourceOrIdArgument): no random sequence.
                if table.starts_with('{') {
                    let (tag, _) = horizon_world::nbt::parse_snbt(&table)
                        .map_err(|e| err("argument.nbt.invalid", &[&e]))?;
                    let json = horizon_client::scoreboard::tag_to_json(&tag);
                    let drops = self
                        .res
                        .items()
                        .inline_loot_table(&json, &mut self.world.random);
                    (drops, None)
                } else {
                    if !self.items().has_loot_table(&table) {
                        return Err(err("argument.resource_or_id.failed_to_parse", &[&table]));
                    }
                    let held = if id == "loot.fish" {
                        self.tool_stack(ctx)?
                    } else {
                        None
                    };
                    let drops = match world_items!(self).loot_sequence(&table) {
                        Some(seq) => {
                            let random = self.rand_sequences.get(&seq, self.level.seed);
                            self.res
                                .items()
                                .loot_table(&table, false, held.as_ref(), random)
                        }
                        None => world_items!(self).loot_table(
                            &table,
                            false,
                            held.as_ref(),
                            &mut self.world.random,
                        ),
                    };
                    (drops, None)
                }
            }
            "loot.kill" => {
                let targets = self.entities(ctx, "target", false)?;
                let Some(t) = targets.first().copied() else {
                    return Ok(0);
                };
                let ty = self.target_type(t);
                let table = format!("minecraft:entities/{}", ty.trim_start_matches("minecraft:"));
                if t == Target::Player || !world_items!(self).has_loot_table(&table) {
                    return Err(err(
                        "commands.drop.no_loot_table.entity",
                        &[&self.target_name(t).plain()],
                    ));
                }
                let by_player = matches!(
                    self.command_source.map(|s| s.entity),
                    None | Some(Some(Target::Player))
                );
                let held = self.slot_get(&super::items::Slot::Inv(self.selected));
                let drops = match world_items!(self).loot_sequence(&table) {
                    Some(seq) => {
                        let random = self.rand_sequences.get(&seq, self.level.seed);
                        self.res
                            .items()
                            .entity_drops(&ty, by_player, held.as_ref(), random)
                    }
                    None => world_items!(self).entity_drops(
                        &ty,
                        by_player,
                        held.as_ref(),
                        &mut self.world.random,
                    ),
                };
                (drops, Some(table))
            }
            _ => {
                // loot mine
                let p = self.block_pos_arg(ctx, "pos")?;
                let state = self.world.block(p[0], p[1], p[2]);
                let name = self.data_block_name(p);
                let table = format!("minecraft:blocks/{}", name.trim_start_matches("minecraft:"));
                if !world_items!(self).has_loot_table(&table) {
                    return Err(err("commands.drop.no_loot_table.block", &[&name]));
                }
                let tool = self.tool_stack(ctx)?;
                let drops = match world_items!(self).loot_sequence(&table) {
                    Some(seq) => {
                        let random = self.rand_sequences.get(&seq, self.level.seed);
                        world_items!(self).block_drops(
                            self.pack.assets.registry,
                            state,
                            tool.as_ref(),
                            random,
                        )
                    }
                    None => world_items!(self).block_drops(
                        self.pack.assets.registry,
                        state,
                        tool.as_ref(),
                        &mut self.world.random,
                    ),
                };
                (drops, Some(table))
            }
        };
        let drops: Vec<ItemStack> = drops.into_iter().filter(|d| d.count > 0).collect();
        // Where they go.
        let mut used: Vec<ItemStack> = Vec::new();
        let literals = &ctx.literals;
        if literals.iter().any(|l| l == "give") {
            for d in &drops {
                for &t in &self.entities(ctx, "players", true)? {
                    if t != Target::Player {
                        continue;
                    }
                    if let Some(rest) = self.give(d.clone()) {
                        let p = self.source_position();
                        self.spawn_item(rest, [p[0], p[1] + 1.32, p[2]], [0.0; 3], 0);
                    }
                    used.push(d.clone());
                }
            }
        } else if literals.iter().any(|l| l == "spawn") {
            let at = self
                .position_arg(ctx, "targetPos")
                .unwrap_or_else(|| self.source_position());
            for d in &drops {
                let r = &mut self.world.random;
                let v = [
                    r.next_double() * 0.2 - 0.1,
                    0.2,
                    r.next_double() * 0.2 - 0.1,
                ];
                self.spawn_item(d.clone(), at, v, 10);
                used.push(d.clone());
            }
        } else if literals.iter().any(|l| l == "insert") {
            let p = self.block_pos_arg(ctx, "targetPos")?;
            let owner = Owner::Block(p);
            let slots = self.slots_named(&owner, "container.*");
            if slots.is_empty() {
                return Err(err(
                    "commands.item.target.not_a_container",
                    &[&p[0], &p[1], &p[2]],
                ));
            }
            for d in &drops {
                let mut left = d.clone();
                let mut placed = false;
                for s in &slots {
                    match self.slot_get(s) {
                        None => {
                            self.slot_set(s, Some(left.clone()));
                            placed = true;
                            left.count = 0;
                            break;
                        }
                        Some(mut cur)
                            if cur.item == left.item
                                && cur.damage == left.damage
                                && cur.extra == left.extra =>
                        {
                            let max = self.items().get(&cur.item).map_or(64, |i| i.max_stack);
                            let take = left.count.min(max.saturating_sub(cur.count));
                            if take > 0 {
                                cur.count += take;
                                left.count -= take;
                                self.slot_set(s, Some(cur));
                                placed = true;
                            }
                            if left.count == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if placed {
                    used.push(d.clone());
                }
            }
        } else {
            // replace entity|block <slot> [count]
            let block = literals.iter().any(|l| l == "block");
            let owners: Vec<Owner> = if block {
                vec![Owner::Block(self.block_pos_arg(ctx, "targetPos")?)]
            } else {
                self.entities(ctx, "entities", false)?
                    .into_iter()
                    .map(Owner::Entity)
                    .collect()
            };
            let Some(Arg::Str(slot_name)) = ctx.get("slot").cloned() else {
                return Ok(0);
            };
            let count = match ctx.get("count") {
                Some(Arg::Int(c)) => *c as usize,
                _ => drops.len(),
            };
            for o in owners {
                // `prefix.N` continues as `prefix.N+1`, ...
                let (prefix, start) = match slot_name.rsplit_once('.') {
                    Some((p, n)) if n.parse::<usize>().is_ok() => {
                        (Some(p.to_owned()), n.parse::<usize>().unwrap_or(0))
                    }
                    _ => (None, 0),
                };
                let first = self.slots_named(&o, &slot_name);
                if first.is_empty() {
                    if let Owner::Block(p) = &o {
                        return Err(err(
                            "commands.item.target.not_a_container",
                            &[&p[0], &p[1], &p[2]],
                        ));
                    }
                    return Err(err("commands.item.target.no_such_slot", &[&slot_name]));
                }
                for i in 0..count {
                    let slot = match &prefix {
                        Some(p) => self
                            .slots_named(&o, &format!("{p}.{}", start + i))
                            .first()
                            .cloned(),
                        None => first.first().cloned().filter(|_| i == 0),
                    };
                    let Some(slot) = slot else { continue };
                    let item = drops.get(i).cloned();
                    if self.slot_set(&slot, item.clone()) {
                        used.push(item.unwrap_or(ItemStack {
                            item: "minecraft:air".into(),
                            count: 0,
                            damage: 0,
                            extra: None,
                        }));
                    }
                }
            }
        }
        let mut tracker = Tracker::default();
        for u in &used {
            let name = self.item_display_name(&u.item);
            tracker.track(&holder(name), u.count as i32);
            let _ = u;
        }
        let counts: Vec<u32> = used.iter().map(|u| u.count).collect();
        let (message, total) = tracker.message(
            false,
            None,
            |h, _| {
                let n = counts.iter().copied().find(|c| *c > 0).unwrap_or(1);
                match &table {
                    Some(t) => Text::translate(
                        "commands.drop.success.single_with_table",
                        &[num(n), h.text.clone(), Text::literal(t.clone())],
                    ),
                    None => {
                        Text::translate("commands.drop.success.single", &[num(n), h.text.clone()])
                    }
                }
            },
            |c, _| match &table {
                Some(t) => Text::translate(
                    "commands.drop.success.multiple_with_table",
                    &[num(c), Text::literal(t.clone())],
                ),
                None => Text::translate("commands.drop.success.multiple", &[num(c)]),
            },
        )?;
        self.feedback(message);
        Ok(total)
    }
}
