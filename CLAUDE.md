# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Wavy is a music and anime player. There is a desktop app (Rust, egui; Windows primary, Linux works) and an Android app (Kotlin, Jetpack Compose, Media3). Code comments and commit messages are in English (so the project is easy to fork). The default UI language is Russian; every user-visible string also has an English translation (see "Translations").

## Commands

Desktop (Cargo workspace: `core` + `native`):

```bash
cargo build --release -p wavy-native          # binary: target/release/wavy-native(.exe)
cargo test --release -p wavy-core             # core unit tests
cargo test --release -p wavy-native           # native unit tests (also builds native/examples)
cargo test --release -p wavy-core sync        # tests whose name contains "sync"
cargo test --release -p wavy-core --test parity  # Rust side of testdata/parity (shared with Android)
cargo check --release --target x86_64-pc-windows-gnu -p wavy-native   # compile-check Windows-only code (tray, hotkeys) from Linux
cargo run --release -p wavy-core --example stream_check -- <url>       # manual checks: core/examples/*, native/examples/*
python3 scripts/mem-check.py                  # Linux: run the built app through idle/scroll/music/video and report its private memory (fails over 100 MB while playing music)
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

Every push to `main` runs `.github/workflows/release.yml`. It runs `wavy-core` tests, then builds `Wavy.exe` on Windows and `ffmpeg.exe` (from `scripts/build-ffmpeg.sh`). On Linux it runs `LogicTest` and builds a signed `Wavy.apk`, and the `memory` job runs `scripts/mem-check.py` and the `wavy-native` tests; if that fails (a test, or over 100 MB while playing music), nothing is released. It then publishes GitHub release `build-<run_number>`. The build number is baked in: `WAVY_BUILD` env for Rust (`native/src/update.rs`) and `-PwavyBuild` for Android (`versionCode`). Both apps compare it with the latest release and update themselves, so a local build (no `WAVY_BUILD`) never auto-updates. Pushing to `main` therefore ships to users.

**Commit messages are user-facing:** after an auto-update the desktop app shows the titles and descriptions of the new commits as "What's new" (`native/src/update.rs`, service lines like `Co-Authored-By:` are dropped). Write them for users: a short title, then what changed for them.

**Git workflow:** commit and push straight to `main`. Don't create feature branches or pull requests, even if the session setup assigns a development branch; the owner wants every change on `main`. Since each push ships a release, run the relevant tests before pushing.

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
  - Embedded HTTP servers: `overlay.rs` (OBS overlay, port 3737, SSE; also the addon API under `/api/v1`, token required) and `wifisync.rs` (phone sync and remote, port 3738, PIN).
  - `romaji.rs`: romaji for Japanese lyrics; kanji readings from `core/assets/romaji-dict.tsv.gz` (common JMdict words, CC BY-SA 4.0 — keep the credit in README). Rebuild it with `python3 scripts/build-romaji-dict.py`; fix wrong lyric readings through its `OVERRIDES`/`STEM_OVERRIDES`, then rebuild.
  - `radio.rs`: internet radio (Icecast/SHOUTcast, `.pls`/`.m3u`): a thread buffers the stream and strips ICY titles. `subsonic.rs`: own music server (Navidrome, Jellyfin…) through the Subsonic API; only a salted token is stored, never the password.
  - Track kinds: `local`, `youtube`, `stream` (other yt-dlp sites), `radio` (url = station link, duration 0), `subsonic` (url = `subsonic:<song id>.<suffix>`; not exported to the phone, which has no server credentials).
  - `addons.rs`: addons are separate processes started from `data_dir/addons/<id>/addon.json`; they get `WAVY_API`/`WAVY_TOKEN` and die with Wavy (job object / `PR_SET_PDEATHSIG`). The API is documented in `docs/addons.md`; keep it backward compatible (addons are written by other people), and add new commands to `API_COMMANDS` and to `api_tick` in `main.rs`.
- **`native`** is the executable:
  - `main.rs` is very large and holds the whole egui app.
  - Also: `theme.rs` (color themes, `th()`), `player.rs`/`fx.rs` (rodio audio and effects), `video.rs` (OpenH264, falls back to the downloaded ffmpeg), `tray.rs`, `hotkeys.rs`, `instance.rs`, `update.rs`, `export.rs`, `recap.rs`.
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
| `data/Lyrics.kt` | `lyrics.rs` | lyrics fetch and parsing (desktop also reads a local `.lrc`/tag lyrics first; Android can't reach files next to a content:// song) |
| `data/Anime.kt` | `anime.rs` | episode name parsing (tests mirror each other) |
| `data/Store.kt` `smart()` | `store.rs` | smart playlists |
| `data/Sync.kt` | `sync.rs` | `wavy-export.json` format |

- **Shared test cases.** `testdata/parity/*.json` holds input → expected output for these algorithms. Both `core/tests/parity.rs` and `ParityLogicTest.kt` read them, so a change on one side that the other lacks fails a test. When you change one of these algorithms, update the JSON and both implementations.
- **Formats.** `Track` JSON fields match between platforms (camelCase, `type` = local | youtube | stream).
- **Exchange format.** `wavy-export.json` is the PC↔phone format, used both as a file and over Wi-Fi. Keep it backward compatible: unknown fields are ignored on both sides, and new fields need defaults.
- **Layout:**
  - `ui/AppState.kt` holds background operations and cross-screen state.
  - `player/PlayerConn.kt` wraps the Media3 controller.
  - YouTube goes through NewPipeExtractor in `yt/`.

## Product constraints

- Keep memory modest, but it no longer has to be minimal: the app should stay comfortable on a weak PC. It used ~45 MB of its own memory while playing; up to roughly 100 MB is fine when a feature needs it. Measure with `scripts/mem-check.py` (private memory, like Task Manager; on a machine without a GPU the in-process software renderer adds ~8 MB). Baseline, 500 tracks with covers: ~58 MB playing music, ~118 MB playing 720p video, ~76 MB back on music afterwards (allocator retention, not a leak: it stays flat over `--cycles`). Still no browser engine, and heavy pieces (yt-dlp, deno, ffmpeg) download on first use.
- Color themes are allowed. The default theme ("dark") is the original black-and-white look. Themes live in `native/src/theme.rs` and `android/.../ui/Theme.kt`; keep their ids and colors in sync. UI code must take colors from the palette (`th().accent`, `th().text`, … on desktop; `Accent`, `TextC`, … on Android) instead of hard-coding them, so every theme, including the light one, stays readable. Video, the in-game overlay and the recap image keep their own fixed colors.
- License is GPL-3.0, required by rustypipe and NewPipeExtractor.
- `CONTEXT.md` is a gitignored local notes file (history, what's verified, what's pending). Read it if it exists, but never commit it.
