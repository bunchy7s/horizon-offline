//! The pure parts of `/data` (DataCommands): merging compounds, the
//! `modify` operations over an NBT path, string sources and the numeric
//! result of `get`.
use crate::{
    command::CommandError,
    nbt_path::{CollectionError, NbtPath, PathError, too_deep},
    snbt::to_snbt,
    text::Text,
};
use horizon_world::nbt::{Compound, Tag};

/// `CompoundTag.merge`: compounds merge recursively, anything else replaces.
pub fn merge_compound(target: &mut Compound, source: &Compound) {
    for (k, v) in source {
        match (target.get_mut(k), v) {
            (Some(Tag::Compound(t)), Tag::Compound(s)) => merge_compound(t, s),
            _ => {
                target.insert(k.clone(), v.clone());
            }
        }
    }
}
/// `old.copy().merge(nbt)`.
pub fn merged(old: &Tag, nbt: &Tag) -> Tag {
    let mut result = old.clone();
    if let (Tag::Compound(t), Tag::Compound(s)) = (&mut result, nbt) {
        merge_compound(t, s);
    }
    result
}

pub fn path_error(e: PathError) -> CommandError {
    let args: Vec<&dyn std::fmt::Display> =
        e.args.iter().map(|a| a as &dyn std::fmt::Display).collect();
    CommandError::tr(e.key, &args)
}
pub fn collection_error(e: CollectionError) -> CommandError {
    match e {
        CollectionError::Path(p) => path_error(p),
        CollectionError::ExpectedList(t) => {
            CommandError::tr("commands.data.modify.expected_list", &[&t])
        }
        CollectionError::InvalidIndex(i) => {
            CommandError::tr("commands.data.modify.invalid_index", &[&i])
        }
    }
}

/// The `modify` operation words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Insert(i32),
    Prepend,
    Append,
    Set,
    Merge,
}
/// Run a `modify` operation; the count is how many tags changed (0 = fail).
pub fn modify(
    op: Op,
    target: &mut Tag,
    path: &NbtPath,
    sources: &[Tag],
) -> Result<usize, CommandError> {
    match op {
        Op::Insert(i) => path.insert(i, target, sources).map_err(collection_error),
        Op::Prepend => path.insert(0, target, sources).map_err(collection_error),
        Op::Append => path.insert(-1, target, sources).map_err(collection_error),
        Op::Set => {
            let Some(last) = sources.last() else {
                return Ok(0);
            };
            path.set(target, last).map_err(path_error)
        }
        Op::Merge => {
            let mut combined = Compound::new();
            for s in sources {
                if too_deep(s, 0) {
                    return Err(CommandError::tr("arguments.nbtpath.too_deep", &[]));
                }
                let Tag::Compound(c) = s else {
                    return Err(CommandError::tr(
                        "commands.data.modify.expected_object",
                        &[&to_snbt(s)],
                    ));
                };
                merge_compound(&mut combined, c);
            }
            let mut changed = 0;
            let mut error = None;
            path.get_or_create(target, &|| Tag::Compound(Compound::new()), &mut |t| {
                if error.is_some() {
                    return;
                }
                match t {
                    Tag::Compound(c) => {
                        let before = c.clone();
                        merge_compound(c, &combined);
                        changed += usize::from(before != *c);
                    }
                    other => {
                        error = Some(CommandError::tr(
                            "commands.data.modify.expected_object",
                            &[&to_snbt(other)],
                        ));
                    }
                }
            })
            .map_err(path_error)?;
            match error {
                Some(e) => Err(e),
                None => Ok(changed),
            }
        }
    }
}
/// `getAsText`: strings and numbers as text, anything else is an error.
pub fn as_text(tag: &Tag) -> Result<String, CommandError> {
    match tag {
        Tag::String(s) => Ok(s.to_string_lossy()),
        Tag::Byte(_)
        | Tag::Short(_)
        | Tag::Int(_)
        | Tag::Long(_)
        | Tag::Float(_)
        | Tag::Double(_) => {
            // PrimitiveTag.toString: the SNBT form (with its suffix).
            Ok(to_snbt(tag))
        }
        other => Err(CommandError::tr(
            "commands.data.modify.expected_value",
            &[&to_snbt(other)],
        )),
    }
}
/// `substring(input, start[, end])` with negative offsets from the end.
pub fn substring(input: &str, start: i32, end: Option<i32>) -> Result<String, CommandError> {
    let chars: Vec<u16> = input.encode_utf16().collect();
    let len = chars.len() as i32;
    let offset = |i: i32| if i >= 0 { i } else { len + i };
    let (s, e) = (offset(start), end.map_or(len, offset));
    if s >= 0 && e <= len && s <= e {
        Ok(String::from_utf16_lossy(&chars[s as usize..e as usize]))
    } else {
        Err(CommandError::tr(
            "commands.data.modify.invalid_substring",
            &[&s, &e],
        ))
    }
}
/// `getSingleTag`: exactly one tag at the path.
pub fn single_tag(path: &NbtPath, data: &Tag) -> Result<Tag, CommandError> {
    let mut tags = path.get(data).map_err(path_error)?;
    if tags.len() > 1 {
        return Err(CommandError::tr("commands.data.get.multiple", &[]));
    }
    Ok(tags.remove(0))
}
/// The value of a tag as a double (NumericTag.doubleValue).
pub fn numeric(tag: &Tag) -> Option<f64> {
    Some(match tag {
        Tag::Byte(v) => f64::from(*v),
        Tag::Short(v) => f64::from(*v),
        Tag::Int(v) => f64::from(*v),
        Tag::Long(v) => *v as f64,
        Tag::Float(v) => f64::from(f32::from_bits(*v)),
        Tag::Double(v) => f64::from_bits(*v),
        _ => return None,
    })
}
/// `data get <path>`'s return value: the number's floor, or a size.
pub fn get_value(tag: &Tag, path: &NbtPath) -> Result<i32, CommandError> {
    Ok(match tag {
        Tag::List(l) => l.len() as i32,
        Tag::ByteArray(a) => a.len() as i32,
        Tag::IntArray(a) => a.len() as i32,
        Tag::LongArray(a) => a.len() as i32,
        Tag::Compound(c) => c.len() as i32,
        Tag::String(s) => s.0.len() as i32,
        Tag::End => {
            return Err(CommandError::tr(
                "commands.data.get.unknown",
                &[&path.as_str()],
            ));
        }
        other => numeric(other).map_or(0, |v| v.floor() as i32),
    })
}
/// `Mth.floor(value * scale)` for `data get <path> <scale>`.
pub fn scaled_value(tag: &Tag, path: &NbtPath, scale: f64) -> Result<i32, CommandError> {
    match numeric(tag) {
        Some(v) => Ok((v * scale).floor() as i32),
        None => Err(CommandError::tr(
            "commands.data.get.invalid",
            &[&path.as_str()],
        )),
    }
}
/// The text `data get ... <scale>` prints for the scale (`%.2f`).
pub fn scale_text(scale: f64) -> Text {
    Text::literal(crate::java_text::format_fixed(scale, 2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_world::nbt::parse_snbt;

    fn tag(s: &str) -> Tag {
        parse_snbt(s).unwrap().0
    }
    fn path(s: &str) -> NbtPath {
        NbtPath::parse(s).unwrap()
    }

    #[test]
    fn merge_is_recursive() {
        let old = tag("{a:{b:1,c:2},d:[1]}");
        assert_eq!(
            to_snbt(&merged(&old, &tag("{a:{b:9},e:1}"))),
            "{a:{b:9,c:2},d:[1],e:1}"
        );
        assert_eq!(merged(&old, &tag("{}")), old);
        // Lists and scalars are replaced, not merged.
        assert_eq!(
            to_snbt(&merged(&old, &tag("{d:[2]}"))),
            "{a:{b:1,c:2},d:[2]}"
        );
    }

    #[test]
    fn modify_operations() {
        let mut t = tag("{l:[1,2],o:{a:1}}");
        assert_eq!(
            modify(Op::Append, &mut t, &path("l"), &[tag("3")]).unwrap(),
            1
        );
        assert_eq!(
            modify(Op::Prepend, &mut t, &path("l"), &[tag("0")]).unwrap(),
            1
        );
        assert_eq!(
            modify(Op::Insert(2), &mut t, &path("l"), &[tag("9")]).unwrap(),
            1
        );
        assert_eq!(to_snbt(&t), "{l:[0,1,9,2,3],o:{a:1}}");
        assert_eq!(
            modify(Op::Set, &mut t, &path("o.a"), &[tag("1"), tag("2")]).unwrap(),
            1
        );
        assert_eq!(
            modify(Op::Set, &mut t, &path("o.a"), &[tag("2")]).unwrap(),
            0
        );
        assert_eq!(
            modify(Op::Merge, &mut t, &path("o"), &[tag("{b:1}")]).unwrap(),
            1
        );
        assert_eq!(
            modify(Op::Merge, &mut t, &path("o"), &[tag("{b:1}")]).unwrap(),
            0
        );
        assert_eq!(to_snbt(&t), "{l:[0,1,9,2,3],o:{a:2,b:1}}");
        let e = modify(Op::Merge, &mut t, &path("o"), &[tag("3")]).unwrap_err();
        assert_eq!(e.message.plain(), "commands.data.modify.expected_object");
        let e = modify(Op::Append, &mut t, &path("o"), &[tag("3")]).unwrap_err();
        assert_eq!(e.message.plain(), "commands.data.modify.expected_list");
        // Merge creates the target compound when missing.
        assert_eq!(
            modify(Op::Merge, &mut t, &path("new.deep"), &[tag("{x:1}")]).unwrap(),
            1
        );
        assert_eq!(single_tag(&path("new.deep.x"), &t).unwrap(), Tag::Int(1));
    }

    #[test]
    fn strings_and_values() {
        assert_eq!(substring("abcdef", 1, Some(3)).unwrap(), "bc");
        assert_eq!(substring("abcdef", -2, None).unwrap(), "ef");
        assert_eq!(substring("abcdef", 0, Some(-1)).unwrap(), "abcde");
        assert!(substring("abc", 2, Some(1)).is_err());
        assert!(substring("abc", 0, Some(9)).is_err());
        assert_eq!(as_text(&tag("\"x\"")).unwrap(), "x");
        assert_eq!(as_text(&tag("5b")).unwrap(), "5b");
        assert!(as_text(&tag("[1]")).is_err());
        let data = tag("{n:2.7d,s:\"abcd\",l:[1,2,3],c:{a:1,b:2},m:[1,2]}");
        assert_eq!(
            get_value(&single_tag(&path("n"), &data).unwrap(), &path("n")).unwrap(),
            2
        );
        assert_eq!(
            get_value(&single_tag(&path("s"), &data).unwrap(), &path("s")).unwrap(),
            4
        );
        assert_eq!(
            get_value(&single_tag(&path("l"), &data).unwrap(), &path("l")).unwrap(),
            3
        );
        assert_eq!(
            get_value(&single_tag(&path("c"), &data).unwrap(), &path("c")).unwrap(),
            2
        );
        assert_eq!(scaled_value(&Tag::Int(7), &path("n"), 0.5).unwrap(), 3);
        assert_eq!(
            scaled_value(&Tag::Double((-0.5f64).to_bits()), &path("n"), 1.0).unwrap(),
            -1
        );
        assert!(scaled_value(&tag("\"x\""), &path("s"), 1.0).is_err());
        assert_eq!(
            single_tag(&path("m[]"), &data).unwrap_err().message.plain(),
            "commands.data.get.multiple"
        );
        assert_eq!(scale_text(1.0).plain(), "1.00");
        assert_eq!(scale_text(0.125).plain(), "0.13");
    }
}
