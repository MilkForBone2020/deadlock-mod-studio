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

## Sound Builder

Make sound mods without opening CSDK's tools. Create a sound mod, add the
game sounds it replaces (searched straight from Deadlock's own files, with
preview), and choose your files: MP3, WAV, MP4, M4A, OGG, FLAC or AAC.
**Build mod** then, for each sound:

1. reads the original from the game and measures its loudness and peak,
2. converts your file to the original's channel count and turns it down if
   it's louder (never up), per the ground rules,
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
