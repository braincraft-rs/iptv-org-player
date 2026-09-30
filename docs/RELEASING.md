# Releasing

## Status of each platform

| Platform | Package | Tested? |
|---|---|---|
| macOS (Apple Silicon) | `IPTV-Org-Player-<ver>-macos-arm64.zip` containing a self-contained `.app` (libmpv and 47 other dylibs bundled, ad-hoc signed) | **Yes**, built and run locally; plays streams with no Homebrew libraries loaded |
| Linux x86_64 | `.tar.gz` with the binary; needs `libmpv2` installed (`sudo apt install libmpv2`, Ubuntu 24.04+ / Debian 13+) | No, first version in CI only |
| Windows x86_64 | `.zip` with `iptv-org-player.exe` and `libmpv-2.dll` | No, first version in CI only |

Not done yet: Intel macOS build, AppImage/Flatpak for Linux, Windows installer, a `.icns`
icon for the macOS `.app` (the window/Dock icon is set at runtime), Apple notarization.

## Cutting a release

1. Bump `version` in `Cargo.toml`, commit.
2. Regenerate notices if dependencies changed: `python3 scripts/gen-notices.py`.
3. Tag and push:

   ```bash
   git tag v0.1.0
   git push origin main --tags
   ```

4. The `Release` workflow builds all three packages and attaches them to a GitHub release.
   If the Linux or Windows job fails, fix the job and re-run it; the macOS one is independent.

## Building the macOS app locally

```bash
brew install mpv dylibbundler
./scripts/bundle-macos.sh      # output in dist/
```

## macOS Gatekeeper

The app is only ad-hoc signed, not notarized, so macOS will warn on first launch of a
downloaded copy. Right-click the app, choose Open, then confirm. (Notarization needs a paid
Apple Developer account.)

## Licensing reminder

Homebrew's mpv is a GPL build, so the distributed macOS package as a whole falls under the
GPL. The project's own code stays MIT and its source is public. See
[THIRD-PARTY-NOTICES.md](../THIRD-PARTY-NOTICES.md).
