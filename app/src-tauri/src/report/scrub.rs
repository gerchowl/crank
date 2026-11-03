//! crank's pre-scrub (ADR-0003 §2.3): device identities replaced in every string of a flat
//! record before squelch sees it, because squelch cannot take custom rules (#433).
use super::flatten::Flat;
use aho_corasick::{AhoCorasick, BuildError};
use crank_core::{DeviceRole, KnownDevice};
use regex::Regex;
use std::sync::LazyLock;
use unicode_normalization::UnicodeNormalization;

/// The flat fields a report keeps besides squelch's scrubbed defaults (`message`, `error`, …):
/// the field names crank logs today. A new log field is absent from reports until it is added
/// here deliberately; `raw` (unknown FE-C page bytes) is not.
pub const ALLOWED_FIELDS: [&str; 16] = [
    "module",
    "event",
    "kind",
    "page",
    "result",
    "connected",
    "takeover",
    "taken",
    "last",
    "discarded",
    "command",
    "st",
    "dir",
    "path",
    "ride_id",
    "db_id",
];

/// The GATT service and characteristic UUIDs crank uses (FE-C, FTMS, Cycling Power, Heart Rate,
/// DIS). They identify a protocol, not a device, so they survive the platform-id rule.
const KEEP_UUIDS: [&str; 11] = [
    "6e40fec1-b5a3-f393-e0a9-e50e24dcca9e",
    "6e40fec2-b5a3-f393-e0a9-e50e24dcca9e",
    "6e40fec3-b5a3-f393-e0a9-e50e24dcca9e",
    "00001826-0000-1000-8000-00805f9b34fb",
    "00001818-0000-1000-8000-00805f9b34fb",
    "0000180d-0000-1000-8000-00805f9b34fb",
    "00002a37-0000-1000-8000-00805f9b34fb",
    "0000180a-0000-1000-8000-00805f9b34fb",
    "00002a24-0000-1000-8000-00805f9b34fb",
    "00002a25-0000-1000-8000-00805f9b34fb",
    "00002a26-0000-1000-8000-00805f9b34fb",
];

/// A run of hex digits joined by single optional separators (`:`, `-`, `_`, `.`, a space), mixed
/// allowed. [`Scrub::identifiers`] decides whether a run is an identifier.
static HEX_RUN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[[:xdigit:]](?:[:_. -]?[[:xdigit:]])*").expect("constant pattern")
});
/// A serial label (`serial`, `serial_number`, `S/N`, `SN`), any label words and punctuation after
/// it (`number`, `no.`, `string`, `is`, `of`, `read`, `:`, `—`, `→`, `;`, quotes, brackets, Rust's
/// `Some(`), then the value: a token with a digit. A value without one (`serial port opened`, `the
/// serial is not available`) is a word, and kept.
static SERIAL_TEXT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?P<label>\b(?:serial(?:_?(?:number|num|no))?|s/n|sn)(?:[^[:alnum:]]*\b(?:number|num|no|string|str|is|of|read)\b)*[^[:alnum:]]*(?:some\([^[:alnum:]]*)?)[[:alnum:]._-]*[0-9][[:alnum:]._-]*",
    )
    .expect("constant pattern")
});
/// The placeholders this module writes; no needle matches inside one.
static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<(?:trainer|strap)(?:-id|-serial)?>|<(?:serial|ble-addr|ble-id|device-name)>")
        .expect("constant pattern")
});
/// Known-device needles with fewer letters and digits are not matched in free text: a 1–3
/// character name or serial (`N`, `Pro`, `12`) is a common word or number, and replacing it
/// everywhere hides the log instead of the device. A short serial stays covered where it is
/// labelled (`serial 12`), and addresses and ids by the structural rule. A needle of digits only
/// needs [`MIN_DECIMAL_NEEDLE`]: four digits are a year or a count (`2026`). A needle with no
/// letter or digit at all (`🚴🚴`) has an empty skeleton and is never matched.
const MIN_NEEDLE: usize = 4;
/// The shortest decimal needle matched in free text.
const MIN_DECIMAL_NEEDLE: usize = 5;

/// Whether a hex run is an identifier: at least 12 hex digits, and either an `a`–`f` letter or the
/// layout of an address (six pairs) or a UUID (8-4-4-4-12). A run of decimal digits alone is a
/// counter, a date or a time (`2026-11-03 21:13:30.619`, `123456789012`), not an identifier.
fn is_identifier(run: &str) -> bool {
    let groups: Vec<usize> = run.split([':', '-', '_', '.', ' ']).map(str::len).collect();
    let hex = groups.iter().sum::<usize>();
    let letter = run.chars().any(|c| matches!(c, 'a'..='f' | 'A'..='F'));
    hex >= 12 && (letter || groups == [2; 6] || groups == [8, 4, 4, 4, 12])
}

/// `text` lower-cased, letters and digits only, each placeholder a `\0` no needle contains: what
/// needles are matched against, so any separators (or none) may sit between a needle's words.
/// The second value maps each byte back to its character's byte range in `text`.
fn skeleton(text: &str) -> (String, Vec<(usize, usize)>) {
    let placeholders: Vec<_> = PLACEHOLDER.find_iter(text).map(|m| m.range()).collect();
    let (mut skel, mut map, mut skip_to) = (String::new(), Vec::new(), 0);
    for (at, c) in text.char_indices() {
        if at < skip_to {
            continue;
        }
        if let Some(p) = placeholders.iter().find(|p| p.start == at) {
            skel.push('\0');
            map.push((p.start, p.end));
            skip_to = p.end;
        } else if c.is_alphanumeric() {
            for lower in c.to_lowercase() {
                skel.push(lower);
                map.extend(std::iter::repeat_n(
                    (at, at + c.len_utf8()),
                    lower.len_utf8(),
                ));
            }
        }
    }
    (skel, map)
}

/// Invisible characters that would split a needle without showing: zero-width and bidi
/// controls, the word joiner family, the BOM and the soft hyphen.
fn invisible(c: char) -> bool {
    matches!(c,
        '\u{200b}'..='\u{200f}'
        | '\u{202a}'..='\u{202e}'
        | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{2069}'
        | '\u{feff}'
        | '\u{00ad}'
    )
}

/// `value` as crank's table compares it: invisibles removed, NFKC (a no-break space becomes a
/// space, full-width letters ASCII), then every whitespace run one space, trimmed. It runs before
/// the table, so a name that squelch would normalise later cannot slip past it.
#[must_use]
pub fn normalize(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut gap = false;
    for c in value.chars().filter(|&c| !invisible(c)).nfkc() {
        if c.is_whitespace() {
            gap = true;
        } else {
            if gap && !out.is_empty() {
                out.push(' ');
            }
            gap = false;
            out.push(c);
        }
    }
    out
}

/// The role's placeholder stem.
fn role(role: DeviceRole) -> &'static str {
    match role {
        DeviceRole::Trainer => "trainer",
        DeviceRole::HeartRate => "strap",
    }
}

/// The spellings of a device's identifiers, each with its placeholder. Needles are matched on the
/// [`skeleton`], so separators and case never need an entry (`Tacx_Neo_55624`, `C47D4F123456`, a
/// dashless id, `2655 624`, `[88, 85, 28, 00]`). Listed: the name and its URL encoding; the
/// platform id; the serial, and for a decimal one its hex (`0x`, also zero-padded, or bare) and
/// its little- and big-endian bytes as hex (the big-endian ones are also the zero-padded bare
/// hex), as `0x` bytes and as decimal bytes (`88 85 28 00`, `0x00 0x28 0x85 0x88`, `[136, 133, 40, 0]` for 2655624).
fn variants(d: &KnownDevice) -> Vec<(String, String)> {
    let role = role(d.role);
    let clean = |v: Option<&str>| v.map(normalize).filter(|v| !v.is_empty());
    let mut out = Vec::new();
    if let Some(name) = clean(d.name.as_deref()) {
        let encoded: String = name
            .bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                    char::from(b).to_string()
                }
                b => format!("%{b:02X}"),
            })
            .collect();
        out.extend([name, encoded].map(|v| (v, format!("<{role}>"))));
    }
    if let Some(id) = clean(Some(&d.platform_id)) {
        out.push((id, format!("<{role}-id>")));
    }
    if let Some(serial) = clean(d.serial.as_deref()) {
        let mut serials = vec![serial.clone()];
        if let Ok(n) = serial.parse::<u64>() {
            serials.extend([format!("0x{n:x}"), format!("{n:x}"), format!("0x{n:08x}")]);
            if let Ok(n) = u32::try_from(n) {
                for bytes in [n.to_le_bytes(), n.to_be_bytes()] {
                    let each = |f: fn(u8) -> String| bytes.map(f).join(" ");
                    serials.extend([
                        each(|b| format!("{b:02x}")),
                        each(|b| format!("0x{b:02x}")),
                        each(|b| b.to_string()),
                    ]);
                }
            }
        }
        out.extend(serials.into_iter().map(|v| (v, format!("<{role}-serial>"))));
    }
    out
}

/// The replacement table of the devices the core knows, and the generic identity rules.
#[derive(Debug)]
pub struct Scrub {
    /// The [`skeleton`]s of every needle of at least [`MIN_NEEDLE`] characters.
    needles: Option<AhoCorasick>,
    /// The placeholder of each pattern of `needles`, in order.
    placeholders: Vec<String>,
    /// Each device's platform id ([`skeleton`]) with its placeholder, for the structural rule.
    ids: Vec<(String, String)>,
    /// Each device's normalised name with its role placeholder, for the `name` field rule.
    names: Vec<(String, String)>,
}

impl Scrub {
    /// The table of `devices` ([`crank_core::Core::known_devices`]): every spelling of each name,
    /// platform id and serial becomes its role's placeholder (`<trainer>`, `<trainer-id>`,
    /// `<trainer-serial>`; `<strap>` and its variants for the strap).
    ///
    /// # Errors
    /// The matcher cannot be built; the report must then not be made (it fails closed).
    pub fn new(devices: &[KnownDevice]) -> Result<Self, BuildError> {
        let (mut patterns, mut placeholders) = (Vec::new(), Vec::new());
        for (needle, placeholder) in devices.iter().flat_map(variants) {
            let skel = skeleton(&needle).0;
            let min = if skel.bytes().all(|b| b.is_ascii_digit()) {
                MIN_DECIMAL_NEEDLE
            } else {
                MIN_NEEDLE
            };
            if skel.chars().count() >= min {
                patterns.push(skel);
                placeholders.push(placeholder);
            }
        }
        let names = devices
            .iter()
            .filter_map(|d| {
                let name = normalize(d.name.as_deref()?).to_lowercase();
                (!name.is_empty()).then(|| (name, format!("<{}>", role(d.role))))
            })
            .collect();
        let ids = devices
            .iter()
            .map(|d| (skeleton(&d.platform_id).0, format!("<{}-id>", role(d.role))))
            .filter(|(id, _)| !id.is_empty())
            .collect();
        let needles = if patterns.is_empty() {
            None
        } else {
            Some(AhoCorasick::new(&patterns)?)
        };
        Ok(Self {
            needles,
            placeholders,
            ids,
            names,
        })
    }

    /// Every value of `record` with the identities replaced. `name` and `local_name` are always
    /// replaced whole (by the device's role placeholder, else `<device-name>`), `serial`,
    /// `serial_number` and `sn` by `<serial>`.
    #[must_use]
    pub fn record(&self, mut record: Flat) -> Flat {
        for (key, value) in &mut record {
            *value = match key.as_str() {
                "name" | "local_name" => {
                    let value = normalize(value).to_lowercase();
                    self.names
                        .iter()
                        .find(|(name, _)| *name == value)
                        .map_or_else(|| "<device-name>".to_owned(), |(_, p)| p.clone())
                }
                "serial" | "serial_number" | "sn" => "<serial>".to_owned(),
                _ => self.text(value),
            };
        }
        record
    }

    /// One string, normalised ([`normalize`]); then the structural rules, so no device needle
    /// can cut an identifier apart first: hex identifiers (`Scrub::identifiers`), then the value
    /// after a serial label; then the known devices (`Scrub::devices`).
    #[must_use]
    pub fn text(&self, value: &str) -> String {
        let out = self.identifiers(&normalize(value));
        let out = SERIAL_TEXT.replace_all(&out, "${label}<serial>");
        self.devices(&out)
    }

    /// Every hex run ([`HEX_RUN`]) that [`is_identifier`] replaced: by the device's placeholder
    /// when it is a known platform id, kept when it is one of crank's GATT UUIDs, else `<ble-addr>`
    /// (exactly 12 hex digits) or `<ble-id>`. A run is bounded only by what is neither a hex digit
    /// nor a separator; a first or last group glued to a letter (`device-C4-…`, the `ce` of
    /// `device`) is left out when the rest still is an identifier.
    fn identifiers(&self, text: &str) -> String {
        let (mut out, mut copied) = (String::with_capacity(text.len()), 0);
        for run in HEX_RUN.find_iter(text) {
            let glued = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
            let (s, mut inner) = (run.as_str(), run.range());
            if glued(text[..run.start()].chars().next_back()) {
                inner.start += s.find([':', '-', '_', '.', ' ']).map_or(0, |i| i + 1);
            }
            if glued(text[run.end()..].chars().next()) {
                inner.end = run.start() + s.rfind([':', '-', '_', '.', ' ']).unwrap_or(s.len());
            }
            let Some(span) = [inner, run.range()]
                .into_iter()
                .find(|r| r.start < r.end && is_identifier(&text[r.clone()]))
            else {
                continue;
            };
            let found = &text[span.clone()];
            let key = skeleton(found).0;
            let placeholder = if KEEP_UUIDS.iter().any(|k| skeleton(k).0 == key) {
                found.to_owned()
            } else if let Some((_, p)) = self.ids.iter().find(|(id, _)| *id == key) {
                p.clone()
            } else if key.len() == 12 {
                "<ble-addr>".to_owned()
            } else {
                "<ble-id>".to_owned()
            };
            out.push_str(&text[copied..span.start]);
            out.push_str(&placeholder);
            copied = span.end;
        }
        out.push_str(&text[copied..]);
        out
    }

    /// `text` with every known needle replaced, matched on its [`skeleton`] in linear time
    /// (Aho–Corasick): only as a whole token or token sequence (between non-alphanumerics), never
    /// inside a placeholder, the longest of overlapping matches first.
    fn devices(&self, text: &str) -> String {
        let Some(needles) = &self.needles else {
            return text.to_owned();
        };
        let (skel, map) = skeleton(text);
        let alnum = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
        let mut hits: Vec<(usize, usize, usize)> = needles
            .find_overlapping_iter(&skel)
            .map(|m| (map[m.start()].0, map[m.end() - 1].1, m.pattern().as_usize()))
            .filter(|&(start, end, _)| {
                !alnum(text[..start].chars().next_back()) && !alnum(text[end..].chars().next())
            })
            .collect();
        hits.sort_by_key(|&(start, end, _)| (start, std::cmp::Reverse(end)));
        let (mut out, mut copied) = (String::with_capacity(text.len()), 0);
        for (start, end, pattern) in hits {
            if start >= copied {
                out.push_str(&text[copied..start]);
                out.push_str(&self.placeholders[pattern]);
                copied = end;
            }
        }
        out.push_str(&text[copied..]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(role: DeviceRole, id: &str, name: &str, serial: Option<&str>) -> KnownDevice {
        KnownDevice {
            role,
            platform_id: id.into(),
            name: Some(name.into()),
            serial: serial.map(Into::into),
        }
    }

    fn neo() -> Scrub {
        Scrub::new(&[
            device(
                DeviceRole::Trainer,
                "3F2504E0-4F89-41D3-9A0C-0305E82C3301",
                "Tacx Neo 55624",
                Some("2655624"),
            ),
            device(
                DeviceRole::HeartRate,
                "C4:7D:4F:12:34:56",
                "HRM-Pro:9871",
                None,
            ),
        ])
        .expect("table")
    }

    #[test]
    fn known_devices_are_replaced_ignoring_case() {
        let s = neo();
        assert_eq!(
            s.text(
                "connect to TACX NEO 55624 (3f2504e0-4f89-41d3-9a0c-0305e82c3301) failed, 2655624"
            ),
            "connect to <trainer> (<trainer-id>) failed, <trainer-serial>"
        );
        assert_eq!(s.text("strap hrm-pro:9871"), "strap <strap>");
    }

    #[test]
    fn a_shorter_needle_that_starts_a_longer_one_does_not_win() {
        let s = Scrub::new(&[
            device(DeviceRole::HeartRate, "a", "Tacx Neo", None),
            device(DeviceRole::Trainer, "b", "Tacx Neo 55624", None),
        ])
        .expect("table");
        assert_eq!(s.text("lost Tacx Neo 55624"), "lost <trainer>");
    }

    #[test]
    fn normalises_before_the_table() {
        let s = neo();
        for hidden in [
            "Tacx\u{a0}Neo 55624",
            "Tacx\u{200b} Neo 55624",
            "Tac\u{200b}x Neo 55624",
            "Tacx\u{200b}Neo 55624",
            "Tacx  Neo\t55624",
            "Ｔａｃｘ Neo 55624",
            "Tacx Neo 556\u{ad}24",
        ] {
            assert_eq!(
                s.text(&format!("seen {hidden}")),
                "seen <trainer>",
                "{hidden:?}"
            );
        }
    }

    #[test]
    fn every_spelling_of_a_known_identifier_is_replaced() {
        let s = neo();
        for (text, expected) in [
            ("3f2504e04f8941d39a0c0305e82c3301", "<trainer-id>"),
            ("C47D4F123456", "<strap-id>"),
            ("c4-7d-4f-12-34-56", "<strap-id>"),
            (
                "/org/bluez/hci0/dev_C4_7D_4F_12_34_56",
                "/org/bluez/hci0/dev_<strap-id>",
            ),
            ("c4.7d.4f.12.34.56", "<strap-id>"),
            ("c47d.4f12.3456", "<strap-id>"),
            ("0x288588 or 288588", "<trainer-serial> or <trainer-serial>"),
            ("0x00288588", "<trainer-serial>"),
            ("00288588", "<trainer-serial>"),
            ("88852800", "<trainer-serial>"),
            ("[88, 85, 28, 00]", "[<trainer-serial>]"),
            ("be 00 28 85 88", "be <trainer-serial>"),
            ("0x88 0x85 0x28 0x00", "<trainer-serial>"),
            (
                "[136, 133, 40, 0] or [0, 40, 133, 136]",
                "[<trainer-serial>] or [<trainer-serial>]",
            ),
            (
                "2655 624 and 2,655,624",
                "<trainer-serial> and <trainer-serial>",
            ),
            ("Tacx_Neo_55624 or Tacx-Neo-55624", "<trainer> or <trainer>"),
            ("/x/Tacx%20Neo%2055624.fecap", "/x/<trainer>.fecap"),
            ("q=Tacx+Neo+55624", "q=<trainer>"),
            ("HRM-Pro%3A9871", "<strap>"),
        ] {
            assert_eq!(s.text(text), expected, "{text}");
        }
    }

    #[test]
    fn a_serial_with_hex_letters_is_replaced_in_both_byte_orders() {
        // 11259375 = 0xabcdef: its big-endian bytes differ from its little-endian ones.
        let s = Scrub::new(&[device(DeviceRole::Trainer, "x", "KICKR", Some("11259375"))])
            .expect("table");
        for text in [
            "00 ab cd ef",
            "ef cd ab 00",
            "0x00 0xab 0xcd 0xef",
            "0xef 0xcd 0xab 0x00",
            "[0, 171, 205, 239]",
            "[239, 205, 171, 0]",
            "0xabcdef",
        ] {
            assert!(!s.text(text).contains("ab"), "{text} -> {}", s.text(text));
            assert!(!s.text(text).contains("171"), "{text} -> {}", s.text(text));
        }
    }

    #[test]
    fn short_needles_cut_into_nothing() {
        let s = Scrub::new(&[
            device(DeviceRole::Trainer, "x1", "Pro", Some("12")),
            device(DeviceRole::HeartRate, "x2", "N", Some("1")),
        ])
        .expect("table");
        for (text, expected) in [
            ("peer C4:7D:4F:12:34:57", "peer <ble-addr>"),
            ("id 7A1B2C3D-4F12-4000-8000-123456789ABC", "id <ble-id>"),
            ("connected via protocol Pro", "connected via protocol Pro"),
            ("retry 1/3 at 1.2 s, 12:00", "retry 1/3 at 1.2 s, 12:00"),
            ("serial 12", "serial <serial>"),
        ] {
            assert_eq!(s.text(text), expected, "{text}");
        }
        let year =
            Scrub::new(&[device(DeviceRole::Trainer, "x", "KICKR", Some("2026"))]).expect("table");
        assert_eq!(
            year.text("in 2026"),
            "in 2026",
            "a 4-digit serial is a year"
        );
    }

    #[test]
    fn hex_identifiers_are_replaced_whatever_their_separators() {
        let s = Scrub::new(&[]).expect("table");
        for (text, expected) in [
            (
                "peer C4:7d:4F:12:34:57,c4-7d-4f-12-34-58",
                "peer <ble-addr>,<ble-addr>",
            ),
            ("device-C4-7D-4F-12-34-57", "device-<ble-addr>"),
            ("node_C4_7D_4F_12_34_57", "node_<ble-addr>"),
            ("id_7A1B2C3D-0000-4000-8000-123456789ABC_x", "id_<ble-id>_x"),
            ("C4:7D-4F:12-34:57 c47d.4f12.3457", "<ble-id>"),
            ("C47D4F123457", "<ble-addr>"),
            ("7a1b2c3d000040008000123456789abc", "<ble-id>"),
            ("12:34:56:78:90:12", "<ble-addr>"),
            ("C4:7D:4F:12:3\u{200b}4:57", "<ble-addr>"),
            (
                "missing characteristic 6E40FEC2-B5A3-F393-E0A9-E50E24DCCA9E",
                "missing characteristic 6E40FEC2-B5A3-F393-E0A9-E50E24DCCA9E",
            ),
        ] {
            assert_eq!(s.text(text), expected, "{text}");
        }
        for kept in [
            "2026-11-03T21:13:30.619Z",
            "at 2026-11-03 21:13:30.619 took 1.234 s",
            "counter 123456789012",
        ] {
            assert_eq!(s.text(kept), kept);
        }
    }

    #[test]
    fn a_needle_never_matches_inside_a_placeholder() {
        let s = Scrub::new(&[device(DeviceRole::Trainer, "x", "Addr", None)]).expect("table");
        assert_eq!(
            s.text("peer C4:7D:4F:12:34:57, addr"),
            "peer <ble-addr>, <trainer>"
        );
    }

    #[test]
    fn the_value_after_any_serial_label_is_replaced() {
        let s = neo();
        for (text, expected) in [
            ("page 81 serial 7788991", "page 81 serial <serial>"),
            ("serial no. 7788991", "serial no. <serial>"),
            ("serial no. 2655624", "serial no. <serial>"),
            ("serial is 7788991", "serial is <serial>"),
            ("serial of 7788991", "serial of <serial>"),
            ("serial read: X9", "serial read: <serial>"),
            (
                "Serial Number String: 7788991",
                "Serial Number String: <serial>",
            ),
            (r#"serial: Some("2655624")"#, r#"serial: Some("<serial>")"#),
            (r#"serial: "7788991""#, r#"serial: "<serial>""#),
            ("serial_number=7788991", "serial_number=<serial>"),
            ("serial7788991", "serial<serial>"),
            (
                "serial — 7788991; serial → X2",
                "serial — <serial>; serial → <serial>",
            ),
            (
                "serial (X1), serial, X2",
                "serial (<serial>), serial, <serial>",
            ),
            (
                "serial -> X3, serial: [X4]",
                "serial -> <serial>, serial: [<serial>]",
            ),
            ("serial `X5`", "serial `<serial>`"),
            ("S/N: X-1, SN=77", "S/N: <serial>, SN=<serial>"),
            ("the serial is not available", "the serial is not available"),
            ("serial port opened", "serial port opened"),
        ] {
            assert_eq!(s.text(text), expected, "{text}");
        }
    }

    #[test]
    fn name_and_serial_fields_are_replaced_whole() {
        let s = neo();
        let record: Flat = [
            ("name", "tacx\u{a0}neo 55624"),
            ("local_name", "KICKR 1A2B"),
            ("sn", "abc"),
            ("path", "/x/Tacx Neo 55624.fecap"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        let out = s.record(record);
        assert_eq!(out["name"], "<trainer>");
        assert_eq!(out["local_name"], "<device-name>");
        assert_eq!(out["sn"], "<serial>");
        assert_eq!(out["path"], "/x/<trainer>.fecap");
    }
}
