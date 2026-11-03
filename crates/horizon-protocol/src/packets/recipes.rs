//! The recipe book's packets and the recipe displays they carry
//! (`RecipeDisplay`, `SlotDisplay`, `RecipeDisplayEntry`): the 26.3
//! `minecraft:recipe_display` and `minecraft:slot_display` types in id order.
use crate::{
    Result,
    buf::{Codec, Reader, Writer},
    item::{Stack, Template},
};
use horizon_world::nbt::Tag;

/// `HolderSet<Item>` on the network: a tag by name, or item ids.
#[derive(Clone, Debug, PartialEq)]
pub enum HolderSet {
    Tag(String),
    Items(Vec<i32>),
}
impl HolderSet {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let n = r.varint()? - 1;
        if n == -1 {
            return Ok(Self::Tag(r.string(32767)?));
        }
        let n = usize::try_from(n).map_err(|_| crate::Error::new("negative holder set size"))?;
        if n > r.remaining() {
            return Err(crate::Error::new("holder set larger than the packet"));
        }
        Ok(Self::Items(
            (0..n).map(|_| r.varint()).collect::<Result<_>>()?,
        ))
    }
    pub fn write(&self, w: &mut Writer) {
        match self {
            Self::Tag(name) => {
                w.varint(0);
                w.string(name);
            }
            Self::Items(ids) => {
                w.varint(ids.len() as i32 + 1);
                ids.iter().for_each(|id| w.varint(*id));
            }
        }
    }
}

/// A trim pattern as `ByteBufCodecs.holder` sends it: a registry id, or
/// the pattern itself.
#[derive(Clone, Debug, PartialEq)]
pub enum TrimPattern {
    Id(i32),
    Direct {
        asset: String,
        description: Tag,
        decal: bool,
    },
}

/// `SlotDisplay`: what a recipe book slot shows.
#[derive(Clone, Debug, PartialEq)]
pub enum SlotDisplay {
    Empty,
    AnyFuel,
    WithAnyPotion(Box<SlotDisplay>),
    /// The source, kept to the stacks with this data component type.
    OnlyWithComponent(Box<SlotDisplay>, i32),
    Item(i32),
    ItemStack(Stack),
    Tag(HolderSet),
    Dyed {
        dye: Box<SlotDisplay>,
        target: Box<SlotDisplay>,
    },
    SmithingTrim {
        base: Box<SlotDisplay>,
        material: Box<SlotDisplay>,
        pattern: TrimPattern,
    },
    WithRemainder {
        input: Box<SlotDisplay>,
        remainder: Box<SlotDisplay>,
    },
    Composite(Vec<SlotDisplay>),
}

/// How deep slot displays may nest (composites of composites…).
const MAX_DEPTH: u32 = 32;

impl SlotDisplay {
    pub fn read(r: &mut Reader) -> Result<Self> {
        Self::read_at(r, 0)
    }
    fn read_at(r: &mut Reader, depth: u32) -> Result<Self> {
        if depth > MAX_DEPTH {
            return Err(crate::Error::new("slot displays nest too deep"));
        }
        let inner = |r: &mut Reader| Self::read_at(r, depth + 1).map(Box::new);
        Ok(match r.varint()? {
            0 => Self::Empty,
            1 => Self::AnyFuel,
            2 => Self::WithAnyPotion(inner(r)?),
            3 => {
                let source = inner(r)?;
                Self::OnlyWithComponent(source, r.varint()?)
            }
            4 => Self::Item(r.varint()?),
            5 => Self::ItemStack(Template::read(r)?),
            6 => Self::Tag(HolderSet::read(r)?),
            7 => Self::Dyed {
                dye: inner(r)?,
                target: inner(r)?,
            },
            8 => {
                let base = inner(r)?;
                let material = inner(r)?;
                let pattern = match r.varint()? {
                    0 => TrimPattern::Direct {
                        asset: r.string(32767)?,
                        description: r.nbt()?,
                        decal: r.bool()?,
                    },
                    id => TrimPattern::Id(id - 1),
                };
                Self::SmithingTrim {
                    base,
                    material,
                    pattern,
                }
            }
            9 => Self::WithRemainder {
                input: inner(r)?,
                remainder: inner(r)?,
            },
            10 => {
                let n = r.count(r.remaining())?;
                let mut out = Vec::with_capacity(n.min(256));
                for _ in 0..n {
                    out.push(Self::read_at(r, depth + 1)?);
                }
                Self::Composite(out)
            }
            id => return Err(crate::Error::new(format!("unknown slot display {id}"))),
        })
    }
    pub fn write(&self, w: &mut Writer) {
        match self {
            Self::Empty => w.varint(0),
            Self::AnyFuel => w.varint(1),
            Self::WithAnyPotion(d) => {
                w.varint(2);
                d.write(w);
            }
            Self::OnlyWithComponent(d, component) => {
                w.varint(3);
                d.write(w);
                w.varint(*component);
            }
            Self::Item(id) => {
                w.varint(4);
                w.varint(*id);
            }
            Self::ItemStack(stack) => {
                w.varint(5);
                Template::write(stack, w);
            }
            Self::Tag(set) => {
                w.varint(6);
                set.write(w);
            }
            Self::Dyed { dye, target } => {
                w.varint(7);
                dye.write(w);
                target.write(w);
            }
            Self::SmithingTrim {
                base,
                material,
                pattern,
            } => {
                w.varint(8);
                base.write(w);
                material.write(w);
                match pattern {
                    TrimPattern::Id(id) => w.varint(id + 1),
                    TrimPattern::Direct {
                        asset,
                        description,
                        decal,
                    } => {
                        w.varint(0);
                        w.string(asset);
                        w.nbt(description);
                        w.bool(*decal);
                    }
                }
            }
            Self::WithRemainder { input, remainder } => {
                w.varint(9);
                input.write(w);
                remainder.write(w);
            }
            Self::Composite(list) => {
                w.varint(10);
                w.varint(list.len() as i32);
                list.iter().for_each(|d| d.write(w));
            }
        }
    }
}

fn read_list(r: &mut Reader) -> Result<Vec<SlotDisplay>> {
    let n = r.count(r.remaining())?;
    let mut out = Vec::with_capacity(n.min(256));
    for _ in 0..n {
        out.push(SlotDisplay::read(r)?);
    }
    Ok(out)
}
fn write_list(list: &[SlotDisplay], w: &mut Writer) {
    w.varint(list.len() as i32);
    list.iter().for_each(|d| d.write(w));
}

/// `RecipeDisplay`: a recipe as the recipe book draws it.
#[derive(Clone, Debug, PartialEq)]
pub enum RecipeDisplay {
    Shapeless {
        ingredients: Vec<SlotDisplay>,
        result: SlotDisplay,
        station: SlotDisplay,
    },
    Shaped {
        width: i32,
        height: i32,
        ingredients: Vec<SlotDisplay>,
        result: SlotDisplay,
        station: SlotDisplay,
    },
    Furnace {
        ingredient: SlotDisplay,
        fuel: SlotDisplay,
        result: SlotDisplay,
        station: SlotDisplay,
        duration: i32,
        experience: f32,
    },
    Stonecutter {
        input: SlotDisplay,
        result: SlotDisplay,
        station: SlotDisplay,
    },
    Smithing {
        template: SlotDisplay,
        base: SlotDisplay,
        addition: SlotDisplay,
        result: SlotDisplay,
        station: SlotDisplay,
    },
}
impl RecipeDisplay {
    pub fn result(&self) -> &SlotDisplay {
        match self {
            Self::Shapeless { result, .. }
            | Self::Shaped { result, .. }
            | Self::Furnace { result, .. }
            | Self::Stonecutter { result, .. }
            | Self::Smithing { result, .. } => result,
        }
    }
}
impl Codec<RecipeDisplay> for RecipeDisplay {
    fn read(r: &mut Reader) -> Result<Self> {
        let slot = SlotDisplay::read;
        Ok(match r.varint()? {
            0 => Self::Shapeless {
                ingredients: read_list(r)?,
                result: slot(r)?,
                station: slot(r)?,
            },
            1 => {
                let (width, height) = (r.varint()?, r.varint()?);
                let ingredients = read_list(r)?;
                if i64::from(width) * i64::from(height) != ingredients.len() as i64 {
                    return Err(crate::Error::new("shaped display size mismatch"));
                }
                Self::Shaped {
                    width,
                    height,
                    ingredients,
                    result: slot(r)?,
                    station: slot(r)?,
                }
            }
            2 => Self::Furnace {
                ingredient: slot(r)?,
                fuel: slot(r)?,
                result: slot(r)?,
                station: slot(r)?,
                duration: r.varint()?,
                experience: r.f32()?,
            },
            3 => Self::Stonecutter {
                input: slot(r)?,
                result: slot(r)?,
                station: slot(r)?,
            },
            4 => Self::Smithing {
                template: slot(r)?,
                base: slot(r)?,
                addition: slot(r)?,
                result: slot(r)?,
                station: slot(r)?,
            },
            id => return Err(crate::Error::new(format!("unknown recipe display {id}"))),
        })
    }
    fn write(v: &Self, w: &mut Writer) {
        match v {
            Self::Shapeless {
                ingredients,
                result,
                station,
            } => {
                w.varint(0);
                write_list(ingredients, w);
                result.write(w);
                station.write(w);
            }
            Self::Shaped {
                width,
                height,
                ingredients,
                result,
                station,
            } => {
                w.varint(1);
                w.varint(*width);
                w.varint(*height);
                write_list(ingredients, w);
                result.write(w);
                station.write(w);
            }
            Self::Furnace {
                ingredient,
                fuel,
                result,
                station,
                duration,
                experience,
            } => {
                w.varint(2);
                ingredient.write(w);
                fuel.write(w);
                result.write(w);
                station.write(w);
                w.varint(*duration);
                w.f32(*experience);
            }
            Self::Stonecutter {
                input,
                result,
                station,
            } => {
                w.varint(3);
                input.write(w);
                result.write(w);
                station.write(w);
            }
            Self::Smithing {
                template,
                base,
                addition,
                result,
                station,
            } => {
                w.varint(4);
                template.write(w);
                base.write(w);
                addition.write(w);
                result.write(w);
                station.write(w);
            }
        }
    }
}

/// `RecipeDisplayEntry` with `ClientboundRecipeBookAddPacket.Entry`'s flags
/// (1 show a notification, 2 highlight as new).
#[derive(Clone, Debug, PartialEq)]
pub struct RecipeBookEntry {
    pub id: i32,
    pub display: RecipeDisplay,
    /// The recipe group (0 none, else the group's index + 1).
    pub group: i32,
    /// `minecraft:recipe_book_category` id.
    pub category: i32,
    /// The ingredients the "craftable" filter checks, if the recipe has any.
    pub requirements: Option<Vec<HolderSet>>,
    pub flags: i8,
}

/// ClientboundRecipeBookAddPacket: the entries, and whether they replace the
/// client's book.
#[derive(Clone, Debug, PartialEq)]
pub struct RecipeBookAdd {
    pub entries: Vec<RecipeBookEntry>,
    pub replace: bool,
}
impl RecipeBookEntry {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let id = r.varint()?;
        let display = RecipeDisplay::read(r)?;
        let group = r.varint()?;
        let category = r.varint()?;
        let requirements = if r.bool()? {
            let k = r.count(r.remaining())?;
            let mut list = Vec::with_capacity(k.min(64));
            for _ in 0..k {
                list.push(HolderSet::read(r)?);
            }
            Some(list)
        } else {
            None
        };
        Ok(Self {
            id,
            display,
            group,
            category,
            requirements,
            flags: r.i8()?,
        })
    }
}
impl Codec<RecipeBookAdd> for RecipeBookAdd {
    fn read(r: &mut Reader) -> Result<Self> {
        let n = r.count(r.remaining())?;
        let mut entries = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            entries.push(RecipeBookEntry::read(r)?);
        }
        Ok(Self {
            entries,
            replace: r.bool()?,
        })
    }
    fn write(v: &Self, w: &mut Writer) {
        w.varint(v.entries.len() as i32);
        for e in &v.entries {
            w.varint(e.id);
            RecipeDisplay::write(&e.display, w);
            w.varint(e.group);
            w.varint(e.category);
            w.bool(e.requirements.is_some());
            if let Some(list) = &e.requirements {
                w.varint(list.len() as i32);
                list.iter().for_each(|s| s.write(w));
            }
            w.i8(e.flags);
        }
        w.bool(v.replace);
    }
}

/// ClientboundRecipeBookRemovePacket: the display ids.
#[derive(Clone, Debug, PartialEq)]
pub struct RecipeBookRemove {
    pub ids: Vec<i32>,
}
impl Codec<RecipeBookRemove> for RecipeBookRemove {
    fn read(r: &mut Reader) -> Result<Self> {
        let n = r.count(r.remaining())?;
        Ok(Self {
            ids: (0..n).map(|_| r.varint()).collect::<Result<_>>()?,
        })
    }
    fn write(v: &Self, w: &mut Writer) {
        w.varint(v.ids.len() as i32);
        v.ids.iter().for_each(|id| w.varint(*id));
    }
}

/// RecipeBookSettings: each book's (open, filtering), in `RecipeBookType`
/// order (crafting, furnace, blast furnace, smoker).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecipeBookSettings {
    pub books: [(bool, bool); 4],
}
impl Codec<RecipeBookSettings> for RecipeBookSettings {
    fn read(r: &mut Reader) -> Result<Self> {
        let mut books = [(false, false); 4];
        for b in &mut books {
            *b = (r.bool()?, r.bool()?);
        }
        Ok(Self { books })
    }
    fn write(v: &Self, w: &mut Writer) {
        for (open, filtering) in v.books {
            w.bool(open);
            w.bool(filtering);
        }
    }
}

/// ServerboundRecipeBookSeenRecipePacket: the `RecipeDisplayId` the player
/// has looked at, so the server stops treating it as a new one.
#[derive(Clone, Debug, PartialEq)]
pub struct RecipeBookSeenRecipe {
    pub recipe: i32,
}
impl Codec<RecipeBookSeenRecipe> for RecipeBookSeenRecipe {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            recipe: r.varint()?,
        })
    }
    fn write(v: &Self, w: &mut Writer) {
        w.varint(v.recipe);
    }
}

/// ServerboundRecipeBookChangeSettingsPacket: the active book's open and
/// filtering flags, with its `RecipeBookType` ordinal as a VarInt.
#[derive(Clone, Debug, PartialEq)]
pub struct RecipeBookChangeSettings {
    pub book_type: i32,
    pub open: bool,
    pub filtering: bool,
}
impl Codec<RecipeBookChangeSettings> for RecipeBookChangeSettings {
    fn read(r: &mut Reader) -> Result<Self> {
        let book_type = r.varint()?;
        if !(0..4).contains(&book_type) {
            return Err(crate::Error::new("invalid recipe book type"));
        }
        Ok(Self {
            book_type,
            open: r.bool()?,
            filtering: r.bool()?,
        })
    }
    fn write(v: &Self, w: &mut Writer) {
        w.varint(v.book_type);
        w.bool(v.open);
        w.bool(v.filtering);
    }
}

/// ClientboundPlaceGhostRecipePacket: the recipe to show faded in a menu
/// whose items were missing.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaceGhostRecipe {
    pub container: i32,
    pub display: RecipeDisplay,
}
impl Codec<PlaceGhostRecipe> for PlaceGhostRecipe {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            container: r.varint()?,
            display: RecipeDisplay::read(r)?,
        })
    }
    fn write(v: &Self, w: &mut Writer) {
        w.varint(v.container);
        RecipeDisplay::write(&v.display, w);
    }
}

/// ServerboundPlaceRecipePacket: place the recipe with this display id in
/// the open menu, as many times as possible when `use_max` (shift).
#[derive(Clone, Debug, PartialEq)]
pub struct PlaceRecipe {
    pub container: i32,
    pub recipe: i32,
    pub use_max: bool,
}
impl Codec<PlaceRecipe> for PlaceRecipe {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            container: r.varint()?,
            recipe: r.varint()?,
            use_max: r.bool()?,
        })
    }
    fn write(v: &Self, w: &mut Writer) {
        w.varint(v.container);
        w.varint(v.recipe);
        w.bool(v.use_max);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn book_entries_round_trip() {
        let add = RecipeBookAdd {
            entries: vec![
                RecipeBookEntry {
                    id: 7,
                    display: RecipeDisplay::Shaped {
                        width: 2,
                        height: 1,
                        ingredients: vec![
                            SlotDisplay::Tag(HolderSet::Tag("minecraft:planks".into())),
                            SlotDisplay::Empty,
                        ],
                        result: SlotDisplay::ItemStack(Stack::new(12, 4)),
                        station: SlotDisplay::Item(300),
                    },
                    group: 3,
                    category: 3,
                    requirements: Some(vec![HolderSet::Items(vec![1, 2])]),
                    flags: 3,
                },
                RecipeBookEntry {
                    id: 8,
                    display: RecipeDisplay::Smithing {
                        template: SlotDisplay::Composite(vec![SlotDisplay::Item(1)]),
                        base: SlotDisplay::WithRemainder {
                            input: Box::new(SlotDisplay::AnyFuel),
                            remainder: Box::new(SlotDisplay::Item(2)),
                        },
                        addition: SlotDisplay::OnlyWithComponent(
                            Box::new(SlotDisplay::WithAnyPotion(Box::new(SlotDisplay::Item(3)))),
                            9,
                        ),
                        result: SlotDisplay::SmithingTrim {
                            base: Box::new(SlotDisplay::Item(4)),
                            material: Box::new(SlotDisplay::Dyed {
                                dye: Box::new(SlotDisplay::Item(5)),
                                target: Box::new(SlotDisplay::Item(6)),
                            }),
                            pattern: TrimPattern::Id(2),
                        },
                        station: SlotDisplay::Item(7),
                    },
                    group: 0,
                    category: 11,
                    requirements: None,
                    flags: 0,
                },
            ],
            replace: true,
        };
        let mut w = Writer::new();
        RecipeBookAdd::write(&add, &mut w);
        let mut r = Reader::new(&w.data);
        assert_eq!(RecipeBookAdd::read(&mut r).unwrap(), add);
        assert_eq!(r.remaining(), 0);
    }
}
