//! Argument parsing and summary writing (offline; no hardware).
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// One HIL subcommand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    /// 30 s scan, advertisement inventory (#206 items 1, 6, 7).
    Scan,
    /// Resolve a remembered id without scanning (#206 item 3).
    Known,
    /// Idle notification pattern with no writes.
    Idle,
    /// Check 1: checksum variants.
    Check1,
    /// Check 2: message type × write-with/without-response.
    Check2,
    /// Check 3: page 70 → 71 semantics and endpoint values (raw).
    Check3,
    /// Check 3 calibration: end-to-end latency through the real driver.
    Latency,
    /// Check 4: pages 54/55/80/81.
    Check4,
    /// Check 3 extension: reply loss against request and control spacing.
    Spacing,
    /// Redact a raw capture for committing.
    Redact,
}

impl Check {
    const ALL: [(&'static str, Self); 10] = [
        ("scan", Self::Scan),
        ("known", Self::Known),
        ("idle", Self::Idle),
        ("check1", Self::Check1),
        ("check2", Self::Check2),
        ("check3", Self::Check3),
        ("latency", Self::Latency),
        ("check4", Self::Check4),
        ("spacing", Self::Spacing),
        ("redact", Self::Redact),
    ];
    /// Subcommand spelling.
    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(_, c)| *c == self)
            .map_or("?", |(n, _)| n)
    }
}

/// Parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    /// Subcommand.
    pub check: Check,
    /// Output directory for `<check>.fecap` and `<check>.json`.
    pub out: PathBuf,
    /// Scan duration in seconds.
    pub scan_secs: u64,
    /// Idle duration in seconds.
    pub idle_secs: u64,
    /// Platform id (`known`) or capture path (`redact`).
    pub target: Option<String>,
}

/// Usage text.
pub const USAGE: &str = "usage: hil_fec <scan|known|idle|check1|check2|check3|latency|check4|spacing|redact> \
[--out DIR] [--scan-secs N] [--idle-secs N] [--target ID_OR_PATH]";

/// Parse arguments (without the program name).
pub fn parse<I: IntoIterator<Item = String>>(args: I, default_out: &Path) -> Result<Args, String> {
    let mut it = args.into_iter();
    let name = it.next().ok_or_else(|| USAGE.to_string())?;
    let check = Check::ALL
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, c)| *c)
        .ok_or_else(|| format!("unknown check {name:?}\n{USAGE}"))?;
    let mut out = Args {
        check,
        out: default_out.to_path_buf(),
        scan_secs: 30,
        idle_secs: 60,
        target: None,
    };
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--out" => out.out = PathBuf::from(value()?),
            "--scan-secs" => {
                out.scan_secs = value()?.parse().map_err(|e| format!("--scan-secs: {e}"))?
            }
            "--idle-secs" => {
                out.idle_secs = value()?.parse().map_err(|e| format!("--idle-secs: {e}"))?
            }
            "--target" => out.target = Some(value()?),
            other => return Err(format!("unknown flag {other:?}\n{USAGE}")),
        }
    }
    Ok(out)
}

/// Write `<check>.json` (pretty, trailing newline) into `dir`, creating it.
pub fn write_summary(dir: &Path, check: Check, summary: &Value) -> std::io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.json", check.name()));
    let mut text = serde_json::to_string_pretty(summary).map_err(std::io::Error::other)?;
    text.push('\n');
    fs::write(&path, text)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn parses_defaults_and_flags() {
        let a = parse(args(&["check1"]), Path::new("/o")).expect("parses");
        assert_eq!(
            (a.check, a.scan_secs, a.idle_secs, a.out.as_path()),
            (Check::Check1, 30, 60, Path::new("/o"))
        );
        let a = parse(
            args(&["idle", "--idle-secs", "5", "--out", "/x", "--target", "t"]),
            Path::new("/o"),
        )
        .expect("parses");
        assert_eq!(
            (a.idle_secs, a.out, a.target),
            (5, PathBuf::from("/x"), Some("t".into()))
        );
    }

    #[test]
    fn rejects_bad_input() {
        let d = Path::new("/o");
        assert!(parse(args(&[]), d).is_err());
        assert!(parse(args(&["bogus"]), d).is_err());
        assert!(parse(args(&["scan", "--scan-secs"]), d).is_err());
        assert!(parse(args(&["scan", "--scan-secs", "x"]), d).is_err());
        assert!(parse(args(&["scan", "--nope", "1"]), d).is_err());
    }

    #[test]
    fn writes_summary_next_to_capture_name() {
        let dir = tempfile::tempdir().expect("tmp");
        let p = write_summary(&dir.path().join("sub"), Check::Check2, &json!({"ok": true}))
            .expect("writes");
        assert_eq!(p.file_name().and_then(|n| n.to_str()), Some("check2.json"));
        let back: Value =
            serde_json::from_str(&fs::read_to_string(p).expect("read")).expect("json");
        assert_eq!(back, json!({"ok": true}));
    }
}
