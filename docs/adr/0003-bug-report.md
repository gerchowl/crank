# ADR-0003: In-app bug reports with squelch

- **Status:** Proposed
- **Date:** 2026-11-02
- **Issue:** #431
- **Related:** ADR-0001 §7.1 (logs via vigil, #414), §4.4 (`ApiError`), #428 (releases), #256, #433

## 1. Context

The owner wants crank to adopt [gerchowl/squelch](https://github.com/gerchowl/squelch) (2026-11-02,
#431). The app collects the environment and a redacted log tail, checks the required fields and shows
a preview, then opens GitHub's prefilled issue form. The user reviews the form and clicks Submit, so
nothing is sent automatically.

The facts that shape the design:

- **squelch status.** squelch 0.1.0 is not on crates.io, calls itself early, and its API may shift.
  It is MIT OR Apache-2.0 with `rust-version` 1.86. `main` is at `4f0d064` (2026-10-01).
- **Redactor.** Its JSONL path, `Redactor::records`, reads only **top-level** keys: `timestamp|ts|time|@timestamp`,
  `level|severity|lvl`, the default scrubbed fields (`message`, `msg`, `error`, `err`, `host`, `addr`,
  …) and the fields the application allows. Every other key is dropped unread. It masks home paths,
  user names, credentials, e-mail addresses, IP addresses and private host names. It has **no hook for
  application value rules**.
- **Browser route.** `Transport::Browser` never puts diagnostics in the URL, because GitHub logs request
  URLs. `Sent::Opened { diagnostics }` hands the log block back for the user to paste.
  `Browser` also refuses to open when a field would be dropped from the URL.
- **Log source.** Since #414, crank's logs are vigil's OTLP/JSON `logs.jsonl` (ADR-0001 §7.1). The owner
  comment on #431 replaces the issue's original plan, a separate tracing ring layer, with that file.

## 2. Decision

### 2.1 Dependency

`squelch = { git = "https://github.com/gerchowl/squelch", rev = "4f0d0640e4dbf941b87b5cc5dc59e5e60dc86816",
default-features = false, features = ["browser", "logs", "serde"] }`, in `crank-desktop` only. The core
never opens a browser. `serde` is needed for `squelch::form::github::IssueForm`, which the template sync
test (§2.5) parses. That test also needs the dev-dependency `serde_yaml_ng = "0.10"`, which squelch's own
test uses (`serde_yaml` is archived). `deny.toml [sources] allow-git` adds the squelch URL. The `deny`
check covers both crates' licences and advisories. Moving the rev is a reviewed
PR, and moving to crates.io is #433. The crate's MSRV, 1.86, is below crank's 1.96.

### 2.2 Input: vigil `logs.jsonl` through an explicit mapping

**Spike result.** squelch's `records` does **not** parse vigil's lines: it returned **0 records** for
a `logs.jsonl` line holding 2 records. The line's top level is `{"resourceLogs":[…]}`, so neither a
timestamp key nor any scrubbed field is present. The same records produced 2 squelch records after
the mapping below.

The spike ran 2026-11-02 in a scratch crate outside the repo, with vigil `0.2.0` and squelch
`4f0d064`, using `cargo run` in crank's dev shell. Exit code 0. Its output:

```text
squelch records (as-is): 0
squelch records (mapped): 2
  1793650453584412000  INFO [logs.jsonl] message="trainer connected" addr=<<host>> device=fec serial=1234567
  1793650453584424000 ERROR [logs.jsonl] message="control write failed" error="write timed out at <path>"
```

The output shows three more things. `addr` was masked, by squelch's field rule. The home path was
masked. `serial` survived **verbatim** once it was allowlisted, which is why §2.3 exists.

**Spike 3** ran the mapping as specified below, including the known-device replacement (§2.3), on
vigil output plus a hand-made edge line. Exit code 0. Its output:

```text
squelch records (raw): 0
squelch records (mapped): 3
  2026-11-02T21:13:30.619Z  WARN [logs.jsonl] message="collision probe" module=obs_spike3 page=25 attr.level=fake attr.module=fake attr.timestamp=fake
  2026-11-02T21:13:30.619Z ERROR [logs.jsonl] message="trainer reconnect failed" error="connect to <trainer> (<trainer-id>) failed" module=obs_spike3
  unknown  WARN [logs.jsonl] message=edge module=crank_ble page=81
leak 55624: false
leak 3F2504E0: false
leak sage-ws: false
```

Two findings changed the mapping:

- squelch treats a field named `target` as a host field (`HOST_FIELD_NAMES`) and masked the crate name
  to `<host>`, so the scope name is mapped to `module`.
- squelch drops a record that has no timestamp key, so a missing time is written as `unknown`.

**Mapping** (`crank-desktop::report::flatten`, pure and unit-tested). For each line of `logs.jsonl`,
then the rotated `logs-<stamp>.jsonl` segments newest first until the tail is full:

| OTLP/JSON | Flat key | Rule |
|---|---|---|
| `resourceLogs[].resource.attributes` | — | **dropped**. They hold `host.name` (the machine name), `process.pid` and `service.instance.id`. Version and commit come from provenance (§2.6) |
| `scopeLogs[].scope.name` | `module` | the crate or module path. Not `target`, which squelch masks as a host field |
| `logRecords[].timeUnixNano` (else `observedTimeUnixNano`) | `timestamp` | decimal-string nanoseconds rendered as RFC 3339 UTC with milliseconds (`time` 0.3, `formatting`, already in `Cargo.lock`). `"0"`, missing or unparsable in both gives `unknown`, so the record is kept and visibly has no time |
| `severityText` | `level` | as is. If it is empty or missing, use `severityNumber`: 1–4 `TRACE`, 5–8 `DEBUG`, 9–12 `INFO`, 13–16 `WARN`, 17–24 `ERROR`. Otherwise no `level`, and squelch's `INFO` default applies |
| `body.stringValue` | `message` | non-string bodies are dropped |
| `attributes[]` `{key, value}` | `key` | `stringValue`, `intValue`, `doubleValue` or `boolValue` as a string. `arrayValue`, `kvlistValue` and `bytesValue` are dropped. A key that collides with `timestamp`, `level`, `message` or `module` becomes `attr.<key>`, so an event field cannot overwrite the record's own |
| `traceId`, `spanId`, `eventName`, `flags` | — | dropped. `eventName` holds the source path |

Each flat record then goes through crank's pre-scrub (§2.3) and is serialized as one flat JSON line.
The joined lines go to `Redactor::records(&flat, Some("logs.jsonl"))` with crank's allowlist. The tail
holds the last **200** records (architect decision 2026-11-02, the owner may override), so the report
never needs the whole file. A line that fails to parse
is skipped, never guessed at, which is squelch's own rule.

### 2.3 Privacy

- **Never attached:** `.fecap` captures, ride data (the database and exports) and the profile. The report
  reads only `app_log_dir/logs*.jsonl`. It does not read `traces.jsonl`, the capture directory or
  `data_dir`.
- **Pre-scrub** (`crank-desktop::report::scrub`). It runs on every string value **before** squelch, because
  squelch cannot take custom rules (#433). The order is normalisation, then the structural rules (hex identifiers,
  serial values after a label), then the known-device table, so a device name or serial can never cut an address
  or id apart before its rule sees it (#482 review, round 2). The rules are structural by design: three review
  rounds each found a new spelling while the rules enumerated spellings, so they describe shapes instead
  (#482 review, round 3):
  - **Normalisation first.** Each value loses its zero-width, bidi and soft-hyphen characters, is NFKC-normalised (a
    no-break space becomes a space, full-width letters ASCII) and has every whitespace run folded to one space,
    before any rule below. squelch normalises later, so a name split by `U+00A0` or `U+200B` would otherwise pass
    crank's table and be joined again by squelch (#482 review).
  - **Serials.** The FE-C page 81 serial and the DIS Serial Number String (`0x2A25`) are replaced by `<serial>`.
    In a field, the rule is by name: `serial`, `serial_number`, `sn`. In free text, after the words `serial`
    (`serial_number`), `S/N` or `SN`, any label words (`number`, `no.`, `string`, `is`, `of`, `read`) and any
    punctuation (`:`, `—`, `→`, `;`, quotes, brackets, Rust's `Some(`, or none as in `serial7788991`), the
    **value** is replaced and the label kept (`serial no. <serial>`). The value is a token **with a digit**, so
    `serial port opened` and `the serial is not available` stay as they are.
  - **Hex identifiers.** A run of hex digits joined by single optional separators (`:`, `-`, `_`, `.`, a space,
    mixed allowed), bounded only by what is neither, is an identifier when it has at least 12 hex digits and
    either an `a`–`f` letter or the layout of an address (six pairs) or a UUID (8-4-4-4-12). It becomes
    `<ble-addr>` with exactly 12 hex digits, else `<ble-id>`, or the device's placeholder when it is a known
    platform id (`<trainer-id>`). This covers colon, dash, underscore and bluez `dev_…` addresses, Cisco and
    bare forms, dashless UUIDs and a hex dump of raw bytes, by construction. A first or last group glued to a
    letter (the `ce` of `device-C4-…`) is left out when the rest still qualifies.
    - **Decimal runs** (only the digits 0–9) count only with the address or UUID layout: a run like
      `2026-11-03 21:13:30.619` or a 12-digit counter is a date, a time or a count, never an identifier.
      Adjacent identifiers joined by a space merge into one `<ble-id>`, and a sentence of pure-hex words
      (`a dead beef cafe`) with 12 or more hex digits would be masked; both only over-redact.
    - The GATT service and characteristic UUIDs crank uses (FE-C, FTMS, Cycling Power, Heart Rate, DIS) are on
      a keep-list, because they identify a protocol, not a device.
  - **Known devices, in every kept field.** `report_preview` asks the core (`Core::known_devices`) for every
    remembered pairing and every **kept sighting**, and builds a replacement table from each one's name,
    serial and platform id:
    - A sighting is recorded when a scan reports a trainer or strap (paired or not), when a pairing is
      attempted (before its connect, so a failed pairing counts) and when a connect answers (with the name and
      serial it reports). Sightings are persisted (`crank-store` `seen_device`, migration 0004), because
      rotated logs of an earlier run can name a device that run forgot. The store keeps the newest 64; a
      forgotten pairing therefore stays in `seen_device` until 64 newer sightings displace it. It is local
      data, used only to scrub.
    - Needles are matched with Aho–Corasick (linear in the text, whatever the number of devices; no regex size
      limit) on the text's **skeleton**: lower-cased, letters and digits only, each placeholder a stop no needle
      crosses. Separators and case therefore never need a spelling of their own: `Tacx_Neo_55624`,
      `Tacx+Neo+55624`, `TacxNeo` (a zero-width space removed), a dashless or Cisco id, `2655 624` and
      `[88, 85, 28, 00]` are the same needle. Listed spellings: the name and its URL encoding (`%20`, `%3A`);
      the platform id; the serial, and for a decimal one its hex (`0x`, also zero-padded, or bare) and its
      little- and big-endian bytes as hex (the big-endian ones are also the zero-padded bare hex), as `0x`
      bytes and as decimal bytes (`88 85 28 00`, `0x00 0x28 0x85 0x88`, `[136, 133, 40, 0]`).
    - A match must be a **whole token or token sequence** (between non-alphanumerics), never inside a word and
      never inside a placeholder; the longest of overlapping matches wins.
    - Needles under **4** letters and digits are not matched in free text, nor decimal ones under **5** digits:
      a name `N` or `Pro`, a serial `1`, `12` or `2026` is a common word, number or year, and replacing it
      everywhere would hide the log, not the device. A short serial stays covered where it is labelled, and
      addresses and ids by the hex rule. A needle with no letter or digit (`🚴🚴`) is never matched.
    - The placeholder is named after the device's role: `<trainer>`, `<strap>`, with `-serial` and `-id`
      variants such as `<trainer-id>`. crank has no third role, so there is no `<sensor>`.
    - The table is applied to **every** string that survives into a record: `message`, `error` and every
      allowlisted field. Only the last 200 records are scrubbed: the tail is selected before the scrub, never
      the whole 8 MiB segment.
    - This closes the free-text leak: AGENTS.md notes that error text carries device names, a name such
      as `Tacx Neo 55624` embeds the serial, and §5 keeps bare digit runs. Spike 3 shows
      `connect to Tacx Neo 55624 (<uuid>) failed` arriving as `connect to <trainer> (<trainer-id>) failed`.
    - The `name` and `local_name` fields are always replaced, by the matching role placeholder or else
      by `<device-name>`, which also covers a device the store has not remembered.
- **Field allowlist** (`Redactor::allow_fields`). It applies in addition to squelch's scrubbed defaults:
  `module`, `event`, `kind`, `page`, `result`, `connected`, `takeover`, `taken`, `last`, `discarded`,
  `command`, `st`, `dir`, `path`, `ride_id`, `db_id`. These are the field names crank logs today (a
  survey of the `tracing` calls in `crates/` and `app/src-tauri/`, 2026-11-02). `dir` and `path` still go through squelch's home-path rule. `raw`
  (unknown FE-C page bytes) is **not** allowlisted. A new log field is absent from reports until it is
  added here deliberately.
- **Pinning test** (`crank-desktop/tests/report_privacy.rs`). It uses a fixture `logs.jsonl`: vigil output of
  crank's own logging pipeline in a capture directory (no hardware session log is committed), then
  hand-salted with a NEO serial, a DIS serial, BLE addresses, CoreBluetooth UUIDs, a home path, `host.name`,
  every spelling the pre-scrub lists, unknown device names and serials in the name and serial fields, and a
  partial last line. It also contains an `error` message with the paired trainer's name with its serial
  (`Tacx Neo 55624`) and its BLE id, and a strap's name. The test's known-device table holds those devices.
  It asserts that none of these appear in `report.preview()`, with crank's allowlist and again with every
  fixture field allowlisted, so removing any one pre-scrub rule fails it; and that the strings a triager
  needs survive: `crank_ble`, `trainer reconnect failed`, `page=25`, and a GATT UUID from the keep-list.

### 2.4 The UI surface

- **Settings → "Report a problem."** Two required fields: *What happened* (`current-behavior`) and *Steps to
  reproduce* (`reproduction`). Below them, the preview shows `report.preview()` verbatim (environment,
  log tail, destination), with a note that the log block is not in the link and is copied for pasting.
  The **Open on GitHub** button is disabled until both fields are filled. squelch's `require` checks the
  same fields in Rust.
- **Error and failure banners** (`ApiError` banners, the reconnect-failed banner) get a "Report this"
  action (architect decision 2026-11-02, because the owner asked for a report surface; the owner may override). It opens the same dialog with `error` prefilled with the banner's code and message, scrubbed
  like any other value.
- **Transport.** `Transport::Browser` only, and no other route is compiled in. The user's click on
  Submit in GitHub is the only send. After the browser opens, the dialog keeps the diagnostics block
  with a **Copy log** button (`navigator.clipboard.writeText` on the click, no new plugin) and points
  the user at the form's *Diagnostics* field.
- **Commands** (tauri-specta, ADR-0001 §4):
  - `report_preview(input) -> Result<ReportPreview, ApiError>` returns the preview, the diagnostics block,
    and the `shortened`/`dropped` lists.
  - `report_open(input) -> Result<(), ApiError>`.
  - squelch's `FieldsDropped` maps to `InvalidInput`, and the UI tells the user to shorten their text.
    A missing required field maps to `InvalidInput`. A browser that fails to open maps to `DeviceUnavailable`.
    The UI mock models these three refusals.

### 2.5 Issue template

`.github/ISSUE_TEMPLATE/bug.yml`, with field ids as the contract:

| id | type | Filled by |
|---|---|---|
| `current-behavior` | textarea, required | user |
| `reproduction` | textarea, required | user |
| `error` | textarea | the banner entry point (§2.4) |
| `environment` | textarea | provenance (`Report::provenance_field("environment")`) |
| `diagnostics` | textarea, `render: text` | the user pastes the copied log block |
| `confirm` | checkboxes, required | user only. GitHub cannot prefill checkboxes, and the app must not attest for the user |

The app's ids are constants in `crank-desktop::report::fields`. The sync test
(`crank-desktop/tests/issue_form.rs`, the same pattern as squelch's `tests/issue_form.rs`) reads the
template with `include_str!` and parses it with squelch's form parser. It asserts that every constant
is declared with the expected type, that the required set matches, and that no app id is missing. A
renamed id fails CI instead of being silently dropped by GitHub.

### 2.6 Provenance

`build.rs` in `crank-desktop` exports `CRANK_TARGET` (`TARGET`), `CRANK_PROFILE` (`PROFILE`) and
`CRANK_COMMIT`. `CRANK_COMMIT` comes from the Nix flake's `self.rev`, or `dirty`/`unknown`, because
the Nix sandbox has no `.git`. The report sends
`Provenance::standard().app("crank", CARGO_PKG_VERSION).build(target, profile).commit(commit)`, scrubbed.
vigil's `revision` (ADR-0001 §7.1) uses the same `CRANK_COMMIT`. Once #428 releases exist, the version
is the release version (release-please bumps `CARGO_PKG_VERSION`), and the template's version check
can rely on it. The destination is set explicitly as `gerchowl/crank`, not taken from
`CARGO_PKG_REPOSITORY`.

### 2.7 Implementation sequence (sub-issues; the architect files them)

1. The squelch dependency (`browser`, `logs`, `serde`), the `serde_yaml_ng` dev-dependency and `deny.toml`
   (§2.1).
2. `bug.yml`, the field-id constants and the sync test (§2.5).
3. `flatten`, `scrub`, the core query for known devices, the allowlist and the privacy pinning test
   (§2.2–2.3). Needs #414 merged.
4. The `build.rs` provenance (§2.6), shared with vigil's `revision`.
5. The `report_preview` and `report_open` commands, the bindings, and the Settings dialog with preview and
   Copy log, plus a mock that models every refusal (§2.4).
6. The banner entry point with the prefilled `error` (§2.4).
7. Docs: AGENTS.md, and a `spec/UX-SPEC.md` entry for the dialog.

## 3. Alternatives considered

| Alternative | Why rejected |
|---|---|
| A `tracing` ring layer mapped to `Record` (#431's original point 2) | The owner replaced it (#431 comment). It is a second log path that sees records vigil does not, and the history is lost on restart |
| Feeding `logs.jsonl` to `Redactor::records` directly | The spike gives 0 records (§2.2) |
| Relying on squelch alone for redaction | No serial, BLE address or UUID rule. The spike shows `serial=1234567` surviving |
| `GhCli`, `Endpoint` or `Mailto` routes | The form's `required` checks are skipped, and these routes need a credential or have no review step. The owner chose `Browser` only |
| Attaching `.fecap` or the ride database | It is the most identifying data crank has (serials, power, schedule), and maintainers can ask for a capture explicitly |
| `tauri-plugin-clipboard-manager` for Copy log | It is a new plugin and capability, and the webview clipboard on a user click is enough |

## 4. Consequences

- A user can file a useful report without knowing where logs are. Maintainers get the version, commit,
  target and a consistent redacted tail.
- The diagnostics block needs a manual paste, which is the price of not putting logs in URLs.
- The allowlist adds friction: a new log field stays out of reports until it is reviewed. That is intended.
- squelch API changes land as reviewed rev bumps. The privacy test and the sync test are the guard.

## 5. Known limits

- Encoded secrets, and hosts outside squelch's suffix list, pass through. The same goes for non-English free
  text, where squelch's position anchors do not apply. These are squelch's documented limits.
- The pre-scrub's free-text serial rule needs a cue word. A bare digit run in a message is kept,
  because masking every number would destroy watts, cadence and page numbers. The serials and names
  of known devices are replaced anyway (§2.3). The residue is a device crank never remembered or
  connected, such as a nearby trainer seen in a scan, whose name appears in free text without a cue.
- The 200-record tail may miss the cause of a long-past failure. A user can attach a longer log
  themselves.
- Span context (`ride{id}`) is not in the tail until vigil puts span fields on log records (#432).
  Records that carry `ride_id` themselves keep it.

## 6. References

- squelch README and `src/redact.rs`, `src/report.rs`, `tests/issue_form.rs` at `4f0d064` (checked 2026-11-02):
  https://github.com/gerchowl/squelch
- vigil README and ADR-0003 at tag `0.2.0` (`64f9b8f`, checked 2026-11-02): https://github.com/vig-os/vigil
- GitHub issue forms syntax: https://docs.github.com/en/communities/using-templates-to-encourage-useful-issues-and-pull-requests/syntax-for-issue-forms
- In-repo: ADR-0001 §4.4, §7.1; ADR-0002 §4.2 (page 81); `spec/TRAINER-BLE.md`; `app/src-tauri/src/logging.rs`
