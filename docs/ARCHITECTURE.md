# Architecture

Diagrams use [Mermaid](https://mermaid.js.org) (rendered natively on GitHub).

## 1. System overview

```mermaid
flowchart LR
    subgraph Remote
        API[iptv-org API<br/>streams / channels /<br/>countries / logos JSON]
        SRC[Stream servers<br/>HLS and others]
    end

    subgraph App["Desktop app (single Rust process)"]
        subgraph UIT["UI thread (eframe / egui)"]
            UI[ui: sidebar, list,<br/>player panel, settings]
            APP[App state]
        end
        subgraph BG["Background (tokio runtime)"]
            SYNC[sync service]
        end
        DB[(SQLite<br/>channels, favourites,<br/>settings)]
        MPV[libmpv<br/>own decode threads<br/>+ OpenGL render]
    end

    API -->|HTTPS GET| SYNC
    SYNC -->|upsert| DB
    UI <--> APP
    APP -->|queries| DB
    APP -->|commands: load / volume / stop| MPV
    MPV -->|Referer, User-Agent,<br/>http:// and https://| SRC
    MPV -->|video frames to GL texture| UI
    SYNC -->|events via channel| APP
```

The UI thread never blocks: sync runs on tokio, decoding runs on mpv's own threads, and both
notify the UI with messages plus `ctx.request_repaint()`.

## 2. Sync sequence

```mermaid
sequenceDiagram
    actor U as User
    participant UI as egui UI
    participant S as sync (tokio)
    participant A as iptv-org API
    participant DB as SQLite

    U->>UI: click "Sync"
    UI->>S: spawn sync task
    par fetch in parallel
        S->>A: GET streams.json
        S->>A: GET channels.json
        S->>A: GET countries.json
        S->>A: GET logos.json
    end
    A-->>S: JSON payloads
    S-->>UI: Progress(step) messages
    S->>S: join, drop closed, flag NSFW, pick logo
    S->>DB: one transaction: replace country/channel/stream
    Note over DB: favourites + settings untouched
    S-->>UI: Done{channels, streams, synced_at}
    UI->>DB: reload grouped list
    UI-->>U: country list
```

## 3. Playback sequence

```mermaid
sequenceDiagram
    actor U as User
    participant UI as egui UI
    participant P as Player (libmpv)
    participant S as Stream server

    U->>UI: click channel
    UI->>P: play(stream{url, referrer, user_agent})
    P->>P: set options referrer / user-agent, loadfile url
    P->>S: open stream
    S-->>P: media
    P-->>UI: frames (GL texture) + events
    UI-->>U: video
    U->>UI: move volume slider
    UI->>P: set_property("volume", 0..100)
    alt error / timeout
        P-->>UI: EndFile(error)
        UI-->>U: error overlay, "try next stream"
        UI->>P: play(next stream of same channel)
    end
```

## 4. Player state machine

```mermaid
stateDiagram-v2
    [*] --> Idle
    Idle --> Loading: play(stream)
    Loading --> Playing: first frame (playback-restart)
    Loading --> Error: end-file(error) / timeout
    Playing --> Paused: pause
    Paused --> Playing: resume
    Playing --> Loading: switch channel / retry
    Error --> Loading: retry / next stream
    Playing --> Idle: stop
    Playing --> Error: end-file(error)
```

## 5. Data model (SQLite)

```mermaid
erDiagram
    COUNTRY ||--o{ CHANNEL : has
    CHANNEL ||--o{ STREAM : has
    CHANNEL ||--o| FAVOURITE : "starred as"

    COUNTRY {
        text code PK "ISO 3166-1 alpha-2"
        text name
        text flag
    }
    CHANNEL {
        text id PK "e.g. ABNAfghanistan.us"
        text name
        text country_code FK
        text categories "JSON array"
        int  is_nsfw
        text logo_url
    }
    STREAM {
        int  id PK
        text channel_id FK
        text url
        text quality
        text referrer
        text user_agent
    }
    FAVOURITE {
        text channel_id PK
        text added_at
    }
    SETTING {
        text key PK
        text value "volume, muted, show_nsfw, last_synced_at"
    }
```

`FAVOURITE` and `SETTING` are **never** wiped by a sync. `COUNTRY/CHANNEL/STREAM` are replaced in
one transaction. A favourite whose channel disappeared is shown greyed out ("no longer available").
Schema changes go through numbered migrations stored in `PRAGMA user_version`.

## 6. Modules and UI components

```mermaid
flowchart TD
    main --> App
    App --> ui
    App --> player
    App --> sync
    App --> db

    ui --> TopBar["TopBar: SyncButton, SearchBox, Settings"]
    ui --> SidePanel["SidePanel: tabs All / Favourites"]
    SidePanel --> CountryGroup["CountryGroup (CollapsingHeader)"]
    CountryGroup --> ChannelRow["ChannelRow: logo, name, quality, star"]
    ui --> PlayerPanel
    PlayerPanel --> VideoSurface["VideoSurface (mpv GL texture)"]
    PlayerPanel --> Controls["Controls: play/pause, VolumeControl, StreamPicker"]
    PlayerPanel --> ErrorOverlay

    player --> MpvHandle["MpvHandle: libmpv2 wrapper"]
    player --> GlRender["GlRender: FBO + paint callback"]
    sync --> Fetch["fetch (reqwest)"]
    sync --> Join["join + filter"]
    db --> Migrations
    db --> Queries
```

| Piece | Responsibility |
|---|---|
| `app::App` | Owns state: loaded groups, selected channel, favourites set, player status, message receivers. Implements `eframe::App::update` |
| `ui::sidebar` | Search + tabs + country groups. Only **open** groups render rows, and rows use `ScrollArea::show_rows`, so 15k channels stay fast |
| `ui::player_panel` | Video surface, play/pause, volume slider + mute, stream picker, error overlay |
| `player::MpvHandle` | Init mpv, `play(stream)`, `set_volume`, `toggle_mute`, `stop`; reads events (`playback-restart`, `end-file`) |
| `player::GlRender` | Creates mpv `RenderContext`, renders into an FBO, registers the texture with egui via a paint callback |
| `sync::run` | Fetch 4 files in parallel, join, write DB, emit progress messages |
| `db` | Open DB in the OS data dir (`directories`), migrations, `groups()`, `channels(country, q)`, `set_favourite`, settings get/set |
| `model` | `Country`, `Channel`, `Stream` and `Quality` types with `serde` derives |

Keyboard shortcuts: `Space` play/pause, `↑/↓` volume, `M` mute, `F` fullscreen, `/` focus search.

## 7. Key data structures (sketch)

```rust
struct Channel { id: String, name: String, country: String, logo: Option<String>,
                 categories: Vec<String>, is_nsfw: bool, streams: Vec<Stream> }
struct Stream  { url: String, quality: Option<String>,
                 referrer: Option<String>, user_agent: Option<String> }
enum PlayerStatus { Idle, Loading, Playing, Paused, Error(String) }
enum SyncMsg { Progress(String), Done(SyncSummary), Failed(String) }
```

## 8. Threading

- **UI thread:** egui render loop, reads DB, draws video texture.
- **tokio runtime:** sync tasks; results posted over an `mpsc` channel.
- **mpv threads:** decoding; an event callback wakes the UI.
- SQLite: one connection owned by the app; sync opens its own connection for the write
  transaction (WAL mode so the UI can keep reading).

## 9. Risks

| Risk | Mitigation |
|---|---|
| Embedding mpv video in egui (GL context sharing) | M1 spike first; fallback to a separate mpv window |
| Bundling libmpv on Windows/macOS | Decide packaging in M1; script it in M7 |
| Many dead or slow streams | Timeouts, clear error, next-stream fallback |
| Large lists slow the UI | Collapsed groups, row virtualization, lazy logo loading |
| Wayland/Linux GL quirks | Test early on a Linux VM in M1 |
| Legal/content exposure | NSFW hidden by default, honour closed channels, disclaimer in README/About |
