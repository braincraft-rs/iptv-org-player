#!/usr/bin/env python3
"""Regenerate THIRD-PARTY-NOTICES.md from `cargo metadata`.

Run from the repo root after changing dependencies:  python3 scripts/gen-notices.py
"""
import collections
import json
import subprocess

meta = json.loads(
    subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--locked"])
)
pkgs = {p["id"]: p for p in meta["packages"]}
nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
root = meta["resolve"]["root"]

# Crates reachable through normal (non-dev, non-build) dependencies.
seen, stack = set(), [root]
while stack:
    i = stack.pop()
    if i in seen:
        continue
    seen.add(i)
    for d in nodes[i]["deps"]:
        if any(k["kind"] in (None, "normal") for k in d["dep_kinds"]):
            stack.append(d["pkg"])
seen.discard(root)

rows = sorted(
    (pkgs[i]["name"], pkgs[i]["version"], pkgs[i].get("license") or "see crate")
    for i in seen
)
summary = collections.Counter(r[2] for r in rows)

out = [
    "# Third-party notices",
    "",
    "IPTV-Org Player's own code is MIT-licensed (see `LICENSE`). It is built from the",
    "components below, each under its own licence. This file is generated from",
    "`cargo metadata` by `scripts/gen-notices.py`; regenerate it when dependencies change.",
    "",
    "## Native libraries (bundled in release packages)",
    "",
    "| Component | Licence | Notes |",
    "|---|---|---|",
    "| [libmpv / mpv](https://mpv.io) | LGPL-2.1-or-later, or GPL-2.0-or-later for GPL builds | "
    "The macOS package bundles Homebrew's build, which is a **GPL build**. "
    "Source: https://github.com/mpv-player/mpv |",
    "| [FFmpeg](https://ffmpeg.org) (libav*) | LGPL-2.1-or-later / GPL-2.0-or-later depending on build | "
    "Used by libmpv. Source: https://ffmpeg.org/download.html |",
    "| libass, libplacebo, zimg, lcms2 and other libmpv dependencies | LGPL, GPL, ISC, BSD, MIT, Zlib | "
    "See each project's source repository. |",
    "",
    "Because the bundled libmpv is a GPL build, a distributed macOS package as a whole is",
    "subject to the GPL. This project's source code is public and MIT-licensed (which is",
    "GPL-compatible), and the corresponding source of the bundled libraries is available at",
    "the links above. To use an LGPL-only libmpv instead, replace the libraries in",
    "`IPTV-Org Player.app/Contents/Frameworks/` or build against an LGPL build of mpv.",
    "",
    "## Channel data",
    "",
    "Channel lists are fetched at runtime from the public [iptv-org](https://github.com/iptv-org/iptv) API",
    "(Unlicense / public domain) and are not bundled.",
    "",
    "## Rust crates",
    "",
    f"{len(rows)} crates. Licence summary:",
    "",
]
out += [f"- {c} x {lic}" for lic, c in summary.most_common()]
out += ["", "| Crate | Version | Licence |", "|---|---|---|"]
out += [f"| {n} | {v} | {lic} |" for n, v, lic in rows]

with open("THIRD-PARTY-NOTICES.md", "w") as f:
    f.write("\n".join(out) + "\n")
print(f"wrote THIRD-PARTY-NOTICES.md ({len(rows)} crates)")
