# Resource packs: what works

Java Edition resource packs load in Horizon: put the folder or `.zip` in `resourcepacks/`, pick it on the Resource Packs
screen (the same screen as vanilla's) or in game, and every loader reloads from it. This page lists everything a
26.3 resource pack can change and whether Horizon honours it, with the proof.

## How it works

`packs.rs` finds the packs (`FolderRepositorySource`: folders and zips, plus the built-in Programmer Art and High
Contrast, which only the asset index holds: preparation copies them, `horizon-prepare packs` repairs older resources),
reads `pack.mcmeta` (format range, overlays, filters) and builds one overlay directory that layers the
selected packs over the prepared vanilla resources, later packs winning. Files vanilla reads as a resource *stack*
(language files, `sounds.json`, font definitions, atlas definitions) are merged layer by layer as vanilla merges them.
Every loader then reads that one tree; `game/packs.rs` (`reload_packs`) reloads all of them in game, as
`Minecraft.reloadResourcePacks` does. Unchanged selections reuse the overlay ([reuse-pack-overlays.md](reuse-pack-overlays.md)).

Vanilla 26.3's reload listeners (`Minecraft.<init>`, `Gui`, `Hud`): languages, textures, shaders, sounds, atlases,
fonts, grass / foliage / dry foliage colour maps, models (block states, block models, item definitions), equipment
assets, paletted textures, entity and block entity renderers, particles, the game renderer (post effects), the level
renderer, clouds, GPU warn list, regional compliance, splashes and waypoint styles. Every row below comes from one
of them.

## The table

**Works**: Horizon reads it from the pack and draws or plays it like vanilla. **Partly**: some of it.
**No**: ignored (the vanilla version stays). Proof: a parity harness in `tools/` (compares with the pinned 26.3
client), a unit test, or the loader reading the overlay (vanilla's files go through the same loader, and the
harness named checks that loader against vanilla).

| Feature | Files | State | Proof |
| --- | --- | --- | --- |
| Pack metadata: description, icon, `min_format`/`max_format`, old `supported_formats`, compatibility warning | `pack.mcmeta`, `pack.png` | works | `test_pack_screen_parity`, `test_pack_selection_parity` |
| Overlays per format | `pack.mcmeta` `overlays` | works | `test_resource_pack_parity` (an overlay for 26.3 applied, an older one not) |
| Filters | `pack.mcmeta` `filter` | works | `test_resource_pack_parity` (`horizon-client-core/src/pattern.rs` regex subset) |
| Folder and zip packs, order, built-in packs | `resourcepacks/` | works | `test_resource_pack_parity`, `test_pack_selection_parity` |
| Server resource packs: the prompt, the server list's Server Resource Packs setting, download (250 MiB, SHA-1, `downloads/` keeps 20), download toasts, a progress screen while the packs reload, applied on top of the player's packs, removed on leaving | the server's `resource_pack_push`/`_pop` | works | `tools/server_pack_e2e.py` (against vanilla's server and Horizon's), unit tests in `server_packs.rs`; `test_server_resource_pack_parity` for Horizon's server |
| Other namespaces | `assets/<namespace>/` | works | `test_resource_pack_parity` |
| Block states: variants, multipart, weights, rotations, uvlock | `blockstates/` | works | `test_blockstate_model_parity`, `test_model_choice_parity`, `test_blockstate_transform_parity` |
| Block models: parents, elements, rotations, faces, cullface, tint, shade, light emission, ambient occlusion | `models/block/` | works | `test_model_resolution_parity`, `test_model_baking_parity`, `test_section_mesh_parity` |
| Item definitions and item models (all model types, display transforms, tints) | `items/`, `models/item/` | works | `test_item_model_parity` |
| Custom entity models (OptiFine/EMF): a `.jem` reshapes a mob's model layer, with its animations and texture; Entity Model Features' files win over OptiFine's | `optifine/cem/`, `emf/cem/` | partly | `mobs::cem` and `mobs::cem_anim` unit tests; no per-bone textures, no random models, no textures under `optifine/` (vanilla's own entity models are code in 26.3, which `test_entity_model_parity` checks) |
| Random entity textures (OptiFine/ETF): numbered variants and their rules (biome, height, name, profession, colour, baby, health, moon, time, weather, size, blocks), chosen per entity from its UUID and rechecked once a second | `optifine/random/`, `optifine/mob/`, `mcpatcher/mob/`, `etf/random/`, `textures/entity/` with a `.properties` | partly | `horizon-optifine` `random` and `random_textures` tests, `mobs::random_textures` tests; no `nbt.*` or ETF's extra conditions (teams, items...), no block entities |
| Connected textures (OptiFine/Continuity): every method (`ctm`, `ctm_compact` with `ctm.N`, `horizontal`, `vertical`, `horizontal+vertical`, `vertical+horizontal`, `top`, `random`, `repeat`, `fixed`, `overlay`, `overlay_ctm`/`_random`/`_repeat`/`_fixed`, and Continuity's `overlay_horizontal`/`_vertical`/`_h+v`/`_v+h`), `matchTiles`/`matchBlocks` and the file-name forms, `tiles` (ranges, names, paths, `<skip>`, `<default>`), `weight`, `faces`, `biomes`, `heights` (and the old `minHeight`/`maxHeight`), `connect`, `connectTiles`/`connectBlocks`, `innerSeams`, `weights`, `randomLoops`, `symmetry`, `linked`, `width`/`height`, `tintIndex`/`tintBlock`, `layer`; Continuity's `orient`, `prioritize`, `optifineOnly` and its stacked-pane culling; tiles a rule picked matched again by another; panes drawn with OptiFine's unmirrored UVs | `optifine/ctm/**/*.properties` (and `mcpatcher/ctm/`) in any namespace, with their tiles | partly | `horizon-optifine` `ctm` and `ctm::connect` tests (the tile tables read off OptiFine's template images), `connected` and `mesher::quarter_tests` tests; `name=` rules are not loaded (the client meshing chunks never receives block entities' custom names, so they could never match); `random` picks from its own fixed hash, so random tiles land in other places than in OptiFine (same weights); no emissive overlays on tiles yet |
| Emissive textures (OptiFine/ETF): `<texture>_e.png` (or the pack's `suffix.emissive`) drawn full bright over mobs, their layers and armour, armour stands, the elytra, the dragon, thrown tridents, block entities and placed blocks | `<texture>_e.png` beside its texture (also random variants' and blink frames'); the suffix from `optifine/emissive.properties`, `etf/emissive.properties` or `textures/emissive.properties` | partly | `horizon-optifine` `emissive` tests, `mobs::render::emissive_tests`, `block_entity_render` and `native_emissive` tests; at most 96 block overlays (native mods keep 32 more), cutout; none on translucent faces, nor on blocks whose texture is pixel-identical to another's (they share a layer); not over items, armour trims, other projectiles, minecarts or end crystals, nor connected-texture tiles |
| Blinking textures (ETF): `<texture>_blink.png` and `_blink2.png`, `blinkLength`/`blinkFrequency` | `textures/entity/`, random variants' folders | works | `horizon-optifine` `blink` tests, `mobs::render` blink test; sleeping villagers keep their eyes shut |
| Textures, any resolution (16x to 512x) | `textures/` | works | atlases stitched from the overlay; Faithful 32x, Default HD 128x and FPBR measured in [reuse-pack-overlays.md](reuse-pack-overlays.md) |
| Texture animation: frames, frame time, frame size | `*.png.mcmeta` `animation` | works | `test_sprite_animation_parity` |
| Texture animation: `interpolate` | `*.png.mcmeta` `animation` | **no** | frames step instead of blending (vanilla's own prismarine, magma, stems, lit furnaces and campfires, command blocks, sculk too): fix in progress |
| Mipmap strategy, alpha cutoff bias | `*.png.mcmeta` `texture` | works | `test_mipmap_parity` |
| `blur` / `clamp` | `*.png.mcmeta` `texture` | partly | vanilla's own files (vignette, pumpkin blur, shadow, glint, nausea) are drawn with their vanilla sampling; a pack that changes the flags is ignored |
| Atlases and sprite sources: `directory`, `single`, `filter`, `unstitch`, `paletted_permutations` | `atlases/` | works | `test_resource_pack_parity` (merged source lists), `atlas_sources.rs` tests |
| GUI sprites | `textures/gui/sprites/` | works | `test_hud_parity`, `test_screen_layout_parity` |
| GUI sprite scaling: `nine_slice` border | `*.png.mcmeta` `gui` | works | `ui.rs` reads the border; vanilla's buttons, sliders and frames use it |
| GUI sprite scaling: `width`/`height`, `stretch_inner`, `tile`, `stretch` | `*.png.mcmeta` `gui` | partly | the border is taken in texels, so HD GUI sprites get thin borders; `stretch_inner` (vanilla's tooltip frame) and `tile` aren't read |
| Fonts: `bitmap`, `space`, `unihex`, `reference` | `font/*.json`, `textures/font/` | works | `test_resource_pack_parity` (merged provider lists) |
| Fonts: `ttf` | `font/*.json` | **no** | no TrueType rasteriser yet |
| Fonts picked by text components (`"font": "..."`, custom fonts) | `font/` | partly | the default font and the enchanting table's `alt` glyphs are drawn; other fonts named by a text component use the default |
| Languages, translations | `lang/*.json` | works | `test_resource_pack_parity` (key by key, as `ClientLanguage`) |
| New languages | `pack.mcmeta` `language` | works | `lang.rs` reads every pack's `language` section into the language list |
| Sounds and sound events (`replace`, weights, streams, event references) | `sounds.json`, `sounds/` | works | `test_resource_pack_parity`, `test_sound_event_parity`, `test_sound_engine_parity` |
| Splashes | `texts/splashes.txt` | works | `title.rs` reads it from the overlay |
| Credits, end poem | `texts/credits.json`, `end.txt`, `postcredits.txt` | **no** | Horizon has no credits screen yet (Options → *Credits & Attribution* opens nothing) |
| Grass, foliage and dry foliage colours | `textures/colormap/` | works | `biome.rs` reads the overlay; `test_section_mesh_parity` checks the tints |
| Entity textures and variants | `textures/entity/` | works | `test_entity_model_parity`, loaded from the overlay |
| Block entities: chests, signs, beds, banners, shields, shulker boxes, decorated pots | `atlases/`, `textures/entity/` | works | `test_block_entity_render_parity` |
| Villager hats | `*.png.mcmeta` `villager` | **no** | the type layer's hat is always drawn; vanilla hides it under a profession hat marked `full`/`partial` |
| Equipment (armour, horse/llama/wolf/nautilus armour, wings, dyeable layers) | `equipment/`, `textures/entity/equipment/` | works | `game/armor_stands.rs`, `mobs/render.rs` read the overlay |
| Armour trims on items | `atlases/items.json`, `textures/trims/items/` | works | `paletted_permutations` (`atlas_sources.rs` tests) |
| Armour trims on worn armour | `textures/trims/entity/`, `textures/palettes/`, `palette` metadata | **no** | worn trims aren't drawn yet (`PalettedTextureManager`) |
| Particles | `particles/`, `textures/particle/` | works | `test_particle_providers_parity` |
| Sky: sun, moon phases, stars, end sky, clouds, rain and snow | `atlases/celestials.json`, `textures/environment/` | works | `test_environment_render_parity`, `test_cloud_render_parity`, `test_weather_render_parity` |
| Title panorama | `textures/gui/title/background/` | works | `gfx/panorama.rs` loads it from the overlay |
| Screen overlays: vignette, pumpkin, powder snow, nausea, underwater, world border | `textures/misc/` | works | `test_world_border_render_parity`, `test_hud_parity`; loaded from the overlay |
| Enchantment glint | `textures/misc/enchanted_glint_*` | works | `item_render/gui.rs`, `hud/cit_glint.rs`, `entity.wgsl` |
| Maps: background, decorations | `textures/map/`, `atlases/map_decorations.json` | works | loaded from the overlay |
| Paintings, mob effect icons | `atlases/paintings.json`, `textures/mob_effect/` | works | loaded from the overlay |
| Locator bar styles | `waypoint_style/` | **no** | the locator bar isn't drawn yet |
| Post effects (spectating a creeper, spider or enderman; glowing outlines) | `post_effect/`, `shaders/post/` | **no** | Horizon's effects are WGSL; a pack's GLSL post chain isn't run |
| Core shaders | `shaders/core/`, `shaders/include/` | partly | `core/text` draws the GUI's text (HUDs moved by its vertex shader, text effects), translated by naga (`horizon-shaders` `core_shader.rs`, `hud/pack_text.rs`); the other core shaders aren't run |
| OptiFine / CIT Resewn custom item textures (`type=item`: `texture`, `model`, `texture.<name>`, `model.<name>`; `type=armor`: `texture`, `texture.<layer>` with the pre-1.21.2 layer names such as `diamond_layer_1`, `leather_layer_2_overlay`; `type=elytra`: `texture`; `type=enchantment` layers on GUI items; `useGlint=false` in `cit.properties`, which removes vanilla's glint from items) | `optifine/cit/`, `mcpatcher/cit/`, `citresewn/cit/` | partly | `item_render/cit.rs`, `cit_armor.rs`, `cit_glint.rs` and `hud/cit_glint.rs` tests; enchantment layers on held, dropped, framed and worn items, and `anim/` glints, come later; special renderers (heads, shields, banners) keep their vanilla look; only players and armour stands wear CIT armour and elytra, other mobs keep vanilla's; worn trims are unchanged; armour textures above 1024x512 (16 times vanilla's 64x32) draw vanilla |
| GPU warn list, regional compliance | `gpu_warnlist.json`, `regional_compliancies.json` | n/a | OpenGL driver warnings and play-time notices for some regions; not used |

Summary: 46 rows; 32 work, 5 partly, 7 don't (interpolated animation, TrueType fonts, credits and end poem,
villager hats, trims on worn armour, locator bar styles, post effects), 2 don't apply. Except the
shaders, the missing pieces are missing vanilla features too, and each vanilla parity fix covers the pack case as well.

## Gaps and their work

| Gap | Size | Where |
| --- | --- | --- |
| Animation `interpolate` | small | fix PR (`gfx.rs` `animate_sprites`) |
| GUI scaling `width`/`height`, `stretch_inner`, `tile`, `stretch` | small | next fix PR (`ui.rs`) |
| Villager hats | small | [#334](https://github.com/horizon-rust/horizon/issues/334) |
| `blur`/`clamp` from packs | small | [#335](https://github.com/horizon-rust/horizon/issues/335) |
| Text component fonts, TrueType fonts | medium | [#336](https://github.com/horizon-rust/horizon/issues/336) |
| Trims on worn armour (paletted textures) | medium | [#337](https://github.com/horizon-rust/horizon/issues/337) |
| Credits and end poem screen | medium | [#338](https://github.com/horizon-rust/horizon/issues/338) |
| Locator bar | medium | [#339](https://github.com/horizon-rust/horizon/issues/339) |
| Post effects, core shaders but GUI text | large | none yet |

## Testing a pack

`tools/test_resource_pack_parity.py` builds small packs (folder and zip, overlays, a filter, textures, languages,
sounds, fonts, atlases), and a Java oracle answers what vanilla's loaders read from them; Horizon's loaders must
give the same answers. Never commit third-party packs: build test packs in the harness or the test.
