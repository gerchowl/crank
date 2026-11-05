//! The Part A checks (#44 checks 1–4, idle pattern, #204/#206 tentative items).
use crate::{
    cli::{Args, Check, write_summary},
    frames::{
        Checksum, analysis, build, command_status, distribution, hex, page_of, page_rates,
        parse_capture, parse_cps, parse_notes, redact_capture, redact_frame, redact_name,
        request_latencies, with_page_byte,
    },
    partb, rider, session,
    session::{Exchange, Result, Session, adapter, find_trainer, scan},
};
use crank_ble::{
    BtleplugAdvertisements, CPS_SERVICE, FEC_SERVICE, FTMS_SERVICE, FecTrainer, HRS_SERVICE,
};
use crank_devices::{
    Clock, ControlMode, DeviceRef, ObservationSource, ObservedControl, RiderConfig, TrainerCommand,
    TrainerDriver, TrainerEvent, Wind,
};
use crank_fec::{ControlPage, Page, decode};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::time::{Instant, sleep, timeout_at};

/// How a run ended.
pub enum Outcome {
    /// Summary written at this path.
    Done(PathBuf),
    /// Precondition missing; the owner must act.
    Blocked(String),
    /// A Part B procedure stopped early (guard, signal, failure): why, with whether the release
    /// was confirmed ([`rider::stopped`]); the summary is at this path.
    Stopped(PathBuf, String),
    /// A Part B procedure completed but its release was not confirmed (e.g. after a drop): the
    /// error and the rider instruction; the summary is at this path.
    Unconfirmed(PathBuf, String),
}

impl Outcome {
    /// The outcome of a Part B run whose summary is at `path`.
    fn of(path: PathBuf, ending: rider::Ending) -> Self {
        match ending {
            rider::Ending::Released => Self::Done(path),
            rider::Ending::Stopped(why) => Self::Stopped(path, why),
            rider::Ending::Unconfirmed(why) => Self::Unconfirmed(path, why),
        }
    }

    /// The lines `hil_fec` prints, the last one the operator reads, and its exit status:
    /// 0 done, 3 blocked, 4 stopped early (no result), 5 completed with the release NOT
    /// confirmed (`main.rs` adds 1 for an error and 2 for bad arguments).
    pub fn lines(&self) -> (Vec<String>, u8) {
        match self {
            Self::Done(path) => (vec![format!("wrote {}", path.display())], 0),
            Self::Blocked(why) => (vec![format!("BLOCKED: {why}")], 3),
            Self::Stopped(path, why) => {
                (vec![format!("STOPPED: {why}; wrote {}", path.display())], 4)
            }
            Self::Unconfirmed(path, why) => (
                vec![
                    format!("wrote {}", path.display()),
                    format!("DONE, but the release was NOT confirmed: {why}"),
                ],
                5,
            ),
        }
    }
}

const BLOCKED_NO_NEO: &str = "NEO not advertising — owner: power the trainer (mains plugged in) and keep the phone/Tacx app disconnected";
const XOR: Checksum = Checksum::XorInclSync;
const ACK: u8 = 0x4f;
const BROADCAST: u8 = 0x4e;

/// Dispatch one subcommand.
pub async fn run(args: &Args) -> Result<Outcome> {
    match args.check {
        Check::Redact => return redact(args),
        Check::Analyse => return analyse(args),
        Check::Scan => return scan_check(args).await,
        Check::Known => return known(args).await,
        Check::Reuse => return crate::reuse::run(args).await,
        _ => {}
    }
    let adapter = adapter().await?;
    let adv = BtleplugAdvertisements::new(adapter.clone());
    if args.check == Check::Check11c {
        return cold(args, &adv, adapter).await;
    }
    let Some((device, _)) = find_trainer(&adv, args.scan_secs).await else {
        return Ok(Outcome::Blocked(BLOCKED_NO_NEO.into()));
    };
    std::fs::create_dir_all(&args.out)?;
    let capture_path = args.out.join(format!("{}.fecap", args.check.name()));
    let mut ending = rider::Ending::Released;
    let summary = if args.check == Check::Latency {
        latency(args, &adv, adapter, device, &capture_path).await?
    } else if let Some(p) = args.check.procedure() {
        let mut s = Session::open(&adv, adapter, device, &capture_path).await?;
        if p == rider::Procedure::Check5
            && let Err(e) = s.subscribe_cps().await
        {
            // Nothing was applied yet; release anyway, so that a hung setup never leaves the
            // trainer in a state this run did not choose.
            let released = rider::release(&s).await;
            s.close().await;
            let released = released.map_or(String::new(), |r| format!("; release: {r}"));
            return Err(
                format!("check 5 needs Cycling Power 2A63 on the NEO: {e}{released}").into(),
            );
        }
        let (summary, e) = ride(args, p, s, &rider::Console::new()).await;
        ending = e;
        summary
    } else {
        let s = Session::open(&adv, adapter, device, &capture_path).await?;
        let summary = match args.check {
            Check::Idle => idle(&s, args).await,
            Check::Check1 => check1(&s).await,
            Check::Check2 => check2(&s).await,
            Check::Check3 => check3(&s).await,
            Check::Check4 => check4(&s).await,
            Check::Spacing => spacing(&s).await,
            Check::Scan
            | Check::Known
            | Check::Latency
            | Check::Redact
            | Check::Analyse
            | Check::Check5
            | Check::Check6
            | Check::Check7
            | Check::Check8
            | Check::Check9
            | Check::Check10
            | Check::Check11a
            | Check::Check11b
            | Check::Check11c
            | Check::Check12
            | Check::Reuse => {
                unreachable!()
            }
        };
        let ended = s.pump_ended();
        s.close().await;
        let mut summary = summary;
        summary["link_dropped_during_run"] = json!(ended);
        summary
    };
    let path = write_summary(&args.out, args.check, &summary)?;
    Ok(Outcome::of(path, ending))
}

/// Drive procedure `p` on `s`, close the link and summarise the run from its capture; the
/// summary and how the run ended ([`rider::ending`]).
async fn ride(
    args: &Args,
    p: rider::Procedure,
    s: Session,
    op: &rider::Console,
) -> (Value, rider::Ending) {
    let report = rider::drive(&s, op, p, rider::signals()).await;
    let identity = identity(&s);
    let dropped = s.pump_ended();
    let capture_path = s.capture_path.clone();
    s.close().await;
    // Read the capture only now that nothing writes it: the row is then exactly what
    // `analyse` reads from the file later.
    let mut summary = procedure_summary(p, args.check.name(), &report, &capture_path, unix_now());
    summary["identity"] = identity;
    summary["link_dropped_during_run"] = json!(dropped);
    if let Some(line) = summary["result"]["line"].as_str() {
        println!("{line}");
    }
    (summary, rider::ending(&report))
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Check 11c: the capture records from before the link ([`rider::cold_start`]: the start cue,
/// the trainer's first FE-C advertisement), then the connect and the procedure as for the other
/// checks. A cold start that stops before the link has written nothing, so it releases nothing.
async fn cold(
    args: &Args,
    adv: &BtleplugAdvertisements,
    adapter: btleplug::platform::Adapter,
) -> Result<Outcome> {
    use crank_ble::MonotonicClock;
    std::fs::create_dir_all(&args.out)?;
    let capture_path = args.out.join(format!("{}.fecap", args.check.name()));
    let clock = Arc::new(MonotonicClock::default());
    let started = clock.now();
    let capture = Arc::new(crank_ble::Capture::create(&capture_path, started)?);
    let op = rider::Console::new();
    let opened = rider::cold_start(
        &op,
        || clock.now().0.saturating_sub(started.0),
        |tag, bytes| {
            capture
                .record_aux(clock.now(), tag, bytes)
                .err()
                .map(|e| e.to_string())
        },
        session::fec_trainers(adv),
        |device| {
            let parts = (clock.clone(), started, capture.clone());
            async {
                Session::open_on(adv, adapter, device, &capture_path, parts)
                    .await
                    .map_err(|e| e.to_string())
            }
        },
    )
    .await;
    let p = rider::Procedure::Check11c;
    let (summary, ending) = match opened {
        Ok(s) => ride(args, p, s, &op).await,
        Err(stop) => {
            let report = rider::Report {
                procedure: Err(stop),
                release_error: Some("not connected: nothing was written".into()),
            };
            drop(capture);
            let summary =
                procedure_summary(p, args.check.name(), &report, &capture_path, unix_now());
            let why = report.procedure.err().map_or_else(String::new, |e| {
                format!("{e:?} (nothing was written to the trainer)")
            });
            (summary, rider::Ending::Stopped(why))
        }
    };
    let path = write_summary(&args.out, args.check, &summary)?;
    Ok(Outcome::of(path, ending))
}

/// The summary of a finished procedure whose link is closed and whose capture is at
/// `capture_path`, finished at `finished_unix_s`. The capture gets its outcome line and finished stamp first, so that check 9's
/// verdict reads check 6 ([`partb::PowerLag::beside`]) from the capture exactly as `analyse` will
/// later: from the same trainer (page-81 serial) and session (finished stamps), else not measured.
fn procedure_summary(
    p: rider::Procedure,
    stem: &str,
    report: &rider::Report,
    capture_path: &std::path::Path,
    finished_unix_s: u64,
) -> Value {
    // The outcome line lets `analyse` tell a completed run from a stopped one.
    let appended = partb::append_outcome(
        capture_path,
        &rider::outcome(&report.procedure),
        finished_unix_s,
    );
    let (text, lag) = match std::fs::read_to_string(capture_path) {
        Ok(text) => {
            let lag = partb::PowerLag::beside(capture_path, &text);
            (text, lag)
        }
        Err(e) => (
            String::new(),
            partb::PowerLag::NotMeasured(format!("{}: {e}", capture_path.display())),
        ),
    };
    let mut summary = rider::summary(
        p,
        stem,
        report,
        &parse_capture(&text),
        &parse_cps(&text),
        &parse_notes(&text),
        &lag,
    );
    if let Err(e) = appended {
        summary["outcome_line_error"] = json!(e.to_string());
    }
    summary
}

fn redact(args: &Args) -> Result<Outcome> {
    let src = args
        .target
        .as_deref()
        .ok_or("redact needs --target <raw .fecap>")?;
    let text = std::fs::read_to_string(src)?;
    std::fs::create_dir_all(&args.out)?;
    let name = std::path::Path::new(src)
        .file_name()
        .ok_or("bad --target")?;
    let dest = args.out.join(name);
    let header = "# Real Tacx NEO capture (HIL #44); page-81 serial redacted to FF FF FF FF.\n";
    std::fs::write(&dest, format!("{header}{}", redact_capture(&text)))?;
    Ok(Outcome::Done(dest))
}

/// Offline: reply accounting and stale-echo summary of a committed capture.
fn analyse(args: &Args) -> Result<Outcome> {
    let src = args
        .target
        .as_deref()
        .ok_or("analyse needs --target <.fecap>")?;
    let text = std::fs::read_to_string(src)?;
    let stem = std::path::Path::new(src)
        .file_stem()
        .and_then(|n| n.to_str())
        .ok_or("bad --target")?;
    std::fs::create_dir_all(&args.out)?;
    let dest = args.out.join(format!("{stem}.analysis.json"));
    let records = parse_capture(&text);
    let mut v = analysis(&records);
    let lag = partb::PowerLag::beside(std::path::Path::new(src), &text);
    let cps = parse_cps(&text);
    if let Some(part_b) = partb::analyse(
        stem,
        &records,
        &cps,
        &parse_notes(&text),
        partb::outcome_in(&text).as_deref(),
        &lag,
    ) {
        if let Some(line) = part_b["line"].as_str() {
            println!("{line}");
        }
        v["part_b"] = part_b;
    }
    let mut body = serde_json::to_string_pretty(&v)?;
    body.push('\n');
    std::fs::write(&dest, body)?;
    Ok(Outcome::Done(dest))
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

async fn scan_check(args: &Args) -> Result<Outcome> {
    let adapter = adapter().await?;
    let adv = BtleplugAdvertisements::new(adapter);
    let seen = scan(&adv, args.scan_secs, false).await;
    let names = [
        (FEC_SERVICE, "fec1"),
        (FTMS_SERVICE, "ftms"),
        (CPS_SERVICE, "cps"),
        (HRS_SERVICE, "hrs"),
    ];
    let devices: Vec<Value> = seen
        .iter()
        .filter(|(_, s)| names.iter().any(|(u, _)| s.services.contains(u)))
        .map(|(id, s)| {
            json!({
                "id_prefix": short_id(id),
                "name": s.name.as_deref().map(redact_name),
                "adverts": s.adverts,
                "supported_services": names.iter().filter(|(u, _)| s.services.contains(u)).map(|(_, n)| n).collect::<Vec<_>>(),
                "advertised_services": s.services.iter().map(ToString::to_string).collect::<Vec<_>>(),
            })
        })
        .collect();
    let summary = json!({
        "scan_secs": args.scan_secs,
        "peripherals_seen": seen.len(),
        "supported": devices,
        "neo_found": seen.values().any(|s| s.services.contains(&FEC_SERVICE)),
    });
    let path = write_summary(&args.out, Check::Scan, &summary)?;
    if summary["neo_found"] == json!(false) {
        return Ok(Outcome::Blocked(BLOCKED_NO_NEO.into()));
    }
    Ok(Outcome::Done(path))
}

/// #206 item 3: can a remembered platform id be resolved without a prior scan?
async fn known(args: &Args) -> Result<Outcome> {
    let id = DeviceRef(
        args.target
            .clone()
            .ok_or("known needs --target <platform id>")?,
    );
    let adapter = adapter().await?;
    let adv = BtleplugAdvertisements::new(adapter);
    let before = adv
        .peripheral(&id)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string());
    let seen = scan(&adv, args.scan_secs, false).await;
    let after = adv
        .peripheral(&id)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string());
    let summary = json!({
        "resolved_without_scan": before.is_ok(),
        "error_without_scan": before.err(),
        "seen_in_scan": seen.contains_key(&id.0),
        "resolved_after_scan": after.is_ok(),
        "error_after_scan": after.err(),
    });
    Ok(Outcome::Done(write_summary(
        &args.out,
        Check::Known,
        &summary,
    )?))
}

fn reserved_bits_page16(records: &[crate::frames::Record]) -> Value {
    let seen: std::collections::BTreeSet<u8> = records
        .iter()
        .filter(|r| r.rx && page_of(&r.bytes) == Some(16))
        .map(|r| r.bytes[5] >> 5)
        .collect();
    json!({ "page16_byte1_bits_5_to_7_values_seen": seen })
}

fn identity(s: &Session) -> Value {
    json!({
        "name": s.info.name.as_deref().map(redact_name),
        "model": s.info.model,
        "firmware": s.info.firmware,
        "dis_serial_present": s.info.serial.is_some(),
        "id_prefix": short_id(&s.device.0),
    })
}

fn read_capture(s: &Session) -> Vec<crate::frames::Record> {
    // Capture flushes per record; re-read what this session wrote.
    std::fs::read_to_string(capture_file(s))
        .map(|t| parse_capture(&t))
        .unwrap_or_default()
}

fn capture_file(s: &Session) -> PathBuf {
    s.capture_path.clone()
}

async fn idle(s: &Session, args: &Args) -> Value {
    sleep(Duration::from_secs(args.idle_secs)).await;
    let records = read_capture(s);
    json!({
        "check": "idle",
        "what_was_sent": "nothing (subscribe fec2 only; no writes)",
        "identity": identity(s),
        "duration_s": args.idle_secs,
        "frames_rx": records.iter().filter(|r| r.rx).count(),
        "page_rates": page_rates(&records),
        "reserved": reserved_bits_page16(&records),
    })
}

fn request(page: u8, times: u8, msg: u8, sum: Checksum) -> [u8; 13] {
    build(&ControlPage::Request { page, times }, msg, sum).expect("request encodes")
}

fn exchange_json(label: &str, x: &Exchange) -> Value {
    let status = x.reply.as_ref().and_then(|r| command_status(&r.bytes));
    json!({
        "label": label,
        "tx": hex(&x.sent),
        "write_error": x.write_error,
        "rx": x.reply.as_ref().map(|r| hex(&redact_frame(&r.bytes))),
        "latency_us": x.latency_us,
        "p71": status.map(|(last, seq, st, echo)| json!({
            "last_page": last, "sequence": seq, "status": st, "echo": hex(&echo),
            "echo_equals_sent_bytes_4_7": echo == [x.sent[8], x.sent[9], x.sent[10], x.sent[11]],
        })),
    })
}

const REPLY_WAIT: Duration = Duration::from_secs(2);

async fn check1(s: &Session) -> Value {
    let mut trials = Vec::new();
    for round in 0..3 {
        for sum in Checksum::ALL {
            let x = s
                .exchange(request(54, 1, ACK, sum), true, 54, REPLY_WAIT)
                .await;
            let mut t = exchange_json(sum.name(), &x);
            t["round"] = json!(round);
            t["accepted"] = json!(x.reply.is_some());
            trials.push(t);
            sleep(Duration::from_millis(500)).await;
        }
    }
    let accepted = |sum: Checksum| {
        trials
            .iter()
            .filter(|t| t["label"] == sum.name() && t["accepted"] == json!(true))
            .count()
    };
    json!({
        "check": 1,
        "what_was_sent": "page 70 request for page 54, msg 0x4F, write-with-response, four checksum rules x3 rounds",
        "accepted_of_3": Checksum::ALL.iter().map(|c| (c.name(), accepted(*c))).collect::<std::collections::BTreeMap<_, _>>(),
        "identity": identity(s),
        "trials": trials,
    })
}

async fn check2(s: &Session) -> Value {
    let mut trials = Vec::new();
    let mut watts = 100.0_f32;
    for round in 0..3 {
        for (msg, with_response) in [
            (ACK, true),
            (ACK, false),
            (BROADCAST, true),
            (BROADCAST, false),
        ] {
            let cell = format!(
                "msg_{msg:02X}_{}",
                if with_response { "resp" } else { "noresp" }
            );
            let x = s
                .exchange(request(54, 1, msg, XOR), with_response, 54, REPLY_WAIT)
                .await;
            let mut t = exchange_json(&format!("{cell}/request54"), &x);
            t["accepted"] = json!(x.reply.is_some());
            trials.push(t);
            sleep(Duration::from_millis(300)).await;
            // Control effect: this envelope's page 49, then the page-71 echo.
            watts += 10.0;
            let control = build(&ControlPage::TargetPower { watts }, msg, XOR).expect("encodes");
            let (_, err) = s.send(control, with_response).await;
            sleep(Duration::from_millis(300)).await;
            let y = s
                .exchange(request(71, 1, ACK, XOR), true, 71, REPLY_WAIT)
                .await;
            let mut t = exchange_json(&format!("{cell}/control49_then_71"), &y);
            t["control_tx"] = json!(hex(&control));
            t["control_write_error"] = json!(err);
            t["round"] = json!(round);
            t["echo_matches_control"] = json!(
                y.reply
                    .as_ref()
                    .and_then(|r| command_status(&r.bytes))
                    .is_some_and(|(last, _, _, e)| {
                        last == 49 && e == [control[8], control[9], control[10], control[11]]
                    })
            );
            trials.push(t);
            sleep(Duration::from_millis(300)).await;
        }
    }
    park(s).await;
    json!({
        "check": 2,
        "what_was_sent": "page 70 request(54) and page 49 target + request(71) under each of msg 0x4F/0x4E x write with/without response, XOR checksum, x3 rounds",
        "identity": identity(s),
        "trials": trials,
    })
}

/// Leave the trainer at basic resistance 0 % rather than a leftover target.
async fn park(s: &Session) {
    let f = build(&ControlPage::BasicResistance { pct: 0.0 }, ACK, XOR).expect("encodes");
    let _ = s.send(f, true).await;
}

/// Page bytes `(index, value)` patched over an encoded frame, for wire values the codec
/// refuses to produce (page 49 `0xFFFF`, page 51 Crr `0xFF`, #215).
type Overrides = &'static [(usize, u8)];

async fn control_then_status(
    s: &Session,
    label: &str,
    page: &ControlPage,
    overrides: Overrides,
) -> Value {
    let Some(mut control) = build(page, ACK, XOR) else {
        return json!({"label": label, "encode_error": true});
    };
    for &(index, value) in overrides {
        control = with_page_byte(control, index, value, XOR);
    }
    let (_, err) = s.send(control, true).await;
    sleep(Duration::from_millis(300)).await; // > one 4 Hz tick
    let mut x = s
        .exchange(request(71, 1, ACK, XOR), true, 71, REPLY_WAIT)
        .await;
    // An unanswered request is not a rejection: retry once, recording that it was needed.
    let retried = x.reply.is_none();
    if retried {
        x = s
            .exchange(request(71, 1, ACK, XOR), true, 71, REPLY_WAIT)
            .await;
    }
    let mut t = exchange_json(label, &x);
    t["control_tx"] = json!(hex(&control));
    t["control_write_error"] = json!(err);
    t["request_retried_after_no_reply"] = json!(retried);
    if let Some((last, _, _, echo)) = x.reply.as_ref().and_then(|r| command_status(&r.bytes)) {
        t["p71"]["echo_equals_control_bytes_4_7"] =
            json!(echo == [control[8], control[9], control[10], control[11]]);
        t["p71"]["last_page_is_control_page"] = json!(last == control[4]);
    }
    t
}

/// Reply loss against request spacing (no control) and against control→request delay.
async fn spacing(s: &Session) -> Value {
    for gap_ms in [250_u64, 500, 1000, 2000] {
        for _ in 0..20 {
            let _ = s.send(request(71, 1, ACK, XOR), true).await;
            sleep(Duration::from_millis(gap_ms)).await;
        }
        sleep(Duration::from_millis(1500)).await;
    }
    for delay_ms in [100_u64, 250, 500, 1000] {
        for i in 0..10 {
            let w = if i % 2 == 0 { 100.0 } else { 110.0 };
            let control = build(&ControlPage::TargetPower { watts: w }, ACK, XOR).expect("encodes");
            let _ = s.send(control, true).await;
            sleep(Duration::from_millis(delay_ms)).await;
            let _ = s.send(request(71, 1, ACK, XOR), true).await;
            sleep(Duration::from_millis(1500)).await;
        }
    }
    sleep(Duration::from_millis(1500)).await;
    park(s).await;
    // Counted from the capture by sequence number (frames::reply_accounting), not by
    // number of page-71 frames: the NEO repeats some replies as 0x4E copies.
    json!({
        "check": "3-spacing",
        "what_was_sent": "20 page-70 requests for page 71 at 250/500/1000/2000 ms; page 49 then request(71) after 100/250/500/1000 ms x10",
        "identity": identity(s),
        "accounting": analysis(&read_capture(s)),
    })
}

/// Wire bytes patched over a case's encoded frame: the NEO's invalid sentinels, which
/// `encode` rejects (#215), are sent to keep re-measuring the settled behaviour.
fn overrides(label: &str) -> Overrides {
    match label {
        "track_crr_0xFF" => &[(7, 0xff)],
        "erg_16383_75W_0xFFFF" => &[(6, 0xff)],
        _ => &[],
    }
}

/// Nominal and endpoint control values for check 3 (#44 pre-mortem, #146, #204).
fn value_cases() -> Vec<(&'static str, ControlPage)> {
    vec![
        ("erg_100W", ControlPage::TargetPower { watts: 100.0 }),
        ("erg_200_25W", ControlPage::TargetPower { watts: 200.25 }),
        ("erg_200_5W", ControlPage::TargetPower { watts: 200.5 }),
        (
            "track_0pct_crr0.004",
            ControlPage::Track {
                grade_pct: 0.0,
                crr: 0.004,
            },
        ),
        (
            "track_5pct_crr0.004",
            ControlPage::Track {
                grade_pct: 5.0,
                crr: 0.004,
            },
        ),
        (
            "track_crr_0xFF",
            // The largest valid Crr (0xFE); `overrides` patches byte 7 to 0xFF.
            ControlPage::Track {
                grade_pct: 0.0,
                crr: 0.0127,
            },
        ),
        (
            "wind_default",
            ControlPage::Wind {
                cw_kg_per_m: 0.51,
                speed_kmh: 0.0,
                drafting: 1.0,
            },
        ),
        (
            "wind_max_1.86_+127_1.0",
            ControlPage::Wind {
                cw_kg_per_m: 1.86,
                speed_kmh: 127.0,
                drafting: 1.0,
            },
        ),
        (
            "resistance_50pct",
            ControlPage::BasicResistance { pct: 50.0 },
        ),
        (
            "resistance_100pct_0xC8",
            ControlPage::BasicResistance { pct: 100.0 },
        ),
        (
            "user_config_75kg_10kg_708mm",
            ControlPage::UserConfig {
                rider_kg: 75.0,
                bike_kg: 10.0,
                wheel_m: 0.708,
            },
        ),
        (
            "erg_16383_75W_0xFFFF",
            // The largest valid target (0xFFFE); `overrides` patches byte 6 to 0xFF.
            ControlPage::TargetPower { watts: 16_383.5 },
        ),
    ]
}

/// Page-70 transmission count: replies counted within 4 s for `times` 1, 0 and 3.
async fn transmission_counts(s: &Session) -> Vec<Value> {
    let mut counts = Vec::new();
    for times in [1_u8, 0, 3] {
        let mark = s.mark();
        let (_, err) = s.send(request(54, times, ACK, XOR), true).await;
        sleep(Duration::from_secs(4)).await;
        let n = s
            .since(mark)
            .iter()
            .filter(|r| page_of(&r.bytes) == Some(54))
            .count();
        counts.push(json!({"times": times, "write_error": err, "page54_replies_in_4s": n}));
    }
    counts
}

async fn check3(s: &Session) -> Value {
    let mut steps = Vec::new();
    // A: page 71 before any control page.
    for i in 0..2 {
        let x = s
            .exchange(request(71, 1, ACK, XOR), true, 71, REPLY_WAIT)
            .await;
        steps.push(exchange_json(&format!("idle_request71_{i}"), &x));
    }
    let counts = transmission_counts(s).await;
    // C: values and endpoints.
    let cases = value_cases();
    for (label, page) in &cases {
        steps.push(control_then_status(s, label, page, overrides(label)).await);
        sleep(Duration::from_millis(300)).await;
    }
    // D: does the sequence byte advance per control page / per request?
    let mut per_control = Vec::new();
    for i in 0..8 {
        let w = if i % 2 == 0 { 100.0 } else { 110.0 };
        let t = control_then_status(
            s,
            "seq_per_control",
            &ControlPage::TargetPower { watts: w },
            &[],
        )
        .await;
        per_control.push(t["p71"]["sequence"].clone());
    }
    let mut per_request = Vec::new();
    for _ in 0..8 {
        let x = s
            .exchange(request(71, 1, ACK, XOR), true, 71, REPLY_WAIT)
            .await;
        per_request.push(json!(
            x.reply
                .as_ref()
                .and_then(|r| command_status(&r.bytes))
                .map(|c| c.1)
        ));
        sleep(Duration::from_millis(300)).await;
    }
    park(s).await;
    json!({
        "check": 3,
        "what_was_sent": "page 70 request(71); page 70 times 1/0/3; pages 49/51/50/48/55 at nominal and endpoint values each followed by request(71); 8 alternating page-49 writes; 8 bare requests",
        "identity": identity(s),
        "steps": steps,
        "page70_transmission_count": counts,
        "sequence_per_control_write": per_control,
        "sequence_per_bare_request": per_request,
    })
}

async fn check4(s: &Session) -> Value {
    let mut replies = Vec::new();
    let config = ControlPage::UserConfig {
        rider_kg: 75.0,
        bike_kg: 10.0,
        wheel_m: 0.708,
    };
    let frame = build(&config, ACK, XOR).expect("encodes");
    let (_, err) = s.send(frame, true).await;
    replies.push(json!({"label": "send_page55_708mm", "tx": hex(&frame), "write_error": err}));
    sleep(Duration::from_millis(300)).await;
    for (page, reps) in [(54_u8, 3), (80, 3), (81, 3), (55, 3), (16, 1), (25, 1)] {
        for i in 0..reps {
            let reply = s
                .exchange(request(page, 1, ACK, XOR), true, page, REPLY_WAIT)
                .await;
            let mut entry = exchange_json(&format!("request_page_{page}_{i}"), &reply);
            if let Some(r) = &reply.reply {
                let bytes = &r.bytes;
                entry["decoded"] = json!(
                    decode(&redact_frame(bytes))
                        .ok()
                        .map(|f| format!("{:?}", f.page))
                );
                match decode(bytes).map(|f| f.page) {
                    Ok(Page::Capabilities(c)) => {
                        entry["max_resistance_raw"] = json!(c.max_resistance_n);
                        entry["max_resistance_is_0xFFFF"] = json!(c.max_resistance_n == 0xffff);
                        entry["modes_byte"] = json!(c.modes);
                    }
                    Ok(Page::UserConfig { .. }) => {
                        entry["wheel_offset_nibble"] = json!(bytes[8] & 0x0f);
                        entry["wheel_byte_6"] = json!(bytes[10]);
                    }
                    _ => {}
                }
            }
            replies.push(entry);
            sleep(Duration::from_millis(300)).await;
        }
    }
    // #146: offsets 10-14 are outside the codec's 0-9 mm domain, so patch the byte.
    // Reply to each is the NEO's stored configuration (page 55 request).
    let mut offsets = Vec::new();
    for nibble in [0x0a_u8, 0x0c, 0x0e, 0x0f] {
        let patched = with_page_byte(frame, 4, (frame[8] & 0xf0) | nibble, XOR);
        let (_, err) = s.send(patched, true).await;
        sleep(Duration::from_millis(400)).await;
        let reply = s
            .exchange(request(55, 1, ACK, XOR), true, 55, REPLY_WAIT)
            .await;
        offsets.push(json!({
            "sent_nibble": nibble, "tx": hex(&patched), "write_error": err,
            "rx": reply.reply.as_ref().map(|r| hex(&r.bytes)),
            "reported_nibble": reply.reply.as_ref().map(|r| r.bytes[8] & 0x0f),
        }));
        sleep(Duration::from_millis(400)).await;
    }
    let _ = s.send(frame, true).await; // restore the 708 mm configuration
    let records = read_capture(s);
    json!({
        "check": 4,
        "page55_offset_nibble_probe": offsets,
        "what_was_sent": "page 55 (75 kg, 10 kg, 708 mm), then page 70 requests for 54, 80, 81, 55, 16, 25",
        "identity": identity(s),
        "exchanges": replies,
        "reserved": reserved_bits_page16(&records),
    })
}

/// Deterministic 0–999 ms spread of change times so changes sample every phase of the
/// 1 s poll and 2 s SIM alternation grids instead of one fixed phase.
fn jitter(i: u64) -> Duration {
    Duration::from_millis(i * 173 % 1000)
}

fn erg(w: u16) -> ControlMode {
    ControlMode::Erg { target_w: w }
}

/// The driver link plus what it reported, for the calibration run.
struct Run {
    link: crank_devices::TrainerLink,
    clock: Arc<crank_ble::MonotonicClock>,
    observed: Vec<(u64, ObservedControl)>,
    faults: Vec<String>,
}

impl Run {
    /// Collect driver events for `wait`.
    async fn drain(&mut self, wait: Duration) {
        let until = Instant::now() + wait;
        while let Ok(Some(event)) = timeout_at(until, self.link.events.recv()).await {
            match event {
                TrainerEvent::Observed(o) if o.source == ObservationSource::Page71Echo => {
                    if let Some(c) = o.control {
                        self.observed.push((o.at.0, c));
                    }
                }
                TrainerEvent::Fault(e) => self.faults.push(e.to_string()),
                _ => {}
            }
        }
    }

    /// Issue `mode` now, then collect for `wait`; returns the issue time.
    async fn change(&mut self, mode: ControlMode, wait: Duration) -> Result<u64> {
        use crank_devices::Clock;
        let issued = self.clock.now().0;
        self.link
            .control
            .send(TrainerCommand::SetMode {
                mode,
                seq: self.link.seq.next(),
            })
            .await?;
        self.drain(wait).await;
        Ok(issued)
    }
}

/// Check 3 calibration through the real driver and writer (ADR-0002 §4.4.1 `L`, SIM alternation).
async fn latency(
    args: &Args,
    adv: &BtleplugAdvertisements,
    adapter: btleplug::platform::Adapter,
    device: DeviceRef,
    capture_path: &std::path::Path,
) -> Result<Value> {
    use crank_ble::{BtleplugTransport, Capture, MonotonicClock};
    use crank_devices::Clock;
    tracing::subscriber::set_global_default(crate::trace::TraceAll)?;
    let clock = Arc::new(MonotonicClock::default());
    let peripheral = adv.peripheral(&device).await?;
    let capture = Arc::new(Capture::create(capture_path, clock.now())?);
    let trainer = FecTrainer::new(
        Arc::new(BtleplugTransport::new(peripheral, adapter)),
        clock.clone(),
    )
    .with_capture(capture);
    let rider = RiderConfig {
        rider_kg: 75.0,
        bike_kg: 10.0,
        wheel_m: 0.708,
    };
    let link = trainer.connect(&device, rider).await?;
    let caps = format!("{:?}", link.caps);
    let mut run = Run {
        link,
        clock,
        observed: Vec::new(),
        faults: Vec::new(),
    };
    run.drain(Duration::from_secs(3)).await;
    // ERG alternation: 100 <-> 150 W.
    let mut erg_changes = Vec::new();
    for i in 0..60_u16 {
        let target = if i % 2 == 0 { 150 } else { 100 };
        let wait = Duration::from_secs(3) + jitter(i.into());
        let issued = run.change(erg(target), wait).await?;
        erg_changes.push((
            issued,
            ObservedControl::Erg {
                quarter_w: target * 4,
            },
        ));
    }
    // SIM: alternate grade 1 % / 2 %. Only the track component changes and is calibrated;
    // wind stays at its default (its calibration is a Part B item, docs/hil/2026-10-14-fec.md).
    let mut sim_changes = Vec::new();
    let sim_start = run.observed.len();
    for i in 0..30_u8 {
        let (grade_pct, centi) = if i % 2 == 0 { (2.0, 200) } else { (1.0, 100) };
        let mode = ControlMode::Sim {
            grade_pct,
            crr: 0.004,
            wind: Wind::default(),
        };
        let wait = Duration::from_secs(4) + jitter(i.into());
        let issued = run.change(mode, wait).await?;
        sim_changes.push((
            issued,
            ObservedControl::Track {
                grade_centi_pct: centi,
                crr_units: 80,
            },
        ));
    }
    run.change(ControlMode::Resistance { pct: 0.0 }, Duration::from_secs(1))
        .await?;
    run.link.control.send(TrainerCommand::Disconnect).await?;
    run.drain(Duration::from_secs(1)).await;

    let records = parse_capture(&std::fs::read_to_string(capture_path)?);
    Ok(latency_summary(&Calibration {
        observed: &run.observed,
        erg_changes: &erg_changes,
        sim_changes: &sim_changes,
        sim_start,
        records: &records,
        faults: &run.faults,
        caps: &caps,
        scan_secs: args.scan_secs,
    }))
}

/// Everything the latency summary is computed from.
struct Calibration<'a> {
    observed: &'a [(u64, ObservedControl)],
    erg_changes: &'a [(u64, ObservedControl)],
    sim_changes: &'a [(u64, ObservedControl)],
    sim_start: usize,
    records: &'a [crate::frames::Record],
    faults: &'a [String],
    caps: &'a str,
    scan_secs: u64,
}

fn latency_summary(c: &Calibration<'_>) -> Value {
    let Calibration {
        observed,
        erg_changes,
        sim_changes,
        sim_start,
        records,
        faults,
        caps,
        scan_secs,
    } = *c;
    let e2e = |changes: &[(u64, ObservedControl)], from: usize| {
        let mut lat = Vec::new();
        let mut missed = 0;
        for (issued, want) in changes {
            match observed[from..]
                .iter()
                .find(|(at, c)| at >= issued && c == want)
            {
                Some((at, _)) => lat.push(at - issued),
                None => missed += 1,
            }
        }
        (lat, missed)
    };
    let over = |lat: &[u64], limit_us: u64| lat.iter().filter(|l| **l > limit_us).count();
    let (erg_lat, erg_missed) = e2e(erg_changes, 0);
    let (sim_lat, sim_missed) = e2e(sim_changes, 0);
    let req = request_latencies(records, 71, 71);
    let sim_obs = &observed[sim_start..];
    let gaps = |keep: fn(&ObservedControl) -> bool| {
        let t: Vec<u64> = sim_obs
            .iter()
            .filter(|(_, c)| keep(c))
            .map(|(a, _)| *a)
            .collect();
        t.windows(2).map(|w| w[1] - w[0]).collect::<Vec<_>>()
    };
    let wind_gaps = gaps(|c| matches!(c, ObservedControl::Wind { .. }));
    let track_gaps = gaps(|c| matches!(c, ObservedControl::Track { .. }));
    json!({
        "check": "3-latency",
        "what_was_sent": "real FecTrainer connect (page 55 + request 54), SetMode ERG 100<->150 W x60 at 3 s + jitter, SIM grade 1<->2 % x30 at 4 s; driver polls page 71 at 1 s",
        "caps": caps,
        "end_to_end_erg_us": distribution(&erg_lat),
        "end_to_end_erg_samples_us": erg_lat,
        "end_to_end_erg_over_2s": over(&erg_lat, 2_000_000),
        "erg_changes": erg_changes.len(), "erg_changes_never_echoed": erg_missed,
        "end_to_end_sim_track_us": distribution(&sim_lat),
        "end_to_end_sim_track_samples_us": sim_lat,
        "end_to_end_sim_track_over_2s": over(&sim_lat, 2_000_000),
        "sim_changes": sim_changes.len(), "sim_changes_never_echoed": sim_missed,
        "request_to_reply_us": distribution(&req),
        "sim_echo_gap_us": { "wind_page50": distribution(&wind_gaps), "track_page51": distribution(&track_gaps) },
        "sim_echo_counts": { "wind": sim_obs.iter().filter(|(_, c)| matches!(c, ObservedControl::Wind{..})).count(),
                              "track": sim_obs.iter().filter(|(_, c)| matches!(c, ObservedControl::Track{..})).count() },
        "faults": faults,
        "accounting": analysis(records),
        "scan_secs": scan_secs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The last line `hil_fec` prints and its exit status: a completed run whose release was not
    /// confirmed ends on that, with the rider instruction, and a non-zero status.
    #[test]
    fn the_last_line_and_the_exit_status_follow_the_release() {
        let path = PathBuf::from("out/check11a.json");
        let unconfirmed = format!("not connected; {}", rider::UNCONFIRMED);
        for (outcome, last, status) in [
            (
                Outcome::Done(path.clone()),
                "wrote out/check11a.json".to_string(),
                0,
            ),
            (
                Outcome::Unconfirmed(path.clone(), unconfirmed.clone()),
                format!("DONE, but the release was NOT confirmed: {unconfirmed}"),
                5,
            ),
            (
                Outcome::Stopped(path, "Aborted(\"x\") (trainer released)".into()),
                "STOPPED: Aborted(\"x\") (trainer released); wrote out/check11a.json".into(),
                4,
            ),
            (Outcome::Blocked("y".into()), "BLOCKED: y".into(), 3),
        ] {
            let (lines, code) = outcome.lines();
            assert_eq!((lines.last(), code), (Some(&last), status));
        }
        assert_eq!(
            Outcome::of(
                PathBuf::from("p"),
                rider::Ending::Unconfirmed(unconfirmed.clone())
            )
            .lines()
            .1,
            5
        );
    }

    /// The two settled-invalid endpoints (#215) are still sent to the NEO: the codec now
    /// refuses them, so the cases patch the largest valid frame to the sentinel bytes.
    #[test]
    fn invalid_endpoint_cases_send_the_sentinel_bytes() {
        let sent = |label: &str| {
            let (_, page) = value_cases()
                .into_iter()
                .find(|(l, _)| *l == label)
                .expect("case");
            let mut f = build(&page, ACK, XOR).expect("largest valid value encodes");
            for &(index, value) in overrides(label) {
                f = with_page_byte(f, index, value, XOR);
            }
            assert_eq!(f.iter().fold(0, |x, v| x ^ v), 0, "checksum stays valid");
            f
        };
        let power = sent("erg_16383_75W_0xFFFF");
        assert_eq!((power[4], power[10], power[11]), (49, 0xff, 0xff));
        let crr = sent("track_crr_0xFF");
        assert_eq!((crr[4], crr[11]), (51, 0xff));
        assert!(build(&ControlPage::TargetPower { watts: 16_383.75 }, ACK, XOR).is_none());
        assert!(
            build(
                &ControlPage::Track {
                    grade_pct: 0.0,
                    crr: 0.01275
                },
                ACK,
                XOR
            )
            .is_none()
        );
    }

    use crate::{
        frames::{Record, hex},
        partb::{FINISHED, OUTCOME, SESSION_BOUND_S},
        rider::{Procedure, Report, Wire},
        sim::{Pattern, Plan, Sim},
    };
    use std::{fmt::Write as _, path::Path};

    /// When check 9 of these tests ended, Unix seconds.
    const T9: u64 = 1_700_100_000;
    const FIRST: &str = "stall below the release cadence: the release comes first";
    /// Another trainer than the default simulated one (`0x1234_5678`).
    const OTHER: u32 = 0xaaaa_0001;

    /// A 2 s power lag that lets go at 40.5 rpm: slower than the bound check 9 relies on.
    const CLIFF: Plan = Plan {
        confirms: true,
        pedals: true,
        stop_at_us: None,
        drop_at_us: None,
        hang_from_us: None,
        silent_from_us: None,
        panic_at_us: None,
        descent: Pattern::Path(&[(0, 90.0), (100_000_000, 43.0), (101_500_000, 37.0)]),
        erg_floor_rpm: 35.0,
        erg_curve: Some(&[(40.5, 0.896), (43.0, 1.0)]),
        warm_w: 0.0,
        power_tau_s: 2.0,
        cadence_unavailable_from_us: None,
        serial: Some(OTHER),
        cps: None,
        advert_every_us: 500_000,
        advertisers: crate::sim::Advertisers::Both,
        scan_ends_at_us: None,
        answers: &[],
        answer_at_us: 0,
        connect_answer_after_us: 0,
    };

    async fn driven(p: Procedure, plan: Plan) -> (Report, Vec<Record>) {
        let sim = Sim::new(plan);
        let report = rider::drive(&sim, &sim, p, std::future::pending()).await;
        (report, Wire::records(&sim))
    }

    /// The `.fecap` lines of a run, before `hil_fec` appends its outcome and finished lines.
    fn lines(records: &[Record]) -> String {
        records.iter().fold(String::new(), |mut t, r| {
            let _ = writeln!(
                t,
                "{} {} {}",
                r.at_us,
                if r.rx { "rx" } else { "tx" },
                hex(&r.bytes)
            );
            t
        })
    }

    /// `lines` with the outcome and a finished stamp, as a completed capture file reads.
    fn capture(records: &[Record], report: &Report, finished: u64) -> String {
        let mut t = lines(records);
        let _ = writeln!(t, "{OUTCOME}{}", rider::outcome(&report.procedure));
        let _ = writeln!(t, "{FINISHED}{finished}");
        t
    }

    /// Check 9 of `plan9` (finished at `T9`) with the check 6 that `place` puts beside it, through
    /// the live path ([`procedure_summary`]) and through `analyse`: the `result` of both, which
    /// must be one and the same.
    async fn check9_beside(plan9: Plan, place: impl Fn(&Path)) -> Value {
        let (report, records) = driven(Procedure::Check9, plan9).await;
        let (live, offline) = (
            tempfile::tempdir().expect("tmp"),
            tempfile::tempdir().expect("tmp"),
        );
        place(live.path());
        place(offline.path());
        let path = live.path().join("check9.fecap");
        std::fs::write(&path, lines(&records)).expect("write");
        let live =
            procedure_summary(Procedure::Check9, "check9", &report, &path, T9)["result"].clone();
        let target = offline.path().join("check9.fecap");
        std::fs::write(&target, capture(&records, &report, T9)).expect("write");
        let args = Args {
            check: Check::Analyse,
            out: offline.path().join("out"),
            scan_secs: 0,
            idle_secs: 0,
            target: Some(target.display().to_string()),
        };
        analyse(&args).expect("analyse");
        let back = std::fs::read_to_string(args.out.join("check9.analysis.json")).expect("read");
        let back: Value = serde_json::from_str(&back).expect("json");
        assert_eq!(live, back["part_b"], "live and offline differ");
        live
    }

    /// `check6.fecap` of the default simulated trainer (serial `serial`) finished at `finished`.
    async fn check6_text(serial: Option<u32>, finished: u64) -> String {
        let plan = Plan {
            serial,
            ..Plan::default()
        };
        let (report, records) = driven(Procedure::Check6, plan).await;
        capture(&records, &report, finished)
    }

    fn put(text: String) -> impl Fn(&Path) {
        move |dir| std::fs::write(dir.join("check6.fecap"), &text).expect("write")
    }

    /// The verdict was not "comes first" and the power lag is `NotMeasured` for a reason with `why`.
    fn assert_unproven(result: &Value, why: &str) {
        let lag = result["power_lag"]["within"]
            .as_str()
            .expect("a reason, not a bool");
        assert!(lag.contains(why), "{lag:?} lacks {why:?}");
        assert_ne!(result["verdict"], json!(FIRST), "{result}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_check_6_of_the_same_trainer_and_session_unlocks_comes_first() {
        let result = check9_beside(
            Plan::default(),
            put(check6_text(Some(0x1234_5678), T9 - 3600).await),
        )
        .await;
        assert_eq!(result["power_lag"]["within"], json!(true), "{result}");
        assert_eq!(result["verdict"], json!(FIRST), "{result}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_check_6_of_another_trainer_is_not_measured() {
        // The falsifier's repro: a 2 s cliff trainer (check 9) beside the check 6 of the
        // default 0.8 s simulation, which would read "comes first".
        let result =
            check9_beside(CLIFF, put(check6_text(Some(0x1234_5678), T9 - 600).await)).await;
        assert_unproven(&result, "different trainer");
    }

    #[tokio::test(start_paused = true)]
    async fn a_check_6_or_check_9_without_a_known_serial_is_not_measured() {
        // Check 6 never got a page-81 answer.
        let none = check6_text(None, T9 - 600).await;
        assert_unproven(
            &check9_beside(Plan::default(), put(none)).await,
            "check 6: no page-81 serial",
        );
        // Check 6 redacted (`FF FF FF FF`): the capture no longer says whose it is.
        let redacted =
            crate::frames::redact_capture(&check6_text(Some(0x1234_5678), T9 - 600).await);
        assert_unproven(
            &check9_beside(Plan::default(), put(redacted)).await,
            "unspecified",
        );
        // Check 9 never got one.
        let ok = check6_text(Some(0x1234_5678), T9 - 600).await;
        let plan = Plan {
            serial: None,
            ..Plan::default()
        };
        assert_unproven(
            &check9_beside(plan, put(ok)).await,
            "check 9: no page-81 serial",
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_check_6_outside_the_session_bound_is_not_measured() {
        let serial = Some(0x1234_5678);
        // Exactly the bound is the same session; one second more is not.
        let at_bound = check6_text(serial, T9 - SESSION_BOUND_S).await;
        let result = check9_beside(Plan::default(), put(at_bound)).await;
        assert_eq!(result["verdict"], json!(FIRST), "{result}");
        let old = check6_text(serial, T9 - SESSION_BOUND_S - 1).await;
        assert_unproven(
            &check9_beside(Plan::default(), put(old)).await,
            "session bound",
        );
        // Check 6 after check 9 is not the check 6 that came before it.
        let later = check6_text(serial, T9 + 1).await;
        assert_unproven(
            &check9_beside(Plan::default(), put(later)).await,
            "ended after check 9",
        );
        // No stamp: the session is unknown.
        let unstamped: String = check6_text(serial, T9)
            .await
            .lines()
            .filter(|l| !l.starts_with(FINISHED))
            .fold(String::new(), |mut t, l| {
                let _ = writeln!(t, "{l}");
                t
            });
        assert_unproven(
            &check9_beside(Plan::default(), put(unstamped)).await,
            "no finished stamp",
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_unreadable_check_6_is_not_measured() {
        // A read error other than NotFound (a directory in its place), not an absent file.
        let result = check9_beside(Plan::default(), |dir| {
            std::fs::create_dir(dir.join("check6.fecap")).expect("dir");
        })
        .await;
        assert_unproven(&result, "check6.fecap");
        let absent = check9_beside(Plan::default(), |_| {}).await;
        assert_unproven(&absent, "no check6.fecap");
    }

    use crate::sim::CpsPlan;

    /// `records` and `cps` written by the real [`crank_ble::Capture`] writer, as a live run does.
    fn write_capture(path: &Path, records: &[Record], cps: &[crate::frames::Cps]) {
        use crank_devices::MonoTime;
        enum Item<'a> {
            Frame(&'a Record),
            Cps(&'a crate::frames::Cps),
        }
        let mut items: Vec<(u64, Item)> = records
            .iter()
            .map(|r| (r.at_us, Item::Frame(r)))
            .chain(cps.iter().map(|c| (c.at_us, Item::Cps(c))))
            .collect();
        items.sort_by_key(|i| i.0);
        let capture = crank_ble::Capture::create(path, MonoTime(0)).expect("create");
        for (at, item) in items {
            match item {
                Item::Frame(r) => capture.record(MonoTime(at), r.rx, &r.bytes),
                Item::Cps(c) => capture.record_aux(MonoTime(at), "cps", &c.bytes),
            }
            .expect("record");
        }
    }

    /// Check 5 of `plan` through the live summary and then through `analyse` on the very file
    /// the live run leaves (written by the real capture writer): the report, and the `result` of
    /// both, which must agree field by field.
    async fn check5_of(plan: Plan) -> (Report, Value) {
        let sim = Sim::new(plan);
        let report = rider::drive(&sim, &sim, Procedure::Check5, std::future::pending()).await;
        let (records, cps) = (Wire::records(&sim), Wire::cps(&sim));
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("check5.fecap");
        write_capture(&path, &records, &cps);
        let live =
            procedure_summary(Procedure::Check5, "check5", &report, &path, T9)["result"].clone();
        let args = Args {
            check: Check::Analyse,
            out: dir.path().join("out"),
            scan_secs: 0,
            idle_secs: 0,
            target: Some(path.display().to_string()),
        };
        analyse(&args).expect("analyse");
        let back = std::fs::read_to_string(args.out.join("check5.analysis.json")).expect("read");
        let back: Value = serde_json::from_str(&back).expect("json");
        let offline = &back["part_b"];
        for (key, value) in live.as_object().expect("object") {
            assert_eq!(value, &offline[key], "live and offline differ in {key}");
        }
        assert_eq!(live, *offline, "live and offline differ");
        assert_eq!(live["cps"]["notifications"], cps.len(), "{live}");
        (report, live)
    }

    /// A completed default ride: its records and CPS notifications.
    async fn sim_ride() -> (Vec<Record>, Vec<crate::frames::Cps>) {
        let sim = Sim::new(Plan::default());
        let report = rider::drive(&sim, &sim, Procedure::Check5, std::future::pending()).await;
        assert_eq!(report.procedure, Ok(()));
        (Wire::records(&sim), Wire::cps(&sim))
    }

    fn verdict_of(records: &[Record], cps: &[crate::frames::Cps]) -> (String, Value) {
        let v = crate::cps::check5(records, cps);
        (v["verdict"].as_str().expect("verdict").to_string(), v)
    }

    /// Seconds from the capture start to where window `i` begins, µs.
    fn window_from_us(v: &Value, i: usize) -> u64 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let us = (v["windows"][i]["from_s"].as_f64().expect("window") * 1e6) as u64;
        us
    }

    #[tokio::test(start_paused = true)]
    async fn check5_no_cps_during_a_whole_target_is_insufficient_whatever_the_stamps() {
        let (records, cps) = sim_ride().await;
        let (verdict, v) = verdict_of(&records, &cps);
        assert_eq!(verdict, "agree", "{v}");
        // CPS stops at the start of the 200 W hold: that target is never compared.
        let cut = window_from_us(&v, 1) - 10_000_000;
        for shift_ms in [0, 100, 250, 500, 750, 999] {
            let late: Vec<_> = cps
                .iter()
                .filter(|c| c.at_us < cut)
                .map(|c| crate::frames::Cps {
                    at_us: c.at_us + shift_ms * 1000,
                    bytes: c.bytes.clone(),
                })
                .collect();
            let (verdict, v) = verdict_of(&records, &late);
            assert!(
                verdict.starts_with("insufficient: ") && verdict.contains("200 W target"),
                "shift {shift_ms} ms: {verdict}\n{v}"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn check5_the_verdict_does_not_depend_on_the_sub_second_alignment() {
        let (records, cps) = sim_ride().await;
        for shift_ms in (0..=1000).step_by(50) {
            let shifted: Vec<_> = cps
                .iter()
                .map(|c| crate::frames::Cps {
                    at_us: c.at_us + shift_ms * 1000,
                    bytes: c.bytes.clone(),
                })
                .collect();
            let (verdict, v) = verdict_of(&records, &shifted);
            assert_eq!(verdict, "agree", "shift {shift_ms} ms\n{v}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn check5_a_long_page_25_gap_inside_a_window_is_insufficient() {
        let (records, cps) = sim_ride().await;
        let (_, v) = verdict_of(&records, &cps);
        let from = window_from_us(&v, 0);
        // 15 s without page 25 inside the first window; the average rate stays above 2 Hz.
        let holed: Vec<Record> = records
            .iter()
            .filter(|r| {
                !(r.rx
                    && page_of(&r.bytes) == Some(25)
                    && r.at_us > from + 2_000_000
                    && r.at_us < from + 17_000_000)
            })
            .cloned()
            .collect();
        let (verdict, v) = verdict_of(&holed, &cps);
        assert!(
            verdict.starts_with("insufficient: ")
                && verdict.contains("150 W target")
                && verdict.contains("no page 25 for"),
            "{verdict}\n{v}"
        );
        assert!(v["page25"]["rate_hz"].as_f64().unwrap_or(0.0) >= 2.0, "{v}");
    }

    #[tokio::test(start_paused = true)]
    async fn check5_a_cps_gap_inside_a_window_and_a_single_target_are_insufficient() {
        let (records, cps) = sim_ride().await;
        let (_, v) = verdict_of(&records, &cps);
        let from = window_from_us(&v, 1);
        let holed: Vec<_> = cps
            .iter()
            .filter(|c| c.at_us < from + 3_000_000 || c.at_us > from + 8_000_000)
            .cloned()
            .collect();
        let (verdict, _) = verdict_of(&records, &holed);
        assert!(
            verdict.starts_with("insufficient: ") && verdict.contains("no 2A63 for"),
            "{verdict}"
        );
        // The 200 W write never happened: one target, so one window.
        let one: Vec<Record> = records
            .iter()
            .filter(|r| {
                r.rx || !matches!(
                    crank_fec::decode(&r.bytes).map(|f| f.page),
                    Ok(crank_fec::Page::TargetPower { watts }) if (watts - 200.0).abs() < 1e-3
                )
            })
            .cloned()
            .collect();
        let (verdict, _) = verdict_of(&one, &cps);
        assert!(
            verdict.starts_with("insufficient: 1 settled windows"),
            "{verdict}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn check5_half_the_cps_notifications_malformed_is_insufficient() {
        let (records, cps) = sim_ride().await;
        let cut: Vec<_> = cps
            .iter()
            .enumerate()
            .map(|(i, c)| crate::frames::Cps {
                at_us: c.at_us,
                bytes: if i % 2 == 0 {
                    c.bytes[..5].to_vec()
                } else {
                    c.bytes.clone()
                },
            })
            .collect();
        let (verdict, _) = verdict_of(&records, &cut);
        assert!(
            verdict.starts_with("insufficient: ") && verdict.contains("malformed"),
            "{verdict}"
        );
    }

    fn with_cps(cps: Option<CpsPlan>) -> Plan {
        Plan {
            cps,
            ..Plan::default()
        }
    }

    #[tokio::test(start_paused = true)]
    async fn check5_agreeing_streams_pass_and_analyse_reproduces_the_row() {
        let (report, v) = check5_of(Plan::default()).await;
        assert_eq!(report.procedure, Ok(()));
        assert_eq!(v["verdict"], "agree", "{v}");
        assert!(v["seconds_compared"].as_u64().unwrap_or(0) >= 20, "{v}");
        assert!(v["page25"]["rate_hz"].as_f64().unwrap_or(0.0) >= 2.0, "{v}");
        let line = v["line"].as_str().expect("row");
        assert!(
            line.starts_with("| 5 |") && line.ends_with("§5 |") && line.contains("agree"),
            "{line}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn check5_a_mismatch_beyond_the_tolerance_fails_and_the_boundary_passes() {
        let cps = |offset_w| {
            with_cps(Some(CpsPlan {
                offset_w,
                ..CpsPlan::default()
            }))
        };
        let (_, v) = check5_of(cps(2.0)).await;
        let verdict = v["verdict"].as_str().expect("verdict");
        assert!(
            verdict.starts_with("disagree: power:") && verdict.contains("outside ±1 W"),
            "{v}"
        );
        assert_eq!(v["bias_w"], -2.0);
        let (_, v) = check5_of(cps(1.0)).await;
        assert_eq!(
            v["verdict"], "agree",
            "exactly ±1 W is within the tolerance: {v}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn check5_derives_cadence_across_the_rollover_of_both_cps_counters() {
        let wrap = CpsPlan {
            revs0: 65_530,
            time0: 65_000,
            ..CpsPlan::default()
        };
        let (_, v) = check5_of(with_cps(Some(wrap))).await;
        assert_eq!(v["verdict"], "agree", "{v}");
        assert_eq!(v["cps"]["rejected_pairs"], 0, "{v}");
        for w in v["cadence_windows"].as_array().expect("windows") {
            assert!(
                (w["cps_rpm"].as_f64().unwrap_or(0.0) - 90.0).abs() <= 1.0,
                "{v}"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn check5_without_cps_ends_released_with_no_result() {
        let (sim, report) = {
            let sim = Sim::new(with_cps(None));
            let report = rider::drive(&sim, &sim, Procedure::Check5, std::future::pending()).await;
            (sim, report)
        };
        assert!(
            matches!(&report.procedure, Err(rider::Stop::Aborted(w)) if w.contains("2A63")),
            "{:?}",
            report.procedure
        );
        assert_eq!(report.release_error, None);
        let (_, v) = check5_of(with_cps(None)).await;
        assert_eq!(v["completed"], false);
        assert!(v["verdict"].is_null(), "{v}");
        assert!(
            v["line"]
                .as_str()
                .is_some_and(|l| l.contains("no result: procedure aborted")),
            "{v}"
        );
        assert_eq!(Wire::cps(&sim).len(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn check5_on_a_capture_without_cps_gives_no_verdict() {
        let sim = Sim::new(Plan::default());
        let report = rider::drive(&sim, &sim, Procedure::Check5, std::future::pending()).await;
        assert_eq!(report.procedure, Ok(()));
        let v = partb::analyse(
            "check5",
            &Wire::records(&sim),
            &[],
            &[],
            Some(partb::COMPLETED),
            &partb::PowerLag::NotMeasured(String::new()),
        )
        .expect("check5");
        assert!(
            v["verdict"]
                .as_str()
                .is_some_and(|x| x.starts_with("no CPS")),
            "{v}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn check5_without_crank_data_cannot_confirm_cadence() {
        let plan = with_cps(Some(CpsPlan {
            no_crank: true,
            ..CpsPlan::default()
        }));
        let (_, v) = check5_of(plan).await;
        assert!(
            v["verdict"]
                .as_str()
                .is_some_and(|x| x.contains("no 2A63 crank-revolution data")),
            "{v}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn check5_an_aborted_run_gives_no_result_even_with_agreeing_streams() {
        let plan = Plan {
            stop_at_us: Some(40_000_000),
            ..Plan::default()
        };
        let (report, v) = check5_of(plan).await;
        assert!(
            matches!(report.procedure, Err(rider::Stop::Aborted(_))),
            "{:?}",
            report.procedure
        );
        assert!(v["verdict"].is_null() && v["completed"] == false, "{v}");
        assert!(
            v["line"]
                .as_str()
                .is_some_and(|l| l.contains("no result: procedure aborted")),
            "{v}"
        );
    }

    #[test]
    fn redact_keeps_the_cps_lines() {
        let text = "0 rx A4 09 4E 05 51 FF FF 01 01 02 03 04 00\n# cps 5 20 00 96 00 01 00 02 00\n";
        let out = crate::frames::redact_capture(text);
        assert!(out.contains("# cps 5 20 00 96 00 01 00 02 00\n"), "{out}");
        assert_eq!(crate::frames::parse_cps(&out).len(), 1);
    }
}
