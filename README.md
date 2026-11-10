# Deadlock Mod Studio (Phase 1)

A personal Windows app for managing my own Deadlock mods: a library of VPKs kept
outside the game, on/off toggles, load order, profiles, safe mode on game
updates, and safe edits to `gameinfo.gi`.

It follows the ground rules in the planning doc:

- The app uses its own folder, `game\citadel\addons_studio`, and its own line in
  `gameinfo.gi`: `Game citadel/addons_studio`. Deadlock Mod Manager's `addons`
  folder and its `Game citadel/addons` line are never touched, so both managers
  can be used side by side. My own mods load first, so they win any clash.
- Only the `SearchPaths` block of `gameinfo.gi` is ever edited, and only that
  one line. ConVars are never touched.
- `gameinfo.gi` is backed up before every edit (the newest 10 are kept).
- Only files the app placed in `addons_studio` are ever removed. Anything you put
  there yourself is left alone, and the app refuses to overwrite it.
- Nothing changes while Deadlock is running.
- Nothing in the game folder changes until you press **Review & apply**, which
  shows every file operation first.

## Updates

Every push to `main` builds the app on GitHub, signs it and publishes a
[Release](../../releases). The installed app checks the latest release when it
opens (and every 6 hours) and offers **Install and restart**. Updates are signed
with a private key kept in this repo's Actions secrets
(`TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`); the app
refuses anything not signed with it. Keep a backup of that key: without it, new
versions can't be signed and you'd have to reinstall by hand.

## Build it on Windows yourself (optional)

One-time setup (about 15 minutes):

1. **Microsoft C++ Build Tools**: install from
   <https://visualstudio.microsoft.com/visual-cpp-build-tools/> and tick
   **Desktop development with C++**.
2. **Rust**: install from <https://rustup.rs> (accept the defaults).
3. **Node.js LTS**: install from <https://nodejs.org>.
4. WebView2 is already part of Windows 10 and 11.

Then, in a terminal inside this folder (for example `D:\Modding\deadlock-mod-studio`):

```
npm install
npm run tauri dev      # run the app in development mode
npm run tauri build    # make an installer
```

The first build compiles a lot of Rust and takes several minutes. Later builds
are quick. The installer ends up in:

```
src-tauri\target\release\bundle\nsis\Deadlock Mod Studio_0.1.0_x64-setup.exe
```

## First test (please do this before real use)

1. Close Deadlock. Open the app and check **Settings** shows
   `D:\Games\SteamLibrary\steamapps\common\Deadlock` as **Found**.
2. Import one small VPK (drag it onto the window or use **Import VPK**).
3. Turn it on, press **Review & apply**, read the list, then apply.
4. Check `game\citadel\addons_studio\pak01_dir.vpk` exists and `gameinfo.gi` has the new
   `Game citadel/addons_studio` line. A backup should be in **Activity → gameinfo backups**.
5. Launch Deadlock and check the mod works. Then mark it **Works on this build**.
6. Close the game, go to **Settings → Restore vanilla**. The addons_studio copy should be
   gone and `gameinfo.gi` back exactly as it was before (Deadlock Mod Manager's line stays).

**Load order is still to confirm.** The top of the list becomes `pak01_dir.vpk`.
Which number wins when two mods change the same file hasn't been tested yet:
enable two mods that share a file (the app flags these) and see which one plays.

## Portraits

**Portraits** replaces a hero's pictures: the card in the HUD's top bar and menus (280 × 380), its low-health and gloating versions (swapped in by the game), the tall icon (120 × 200), the small icon (128 × 128) and the minimap icon (50 × 50). Pick a picture per slot (render at 2× for sharper results); the app crops it to the slot's shape (tall pictures keep their top), writes a flat PSD at the game's path (`panorama/images/heroes/<hero>_card.psd`, as the hero data names it), compiles it with CSDK to the name the game loads (`<hero>_card_psd.vtex_c`) and packs a VPK into the library (`modcore/src/icons.rs`). Slots left empty keep Valve's pictures. **Fill icons from card** crops your card for the small, tall and minimap icons.

## Mod packs

Group mods into a pack (Library → **New pack**, or a mod's **Pack** picker), e.g. a full Seven pack with his model, sounds and HUD. The pack shows as one card with its own switch: off means none of its mods load; on means each mod follows its own switch. A pack's mods always sit together in the load order and move as one. **Export pack…** writes one `.dmpack` file (a zip with `pack.json` and the VPKs); import it with **Import** or by dropping it on the window. Imported packs start switched off.

## Sound Builder

Make sound mods without opening CSDK's tools. Create a sound mod, add the
game sounds it replaces and choose your files. The picker browses by hero and
ability (e.g. *Seven · Storm Cloud (Ult) · Bolt cast*), worked out from the
game's own sound events, hero data and localisation, with preview. Each
ability's voice lines can be muted in the mod. Supported files: MP3, WAV, MP4, M4A, OGG, FLAC or AAC.
**Build mod** then, for each sound:

1. reads the original from the game and measures its loudness and peak,
2. converts your file to the original's channel count and matches it to the
   original's loudness (up or down), then applies your volume setting: from
   −12 dB up to a hard cap of **+3 dB** above the original, with a limiter so
   boosts never clip (ground rules),
3. writes a WAV at the original's path in a `dms_*` addon inside CSDK
   (looping sounds get WAV loop markers),
4. compiles it with `resourcecompiler.exe`,

and packs everything into one VPK in your library. Rebuilding updates the
same library mod. Needs CSDK 13 (set its folder in Settings) and Steam running.

## How safe mode works

On start, every 60 seconds, and whenever the window gets focus, the app reads
Deadlock's build ID from Steam (`steamapps\appmanifest_1422450.acf`). When it
changes, every mod is switched off and the app's files are removed from `addons_studio`
(straight away if the game is closed, otherwise as soon as you apply). A banner
lists what was switched off. Mods marked verified on the old build show
**Needs testing**.

## Where things are kept

`%APPDATA%\app.deadlockmodstudio.local\`

| Path | What |
| --- | --- |
| `library.json` | Mods, order, profiles, settings (`library.json.bak` is the previous save) |
| `library\` | The stored VPKs |
| `backups\` | `gameinfo.gi` backups |
| `logs\actions.log` | Every file the app moved, renamed or edited |
| `models\` | Model Builder projects: the extracted hero, its Blender file and builds' inputs |
| `tools\` | Source 2 Viewer's command-line tool, downloaded on first use |

## Project layout

| Folder | What |
| --- | --- |
| `src-tauri/modcore/` | All file logic (gameinfo.gi, VPK reading, library, sync, Steam). No UI code, fully unit tested |
| `src-tauri/src/lib.rs` | Tauri commands, the game-running check, startup |
| `src/` | React + TypeScript interface |
| `src/mock.ts` | Fake backend so `npm run dev` works in a normal browser for design work |

Run the tests with:

```
cd src-tauri
cargo test -p modcore
```

## Blender add-on

`blender/deadlock_mod_studio/` is a Blender extension (4.2 or newer, tested on 5.0). The app builds it in and installs it with Blender's own `blender --command extension install-file` the first time you use **Edit in Blender**. It:

- imports the extracted hero, keeps a wireframe copy of the original, locks the original bones and saves a work file in the project's `blender` folder
- adds a **Deadlock** tab to the 3D view's sidebar: assign objects to the hero's parts, pose the rig with a few of the hero's animations, and check the model (bones kept and unmoved, every vertex weighted, at most 4 bones per vertex, power-of-two textures, no more than 10% smaller or 40% taller than the original)
- **Attach to head** / **Attach to bone…** makes a new object (a hat, a badge) follow one bone rigidly: it's parented to the rig, gets an Armature modifier and is weighted 1.0 to that bone, stays where you put it whatever the pose, and goes into the part that uses that bone most
- **Send to app** exports the model and a manifest into the project's `return` folder

When you build with your model, the app reads the returned glTF (`modcore/src/glb.rs`), moves each part back into the game's coordinates and writes its geometry into a copy of that part's original DMX (`modcore/src/meshswap.rs`, `modcore/src/dmx.rs`). The skeleton, cloth, hitboxes and settings stay Valve's, every detail level of a changed part uses your mesh, and parts that come back identical keep their original files.

Repainted colour textures: the add-on records each material's colour image at import and sends any that changed. The app rebuilds that material at its original path from the extracted source (`modcore/src/material.rs`): Valve's hero shader and settings stay, the colour slot gets your image (resized to a power of two if needed, alpha dropped), and CSDK compiles it with the model.

New materials: any material that isn't one of the hero's is made from the hero's main material (hero shader, outline, rim light and status effects kept) and written under `<model folder>/dms_materials/`. Its colour is your image, or the Principled Base Color when no image is plugged in; Roughness and Metallic values come across too. Textures drawn for the hero's UV layout (shadowing, metal, glow) are replaced with neutral CSDK defaults or flat colours, glow is turned off, and the hero's vertex-colour tint is ignored.

Preview: the Model Builder's **Preview** button shows the latest model sent from Blender (or the original) in a 3D view with Seven's animations, drawn roughly like the game (`src/preview3d.ts`): banded lighting, the material's solid outline colour, rim light, the shadowing and glow maps, and front faces only, so see-through spots show as they would in game. *Original beside it* adds Valve's model for comparison. Look settings come from the extracted materials (`modcore/src/preview.rs`). It has no map lighting, fog or post-processing.

Fit a model: the add-on's **Fit a model** section brings in a character from another .blend (**Bring in model…**, its meshes and rig) and fits it onto the hero (**Fit to hero**, `blender/deadlock_mod_studio/fit.py`). It works with Rigify, Mixamo and Unity/VRChat humanoid bone names: the model is baked in its rest pose with its modifiers applied, scaled to the hero's height, kept in its own shape by default (torso, neck and head as modelled; arms, legs, hands, feet and fingers only turn at their own joints to the hero's rest pose), or, with *Match the hero's skeleton*, posed so every joint sits on the hero's (limbs take the hero's lengths), and its weights are renamed and merged onto the hero's bones. Bones with no match (a bow, breasts) follow the nearest matched bone rigidly. The hero's own parts it replaces are switched off; weapon parts are kept by default.

Physics bones: when Fit to hero finds bones on their own little rig (a bow and its cables), it adds them to the hero's rig as new bones (`dms_…`) hanging from the bone they followed, and records a cloth chain: the top bone pinned, short branches *wobbly* (hold their shape), long ones *dangling*. The build adds the new bones to the part's DMX skeleton and writes Valve's `ClothChain` nodes (copied from one of the hero's own chains) with settings on Valve's scale (`modcore/src/model.rs`, `joint_params`). For models without such bones, the add-on's **Physics** section has **Make chain from selection**: select the part that should move in Edit Mode, pick the bone it hangs from (it suggests the one those vertices follow most), the number of bones and how it behaves; bones are run through the middle of the selection from the end nearest the parent, following the mesh's edges round curves, and the selection is weighted smoothly along them (`blender/deadlock_mod_studio/physics.py`). Chains can be removed (weights go back to the parent bone). Model Builder's **Physics** panel tunes each group with a preset and stiffness, damping, weight and reach sliders.

At extract, faces Valve uses back to back (double-sided parts) are given their own vertices in the Blender file, because Blender drops a second face on the same three vertices.

You can also install it by hand: zip the folder's two files and use Blender's *Install from Disk*.

## Third-party code and tools

- The binary KeyValues3 reader (`modcore/src/kv3.rs`) is ported from [ValveResourceFormat](https://github.com/ValveResourceFormat/ValveResourceFormat) (MIT licence).
- Model Builder uses Source 2 Viewer's command-line tool (MIT licence, by the ValveResourceFormat contributors). The 20.1 release can't rebuild cloth yet, so `.github/workflows/build-s2v.yml` builds it from their official source at a pinned commit and publishes it as a pre-release of this repo. The app downloads it the first time a hero is extracted, checks its SHA-256 before unpacking, and keeps it in `tools\` in the data folder.
- Building uses Valve's CSDK resource compiler, which you install yourself.
