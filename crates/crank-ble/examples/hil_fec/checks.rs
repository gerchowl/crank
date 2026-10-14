//! The Part A checks (#44 checks 1–4, idle pattern, #204/#206 tentative items).
use crate::{
    cli::{Args, Check, write_summary},
    frames::{
        Checksum, build, command_status, distribution, hex, page_of, page_rates, parse_capture,
        redact_capture, redact_frame, redact_name, request_latencies, with_page_byte,
    },
    session::{Exchange, Result, Session, adapter, find_trainer, scan},
};
use crank_ble::{
    BtleplugAdvertisements, CPS_SERVICE, FEC_SERVICE, FTMS_SERVICE, FecTrainer, HRS_SERVICE,
};
use crank_devices::{
    ControlMode, DeviceRef, ObservationSource, ObservedControl, RiderConfig, TrainerCommand,
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
}

const BLOCKED_NO_NEO: &str = "NEO not advertising — owner: power the trainer (mains plugged in) and keep the phone/Tacx app disconnected";
const XOR: Checksum = Checksum::XorInclSync;
const ACK: u8 = 0x4f;
const BROADCAST: u8 = 0x4e;

/// Dispatch one subcommand.
pub async fn run(args: &Args) -> Result<Outcome> {
    match args.check {
        Check::Redact => return redact(args),
        Check::Scan => return scan_check(args).await,
        Check::Known => return known(args).await,
        _ => {}
    }
    let adapter = adapter().await?;
    let adv = BtleplugAdvertisements::new(adapter.clone());
    let Some((device, _)) = find_trainer(&adv, args.scan_secs).await else {
        return Ok(Outcome::Blocked(BLOCKED_NO_NEO.into()));
    };
    std::fs::create_dir_all(&args.out)?;
    let capture_path = args.out.join(format!("{}.fecap", args.check.name()));
    let summary = if args.check == Check::Latency {
        latency(args, &adv, adapter, device, &capture_path).await?
    } else {
        let s = Session::open(&adv, adapter, device, &capture_path).await?;
        let summary = match args.check {
            Check::Idle => idle(&s, args).await,
            Check::Check1 => check1(&s).await,
            Check::Check2 => check2(&s).await,
            Check::Check3 => check3(&s).await,
            Check::Check4 => check4(&s).await,
            Check::Spacing => spacing(&s).await,
            Check::Scan | Check::Known | Check::Latency | Check::Redact => unreachable!(),
        };
        let ended = s.pump_ended();
        s.close().await;
        let mut summary = summary;
        summary["link_dropped_during_run"] = json!(ended);
        summary
    };
    Ok(Outcome::Done(write_summary(
        &args.out, args.check, &summary,
    )?))
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

async fn control_then_status(s: &Session, label: &str, page: &ControlPage) -> Value {
    let Some(control) = build(page, ACK, XOR) else {
        return json!({"label": label, "encode_error": true});
    };
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
    let mut requests = Vec::new();
    for gap_ms in [250_u64, 500, 1000, 2000] {
        let mark = s.mark();
        for _ in 0..20 {
            let _ = s.send(request(71, 1, ACK, XOR), true).await;
            sleep(Duration::from_millis(gap_ms)).await;
        }
        sleep(Duration::from_millis(1500)).await;
        let n = s
            .since(mark)
            .iter()
            .filter(|r| page_of(&r.bytes) == Some(71))
            .count();
        requests.push(json!({"gap_ms": gap_ms, "requests": 20, "page71_replies": n}));
    }
    let mut after_control = Vec::new();
    for delay_ms in [100_u64, 250, 500, 1000] {
        let mut answered = 0;
        for i in 0..10 {
            let w = if i % 2 == 0 { 100.0 } else { 110.0 };
            let control = build(&ControlPage::TargetPower { watts: w }, ACK, XOR).expect("encodes");
            let _ = s.send(control, true).await;
            sleep(Duration::from_millis(delay_ms)).await;
            let x = s
                .exchange(
                    request(71, 1, ACK, XOR),
                    true,
                    71,
                    Duration::from_millis(1500),
                )
                .await;
            answered += usize::from(x.reply.is_some());
            sleep(Duration::from_millis(1500)).await;
        }
        after_control.push(json!({"control_to_request_ms": delay_ms, "trials": 10, "answered_within_1500ms": answered}));
    }
    park(s).await;
    json!({
        "check": "3-spacing",
        "what_was_sent": "20 page-70 requests for page 71 at 250/500/1000/2000 ms; page 49 then request(71) after 100/250/500/1000 ms x10",
        "identity": identity(s),
        "bare_request_spacing": requests,
        "control_to_request_delay": after_control,
    })
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
            ControlPage::Track {
                grade_pct: 0.0,
                crr: 0.01275,
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
            ControlPage::TargetPower { watts: 16_383.75 },
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
        steps.push(control_then_status(s, label, page).await);
        sleep(Duration::from_millis(300)).await;
    }
    // D: does the sequence byte advance per control page / per request?
    let mut per_control = Vec::new();
    for i in 0..8 {
        let w = if i % 2 == 0 { 100.0 } else { 110.0 };
        let t =
            control_then_status(s, "seq_per_control", &ControlPage::TargetPower { watts: w }).await;
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
            .send(TrainerCommand::SetMode(mode))
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
    // SIM: alternate grade 1 % / 2 % so each change moves both components' desired state.
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
        "scan_secs": scan_secs,
    })
}
