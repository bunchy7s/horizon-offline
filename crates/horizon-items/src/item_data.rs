//! Item stack components beyond count and damage: enchantments, stored
//! (book) enchantments, `repair_cost`, `custom_name`, `potion_contents` and
//! `trim`, with their NBT form in vanilla's `components` compound.
use crate::items::ItemStack;
use horizon_world::nbt::{JavaString, Tag};
use std::sync::Arc;

/// Enchantment id and level, in insertion order.
pub type Levels = Vec<(Arc<str>, u32)>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StackData {
    /// `minecraft:enchantments`.
    pub enchantments: Levels,
    /// `minecraft:stored_enchantments` (enchanted books).
    pub stored_enchantments: Levels,
    /// `minecraft:repair_cost` (anvil prior-work penalty).
    pub repair_cost: u32,
    /// `minecraft:custom_name` as plain text.
    pub custom_name: Option<String>,
    /// `minecraft:potion_contents.potion`.
    pub potion: Option<Arc<str>>,
    /// `minecraft:trim`: (material, pattern).
    pub trim: Option<(Arc<str>, Arc<str>)>,
    /// `minecraft:container` (shulker boxes): the non-empty slots.
    pub container: Vec<(u8, ItemStack)>,
    /// `minecraft:charged_projectiles` (a loaded crossbow).
    pub charged: Vec<ItemStack>,
    /// `minecraft:fireworks`: flight duration and explosions (None: the
    /// item's default).
    pub fireworks: Option<(u8, Vec<crate::firework::FireworkExplosion>)>,
    /// `minecraft:map_id` (filled maps).
    pub map_id: Option<i32>,
    /// `minecraft:writable_book_content` pages.
    pub writable_pages: Vec<String>,
    /// `minecraft:written_book_content`.
    pub written: Option<WrittenBook>,
    /// `minecraft:banner_patterns` (banners and shields).
    pub banner_patterns: Vec<crate::banner_layer::BannerLayer>,
    /// `minecraft:base_color` (a shield's dye).
    pub base_color: Option<Arc<str>>,
    /// The raid captain's banner: `minecraft:banner_patterns` with the
    /// ominous layers, its item name, rarity and hidden tooltip.
    pub ominous_banner: bool,
    /// `minecraft:ominous_bottle_amplifier` (None: the item's default, 0).
    pub ominous_amplifier: Option<u8>,
    /// `minecraft:dyed_color` (leather armour, wolf armour, ... as RGB).
    pub dyed_color: Option<i32>,
    /// `minecraft:firework_explosion` (a firework star).
    pub firework_explosion: Option<crate::firework::FireworkExplosion>,
    /// `minecraft:pot_decorations`: back, left, right, front sherds.
    pub pot_decorations: Option<Box<[Option<ItemStack>; 4]>>,
    /// `minecraft:bundle_contents`.
    pub bundle_contents: Vec<ItemStack>,
    /// `BundleContents.selectedItem`: runtime only, never saved or sent.
    pub bundle_selected: BundleSelected,
    /// `minecraft:suspicious_stew_effects`: (effect id, duration).
    pub stew_effects: Vec<(Arc<str>, i32)>,
    /// `minecraft:map_post_processing`: 0 lock, 1 scale.
    pub map_post_processing: Option<u8>,
    /// Components Horizon does not model (lore, custom data, item name,
    /// max stack size...), kept as their saved values in key order.
    pub other: Vec<(String, Tag)>,
    /// What the server sent for this stack, for sending it back unchanged.
    pub wire: Wire,
}
/// The component patch a server sent for a stack, as the creative slot packet
/// writes it, and the stack Horizon decoded from it, to tell whether anything
/// changed since. Like vanilla's `BundleContents.selectedItem` it is left out
/// when stacks are compared.
#[derive(Clone, Debug, Default)]
pub struct Wire(Option<Arc<(ItemStack, Arc<[u8]>)>>);
impl Wire {
    pub fn new(decoded: ItemStack, patch: Arc<[u8]>) -> Self {
        Self(Some(Arc::new((decoded, patch))))
    }
}
impl PartialEq for Wire {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
/// A bundle's selected item index (-1: none). Like vanilla's
/// `BundleContents.equals` it is left out when stacks are compared.
#[derive(Clone, Copy, Debug)]
pub struct BundleSelected(pub i32);
impl Default for BundleSelected {
    fn default() -> Self {
        Self(-1)
    }
}
impl PartialEq for BundleSelected {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
/// The components `components_tag` writes from modelled fields.
const MODELLED: &[&str] = &[
    "minecraft:banner_patterns",
    "minecraft:base_color",
    "minecraft:bundle_contents",
    "minecraft:charged_projectiles",
    "minecraft:container",
    "minecraft:custom_name",
    "minecraft:damage",
    "minecraft:dyed_color",
    "minecraft:enchantments",
    "minecraft:firework_explosion",
    "minecraft:fireworks",
    "minecraft:map_id",
    "minecraft:map_post_processing",
    "minecraft:ominous_bottle_amplifier",
    "minecraft:pot_decorations",
    "minecraft:potion_contents",
    "minecraft:repair_cost",
    "minecraft:stored_enchantments",
    "minecraft:suspicious_stew_effects",
    "minecraft:trim",
    "minecraft:writable_book_content",
    "minecraft:written_book_content",
];
impl StackData {
    /// An unmodelled component's saved value.
    pub fn other(&self, id: &str) -> Option<&Tag> {
        self.other.iter().find(|(k, _)| k == id).map(|(_, v)| v)
    }
    /// `SetNameFunction.run`: a loot text component stored as `custom_name`
    /// (plain text, as modelled) or as the unmodelled `item_name`.
    pub fn set_loot_name(&mut self, id: &str, text: &serde_json::Value) {
        let tag = horizon_client_core::scoreboard::json_to_tag(text);
        if id == "minecraft:custom_name" {
            self.custom_name = read_text(&tag);
        } else {
            self.set_other(id, Some(tag));
        }
    }
    /// `SetComponentsFunction`: one component of a loot patch, in its JSON
    /// form. Trim is modelled, a custom name is text, the rest are kept as
    /// saved. A null value removes the component.
    pub fn set_loot_component(&mut self, id: &str, value: &serde_json::Value) {
        let tag = (!value.is_null()).then(|| horizon_client_core::scoreboard::json_to_tag(value));
        match id {
            "minecraft:trim" => {
                self.trim = match (value["material"].as_str(), value["pattern"].as_str()) {
                    (Some(material), Some(pattern)) => {
                        Some((Arc::from(material), Arc::from(pattern)))
                    }
                    _ => None,
                };
            }
            "minecraft:custom_name" => self.custom_name = tag.as_ref().and_then(read_text),
            _ => self.set_other(id, tag),
        }
    }
    /// Set (or with None remove) an unmodelled component.
    pub fn set_other(&mut self, id: &str, value: Option<Tag>) {
        self.other.retain(|(k, _)| k != id);
        if let Some(v) = value {
            self.other.push((id.to_owned(), v));
            self.other.sort_by(|a, b| a.0.cmp(&b.0));
        }
    }
}
/// `Raid.getBannerComponentPatch`: the ominous banner's pattern layers.
pub const OMINOUS_LAYERS: [(&str, &str); 8] = [
    ("rhombus", "cyan"),
    ("stripe_bottom", "light_gray"),
    ("stripe_center", "gray"),
    ("border", "light_gray"),
    ("stripe_middle", "black"),
    ("half_horizontal", "light_gray"),
    ("circle", "light_gray"),
    ("border", "black"),
];

/// The raid captain banner's default tooltip setting, stored by vanilla as a
/// component but represented by `StackData::ominous_banner` in Horizon.
fn ominous_tooltip_display() -> Tag {
    Tag::compound([(
        "hidden_components",
        Tag::List(vec![Tag::String("minecraft:banner_patterns".into())]),
    )])
}

/// `WrittenBookContent` with the pages as plain text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WrittenBook {
    pub title: String,
    pub author: String,
    pub generation: u8,
    pub pages: Vec<String>,
    /// Each page's component when it is not a plain string, so that its
    /// click events survive; empty or one per page.
    pub page_tags: Vec<Tag>,
}
impl StackData {
    pub fn is_empty(&self) -> bool {
        self.wire.0.is_none() && *self == Self::default()
    }
}

impl ItemStack {
    pub fn new(item: &str, count: u32) -> Self {
        Self {
            item: Arc::from(item),
            count,
            damage: 0,
            extra: None,
        }
    }
    pub fn data(&self) -> Option<&StackData> {
        self.extra.as_deref()
    }
    /// The patch the server sent for this stack, while the stack is still what
    /// it decoded to (its count aside); None once anything else changed.
    pub fn wire_patch(&self) -> Option<Arc<[u8]>> {
        let (decoded, patch) = &**self.data()?.wire.0.as_ref()?;
        let none = StackData::default();
        (decoded.item == self.item
            && decoded.damage == self.damage
            && decoded.data().unwrap_or(&none) == self.data().unwrap_or(&none))
        .then(|| patch.clone())
    }
    pub fn data_mut(&mut self) -> &mut StackData {
        self.extra.get_or_insert_with(Box::default)
    }
    /// Drops an all-default component box so equal stacks compare equal.
    pub fn tidy(&mut self) {
        if self.extra.as_ref().is_some_and(|d| d.is_empty()) {
            self.extra = None;
        }
    }
    pub fn enchantments(&self) -> &[(Arc<str>, u32)] {
        self.data().map_or(&[], |d| &d.enchantments)
    }
    pub fn stored_enchantments(&self) -> &[(Arc<str>, u32)] {
        self.data().map_or(&[], |d| &d.stored_enchantments)
    }
    /// `EnchantmentHelper.getItemEnchantmentLevel` (0 when absent).
    pub fn enchantment_level(&self, id: &str) -> u32 {
        self.enchantments()
            .iter()
            .find(|(e, _)| &**e == id)
            .map_or(0, |(_, l)| *l)
    }
    /// The component an anvil/grindstone works on: stored for books.
    pub fn crafting_enchantments(&self) -> &[(Arc<str>, u32)] {
        if &*self.item == "minecraft:enchanted_book" {
            self.stored_enchantments()
        } else {
            self.enchantments()
        }
    }
    pub fn set_crafting_enchantments(&mut self, levels: Levels) {
        if &*self.item == "minecraft:enchanted_book" {
            self.data_mut().stored_enchantments = levels;
        } else {
            self.data_mut().enchantments = levels;
        }
        self.tidy();
    }
    /// `ItemStack.enchant`: upgrade to the higher level.
    pub fn enchant(&mut self, id: &str, level: u32) {
        let book = &*self.item == "minecraft:enchanted_book";
        let data = self.data_mut();
        let list = if book {
            &mut data.stored_enchantments
        } else {
            &mut data.enchantments
        };
        upgrade(list, id, level);
    }
    pub fn has_any_enchantments(&self) -> bool {
        !self.enchantments().is_empty() || !self.stored_enchantments().is_empty()
    }
    /// `ItemStack.hasFoil`: an explicit `minecraft:enchantment_glint_override`
    /// component wins, then the item's default override (`item_glint`), then a
    /// non-empty `minecraft:enchantments` component (`Item.isFoil` is
    /// `ItemStack.isEnchanted`; stored book enchantments do not count).
    pub fn has_foil(&self, item_glint: bool) -> bool {
        if let Some(Tag::Byte(flag)) = self
            .data()
            .and_then(|d| d.other("minecraft:enchantment_glint_override"))
        {
            return *flag != 0;
        }
        item_glint || !self.enchantments().is_empty()
    }
    pub fn repair_cost(&self) -> u32 {
        self.data().map_or(0, |d| d.repair_cost)
    }
    pub fn custom_name(&self) -> Option<&str> {
        self.data().and_then(|d| d.custom_name.as_deref())
    }
    /// `minecraft:item_name` in the current language (`getItemName`'s
    /// component). It stays in the patch as loaded, so a translatable name
    /// saves and reaches clients unchanged; the ominous banner's is modelled
    /// by `ominous_banner` instead.
    pub fn item_name(&self) -> Option<String> {
        let name = self.data()?.other("minecraft:item_name")?;
        Some(horizon_client_core::component::name_text(name).plain())
    }
    pub fn potion(&self) -> Option<&str> {
        self.data().and_then(|d| d.potion.as_deref())
    }
    pub fn with_potion(mut self, potion: &str) -> Self {
        self.data_mut().potion = Some(Arc::from(potion));
        self
    }
    /// `ItemStack.isSameItemSameComponents`.
    pub fn same_item_same_components(&self, other: &Self) -> bool {
        self.item == other.item && self.damage == other.damage && self.data() == other.data()
    }
    /// `ItemStack.transmuteCopy`: another item keeping every component.
    pub fn transmute(&self, item: &str, count: u32) -> Self {
        let mut stack = Self {
            item: Arc::from(item),
            count,
            ..self.clone()
        };
        stack.tidy();
        stack
    }
}
/// `ItemEnchantments.Mutable.upgrade`.
pub fn upgrade(list: &mut Levels, id: &str, level: u32) {
    if level == 0 {
        return;
    }
    match list.iter_mut().find(|(e, _)| &**e == id) {
        Some((_, l)) => *l = (*l).max(level),
        None => list.push((Arc::from(id), level)),
    }
}
/// `ItemEnchantments.Mutable.set` (level 0 removes).
pub fn set_level(list: &mut Levels, id: &str, level: u32) {
    if level == 0 {
        list.retain(|(e, _)| &**e != id);
    } else {
        match list.iter_mut().find(|(e, _)| &**e == id) {
            Some((_, l)) => *l = level,
            None => list.push((Arc::from(id), level)),
        }
    }
}

fn levels_tag(levels: &Levels) -> Tag {
    Tag::compound(levels.iter().map(|(id, l)| (&**id, Tag::Int(*l as i32))))
}
fn int(t: &Tag) -> Option<i64> {
    match t {
        Tag::Byte(v) => Some(i64::from(*v)),
        Tag::Short(v) => Some(i64::from(*v)),
        Tag::Int(v) => Some(i64::from(*v)),
        Tag::Long(v) => Some(*v),
        _ => None,
    }
}
fn string(t: &Tag) -> Option<String> {
    match t {
        Tag::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}
fn read_levels(t: Option<&Tag>) -> Levels {
    let Some(Tag::Compound(map)) = t else {
        return Vec::new();
    };
    // Pre-1.21.5 worlds wrap the map in {levels: {...}}.
    let map = match map.get(&JavaString::from("levels")) {
        Some(Tag::Compound(inner)) => inner,
        _ => map,
    };
    // The keys are enchantment Identifiers: `knockback` is `minecraft:knockback`.
    map.iter()
        .filter_map(|(k, v)| {
            let id = horizon_world::datafix::dynamic::namespaced(&k.to_string_lossy());
            Some((Arc::from(id), int(v)?.clamp(1, 255) as u32))
        })
        .collect()
}
/// A text component as stored by vanilla: a plain string or `{"text": ...}`.
/// A `Filterable<String/Component>`: the value itself or `{raw: value}`.
/// `ComponentSerialization.CODEC`'s short form: a plain text component with
/// no style and no siblings is written as its string.
fn plain_component(t: &Tag) -> Tag {
    if let Tag::Compound(c) = t
        && let Some(text @ Tag::String(_)) = c.get(&"text".into())
        && c.iter().all(|(k, v)| {
            k.to_string_lossy() == "text"
                || (k.to_string_lossy() == "type"
                    && matches!(v, Tag::String(s) if s.to_string_lossy() == "text"))
        })
    {
        return text.clone();
    }
    t.clone()
}
fn filterable_text(t: &Tag) -> Option<String> {
    match t.get("raw") {
        Some(raw) => read_text(raw),
        None => read_text(t),
    }
}
fn read_text(t: &Tag) -> Option<String> {
    match t {
        Tag::String(s) => Some(s.to_string_lossy()),
        // A component's root `text` may be empty with the visible words in
        // `extra` (every legacy display name ViaVersion produces looks like
        // this), so flatten the whole component instead of reading one key.
        // ComponentSerialization.CODEC also takes a list (its first element
        // styling the rest), as servers behind ViaVersion send book pages.
        Tag::Compound(_) | Tag::List(_) => {
            Some(horizon_client_core::component::name_text(t).plain())
        }
        _ => None,
    }
}

/// A `FireworkExplosion` as vanilla's codec writes it (defaults omitted).
fn explosion_tag(e: &crate::firework::FireworkExplosion) -> Tag {
    let colors = |c: &[u32]| Tag::List(c.iter().map(|c| Tag::Int(*c as i32)).collect());
    let mut fields = vec![("shape", Tag::String(e.shape.name().into()))];
    if !e.colors.is_empty() {
        fields.push(("colors", colors(&e.colors)));
    }
    if !e.fade.is_empty() {
        fields.push(("fade_colors", colors(&e.fade)));
    }
    if e.trail {
        fields.push(("has_trail", Tag::Byte(1)));
    }
    if e.twinkle {
        fields.push(("has_twinkle", Tag::Byte(1)));
    }
    Tag::compound(fields)
}
fn explosion_from(e: &Tag) -> crate::firework::FireworkExplosion {
    let ints = |t: Option<&Tag>| match t {
        Some(Tag::IntArray(v)) => v.iter().map(|c| *c as u32).collect(),
        Some(Tag::List(v)) => v.iter().filter_map(int).map(|c| c as u32).collect(),
        _ => Vec::new(),
    };
    crate::firework::FireworkExplosion {
        shape: crate::firework::FireworkShape::from_name(
            e.get("shape")
                .and_then(string)
                .as_deref()
                .map_or("small_ball", |s| s.strip_prefix("minecraft:").unwrap_or(s)),
        ),
        colors: ints(e.get("colors")),
        fade: ints(e.get("fade_colors")),
        trail: e.get("has_trail").and_then(int).unwrap_or(0) != 0,
        twinkle: e.get("has_twinkle").and_then(int).unwrap_or(0) != 0,
    }
}
/// An `ItemStackTemplate`: `{id, count?, components?}`.
fn template_tag(stack: &ItemStack) -> Tag {
    let mut entries = vec![("id", Tag::String((&*stack.item).into()))];
    if stack.count != 1 {
        entries.push(("count", Tag::Int(stack.count as i32)));
    }
    if let Some(components) = encode_components(stack) {
        entries.push(("components", components));
    }
    Tag::compound(entries)
}
/// The pot prototype's decorations: a brick on every side.
pub fn all_bricks(sides: &[Option<ItemStack>; 4]) -> bool {
    sides.iter().all(|s| {
        s.as_ref()
            .is_some_and(|s| &*s.item == "minecraft:brick" && s.count == 1 && s.extra.is_none())
    })
}
fn template_from(tag: &Tag) -> Option<ItemStack> {
    match tag {
        Tag::String(id) => Some(ItemStack::new(&id.to_string_lossy(), 1)),
        _ => {
            let id = tag.get("id").and_then(string)?;
            let (damage, extra) = decode_components(tag.get("components"));
            Some(ItemStack {
                item: Arc::from(id.as_str()),
                count: tag.get("count").and_then(int).unwrap_or(1).max(1) as u32,
                damage,
                extra,
            })
        }
    }
}
/// The stack a `show_item` hover event holds (`HoverEvent.ShowItem`): its
/// `{id, count?, components?}` object.
pub fn stack_from_hover(event: &serde_json::Value) -> Option<ItemStack> {
    template_from(&horizon_client_core::scoreboard::json_to_tag(event))
}
/// Sorted-key, type-annotated text of a tag (the differential harnesses
/// print the same form from the vanilla side).
pub fn canon_tag(tag: &Tag) -> String {
    fn list(out: &mut String, prefix: &str, items: impl Iterator<Item = String>) {
        out.push_str(prefix);
        out.push_str(&items.collect::<Vec<_>>().join(","));
        out.push(']');
    }
    fn go(tag: &Tag, out: &mut String) {
        use std::fmt::Write as _;
        match tag {
            Tag::End => {}
            Tag::Byte(v) => write!(out, "{v}b").unwrap(),
            Tag::Short(v) => write!(out, "{v}s").unwrap(),
            Tag::Int(v) => write!(out, "{v}").unwrap(),
            Tag::Long(v) => write!(out, "{v}L").unwrap(),
            Tag::Float(v) => write!(out, "{}f", f32::from_bits(*v)).unwrap(),
            Tag::Double(v) => write!(out, "{}d", f64::from_bits(*v)).unwrap(),
            Tag::String(s) => write!(
                out,
                "\"{}\"",
                s.to_string_lossy()
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
            )
            .unwrap(),
            Tag::ByteArray(v) => list(out, "[B;", v.iter().map(|b| (*b as i8).to_string())),
            Tag::IntArray(v) => list(out, "[I;", v.iter().map(i32::to_string)),
            Tag::LongArray(v) => list(out, "[L;", v.iter().map(i64::to_string)),
            Tag::List(items) => {
                out.push('[');
                for (i, t) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    go(t, out);
                }
                out.push(']');
            }
            Tag::Compound(map) => {
                let mut keys: Vec<(String, &Tag)> =
                    map.iter().map(|(k, v)| (k.to_string_lossy(), v)).collect();
                keys.sort_by(|a, b| a.0.cmp(&b.0));
                out.push('{');
                for (i, (k, v)) in keys.into_iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&k);
                    out.push(':');
                    go(v, out);
                }
                out.push('}');
            }
        }
    }
    let mut out = String::new();
    go(tag, &mut out);
    out
}
/// `canon_tag` of a stack as `ItemStack.CODEC` writes it; "-" for none.
pub fn canon_stack(stack: Option<&ItemStack>) -> String {
    stack.map_or_else(
        || "-".to_owned(),
        |s| canon_tag(&crate::item_nbt::item_to_nbt(s)),
    )
}
/// The stack of an SNBT `ItemStack.CODEC` compound; None for "-".
pub fn parse_stack(snbt: &str) -> Result<Option<ItemStack>, String> {
    if snbt == "-" {
        return Ok(None);
    }
    let (tag, _) = horizon_world::nbt::parse_snbt(snbt)?;
    Ok(crate::item_nbt::item_from_nbt(&tag))
}
/// The `components` compound for a stack, or None when every component is default.
pub fn encode_components(stack: &ItemStack) -> Option<Tag> {
    components_tag(stack.damage, stack.data())
}
/// `encode_components` from the damage value and extra components.
pub fn components_tag(damage: u32, data: Option<&StackData>) -> Option<Tag> {
    let mut entries: Vec<(&str, Tag)> = Vec::new();
    if damage > 0 {
        entries.push(("minecraft:damage", Tag::Int(damage as i32)));
    }
    if let Some(d) = data {
        if !d.enchantments.is_empty() {
            entries.push(("minecraft:enchantments", levels_tag(&d.enchantments)));
        }
        if !d.stored_enchantments.is_empty() {
            entries.push((
                "minecraft:stored_enchantments",
                levels_tag(&d.stored_enchantments),
            ));
        }
        if d.repair_cost > 0 {
            entries.push(("minecraft:repair_cost", Tag::Int(d.repair_cost as i32)));
        }
        if let Some(name) = &d.custom_name {
            entries.push(("minecraft:custom_name", Tag::String(name.as_str().into())));
        }
        if let Some(potion) = &d.potion {
            entries.push((
                "minecraft:potion_contents",
                Tag::compound([("potion", Tag::String((&**potion).into()))]),
            ));
        }
        if let Some((material, pattern)) = &d.trim {
            entries.push((
                "minecraft:trim",
                Tag::compound([
                    ("material", Tag::String((&**material).into())),
                    ("pattern", Tag::String((&**pattern).into())),
                ]),
            ));
        }
        if let Some(id) = d.map_id {
            entries.push(("minecraft:map_id", Tag::Int(id)));
        }
        if !d.banner_patterns.is_empty() {
            entries.push((
                "minecraft:banner_patterns",
                crate::banner_layer::layers_to_nbt(&d.banner_patterns),
            ));
        }
        if let Some(color) = &d.base_color {
            entries.push(("minecraft:base_color", Tag::String((&**color).into())));
        }
        // Filterable values are saved as {raw: ..}.
        let raw = |t: Tag| Tag::compound([("raw", t)]);
        if !d.writable_pages.is_empty() {
            let pages = d
                .writable_pages
                .iter()
                .map(|p| raw(Tag::String(p.as_str().into())))
                .collect();
            entries.push((
                "minecraft:writable_book_content",
                Tag::compound([("pages", Tag::List(pages))]),
            ));
        }
        if let Some(w) = &d.written {
            let pages = if w.page_tags.len() == w.pages.len() {
                w.page_tags.iter().cloned().map(raw).collect()
            } else {
                w.pages
                    .iter()
                    .map(|p| raw(Tag::String(p.as_str().into())))
                    .collect()
            };
            entries.push((
                "minecraft:written_book_content",
                Tag::compound({
                    let mut fields = vec![
                        ("title", raw(Tag::String(w.title.as_str().into()))),
                        ("author", Tag::String(w.author.as_str().into())),
                    ];
                    // `optionalFieldOf("pages", List.of())`: no pages, no field.
                    if !w.pages.is_empty() {
                        fields.push(("pages", Tag::List(pages)));
                    }
                    if w.generation != 0 {
                        fields.push(("generation", Tag::Int(i32::from(w.generation))));
                    }
                    fields
                }),
            ));
        }
        if let Some((flight, explosions)) = &d.fireworks {
            let mut fields = Vec::new();
            if *flight != 0 {
                fields.push(("flight_duration", Tag::Byte(*flight as i8)));
            }
            if !explosions.is_empty() {
                fields.push((
                    "explosions",
                    Tag::List(explosions.iter().map(explosion_tag).collect()),
                ));
            }
            entries.push(("minecraft:fireworks", Tag::compound(fields)));
        }
        if let Some(e) = &d.firework_explosion {
            entries.push(("minecraft:firework_explosion", explosion_tag(e)));
        }
        if let Some(color) = d.dyed_color {
            entries.push(("minecraft:dyed_color", Tag::Int(color)));
        }
        if let Some(sides) = &d.pot_decorations {
            let mut sherds = Vec::new();
            for (name, side) in ["back", "left", "right", "front"].iter().zip(sides.iter()) {
                if let Some(stack) = side {
                    sherds.push((*name, template_tag(stack)));
                }
            }
            entries.push(("minecraft:pot_decorations", Tag::compound(sherds)));
        }
        if !d.bundle_contents.is_empty() {
            entries.push((
                "minecraft:bundle_contents",
                Tag::List(d.bundle_contents.iter().map(template_tag).collect()),
            ));
        }
        if !d.stew_effects.is_empty() {
            entries.push((
                "minecraft:suspicious_stew_effects",
                Tag::List(
                    d.stew_effects
                        .iter()
                        .map(|(id, duration)| {
                            // The codec leaves out the default duration (160).
                            let mut e = vec![("id", Tag::String((&**id).into()))];
                            if *duration != 160 {
                                e.push(("duration", Tag::Int(*duration)));
                            }
                            Tag::compound(e)
                        })
                        .collect(),
                ),
            ));
        }
        // minecraft:map_post_processing is transient: ItemStack.CODEC never
        // saves it (only the network carries it).
        if !d.charged.is_empty() {
            entries.push((
                "minecraft:charged_projectiles",
                Tag::List(d.charged.iter().map(crate::item_nbt::item_to_nbt).collect()),
            ));
        }
        if d.ominous_banner {
            let layer = |pattern: &str, color: &str| {
                Tag::compound([
                    (
                        "pattern",
                        Tag::String(format!("minecraft:{pattern}").as_str().into()),
                    ),
                    ("color", Tag::String(color.into())),
                ])
            };
            entries.push((
                "minecraft:banner_patterns",
                Tag::List(OMINOUS_LAYERS.iter().map(|(p, c)| layer(p, c)).collect()),
            ));
            if d.other("minecraft:tooltip_display").is_none() {
                entries.push(("minecraft:tooltip_display", ominous_tooltip_display()));
            }
            entries.push((
                "minecraft:item_name",
                Tag::compound([(
                    "translate",
                    Tag::String("block.minecraft.ominous_banner".into()),
                )]),
            ));
            entries.push(("minecraft:rarity", Tag::String("uncommon".into())));
        }
        if let Some(a) = d.ominous_amplifier {
            entries.push(("minecraft:ominous_bottle_amplifier", Tag::Int(i32::from(a))));
        }
        if !d.container.is_empty() {
            let slots = d
                .container
                .iter()
                .map(|(slot, stack)| {
                    Tag::compound([
                        ("slot", Tag::Int(i32::from(*slot))),
                        ("item", template_tag(stack)),
                    ])
                })
                .collect();
            entries.push(("minecraft:container", Tag::List(slots)));
        }
    }
    let mut out: horizon_world::nbt::Compound = entries
        .into_iter()
        .map(|(k, v)| (JavaString::from(k), v))
        .collect();
    if let Some(d) = data {
        for (k, v) in &d.other {
            out.entry(JavaString::from(k.as_str()))
                .or_insert_with(|| v.clone());
        }
    }
    (!out.is_empty()).then_some(Tag::Compound(out))
}
/// Damage and extra components from a stack's `components` compound.
pub fn decode_components(components: Option<&Tag>) -> (u32, Option<Box<StackData>>) {
    let Some(c) = components else {
        return (0, None);
    };
    let damage = c.get("minecraft:damage").and_then(int).unwrap_or(0).max(0) as u32;
    let potion = match c.get("minecraft:potion_contents") {
        Some(Tag::String(s)) => Some(Arc::from(s.to_string_lossy().as_str())),
        Some(t) => t
            .get("potion")
            .and_then(string)
            .map(|s| Arc::from(s.as_str())),
        None => None,
    };
    let data = StackData {
        enchantments: read_levels(c.get("minecraft:enchantments")),
        stored_enchantments: read_levels(c.get("minecraft:stored_enchantments")),
        repair_cost: c
            .get("minecraft:repair_cost")
            .and_then(int)
            .unwrap_or(0)
            .max(0) as u32,
        custom_name: c.get("minecraft:custom_name").and_then(read_text),
        potion,
        map_id: c.get("minecraft:map_id").and_then(int).map(|v| v as i32),
        banner_patterns: crate::banner_layer::layers_from_nbt(c.get("minecraft:banner_patterns")),
        base_color: c
            .get("minecraft:base_color")
            .and_then(string)
            .map(|s| Arc::from(s.as_str())),
        writable_pages: match c
            .get("minecraft:writable_book_content")
            .and_then(|b| b.get("pages"))
        {
            Some(Tag::List(pages)) => pages.iter().filter_map(filterable_text).collect(),
            _ => Vec::new(),
        },
        written: c
            .get("minecraft:written_book_content")
            .map(|b| WrittenBook {
                title: b.get("title").and_then(filterable_text).unwrap_or_default(),
                author: b.get("author").and_then(string).unwrap_or_default(),
                generation: b.get("generation").and_then(int).unwrap_or(0).clamp(0, 3) as u8,
                pages: match b.get("pages") {
                    Some(Tag::List(pages)) => pages.iter().filter_map(filterable_text).collect(),
                    _ => Vec::new(),
                },
                page_tags: match b.get("pages") {
                    Some(Tag::List(pages)) => pages
                        .iter()
                        .map(|p| plain_component(p.get("raw").unwrap_or(p)))
                        .collect(),
                    _ => Vec::new(),
                },
            }),
        fireworks: c
            .get("minecraft:fireworks")
            .map(|f| {
                let flight = f
                    .get("flight_duration")
                    .and_then(int)
                    .unwrap_or(0)
                    .clamp(0, 255) as u8;
                let explosions = match f.get("explosions") {
                    Some(Tag::List(list)) => list.iter().map(explosion_from).collect(),
                    _ => Vec::new(),
                };
                (flight, explosions)
            })
            .filter(|(flight, explosions)| !(*flight == 1 && explosions.is_empty())),
        firework_explosion: c.get("minecraft:firework_explosion").map(explosion_from),
        dyed_color: c
            .get("minecraft:dyed_color")
            .and_then(int)
            .map(|v| v as i32),
        pot_decorations: c
            .get("minecraft:pot_decorations")
            .map(|p| {
                let side = |name: &str| p.get(name).and_then(template_from);
                Box::new([side("back"), side("left"), side("right"), side("front")])
            })
            .filter(|sides| !all_bricks(sides)),
        bundle_contents: match c.get("minecraft:bundle_contents") {
            Some(Tag::List(list)) => list.iter().filter_map(template_from).collect(),
            _ => Vec::new(),
        },
        stew_effects: match c.get("minecraft:suspicious_stew_effects") {
            Some(Tag::List(list)) => list
                .iter()
                .filter_map(|e| {
                    Some((
                        Arc::from(e.get("id").and_then(string)?.as_str()),
                        e.get("duration").and_then(int).unwrap_or(160) as i32,
                    ))
                })
                .collect(),
            _ => Vec::new(),
        },
        map_post_processing: match c.get("minecraft:map_post_processing") {
            Some(Tag::String(s)) => Some(u8::from(s.to_string_lossy() != "lock")),
            Some(t) => int(t).map(|v| v.clamp(0, 1) as u8),
            None => None,
        },
        charged: match c.get("minecraft:charged_projectiles") {
            Some(Tag::List(list)) => list
                .iter()
                .filter_map(crate::item_nbt::item_from_nbt)
                .collect(),
            _ => Vec::new(),
        },
        container: match c.get("minecraft:container") {
            Some(Tag::List(list)) => list
                .iter()
                .filter_map(|e| {
                    Some((
                        e.get("slot").and_then(int)?.clamp(0, 255) as u8,
                        template_from(e.get("item")?)?,
                    ))
                })
                .collect(),
            _ => Vec::new(),
        },
        ominous_banner: c
            .get("minecraft:item_name")
            .and_then(|n| n.get("translate"))
            .and_then(string)
            .as_deref()
            == Some("block.minecraft.ominous_banner")
            && matches!(
                c.get("minecraft:banner_patterns"),
                Some(Tag::List(layers)) if layers.len() == OMINOUS_LAYERS.len()),
        ominous_amplifier: c
            .get("minecraft:ominous_bottle_amplifier")
            .and_then(int)
            .map(|v| v.clamp(0, 4) as u8),
        trim: c.get("minecraft:trim").and_then(|t| {
            Some((
                Arc::from(t.get("material").and_then(string)?.as_str()),
                Arc::from(t.get("pattern").and_then(string)?.as_str()),
            ))
        }),
        bundle_selected: BundleSelected::default(),
        other: Vec::new(),
        wire: Wire::default(),
    };
    let mut data = data;
    if let Tag::Compound(map) = c {
        for (k, v) in map {
            let key = k.to_string_lossy();
            let ominous_part = data.ominous_banner
                && (matches!(key.as_str(), "minecraft:item_name" | "minecraft:rarity")
                    || (key == "minecraft:tooltip_display" && v == &ominous_tooltip_display()));
            if !MODELLED.contains(&key.as_str()) && !ominous_part {
                data.other.push((key, v.clone()));
            }
        }
        data.other.sort_by(|a, b| a.0.cmp(&b.0));
    }
    (damage, (!data.is_empty()).then(|| Box::new(data)))
}

/// The `[id=value,...]` component list of an item argument (`ItemParser`)
/// as a `components` compound; `!id` removals are skipped.
pub fn command_components(text: &str) -> Result<Tag, String> {
    let inner = text
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .ok_or("expected '['")?;
    let mut entries = horizon_world::nbt::Compound::new();
    let mut rest = inner.trim_start();
    while !rest.is_empty() {
        let end = rest.find(['=', ',']).unwrap_or(rest.len());
        let key = rest[..end].trim();
        rest = &rest[end..];
        if let Some(value) = rest.strip_prefix('=') {
            let (tag, used) = horizon_world::nbt::parse_snbt(value)?;
            let id = if key.contains(':') {
                key.to_owned()
            } else {
                format!("minecraft:{key}")
            };
            entries.insert(JavaString::from(id.as_str()), tag);
            rest = &value[used..];
        }
        rest = rest.trim_start();
        rest = rest.strip_prefix(',').unwrap_or(rest).trim_start();
    }
    Ok(Tag::Compound(entries))
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_round_trip() {
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        stack.damage = 12;
        stack.enchant("minecraft:sharpness", 3);
        stack.enchant("minecraft:unbreaking", 2);
        stack.data_mut().repair_cost = 3;
        stack.data_mut().custom_name = Some("Blade".into());
        let tag = encode_components(&stack).expect("components");
        let (damage, extra) = decode_components(Some(&tag));
        assert_eq!(damage, 12);
        let back = ItemStack {
            damage,
            extra,
            ..ItemStack::new("minecraft:diamond_sword", 1)
        };
        assert_eq!(back.enchantment_level("minecraft:sharpness"), 3);
        assert_eq!(back.enchantment_level("minecraft:unbreaking"), 2);
        assert_eq!(back.repair_cost(), 3);
        assert_eq!(back.custom_name(), Some("Blade"));
    }

    #[test]
    fn enchantment_ids_without_a_namespace_are_minecraft_ones() {
        let levels = Tag::compound([
            ("knockback", Tag::Int(2)),
            ("minecraft:sharpness", Tag::Int(1)),
        ]);
        let components = Tag::compound([("minecraft:enchantments", levels)]);
        let (_, extra) = decode_components(Some(&components));
        let stack = ItemStack {
            extra,
            ..ItemStack::new("minecraft:wooden_sword", 1)
        };
        assert_eq!(stack.enchantment_level("minecraft:knockback"), 2);
        assert_eq!(stack.enchantment_level("minecraft:sharpness"), 1);
    }

    #[test]
    fn custom_name_reads_the_extra_spans() {
        let name = Tag::compound([
            (
                "extra",
                Tag::List(vec![
                    Tag::compound([("text", Tag::String("Games ".into()))]),
                    Tag::compound([("text", Tag::String("Right click".into()))]),
                ]),
            ),
            ("text", Tag::String("".into())),
        ]);
        let components = Tag::compound([("minecraft:custom_name", name)]);
        let (_, extra) = decode_components(Some(&components));
        assert_eq!(
            extra.and_then(|d| d.custom_name.clone()).as_deref(),
            Some("Games Right click")
        );
    }

    #[test]
    fn written_book_pages_may_be_component_lists() {
        // A page as Hypixel sends it through ViaVersion: a list, not a compound.
        let page = Tag::List(vec![
            Tag::compound([("text", Tag::String("".into()))]),
            Tag::compound([("text", Tag::String("Please read ".into()))]),
            Tag::compound([("text", Tag::String("the rules".into()))]),
        ]);
        let book = Tag::compound([
            (
                "title",
                Tag::compound([("raw", Tag::String("Rules".into()))]),
            ),
            ("author", Tag::String("Server".into())),
            (
                "pages",
                Tag::List(vec![
                    page,
                    Tag::compound([("raw", Tag::String("Two".into()))]),
                ]),
            ),
        ]);
        let components = Tag::compound([("minecraft:written_book_content", book)]);
        let (_, extra) = decode_components(Some(&components));
        let pages = extra.and_then(|d| d.written.clone()).map(|w| w.pages);
        assert_eq!(
            pages,
            Some(vec!["Please read the rules".to_owned(), "Two".to_owned()])
        );
    }

    #[test]
    fn banner_layers_and_shield_base_round_trip() {
        let mut stack = ItemStack::new("minecraft:shield", 1);
        stack.data_mut().base_color = Some(Arc::from("red"));
        stack.data_mut().banner_patterns = vec![
            crate::banner_layer::BannerLayer::new("minecraft:flower", "black"),
            crate::banner_layer::BannerLayer::new("minecraft:border", "white"),
        ];
        let tag = encode_components(&stack).expect("components");
        let (_, extra) = decode_components(Some(&tag));
        let extra = extra.expect("data");
        assert_eq!(extra.base_color.as_deref(), Some("red"));
        assert_eq!(extra.banner_patterns, stack.data().unwrap().banner_patterns);
    }

    #[test]
    fn ominous_items_round_trip() {
        let mut banner = ItemStack::new("minecraft:white_banner", 1);
        banner.data_mut().ominous_banner = true;
        let tag = encode_components(&banner).unwrap();
        assert!(tag.get("minecraft:banner_patterns").is_some());
        let (_, back) = decode_components(Some(&tag));
        let back = back.unwrap();
        assert!(back.ominous_banner);
        assert!(back.other("minecraft:tooltip_display").is_none());

        banner.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(Tag::Compound(Default::default())),
        );
        let (_, custom_display) = decode_components(encode_components(&banner).as_ref());
        assert_eq!(
            custom_display.unwrap().other("minecraft:tooltip_display"),
            Some(&Tag::Compound(Default::default()))
        );
        let mut bottle = ItemStack::new("minecraft:ominous_bottle", 1);
        bottle.data_mut().ominous_amplifier = Some(3);
        let (_, back) = decode_components(encode_components(&bottle).as_ref());
        assert_eq!(back.unwrap().ominous_amplifier, Some(3));
    }
    #[test]
    fn plain_stacks_have_no_components() {
        let stack = ItemStack::new("minecraft:stone", 64);
        assert!(encode_components(&stack).is_none());
        let mut named = stack.clone();
        named.data_mut();
        named.tidy();
        assert_eq!(named, stack);
    }

    #[test]
    fn books_store_enchantments() {
        let mut book = ItemStack::new("minecraft:enchanted_book", 1);
        book.enchant("minecraft:mending", 1);
        assert!(book.enchantments().is_empty());
        assert_eq!(book.stored_enchantments().len(), 1);
        upgrade(
            &mut book.data_mut().stored_enchantments,
            "minecraft:mending",
            1,
        );
        assert_eq!(book.crafting_enchantments()[0].1, 1);
        let book = ItemStack::new("minecraft:book", 1).transmute("minecraft:enchanted_book", 1);
        assert!(book.extra.is_none());
    }
    #[test]
    fn command_components_decode_fireworks() {
        let tag = command_components(
            "[fireworks={flight_duration:2,explosions:[{shape:large_ball,colors:[I;16711680],has_twinkle:true}]},damage=3]",
        )
        .unwrap();
        let (damage, data) = decode_components(Some(&tag));
        assert_eq!(damage, 3);
        let (flight, explosions) = data.unwrap().fireworks.unwrap();
        assert_eq!(flight, 2);
        assert_eq!(
            explosions[0].shape,
            crate::firework::FireworkShape::LargeBall
        );
        assert_eq!(explosions[0].colors, vec![0xFF0000]);
        assert!(explosions[0].twinkle && !explosions[0].trail);
    }
}
