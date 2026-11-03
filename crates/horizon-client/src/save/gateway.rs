//! One-time migration of Horizon's old gateway side file into chunk NBT.
use super::*;

const LEGACY_FILE: &str = "horizon-end-gateways.json";

/// The block actually at an old entry, rather than a stale block entity.
fn is_gateway(root: &Tag, pos: BlockPos) -> Result<bool> {
    let Ok(y) = i8::try_from(pos.y >> 4) else {
        return Ok(false);
    };
    let Some(Tag::List(sections)) = root.get("sections") else {
        return Err("gateway migration: missing chunk sections".into());
    };
    let Some(section) = sections.iter().find(|s| s.get("Y") == Some(&Tag::Byte(y))) else {
        return Ok(false);
    };
    let Some(states) = section.get("block_states") else {
        return Ok(false);
    };
    let palette = horizon_world::chunk::PalettedContainer::from_nbt(states, 4096, 4)?;
    Ok(BlockState::from_nbt(palette.get(pos.section_index())?)?.name == "minecraft:end_gateway")
}

impl SaveDir {
    /// Horizon-only legacy migration, before this store's chunk loaders start.
    /// Chunk NBT is authoritative; preserve its exit and all unrelated tags.
    /// Retire the JSON only after the region writer confirms every update.
    pub fn migrate_end_gateway_exits(&self) -> Result<usize> {
        let path = self.root.join(LEGACY_FILE);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e.into()),
        };
        let entries: Vec<([i32; 3], [i32; 3], bool)> = serde_json::from_slice(&bytes)?;
        // The old in-memory map kept the last entry for a position.
        let entries: BTreeMap<_, _> = entries.into_iter().map(|(p, e, x)| (p, (e, x))).collect();
        let mut chunks = BTreeMap::new();
        let mut changed = HashSet::new();
        let mut migrated = 0;
        for (p, (exit, exact)) in entries {
            let pos = BlockPos::new(p[0], p[1], p[2]);
            let key = (p[0] >> 4, p[2] >> 4);
            let root = match chunks.entry(key) {
                std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::btree_map::Entry::Vacant(e) => {
                    // Propagate damaged/unreadable regions; never discard their legacy exits.
                    let Some(mut root) =
                        read_region_record(&self.root, &self.regions, "region", key.0, key.1)?
                    else {
                        continue;
                    };
                    self.upgrade_record(Folder::Region, &mut root)?;
                    e.insert(root)
                }
            };
            if !is_gateway(&root.tag, pos)? {
                continue;
            }
            let Tag::Compound(fields) = &mut root.tag else {
                return Err("invalid chunk compound".into());
            };
            let Tag::List(entities) = fields
                .entry("block_entities".into())
                .or_insert_with(|| Tag::List(Vec::new()))
            else {
                return Err("invalid chunk block entities".into());
            };
            let index = entities.iter().position(|e| {
                ["x", "y", "z"]
                    .iter()
                    .zip(p)
                    .all(|(k, n)| number(e.get(k)) == Some(i64::from(n)))
            });
            let entity = match index {
                Some(i) => &mut entities[i],
                None => {
                    entities.push(Tag::Compound(Compound::from_iter([
                        ("id".into(), Tag::String("minecraft:end_gateway".into())),
                        ("x".into(), Tag::Int(p[0])),
                        ("y".into(), Tag::Int(p[1])),
                        ("z".into(), Tag::Int(p[2])),
                    ])));
                    entities.last_mut().unwrap()
                }
            };
            if text(entity.get("id")).as_deref() != Some("minecraft:end_gateway") {
                return Err("gateway migration: mismatched block entity".into());
            }
            if decode_end_gateway(entity).exit_portal.is_some() {
                continue;
            }
            let exit = Tag::IntArray(exit.to_vec());
            if crate::block_entity_nbt::gateway_exit(&exit).is_none() {
                return Err("gateway migration: invalid legacy exit".into());
            }
            let Tag::Compound(entity) = entity else {
                unreachable!()
            };
            entity.insert("exit_portal".into(), exit);
            if exact {
                entity.insert("ExactTeleport".into(), Tag::Byte(1));
            } else {
                entity.remove(&"ExactTeleport".into());
            }
            changed.insert(key);
            migrated += 1;
        }
        let records = chunks
            .into_iter()
            .filter(|(k, _)| changed.contains(k))
            .map(|(k, root)| (k, Some(root)))
            .collect();
        self.write_records("region", records, true)?;
        fs::remove_file(path)?;
        Ok(migrated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_world::chunk::ChunkData;
    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn fixture() -> (PathBuf, SaveDir) {
        let dir = std::env::temp_dir().join(format!(
            "gateway-migration-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let save = SaveDir::new(dir.clone(), LevelShape::END);
        (dir, save)
    }
    fn root() -> NamedTag {
        NamedTag { name: "".into(), tag: horizon_world::nbt::parse_snbt(&format!(
            "{{DataVersion:{DATA_VERSION},xPos:0,zPos:0,Status:\"minecraft:full\",opaque:\"keep\",sections:[{{Y:4b,block_states:{{palette:[{{id:\"minecraft:end_gateway\"}}]}}}}],block_entities:[{{id:\"minecraft:end_gateway\",x:2,y:75,z:2,Age:321L,exit_portal:[I;9,80,9],ExactTeleport:0b,other:\"keep\"}}]}}"
        )).unwrap().0 }
    }
    fn write(save: &SaveDir, root: NamedTag) {
        save.write_records("region", vec![((0, 0), Some(root))], true)
            .unwrap();
    }
    fn read(save: &SaveDir) -> NamedTag {
        save.read_record("region", 0, 0).unwrap()
    }
    fn entity(root: &NamedTag, x: i32) -> &Tag {
        let Some(Tag::List(list)) = root.tag.get("block_entities") else {
            panic!("entities")
        };
        list.iter()
            .find(|e| e.get("x") == Some(&Tag::Int(x)))
            .unwrap()
    }

    #[test]
    fn legacy_gateways_migrate_once_with_nbt_precedence_and_no_stale_resurrection() {
        let (dir, save) = fixture();
        write(&save, root());
        fs::write(
            dir.join(LEGACY_FILE),
            serde_json::to_vec(&vec![
                ([1, 75, 1], [100, 80, 100], true),
                ([2, 75, 2], [200, 80, 200], true),
                ([3, 50, 3], [300, 80, 300], false),
                ([1000, 75, 0], [400, 80, 400], false),
            ])
            .unwrap(),
        )
        .unwrap();
        assert_eq!(save.migrate_end_gateway_exits().unwrap(), 1);
        assert!(!dir.join(LEGACY_FILE).exists());
        let saved = read(&save);
        assert_eq!(
            decode_end_gateway(entity(&saved, 1)).exit_portal,
            Some([100, 80, 100])
        );
        assert!(decode_end_gateway(entity(&saved, 1)).exact_teleport);
        assert_eq!(
            decode_end_gateway(entity(&saved, 2)).exit_portal,
            Some([9, 80, 9])
        );
        assert!(!decode_end_gateway(entity(&saved, 2)).exact_teleport);
        assert_eq!(
            entity(&saved, 2).get("other"),
            Some(&Tag::String("keep".into()))
        );
        assert_eq!(saved.tag.get("opaque"), Some(&Tag::String("keep".into())));
        assert_eq!(save.migrate_end_gateway_exits().unwrap(), 0);
        // A stale entry for a removed gateway must not create a block entity.
        let mut removed = ChunkData::new(saved).unwrap();
        removed
            .set_block_data(
                BlockPos::new(1, 75, 1),
                BlockState {
                    name: "minecraft:air".into(),
                    properties: Default::default(),
                },
            )
            .unwrap();
        let mut removed = removed.into_nbt();
        let Some(Tag::List(list)) = removed.tag.get_mut("block_entities") else {
            panic!("entities")
        };
        list.retain(|e| e.get("x") != Some(&Tag::Int(1)));
        write(&save, removed);
        fs::write(dir.join(LEGACY_FILE), b"[[[1,75,1],[100,80,100],true]]").unwrap();
        assert_eq!(save.migrate_end_gateway_exits().unwrap(), 0);
        let Some(Tag::List(list)) = read(&save).tag.get("block_entities").cloned() else {
            panic!("entities")
        };
        assert_eq!(list.len(), 1);
        drop(save);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn malformed_or_undurable_migrations_retain_the_legacy_file() {
        let (dir, save) = fixture();
        write(&save, root());
        let path = dir.join(LEGACY_FILE);
        fs::write(&path, b"malformed").unwrap();
        assert!(save.migrate_end_gateway_exits().is_err());
        assert_eq!(fs::read(&path).unwrap(), b"malformed");
        let bytes = b"[[[1,75,1],[100,80,100],false]]";
        fs::write(&path, bytes).unwrap();
        *save.outstanding.error.lock().unwrap() = Some("injected save failure".into());
        assert!(save.migrate_end_gateway_exits().is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        drop(save);
        fs::remove_dir_all(dir).unwrap();
    }
}
