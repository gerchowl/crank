# Releasing crank

Adopted from devkit's release flow (#428): `dev` is cut into `release/X.Y.Z`, a draft PR carries it into `main`, the owner
merges, and CI builds `crank.app` and drafts the GitHub release. Agents never push or merge `main` and never publish.
The workflows use `GITHUB_TOKEN` only; there are no secrets.

## Cutting a release (owner)

1. Make sure `## Unreleased` in [CHANGELOG.md](CHANGELOG.md) lists the release's changes and `dev` is green.
2. Run **Prepare Release** (Actions, `workflow_dispatch`) with the version `X.Y.Z`. It refuses a bad version, an existing tag,
   another `release/*` train, or a `dev` that is behind `main`. It bumps the version sources, freezes the changelog, pushes
   `release/X.Y.Z`, dispatches CI on it and opens a draft PR into `main`. `dry-run` stops before pushing.
3. When `CI Summary` is green, mark the PR ready and **merge it with a merge commit**.
4. **Release** runs on the merge: it builds `.#crank-app` on a macOS runner and drafts the release `X.Y.Z` with
   `crank-X.Y.Z-aarch64-darwin.zip`, its `.sha256` and the notes. Read the draft, then **publish** it (that creates the tag).
5. Merge the `chore: sync main into dev` PR that Release opened (merge commit), so `dev` has the version bump.

To back out before publishing, run **Abandon Release** with the version: it deletes the draft release, closes the PR and
deletes `release/X.Y.Z`. A published release cannot be abandoned (its tag is permanent): release the next version. If the
release PR was already merged, revert it by PR.

## Repository settings the flow relies on

* Settings > Actions > General > "Allow GitHub Actions to create and approve pull requests". It is **off** today. With it off,
  Prepare Release and Release still push, build and draft, but cannot open the PRs; they print a warning with the compare URL
  and you open the PR by hand.
* CI on the release branch is started by `workflow_dispatch` (a push by `GITHUB_TOKEN` starts no workflow), so `CI Summary`
  appears on the branch head and satisfies `main`'s required check.

## Version sources

`Cargo.toml` (`[workspace.package]`), `app/src-tauri/tauri.conf.json`, both bundle versions in `app/src-tauri/Info.plist`
and every `package.json` that has a `version` are bumped together by `scripts/release-version.py bump`; `Cargo.lock` follows
(`cargo update --workspace`) and the flake reads the version from `Cargo.toml`. `scripts/release-version.py check` fails on any
disagreement: it is the `release-versions` flake check (CI) and part of `just config-checks`.

## The build is not signed

Until Developer ID signing and notarization land (#47, #426) the zip holds an ad-hoc signed app. The release notes say so,
give `xattr -dr com.apple.quarantine /Applications/crank.app` and tell users that Bluetooth is asked again on each release
(text: `.github/release-notes-footer.md`). Fleet Macs keep `just bundle` for a stable grant.
