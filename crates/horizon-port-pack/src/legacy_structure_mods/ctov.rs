//! Choice Theorem's Overhauled Village: the entries it adds to other
//! structure sets in Java code (its `modular_compat` processor is
//! `crate::lithostitched`'s now).

/// ChoiceTheorem's Overhauled Village's villages and outposts, which it adds
/// to vanilla's `villages` and `pillager_outposts` sets in Java code
/// (`CTOV.registerstructure`, through lithostitched), with its default
/// config: every village type in three sizes, weighted 10, 4 and 1, and
/// every outpost weighted 1. Vanilla's own entries stay (weight 1 each).
const CTOV_VILLAGES: [&str; 21] = [
    "beach",
    "christmas",
    "desert",
    "desert_oasis",
    "dark_forest",
    "jungle",
    "jungle_tree",
    "mesa",
    "mesa_fortified",
    "mountain",
    "mountain_alpine",
    "mushroom",
    "plains",
    "plains_fortified",
    "savanna",
    "savanna_na",
    "snowy_igloo",
    "swamp",
    "swamp_fortified",
    "taiga",
    "taiga_fortified",
];
const CTOV_OUTPOSTS: [&str; 11] = [
    "beach",
    "dark_forest",
    "desert",
    "jungle",
    "badlands",
    "mountain",
    "plains",
    "savanna",
    "snowy",
    "swamp",
    "taiga",
];

/// The entries a mod adds to other structure sets in Java code, as its
/// default config has them: (file name, set, `[(structure, weight)]`).
/// Lithostitched appends them in the order of a hash map of every modifier
/// in the pack, which carry cannot know; they are written in the order the
/// mod registers them. Each set picks by weight, so how often each starts
/// is the same.
pub(crate) fn code_set_entries(
    mod_id: &str,
) -> Vec<(&'static str, &'static str, Vec<(String, u32)>)> {
    match mod_id {
        "ctov" => {
            let outposts = CTOV_OUTPOSTS
                .iter()
                .map(|v| (format!("ctov:pillager_outpost_{v}"), 1))
                .collect();
            let villages = [("small", 10), ("medium", 4), ("large", 1)]
                .into_iter()
                .flat_map(|(size, weight)| {
                    CTOV_VILLAGES
                        .iter()
                        .map(move |v| (format!("ctov:{size}/village_{v}"), weight))
                })
                .collect();
            vec![
                ("pillager_outposts", "minecraft:pillager_outposts", outposts),
                ("villages", "minecraft:villages", villages),
            ]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctov_joins_vanillas_villages_and_outposts() {
        let sets = code_set_entries("ctov");
        let (file, set, outposts) = &sets[0];
        assert_eq!(
            (*file, *set),
            ("pillager_outposts", "minecraft:pillager_outposts")
        );
        assert_eq!(outposts.len(), 11);
        assert!(outposts.iter().all(|(_, w)| *w == 1));
        let (_, set, villages) = &sets[1];
        assert_eq!(*set, "minecraft:villages");
        assert_eq!(villages.len(), 63);
        assert_eq!(villages[0], ("ctov:small/village_beach".into(), 10));
        assert_eq!(villages[21], ("ctov:medium/village_beach".into(), 4));
        assert_eq!(
            villages[62],
            ("ctov:large/village_taiga_fortified".into(), 1)
        );
        assert!(code_set_entries("mvs").is_empty());
    }
}
