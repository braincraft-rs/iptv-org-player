# IPTV-Org Player

A simple, cross-platform desktop IPTV player written in Rust. It syncs the public channel
lists from [iptv-org/iptv](https://github.com/iptv-org/iptv), groups them by country, and lets
you watch a channel, star your favourites and control the volume.

> **Status:** working app (milestones M0-M6 done). Packaged releases (M7) are not available yet. See [docs/PLAN.md](docs/PLAN.md).

## Features (MVP)

- **Sync** channels and stream URLs from the iptv-org public API with one click
- **Browse** channels grouped by country (collapsible, searchable)
- **Play** any channel in the built-in player (HLS and most other live formats)
- **Favourites**: star/unstar channels, dedicated Favourites view
- **Volume** slider and mute
- NSFW channels hidden by default (toggle in settings)
- Works offline for browsing: the synced list is cached locally
- Runs on macOS, Windows and Linux

## Tech stack

Everything is Rust; the only native dependency is libmpv.

| Layer | Choice | Why |
|---|---|---|
| Language | Rust (stable, 2021+ edition) | Single language, memory safe, easy cross-platform builds with `cargo` |
| UI | [eframe / egui](https://github.com/emilk/egui) (glow/OpenGL backend) | Simple, fast, identical on macOS/Windows/Linux, easy to draw video into |
| Player | [libmpv](https://mpv.io) via the `libmpv2` crate | Plays almost any stream; sets `Referer` / `User-Agent` natively, allows `http://` |
| HTTP + JSON | `reqwest` + `serde` | Sync of the iptv-org API |
| Storage | SQLite via `rusqlite` (bundled) | Channel cache, favourites, settings |
| Misc | `tokio`, `anyhow`, `tracing`, `directories` | Async sync, errors, logs, per-OS data paths |

Rationale and alternatives: [docs/PLAN.md](docs/PLAN.md#technology-choices).

## Documentation

- [docs/PLAN.md](docs/PLAN.md) - goals, scope, tech decisions, milestones
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) - diagrams, modules, data model
- [docs/DATA-SOURCES.md](docs/DATA-SOURCES.md) - findings from investigating iptv-org

## Planned project layout

```
iptv-org-player/
├── Cargo.toml
├── src/
│   ├── main.rs          # eframe entry point
│   ├── app.rs           # top-level App state + update loop
│   ├── ui/              # sidebar, channel list, player panel, settings
│   ├── player/          # libmpv wrapper + GL render
│   ├── sync/            # fetch + join iptv-org JSON
│   ├── db/              # rusqlite schema, migrations, queries
│   └── model.rs         # Country, Channel, Stream
├── tests/fixtures/      # small JSON samples
├── docs/
└── README.md  LICENSE  .gitignore
```

## Getting started

Prerequisites: Rust stable (`rustup`) and libmpv.

- macOS: `brew install mpv pkg-config`
- Debian/Ubuntu: `sudo apt install libmpv-dev`
- Windows: mpv dev build (`libmpv-2.dll` + import lib); not tested yet

```bash
cargo run --release
```

On first start click **Sync** to download the channel list (about 30 seconds). Then expand a
country and click a channel to play it. Click the star to add a favourite.

### Keyboard shortcuts

| Key | Action |
|---|---|
| Space | Pause / resume |
| M | Mute |
| Up / Down | Volume +5 / -5 |
| F or double-click video | Fullscreen |
| Esc | Leave fullscreen |
| / | Search channels |

Data (channel cache, favourites, settings) is stored in a SQLite file in your OS data directory
(on macOS: `~/Library/Application Support/rs.BrainCraft.iptv-org-player/iptv.db`).

## Disclaimer

This is a non-profit project made for learning purposes. The author has no affiliation with
iptv-org or with any stream, broadcaster or channel. The app does not host, store or
distribute any video content; it only lists links that are published in the public
[iptv-org/iptv](https://github.com/iptv-org/iptv) repository and plays the streams directly
from their original sources. Channel availability, legality in your jurisdiction and content
are the responsibility of the stream owners and of you as the viewer. Copyright holders
should direct removal requests to the iptv-org repository.

## License

The code in this repository is released under the [MIT License](LICENSE).
Channel data belongs to [iptv-org](https://github.com/iptv-org/iptv) (Unlicense) and is
fetched at runtime, never bundled. libmpv is LGPL/GPL depending on how it is built; see
[docs/PLAN.md](docs/PLAN.md#licensing-notes).
