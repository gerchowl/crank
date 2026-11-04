# ADR-0003: In-app bug reports with squelch

- **Status:** Proposed
- **Date:** 2026-11-02; amended 2026-11-04 (#508: structured fields only, §2.2, §2.3, §2.7, §3, §5)
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

**Spike 3** ran the first mapping, including the known-device replacement of the rejected scrub (§3), on
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

**Amendment 2026-11-04 (#508, owner decision).** A report carries only allowlisted structured fields,
never free-text log messages. The mapping below is the amended one: the body and every free-text
attribute are dropped, not scrubbed (§2.3), and the history of the scrub that was tried instead is in §3.

**Mapping** (`crank-desktop::report::flatten` reads, `report::allowlist` reduces; pure and unit-tested).
The tail is selected **first**: lines are parsed from the end of `logs.jsonl`, then of the rotated
`logs-<stamp>.jsonl` segments newest first, only until it is full, and only its records are processed.

| OTLP/JSON | Flat key | Rule |
|---|---|---|
| `resourceLogs[].resource.attributes` | — | **dropped**. They hold `host.name` (the machine name), `process.pid` and `service.instance.id`. Version and commit come from provenance (§2.6) |
| `scopeLogs[].scope.name` | `module` | the crate or module path. Not `target`, which squelch masks as a host field |
| `logRecords[].timeUnixNano` (else `observedTimeUnixNano`) | `timestamp` | decimal-string nanoseconds rendered as RFC 3339 UTC with milliseconds (`time` 0.3, `formatting`, already in `Cargo.lock`). `"0"`, missing or unparsable in both gives `unknown`, so the record is kept and visibly has no time |
| `severityText` | `level` | when it is `TRACE`, `DEBUG`, `INFO`, `WARN` or `ERROR`. Otherwise use `severityNumber`: 1–4 `TRACE`, 5–8 `DEBUG`, 9–12 `INFO`, 13–16 `WARN`, 17–24 `ERROR`. Otherwise no `level`, and squelch's `INFO` default applies |
| `body.stringValue` | — | **never included.** It only selects `event` when it is exactly a registered name (§2.3) |
| attribute `event`, else the body | `event` | a stable event name from `report::allowlist::EVENTS`, else the literal `log` (§2.3) |
| `attributes[]` `{key, value}` | `key` | only the typed allowlist and the identifier keys of §2.3; every other attribute (`message`, `error`, `e`, `err`, `raw`, `?Debug` values) is dropped. `arrayValue`, `kvlistValue` and `bytesValue` are dropped |
| `traceId`, `spanId`, `eventName`, `flags` | — | dropped. `eventName` holds the call site's source path, which is absolute for a dependency (a home directory) |

Each record is serialized as one flat JSON line. The joined lines go to
`Redactor::records(&flat, Some("logs.jsonl"))` with `report::allowlisted` (squelch's allowlist replaced by
the keys a record can have). The tail holds the last **200** records (architect decision 2026-11-02, the
owner may override). A line that fails to parse is skipped, never guessed at, which is squelch's own rule.
**Fail closed:** `report::diagnostics` returns an error, and the caller sends no report, when squelch
keeps fewer records than it was given; the caller sends none either when `Core::known_devices` fails.

### 2.3 Privacy

- **Never attached:** `.fecap` captures, ride data (the database and exports) and the profile. The report
  reads only `app_log_dir/logs*.jsonl`. It does not read `traces.jsonl`, the capture directory or
  `data_dir`.
- **Structured fields only** (`crank-desktop::report::allowlist`, #508). A record is `timestamp`, `level`,
  `module`, `event` and the fields below. Free text never reaches a report: the OTLP body (the message,
  values interpolated) and every free-text attribute (`message`, `error` and `e` Display strings, `?Debug`
  values such as `command` and `raw`) are **dropped, not scrubbed**. An error reaches a report only as its
  kind or variant name in a separate field (`kind`); otherwise it is dropped. Nothing searches inside a
  value: there is no pattern scrub of free text (§3).
- **Typed allowlist** (`ALLOWED`). A value of another OTLP type, or a string outside the field's closed
  vocabulary, is dropped. A new field is absent from reports until it is added here deliberately.

  | Field | Type |
  |---|---|
  | `page`, `attempt`, `ms`, `lost`, `watts`, `rpm` | integer or finite double |
  | `force`, `connected`, `takeover`, `taken` | bool |
  | `device` | `trainer`, `strap` |
  | `from`, `to` | the connect phases `resolve`, `gatt`, `identity`, `handshake`, `ready`, `not_found` |
  | `via` | `lookup`, `scan` |
  | `kind` | the variant names of `ProtocolErrorKind` and vigil's `FlushErrorKind` |
  | `characteristic` | the GATT UUIDs crank uses (FE-C, FTMS, Cycling Power, Heart Rate, DIS): a protocol, not a device |
  | `ride_id` | `ride-` and lowercase hex (the ride's start time) |

- **Identifier fields, redacted by key** (`IDENTIFIERS`). Whatever the value, the field is kept only as a
  placeholder: `name`, `local_name`, `device_name` → `<device-name>`; `serial`, `serial_number`, `sn` →
  `<serial>`; `address`, `addr`, `mac` → `<ble-addr>`; `id`, `peripheral_id`, `platform_id`, `device_id`,
  `db_id` (the store's `<kind>:<platform id>`) → `<ble-id>`; `path`, `dir`, `file` → `<path>`. A value
  that is **wholly** a known device's name, serial or platform id (whitespace folded, case ignored) gets
  that device's role instead: `<trainer>` or `<strap>`. `report_preview` asks `Core::known_devices` (the
  remembered pairings). Known-device matching never searches inside a value, and a device missing from
  it costs only the role, never privacy.
- **Event names** (`EVENTS`). `event` is an explicit `event` attribute when it is a registered name, else
  the body when it is exactly a registered name, else the literal `log`. The registry holds crank's
  literal log messages (a survey of the `tracing` calls, 2026-11-04; a test keeps each one in the source)
  and will hold #509's event names. vigil 0.2.0 writes the formatted message, not its template, so the
  template of a message with interpolated values cannot be derived reliably, and an unregistered
  `event` value could be anything. **Triage cost:** until call sites log typed fields and an `event`
  (#509), records of interpolated messages and of other crates show `event=log`: triage sees module,
  level, time and the typed fields, never the interpolated values.
- **`module`** is the scope name, the `tracing` target: a module path fixed at compile time.
- **Pinning test** (`crank-desktop/tests/report_privacy.rs`). Its fixture `logs.jsonl` is built from vigil
  output of crank's logging pipeline and hand-salted with device names, serials, BLE addresses,
  CoreBluetooth UUIDs, a home path and `host.name`. The test also salts every leak spelling that #482's
  review rounds found (URL-encoded, byte-array and mixed-separator addresses, UUIDs next to `_`, serial-label
  variants, decimal and byte forms of serials, NFKC and zero-width variants of known names), one at a
  time, into the body, `error`, `e`, `err`, `message`, `event`, `severityText` and every field a record can
  keep, as a string and as an integer. It asserts that no run of four or more letters or digits of any
  spelling reaches `Report::preview()`, and that what a triager needs survives: module, level, event names,
  `page`, `watts`, `rpm`, the role placeholders and a FE-C GATT UUID in `characteristic`.

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
3. `flatten`, the core query for known devices, the typed allowlist with identifier keys and event names,
   and the privacy pinning test (§2.2–2.3; #444, amended by #508: structured fields only). Needs #414 merged.
   The call sites' conversion to typed fields and `event` names is #509.
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
| Scrubbing free text: pattern rules for serials, BLE addresses and UUIDs plus a known-device substring table over `message` and `error` (#444's first design, PR #482) | Rejected 2026-11-04 by the owner (#508). PR #482 used all three fix rounds, and every falsify round found new leak spellings: serial-label variants (`serial no. X`, `Some("X")`), no-break and zero-width spaces, URL-encoded names and addresses, byte-array and big-endian serial forms, mixed-separator, Cisco and bare addresses, UUIDs next to `_`. The fixes over-redacted in turn (short serials inside numbers, timestamps). Enumerating spellings cannot be made safe |
| A rider opt-in to include the raw text | A rider cannot review 200 records for identifiers, so the consent would not be informed, and the issue form is public. A maintainer who needs the text asks for the log, which the rider can review and attach themselves |
| `tauri-plugin-clipboard-manager` for Copy log | It is a new plugin and capability, and the webview clipboard on a user click is enough |

## 4. Consequences

- A user can file a useful report without knowing where logs are. Maintainers get the version, commit,
  target and a consistent redacted tail.
- The diagnostics block needs a manual paste, which is the price of not putting logs in URLs.
- The allowlist adds friction: a new log field stays out of reports until it is reviewed. That is intended.
- squelch API changes land as reviewed rev bumps. The privacy test and the sync test are the guard.

## 5. Known limits

- A report has no free text, so the interpolated values of today's messages (error causes, device
  states) are not in it until #509 logs them as typed fields (§2.3, triage cost).
- The privacy of `event` and of the typed fields rests on their registries: a name added to `EVENTS` or
  a word added to a vocabulary must be a constant, never a value from a device or the machine. Review of
  `report/allowlist.rs` is the guard.
- A number in an integer field is kept as it is. A call site that logs a serial under a field such as
  `page` would leak it; serials belong under the identifier keys.
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
