# Breakmine Launcher (GPUI)

A native desktop launcher for Breakmine: Revived, built with GPUI.

## Run

```bash
cargo build --locked --manifest-path gpui-launcher/game-host/Cargo.toml
cargo run --locked --manifest-path gpui-launcher/Cargo.toml
```

Run these commands from the repository root. The game itself runs in `game-host`, a separate WebView process that uses GTK and WebKitGTK 4.1 on Linux. Use `BM_ROUTE=skins` to open a specific page, `BM_CREATE_TAB=blocks` to open a Mod Maker tab, or `BM_FONT_DEBUG=1` to list the fonts the launcher loaded. Other routes include `home`, `instances`, `mods`, `servers`, `create`, `settings`, and `signin`.

## Pages

| Page | What's there |
|---|---|
| **Home** | 3D player whose head follows the cursor, Hoof's art with the status strip (starting, playing with Show/Stop, offline download, new version), instance rows with a version picker and Play, servers with live status, Mod Maker card. |
| **Instances** | Grouped card grid with search and filters; a page per instance with Overview / Mods / Exports, icon picker, pin, folder, duplicate, edit (name, folder, build, older game version, delete), reset game data, notes. |
| **Mods** | mods.breakmine.com catalog (icons, sort, search, install/update/remove) and the installed list (toggle, remove, update); install a .zip/.jph or an unpacked mod folder. |
| **Servers** | Favorites / Your worlds / Recent with ping bars, player counts and server icons; add/edit/remove, copy address, Join. **Local servers**: install the server runtime, create/edit/start/stop/delete servers, icons, logs, and Cloudflare sharing. |
| **Skins** | 3D preview (drag, scroll to zoom, turn around) with the worn cape, skin library (add, rename, delete, classic/slim), upload to breakmine.com, capes (choose, redeem a code). |
| **Create** | The Mod Maker: blocks (turning 3D block, 16×16 pixel editor, hardness, light, sound, see-through), items, 3×3 recipes with the vanilla ingredient picker, chat commands, ModLoad.js with snippets, game-code patches (.jph import/export), mod image, Test in an instance, Export .zip, Save/Open .bmproject. |
| **Settings** | General (game folder, autostart, session summaries), Game window, Game build (default build, URLs, offline build download, dev builds, advanced URLs), Appearance (Home artwork, reduce motion), Accounts, About. |
| **Sign in** | Over the game's title-screen panorama: Connect with Breakmine (OAuth), password login/register, offline profile. |

## How it's built

- `src/app.rs`: window shell (custom title bar, tabs, inbox, account menu, dialogs, toasts).
- `src/ui/`: design tokens, generated pixel icons, and shared controls.
- `src/views/`: a module for each page.
- `src/skin3d.rs` / `src/pano.rs`: CPU-rendered scenes for the player, Mod Maker blocks, and title panorama. This crate uses `opt-level = 2` in dev builds.
- `src/creator/`: the Mod Maker's generator, projects, pixel editor, and vanilla item IDs.
- `src/game_init.js` and `src/mod_share.rs`: the in-game bridge and temporary mod sharing.
- `src/text_input.rs`: text fields, including multi-line ones for code.
- Fonts in `assets/fonts`: Inter, JetBrains Mono, and Breakmine. Shared game art, icons, and attribution are under the project root's `src/assets/`.

## Data

- Existing instances, mods, skins, projects, older versions, and hosted servers stay in their current folders.
- Settings, servers, the skins list, and the inbox fall back to the legacy `launcher.json` until GPUI saves its own `gpui-launcher.json`.
- Legacy accounts are read-only. New OAuth, password, and offline profiles are stored in `gpui-accounts.json` with mode 0600 on Unix. Tokens are stored unencrypted.
- GPUI stores each instance's worlds and in-game settings in its own `profile-gpui` directory.

## Known limitations

- Discord Rich Presence and "Start in fullscreen" are saved but not applied; "Hide" while playing minimizes instead.
- The pixel editor's keyboard shortcuts (B/E/G/I, Ctrl+Z) and the native colour picker are missing; type a hex colour instead.
- Patch code is saved without syntax validation.

## UI layout checks

Run `cargo test --manifest-path gpui-launcher/Cargo.toml -j 2` after changing
shared dialogs, headers or responsive layouts. The GPUI test platform renders
representative dialogs and Mod Maker headers at constrained window sizes and
checks actual element bounds, including wrapped confirmation text and the
spacing between a project icon and its title. These tests do not replace
checking the real desktop for font rendering, animations or platform-specific
window behaviour.
