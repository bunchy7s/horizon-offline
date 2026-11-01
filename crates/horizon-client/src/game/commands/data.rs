//! `/data` (DataCommands), `/tag` (TagCommand) and `/summon` with NBT: entity,
//! block entity and storage data accessors over the game state.
use super::{Arg, Context, Game, R, Target, err};
use horizon_client::{
    data_cmd::{self, Op, path_error},
    entity_nbt,
    nbt_path::NbtPath,
    save::{self, random_uuid},
    scoreboard_cmd::{Holder, Tracker, format_list},
    snbt,
    text::Text,
};
use horizon_world::nbt::{Compound, JavaString, Tag, parse_snbt};

/// A `DataAccessor`.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::game) enum DataRef {
    Entity(Target),
    Block([i32; 3]),
    Storage(String),
}

fn empty() -> Tag {
    Tag::Compound(Compound::new())
}
fn set_key(tag: &mut Tag, key: &str, value: Tag) {
    if let Tag::Compound(c) = tag {
        c.insert(JavaString::from(key), value);
    }
}
/// `EntityType.loadEntityRecursive` snaps every entity of the stack,
/// passengers included, to the position (`/summon`, `native_summon`).
pub(in crate::game) fn place_summoned(tag: &mut Tag, pos: [f64; 3]) {
    let p = pos.map(|v| Tag::Double(v.to_bits()));
    set_key(tag, "Pos", Tag::List(p.to_vec()));
    // Entity.load: an absent Fire reads as 0 (until the entity moves).
    if tag.get("Fire").is_none() {
        set_key(tag, "Fire", Tag::Short(0));
    }
    if tag.get("Rotation").is_none() {
        set_key(
            tag,
            "Rotation",
            Tag::List(vec![
                Tag::Float(0.0f32.to_bits()),
                Tag::Float(0.0f32.to_bits()),
            ]),
        );
    }
    if let Tag::Compound(c) = tag
        && let Some(Tag::List(riders)) = c.get_mut(&"Passengers".into())
    {
        for r in riders {
            if matches!(r, Tag::Compound(_)) {
                place_summoned(r, pos);
            }
        }
    }
}

impl Game<'_> {
    // ---- entity data ---------------------------------------------------

    /// `NbtPredicate.getEntityTagToCompare`: the entity as saved. An entity
    /// in another level is read in the level that holds it.
    pub(in crate::game) fn entity_nbt(&mut self, t: Target) -> Option<Tag> {
        self.in_target_level(t, |g| g.entity_nbt_in_level(t))
            .flatten()
    }
    fn entity_nbt_in_level(&mut self, t: Target) -> Option<Tag> {
        let player_uuid = self.level.player_uuid;
        Some(match t {
            Target::Guest(_) => {
                return self
                    .for_player(t, |g| g.entity_nbt(Target::Player))
                    .flatten();
            }
            Target::Player => {
                self.player.as_ref()?;
                self.sync_level();
                let mut tag = self.level.player_tag();
                if let Some(u) = player_uuid {
                    set_key(&mut tag, "UUID", Tag::IntArray(u.to_vec()));
                }
                // NbtPredicate.getEntityTagToCompare: a player's held stack.
                let selected = self.level.selected_slot;
                let held = match tag.get("Inventory") {
                    Some(Tag::List(items)) => items
                        .iter()
                        .find(|i| matches!(i.get("Slot"), Some(Tag::Byte(b)) if i32::from(*b) == selected))
                        .cloned(),
                    _ => None,
                };
                if let Some(mut stack) = held {
                    if let Tag::Compound(c) = &mut stack {
                        c.remove(&"Slot".into());
                    }
                    set_key(&mut tag, "SelectedItem", stack);
                }
                tag
            }
            Target::Mob(id) => {
                // `saveWithoutId` writes the entity as saved, riders under
                // `Passengers`: `/data get` reads the same tree persistence does (#1519).
                let i = self.mobs.iter().position(|m| m.id == id)?;
                self.mob_tree_nbt(i)
            }
            Target::Item(id) => {
                let uuid = self.uuid_for(id);
                let e = self.dropped.iter().find(|e| e.id == id)?;
                entity_nbt::item_entity_to_nbt(e, uuid, self.persist.raw.get(&id))
            }
            Target::Stand(id) => {
                let uuid = self.uuid_for(id);
                let a = self.armor_stands.iter().find(|a| a.id == id)?;
                entity_nbt::armor_stand_to_nbt(a, uuid, self.persist.raw.get(&id))
            }
            Target::Frame(id) => {
                let uuid = self.uuid_for(id);
                let f = self.item_frames.iter().find(|f| f.id == id)?;
                entity_nbt::item_frame_to_nbt(f, uuid, self.persist.raw.get(&id))
            }
            Target::Painting(id) => {
                let uuid = self.uuid_for(id);
                let p = self.paintings.iter().find(|p| p.id == id)?;
                entity_nbt::painting_to_nbt(p, uuid, self.persist.raw.get(&id))
            }
            Target::Boat(id) => {
                let uuid = self.uuid_for(id);
                let b = self.boats.iter().find(|b| b.id == id)?;
                entity_nbt::boat_to_nbt(
                    &b.entity_type(),
                    b.position,
                    b.velocity,
                    b.yaw,
                    uuid,
                    self.persist.raw.get(&id),
                    b.chest.then_some(&b.items[..]),
                )
            }
            Target::Cart(id) => {
                let uuid = self.uuid_for(id);
                let c = self.carts.iter().find(|c| c.id == id)?;
                c.to_nbt(uuid, self.persist.raw.get(&id))
            }
            Target::Other(id) => {
                let uuid = self.uuid_for(id);
                let o = self.others.iter().find(|o| o.id == id)?.clone();
                self.other_to_nbt(&o, uuid)
            }
            Target::Orb(id) => {
                entity_nbt::orb_to_nbt(self.orbs.iter().find(|o| o.id == id)?, random_uuid())
            }
            Target::Projectile(id) => {
                let p = self.creatures.projectiles.iter().find(|p| p.id == id)?;
                let owner = if p.owner == Some(horizon_client::mobs::Who::Player) {
                    player_uuid
                } else {
                    None
                };
                let in_block = p
                    .arrow
                    .as_ref()
                    .and_then(|a| a.last_state)
                    .map(|s| self.state_to_nbt(s));
                entity_nbt::arrow_to_nbt(p, random_uuid(), owner, in_block, None)
                    .or_else(|| entity_nbt::fireball_to_nbt(p, random_uuid(), owner, None))?
            }
            Target::Crystal(id) => {
                let c = self.crystals.iter().find(|c| c.id == id)?;
                let (uuid, raw) = self.persist_anon("minecraft:end_crystal", c.position);
                c.to_nbt(uuid.unwrap_or_else(random_uuid), raw)
            }
            Target::Dragon(id) => {
                let d = self.dragon.as_ref().filter(|d| d.id == id)?;
                let (uuid, raw) = self.persist_anon("minecraft:ender_dragon", d.position);
                d.to_nbt(uuid.unwrap_or_else(random_uuid), raw)
            }
        })
    }
    /// `Entity.load` of new data (`EntityDataAccessor.setData`): the entity is
    /// rebuilt from its compound, keeping its id and UUID. Also what
    /// `TypedEntityData.loadInto` is: a spawn egg's data on the mob it spawned.
    pub(in crate::game) fn apply_entity_nbt(&mut self, t: Target, mut tag: Tag) -> R<()> {
        let stale = || err("commands.data.merge.failed", &[]);
        match t {
            Target::Player => return Err(err("commands.data.entity.invalid", &[])),
            Target::Mob(id) => {
                let uuid = self.uuid_for(id);
                let kind = self
                    .mobs
                    .iter()
                    .find(|m| m.id == id)
                    .map(|m| m.kind.id.clone());
                set_key(&mut tag, "UUID", Tag::IntArray(uuid.to_vec()));
                if let Some(k) = kind {
                    set_key(&mut tag, "id", Tag::String(k.as_str().into()));
                }
                let new = entity_nbt::mob_from_nbt(
                    &tag,
                    self.res.mob_types(),
                    id,
                    Some(self.pack.assets.registry),
                )
                .ok_or_else(stale)?;
                if let Some(slot) = self.mobs.iter_mut().find(|m| m.id == id) {
                    // Runtime-only state (AI, animation) carries over.
                    let old = std::mem::replace(slot, new);
                    slot.ai = old.ai;
                    slot.age = old.age;
                    slot.previous = slot.position;
                }
            }
            Target::Item(id) => {
                let new = entity_nbt::item_entity_from_nbt(&tag, id, 0.0).ok_or_else(stale)?;
                if let Some(slot) = self.dropped.iter_mut().find(|e| e.id == id) {
                    let bob = slot.bob_offset;
                    *slot = new;
                    slot.bob_offset = bob;
                }
            }
            Target::Stand(id) => {
                let new = entity_nbt::armor_stand_from_nbt(&tag, id).ok_or_else(stale)?;
                if let Some(slot) = self.armor_stands.iter_mut().find(|a| a.id == id) {
                    *slot = new;
                }
            }
            Target::Frame(id) => {
                let new = entity_nbt::item_frame_from_nbt(&tag, id).ok_or_else(stale)?;
                if let Some(slot) = self.item_frames.iter_mut().find(|f| f.id == id) {
                    *slot = new;
                }
            }
            _ => return Err(stale()),
        }
        if let Target::Mob(id) | Target::Item(id) | Target::Stand(id) | Target::Frame(id) = t {
            self.persist.raw.insert(id, tag);
        }
        Ok(())
    }
    pub(in crate::game) fn data_block_name(&self, p: [i32; 3]) -> String {
        let state = self.world.block(p[0], p[1], p[2]);
        self.pack
            .assets
            .registry
            .state(state)
            .map_or_else(|| "minecraft:air".into(), |s| s.name.to_string())
    }
    fn raw_mut(&mut self, t: Target) -> Option<&mut Tag> {
        match t {
            Target::Player => Some(self.level.player_extra.get_or_insert_with(empty)),
            Target::Mob(id)
            | Target::Item(id)
            | Target::Stand(id)
            | Target::Frame(id)
            | Target::Painting(id)
            | Target::Boat(id)
            | Target::Cart(id)
            | Target::Other(id) => Some(self.persist.raw.entry(id).or_insert_with(empty)),
            _ => None,
        }
    }
    fn set_entity_tags(&mut self, t: Target, tags: &[String]) {
        if let Some(raw) = self.raw_mut(t) {
            let list = tags
                .iter()
                .map(|s| Tag::String(s.as_str().into()))
                .collect();
            set_key(raw, "Tags", Tag::List(list));
        }
    }

    // ---- accessors -----------------------------------------------------

    /// The `role`th provider of the command (`entity`, `block` or `storage`).
    pub(in crate::game) fn data_ref(&mut self, ctx: &Context, role: usize) -> R<DataRef> {
        let prefix = if role == 0 { "target" } else { "source" };
        self.data_ref_named(ctx, role, prefix)
    }
    pub(in crate::game) fn data_ref_named(
        &mut self,
        ctx: &Context,
        role: usize,
        prefix: &str,
    ) -> R<DataRef> {
        let kind = ctx
            .literals
            .iter()
            .filter(|l| matches!(l.as_str(), "entity" | "block" | "storage"))
            .nth(role)
            .cloned()
            .unwrap_or_default();
        match kind.as_str() {
            "entity" => {
                let Some(Arg::Entity(sel)) = ctx.get(prefix).cloned() else {
                    return Err(err("argument.entity.notfound.entity", &[]));
                };
                let found = self.select(&sel);
                found
                    .first()
                    .map(|t| DataRef::Entity(*t))
                    .ok_or_else(|| err("argument.entity.notfound.entity", &[]))
            }
            "block" => {
                let p = self.block_pos_arg(ctx, &format!("{prefix}Pos"))?;
                if !self.world.block_entities.contains_key(&(p[0], p[1], p[2])) {
                    return Err(err("commands.data.block.invalid", &[]));
                }
                Ok(DataRef::Block(p))
            }
            _ => match ctx.get(prefix) {
                Some(Arg::Resource(id)) => Ok(DataRef::Storage(id.clone())),
                _ => Err(err("command.failed", &[])),
            },
        }
    }
    pub(in crate::game) fn data_of(&mut self, r: &DataRef) -> R<Tag> {
        match r {
            // EntityDataAccessor.getData: Entity.saveWithoutId.
            DataRef::Entity(t) => {
                let mut tag = self
                    .entity_nbt(*t)
                    .ok_or_else(|| err("argument.entity.notfound.entity", &[]))?;
                if let Tag::Compound(c) = &mut tag {
                    c.remove(&"id".into());
                }
                Ok(tag)
            }
            DataRef::Block(p) => {
                let key = (p[0], p[1], p[2]);
                let name = self.data_block_name(*p);
                let e = self
                    .world
                    .block_entities
                    .get(&key)
                    .ok_or_else(|| err("commands.data.block.invalid", &[]))?;
                // BlockEntity.saveWithFullMetadata: no chunk-only keepPacked,
                // and the lock the model does not hold.
                let mut tag = save::block_entity_data(key, e, &name, self.world.block_lock(key));
                self.restore_loot_table(key, &mut tag);
                Ok(tag)
            }
            DataRef::Storage(id) => Ok(self.score.storage.get(id)),
        }
    }
    pub(in crate::game) fn set_data_of(&mut self, r: &DataRef, tag: Tag) -> R<()> {
        match r {
            DataRef::Entity(t) => self.apply_entity_nbt(*t, tag),
            DataRef::Block(p) => {
                let key = (p[0], p[1], p[2]);
                if let Some((_, mut new)) = save::decode_block_entity(&tag) {
                    self.take_loot_table(key, &tag, &mut new);
                    self.world.block_entities.insert(key, new);
                    self.world.set_block_lock(key, &tag);
                    self.update_spawner_display(key);
                    // BlockDataAccessor.setData: setChanged and sendBlockUpdated(3),
                    // even when the block state itself stays the same.
                    self.mark_block_entity_updated(key);
                    let state = self.world.block(p[0], p[1], p[2]);
                    // Redraw what shows the block entity (signs, banners).
                    let _ = self.world.set(p[0], p[1], p[2], state);
                }
                Ok(())
            }
            DataRef::Storage(id) => {
                self.score.storage.set(id, tag);
                Ok(())
            }
        }
    }
    fn data_name(&self, r: &DataRef) -> Text {
        match r {
            DataRef::Entity(t) => self.target_name(*t),
            DataRef::Block(p) => Text::literal(format!("{}, {}, {}", p[0], p[1], p[2])),
            DataRef::Storage(id) => Text::literal(id.clone()),
        }
    }
    fn modified_text(&self, r: &DataRef) -> Text {
        match r {
            DataRef::Entity(_) => {
                Text::translate("commands.data.entity.modified", &[self.data_name(r)])
            }
            DataRef::Block(p) => Text::translate(
                "commands.data.block.modified",
                &[
                    Text::literal(p[0].to_string()),
                    Text::literal(p[1].to_string()),
                    Text::literal(p[2].to_string()),
                ],
            ),
            DataRef::Storage(id) => Text::translate(
                "commands.data.storage.modified",
                &[Text::literal(id.clone())],
            ),
        }
    }
    fn query_text(&self, r: &DataRef, data: &Tag) -> Text {
        let pretty = snbt::pretty(data);
        match r {
            DataRef::Entity(_) => {
                Text::translate("commands.data.entity.query", &[self.data_name(r), pretty])
            }
            DataRef::Block(p) => Text::translate(
                "commands.data.block.query",
                &[
                    Text::literal(p[0].to_string()),
                    Text::literal(p[1].to_string()),
                    Text::literal(p[2].to_string()),
                    pretty,
                ],
            ),
            DataRef::Storage(id) => Text::translate(
                "commands.data.storage.query",
                &[Text::literal(id.clone()), pretty],
            ),
        }
    }
    fn get_text(&self, r: &DataRef, path: &NbtPath, scale: f64, value: i32) -> Text {
        let p = Text::literal(path.as_str());
        let (s, v) = (
            data_cmd::scale_text(scale),
            Text::literal(value.to_string()),
        );
        match r {
            DataRef::Entity(_) => {
                Text::translate("commands.data.entity.get", &[p, self.data_name(r), s, v])
            }
            DataRef::Block(b) => Text::translate(
                "commands.data.block.get",
                &[
                    p,
                    Text::literal(b[0].to_string()),
                    Text::literal(b[1].to_string()),
                    Text::literal(b[2].to_string()),
                    s,
                    v,
                ],
            ),
            DataRef::Storage(id) => Text::translate(
                "commands.data.storage.get",
                &[p, Text::literal(id.clone()), s, v],
            ),
        }
    }

    // ---- /data -----------------------------------------------------------

    fn path_arg(&self, ctx: &Context, name: &str) -> R<NbtPath> {
        let Some(Arg::Str(text)) = ctx.get(name) else {
            return Err(err("command.failed", &[]));
        };
        NbtPath::parse(text).map_err(path_error)
    }
    pub(super) fn cmd_data(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let target = self.data_ref(ctx, 0)?;
        // An entity in another level is read and written in the level that
        // holds it.
        if let DataRef::Entity(t) = &target {
            let t = *t;
            let target = target.clone();
            return match self.in_target_level(t, |g| g.cmd_data_in(id, ctx, target)) {
                Some(r) => r,
                None => Err(err("argument.entity.notfound.entity", &[])),
            };
        }
        self.cmd_data_in(id, ctx, target)
    }
    fn cmd_data_in(&mut self, id: &str, ctx: &Context, target: DataRef) -> R<i32> {
        match id {
            "data.merge" => {
                let Some(Arg::Str(nbt)) = ctx.get("nbt") else {
                    return Ok(0);
                };
                let (nbt, _) = parse_snbt(nbt).map_err(|e| err("argument.nbt.invalid", &[&e]))?;
                let old = self.data_of(&target)?;
                if horizon_client::nbt_path::too_deep(&nbt, 0) {
                    return Err(err("arguments.nbtpath.too_deep", &[]));
                }
                let result = data_cmd::merged(&old, &nbt);
                if old == result {
                    return Err(err("commands.data.merge.failed", &[]));
                }
                self.set_data_of(&target, result)?;
                let message = self.modified_text(&target);
                self.feedback(message);
                Ok(1)
            }
            "data.get" => {
                let data = self.data_of(&target)?;
                if ctx.get("path").is_none() {
                    let message = self.query_text(&target, &data);
                    self.system_feedback(message);
                    return Ok(1);
                }
                let path = self.path_arg(ctx, "path")?;
                let tag = data_cmd::single_tag(&path, &data)?;
                if let Some(Arg::Double(scale)) = ctx.get("scale") {
                    let value = data_cmd::scaled_value(&tag, &path, *scale)?;
                    let message = self.get_text(&target, &path, *scale, value);
                    self.system_feedback(message);
                    return Ok(value);
                }
                let value = data_cmd::get_value(&tag, &path)?;
                let message = self.query_text(&target, &tag);
                self.system_feedback(message);
                Ok(value)
            }
            "data.remove" => {
                let path = self.path_arg(ctx, "path")?;
                let mut data = self.data_of(&target)?;
                let count = path.remove(&mut data);
                if count == 0 {
                    return Err(err("commands.data.merge.failed", &[]));
                }
                self.set_data_of(&target, data)?;
                let message = self.modified_text(&target);
                self.feedback(message);
                Ok(count as i32)
            }
            "data.modify" => self.cmd_data_modify(ctx, target),
            _ => Err(err("command.failed", &[])),
        }
    }
    /// A message sent without the feedback gamerule check (`sendSuccess(.., false)`).
    fn system_feedback(&mut self, message: Text) {
        self.feedback(message);
    }
    fn cmd_data_modify(&mut self, ctx: &Context, target: DataRef) -> R<i32> {
        let target_path = self.path_arg(ctx, "targetPath")?;
        let after_modify: Vec<&String> = ctx
            .literals
            .iter()
            .skip_while(|l| l.as_str() != "modify")
            .collect();
        let op = after_modify
            .iter()
            .find_map(|l| match l.as_str() {
                "prepend" => Some(Op::Prepend),
                "append" => Some(Op::Append),
                "set" => Some(Op::Set),
                "merge" => Some(Op::Merge),
                "insert" => Some(Op::Insert(match ctx.get("index") {
                    Some(Arg::Int(i)) => *i,
                    _ => 0,
                })),
                _ => None,
            })
            .unwrap_or(Op::Set);
        let sources: Vec<Tag> = if ctx.has("value") {
            let Some(Arg::Str(text)) = ctx.get("value") else {
                return Ok(0);
            };
            vec![
                parse_snbt(text)
                    .map_err(|e| err("argument.nbt.invalid", &[&e]))?
                    .0,
            ]
        } else {
            let source = self.data_ref(ctx, 1)?;
            let data = self.data_of(&source)?;
            let tags = if ctx.get("sourcePath").is_some() {
                self.path_arg(ctx, "sourcePath")?
                    .get(&data)
                    .map_err(path_error)?
            } else {
                vec![data]
            };
            if ctx.has("string") {
                let start = match ctx.get("start") {
                    Some(Arg::Int(i)) => Some(*i),
                    _ => None,
                };
                let end = match ctx.get("end") {
                    Some(Arg::Int(i)) => Some(*i),
                    _ => None,
                };
                let mut out = Vec::new();
                for t in &tags {
                    let text = data_cmd::as_text(t)?;
                    let text = match start {
                        Some(s) => data_cmd::substring(&text, s, end)?,
                        None => text,
                    };
                    out.push(Tag::String(text.as_str().into()));
                }
                out
            } else {
                tags
            }
        };
        let mut data = self.data_of(&target)?;
        let changed = data_cmd::modify(op, &mut data, &target_path, &sources)?;
        if changed == 0 {
            return Err(err("commands.data.merge.failed", &[]));
        }
        self.set_data_of(&target, data)?;
        let message = self.modified_text(&target);
        self.feedback(message);
        Ok(changed as i32)
    }

    // ---- /tag ------------------------------------------------------------

    pub(super) fn cmd_tag(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let targets = self.entities(ctx, "targets", false)?;
        let name = match ctx.get("name") {
            Some(Arg::Str(n)) => n.clone(),
            _ => String::new(),
        };
        let mut tracker = Tracker::default();
        let holder = |g: &Self, t: Target| Holder {
            name: String::new(),
            text: g.target_name(t),
            display: None,
        };
        let mut all: Vec<String> = Vec::new();
        for &t in &targets {
            let mut tags = self.tags_of(t);
            let h = holder(self, t);
            match id {
                "tag.add" => {
                    let added = tags.len() < 1024 && !tags.contains(&name);
                    if added {
                        tags.push(name.clone());
                        self.set_entity_tags(t, &tags);
                    }
                    tracker.track(&h, i32::from(added));
                }
                "tag.remove" => {
                    let before = tags.len();
                    tags.retain(|x| x != &name);
                    let removed = tags.len() != before;
                    if removed {
                        self.set_entity_tags(t, &tags);
                    }
                    tracker.track(&h, i32::from(removed));
                }
                _ => {
                    tracker.track(&h, tags.len() as i32);
                    for tag in tags {
                        if !all.contains(&tag) {
                            all.push(tag);
                        }
                    }
                }
            }
        }
        let num = |n: usize| Text::literal(n.to_string());
        let n = name.clone();
        let (message, total) = match id {
            "tag.add" => tracker.message(
                false,
                Some(err("commands.tag.add.failed", &[])),
                |h, _| {
                    Text::translate(
                        "commands.tag.add.success.single",
                        &[Text::literal(n.clone()), h.text.clone()],
                    )
                },
                |c, _| {
                    Text::translate(
                        "commands.tag.add.success.multiple",
                        &[Text::literal(n.clone()), num(c)],
                    )
                },
            ),
            "tag.remove" => tracker.message(
                false,
                Some(err("commands.tag.remove.failed", &[])),
                |h, _| {
                    Text::translate(
                        "commands.tag.remove.success.single",
                        &[Text::literal(n.clone()), h.text.clone()],
                    )
                },
                |c, _| {
                    Text::translate(
                        "commands.tag.remove.success.multiple",
                        &[Text::literal(n.clone()), num(c)],
                    )
                },
            ),
            _ => {
                let count = all.len();
                let message = if all.is_empty() {
                    tracker
                        .message(
                            false,
                            None,
                            |h, _| {
                                Text::translate(
                                    "commands.tag.list.single.empty",
                                    std::slice::from_ref(&h.text),
                                )
                            },
                            |c, _| Text::translate("commands.tag.list.multiple.empty", &[num(c)]),
                        )?
                        .0
                } else {
                    let list = {
                        let mut sorted = all.clone();
                        sorted.sort();
                        format_list(
                            sorted
                                .into_iter()
                                .map(|s| Text::literal(s).with_color(horizon_client::text::GREEN))
                                .collect(),
                        )
                    };
                    tracker
                        .message(
                            true,
                            None,
                            |h, _| {
                                Text::translate(
                                    "commands.tag.list.single.success",
                                    &[h.text.clone(), num(count), list.clone()],
                                )
                            },
                            |c, _| {
                                Text::translate(
                                    "commands.tag.list.multiple.success",
                                    &[num(c), num(count), list.clone()],
                                )
                            },
                        )?
                        .0
                };
                self.feedback(message);
                return Ok(count as i32);
            }
        }?;
        self.feedback(message);
        Ok(total)
    }

    // ---- /summon ---------------------------------------------------------

    /// Spawn an entity from its compound (`EntityType.loadEntityRecursive`);
    /// returns what was created.
    pub(in crate::game) fn spawn_entity_from_nbt(&mut self, tag: Tag) -> Option<Target> {
        let key = {
            let p = entity_nbt::position(&tag)?;
            ((p.x.floor() as i32) >> 4, (p.z.floor() as i32) >> 4)
        };
        // Only this stack spawns now: the entities of chunks that finished
        // loading wait for the level's entity tick, as in vanilla
        // (PersistentEntitySectionManager.processPendingLoads), and must not
        // take the ids counted as this stack's.
        let mut waiting = std::mem::take(&mut self.world.loaded_entities);
        let before = self.next_entity;
        let mut tag = tag;
        crate::game::persist::mark_fresh(&mut tag);
        self.world.loaded_entities.push((key, vec![tag]));
        self.spawn_loaded_entities();
        waiting.append(&mut self.world.loaded_entities);
        self.world.loaded_entities = waiting;
        if self.next_entity == before {
            return None;
        }
        // The root of the stack is the first entity spawned.
        let id = before + 1;
        if self.mobs.iter().any(|m| m.id == id) {
            Some(Target::Mob(id))
        } else if self.dropped.iter().any(|e| e.id == id) {
            Some(Target::Item(id))
        } else if self.armor_stands.iter().any(|a| a.id == id) {
            Some(Target::Stand(id))
        } else if self.item_frames.iter().any(|f| f.id == id) {
            Some(Target::Frame(id))
        } else if self.paintings.iter().any(|p| p.id == id) {
            Some(Target::Painting(id))
        } else if self.boats.iter().any(|b| b.id == id) {
            Some(Target::Boat(id))
        } else if self.carts.iter().any(|c| c.id == id) {
            Some(Target::Cart(id))
        } else if self.others.iter().any(|o| o.id == id) {
            Some(Target::Other(id))
        } else {
            None
        }
    }
    /// The `summon <entity> <pos> <nbt>` form.
    pub(super) fn summon_with_nbt(
        &mut self,
        kind: &str,
        pos: [f64; 3],
        nbt: &str,
    ) -> R<Option<Target>> {
        let (mut tag, _) = parse_snbt(nbt).map_err(|e| err("argument.nbt.invalid", &[&e]))?;
        if !matches!(tag, Tag::Compound(_)) {
            return Err(err("argument.nbt.expected.compound", &[]));
        }
        set_key(&mut tag, "id", Tag::String(kind.into()));
        place_summoned(&mut tag, pos);
        let from = self.mobs.len();
        let canceled = self.persist.joins_canceled;
        let target = self.spawn_entity_from_nbt(tag);
        self.native_mobs_joined(from, false);
        // A mod kept the entity out: tryAddFreshEntityWithPassengers is false.
        if target.is_none() && self.persist.joins_canceled != canceled {
            return Err(err("commands.summon.failed.uuid", &[]));
        }
        Ok(target)
    }
}
