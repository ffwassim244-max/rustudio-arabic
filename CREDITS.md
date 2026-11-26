# Credits: original files used

## Project Authors & Upstream

- **[rippelz/rustudio](https://github.com/rippelz/rustudio)**: Creator of the original rustudio project and maintainer of the PC (Linux, Windows, macOS) version.
- **[TouseefX/rustudio](https://github.com/TouseefX/rustudio)**: Developer and core contributor to rustudio.

---

rustudio is the person's own editor for their Roblox places. Nothing from
Roblox Studio is copied into this repository or compiled into the program. The
files below are downloaded from Roblox's own CDN into the app data directory on first launch and read at run time only,
through the content cache (`RUSTUDIO_CONTENT_DIR` points it elsewhere).

| File in the game's folder | Used for |
|---|---|
| `content/fonts/*.mesh` (e.g. `content/fonts/torso.mesh`, `leftarm.mesh`, `rightarm.mesh`, `sword.mesh`, `rocketlauncher.mesh`) | `rbxasset://fonts/…` meshes that places reference (SpecialMesh / MeshPart) |
| `content/textures/*` (e.g. `content/textures/face.png`, `SwordTexture.png`, `rocketlaunchertex.png`, `water/normal_21.dds`) | `rbxasset://textures/…` images on parts, decals and meshes |
| `content/sounds/*` | `rbxasset://sounds/…` sounds played by Sound instances |
| `content/fonts/families/*.json` | named by `Font` / `FontFace` values (`rbxasset://fonts/families/…`): which face file holds each weight and style |
| `content/fonts/*.ttf`, `content/fonts/*.otf` (e.g. `Oswald-Regular.ttf`, `Sarpanch-Regular.ttf`, `BuilderExtended-Bold.otf`, `Montserrat-Bold.ttf`) | the text of TextLabels, TextButtons and TextBoxes in GUIs, drawn with the same font files Studio uses |
| `content/fonts/BuilderSans-Regular.otf`, `SourceSansPro-Regular.ttf`, `NotoSans*UI-Regular.ttf`, `NotoNaskhArabicUI-Regular.ttf` | fallback glyphs for GUI text whose font lacks a character (e.g. "▶" in Sarpanch), after the text's own font |

Read straight from the Studio install or downloaded on first run from Roblox's deployment CDN:

| File in the game's folder | Used for |
|---|---|
| `ExtraContent/scripts/PlayerScripts/StarterPlayerScripts/PlayerScriptsLoader.lua`, `PlayerModule.module.lua` and the `PlayerModule.module/` folder (CameraModule, ControlModule, CommonUtils) | Roblox's default player controller, put into StarterPlayerScripts at Play as Studio does |
| `ExtraContent/scripts/PlayerScripts/StarterPlayerScriptsCommon/RbxCharacterSounds.lua` (+ `RbxCharacterSounds/AtomicBinding.lua`) | the default character sounds script |
| `content/avatar/scripts/humanoidHealthRegenScript.rbxmx`, `humanoidAnimateR15_DEPRECATED_20260813.rbxm`, `humanoidAnimatePlayEmote.rbxm` | the Health and Animate scripts each character gets |
| `PlatformContent/pc/textures/studs.dds` | legacy surfaces (Studs, Inlet, Weld, Glue, Universal) on parts |
| `PlatformContent/pc/terrain/materials.json` | default terrain colours (`base_color`) |
| `content/avatar/characterR15DynamicHeadV2.rbxm`, `content/avatar/defaultDynamicHead.rbxm` | Roblox's default R15 body for characters in Play (body-part MeshParts, WrapTarget cages, attachments, Motor6Ds) and its head |
| `content/avatar/character.rbxm`, `content/avatar/heads/head.mesh` | Roblox's default R6 body, and the mesh drawn for SpecialMesh MeshType Head |
| `ReflectionMetadata.xml` | the Insert Object popup's categories (ClassCategory) and the Explorer's sibling order (ExplorerOrder) |
| `content/fonts/BuilderSans-Regular.otf`, `BuilderSans-Bold.otf` (through the content link) | the editor's own text, as Studio's panels and ribbon use |

Studio's rules and default values (camera speed settings, the fly speed-up
rule, wheel zoom, snap increments) are written into rustudio's own code; no
Studio code is copied. Values measured from Studio (listed in
`docs/MECHANICS.md` and `mechanics.json`): camera speed, wheel step, field of view, snap increments, part defaults, theme colours, Studio's settings in an offline Studio's `appdata/Roblox/GlobalSettings_13.xml`, and Output messages seen while playing og.rbxl and Place A in an offline Studio.

Downloaded at run time from Roblox's public asset delivery (no login; `docs/MATERIALS.md`) and cached: the material texture packs Studio uses and their images, the default animations Roblox's Animate scripts ask for, the body part meshes and cages the body files above point at, and any other public asset a place uses.

With the person's own Roblox session (`docs/AUTH.md`; signed in only), the same endpoint also gives assets Roblox serves to signed-in accounts: the current dynamic head of the R15 body file (mesh, cage, face), the person's avatar items (clothing, accessories, layered clothing), models a place loads with InsertService, and the person's own places.

## Bundled third-party art

| What | Licence | Used for |
|---|---|---|
| [Tabler Icons](https://tabler.io/icons) 3.50.0 (`app/assets/tabler/`: `tabler-icons.ttf`, `tabler-icons-filled.ttf`, the name → code point lists made from its CSS, and its `LICENSE`), © 2020-2026 Paweł Kuna | MIT | every ribbon, menu, top-row and class icon (src/ti.rs, src/class_icons.rs); no Roblox icon is copied |

## Vendored and patched crates

Under `vendor/`, each with its upstream licence file and a note of what rustudio changed and why:

| Crate | Licence | Change |
|---|---|---|
| `luau0-src` (Luau) | MIT | host hooks, incl. `lua_hostcall` (`vendor/luau0-src/PATCHES.md`) |
| `mlua` 0.12.2 | MIT | `Lua::with_current_state` for the direct Instance bridge (`vendor/mlua/PATCHES.md`) |
| `rbx_dom_weak` 4.2.0 (rbx-dom) | MIT | bulk child detach and end-first child removal (`vendor/rbx_dom_weak/PATCHES.md`) |
| `rustls` 0.23.46 | Apache-2.0 / ISC / MIT | `vendor/rustls/RUSTUDIO-PATCH.md` |
| `egui` 0.36.2 | MIT / Apache-2.0 | click selection on press (see the root `Cargo.toml`) |
| `mlua-sys` | MIT | see the root `Cargo.toml` |
