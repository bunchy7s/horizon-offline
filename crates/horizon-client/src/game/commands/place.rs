//! `/place template` (PlaceCommand): a saved structure template placed with
//! rotation, mirroring and integrity.
use super::{Arg, Context, Game, R, err};
use horizon_client::{save, text::Text};
use horizon_core::random::{LegacyRandom, RandomSource};
use horizon_world::{
    compression::Compression,
    nbt::{self, Limits, Tag},
    worldgen::structure::{Mirror, Rotation, StateTransforms, StructureTemplate},
};
use std::sync::{Arc, OnceLock};

fn num(n: impl std::fmt::Display) -> Text {
    Text::literal(n.to_string())
}

impl Game<'_> {
    fn state_transforms(&self) -> Option<&'static StateTransforms> {
        static TRANSFORMS: OnceLock<Option<StateTransforms>> = OnceLock::new();
        TRANSFORMS
            .get_or_init(|| {
                let json = std::fs::read_to_string(
                    self.options
                        .resources
                        .join("worldgen/blocks/structure-blocks.json"),
                )
                .ok()?;
                StateTransforms::from_json(&json, self.pack.assets.registry.len()).ok()
            })
            .as_ref()
    }
    /// A template by id: a structure block's save in the world, then the
    /// vanilla templates.
    fn load_template(&self, id: &str) -> Option<Arc<StructureTemplate>> {
        let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
        let candidates = [
            self.world_root
                .join("generated")
                .join(ns)
                .join("structures")
                .join(format!("{path}.nbt")),
            self.options
                .resources
                .join("worldgen/structure")
                .join(ns)
                .join(format!("{path}.nbt")),
        ];
        let transforms = self.state_transforms()?;
        for file in candidates {
            let Ok(bytes) = std::fs::read(&file) else {
                continue;
            };
            let Ok(raw) = Compression::Gzip.decode(&bytes, 1 << 28) else {
                continue;
            };
            let Ok((named, _)) = nbt::read_named(&raw, Limits::templates()) else {
                continue;
            };
            return Some(Arc::new(StructureTemplate::from_nbt(
                &named.tag,
                self.pack.assets.registry,
                transforms,
            )));
        }
        None
    }
    /// `BlockPosArgument.getLoadedBlockPos(pos)`, or the source's block.
    fn place_pos(&self, ctx: &Context, name: &str) -> R<[i32; 3]> {
        if ctx.get(name).is_some() {
            return self.block_pos_arg(ctx, name);
        }
        Ok(self.source_position().map(|v| v.floor() as i32))
    }
    /// `PlaceCommand.checkLoaded`: every chunk of the inclusive range.
    fn place_check_loaded(&self, min: [i32; 2], max: [i32; 2]) -> R<()> {
        let dimension = self.command_source.map_or(self.dim, |s| s.dimension);
        for cx in min[0]..=max[0] {
            for cz in min[1]..=max[1] {
                if dimension != self.dim || !self.world.chunks.contains_key(&(cx, cz)) {
                    return Err(err("argument.pos.unloaded", &[]));
                }
            }
        }
        Ok(())
    }
    /// `PlaceCommand.placeFeature`, `placeJigsaw` and `placeStructure`.
    pub(super) fn cmd_place_generated(&mut self, id: &str, ctx: &Context) -> R<i32> {
        match id {
            "place.feature" => {
                let raw = match ctx.get("feature") {
                    Some(Arg::Resource(s) | Arg::Str(s)) => s.clone(),
                    _ => return Ok(0),
                };
                let pos = self.place_pos(ctx, "pos")?;
                let (cx, cz) = (pos[0] >> 4, pos[2] >> 4);
                self.place_check_loaded([cx - 1, cz - 1], [cx + 1, cz + 1])?;
                // Inline features are not built by the live feature registry.
                if raw.starts_with('{') {
                    return Err(err("command.failed", &[]));
                }
                let name = if raw.contains(':') {
                    raw
                } else {
                    format!("minecraft:{raw}")
                };
                let at = horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
                let placed = self
                    .with_level(|b, level, ticks| b.place_feature(level, ticks, &name, at))
                    .ok_or_else(|| err("command.failed", &[]))?;
                self.process_level_events()
                    .map_err(|_| err("command.failed", &[]))?;
                if !placed {
                    return Err(err("commands.place.feature.failed", &[]));
                }
                self.feedback(Text::translate(
                    "commands.place.feature.success",
                    &[Text::literal(name), num(pos[0]), num(pos[1]), num(pos[2])],
                ));
                Ok(1)
            }
            "place.jigsaw" => {
                let Some(Arg::Resource(pool)) = ctx.get("pool").cloned() else {
                    return Ok(0);
                };
                if !self
                    .ui
                    .registries
                    .ids("minecraft:worldgen/template_pool")
                    .contains(&pool)
                {
                    return Err(err("commands.place.jigsaw.invalid", &[&pool]));
                }
                let target = match ctx.get("target") {
                    Some(Arg::Resource(s) | Arg::Str(s)) => s.clone(),
                    _ => return Ok(0),
                };
                let max_depth = match ctx.get("max_depth") {
                    Some(Arg::Int(n)) => *n,
                    _ => return Ok(0),
                };
                let pos = self.place_pos(ctx, "position")?;
                self.place_check_loaded([pos[0] >> 4, pos[2] >> 4], [pos[0] >> 4, pos[2] >> 4])?;
                let terrain = self
                    .end
                    .locator()
                    .ok_or_else(|| err("command.failed", &[]))?;
                let query = horizon_world::worldgen::generator::JigsawQuery {
                    pool: &pool,
                    target: &target,
                    max_depth,
                    position: pos,
                };
                let placed = self.with_level(|b, level, ticks| {
                    b.place_jigsaw(level, ticks, pos, || terrain.jigsaw_pieces(&query))
                });
                self.process_level_events()
                    .map_err(|_| err("command.failed", &[]))?;
                if !placed {
                    return Err(err("commands.place.jigsaw.failed", &[]));
                }
                self.feedback(Text::translate(
                    "commands.place.jigsaw.success",
                    &[num(pos[0]), num(pos[1]), num(pos[2])],
                ));
                Ok(1)
            }
            _ => {
                let Some(Arg::Resource(structure)) = ctx.get("structure").cloned() else {
                    return Ok(0);
                };
                if !self
                    .ui
                    .registries
                    .ids("minecraft:worldgen/structure")
                    .contains(&structure)
                {
                    return Err(err("commands.place.structure.invalid", &[&structure]));
                }
                self.place_pos(ctx, "pos")?;
                // Structure.generate is not wired to the live level.
                Err(err("command.failed", &[]))
            }
        }
    }
    pub(super) fn cmd_place(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Resource(id)) = ctx.get("template").cloned() else {
            return Ok(0);
        };
        let pos = self.place_pos(ctx, "pos")?;
        let rotation = match ctx.get("rotation") {
            Some(Arg::Str(r)) => Rotation::from_name(r).unwrap_or(Rotation::None),
            _ => Rotation::None,
        };
        let mirror = match ctx.get("mirror") {
            Some(Arg::Str(m)) => match m.as_str() {
                "left_right" => Mirror::LeftRight,
                "front_back" => Mirror::FrontBack,
                _ => Mirror::None,
            },
            _ => Mirror::None,
        };
        let integrity = match ctx.get("integrity") {
            Some(Arg::Float(f)) => *f,
            _ => 1.0,
        };
        let seed = match ctx.get("seed") {
            Some(Arg::Int(s)) => i64::from(*s),
            _ => 0,
        };
        let strict = ctx.has("strict");
        let template = self
            .load_template(&id)
            .filter(|t| !t.palettes().is_empty())
            .ok_or_else(|| err("commands.place.template.invalid", &[&id]))?;
        let transforms = self
            .state_transforms()
            .ok_or_else(|| err("commands.place.template.invalid", &[&id]))?;
        let mut random = LegacyRandom::new(if seed == 0 {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(1, |d| d.as_millis() as i64)
        } else {
            seed
        });
        let palette = template.palette_index(&mut random);
        let blocks = template.palettes()[palette].blocks().to_vec();
        let mut placed = 0;
        for b in &blocks {
            if integrity < 1.0 && random.next_float() > integrity {
                continue;
            }
            let t = StructureTemplate::transform(b.pos, mirror, rotation, [0, 0, 0]);
            let at = [pos[0] + t[0], pos[1] + t[1], pos[2] + t[2]];
            if !self.world.chunks.contains_key(&(at[0] >> 4, at[2] >> 4)) {
                continue;
            }
            let state = transforms.rotate(transforms.mirror(b.state, mirror), rotation);
            if self.place_state(at, state, strict).is_ok() {
                placed += 1;
            }
            if let Some(data) = &b.nbt {
                let mut tag = Tag::Compound(data.clone());
                if let Tag::Compound(c) = &mut tag {
                    c.insert("x".into(), Tag::Int(at[0]));
                    c.insert("y".into(), Tag::Int(at[1]));
                    c.insert("z".into(), Tag::Int(at[2]));
                }
                if let Some((_, entity)) = save::decode_block_entity(&tag) {
                    let key = (at[0], at[1], at[2]);
                    self.world.block_entities.insert(key, entity);
                    self.update_spawner_display(key);
                }
            }
        }
        // Entities saved with the template.
        for e in &template.entities {
            let p = StructureTemplate::transform_vec(e.pos, mirror, rotation, [0, 0, 0]);
            let at = [
                f64::from(pos[0]) + p[0],
                f64::from(pos[1]) + p[1],
                f64::from(pos[2]) + p[2],
            ];
            let mut tag = Tag::Compound(e.nbt.clone());
            if let Tag::Compound(c) = &mut tag {
                c.insert(
                    "Pos".into(),
                    Tag::List(at.iter().map(|v| Tag::Double(v.to_bits())).collect()),
                );
                c.remove(&"UUID".into());
            }
            self.spawn_entity_from_nbt(tag);
        }
        if placed == 0 {
            return Err(err("commands.place.template.failed", &[]));
        }
        self.feedback(Text::translate(
            "commands.place.template.success",
            &[Text::literal(id), num(pos[0]), num(pos[1]), num(pos[2])],
        ));
        Ok(1)
    }
}
