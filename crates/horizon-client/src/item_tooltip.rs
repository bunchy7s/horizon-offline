//! Item tooltip formatting: equipment modifiers with a player's base values,
//! and lines without player context for the recipe book's search tree.
use crate::text::{GRAY, Text};
use crate::{
    hud::rgba,
    items::{ItemInfo, ItemStack},
};
use horizon_world::nbt::Tag;

/// The `minecraft:tooltip_display` component applied by `ItemStack` while it
/// builds a hover tooltip.
pub struct TooltipDisplay<'a> {
    component: Option<&'a Tag>,
    ominous_banner: bool,
}

impl<'a> TooltipDisplay<'a> {
    pub fn for_stack(stack: &'a ItemStack) -> Self {
        Self {
            component: stack
                .data()
                .and_then(|data| data.other("minecraft:tooltip_display")),
            ominous_banner: stack.data().is_some_and(|data| data.ominous_banner),
        }
    }

    fn hides_tooltip(&self) -> bool {
        matches!(
            self.component.and_then(|display| display.get("hide_tooltip")),
            Some(Tag::Byte(value)) if *value != 0
        )
    }

    /// `ItemStack.getTooltipLines`: `hide_tooltip` suppresses the name unless
    /// the tooltip flag is creative.
    pub fn name_visible(&self, creative: bool) -> bool {
        creative || !self.hides_tooltip()
    }

    /// `TooltipDisplay.shows`: whether details for a data component are shown.
    pub fn shows(&self, component: &str) -> bool {
        if self.hides_tooltip() {
            return false;
        }
        // `item_data::decode_components` models the raid captain banner's
        // default hidden pattern tooltip as `ominous_banner` instead of
        // retaining its redundant `tooltip_display` component.
        if self.ominous_banner
            && self.component.is_none()
            && component == "minecraft:banner_patterns"
        {
            return false;
        }
        let Some(Tag::List(hidden)) = self
            .component
            .and_then(|display| display.get("hidden_components"))
        else {
            return true;
        };
        !hidden.iter().any(|tag| {
            matches!(tag, Tag::String(id) if id.0.iter().copied().eq(component.encode_utf16()))
        })
    }
}

/// `EquipmentSlotGroup` in declaration order, the order tooltips list them in.
const SLOT_GROUPS: [&str; 11] = [
    "any", "mainhand", "offhand", "hand", "feet", "legs", "chest", "head", "armor", "body",
    "saddle",
];

/// `ItemAttributeModifiers.ATTRIBUTE_MODIFIER_FORMAT`, `DecimalFormat("#.##")`
/// of a positive amount (half-even on the exact value).
fn format_amount(value: f64) -> String {
    let text = format!("{:.2}", value);
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// `ItemAttributeModifiers.Display.Default.apply`: a modifier's line, none
/// for a zero amount. `percent` is the modifier's operation id (0: added
/// value, 1 and 2: multiplied).
fn modifier_line(
    translate: &dyn Fn(&str) -> Option<String>,
    attribute: &str,
    amount: f64,
    operation: usize,
    total: bool,
) -> Option<String> {
    let text = |key: &str| translate(key).unwrap_or_else(|| key.to_owned());
    let scale = match operation {
        0 if attribute == "knockback_resistance" => 10.0,
        0 => 1.0,
        _ => 100.0,
    };
    let (kind, shown) = if total {
        ("equals", amount * scale)
    } else if amount > 0.0 {
        ("plus", amount * scale)
    } else if amount < 0.0 {
        ("take", -amount * scale)
    } else {
        return None;
    };
    // TranslatableContents: `%s` and numbered `%1$s` arguments, `%%` a percent sign.
    Some(
        crate::text::format_translation(
            &text(&format!("attribute.modifier.{kind}.{operation}")),
            &[
                crate::text::Text::literal(format_amount(shown)),
                crate::text::Text::literal(text(&format!("attribute.name.{attribute}"))),
            ],
        )
        .plain(),
    )
}

/// `Attribute.getStyle`: the colour of a line for an attribute whose value
/// rises or falls. Gravity, scale and the waypoint ranges are neutral, burning
/// time and fall damage are negative, every other attribute is positive.
fn attribute_color(attribute: &str, rises: bool) -> u32 {
    let (blue, red) = (rgba(85, 85, 255, 255), rgba(255, 85, 85, 255));
    match attribute {
        "burning_time" | "fall_damage_multiplier" => {
            if rises {
                red
            } else {
                blue
            }
        }
        "gravity" | "scale" | "waypoint_transmit_range" | "waypoint_receive_range" => {
            rgba(170, 170, 170, 255)
        }
        _ => {
            if rises {
                blue
            } else {
                red
            }
        }
    }
}

/// `ItemStack.addAttributeTooltips`: slot groups, colors and optional player
/// attack damage/speed bases. Recipe search has no player context.
pub fn attribute_lines(
    modifiers: &[(String, String, f64, String, String)],
    player_base: Option<[f64; 2]>,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<(String, u32)> {
    let mut lines = Vec::new();
    for group in SLOT_GROUPS {
        let mut first = true;
        for (attribute, modifier, amount, operation, slot) in modifiers {
            if slot != group {
                continue;
            }
            if std::mem::take(&mut first) {
                lines.push((String::new(), u32::MAX));
                lines.push((
                    translate(&format!("item.modifiers.{group}"))
                        .unwrap_or_else(|| format!("item.modifiers.{group}")),
                    rgba(170, 170, 170, 255),
                ));
            }
            let id = match operation.as_str() {
                "add_multiplied_base" => 1,
                "add_multiplied_total" => 2,
                _ => 0,
            };
            let path = attribute.strip_prefix("minecraft:").unwrap_or(attribute);
            let base = player_base.and_then(|[damage, speed]| match modifier.as_str() {
                "minecraft:base_attack_damage" => Some(damage),
                "minecraft:base_attack_speed" => Some(speed),
                _ => None,
            });
            if let Some(line) = modifier_line(
                translate,
                path,
                amount + base.unwrap_or(0.0),
                id,
                base.is_some(),
            ) {
                let (line, color) = if base.is_some() {
                    (format!(" {line}"), rgba(0, 170, 0, 255))
                } else {
                    (line, attribute_color(path, *amount > 0.0))
                };
                lines.push((line, color));
            }
        }
    }
    lines
}

/// The attribute modifiers a mob effect adds per amplifier level
/// (`MobEffects`): attribute, amount, operation id.
fn effect_modifier(effect: &str) -> Option<(&'static str, f64, usize)> {
    Some(match effect {
        "speed" => ("movement_speed", 0.2, 2),
        "slowness" => ("movement_speed", -0.15, 2),
        "strength" => ("attack_damage", 3.0, 0),
        "weakness" => ("attack_damage", -4.0, 0),
        "jump_boost" => ("safe_fall_distance", 1.0, 0),
        "luck" => ("luck", 1.0, 0),
        "invisibility" => ("waypoint_transmit_range", -1.0, 2),
        _ => return None,
    })
}

fn is_potion_item(item: &str) -> bool {
    item.ends_with("potion") || item == "minecraft:tipped_arrow"
}

/// `PlayerHeadItem.getName`: a player head whose profile has a name is titled
/// after it.
pub struct PlayerHeadName(String);

impl PlayerHeadName {
    /// The title as `getName`'s `%s's Head` translation.
    pub fn title(&self, translate: &dyn Fn(&str) -> Option<String>) -> String {
        translate("block.minecraft.player_head.named")
            .unwrap_or_else(|| "%s's Head".to_owned())
            .replacen("%s", &self.0, 1)
    }
    /// The title as a component, for command feedback.
    pub fn text(&self) -> crate::text::Text {
        crate::text::Text::translate(
            "block.minecraft.player_head.named",
            &[crate::text::Text::literal(self.0.clone())],
        )
    }
}

/// The name in a player head's `minecraft:profile`, if it has one.
pub fn player_head_name(stack: &ItemStack) -> Option<PlayerHeadName> {
    if &*stack.item != "minecraft:player_head" {
        return None;
    }
    Some(PlayerHeadName(stack.profile_name()?))
}

/// The name line of `stack`, as the item's translation or, for potions and
/// smithing templates, its name component gives it.
pub fn name_line(stack: &ItemStack, translate: &dyn Fn(&str) -> Option<String>) -> String {
    let (namespace, path) = stack
        .item
        .split_once(':')
        .unwrap_or(("minecraft", &stack.item));
    if let Some(name) = stack.custom_name() {
        return name.to_owned();
    }
    if let Some(name) = stack.item_name() {
        return name;
    }
    if stack.data().is_some_and(|d| d.ominous_banner) {
        return translate("block.minecraft.ominous_banner")
            .unwrap_or_else(|| "Ominous Banner".to_owned());
    }
    if let Some(head) = player_head_name(stack) {
        return head.title(translate);
    }
    let key = if is_potion_item(&stack.item) {
        Some(crate::brewing::potion_name_key(&stack.item, stack.potion()))
    } else if let Some(pattern) = path.strip_suffix("_armor_trim_smithing_template") {
        Some(format!("trim_pattern.minecraft.{pattern}"))
    } else if path == "netherite_upgrade_smithing_template" {
        Some("upgrade.minecraft.netherite_upgrade".to_owned())
    } else if path.ends_with("_banner_pattern") {
        // The item name component of the pattern items that have one.
        Some(format!("item.minecraft.{path}.new"))
    } else if &*stack.item == "minecraft:shield"
        && let Some(color) = stack.data().and_then(|d| d.base_color.as_deref())
    {
        // `ShieldItem.getName`: the banner's base colour names the shield.
        Some(format!("item.minecraft.shield.{color}"))
    } else {
        None
    };
    // `Item.getDescriptionId`: `Util.makeDescriptionId("item", id)`, or the
    // block's for a block item, in the item's own namespace.
    let id = format!("{namespace}.{}", path.replace('/', "."));
    key.and_then(|k| translate(&k))
        .or_else(|| translate(&format!("item.{id}")))
        .or_else(|| translate(&format!("block.{id}")))
        .unwrap_or_else(|| path.replace('_', " "))
}

/// ItemStack.getRarity: the ominous banner override, then the enchantment upgrade.
pub fn rarity<'a>(stack: &ItemStack, default: &'a str) -> &'a str {
    let rarity = if stack.data().is_some_and(|d| d.ominous_banner) {
        "uncommon"
    } else {
        default
    };
    if stack.enchantments().is_empty() {
        rarity
    } else {
        match rarity {
            "common" | "uncommon" => "rare",
            "rare" => "epic",
            r => r,
        }
    }
}

fn is_beehive(path: &str) -> bool {
    matches!(path, "beehive" | "bee_nest")
}

/// `InstrumentComponent.addToTooltip`: the instrument's description. A goat
/// horn without the component has the item's default, the ponder horn.
pub fn instrument_line(
    stack: &ItemStack,
    display: &TooltipDisplay,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    if !display.shows("minecraft:instrument") {
        return None;
    }
    let id = match stack.data().and_then(|d| d.other("minecraft:instrument")) {
        Some(Tag::String(id)) => id.to_string_lossy(),
        // An inline instrument carries its own description.
        Some(inline) => {
            return inline
                .get("description")
                .map(|d| crate::entity_nbt::name_text(d).plain());
        }
        None if &*stack.item == "minecraft:goat_horn" => "minecraft:ponder_goat_horn".to_owned(),
        None => return None,
    };
    let key = format!("instrument.{}", id.replacen(':', ".", 1));
    Some(translate(&key).unwrap_or(key))
}

/// `TropicalFish.COMMON_VARIANTS`: pattern, base colour and pattern colour of
/// the 22 named fish (`entity.minecraft.tropical_fish.predefined.<index>`).
const COMMON_TROPICAL_FISH: [(&str, &str, &str); 22] = [
    ("stripey", "orange", "gray"),
    ("flopper", "gray", "gray"),
    ("flopper", "gray", "blue"),
    ("clayfish", "white", "gray"),
    ("sunstreak", "blue", "gray"),
    ("kob", "orange", "white"),
    ("spotty", "pink", "light_blue"),
    ("blockfish", "purple", "yellow"),
    ("clayfish", "white", "red"),
    ("spotty", "white", "yellow"),
    ("glitter", "white", "gray"),
    ("clayfish", "white", "orange"),
    ("dasher", "cyan", "pink"),
    ("brinely", "lime", "light_blue"),
    ("betty", "red", "white"),
    ("snooper", "gray", "red"),
    ("blockfish", "red", "white"),
    ("flopper", "white", "yellow"),
    ("kob", "red", "white"),
    ("sunstreak", "gray", "white"),
    ("dasher", "cyan", "yellow"),
    ("flopper", "yellow", "yellow"),
];

/// `TropicalFish.Pattern.addToTooltip`: a bucket of tropical fish names a
/// common variant, or else shows the pattern and then the base colour (with
/// the pattern colour when it differs), all in italic gray. Only a stack that
/// has the pattern component shows it.
pub fn tropical_fish_lines(stack: &ItemStack, display: &TooltipDisplay) -> Vec<Text> {
    if !display.shows("minecraft:tropical_fish/pattern") {
        return Vec::new();
    }
    let Some(data) = stack.data() else {
        return Vec::new();
    };
    let name = |key: &str, default: &'static str| match data.other(key) {
        Some(Tag::String(s)) => s
            .to_string_lossy()
            .trim_start_matches("minecraft:")
            .to_owned(),
        _ => default.to_owned(),
    };
    let Some(Tag::String(pattern)) = data.other("minecraft:tropical_fish/pattern") else {
        return Vec::new();
    };
    let pattern = pattern
        .to_string_lossy()
        .trim_start_matches("minecraft:")
        .to_owned();
    let base = name("minecraft:tropical_fish/base_color", "white");
    let color = name("minecraft:tropical_fish/pattern_color", "white");
    let styled = |t: Text| t.italic().with_color(GRAY);
    let common = COMMON_TROPICAL_FISH
        .iter()
        .position(|(p, b, c)| *p == pattern && *b == base && *c == color);
    if let Some(index) = common {
        let key = format!("entity.minecraft.tropical_fish.predefined.{index}");
        return vec![styled(Text::translate(&key, &[]))];
    }
    let mut colors = Text::translate(&format!("color.minecraft.{base}"), &[]);
    if base != color {
        colors = colors
            .append(", ")
            .append(Text::translate(&format!("color.minecraft.{color}"), &[]));
    }
    vec![
        styled(Text::translate(
            &format!("entity.minecraft.tropical_fish.type.{pattern}"),
            &[],
        )),
        styled(colors),
    ]
}

/// `MapId.addToTooltip`: a map whose data is not known (`map` is `None`, else
/// its `locked` flag and `scale`) reads "Unknown Map"; a known one gives its
/// id (unless it has a custom name or is to be locked or scaled), "Locked"
/// when it is or is being locked, and with advanced tooltips its scale (a
/// copy being scaled one up, at most 4) and level.
pub fn map_id_lines(
    stack: &ItemStack,
    display: &TooltipDisplay,
    advanced: bool,
    map: Option<(bool, u8)>,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    let Some(id) = stack.data().and_then(|d| d.map_id) else {
        return Vec::new();
    };
    if !display.shows("minecraft:map_id") {
        return Vec::new();
    }
    let text = |key: &str, fallback: &str| translate(key).unwrap_or_else(|| fallback.to_owned());
    let Some((locked, scale)) = map else {
        return vec![text("filled_map.unknown", "Unknown Map")];
    };
    // MapPostProcessing: 0 lock, 1 scale.
    let post = stack.data().and_then(|d| d.map_post_processing);
    let mut lines = Vec::new();
    if stack.custom_name().is_none() && post.is_none() {
        lines.push(text("filled_map.id", "ID #%s").replacen("%s", &id.to_string(), 1));
    }
    if locked || post == Some(0) {
        lines.push(text("filled_map.locked", "Locked"));
    }
    if advanced {
        let scale = (scale + u8::from(post == Some(1))).min(4);
        lines.push(text("filled_map.scale", "Scaling at 1:%s").replacen(
            "%s",
            &(1u32 << scale).to_string(),
            1,
        ));
        lines.push(
            text("filled_map.level", "(Level %s/%s)")
                .replacen("%s", &scale.to_string(), 1)
                .replacen("%s", "4", 1),
        );
    }
    lines
}

/// `Bees.addToTooltip`: how many bees a hive item holds (a hive item without
/// the component has the item's default, none).
pub fn bees_line(
    stack: &ItemStack,
    display: &TooltipDisplay,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    if !display.shows("minecraft:bees") {
        return None;
    }
    let bees = match stack.data().and_then(|d| d.other("minecraft:bees")) {
        Some(Tag::List(bees)) => bees.len(),
        Some(_) => return None,
        None if is_beehive(stack.item.trim_start_matches("minecraft:")) => 0,
        None => return None,
    };
    let text =
        translate("container.beehive.bees").unwrap_or_else(|| "container.beehive.bees".into());
    Some(
        text.replacen("%s", &bees.to_string(), 1)
            .replacen("%s", "3", 1),
    )
}

/// `TypedEntityData.addToTooltip` for a spawn egg: in Peaceful, the red
/// "Disabled in Peaceful" under an egg whose entity type is not allowed there
/// (`EntityType.isAllowedInPeaceful`). The type is the one in the egg's
/// `entity_data` component, else the egg's own (`zombie_spawn_egg`).
/// `allowed_in_peaceful` answers for an entity type id; one it does not know
/// is allowed.
pub fn spawn_egg_peaceful_line(
    stack: &ItemStack,
    peaceful: bool,
    display: &TooltipDisplay,
    allowed_in_peaceful: &dyn Fn(&str) -> bool,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    if !peaceful || !display.shows("minecraft:entity_data") {
        return None;
    }
    // `TypedEntityData.addToTooltip` hangs on the `entity_data` component, whatever the item;
    // a spawn egg without the component has its own mob as the default.
    let id = match stack
        .data()
        .and_then(|d| d.other("minecraft:entity_data"))
        .and_then(|data| data.get("id"))
    {
        Some(Tag::String(id)) => {
            let id = id.to_string_lossy();
            if id.contains(':') {
                id
            } else {
                format!("minecraft:{id}")
            }
        }
        _ if stack.item.ends_with("_spawn_egg") => format!(
            "minecraft:{}",
            stack
                .item
                .trim_start_matches("minecraft:")
                .trim_end_matches("_spawn_egg")
        ),
        _ => return None,
    };
    (!allowed_in_peaceful(&id)).then(|| {
        translate("item.spawn_egg.peaceful").unwrap_or_else(|| "item.spawn_egg.peaceful".into())
    })
}

/// `BlockItemStateProperties.addToTooltip`: a hive item's honey level (a hive
/// item without the component has the item's default, 0).
pub fn honey_line(
    stack: &ItemStack,
    display: &TooltipDisplay,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    if !display.shows("minecraft:block_state") {
        return None;
    }
    let level = match stack.data().and_then(|d| d.other("minecraft:block_state")) {
        Some(Tag::Compound(state)) => {
            match state.get(&horizon_world::nbt::JavaString::from("honey_level")) {
                Some(Tag::String(level)) => level.to_string_lossy().parse::<u32>().ok()?,
                _ => return None,
            }
        }
        Some(_) => return None,
        None if is_beehive(stack.item.trim_start_matches("minecraft:")) => 0,
        None => return None,
    };
    let text =
        translate("container.beehive.honey").unwrap_or_else(|| "container.beehive.honey".into());
    Some(
        text.replacen("%s", &level.to_string(), 1)
            .replacen("%s", "5", 1),
    )
}

/// `WrittenBookContent.addToTooltip`: "by <author>" and the generation.
pub fn written_book_lines(
    stack: &ItemStack,
    display: &TooltipDisplay,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    let text = |key: &str| translate(key).unwrap_or_else(|| key.to_owned());
    let Some(book) = stack
        .data()
        .and_then(|d| d.written.as_ref())
        .filter(|_| display.shows("minecraft:written_book_content"))
    else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    if !book.author.trim().is_empty() {
        lines.push(
            crate::text::format_translation(
                &text("book.byAuthor"),
                &[crate::text::Text::literal(&book.author)],
            )
            .plain(),
        );
    }
    lines.push(text(&format!("book.generation.{}", book.generation)));
    lines
}

/// `JukeboxPlayable.addToTooltip`: the song's description.
pub fn jukebox_line(
    stack: &ItemStack,
    info: Option<&ItemInfo>,
    display: &TooltipDisplay,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    if !display.shows("minecraft:jukebox_playable") {
        return None;
    }
    let song = match stack
        .data()
        .and_then(|d| d.other("minecraft:jukebox_playable"))
    {
        Some(Tag::String(song)) => song.to_string_lossy(),
        // An inline song carries its own description.
        Some(inline) => {
            return inline
                .get("description")
                .map(|d| crate::entity_nbt::name_text(d).plain());
        }
        None => info?.jukebox_song.clone()?,
    };
    let key = format!("jukebox_song.{}", song.replacen(':', ".", 1));
    Some(translate(&key).unwrap_or(key))
}

/// `DyedItemColor.addToTooltip`: "Dyed" (italic, the flag), or with advanced
/// tooltips the colour's hex code.
pub fn dyed_line(
    stack: &ItemStack,
    display: &TooltipDisplay,
    advanced: bool,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Option<(String, bool)> {
    let rgb = stack.data()?.dyed_color?;
    if !display.shows("minecraft:dyed_color") {
        return None;
    }
    let text = |key: &str| translate(key).unwrap_or_else(|| key.to_owned());
    Some(if advanced {
        let hex = format!("#{:06X}", rgb & 0xFF_FFFF);
        (text("item.color").replacen("%s", &hex, 1), false)
    } else {
        (text("item.dyed"), true)
    })
}

/// `ItemStack.UNBREAKABLE_TOOLTIP`.
pub fn unbreakable_line(
    stack: &ItemStack,
    display: &TooltipDisplay,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    (stack.data()?.other("minecraft:unbreakable").is_some()
        && display.shows("minecraft:unbreakable"))
    .then(|| translate("item.unbreakable").unwrap_or_else(|| "item.unbreakable".into()))
}

/// `OminousBottleAmplifier.addToTooltip`: the effect the bottle gives,
/// `MobEffectInstance(BAD_OMEN, 120000, amplifier)`, listed as
/// `PotionContents.addPotionTooltip` lists it, in the colour of Bad Omen's
/// category (NEUTRAL, blue). An ominous bottle without the component has the
/// item's default amplifier, 0.
pub fn ominous_bottle_line(
    stack: &ItemStack,
    display: &TooltipDisplay,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Option<(String, u32)> {
    const BAD_OMEN_TICKS: i32 = 120_000;
    let amplifier = stack
        .data()
        .and_then(|d| d.ominous_amplifier)
        .or_else(|| (&*stack.item == "minecraft:ominous_bottle").then_some(0))?;
    if !display.shows("minecraft:ominous_bottle_amplifier") {
        return None;
    }
    use crate::text::{Text, format_translation};
    let text = |key: &str, fallback: &str| translate(key).unwrap_or_else(|| fallback.to_owned());
    // PotionContents.getPotionDescription.
    let mut line = Text::literal(text(
        "effect.minecraft.bad_omen",
        "effect.minecraft.bad_omen",
    ));
    if amplifier > 0 {
        let potency = translate(&format!("potion.potency.{amplifier}"))
            .unwrap_or_else(|| crate::enchant::level_name(u32::from(amplifier) + 1));
        line = format_translation(
            &text("potion.withAmplifier", "%s %s"),
            &[line, Text::literal(potency)],
        );
    }
    let duration = Text::literal(crate::brewing::format_duration(BAD_OMEN_TICKS));
    let line = format_translation(&text("potion.withDuration", "%s (%s)"), &[line, duration]);
    Some((line.plain(), rgba(85, 85, 255, 255)))
}

/// `PotionContents.addPotionTooltip`'s attribute block: an empty line, "When
/// Applied:" and the modifiers of the potion's effects (blue for a gain, red
/// for a loss); nothing for a potion whose effects add none.
pub fn potion_modifier_lines(
    stack: &ItemStack,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<(String, u32)> {
    if !is_potion_item(&stack.item) {
        return Vec::new();
    }
    let effects = stack
        .potion()
        .and_then(crate::brewing::potion)
        .into_iter()
        .flat_map(|p| p.effects.iter())
        .map(|(effect, _, amplifier)| (*effect, *amplifier));
    effect_modifier_lines(effects, translate)
}

/// `addPotionTooltip`'s attribute block for (effect id, amplifier) entries.
fn effect_modifier_lines<'a>(
    effects: impl IntoIterator<Item = (&'a str, i32)>,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<(String, u32)> {
    let applied: Vec<(String, u32)> = effects
        .into_iter()
        .filter_map(|(effect, amplifier)| {
            let (attribute, amount, operation) =
                effect_modifier(effect.trim_start_matches("minecraft:"))?;
            let amount = amount * (amplifier + 1) as f64;
            let line = modifier_line(translate, attribute, amount, operation, false)?;
            let color = if amount > 0.0 {
                rgba(85, 85, 255, 255)
            } else {
                rgba(255, 85, 85, 255)
            };
            Some((line, color))
        })
        .collect();
    if applied.is_empty() {
        return applied;
    }
    let mut lines = vec![
        (String::new(), u32::MAX),
        (
            translate("potion.whenDrank").unwrap_or_else(|| "potion.whenDrank".into()),
            rgba(170, 0, 170, 255),
        ),
    ];
    lines.extend(applied);
    lines
}

/// `SuspiciousStewEffects.addToTooltip`: only with the creative tooltip flag,
/// the stew's effects as `PotionContents.addPotionTooltip` lists them (each in
/// its `MobEffectCategory` colour, "No Effects" for none), then their
/// attribute modifiers.
pub fn stew_lines(
    stack: &ItemStack,
    creative: bool,
    display: &TooltipDisplay,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<(String, u32)> {
    let effects = stack.data().map_or(&[][..], |d| &d.stew_effects[..]);
    // The component is the item's default (empty) on a suspicious stew.
    if !creative
        || (effects.is_empty() && &*stack.item != "minecraft:suspicious_stew")
        || !display.shows("minecraft:suspicious_stew_effects")
    {
        return Vec::new();
    }
    if effects.is_empty() {
        let none = translate("effect.none").unwrap_or_else(|| "No Effects".into());
        return vec![(none, rgba(170, 170, 170, 255))];
    }
    // `Entry.createEffectInstance`: amplifier 0.
    let entries = effects.iter().map(|(id, duration)| (&**id, *duration, 0));
    let mut lines: Vec<(String, u32)> = crate::brewing::effect_lines(entries, 1.0, translate)
        .into_iter()
        .map(|(line, category)| {
            // MobEffectCategory.getTooltipFormatting: harmful red, else blue.
            let color = match category {
                crate::brewing::Category::Harmful => rgba(255, 85, 85, 255),
                _ => rgba(85, 85, 255, 255),
            };
            (line, color)
        })
        .collect();
    lines.extend(effect_modifier_lines(
        effects.iter().map(|(id, _)| (&**id, 0)),
        translate,
    ));
    lines
}

/// `ItemStack.components.size()`: the item's default components with the
/// stack's own changes applied.
pub fn component_count(stack: &ItemStack, info: Option<&ItemInfo>) -> usize {
    let mut keys: std::collections::HashSet<String> = info
        .map(|i| i.components.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(Tag::Compound(patch)) = crate::item_data::components_tag(stack.damage, stack.data())
    {
        for key in patch.keys() {
            let key = key.to_string_lossy();
            match key.strip_prefix('!') {
                Some(removed) => keys.remove(removed),
                None => keys.insert(key),
            };
        }
    }
    keys.len()
}

/// `SmithingTemplateItem.appendHoverText`: the template's kind, what it applies to
/// and its ingredients (gray titles over blue descriptions); none for other items.
pub fn smithing_template_lines(
    stack: &ItemStack,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<(String, u32)> {
    let path = stack.item.trim_start_matches("minecraft:");
    if !path.ends_with("_smithing_template") {
        return Vec::new();
    }
    let text = |key: &str| translate(key).unwrap_or_else(|| key.to_owned());
    let kind = if path == "netherite_upgrade_smithing_template" {
        "netherite_upgrade"
    } else {
        "armor_trim"
    };
    let (title, description) = (rgba(170, 170, 170, 255), rgba(85, 85, 255, 255));
    let described = |what: &str| {
        let key = format!("item.minecraft.smithing_template.{kind}.{what}");
        (format!(" {}", text(&key)), description)
    };
    vec![
        (text("item.minecraft.smithing_template"), title),
        (String::new(), u32::MAX),
        (text("item.minecraft.smithing_template.applies_to"), title),
        described("applies_to"),
        (text("item.minecraft.smithing_template.ingredients"), title),
        described("ingredients"),
    ]
}

/// The lines of `stack` below its name that its components give, in
/// `ItemStack.addDetailsToTooltip` order.
pub fn detail_lines(
    stack: &ItemStack,
    info: Option<&ItemInfo>,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    let text = |key: &str| translate(key).unwrap_or_else(|| key.to_owned());
    let path = stack.item.trim_start_matches("minecraft:");
    let mut lines = Vec::new();
    lines.extend(
        smithing_template_lines(stack, translate)
            .into_iter()
            .map(|l| l.0),
    );
    let display = TooltipDisplay::for_stack(stack);
    lines.extend(bees_line(stack, &display, translate));
    lines.extend(honey_line(stack, &display, translate));
    let data = stack.data();
    // PotDecorations.addToTooltip.
    if let Some(sherds) = data
        .and_then(|d| d.pot_decorations.as_ref())
        .filter(|s| s.iter().any(Option::is_some))
    {
        lines.push(String::new());
        for sherd in sherds.iter() {
            let item = sherd.as_ref().map_or("minecraft:brick", |s| &*s.item);
            lines.push(name_line(&ItemStack::new(item, 1), translate));
        }
    }
    lines.extend(firework_lines(stack, translate));
    // PotionContents.addToTooltip: the effects, then their attribute modifiers.
    if is_potion_item(&stack.item) {
        let scale = match path {
            "lingering_potion" => 0.25,
            "tipped_arrow" => 0.125,
            _ => 1.0,
        };
        lines.extend(
            crate::brewing::tooltip(stack.potion(), scale, translate)
                .into_iter()
                .map(|l| l.0),
        );
        lines.extend(
            potion_modifier_lines(stack, translate)
                .into_iter()
                .map(|l| l.0),
        );
    }
    lines.extend(jukebox_line(stack, info, &display, translate));
    // Trim.addToTooltip.
    if let Some((material, pattern)) = data.and_then(|d| d.trim.as_ref()) {
        let short = |s: &str| s.trim_start_matches("minecraft:").to_owned();
        lines.push(text("item.minecraft.smithing_template.upgrade"));
        lines.push(format!(
            " {}",
            text(&format!("trim_pattern.minecraft.{}", short(pattern)))
        ));
        lines.push(format!(
            " {}",
            text(&format!("trim_material.minecraft.{}", short(material)))
        ));
    }
    lines.extend(dyed_line(stack, &display, false, translate).map(|l| l.0));
    if let Some(info) = info {
        lines.extend(
            attribute_lines(&info.modifiers, None, translate)
                .into_iter()
                .map(|(line, _)| line),
        );
    }
    lines.extend(ominous_bottle_line(stack, &display, translate).map(|l| l.0));
    lines
}

/// `Fireworks.addToTooltip` (a rocket's flight duration, its explosions with
/// runs of equal ones counted) and `FireworkExplosion.addToTooltip` (a
/// star's), unless the stack's tooltip display hides them. A rocket without
/// the component has the item's default: flight duration 1, no explosions.
pub fn firework_lines(
    stack: &ItemStack,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    use crate::particles::FireworkExplosion;
    let text = |key: &str| translate(key).unwrap_or_else(|| key.to_owned());
    let shape = |e: &FireworkExplosion| {
        text(&format!(
            "item.minecraft.firework_star.shape.{}",
            e.shape.name()
        ))
    };
    // FireworkExplosion.appendColors: dye names (getColorName), comma-separated.
    let colors = |colors: &[u32]| {
        colors
            .iter()
            .map(|c| {
                match crate::particles::DYE_FIREWORK_COLORS
                    .iter()
                    .find(|(_, firework)| firework == c)
                {
                    Some((name, _)) => text(&format!("item.minecraft.firework_star.{name}")),
                    None => text("item.minecraft.firework_star.custom_color"),
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    // FireworkExplosion.addAdditionalTooltip.
    let details = |e: &FireworkExplosion, indent: &str, lines: &mut Vec<String>| {
        if !e.colors.is_empty() {
            lines.push(format!("{indent}{}", colors(&e.colors)));
        }
        if !e.fade.is_empty() {
            let fade = text("item.minecraft.firework_star.fade_to");
            lines.push(format!("{indent}{fade} {}", colors(&e.fade)));
        }
        if e.trail {
            lines.push(format!(
                "{indent}{}",
                text("item.minecraft.firework_star.trail")
            ));
        }
        if e.twinkle {
            lines.push(format!(
                "{indent}{}",
                text("item.minecraft.firework_star.flicker")
            ));
        }
    };
    let display = TooltipDisplay::for_stack(stack);
    let data = stack.data();
    let mut lines = Vec::new();
    let fireworks = match data.and_then(|d| d.fireworks.as_ref()) {
        Some((flight, explosions)) => Some((*flight, explosions.as_slice())),
        None => (&*stack.item == "minecraft:firework_rocket").then_some((1, &[][..])),
    };
    if let Some((flight, explosions)) = fireworks
        && display.shows("minecraft:fireworks")
    {
        if flight > 0 {
            lines.push(
                text("item.minecraft.firework_rocket.flight_duration").replacen(
                    "%s",
                    &flight.to_string(),
                    1,
                ),
            );
        }
        // Fireworks.addExplosionTooltip for each run of equal explosions.
        for run in explosions.chunk_by(|a, b| a == b) {
            let e = &run[0];
            lines.push(if run.len() == 1 {
                text("item.minecraft.firework_rocket.single_star").replacen("%s", &shape(e), 1)
            } else {
                text("item.minecraft.firework_rocket.multiple_stars")
                    .replacen("%s", &run.len().to_string(), 1)
                    .replacen("%s", &shape(e), 1)
            });
            details(e, "  ", &mut lines);
        }
    }
    if let Some(e) = data.and_then(|d| d.firework_explosion.as_ref())
        && display.shows("minecraft:firework_explosion")
    {
        lines.push(shape(e));
        details(e, "", &mut lines);
    }
    lines
}

/// `ItemContainerContents.addToTooltip`: "count x name" for the first five
/// stacks, by slot, then how many more there are (an italic line).
pub fn container_lines(
    stack: &ItemStack,
    name: &dyn Fn(&ItemStack) -> String,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    let Some(data) = stack.data() else {
        return Vec::new();
    };
    let mut slots: Vec<&(u8, ItemStack)> = data.container.iter().collect();
    slots.sort_by_key(|(slot, _)| *slot);
    let count = translate("item.container.item_count").unwrap_or_else(|| "%s x %s".into());
    let mut lines: Vec<String> = slots
        .iter()
        .take(5)
        .map(|(_, s)| {
            count
                .replacen("%s", &s.count.to_string(), 1)
                .replacen("%s", &name(s), 1)
        })
        .collect();
    if slots.len() > 5 {
        let more =
            translate("item.container.more_items").unwrap_or_else(|| "and %s more...".into());
        lines.push(more.replacen("%s", &(slots.len() - 5).to_string(), 1));
    }
    lines
}

/// Every line of `stack`'s tooltip, name first.
pub fn lines(
    stack: &ItemStack,
    info: Option<&ItemInfo>,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    let mut lines = vec![name_line(stack, translate)];
    if TooltipDisplay::for_stack(stack).shows("minecraft:lore") {
        lines.extend(lore_texts(stack).into_iter().map(|t| t.plain()));
    }
    lines.extend(detail_lines(stack, info, translate));
    lines
}

/// `ItemLore.addToTooltip`: each lore component under dark purple italic, with
/// the component's own styles (including explicit `false`) winning on children.
pub fn lore_texts(stack: &ItemStack) -> Vec<crate::text::Text> {
    let Some(Tag::List(entries)) = stack.data().and_then(|d| d.other("minecraft:lore")) else {
        return Vec::new();
    };
    let base = crate::text::Style {
        color: Some(crate::text::DARK_PURPLE),
        italic: Some(true),
        ..Default::default()
    };
    entries
        .iter()
        .map(|entry| {
            let mut text = crate::entity_nbt::name_text(entry).resolve_legacy();
            for span in &mut text.spans {
                span.style.inherit(&base);
            }
            text
        })
        .collect()
}

/// Preserve child colours and explicit styles in the stored hover name.
/// A custom name inherits italic from `ItemStack.getStyledHoverName`.
pub fn styled_name(stack: &ItemStack, fallback: impl FnOnce() -> String) -> crate::text::Text {
    if let Some(component) = stack.data().and_then(|d| d.custom_name.as_ref()) {
        return crate::entity_nbt::name_text(component).with_style(&crate::text::Style {
            italic: Some(true),
            ..Default::default()
        });
    }
    if let Some(component) = stack.data().and_then(|d| d.other("minecraft:item_name")) {
        return crate::entity_nbt::name_text(component);
    }
    crate::text::Text::legacy(&fallback())
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_world::loot::PackOverlay;
    use std::sync::Arc;

    #[test]
    fn styled_name_keeps_gradient_spans_and_explicit_italic() {
        let component = Tag::compound([
            ("text", Tag::String("".into())),
            (
                "extra",
                Tag::List(vec![
                    Tag::compound([
                        ("text", Tag::String("A".into())),
                        ("color", Tag::String("#ff0000".into())),
                        ("italic", Tag::Byte(0)),
                    ]),
                    Tag::compound([
                        ("text", Tag::String("B".into())),
                        ("color", Tag::String("#0000ff".into())),
                    ]),
                ]),
            ),
        ]);
        let mut stack = ItemStack::new("minecraft:stick", 1);
        stack.data_mut().custom_name = Some(component);
        let text = styled_name(&stack, || "unused".into());
        assert_eq!(text.plain(), "AB");
        assert_eq!(text.spans[0].style.color, Some(0xff0000));
        assert_eq!(text.spans[1].style.color, Some(0x0000ff));
        assert!(!text.spans[0].style.is_italic());
        assert!(text.spans[1].style.is_italic());
    }

    #[test]
    fn lore_keeps_component_colours_under_dark_purple_italic() {
        let mut stack = ItemStack::new("minecraft:stick", 1);
        stack.data_mut().set_other(
            "minecraft:lore",
            Some(Tag::List(vec![
                Tag::compound([
                    ("text", Tag::String("Gold line".into())),
                    ("color", Tag::String("gold".into())),
                    ("italic", Tag::Byte(0)),
                ]),
                Tag::String("Plain lore".into()),
            ])),
        );
        let lines = lore_texts(&stack);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].plain(), "Gold line");
        assert_eq!(lines[0].spans[0].style.color, Some(crate::text::GOLD));
        assert!(!lines[0].spans[0].style.is_italic());
        assert_eq!(lines[1].plain(), "Plain lore");
        assert_eq!(
            lines[1].spans[0].style.color,
            Some(crate::text::DARK_PURPLE)
        );
        assert!(lines[1].spans[0].style.is_italic());
        assert!(TooltipDisplay::for_stack(&stack).shows("minecraft:lore"));
        stack.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(Tag::compound([(
                "hidden_components",
                Tag::List(vec![Tag::String("minecraft:lore".into())]),
            )])),
        );
        assert!(!TooltipDisplay::for_stack(&stack).shows("minecraft:lore"));
    }

    #[test]
    fn tooltip_display_controls_name_and_component_visibility() {
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        let display = TooltipDisplay::for_stack(&stack);
        assert!(display.name_visible(false));
        assert!(display.shows("minecraft:attribute_modifiers"));

        stack.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(Tag::compound([(
                "hidden_components",
                Tag::List(vec![Tag::String("minecraft:attribute_modifiers".into())]),
            )])),
        );
        let display = TooltipDisplay::for_stack(&stack);
        assert!(display.name_visible(false));
        assert!(!display.shows("minecraft:attribute_modifiers"));
        assert!(display.shows("minecraft:enchantments"));

        stack.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(Tag::compound([("hide_tooltip", Tag::Byte(1))])),
        );
        let display = TooltipDisplay::for_stack(&stack);
        assert!(!display.name_visible(false));
        assert!(display.name_visible(true));
        assert!(!display.shows("minecraft:attribute_modifiers"));
    }

    #[test]
    fn stew_effects_show_only_with_the_creative_flag() {
        let translate = |key: &str| {
            Some(
                match key {
                    "effect.minecraft.fire_resistance" => "Fire Resistance",
                    "effect.minecraft.weakness" => "Weakness",
                    "effect.minecraft.saturation" => "Saturation",
                    "effect.none" => "No Effects",
                    "potion.whenDrank" => "When Applied:",
                    "attribute.modifier.take.0" => "-%s %s",
                    "attribute.name.attack_damage" => "Attack Damage",
                    _ => return None,
                }
                .to_owned(),
            )
        };
        let (blue, red) = (rgba(85, 85, 255, 255), rgba(255, 85, 85, 255));
        let lines = |stack: &ItemStack, creative: bool| {
            stew_lines(
                stack,
                creative,
                &TooltipDisplay::for_stack(stack),
                &translate,
            )
        };
        let mut stew = ItemStack::new("minecraft:suspicious_stew", 1);
        // The item's default, an empty list.
        assert_eq!(
            lines(&stew, true),
            [("No Effects".to_owned(), rgba(170, 170, 170, 255))]
        );
        stew.data_mut().stew_effects = vec![
            (Arc::from("minecraft:fire_resistance"), 60),
            (Arc::from("minecraft:weakness"), 140),
            (Arc::from("minecraft:saturation"), 7),
        ];
        assert!(lines(&stew, false).is_empty());
        assert_eq!(
            lines(&stew, true),
            [
                ("Fire Resistance (00:03)".to_owned(), blue),
                ("Weakness (00:07)".to_owned(), red),
                // 20 ticks or less: no duration.
                ("Saturation".to_owned(), blue),
                (String::new(), u32::MAX),
                ("When Applied:".to_owned(), rgba(170, 0, 170, 255)),
                ("-4 Attack Damage".to_owned(), red),
            ]
        );
        assert!(lines(&ItemStack::new("minecraft:bowl", 1), true).is_empty());
    }

    #[test]
    fn ominous_bottle_lists_bad_omen_at_its_amplifier_for_100_minutes() {
        let lang = |key: &str| {
            Some(
                match key {
                    "effect.minecraft.bad_omen" => "Bad Omen",
                    "potion.withAmplifier" => "%s %s",
                    "potion.withDuration" => "%s (%s)",
                    "potion.potency.3" => "IV",
                    _ => return None,
                }
                .to_owned(),
            )
        };
        let duration = crate::brewing::format_duration(120_000);
        let blue = rgba(85, 85, 255, 255);
        // The item's default amplifier, 0: no potency.
        let mut bottle = ItemStack::new("minecraft:ominous_bottle", 1);
        let line = |s: &ItemStack| ominous_bottle_line(s, &TooltipDisplay::for_stack(s), &lang);
        assert_eq!(
            line(&bottle),
            Some((format!("Bad Omen ({duration})"), blue))
        );
        bottle.data_mut().ominous_amplifier = Some(3);
        assert_eq!(
            line(&bottle),
            Some((format!("Bad Omen IV ({duration})"), blue))
        );
        assert!(
            lines(&bottle, None, &lang).contains(&format!("Bad Omen IV ({duration})")),
            "{:?}",
            lines(&bottle, None, &lang)
        );
        // A numbered template, as some languages write it.
        let numbered = |key: &str| match key {
            "potion.withDuration" => Some("%2$s: %1$s".to_owned()),
            other => lang(other),
        };
        assert_eq!(
            ominous_bottle_line(&bottle, &TooltipDisplay::for_stack(&bottle), &numbered)
                .map(|l| l.0),
            Some(format!("{duration}: Bad Omen IV"))
        );
        bottle.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(Tag::compound([(
                "hidden_components",
                Tag::List(vec![Tag::String(
                    "minecraft:ominous_bottle_amplifier".into(),
                )]),
            )])),
        );
        assert_eq!(line(&bottle), None);
        assert_eq!(line(&ItemStack::new("minecraft:glass_bottle", 1)), None);
    }

    #[test]
    fn ominous_banner_keeps_its_default_hidden_pattern_tooltip() {
        let mut banner = crate::mobs::raider::ominous_banner();
        let display = TooltipDisplay::for_stack(&banner);
        assert!(!display.shows("minecraft:banner_patterns"));

        banner.data_mut().set_other(
            "minecraft:tooltip_display",
            Some(Tag::Compound(Default::default())),
        );
        let display = TooltipDisplay::for_stack(&banner);
        assert!(display.shows("minecraft:banner_patterns"));
    }

    #[test]
    fn ominous_banner_name_and_rarity_use_the_stack() {
        let translate = |key: &str| match key {
            "block.minecraft.ominous_banner" => Some("Ominous Banner".to_owned()),
            "block.minecraft.white_banner" => Some("White Banner".to_owned()),
            _ => None,
        };
        let mut banner = crate::mobs::raider::ominous_banner();
        assert_eq!(name_line(&banner, &translate), "Ominous Banner");
        assert_eq!(name_line(&banner, &|_| None), "Ominous Banner");
        assert_eq!(rarity(&banner, "common"), "uncommon");
        banner.data_mut().custom_name = Some(Tag::String("Captain's Standard".into()));
        assert_eq!(name_line(&banner, &translate), "Captain's Standard");
        let mut custom_item = ItemStack::new("minecraft:diamond_sword", 1);
        custom_item.data_mut().other.push((
            "minecraft:item_name".into(),
            Tag::String("Excalibur".into()),
        ));
        assert_eq!(name_line(&custom_item, &translate), "Excalibur");
        custom_item.data_mut().custom_name = Some(Tag::String("Custom Blade".into()));
        assert_eq!(name_line(&custom_item, &translate), "Custom Blade");
        banner.enchant("minecraft:unbreaking", 1);
        assert_eq!(rarity(&banner, "common"), "rare");
        let plain = ItemStack::new("minecraft:white_banner", 1);
        assert_eq!(name_line(&plain, &translate), "White Banner");
        assert_eq!(rarity(&plain, "common"), "common");
    }

    #[test]
    fn name_line_translates_items_of_any_namespace() {
        let translate = |key: &str| match key {
            "item.example.ruby" => Some("Ruby".to_owned()),
            "block.example.ruby_block" => Some("Block of Ruby".to_owned()),
            "item.example.tools.chisel" => Some("Chisel".to_owned()),
            "item.minecraft.diamond" => Some("Diamond".to_owned()),
            _ => None,
        };
        let name = |id: &str| name_line(&ItemStack::new(id, 1), &translate);
        // `Util.makeDescriptionId`: the item's own namespace, `/` as `.`.
        assert_eq!(name("example:ruby"), "Ruby");
        assert_eq!(name("example:ruby_block"), "Block of Ruby");
        assert_eq!(name("example:tools/chisel"), "Chisel");
        assert_eq!(name("minecraft:diamond"), "Diamond");
        // No translation: the path, as the creative names fall back to.
        assert_eq!(name("example:raw_ruby"), "raw ruby");
    }

    #[test]
    fn player_head_names_use_the_profile() {
        let translate = |key: &str| match key {
            "block.minecraft.player_head.named" => Some("%s's Head".to_owned()),
            "block.minecraft.player_head" => Some("Player Head".to_owned()),
            _ => None,
        };
        let mut named = ItemStack::new("minecraft:player_head", 1);
        named
            .data_mut()
            .set_other("minecraft:profile", Some(Tag::String("Notch".into())));
        assert_eq!(name_line(&named, &translate), "Notch's Head");

        let mut compound = ItemStack::new("minecraft:player_head", 1);
        compound.data_mut().set_other(
            "minecraft:profile",
            Some(Tag::compound([("name", Tag::String("Notch".into()))])),
        );
        assert_eq!(name_line(&compound, &translate), "Notch's Head");

        let mut id_only = ItemStack::new("minecraft:player_head", 1);
        id_only.data_mut().set_other(
            "minecraft:profile",
            Some(Tag::compound([("id", Tag::IntArray(vec![1, 2, 3, 4]))])),
        );
        assert_eq!(name_line(&id_only, &translate), "Player Head");

        named.data_mut().custom_name = Some(Tag::String("Trophy".into()));
        assert_eq!(name_line(&named, &translate), "Trophy");
    }

    #[test]
    fn container_lines_list_five_stacks_then_how_many_more() {
        let mut chest = ItemStack::new("minecraft:chest", 1);
        for slot in [9u8, 1, 2, 3, 4, 5, 6] {
            chest
                .data_mut()
                .container
                .push((slot, ItemStack::new("minecraft:stone", u32::from(slot))));
        }
        let lang = |k: &str| {
            Some(
                match k {
                    "item.container.item_count" => "%s x %s",
                    "item.container.more_items" => "and %s more...",
                    _ => return None,
                }
                .to_owned(),
            )
        };
        let name = |s: &ItemStack| s.item.trim_start_matches("minecraft:").to_owned();
        assert_eq!(
            container_lines(&chest, &name, &lang),
            [
                "1 x stone",
                "2 x stone",
                "3 x stone",
                "4 x stone",
                "5 x stone",
                "and 2 more..."
            ]
        );
        assert!(container_lines(&ItemStack::new("minecraft:chest", 1), &name, &lang).is_empty());
    }

    #[test]
    fn fireworks_use_loaded_tooltip_translations() {
        let root = std::env::temp_dir().join(format!(
            "horizon-firework-tooltip-language-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let dir = root.join("assets/minecraft/lang");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("en_us.json"),
            serde_json::to_vec(&serde_json::json!({
                "item.minecraft.firework_rocket.flight_duration": "Flight Duration: %s",
                "item.minecraft.firework_rocket.multiple_stars": "%s x %s",
                "item.minecraft.firework_star.shape.small_ball": "Small Ball",
                "item.minecraft.firework_star.red": "Red",
                "item.minecraft.firework_star.custom_color": "Custom",
                "item.minecraft.firework_star.fade_to": "Fade to",
                "item.minecraft.firework_star.trail": "Trail",
                "item.minecraft.firework_star.flicker": "Twinkle",
            }))
            .unwrap(),
        )
        .unwrap();
        let enchantments = crate::enchant::Enchantments::load(&root);
        std::fs::remove_dir_all(&root).unwrap();
        let translate = |key: &str| enchantments.translate(key).map(str::to_owned);
        let mut rocket = ItemStack::new("minecraft:firework_rocket", 1);
        assert_eq!(firework_lines(&rocket, &translate), ["Flight Duration: 1"]);
        let explosion = crate::particles::FireworkExplosion {
            colors: vec![0xB3312C],
            fade: vec![0x123456],
            trail: true,
            twinkle: true,
            ..Default::default()
        };
        rocket.data_mut().fireworks = Some((3, vec![explosion.clone(), explosion.clone()]));
        assert_eq!(
            firework_lines(&rocket, &translate),
            [
                "Flight Duration: 3",
                "2 x Small Ball",
                "  Red",
                "  Fade to Custom",
                "  Trail",
                "  Twinkle",
            ]
        );
        let mut star = ItemStack::new("minecraft:firework_star", 1);
        star.data_mut().firework_explosion = Some(explosion);
        assert_eq!(
            firework_lines(&star, &translate),
            ["Small Ball", "Red", "Fade to Custom", "Trail", "Twinkle"]
        );
    }

    #[test]
    fn amounts_use_two_decimals_at_most() {
        assert_eq!(format_amount(8.0), "8");
        assert_eq!(format_amount(3.2000000953674316), "3.2");
        assert_eq!(format_amount(2.8200000524520874), "2.82");
        assert_eq!(format_amount(0.125), "0.12");
        assert_eq!(format_amount(0.375), "0.38");
    }

    #[test]
    fn equipment_attributes_use_player_totals_and_slot_group_colors() {
        let modifiers = [
            ("attack_damage", "base_attack_damage", 6.0, "mainhand"),
            ("attack_speed", "base_attack_speed", -2.4, "mainhand"),
            ("armor", "armor", 3.0, "head"),
        ]
        .map(|(attribute, id, amount, slot)| {
            (
                format!("minecraft:{attribute}"),
                format!("minecraft:{id}"),
                amount,
                "add_value".to_owned(),
                slot.to_owned(),
            )
        });
        let root = std::env::temp_dir().join(format!(
            "horizon-tooltip-language-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let dir = root.join("assets/minecraft/lang");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("en_us.json"),
            serde_json::to_vec(&serde_json::json!({
                "item.modifiers.mainhand": "When in Main Hand:",
                "item.modifiers.head": "When on Head:",
                "attribute.modifier.equals.0": "%s %s",
                "attribute.modifier.plus.0": "+%s %s",
                "attribute.modifier.take.0": "-%s %s",
                "attribute.name.attack_damage": "Attack Damage",
                "attribute.name.attack_speed": "Attack Speed",
                "attribute.name.armor": "Armor",
            }))
            .unwrap(),
        )
        .unwrap();
        let enchantments = crate::enchant::Enchantments::load(&root);
        std::fs::remove_dir_all(&root).unwrap();
        let lang = |key: &str| enchantments.translate(key).map(str::to_owned);
        let green = rgba(0, 170, 0, 255);
        let gray = rgba(170, 170, 170, 255);
        let blue = rgba(85, 85, 255, 255);
        let red = rgba(255, 85, 85, 255);
        assert_eq!(
            attribute_lines(&modifiers, Some([1.0, 4.0]), &lang),
            [
                (String::new(), u32::MAX),
                ("When in Main Hand:".into(), gray),
                (" 7 Attack Damage".into(), green),
                (" 1.6 Attack Speed".into(), green),
                (String::new(), u32::MAX),
                ("When on Head:".into(), gray),
                ("+3 Armor".into(), blue),
            ]
        );
        let recipe = attribute_lines(&modifiers, None, &lang);
        assert_eq!(recipe[2], ("+6 Attack Damage".into(), blue));
        assert_eq!(recipe[3], ("-2.4 Attack Speed".into(), red));
        let custom_base = attribute_lines(&modifiers, Some([2.0, 6.0]), &lang);
        assert_eq!(custom_base[2].0, " 8 Attack Damage");
        assert_eq!(custom_base[3].0, " 3.6 Attack Speed");
        assert_eq!(
            modifier_line(&lang, "attack_damage", 0.0, 0, true).as_deref(),
            Some("0 Attack Damage")
        );
    }

    #[test]
    #[ignore = "requires prepared items/language in HORIZON_TEST_RESOURCES"]
    fn equipment_attributes_with_prepared_items_and_language() {
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES"),
        );
        let items = crate::items::Items::catalog(&root, &PackOverlay::NONE).unwrap();
        let enchantments = crate::enchant::Enchantments::load(&root);
        let translate = |key: &str| enchantments.translate(key).map(str::to_owned);
        for (item, expected) in [
            (
                "minecraft:diamond_sword",
                [
                    "",
                    "When in Main Hand:",
                    " 7 Attack Damage",
                    " 1.6 Attack Speed",
                ],
            ),
            (
                "minecraft:diamond_helmet",
                ["", "When on Head:", "+3 Armor", "+2 Armor Toughness"],
            ),
        ] {
            let info = items.get(item).unwrap();
            let lines = attribute_lines(&info.modifiers, Some([1.0, 4.0]), &translate);
            assert_eq!(
                lines
                    .iter()
                    .map(|(text, _)| text.as_str())
                    .collect::<Vec<_>>(),
                expected,
                "{item}"
            );
        }
    }

    #[test]
    fn modifier_lines_follow_the_sign_and_operation() {
        let lang = |k: &str| {
            Some(
                match k {
                    "attribute.modifier.plus.2" => "+%s%% %s",
                    "attribute.modifier.take.2" => "-%s%% %s",
                    "attribute.name.movement_speed" => "Speed",
                    _ => return None,
                }
                .to_owned(),
            )
        };
        assert_eq!(
            modifier_line(&lang, "movement_speed", 0.2, 2, false).as_deref(),
            Some("+20% Speed")
        );
        assert_eq!(
            modifier_line(&lang, "movement_speed", -0.6, 2, false).as_deref(),
            Some("-60% Speed")
        );
        assert_eq!(modifier_line(&lang, "movement_speed", 0.0, 2, false), None);
    }

    #[test]
    fn modifier_lines_support_positional_format() {
        let lang = |k: &str| {
            Some(
                match k {
                    "attribute.modifier.equals.0" => "%2$s: %1$s",
                    "attribute.modifier.plus.2" => "+%1$s%% %2$s",
                    "attribute.name.attack_damage" => "Damage",
                    _ => return None,
                }
                .to_owned(),
            )
        };
        assert_eq!(
            modifier_line(&lang, "attack_damage", 5.0, 0, true).as_deref(),
            Some("Damage: 5")
        );
        assert_eq!(
            modifier_line(&lang, "attack_damage", 0.2, 2, false).as_deref(),
            Some("+20% Damage")
        );
    }

    #[test]
    fn a_spawn_egg_of_a_monster_says_so_in_peaceful() {
        let lang =
            |k: &str| (k == "item.spawn_egg.peaceful").then(|| "Disabled in Peaceful".to_owned());
        let monsters = |id: &str| !matches!(id, "minecraft:zombie" | "minecraft:creeper");
        let line = |s: &ItemStack, peaceful| {
            spawn_egg_peaceful_line(s, peaceful, &TooltipDisplay::for_stack(s), &monsters, &lang)
        };
        let zombie = ItemStack::new("minecraft:zombie_spawn_egg", 1);
        assert_eq!(line(&zombie, true).as_deref(), Some("Disabled in Peaceful"));
        // Not in Peaceful, or an egg of a mob that is allowed there, or no egg.
        assert_eq!(line(&zombie, false), None);
        assert_eq!(
            line(&ItemStack::new("minecraft:pig_spawn_egg", 1), true),
            None
        );
        assert_eq!(line(&ItemStack::new("minecraft:stone", 1), true), None);
        // The entity data's type wins over the egg's own.
        let mut pig = ItemStack::new("minecraft:pig_spawn_egg", 1);
        pig.data_mut().set_other(
            "minecraft:entity_data",
            Some(Tag::compound([(
                "id",
                Tag::String("minecraft:creeper".into()),
            )])),
        );
        assert_eq!(line(&pig, true).as_deref(), Some("Disabled in Peaceful"));
        // Any item with the component, and an id without its namespace.
        let mut stand = ItemStack::new("minecraft:armor_stand", 1);
        stand.data_mut().set_other(
            "minecraft:entity_data",
            Some(Tag::compound([("id", Tag::String("zombie".into()))])),
        );
        assert_eq!(line(&stand, true).as_deref(), Some("Disabled in Peaceful"));
    }
}
