
## Install

Assets of this release (Apple Silicon, macOS 13 or later):

* `crank-@VERSION@-aarch64-darwin.zip`: `crank.app`
* `crank-@VERSION@-aarch64-darwin.zip.sha256`: its SHA-256
* `crank-@VERSION@-aarch64-darwin.zip.sigstore.json`: its build-provenance attestation

Verify, unzip and move `crank.app` to `/Applications`:

```sh
shasum -a 256 -c crank-@VERSION@-aarch64-darwin.zip.sha256
gh attestation verify crank-@VERSION@-aarch64-darwin.zip -R gerchowl/crank
```

**This build is not signed with a Developer ID and not notarized** (signing is tracked in #47 and #426), so Gatekeeper
refuses to open a zip downloaded with a browser. Clear the quarantine flag once, before the first launch:

```sh
xattr -dr com.apple.quarantine /Applications/crank.app
```

**Bluetooth is asked again on every release**: macOS ties the permission to the app's code identity, and an ad-hoc
signature changes with each build. Allow it when prompted (System Settings > Privacy & Security > Bluetooth).
Macs that build from source keep a stable grant with `just bundle`.
