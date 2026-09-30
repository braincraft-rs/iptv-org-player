# Implementation plan

## Goals

1. Sync the public iptv-org channel data.
2. Show channels grouped by country.
3. Click a channel to play it.
4. Star favourites.
5. Volume control.
6. Run on macOS, Windows and Linux.

**Non-goals (MVP):** EPG/TV guide, recording, casting, accounts/cloud sync, custom user
playlists, mobile apps.

## Technology choices

| Decision | Choice | Alternatives considered | Reason |
|---|---|---|---|
| Language | **Rust** | C/C++ (Qt), Flutter, .NET, Go, Tauri/TS | One language, memory safe, `cargo` makes cross-platform builds easy |
| UI | **eframe/egui** (glow backend) | Slint, iced, Tauri webview | Quickest to build, same on all OSes, OpenGL backend lets us draw mpv video in the window. Slint is the fallback if egui looks too "tool-like" |
| Player | **libmpv** (`libmpv2` crate) | hls.js in a webview, GStreamer, libVLC | Handles `Referer`/`User-Agent`, plain `http://`, HLS, MPEG-TS and odd live streams with no custom proxy |
| Video output | mpv **render API** (OpenGL) into an FBO, shown as an egui texture | `wid` native-window embedding | Portable across the 3 OSes; `wid` is per-platform |
| HTTP/JSON | `reqwest` (rustls) + `serde` | `ureq` | Async, parallel downloads of the 4 JSON files |
| Storage | **SQLite** (`rusqlite`, bundled) | JSON file | Durable favourites, cache of ~18k streams, simple migrations |
| Logos | `egui_extras` image loaders (HTTP, cached) | manual fetch | Lazy-load only logos that are visible |
| Tooling | `cargo fmt`, `clippy`, `cargo test`, GitHub Actions | - | Standard |

### Why playback dictated the design

About 20% of streams are plain `http://` and ~1,100 need custom `Referer`/`User-Agent` headers
(see [DATA-SOURCES.md](DATA-SOURCES.md)). libmpv takes those as options, so the app needs no
proxy at all.

### Licensing notes

- Your code is MIT. It links dynamically to libmpv (LGPL-2.1+ in default builds; some builds
  are GPL). Ship libmpv as a separate shared library and keep it replaceable.
- Bundled SQLite is public domain. egui, reqwest, serde are MIT/Apache-2.0.
- Add a `THIRD-PARTY-NOTICES` file before the first release.

## Milestones

| # | Milestone | Outcome |
|---|---|---|
| M0 | Scaffold | `cargo` project opens an egui window; fmt/clippy/test in CI |
| M1 | **Playback spike** | Hard-coded channel plays in the window via libmpv on macOS, Windows and Linux; a referrer-protected and an `http://` stream also work |
| M2 | Sync + storage | Fetch the 4 JSON files, join, write SQLite in one transaction; progress in UI |
| M3 | Browse | Country-grouped collapsible list, search, logos, NSFW filter |
| M4 | Player UI | Click-to-play, play/pause, volume + mute (persisted), error overlay, next-stream fallback |
| M5 | Favourites | Star toggle, Favourites view, survives re-sync |
| M6 | Polish | Settings, About/disclaimer, keyboard shortcuts, app icon |
| M7 | Release | Packaged builds (macOS `.app`/dmg, Windows zip/installer, Linux AppImage) via GitHub Actions, libmpv bundled |

M1 comes first on purpose: embedding mpv video in egui and bundling libmpv are the two biggest risks.
Fallback if the GL embedding proves too painful: launch libmpv in its own window (still
controlled from the app), then revisit.

## Testing strategy

- **Unit tests:** join/filter logic, grouping, settings, volume clamping.
- **DB tests:** migrations and favourite-survives-sync using an in-memory SQLite.
- **Fixtures:** small sample JSON in `tests/fixtures/` (never the full data).
- **Manual matrix:** macOS / Windows / Linux x (https stream, http stream, referrer stream, dead stream).

## Open questions / assumptions

- Copyright holder in LICENSE: BrainCraft.rs.
- App name: "IPTV-Org Player" (confirmed).
- Favourite identity is the channel id, so favourites survive re-sync and stream URL changes.
- Sync is manual (button) plus a "stale" hint after 24 h; no background auto-sync in MVP.
- Rust is installed via rustup; libmpv is installed at M0.
