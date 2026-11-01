//! Component conditions used by enchantment-dependent item models.
use super::*;

/// `DataComponentPredicate.singleCodec`: a single predicate's type discriminator
/// (`predicate`) and its wrapped codec's separate `value` field.
pub(super) fn decode(m: &Map<String, Value>) -> Result<Value> {
    let id = component_type(required(m, "predicate")?)?;
    let mut value = required(m, "value")?.clone();
    if matches!(
        id.as_str(),
        "minecraft:enchantments" | "minecraft:stored_enchantments"
    ) {
        let mut entries = Vec::new();
        for entry in list(&value)? {
            let mut entry = object(entry)?.clone();
            if let Some(selector) = get(&entry, "enchantments") {
                let selector = match selector {
                    Value::Array(ids) => Value::Array(
                        ids.iter()
                            .map(ident)
                            .map(|r| r.map(Value::String))
                            .collect::<Result<_>>()?,
                    ),
                    Value::String(name) if name.starts_with('#') => {
                        Value::String(format!("#{}", identifier(&name[1..])?))
                    }
                    _ => Value::String(ident(selector)?),
                };
                entry.insert("enchantments".into(), selector);
            }
            if let Some(levels) = get(&entry, "levels") {
                let levels = if levels.is_object() {
                    let mut bounds = Map::new();
                    for key in ["min", "max"] {
                        if let Some(n) = get(object(levels)?, key) {
                            bounds.insert(key.into(), Value::from(int(n)?));
                        }
                    }
                    // MinMaxBounds.Bounds.validateSwappedBoundsInCodec.
                    if bounds
                        .get("min")
                        .and_then(Value::as_i64)
                        .zip(bounds.get("max").and_then(Value::as_i64))
                        .is_some_and(|(min, max)| min > max)
                    {
                        return Err(err("minimum exceeds maximum"));
                    }
                    Value::Object(bounds)
                } else {
                    Value::from(int(levels)?)
                };
                entry.insert("levels".into(), levels);
            }
            entries.push(Value::Object(entry));
        }
        value = Value::Array(entries);
    }
    Ok(Value::Object(Map::from_iter([(id, value)])))
}

/// `EnchantmentsPredicate.matches` and `EnchantmentPredicate.containedIn`:
/// all entries must match; each entry needs one selected, present enchantment
/// with a matching level. An empty predicate list still needs the component.
pub(super) fn matches(
    predicate: &Value,
    components: &Components,
    env: &dyn ItemEnv,
) -> Option<bool> {
    let (id, expected) = predicate.as_object()?.iter().next()?;
    if !matches!(
        id.as_str(),
        "minecraft:enchantments" | "minecraft:stored_enchantments"
    ) {
        return None;
    }
    let Some(have) = components.get(id).and_then(Value::as_object) else {
        return Some(false);
    };
    Some(expected.as_array()?.iter().all(|entry| {
        let Some(entry) = entry.as_object() else {
            return false;
        };
        have.iter().any(|(id, level)| {
            let level = level.as_i64().unwrap_or(0);
            let selector = get(entry, "enchantments");
            let selected = match selector {
                None => true,
                Some(Value::String(s)) => s
                    .strip_prefix('#')
                    .map_or(s == id, |tag| env.enchantment_in_tag(id, tag)),
                Some(Value::Array(ids)) => ids.iter().any(|s| s.as_str() == Some(id)),
                _ => false,
            };
            selected
                && if selector.is_some() {
                    matches_enchantment(level, get(entry, "levels"))
                } else {
                    level_matches(level, get(entry, "levels"))
                }
        })
    }))
}

/// `EnchantmentPredicate.matchesEnchantment`: a named enchantment needs a
/// nonzero level even when its bounds accept zero.
fn matches_enchantment(level: i64, bounds: Option<&Value>) -> bool {
    level != 0 && level_matches(level, bounds)
}

/// `MinMaxBounds.Ints.matches`: inclusive bounds, or an exact integer.
fn level_matches(level: i64, bounds: Option<&Value>) -> bool {
    match bounds {
        None => true,
        Some(Value::Number(n)) => n.as_i64() == Some(level),
        Some(Value::Object(bounds)) => {
            get(bounds, "min")
                .and_then(Value::as_i64)
                .is_none_or(|min| level >= min)
                && get(bounds, "max")
                    .and_then(Value::as_i64)
                    .is_none_or(|max| level <= max)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Tags;
    impl ItemEnv for Tags {
        fn enchantment_in_tag(&self, enchantment: &str, tag: &str) -> bool {
            enchantment == "minecraft:density" && tag == "horizon:mace"
        }
    }

    #[test]
    fn unnamed_predicates_inspect_entries_but_named_ones_require_a_nonzero_level() {
        let components = Components::from_iter([(
            "minecraft:enchantments".into(),
            json!({"minecraft:density":0}),
        )]);
        assert_eq!(
            matches(&json!({"minecraft:enchantments":[{}]}), &components, &NoEnv),
            Some(true)
        );
        assert_eq!(
            matches(
                &json!({"minecraft:enchantments":[{"enchantments":"minecraft:density"}]}),
                &components,
                &NoEnv
            ),
            Some(false)
        );
    }

    #[test]
    fn enchantment_selectors_use_the_hosts_registry_tags() {
        let condition = json!({"predicate":"enchantments", "value":[
            {"enchantments":"#horizon:mace", "levels":{"min":3}}
        ]});
        let predicate = decode(condition.as_object().unwrap()).unwrap();
        let mut components = Components::from_iter([(
            "minecraft:enchantments".into(),
            json!({"minecraft:density":3}),
        )]);
        assert_eq!(matches(&predicate, &components, &Tags), Some(true));
        assert_eq!(matches(&predicate, &components, &NoEnv), Some(false));
        components.insert(
            "minecraft:enchantments".into(),
            json!({"minecraft:density":2}),
        );
        assert_eq!(matches(&predicate, &components, &Tags), Some(false));
    }
}
