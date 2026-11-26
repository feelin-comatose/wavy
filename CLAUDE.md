# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Wavy is a music and anime player. There is a desktop app (Rust, egui; Windows primary, Linux works) and an Android app (Kotlin, Jetpack Compose, Media3). Code comments, commit messages and the default UI language are Russian; every user-visible string also has an English translation (see "Translations").

## Commands

Desktop (Cargo workspace: `core` + `native`):

```bash
cargo build --release -p wavy-native          # binary: target/release/wavy-native(.exe)
cargo test --release -p wavy-core             # core unit tests
cargo test --release -p wavy-native           # native unit tests (also builds native/examples)
cargo test --release -p wavy-core sync        # tests whose name contains "sync"
cargo check --release --target x86_64-pc-windows-gnu -p wavy-native   # compile-check Windows-only code (tray, hotkeys) from Linux
cargo run --release -p wavy-core --example stream_check -- <url>       # manual checks: core/examples/*, native/examples/*
```

Linux build needs `libasound2-dev`, `libxkbcommon-x11-0`. The release profile is size-optimized (`opt-level = "s"`, LTO, strip, `panic = "abort"`), so release builds take a few minutes. `.cargo/config.toml` statically links the MSVC C runtime.

Android (from `android/`):

```bash
./gradlew testDebugUnitTest --tests '*LogicTest*'      # logic tests (CI runs exactly this)
./gradlew testDebugUnitTest --tests '*ScreenshotTest*' # Robolectric + Roborazzi screenshots → app/build/outputs/roborazzi/*.png
./gradlew compileDebugKotlin
./gradlew assembleRelease -PwavyBuild=N
```

## Release and auto-update

Every push to `main` runs `.github/workflows/release.yml`. It builds `Wavy.exe` on Windows and `ffmpeg.exe` (from `scripts/build-ffmpeg.sh`). On Linux it runs `LogicTest` and builds a signed `Wavy.apk`. It then publishes GitHub release `build-<run_number>`. The build number is baked in: `WAVY_BUILD` env for Rust (`native/src/update.rs`) and `-PwavyBuild` for Android (`versionCode`). Both apps compare it with the latest release and update themselves, so a local build (no `WAVY_BUILD`) never auto-updates. Pushing to `main` therefore ships to users.

## Desktop architecture

- **`core`** is a GUI-free library holding all logic:
  - `store.rs`: the library (`library.json` in `data_dir()`, `%APPDATA%\wavy` or `~/.local/share/wavy`), settings, smart playlists, backups.
  - `youtube.rs`: yt-dlp + rustypipe.
  - `ytaccount.rs`: YouTube Music account via cookies, "My wave".
  - `spotify.rs`: Spotify list → YouTube matching.
  - `stream.rs`: progressive chunked download into the cache.
  - `anime.rs` / `media.rs`: episode filename parsing, MKV tracks and subtitles.
  - `shazam.rs`: audio fingerprint.
  - Integrations: Twitch, Discord, Last.fm, Shikimori, AniSkip, LRCLIB, AnimeThemes.
  - Embedded HTTP servers: `overlay.rs` (OBS overlay, port 3737, SSE) and `wifisync.rs` (phone sync and remote, port 3738, PIN).
- **`native`** is the executable:
  - `main.rs` is very large and holds the whole egui app.
  - Also: `player.rs`/`fx.rs` (rodio audio and effects), `video.rs` (OpenH264, falls back to the downloaded ffmpeg), `tray.rs`, `hotkeys.rs`, `instance.rs`, `update.rs`, `export.rs`, `recap.rs`.
- **UI/event pattern in `main.rs`** (immediate-mode egui):
  - UI code pushes `Act` values into `acts`.
  - `App::apply(Act, ctx)` executes them.
  - Slow work runs on `std::thread` and reports back through the `Bg` enum over an mpsc channel. Messages are drained at the start of each frame in `tick`.
  - Dialogs are the `Modal` enum, rendered in `modal_ui`.
  - To add a feature, usually add an `Act` variant, its `apply` arm, a `Bg` variant if async, and the UI.
- **Settings** change via `Act::Settings(json!({...}))`. `Store::patch_settings` deep-merges camelCase JSON into `Settings` (serde `rename_all = "camelCase"`). Two consequences:
  - Merging never removes a key, so "reset" or "logout" must write explicit empty or default values.
  - New `Settings` fields need a default in `impl Default` (`#[serde(default)]`), so old `library.json` files still load.
- **YouTube playback** (`fetch_audio` in `main.rs`) is a cascade: complete cache file → rustypipe direct URL → yt-dlp streaming into the cache → full download. Playback starts once ~200 KB are available. If a video is gone or 18+, `spotify::find_replacements` substitutes another video under the same track id, so playlists and likes survive. Browser cookies are optional.
- **rustypipe** runs on a single-worker shared tokio runtime (`youtube::rt()`). The YouTube Music account uses a separate rustypipe client with cookies (`YouTube.acc`), so the account never touches playback requests.
- **Single instance:** the app binds `127.0.0.1:37391`. A second launch asks the first to show its window. A launch with `--restarted` (after an update) tells the old process to quit.

## Translations

- **Rust:**
  - `l!("ру", "en")` returns `&'static str`.
  - `lf!("ру {x}", "en {x}", args…)` works like `format!`.
  - Both macros are exported from `core/src/i18n.rs` and available everywhere in `native` through `#[macro_use] extern crate wavy_core`.
  - Strings inside `const` tables (EQ presets, hotkey labels, smart playlists) stay Russian and go through `wavy_core::i18n::tr()` at display time; add new ones to its match.
  - The language is a global flag set from `settings.lang`. Avoid switching it in tests, which run in parallel and assert Russian text.
- **Kotlin:** `L("ру", "en")` in `data/Lang.kt` (a Compose state, so the UI recomposes on change).
- Log messages (`wavy_core::log`) stay Russian only.

## Android app

A separate Kotlin implementation with no shared code. Several algorithms are deliberately duplicated from `core`; change both sides together:

| Kotlin | Rust (`core/src/`) | What |
| --- | --- | --- |
| `yt/Spotify.kt` | `spotify.rs` | matching score, `cleanTitle` |
| `data/Lyrics.kt` | `lyrics.rs` | lyrics fetch and parsing |
| `data/Anime.kt` | `anime.rs` | episode name parsing (tests mirror each other) |
| `data/Store.kt` `smart()` | `store.rs` | smart playlists |
| `data/Sync.kt` | `sync.rs` | `wavy-export.json` format |

- **Formats.** `Track` JSON fields match between platforms (camelCase, `type` = local | youtube | stream).
- **Exchange format.** `wavy-export.json` is the PC↔phone format, used both as a file and over Wi-Fi. Keep it backward compatible: unknown fields are ignored on both sides, and new fields need defaults.
- **Layout:**
  - `ui/AppState.kt` holds background operations and cross-screen state.
  - `player/PlayerConn.kt` wraps the Media3 controller.
  - YouTube goes through NewPipeExtractor in `yt/`.

## Product constraints

- Low memory is a core goal: the target is a weak PC, ~45 MB of its own memory while playing, and no browser engine. Heavy pieces (yt-dlp, deno, ffmpeg) download on first use.
- UI is strictly black and white: no purple or colored accents, no gradients.
- License is GPL-3.0, required by rustypipe and NewPipeExtractor.
- `CONTEXT.md` is a gitignored local notes file (history, what's verified, what's pending). Read it if it exists, but never commit it.
