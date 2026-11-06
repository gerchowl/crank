# Releasing crank

Adopted from devkit's release flow (#428) and amended by ADR-0001 §9.6 (#485): `dev` is cut into `release/<version>`,
a draft PR carries it back, the owner merges, and CI runs the macOS gate, builds `crank.app`, attests it and publishes
the GitHub release. Agents never push or merge `main`, never merge release PRs and never publish. The workflows use
`GITHUB_TOKEN` only; there are no secrets and no PAT.

## Versions

* `X.Y.Z-rc.N` (N ≥ 1) is release candidate N of `X.Y.Z`. It is published as a GitHub **pre-release**.
* `X.Y.Z` is a final release. **Final releases are refused until Developer ID signing and notarization land** (#47,
  #426): Prepare Release refuses one (except as a dry run) and Release refuses to build one. Until then every build that
  reaches a test Mac is a release candidate.

Tags, release names, `Cargo.toml` and the other version sources carry the version as written. macOS needs numeric bundle
versions, so `app/src-tauri/Info.plist` carries a mapping:

| Version | `CFBundleShortVersionString` | `CFBundleVersion` |
|---|---|---|
| `X.Y.Z-rc.N` | `X.Y.Z` | `X·1000000 + Y·10000 + Z·100 + N` |
| `X.Y.Z` | `X.Y.Z` | `X·1000000 + Y·10000 + Z·100 + 99` |

For example 0.2.0-rc.1 → 20001, 0.2.0-rc.2 → 20002, 0.2.0 → 20099. The integer orders like semver, so a later build is
always newer to macOS; that bounds Y and Z to 0–99 and N to 1–98, and anything else is refused.

## Cutting a release candidate (owner)

1. Make sure `## Unreleased` in [CHANGELOG.md](CHANGELOG.md) lists the changes and `dev` is green.
2. Run **Prepare Release** (Actions, `workflow_dispatch`, on `dev`) with the version `X.Y.Z-rc.N`. It refuses a malformed
   version, one that is not later than `dev`'s current version, an existing tag, another `release/*` branch, and a `dev`
   that is behind `main`. It pushes `release/X.Y.Z-rc.N` (one commit: the version bump; the changelog stays in
   `## Unreleased`), dispatches CI on it (Linux **and** macOS) and opens a draft PR into **`dev`**. `dry-run` stops before
   pushing.
3. When `CI Summary` is green on the PR, mark it ready and **merge it into `dev` with a merge commit**. If `dev` moved
   meanwhile, update the branch first (dev requires up-to-date branches; the update re-runs CI with macOS).
4. **Release** runs on that merge: it runs `nix flake check -L` on macOS for the merge commit, builds `.#crank-app`, zips it,
   writes the checksum, attests the zip, verifies the attestation, and **publishes the pre-release** `X.Y.Z-rc.N` (the tag
   is created at the merge commit) with the notes from `## Unreleased`. It then deletes `release/X.Y.Z-rc.N`.
5. Test the pre-release (below). Fixes go to `dev` as ordinary PRs; cut `X.Y.Z-rc.N+1` from `dev` again.

`dev` carries the version of the last candidate, so no main→dev sync is needed for candidates.

**If Release fails after the merge** (for example the macOS runner was not acquired, #480): re-run the failed jobs of that
run. If the gate found a real defect, fix it on `dev`, run **Abandon Release** with `X.Y.Z-rc.N` (it deletes the branch;
there is no release to delete), and prepare `X.Y.Z-rc.N+1`.

## Final releases (after #426)

The final path keeps #428's shape: Prepare Release with `X.Y.Z` freezes `## Unreleased` into `## [X.Y.Z]`, pushes
`release/X.Y.Z` and opens a draft PR into `main`; the owner merges it with a merge commit and Release builds it. #426 adds
Developer ID signing, notarization and stapling there, and brings back the main→dev sync PR (failing loudly when it cannot
be opened, #439). Until then both workflows refuse it, and a dry run (`dry-run: true`) is the only way to exercise it.

## Abandoning

To back out before a release is published, run **Abandon Release** with the version: it deletes a draft release, closes
the open release PR (into `dev` for a candidate, `main` for a final release) and deletes `release/<version>`. A published
release, candidate or final, cannot be abandoned (its tag is permanent): release the next version. If the release PR
was already merged, revert its commit by PR.

## Assets (a contract: g-fleet#1203 consumes them)

Every published release carries exactly these assets, `<version>` being the tag (`0.2.0-rc.1`):

| Asset | Content |
|---|---|
| `crank-<version>-aarch64-darwin.zip` | `crank.app`, `ditto -c -k --keepParent` (Apple Silicon, macOS 13+) |
| `crank-<version>-aarch64-darwin.zip.sha256` | one line of `shasum -a 256`: `<64 hex>  crank-<version>-aarch64-darwin.zip` |
| `crank-<version>-aarch64-darwin.zip.sigstore.json` | the Sigstore bundle of the SLSA build-provenance attestation of the zip (`actions/attest-build-provenance`) |

The attestation is also stored in the repository's attestations, so `gh attestation verify` finds it online. Its signer
workflow is `gerchowl/crank/.github/workflows/release.yml`, its subject the zip's SHA-256, its source ref
`refs/heads/dev` for a candidate (`refs/heads/main` for a final release) and its runner GitHub-hosted. An installer
should pin all of these (`--signer-workflow`, `--source-ref`, `--deny-self-hosted-runners`), so that a dry-run
attestation from another branch never vouches for a download. The pins do not tell a dry run **dispatched on `dev`**
from a real release: both are `release.yml` on `refs/heads/dev`. The attestation proves where bytes were built, not that
they were released, so an installer must consume only the assets of a published release (`gh release download`), never
a workflow artifact. GitHub's
`/releases/latest` ignores pre-releases: to find the newest build, list releases (`gh release list -R gerchowl/crank
--limit 1 --json tagName,isPrerelease`, or `GET /repos/gerchowl/crank/releases?per_page=1`).

## Verifying and installing on a Mac

```sh
V=0.2.0-rc.1
gh release download "$V" -R gerchowl/crank -p "crank-$V-aarch64-darwin.zip*"
shasum -a 256 -c "crank-$V-aarch64-darwin.zip.sha256"
gh attestation verify "crank-$V-aarch64-darwin.zip" -R gerchowl/crank \
  --signer-workflow gerchowl/crank/.github/workflows/release.yml --source-ref refs/heads/dev --deny-self-hosted-runners
# offline, against the attached bundle:
gh attestation verify "crank-$V-aarch64-darwin.zip" -R gerchowl/crank --bundle "crank-$V-aarch64-darwin.zip.sigstore.json"
ditto -x -k "crank-$V-aarch64-darwin.zip" . && mv crank.app /Applications/
```

**Signing and Gatekeeper.** The zip holds an unsigned bundle whose Mach-O files carry only the ad-hoc signatures the Nix
build writes (ADR-0001 §9.3, #413). CI adds no signature, and there is no Developer ID before #426.

* A zip downloaded **with a browser** is quarantined (`com.apple.quarantine`), and Gatekeeper refuses to open the app
  ("cannot be verified"). Clear the flag once before the first launch, `xattr -dr com.apple.quarantine
  /Applications/crank.app`, or allow it under System Settings > Privacy & Security > Open Anyway.
* `gh release download` and `curl` set no quarantine flag, so the app opens directly.
* Bluetooth: an ad-hoc signature changes with every build, so macOS asks for Bluetooth again for each release. Fleet Macs
  avoid that by re-signing with the local `nix-tcc` identity: g-fleet's installer (g-fleet#1203) after verifying, or
  `just bundle` from source (ADR-0001 §9.3).

The release notes carry the same steps (`.github/release-notes-footer.md`).

## Dry run (agents may run it)

**Release** dispatched by hand on any branch is a dry run: the full macOS gate, the zip, the checksum, the attestation and
its verification, uploaded as the workflow artifact `crank-<version>-dry-run` (14 days). It creates no tag and no
release. The optional `version` input rehearses another version (for example `0.2.0-rc.1`): the runner bumps the sources
to it before building, so the rc mapping is exercised in a real bundle. The attestation of a dry run is real (it is in the
repository's attestations and the public Sigstore log) and names the branch it ran on; a dry run on `dev` names `dev`,
like a release (see Assets: only release assets are installable).

## CI and the macOS gate

`CI Summary` (required on `dev` and `main`) requires the `linux` job; it requires the `macos` job only where it runs: on
`release/*` branches (their PRs and the CI run Prepare Release dispatches), **post-merge on every push to `dev`**, and on
a manual CI dispatch with `macos`. A post-merge run never gates a PR (about 41 min of hosted macOS time per merge; no
billed minutes on a public repo, ADR-0001 §9.6). macOS-only code (CoreBluetooth via `btleplug`, `crank-desktop`'s macOS
lifecycle and display code, the `crank-app` bundle and its `bundle-no-nix-store` and `install-app` checks) is therefore
checked after each merge and per candidate, not per PR.

**Triage of a red post-merge macOS run on `dev`** (anyone, agents included):

1. If the job failed for the runner (not acquired, cancelled with `runner=null`, #480), re-run it **once** and note
   the re-run with its link on the merged PR. A second runner failure is handled as below.
2. Otherwise file a `bug` issue (`gh issue create -R gerchowl/crank --label bug`) with the run link and the merged PR
   **before the next merge into `dev`**, and fix it by an ordinary PR. No release candidate is cut while `dev`'s
   latest macOS run is red.

## Repository settings the flow relies on (owner)

* Settings > Actions > General > **"Allow GitHub Actions to create and approve pull requests"**. It is **off** today
  (#439). With it off, Prepare Release still pushes the branch and dispatches CI, then **fails** at opening the PR and
  prints the compare URL: open the draft PR by hand from it. Turn it on to make that step work.
* CI on the release branch is started by `workflow_dispatch` (a push by `GITHUB_TOKEN` starts no workflow), so
  `CI Summary` appears on the branch head and satisfies the required check of `dev` (candidates) or `main` (final).
* The first Prepare Release needs `dev` not to be behind `main`; if it is, merge `main` into `dev` by a PR first.

## Version sources

`Cargo.toml` (`[workspace.package]`), `app/src-tauri/tauri.conf.json`, both bundle versions in `app/src-tauri/Info.plist`
(mapped as above) and every `package.json` that has a `version` are bumped together by `scripts/release-version.py bump`;
`Cargo.lock` follows (`cargo update --workspace`) and the flake reads the version from `Cargo.toml`. `scripts/release-version.py
check` fails on any disagreement: it is the `release-versions` flake check (CI) and part of `just config-checks`; its tests
(`scripts/test-release-version.py`) are the `release-versions-test` check.
