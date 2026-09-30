# Data sources: findings from investigating iptv-org

Measured on 2026-09-30.

## What iptv-org offers

| Source | URL | Size | Notes |
|---|---|---|---|
| Combined M3U | `https://iptv-org.github.io/iptv/index.m3u` | 2.5 MB | All channels |
| M3U by country | `https://iptv-org.github.io/iptv/index.country.m3u` | 2.9 MB | ~12.4k entries, `group-title` = country (187 groups) |
| Per-country M3U | `https://iptv-org.github.io/iptv/countries/hr.m3u` | small | `group-title` = category here |
| **API: streams** | `https://iptv-org.github.io/api/streams.json` | 3.7 MB | 17,766 streams |
| **API: channels** | `https://iptv-org.github.io/api/channels.json` | 7.9 MB | 31,407 channels |
| API: countries | `https://iptv-org.github.io/api/countries.json` | 19 KB | name, ISO code, flag emoji, languages |
| API: logos | `https://iptv-org.github.io/api/logos.json` | - | logo URL per channel |
| API: blocklist | `https://iptv-org.github.io/api/blocklist.json` | - | channels removed for `nsfw` / `dmca` |

All files are served with `access-control-allow-origin: *` and refreshed daily by GitHub Actions.
Streams are plain links to third-party servers; iptv-org hosts no video.

## Chosen source: JSON API

We sync `streams.json`, `channels.json`, `countries.json` and `logos.json` and join them.

```
stream.channel  ->  channel.id          (stream.channel can be null)
channel.country ->  country.code
channel.id      ->  logo.channel        (several logos per channel; pick in_use, largest)
```

Fields:

- **stream**: `channel`, `feed`, `title`, `url`, `quality`, `labels`, `user_agent`, `referrer`
- **channel**: `id`, `name`, `alt_names`, `network`, `country`, `categories`, `is_nsfw`,
  `launched`, `closed`, `replaced_by`, `website`

Join rules:

1. Skip streams with `channel == null` or an unknown channel id (or keep under "Unknown").
2. Skip channels with `closed` set (or `replaced_by`) and `is_nsfw` (unless user enables it).
3. One channel can have several streams (different quality/feed): show one row, let the user
   pick the stream (default: best quality), fall back to the next on error.
4. Group by `channel.country` -> country name + flag from `countries.json`.

## Playback constraints that drive the architecture

| Observation | Count | Consequence |
|---|---|---|
| `http://` stream URLs | 3,626 of 17,766 (~20%) | Blocked as mixed content in a pure `https` web page |
| Streams needing `referrer` | 325 | Browsers cannot set the `Referer` header on media requests |
| Streams needing `user_agent` | 817 | Browsers cannot set `User-Agent` |
| Most streams are `.m3u8` (HLS) | ~96% in M3U | Any HLS-capable player covers nearly all |
| Streams are often dead/slow | many | Need timeout, error state and "try next stream" |

=> A plain browser app could play roughly 75-80% of channels. A native player (libmpv) accepts
`referrer` and `user-agent` as options and opens `http://` URLs directly, so **no proxy is needed**.

## M3U format reference (for reference only; the API is the source of truth)

```
#EXTM3U
#EXTINF:-1 tvg-id="ABNAfghanistan.us@SD" tvg-logo="https://..." group-title="Afghanistan",ABN Afghanistan (540p)
https://mediaserver.abnvideos.com/streams/abnafghanistan.m3u8
```

Some entries add `http-referrer=`, `http-user-agent=` and `#EXTVLCOPT:` lines.

## Legal

iptv-org is released under the Unlicense. The repo lists user-submitted links to publicly
available streams and states it has no control over the targets. This project stores no
streams and bundles no channel data; it fetches them from the public API at runtime.
