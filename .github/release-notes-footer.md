
## Install

Download `crank-@VERSION@-aarch64-darwin.zip` (Apple Silicon), check it against `crank-@VERSION@-aarch64-darwin.zip.sha256`
(`shasum -a 256 -c crank-@VERSION@-aarch64-darwin.zip.sha256`), unzip it and move `crank.app` to `/Applications`.

**This build is not signed with a Developer ID and not notarized** (signing is tracked in #47 and #426), so Gatekeeper
quarantines the download. Clear the flag once, before the first launch:

```sh
xattr -dr com.apple.quarantine /Applications/crank.app
```

**Bluetooth is asked again on every release**: macOS ties the permission to the app's code identity, and an ad-hoc
signature changes with each build. Allow it when prompted (System Settings > Privacy & Security > Bluetooth).
Macs that build from source keep a stable grant with `just bundle`.
