//! The recipe book's drawing, for tools/test_recipe_book_render_parity.py:
//! reads one JSON case per line (book type, GUI size, tabs, search box,
//! filter, recipe collections and page, overlay, ghost slots, cycling index,
//! mouse) and writes the calls `horizon_client::recipe_book_screen` makes,
//! in the oracle's JSON form.
//!
//! usage: recipe_book_render_fixture RESOURCES CASES OUTPUT
use horizon_client::{
    crafting::Recipes,
    hud::{HudAtlas, HudBatch},
    items::{ItemStack, Items},
    lang,
    recipe_book::{Book, BookType, Entry},
    recipe_book_screen::{
        ButtonView, Call, GhostView, ItemCall, OverlayRecipe, OverlayView, Run, Screen, Search,
        TabView, draw, overlay_cells, overlay_origin,
    },
};
use horizon_world::loot::PackOverlay;
use serde_json::{Value, json};
use std::{fmt::Write as _, fs, path::Path};

fn runs(r: &[Run]) -> Value {
    Value::Array(
        r.iter()
            .map(|r| json!([r.text, r.color, r.italic]))
            .collect(),
    )
}
fn round(v: f32) -> f32 {
    (v * 1e4).round() / 1e4
}

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let usage = "usage: recipe_book_render_fixture RESOURCES CASES OUTPUT";
    let root = Path::new(args.get(1).ok_or(usage)?);
    let cases = fs::read_to_string(args.get(2).ok_or(usage)?)?;
    let items = Items::catalog(root, &PackOverlay::NONE)?;
    let recipes = Recipes::load(root, &items, &PackOverlay::NONE)?;
    let book = Book::build(&recipes, &items);
    let atlas = HudAtlas::build(root)?;
    let root_buf = root.to_path_buf();
    lang::set(lang::Language::load(&[&root_buf], lang::DEFAULT));
    let translate = |key: &str| lang::get(key);
    let text_width = |s: &str| HudBatch::text_width(&atlas, s) as i32;
    let mut out = String::new();
    for line in cases.lines() {
        let case: Value = serde_json::from_str(line)?;
        let int = |v: &Value| v.as_i64().unwrap_or(0) as i32;
        let kind = match case["type"].as_str().unwrap_or_default() {
            "crafting" => BookType::Crafting,
            "furnace" => BookType::Furnace,
            "blast_furnace" => BookType::BlastFurnace,
            _ => BookType::Smoker,
        };
        let filtering = case["filtering"].as_bool().unwrap_or(false);
        let tab_list = kind.tabs();
        let tabs: Vec<TabView> = case["tabs"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| {
                let (a, b) = tab_list[t["tab"].as_u64().unwrap_or(0) as usize].icons();
                TabView {
                    icons: (a.to_owned(), b.map(str::to_owned)),
                    selected: t["selected"].as_bool().unwrap_or(false),
                    anim: t["anim"].as_f64().unwrap_or(0.0) as f32,
                }
            })
            .collect();
        let s = &case["search"];
        let search = Search {
            value: s["value"].as_str().unwrap_or_default().to_owned(),
            cursor: s["cursor"].as_u64().unwrap_or(0) as usize,
            highlight: s["highlight"].as_u64().unwrap_or(0) as usize,
            display: s["display"].as_u64().unwrap_or(0) as usize,
            focused: s["focused"].as_bool().unwrap_or(false),
            blink_ms: s["blink"].as_u64().unwrap_or(0),
        };
        // Each collection's recipes and craftable flags.
        let collections: Vec<Vec<(&Entry, bool)>> = case["collections"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| {
                c["recipes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|r| Some((book.entry(r[0].as_str()?)?, r[1].as_bool()?)))
                    .collect()
            })
            .collect();
        let page = case["page"].as_u64().unwrap_or(0) as usize;
        let pages = collections.len().div_ceil(20);
        let anims = case["buttonAnims"].as_array().cloned().unwrap_or_default();
        let shown = &collections[(20 * page).min(collections.len())..];
        let buttons: Vec<ButtonView> = shown
            .iter()
            .take(20)
            .enumerate()
            .map(|(i, c)| ButtonView {
                has_craftable: c.iter().any(|e| e.1),
                entries: c
                    .iter()
                    .filter(|e| e.1 || !filtering)
                    .map(|e| vec![e.0.result.clone()])
                    .collect(),
                anim: anims.get(i).and_then(Value::as_f64).unwrap_or(0.0) as f32,
            })
            .collect();
        let (xo, yo) = Screen {
            book: kind,
            width: int(&case["width"]),
            height: int(&case["height"]),
            narrow: case["narrow"].as_bool().unwrap_or(false),
            mouse: (0, 0),
            index: 0,
            filtering,
            search: &search,
            tabs: &[],
            buttons: &[],
            page,
            pages,
            overlay: None,
            ghost: None,
            translate: &translate,
            text_width: &text_width,
        }
        .origin();
        let overlay = case["overlay"].as_u64().map(|i| {
            let i = i as usize;
            let c = &shown[i];
            let mut recipes: Vec<OverlayRecipe> = c
                .iter()
                .filter(|e| e.1)
                .map(|e| OverlayRecipe {
                    craftable: true,
                    cells: overlay_cells(&book, e.0),
                })
                .collect();
            if !filtering {
                recipes.extend(c.iter().filter(|e| !e.1).map(|e| OverlayRecipe {
                    craftable: false,
                    cells: overlay_cells(&book, e.0),
                }));
            }
            let (x, y) = overlay_origin(
                xo + 11 + 25 * (i as i32 % 5),
                yo + 31 + 25 * (i as i32 / 5),
                recipes.len(),
                xo,
                yo,
            );
            OverlayView { x, y, recipes }
        });
        let ghosts: Vec<GhostView> = case["ghost"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|g| GhostView {
                x: int(&g["x"]),
                y: int(&g["y"]),
                result: g["result"].as_bool().unwrap_or(false),
                items: g["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(|i| ItemStack::new(i, 1))
                    .collect(),
            })
            .collect();
        let screen = Screen {
            book: kind,
            width: int(&case["width"]),
            height: int(&case["height"]),
            narrow: case["narrow"].as_bool().unwrap_or(false),
            mouse: (int(&case["mouse"][0]), int(&case["mouse"][1])),
            index: case["index"].as_u64().unwrap_or(0) as usize,
            filtering,
            search: &search,
            tabs: &tabs,
            buttons: &buttons,
            page,
            pages,
            overlay: overlay.as_ref(),
            ghost: case.get("ghost").map(|_| {
                (
                    int(&case["left"]),
                    int(&case["top"]),
                    case["big"].as_bool().unwrap_or(false),
                    ghosts.as_slice(),
                )
            }),
            translate: &translate,
            text_width: &text_width,
        };
        let draws: Vec<Value> = draw(&screen)
            .into_iter()
            .filter_map(|d| {
                let m: Vec<f32> = d.pose.iter().map(|v| round(*v)).collect();
                let mut o = match d.call {
                    Call::Blit {
                        texture,
                        x,
                        y,
                        w,
                        h,
                        u,
                        v,
                        tw,
                        th,
                    } => json!({"t": "blit", "texture": texture, "x": x, "y": y, "w": w, "h": h,
                                "u": u, "v": v, "tw": tw, "th": th}),
                    Call::Sprite { id, x, y, w, h } => {
                        json!({"t": "sprite", "id": id, "x": x, "y": y, "w": w, "h": h})
                    }
                    Call::Fill {
                        x0,
                        y0,
                        x1,
                        y1,
                        color,
                    } => json!({"t": "fill", "x0": x0, "y0": y0, "x1": x1, "y1": y1, "color": color}),
                    Call::Highlight { x0, y0, x1, y1 } => {
                        json!({"t": "highlight", "x0": x0, "y0": y0, "x1": x1, "y1": y1})
                    }
                    Call::Text {
                        runs: r,
                        x,
                        y,
                        color,
                        shadow,
                    } => json!({"t": "text", "runs": runs(&r), "x": x, "y": y, "color": color, "shadow": shadow}),
                    Call::Item { call, stack, x, y } => {
                        if stack.count == 0 || &*stack.item == "minecraft:air" {
                            return None;
                        }
                        let t = match call {
                            ItemCall::Item => "item",
                            ItemCall::Fake => "fake",
                            ItemCall::Decorations => "decorations",
                        };
                        json!({"t": t, "id": &*stack.item, "count": stack.count, "x": x, "y": y})
                    }
                    Call::Tooltip(lines) => {
                        json!({"t": "tooltip", "lines": lines.iter().map(|l| runs(l)).collect::<Vec<_>>()})
                    }
                    Call::ItemTooltip { stack, more } => {
                        json!({"t": "item_tooltip", "id": &*stack.item, "more": more})
                    }
                    Call::Stratum => json!({"t": "stratum"}),
                };
                o["m"] = json!(m);
                Some(o)
            })
            .collect();
        let _ = writeln!(out, "{}", json!({ "draws": draws }));
    }
    fs::write(args.get(3).ok_or(usage)?, out)?;
    Ok(())
}
