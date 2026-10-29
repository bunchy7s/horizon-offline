//! Potions and the brewing stand: the `Potions` / `MobEffects` registries (code
//! in vanilla, ported as tables), data-driven `minecraft:brewing` recipes from
//! `data/minecraft/recipe/brewing`, potion colours and tooltips, and
//! `BrewingStandBlockEntity.serverTick`.
pub use crate::chunk::block_entity::BrewingStand;
use crate::{
    items::{ItemStack, Items},
    providers::Providers,
};
use serde_json::Value;
use std::{collections::HashSet, fs, path::Path};

pub use horizon_client_core::potions::{
    BASE_POTION_COLOR, Category, EFFECTS, EffectDef, POTIONS, PotionDef, effect, namespaced,
    potion, potion_color,
};
/// `MobEffectUtil.formatDuration` at 20 TPS.
pub fn format_duration(ticks: i32) -> String {
    format_tick_duration(ticks, 20.0)
}
/// `StringUtil.formatTickDuration`: mm:ss, or hh:mm:ss from an hour up, every
/// field two digits.
pub fn format_tick_duration(ticks: i32, tick_rate: f32) -> String {
    let secs = (ticks as f32 / tick_rate).floor() as i32;
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}
/// `item.minecraft.potion.effect.<name>` style translation key for a potion item.
pub fn potion_name_key(item: &str, potion: Option<&str>) -> String {
    let prefix = item.trim_start_matches("minecraft:");
    let suffix = potion.and_then(self::potion).map_or("empty", |p| p.name);
    format!("item.minecraft.{prefix}.effect.{suffix}")
}
/// Roman amplifier suffix (`potion.potency.N`) and duration lines of a potion.
pub fn tooltip(
    potion: Option<&str>,
    scale: f32,
    translate: &dyn Fn(&str) -> Option<String>,
) -> Vec<(String, Category)> {
    let Some(def) = potion.and_then(self::potion) else {
        return vec![];
    };
    if def.effects.is_empty() {
        return vec![(
            translate("effect.none").unwrap_or_else(|| "No Effects".into()),
            Category::Neutral,
        )];
    }
    def.effects
        .iter()
        .map(|(e, duration, amp)| {
            let mut line =
                translate(&format!("effect.minecraft.{e}")).unwrap_or_else(|| (*e).to_owned());
            if *amp > 0 {
                line.push(' ');
                line.push_str(
                    &translate(&format!("potion.potency.{amp}"))
                        .unwrap_or_else(|| crate::enchant::level_name(*amp as u32 + 1)),
                );
            }
            let instant = effect(e).is_some_and(|d| d.instant);
            let scaled = (*duration as f32 * scale) as i32;
            if !instant && scaled > 20 {
                line = format!("{line} ({})", format_duration(scaled));
            }
            (line, effect(e).map_or(Category::Neutral, |d| d.category))
        })
        .collect()
}

/// A `PotionIngredient`: item ids plus an optional potion filter.
#[derive(Clone, Debug)]
pub struct PotionIngredient {
    pub items: HashSet<String>,
    pub potions: Option<HashSet<String>>,
}
impl PotionIngredient {
    fn parse(v: &Value, items: &Items) -> Self {
        let mut set = HashSet::new();
        let add = |s: &str, set: &mut HashSet<String>| match s.strip_prefix('#') {
            Some(tag) => set.extend(items.item_tags.get(tag).into_iter().flatten().cloned()),
            None => {
                set.insert(s.to_owned());
            }
        };
        match &v["item"] {
            Value::String(s) => add(s, &mut set),
            Value::Array(a) => a
                .iter()
                .filter_map(Value::as_str)
                .for_each(|s| add(s, &mut set)),
            _ => {}
        }
        let potions = match &v["potion_contents"]["potions"] {
            Value::String(s) => Some([s.clone()].into_iter().collect()),
            Value::Array(a) => Some(
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect(),
            ),
            _ => None,
        };
        Self {
            items: set,
            potions,
        }
    }
    pub fn test(&self, stack: &ItemStack) -> bool {
        self.items.contains(&*stack.item)
            && self
                .potions
                .as_ref()
                .is_none_or(|p| stack.potion().is_some_and(|x| p.contains(x)))
    }
}
#[derive(Clone, Debug)]
pub struct BrewingRecipe {
    pub id: String,
    pub input: PotionIngredient,
    pub reagent: PotionIngredient,
    pub output: ItemStack,
}
pub struct Brewing {
    pub recipes: Vec<BrewingRecipe>,
    /// `RecipePropertySet.BREWING_REAGENTS`.
    pub reagents: HashSet<String>,
    /// Items accepted in the bottle slots (`PotionIngredient.isPotionInput`).
    pub inputs: HashSet<String>,
}
impl Brewing {
    pub fn load(root: &Path, items: &Items) -> Self {
        let mut files: Vec<_> = fs::read_dir(root.join("data/minecraft/recipe/brewing"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        files.sort();
        let mut recipes = Vec::new();
        for p in files {
            let Ok(bytes) = fs::read(&p) else { continue };
            let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
                continue;
            };
            if v["type"] != "minecraft:brewing" {
                continue;
            }
            let mut output = ItemStack::new(
                v["output"]["id"].as_str().unwrap_or("minecraft:potion"),
                v["output"]["count"].as_u64().unwrap_or(1) as u32,
            );
            if let Some(potion) =
                v["output"]["components"]["minecraft:potion_contents"]["potion"].as_str()
            {
                output = output.with_potion(potion);
            }
            recipes.push(BrewingRecipe {
                id: format!(
                    "minecraft:brewing/{}",
                    p.file_stem().and_then(|s| s.to_str()).unwrap_or("")
                ),
                input: PotionIngredient::parse(&v["input"], items),
                reagent: PotionIngredient::parse(&v["reagent"], items),
                output,
            });
        }
        let reagents = recipes
            .iter()
            .flat_map(|r| r.reagent.items.iter().cloned())
            .collect();
        let inputs = recipes
            .iter()
            .flat_map(|r| r.input.items.iter().cloned())
            .chain(std::iter::once("minecraft:glass_bottle".to_owned()))
            .collect();
        Self {
            recipes,
            reagents,
            inputs,
        }
    }
    pub fn find(&self, input: &ItemStack, reagent: &ItemStack) -> Option<&BrewingRecipe> {
        self.recipes
            .iter()
            .find(|r| r.input.test(input) && r.reagent.test(reagent))
    }
    pub fn is_reagent(&self, stack: &ItemStack) -> bool {
        self.reagents.contains(&*stack.item)
    }
    pub fn is_bottle(&self, stack: &ItemStack) -> bool {
        self.inputs.contains(&*stack.item)
    }
}

/// What a brewing tick asks the world to do.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BrewTick {
    /// New `has_bottle_0..2` block state values.
    pub bottles: Option<[bool; 3]>,
    /// Level event 1035 (brew sound).
    pub brewed: bool,
    /// Remainders dropped at the stand.
    pub drops: Vec<ItemStack>,
}
impl BrewingStand {
    pub fn new() -> Self {
        Self {
            total_brew_time: 400,
            total_fuel: 20,
            speed: 1.0,
            ..Default::default()
        }
    }
    fn brewable(&self, brewing: &Brewing) -> bool {
        let Some(ingredient) = &self.items[3] else {
            return false;
        };
        if !brewing.is_reagent(ingredient) {
            return false;
        }
        self.items[..3]
            .iter()
            .flatten()
            .any(|b| brewing.find(b, ingredient).is_some())
    }
    /// `BrewingStandBlockEntity.serverTick`.
    pub fn tick(&mut self, brewing: &Brewing, items: &Items, providers: &Providers) -> BrewTick {
        let mut out = BrewTick::default();
        if self.fuel <= 0
            && let Some(fuel) = self.items[4].as_mut()
            && let Some((uses, speed)) = brewing_fuel(items, fuel)
        {
            self.fuel = providers.int_value(&uses);
            self.total_fuel = self.fuel;
            self.speed = providers.float_value(&speed, 1.0);
            let remainder = items.get(&fuel.item).and_then(|i| i.remainder.clone());
            fuel.count -= 1;
            if fuel.count == 0 {
                self.items[4] = remainder.map(|r| ItemStack::new(&r, 1));
            } else if let Some(r) = remainder {
                out.drops.push(ItemStack::new(&r, 1));
            }
        }
        let brewable = self.brewable(brewing);
        if self.brew_time > 0 {
            self.brew_time -= 1;
            let same = self.items[3].as_ref().map(|s| s.item.clone()) == self.ingredient;
            if self.brew_time == 0 && brewable {
                self.brew(brewing, items, &mut out);
            } else if !brewable || !same {
                self.brew_time = 0;
            }
        } else if brewable && self.fuel > 0 {
            let speed = if self.speed > 0.0 { self.speed } else { 1.0 };
            self.fuel -= 1;
            self.brew_time = (400.0 / speed).ceil() as i32;
            self.total_brew_time = self.brew_time;
            self.ingredient = self.items[3].as_ref().map(|s| s.item.clone());
        }
        let bottles = [
            self.items[0].is_some(),
            self.items[1].is_some(),
            self.items[2].is_some(),
        ];
        if self.last_bottles != Some(bottles) {
            self.last_bottles = Some(bottles);
            out.bottles = Some(bottles);
        }
        out
    }
    fn brew(&mut self, brewing: &Brewing, items: &Items, out: &mut BrewTick) {
        let Some(ingredient) = self.items[3].clone() else {
            return;
        };
        for slot in 0..3 {
            if let Some(input) = &self.items[slot]
                && let Some(r) = brewing.find(input, &ingredient)
            {
                self.items[slot] = Some(r.output.clone());
            }
        }
        let remainder = items
            .get(&ingredient.item)
            .and_then(|i| i.remainder.clone());
        if let Some(i) = self.items[3].as_mut() {
            i.count -= 1;
            if i.count == 0 {
                self.items[3] = remainder.map(|r| ItemStack::new(&r, 1));
            } else if let Some(r) = remainder {
                out.drops.push(ItemStack::new(&r, 1));
            }
        }
        out.brewed = true;
    }
}
/// The `minecraft:brewing_fuel` providers of an item: (uses, speed_multiplier).
pub fn brewing_fuel(items: &Items, stack: &ItemStack) -> Option<(Value, Value)> {
    items.get(&stack.item).and_then(|i| i.brewing_fuel.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn potion_colours_and_names() {
        // Swiftness: speed colour 3402751 = 0x33EBFF.
        assert_eq!(potion_color(Some("minecraft:swiftness")), 0x33EBFF);
        assert_eq!(potion_color(Some("minecraft:water")), BASE_POTION_COLOR);
        // Turtle master mixes slowness x4 and resistance x3.
        let (s, r) = (9154528u32, 9520880u32);
        let mix = |sh: u32| ((s >> sh & 255) * 4 + (r >> sh & 255) * 3) / 7;
        assert_eq!(
            potion_color(Some("minecraft:turtle_master")),
            mix(16) << 16 | mix(8) << 8 | mix(0)
        );
        assert_eq!(
            potion_name_key("minecraft:splash_potion", Some("minecraft:long_swiftness")),
            "item.minecraft.splash_potion.effect.swiftness"
        );
        assert_eq!(format_duration(9600), "08:00");
        assert_eq!(format_duration(450), "00:22");
        assert_eq!(format_duration(140), "00:07");
        assert_eq!(format_duration(1800), "01:30");
        assert_eq!(format_duration(72000), "01:00:00");
        assert_eq!(format_tick_duration(9600, 40.0), "04:00");
        let t = tooltip(Some("minecraft:strong_poison"), 1.0, &|_| None);
        assert_eq!(t[0].0, "poison II (00:21)");
        assert_eq!(POTIONS.len(), 46);
    }

    #[test]
    fn brewing_stand_brews_after_400_ticks() {
        let mut item_tags = std::collections::HashMap::new();
        item_tags.insert("x".into(), HashSet::new());
        let items = Items::for_tests(item_tags);
        let recipe = BrewingRecipe {
            id: "awkward".into(),
            input: PotionIngredient {
                items: ["minecraft:potion".to_owned()].into_iter().collect(),
                potions: Some(["minecraft:water".to_owned()].into_iter().collect()),
            },
            reagent: PotionIngredient {
                items: ["minecraft:nether_wart".to_owned()].into_iter().collect(),
                potions: None,
            },
            output: ItemStack::new("minecraft:potion", 1).with_potion("minecraft:awkward"),
        };
        let brewing = Brewing {
            reagents: ["minecraft:nether_wart".to_owned()].into_iter().collect(),
            inputs: HashSet::new(),
            recipes: vec![recipe],
        };
        let providers = Providers::empty();
        let mut stand = BrewingStand::new();
        stand.fuel = 1;
        stand.items[0] = Some(ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water"));
        stand.items[2] =
            Some(ItemStack::new("minecraft:potion", 1).with_potion("minecraft:awkward"));
        stand.items[3] = Some(ItemStack::new("minecraft:nether_wart", 2));
        let first = stand.tick(&brewing, &items, &providers);
        assert_eq!(first.bottles, Some([true, false, true]));
        assert_eq!((stand.brew_time, stand.fuel), (400, 0));
        let mut brewed_at = 0;
        for t in 1..=400 {
            if stand.tick(&brewing, &items, &providers).brewed {
                brewed_at = t;
            }
        }
        assert_eq!(brewed_at, 400);
        assert_eq!(
            stand.items[0].as_ref().unwrap().potion(),
            Some("minecraft:awkward")
        );
        // The awkward potion had no recipe and stays.
        assert_eq!(
            stand.items[2].as_ref().unwrap().potion(),
            Some("minecraft:awkward")
        );
        assert_eq!(stand.items[3].as_ref().unwrap().count, 1);
        // No fuel left: nothing more brews.
        stand.tick(&brewing, &items, &providers);
        assert_eq!(stand.brew_time, 0);
    }

    /// #3312: `PotionContents.getPotionDescription` names the effects with
    /// translatable components, so a Russian game reads them from ru_ru.json
    /// (the prepared one; skipped without it), as the tooltip's other lines.
    #[test]
    fn effect_lines_read_the_selected_language() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("assets/minecraft/lang/ru_ru.json").exists() {
            return;
        }
        let ru = crate::lang::Language::load(&[&root], "ru_ru");
        let translate = |key: &str| ru.get(key).map(str::to_owned);
        let lines = tooltip(Some("minecraft:turtle_master"), 0.125, &translate);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].0.starts_with("Замедление IV ("), "{lines:?}");
        assert!(lines[1].0.starts_with("Сопротивление III ("), "{lines:?}");
    }
}
