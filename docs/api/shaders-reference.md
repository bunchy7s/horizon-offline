# Shader API reference (0.1)

Generated from `crates/horizon-shader-api` (`HORIZON_WRITE_DOCS=1 cargo test -p horizon-shader-api --test reference`
rewrites it; the test fails when it drifts). The API's design and pack format: [shaders.md](shaders.md).

A stage is a function your WGSL defines, with the signature below; Horizon puts the prelude (these structs, the
resources and helpers) before your files and calls the function from its own pass. Your files declare no
bindings and no entry points, and no names starting with `horizon_`.

## Stages

| Stage | In `horizon-shaders.toml` | Your function |
| --- | --- | --- |
| sky | `[sky]` | `fn sky(input: SkyInput) -> vec4<f32>` |
| vertex | `[vertex]` | `fn vertex(input: VertexInput) -> vec3<f32>` |
| material | `[material]` | `fn material(input: MaterialInput) -> Surface` |
| weather | `[weather]` | `fn weather(input: WeatherInput) -> vec4<f32>` |
| post | `[[post]]` | `fn post(input: PostInput) -> vec4<f32>` |

## sky

`fn sky(input: SkyInput) -> vec4<f32>`

### `SkyInput`

One direction of the sky, for `fn sky(input: SkyInput) -> vec4<f32>`. The sun, moon, stars and clouds are drawn over the result.

| Field | Type | Meaning |
| --- | --- | --- |
| `direction` | `vec3<f32>` | Unit view ray, camera-relative world axes. |
| `color` | `vec4<f32>` | Vanilla's sky colour along the ray (sky, fog and sunrise). |

Resources (bind group 3):

| Binding | Declaration | Meaning |
| --- | --- | --- |
| 0 | `var<uniform> frame: Frame` | The frame (`Frame`). |

## vertex

`fn vertex(input: VertexInput) -> vec3<f32>`

### `VertexInput`

One corner of a block's surface, for `fn vertex(input: VertexInput) -> vec3<f32>`, which returns where it is drawn (camera-relative, in blocks; `input.position` where it stays).

| Field | Type | Meaning |
| --- | --- | --- |
| `position` | `vec3<f32>` | The corner, camera-relative, in blocks (`+ frame.camera_position` for the world's). |
| `uv` | `vec2<f32>` | The texture coordinate inside the block's sprite. |
| `material` | `u32` | The `[[materials]]` id of the block's sprite (0 when none matches). |
| `light` | `vec2<f32>` | Block and sky light, 0..1. |
| `tint` | `vec4<f32>` | The vertex colour (biome tint, face shade, ambient occlusion); alpha 1. |
| `face` | `u32` | For a fluid, the face of the block the corner is on (as `MaterialInput.face`), else 0. |
| `fluid` | `u32` | 1 on a fluid's surface (water or lava), else 0. |

Resources (bind group 3):

| Binding | Declaration | Meaning |
| --- | --- | --- |
| 0 | `var<uniform> frame: Frame` | The frame (`Frame`). |

Helpers:

- `fn shadow_position(position: vec3<f32>) -> vec3<f32>`: where the camera-relative point `position` is
  stored in the shadow map, for `textureSampleCompareLevel`.
- `fn shadow_distort(xy: vec2<f32>) -> vec2<f32>`: the map's distortion of a point at shadow clip `xy`.

## material

`fn material(input: MaterialInput) -> Surface`

### `MaterialInput`

One point of a block's surface, for `fn material(input: MaterialInput) -> Surface`.

| Field | Type | Meaning |
| --- | --- | --- |
| `albedo` | `vec4<f32>` | The block texture's colour here (alpha 0..1). |
| `tint` | `vec4<f32>` | The vertex colour: biome tint, face shade and ambient occlusion. |
| `light` | `vec2<f32>` | Block and sky light, 0..1, smoothed over the face. |
| `lightmap` | `vec3<f32>` | Vanilla's light colour for that light this frame. |
| `position` | `vec3<f32>` | The point, camera-relative, in blocks. |
| `uv` | `vec2<f32>` | The texture coordinate inside the block atlas. |
| `translucent` | `bool` | Drawn in the translucent pass (water, stained glass, ice). |
| `tags` | `u32` | The pack's sprite tags this surface's sprite has: bit i for the manifest's i-th `[[tags]]`. |
| `material` | `u32` | The `[[materials]]` id of this surface's sprite (0 when no entry names it). |
| `screen` | `vec2<f32>` | Where the point is on the screen, 0..1 from the top left (`opaque_depth` and `opaque_color` at `screen * frame.resolution`). |
| `face` | `u32` | For a fluid, which face of the block the point is on, `Direction` order: 0 down, 1 up, 2 north, 3 south, 4 west, 5 east. 0 elsewhere. |
| `fluid` | `u32` | 0 not a fluid, 1 water, 2 lava. |
| `emission` | `f32` | How strongly the block itself shines, 0..1, by its kind of light: a redstone torch or a candle 7/15, a torch or lava 14/15, glowstone 1; 0 for most blocks. |
| `normal` | `vec3<f32>` | The surface's geometric normal, a unit vector in world axes, facing the camera. A normal within about 3.6 degrees of an axis is that axis exactly, so distant block faces do not shimmer; flowing water's slopes are steeper and keep theirs. |
| `tangent` | `vec4<f32>` | The surface's tangent: xyz the unit direction in world axes along which `uv.x` grows, w the sign (1 or -1) that makes `cross(normal, tangent.xyz) * tangent.w` the direction `uv.y` grows along (down the texture). With `normal` it turns a normal map's normals into world axes. |
| `colored_light` | `vec3<f32>` | The frame's coloured lights at this point (`[[particle_lights]]` and mods' visual lights), 0..1, to light the surface with as the pack likes; 0 where none reaches. |

### `Surface`

What `material` returns. Vanilla's own is `albedo * tint * vec4(lightmap, 1.0)`; Horizon then applies the alpha test (cutout and translucent) and fog.

| Field | Type | Meaning |
| --- | --- | --- |
| `color` | `vec4<f32>` | The lit colour, 0..1, with its alpha. |

Resources (bind group 3):

| Binding | Declaration | Meaning |
| --- | --- | --- |
| 0 | `var<uniform> frame: Frame` | The frame (`Frame`). |
| 1 | `var reflection: texture_2d<f32>` | The water mirror: the world as a camera mirrored about the horizontal water plane around the camera (`frame.water_plane`) sees it, at half resolution, mapped like the world's picture, so water samples it at `input.screen`. Solid terrain, cut away below the plane, over the vanilla sky gradient: not the pack's `sky` stage, nor the sun, moon and stars. Drawn only for a pack whose material reads it, while `frame.has_reflection` is 1; else a 1×1 blank. |
| 2 | `var reflection_sampler: sampler` | Bilinear filtering, clamped to the edges. |
| 3 | `var shadow_map: texture_depth_2d` | This frame's shadow map: the depth of the solid terrain and entities the light sees, 0 at the light, 1 away from it (and where nothing is), stored at `shadow_position`'s mapping. Valid while `frame.has_shadow` is 1 (the pack asks for it with `[shadow]`), else a 1×1 map that lights everything. |
| 4 | `var shadow_sampler: sampler_comparison` | Depth comparison, bilinear (2×2 percentage-closer filtering): `textureSampleCompareLevel(shadow_map, shadow_sampler, p.xy, p.z)` is 1 where the point is lit, 0 in shadow. |
| 7 | `var shadow_map_translucent: texture_depth_2d` | `shadow_map` with the translucent terrain (water, stained glass, ice) the light passes through drawn in too, read with `shadow_sampler` the same way: a point lit in `shadow_map` but not here lies under such a surface, which `shadow_color` gives the colour of. |
| 8 | `var shadow_color: texture_2d<f32>` | The texture colour times the vertex colour (rgb: the tint, the face's directional shade and ambient occlusion) of the nearest translucent surface the light passes through at each texel of `shadow_map_translucent`, white where there is none; alpha is 1 on a fluid (water) and the texture's alpha, at most 0.99, on anything else. Read it with `textureLoad` at `shadow_position`'s xy times `textureDimensions(shadow_color)`. |
| 6 | `var opaque_depth: texture_depth_2d` | The world's depth before translucent terrain (reversed, like `scene_depth`), for the translucent pass: `textureLoad(opaque_depth, vec2<i32>(input.screen * frame.resolution), 0)` is what lies behind water or glass. Stale while solid terrain draws. |
| 9 | `var block_light_colors: texture_3d<f32>` | The colour of the block light around the camera, from the `[[lights]]` of the light sources nearby: rgb their colours averaged by how near each is, over 4; a how strongly they reach the cell. Read it through `block_light_color`. |
| 10 | `var<uniform> block_light_grid: vec4<f32>` | Where `block_light_colors` lies: xyz its corner, camera-relative, w the side of one of its cells in blocks. |
| 11 | `var opaque_color: texture_2d<f32>` | The world's picture before translucent terrain (sky, solid terrain, entities), the size of `opaque_depth`, for the translucent pass: what a surface refracts. Read it with `textureLoad`. Stale while solid terrain draws; black when the world draws straight to the window (a pack without `[[post]]` passes, at full resolution). |

Helpers:

- `fn block_texture(uv: vec2<f32>) -> vec4<f32>`: the surface's block texture at `uv`, anywhere on the sprite, without the tint.
- `fn block_texture_level(uv: vec2<f32>, lod: f32) -> vec4<f32>`: the same at mip level `lod`.
- `fn block_texture_size() -> vec2<f32>`: its size in texels.
- `fn block_texture_lod() -> f32`: the mip level `block_texture` reads at this point.
- `fn block_pbr_maps() -> bool`: whether the resource packs ship LabPBR maps (`_n`, `_s`).
- `fn block_normal_map(uv: vec2<f32>) -> vec4<f32>`: the surface's LabPBR normal map at `uv` (rg the normal along `input.tangent` and down the texture, b ambient occlusion, a height); (0.5, 0.5, 1, 1) without one.
- `fn block_specular_map(uv: vec2<f32>) -> vec4<f32>`: its LabPBR specular map (r smoothness, g F0 or a metal, b porosity or subsurface, a emission, 1 none); (0, 0, 0, 0) without one.
- `fn block_normal_map_level(uv: vec2<f32>, lod: f32) -> vec4<f32>`, `fn block_specular_map_level(uv: vec2<f32>, lod: f32) -> vec4<f32>`: the same at the block texture's mip level `lod`.
- `fn block_pbr_size() -> vec2<f32>`: the LabPBR maps' size in texels.
- `fn set_surface(normal: vec3<f32>, smoothness: f32, data: f32)`: what the post stage's `surface_at` reads for this point of solid terrain (`data` the pack's own; the face's normal and 0, 0 unset).
- `fn shadow_position(position: vec3<f32>) -> vec3<f32>`: where the camera-relative point `position` is
  stored in the shadow map, for `textureSampleCompareLevel`.
- `fn shadow_distort(xy: vec2<f32>) -> vec2<f32>`: the map's distortion of a point at shadow clip `xy`.

## weather

`fn weather(input: WeatherInput) -> vec4<f32>`

### `WeatherInput`

One point of a rain or snow streak, for `fn weather(input: WeatherInput) -> vec4<f32>`, which returns the colour blended over the world (alpha its opacity).

| Field | Type | Meaning |
| --- | --- | --- |
| `color` | `vec4<f32>` | The rain or snow texture's colour here times the streak's vertex colour (alpha 0.1..1). |
| `light` | `vec2<f32>` | Block and sky light, 0..1. |
| `position` | `vec3<f32>` | The point, camera-relative, in blocks. |
| `snow` | `bool` | A snow streak (rain otherwise). |

Resources (bind group 3):

| Binding | Declaration | Meaning |
| --- | --- | --- |
| 0 | `var<uniform> frame: Frame` | The frame (`Frame`). |

Helpers:

- `fn shadow_position(position: vec3<f32>) -> vec3<f32>`: where the camera-relative point `position` is
  stored in the shadow map, for `textureSampleCompareLevel`.
- `fn shadow_distort(xy: vec2<f32>) -> vec2<f32>`: the map's distortion of a point at shadow clip `xy`.

## post

`fn post(input: PostInput) -> vec4<f32>`

### `PostInput`

One pixel of the picture, for `fn post(input: PostInput) -> vec4<f32>`.

| Field | Type | Meaning |
| --- | --- | --- |
| `uv` | `vec2<f32>` | Where the pixel is, 0..1 from the top left. |
| `pixel` | `vec2<f32>` | The pixel's centre, in pixels from the top left of what the pass draws (the world's size; the window's for the last pass). |
| `color` | `vec4<f32>` | The previous pass's colour here (the world's picture for the first pass). |
| `depth` | `f32` | The world's depth here, reversed: 0 is the sky. |

Resources (bind group 3):

| Binding | Declaration | Meaning |
| --- | --- | --- |
| 0 | `var<uniform> frame: Frame` | The frame (`Frame`). |
| 1 | `var scene_color: texture_2d<f32>` | The world's picture before any pass. |
| 2 | `var scene_depth: texture_depth_2d` | The world's depth, reversed (1 near, 0 far and sky). |
| 3 | `var previous: texture_2d<f32>` | The previous pass's output (`scene_color` for the first pass). |
| 4 | `var linear_sampler: sampler` | Bilinear filtering, clamped to the edges. |
| 5 | `var nearest_sampler: sampler` | Nearest texel, clamped to the edges. |
| 6 | `var shadow_map: texture_depth_2d` | This frame's shadow map: the depth of the solid terrain and entities the light sees, 0 at the light, 1 away from it (and where nothing is), stored at `shadow_position`'s mapping. Valid while `frame.has_shadow` is 1 (the pack asks for it with `[shadow]`), else a 1×1 map that lights everything. |
| 7 | `var shadow_sampler: sampler_comparison` | Depth comparison, bilinear (2×2 percentage-closer filtering): `textureSampleCompareLevel(shadow_map, shadow_sampler, p.xy, p.z)` is 1 where the point is lit, 0 in shadow. |
| 8 | `var history: texture_2d<f32>` | Last frame's input to the last pass (what the pass before it wrote), for temporal passes (TAA, denoising): put such a pass second to last and it reads its own previous output; reproject with `frame.previous_view_projection`. Black on the first frame, after a gap of over half a second, and with a single pass. |
| 9 | `var shadow_map_translucent: texture_depth_2d` | `shadow_map` with the translucent terrain (water, stained glass, ice) the light passes through drawn in too, read with `shadow_sampler` the same way: a point lit in `shadow_map` but not here lies under such a surface, which `shadow_color` gives the colour of. |
| 10 | `var shadow_color: texture_2d<f32>` | The texture colour times the vertex colour (rgb: the tint, the face's directional shade and ambient occlusion) of the nearest translucent surface the light passes through at each texel of `shadow_map_translucent`, white where there is none; alpha is 1 on a fluid (water) and the texture's alpha, at most 0.99, on anything else. Read it with `textureLoad` at `shadow_position`'s xy times `textureDimensions(shadow_color)`. |
| 11 | `var scene_surface: texture_2d<f32>` | What the material stage's `set_surface` stored for the solid terrain at each pixel, unfiltered (`textureLoad`): read it through the prelude's `surface_at`, which knows its layout and where it no longer shows. Blank without a material stage that calls `set_surface`. |

Helpers:

- `fn depth_at(uv: vec2<f32>) -> f32`: the world's depth at `uv`.
- `fn view_position(uv: vec2<f32>, depth: f32) -> vec3<f32>`: the camera-relative position, in blocks, of
  the point at `uv` and `depth`.
- `fn surface_at(texel: vec2<i32>) -> SurfaceData`: the solid terrain's surface at a pixel, as the material stage's `set_surface` left it: `present` (false where the sky, an entity, water, glass or a particle shows instead), `normal` (world axes), `smoothness` and `data`.
- `fn shadow_position(position: vec3<f32>) -> vec3<f32>`: where the camera-relative point `position` is
  stored in the shadow map, for `textureSampleCompareLevel`.
- `fn shadow_distort(xy: vec2<f32>) -> vec2<f32>`: the map's distortion of a point at shadow clip `xy`.

A `[[post]]` entry with `output = "<name>"` writes a target of the pack's own, at its `scale` of the
world's size (1, 0.5 or 0.25), instead of the next pass's `previous`; the prelude of every pass declares
`var <name>: texture_2d<f32>` (group 3 from binding 32), which holds last frame's until written again (a
pass reads a blank one in place of the target it writes).

## The frame

### `Frame`

The frame, as `frame` in every stage. Positions and directions are in world axes relative to the camera (x east, y up, z south), so they stay precise far from the world's origin; depth is reversed (1 near, 0 at the far plane and in the sky).

| Field | Type | Offset | Meaning |
| --- | --- | --- | --- |
| `view` | `mat4x4<f32>` | 0 | Camera-relative world to view space: the camera's rotation. |
| `projection` | `mat4x4<f32>` | 64 | View to clip space (reversed depth, wgpu's clip space: z 0..1). |
| `view_inverse` | `mat4x4<f32>` | 128 | View to camera-relative world space. |
| `projection_inverse` | `mat4x4<f32>` | 192 | Clip to view space. |
| `camera_position` | `vec3<f32>` | 256 | The camera's world position, in blocks (about a millimetre of precision 10 000 blocks out). |
| `time` | `f32` | 268 | Seconds since the pack was loaded, wrapping to 0 every hour. |
| `sun_direction` | `vec3<f32>` | 272 | Unit vector toward the sun (below the horizon at night). |
| `day_time` | `f32` | 284 | The time of day, 0..1: 0 sunrise, 0.25 noon, 0.5 sunset, 0.75 midnight. |
| `moon_direction` | `vec3<f32>` | 288 | Unit vector toward the moon. |
| `game_time` | `f32` | 300 | Vanilla's GameTime: (game time % 24000 + partial tick) / 24000, 0..1. |
| `sky_color` | `vec3<f32>` | 304 | Vanilla's sky colour this frame, 0..1. |
| `rain` | `f32` | 316 | Rain strength, 0..1. |
| `fog_color` | `vec3<f32>` | 320 | Vanilla's fog colour this frame (also the clear colour), 0..1. |
| `thunder` | `f32` | 332 | Thunder strength, 0..1. |
| `resolution` | `vec2<f32>` | 336 | The size of the world's picture, in pixels (the resolution scale applied). |
| `eye_light` | `vec2<f32>` | 344 | Block and sky light at the camera, 0..1 (level / 15). |
| `fog_start` | `f32` | 352 | Where vanilla's fog starts, in blocks from the camera. |
| `fog_end` | `f32` | 356 | Where vanilla's fog is full, in blocks. |
| `render_distance` | `f32` | 360 | The render distance, in blocks. |
| `far` | `f32` | 364 | The far plane, in blocks. |
| `night_vision` | `f32` | 368 | Night vision strength, 0..1. |
| `blindness` | `f32` | 372 | Blindness strength, 0..1. |
| `darkness` | `f32` | 376 | The darkness effect's strength, 0..1. |
| `brightness` | `f32` | 380 | The Brightness option, 0..1 (Moody to Bright). |
| `eye_in` | `u32` | 384 | What the camera is in: 0 air, 1 water, 2 lava, 3 powder snow. |
| `dimension` | `i32` | 388 | 0 the Overworld, -1 the Nether, 1 the End, 2 any other dimension. |
| `frame_index` | `u32` | 392 | Frames drawn since the pack was loaded (wraps). |
| `hide_gui` | `u32` | 396 | 1 while F1 hides the HUD, else 0. |
| `precipitation` | `u32` | 400 | What falls in the biome at the camera while it rains: 0 nothing, 1 rain, 2 snow. |
| `temperature` | `f32` | 404 | The biome at the camera's temperature (vanilla's, -0.7..2.0; it snows below 0.15). |
| `downfall` | `f32` | 408 | The biome at the camera's downfall, 0..1. |
| `wetness` | `f32` | 412 | `rain`, smoothed: rises over about 30 seconds and dries over about 10, 0..1. |
| `previous_view_projection` | `mat4x4<f32>` | 416 | Last frame's camera-relative world to clip space (its `projection * view`); a point at camera-relative `p` now was at `p + camera_motion` relative to last frame's camera. This frame's on the first frame and after a gap of over half a second (a pause, a world load), when last frame's camera says nothing about this one. |
| `camera_motion` | `vec3<f32>` | 480 | How far the camera moved since last frame, in blocks (0 after such a gap). |
| `delta_time` | `f32` | 492 | Seconds since last frame (0 on the first and after such a gap). |
| `eye_light_smooth` | `vec2<f32>` | 496 | `eye_light` eased with a half-life of 2 seconds: it follows the light levels' steps without stepping itself (for exposure and indoor fog). |
| `in_water` | `f32` | 504 | 1 while the camera is in water (`eye_in` 1), eased with a half-life of a quarter second in and out. |
| `in_lava` | `f32` | 508 | The same for lava (`eye_in` 2). |
| `water_plane` | `f32` | 512 | The y of the horizontal water plane `reflection` mirrors the world about, in world blocks, while `has_reflection` is 1. |
| `has_reflection` | `u32` | 516 | 1 while the material stage's `reflection` holds this frame's water mirror, else 0. |
| `reflection_resolution` | `vec2<f32>` | 520 | The size of `reflection`, in pixels: half the world's picture. |
| `mods` | `array<vec4<f32>, 8>` | 528 | Eight values a native mod hands its own pack each frame (`native_shader_params`, docs/native-mods.md): what they mean is that mod's, such as where and when something happened in the world. All 0 when no mod sets them. |
| `light_direction` | `vec3<f32>` | 656 | Unit vector toward the body casting this frame's shadows: the sun by day, the moon by night. |
| `has_shadow` | `u32` | 668 | 1 while `shadow_map` holds this frame's shadow map, else 0. |
| `shadow_view_proj` | `mat4x4<f32>` | 672 | Camera-relative world to the shadow camera's clip space, before the distortion (orthographic, looking along `-light_direction`; z 0 at the light, 1 away from it). |
| `shadow_distance` | `f32` | 736 | Half the side of the square the shadow map covers, in blocks (the manifest's `[shadow] distance`); 0 without a shadow map. |
| `shadow_distortion` | `f32` | 740 | How strongly the shadow map is distorted toward its centre, 0..1: a point at shadow clip `xy` is stored at `xy / (length(xy) * k + 1 - k)`, so texels near the camera cover less ground (`shadow_position` applies it). |
| `shadow_resolution` | `f32` | 744 | The shadow map's side, in texels; 0 without a shadow map. |
| `shadow_center_texel` | `f32` | 748 | The ground one shadow texel covers at the map's centre, in blocks (for a bias that scales with it); 0 without a shadow map. |

## Options

Each `[[options]]` entry sets the WGSL `override` of the same name when the pipelines are built:

| `type` | Keys | The `override` |
| --- | --- | --- |
| `bool` | `default` | `override <id>: bool` |
| `float` | `default`, `min`, `max`, `step` | `override <id>: f32` |
| `int` | `default`, `min`, `max` | `override <id>: i32` |
| `choice` | `values`, `default` (one of them) | `override <id>: u32`, the value's index |

## Limits

- 8 post passes, 64 options, `budget.gpu_ms` up to 16.
- 256 KiB per WGSL file, 8 MiB per pack.
- Per stage: 20000 naga expressions over all its functions, 64 texture reads (samples,
  gathers and loads, the prelude's included).

## The check

A pack runs only after it passes `horizon_shader_api::check` (the game when it loads a pack, the launcher before it
installs one, the registry's CI): each stage is parsed and validated by naga, checked against its signature and
the rules above, and translated for Vulkan (SPIR-V), Metal and DirectX 12 (HLSL) with its options at their
defaults, minimums and maximums. Every option must be an `override` of its type in some stage; an `override`
that is no option needs a default.
