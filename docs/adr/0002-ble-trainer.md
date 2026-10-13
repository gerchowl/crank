# ADR-0002: Bluetooth stack and Trainer abstraction (FE-C first)

- **Status:** Proposed
- **Date:** 2026-10-13
- **Issue:** #27 (decides the design questions of #23)
- **Related:** ADR-0001 (crate layout, clock, testing), `spec/TRAINER-BLE.md`

## 1. Context

The owner's trainer is a Tacx NEO gen 1 (`Tacx Neo 55624`, firmware 0.7.4, hardware rev 0). The probe of
2026-10-13 (`spec/TRAINER-BLE.md`) found **no FTMS (0x1826)** on either firmware. The NEO exposes:

| Service | Characteristics | Role in crank |
|---|---|---|
| `6e40fec1-b5a3-f393-e0a9-e50e24dcca9e` Tacx FE-C over BLE | `…fec2` notify (trainer → app), `…fec3` write / write-without-response (app → trainer) | control + telemetry |
| `0x1818` Cycling Power | 2A63 Measurement, 2A64 Vector, 2A65 Feature, 2A5D Location, 2A66 Control Point | not used during FE-C rides (§5) |
| `0x1816` Cycling Speed & Cadence | 2A5B, 2A5C, 2A5D | not used |
| `0x180A` Device Information | manufacturer, model, serial, hw/fw revision | pairing memory, diagnostics |
| `669aa501-0c08-969e-e211-86ad5062675f` | `669aac01…` | unknown Tacx service, never touched |

Constraints: only one BLE central can be connected to the NEO at a time; the core must ride headless
(CLI, tests) as well as in the app (ADR-0001 §2); macOS gates Bluetooth per signed identity (TCC); agents
on the shared host sage are denied Bluetooth inside flock panes, so hardware runs go through an Alacritty
window and need the owner for anything involving pedalling.

## 2. Decision: BLE library and where the stack runs

**btleplug** (0.13.5, released 2026-10-10 on crates.io) in the Rust core, crate `crank-ble`. The UI
never talks to Bluetooth; it sees device state through the `StreamFrame` channel and device commands
(ADR-0001 §4). The CLI uses the same crate, so `crank ride` works with a real trainer.

Facts used for the decision (btleplug README, https://github.com/deviceplug/btleplug):

- Central role only (enough: crank is always the central). Device enumeration and service/characteristic
  discovery work on Linux, Windows, macOS 10.15+, iOS and Android; read, write, notify are implemented on
  all of them.
- macOS apps need `NSBluetoothAlwaysUsageDescription`; command-line tools need the terminal to hold the
  Bluetooth grant.
- On Linux, BlueZ merges discovery filters across D-Bus clients, so scan results must be post-filtered;
  crank always post-filters by service UUID (§5).
- `adapter_address()` is `None` on CoreBluetooth; peripheral ids on macOS are per-host CoreBluetooth UUIDs,
  not MAC addresses (affects pairing memory, §5).

Rejected:

| Alternative | Why rejected |
|---|---|
| `tauri-plugin-blec` 0.17 (btleplug underneath on desktop/iOS, own Android backend; JS API plus a host-agnostic `blec` crate) | A wrapper over btleplug whose main value is a frontend JS API we don't want (BLE in the UI would leave CLI/MCP unable to ride). Its Android backend is the reason to revisit if Android ever enters scope. |
| `bluest` 0.6.9 | Last release 2025-06; no Android; smaller user base (75k downloads vs 1.7M). No capability we need that btleplug lacks. |
| CoreBluetooth directly via `objc2` | macOS-only and duplicates what btleplug wraps; Windows/Linux (#41) would need a second stack. |
| Web Bluetooth in the webview | WebKit (WKWebView, which Tauri uses on macOS) does not implement Web Bluetooth (MDN compatibility table, https://developer.mozilla.org/en-US/docs/Web/API/Web_Bluetooth_API). |

## 3. Decision: the device abstraction

The engine (`crank-engine`) talks to devices only through traits in `crank-devices`. Drivers are
connection factories; a connected device is a pair of channels owned by the driver's task. This keeps
the traits object-safe without `async_trait`, lets the engine actor `select!` over device events and
commands, and makes the simulator and replay transport drop-in replacements.

```rust
// crates/crank-devices/src/lib.rs (sketch)

pub enum DriverKind { FeC, Ftms, Sim }

pub trait TrainerDriver: Send + Sync + 'static {
    fn kind(&self) -> DriverKind;
    /// Connect, discover services, subscribe, apply rider config. Returns once telemetry flows.
    fn connect(&self, dev: &DeviceRef, rider: RiderConfig)
        -> BoxFuture<'static, Result<TrainerLink, DeviceError>>;
}

pub struct TrainerLink {
    pub info: DeviceInfo,                       // name, model, serial, fw (DIS / pages 80-81)
    pub caps: TrainerCaps,
    pub control: mpsc::Sender<TrainerCommand>,
    pub events: mpsc::Receiver<TrainerEvent>,
}

pub struct TrainerCaps {
    pub erg: bool, pub sim: bool, pub resistance: bool,
    pub max_resistance_n: Option<u16>,          // FE-C page 54
    pub max_target_w: u16,                      // clamp for ERG targets
}

pub enum ControlMode {
    Erg { target_w: u16 },
    Sim { grade_pct: f32, crr: f32, wind: Wind },
    Resistance { pct: f32 },                    // 0..=100
    Free,                                       // explicit flat-road SIM: grade 0, default Crr and wind (§7.9)
}
pub struct Wind { pub cw_kg_per_m: f32, pub speed_kmh: f32, pub drafting: f32 }
pub struct RiderConfig { pub rider_kg: f32, pub bike_kg: f32, pub wheel_m: f32 }

pub enum TrainerCommand { SetMode(ControlMode), SetRider(RiderConfig), Disconnect }

pub enum TrainerEvent {
    Sample(TrainerSample),
    Delivered { mode: ControlMode },            // GATT write-with-response succeeded: bytes reached the peripheral, nothing more
    Observed(TrainerObservation),               // what the trainer reports it is doing (§4.4); never "ack of command N"
    Link(LinkState),                            // Connecting | Ready | Lost (Ready = transport, rider config, caps done)
    Fault(DeviceError),
}

pub struct TrainerSample {
    pub at: MonoTime,
    pub power_w: Option<u16>,
    pub cadence_rpm: Option<u8>,
    pub speed_mps: Option<f32>,
    pub distance_m: Option<f32>,
    pub accumulated_power_w: Option<u32>,       // drop-robust averaging (§5)
    pub event_count: Option<u8>,
    pub status: TrainerStatus,                  // calibration / user-config required
    pub target_limit: TargetLimit,              // AtTarget | SpeedTooLow | SpeedTooHigh | Undetermined
}

pub struct TrainerObservation {
    pub at: MonoTime,
    pub source: ObservationSource,              // Page71Echo | Page25Flags
    pub mode: Option<ControlMode>,              // decoded from the page-71 echo of the last received control page
    pub status: Option<CommandStatus>,          // page-71 status byte as reported
    pub target_limit: Option<TargetLimit>,      // page 25 flags
}

pub trait HeartRateDriver: Send + Sync + 'static {
    fn connect(&self, dev: &DeviceRef) -> BoxFuture<'static, Result<HeartRateLink, DeviceError>>;
}
pub struct HeartRateLink { pub info: DeviceInfo, pub events: mpsc::Receiver<HrEvent> }
pub enum HrEvent { Sample(HrSample), Link(LinkState), Fault(DeviceError) }
pub struct HrSample { pub at: MonoTime, pub bpm: Option<u8>, pub rr_ms: SmallVec<[u16; 4]>, pub contact: Option<bool> }

pub trait Discovery: Send + Sync + 'static {
    fn scan(&self) -> BoxStream<'static, DiscoveredDevice>;   // post-filtered, classified
}

#[derive(thiserror::Error, Debug)]
pub enum DeviceError {
    AdapterUnavailable, PermissionDenied, NotFound,
    ConnectFailed(String), Gatt(String),
    Protocol { kind: ProtocolErrorKind, detail: String },  // transport-neutral; crank-ble maps crank_fec::FrameError
    Disconnected,
}
pub enum ProtocolErrorKind { Framing, Checksum, UnexpectedPage, Encode }
```

Implementations:

| Driver | Crate | Milestone |
|---|---|---|
| `FecTrainer` (FE-C over BLE) | `crank-ble` | M1 (#43) |
| `HrsHeartRate` (Heart Rate Service 0x180D) | `crank-ble` | M1 (#43) |
| `SimTrainer`, `SimHeart` (port of `shared/src/sim.ts`, seeded) | `crank-devices::sim` | M1 (#31) |
| `ReplayTrainer` (feeds a `.fecap` capture through the real FE-C driver over a fake transport) | `crank-ble` (test-only feature) | M1 (#42) |
| `FtmsTrainer` | `crank-ble` | M3 (#45) |

Mode semantics are the engine's, not the driver's: the engine decides *what* the trainer should do
(target watts, grade); a driver only encodes it. ERG policy (§7) therefore works the same for FE-C,
FTMS and the simulator.

## 4. Decision: FE-C over BLE codec (`crank-fec`)

### 4.1 Sources

- **Tacx, "FE-C over BLE — Simple how-to for engineers", v1.0.0** (D. de Nijs, effective 20-8-2015; title
  page dated 17/02/2015). Tacx does not publish it; a community copy is in
  https://github.com/abellono/tacx-ios-bluetooth-example (`How-to FE-C over BLE v1_0_0.pdf`), read for this
  ADR. Garmin lists Tacx trainer documentation as "available by request"
  (https://developer.garmin.com/tacx-trainer/); asking for a current revision is a pre-step of #44.
- **ANT+ Device Profile — Fitness Equipment** (thisisant.com) defines the page contents. It needs an ANT+
  adopter login and was **not retrieved**. Page layouts below are from the profile as implemented by
  pycycling (`pycycling/tacx_trainer_control.py`, https://github.com/zacharyedwardbull/pycycling, tested by
  its authors against a NEO) for pages 16, 25, 48–51, 55, 70, 71; pages 54, 80, 81 are not cross-checked
  and are logged raw during HIL (#44).

What the Tacx document states:

1. The transport is the Nordic UART Service with Tacx UUIDs: `6E40FEC1-…` service, `…FEC2` notifies
   trainer → app, the other characteristic receives app → trainer. (The document labels both characteristics
   `FEC2`, an evident typo; the probe and pycycling both write to `…fec3`.)
2. Writes carry 1–20 bytes; an FE-C message fits in one write, no fragmentation.
3. **Recommended rate is 4 Hz in both directions**, "the native rate over ANT".
4. Each write/notification is one complete ANT message: `A4 | len | type | channel | 8-byte page | checksum`.
   `len` = data length + 1 (= 9). Channel is 5 from the trainer and is ignored on writes.
5. Checksum: the prose says "the sum of all bytes excluding the 0xA4 sync byte", but the document's own
   worked example `A4 09 4E 05 10 19 04 00 00 00 FF 24 → 30` matches **XOR over all bytes including
   sync** (`0x30`), not the sum excluding sync (`0xAC`) nor XOR excluding sync (`0x94`) (computed for this
   ADR). XOR including sync is also the standard ANT message checksum. pycycling sends the arithmetic sum,
   so the NEO may not check the checksum on writes at all.

**Decision:** encode with XOR over all bytes including sync; decode verifies it and rejects mismatching
frames as `FrameError::Checksum` (counted in a metric and logged, never passed on). HIL check 1 (#44)
confirms the NEO accepts XOR-checksummed writes. Writes use message type **`0x4F` (acknowledged data)**,
as pycycling does against a NEO; HIL check 2 confirms.

### 4.2 Pages

From trainer (decoded):

| Page | Name | Fields (byte: meaning, unit) |
|---|---|---|
| 16 `0x10` | General FE data | 1: equipment type (25 = trainer); 2: elapsed time, 0.25 s, rolls over at 64 s; 3: distance, m, rolls over at 256; 4–5: speed, 0.001 m/s; 6: HR (0xFF = none); 7 hi nibble: FE state (ready / in use / finished) |
| 25 `0x19` | Specific trainer data | 1: update event count; 2: cadence, rpm (0xFF = none); 3–4: accumulated power, 1 W, rolls over at 65 536; 5 + 6 lo nibble: instantaneous power, 12 bit, 1 W (0xFFF = none); 6 hi nibble: trainer status (power calibration / resistance calibration / user configuration required); 7 lo bits: target power limits (0 at target, 1 speed too low, 2 speed too high, 3 undetermined); 7 hi nibble: FE state |
| 54 `0x36` | FE capabilities (on request) | 5–6: maximum resistance, N; 7: supported modes (basic resistance, target power, simulation) |
| 71 `0x47` | Command status (on request) | 1: last command page; 2: sequence; 3: status (0 pass, 1 fail, 2 not supported, 3 rejected, 4 pending); 4–7: echo of command data |
| 80 `0x50`, 81 `0x51` | Manufacturer / product info | hardware revision, manufacturer id, model; software revision, serial |

To trainer (encoded):

| Page | Name | Fields |
|---|---|---|
| 48 `0x30` | Basic resistance | 7: total resistance, 0.5 % units (0–200) |
| 49 `0x31` | Target power (ERG) | 6–7: target, **0.25 W** units, little-endian |
| 50 `0x32` | Wind resistance | 5: wind resistance coefficient, 0.01 kg/m; 6: wind speed, km/h + 127; 7: drafting factor, 0.01 |
| 51 `0x33` | Track resistance (SIM) | 5–6: grade, **0.01 %, offset −200.00 %** (`0x4E20` = 0 %); 7: rolling resistance coefficient, 5×10⁻⁵ |
| 55 `0x37` | User configuration | 1–2: rider mass, 0.01 kg; 4–5: bike mass, 0.05 kg (12 bit) + wheel diameter offset; 6: wheel diameter, 0.01 m; 7: gear ratio, 0.03 |
| 70 `0x46` | Request data page | 1–2: `0xFFFF`; 3–4: `0xFF`; 5: transmission count, bit 7 = reply acknowledged; 6: requested page; 7: `0x01` (data page) |

Unused bytes are `0xFF`. Unknown pages decode to `Page::Unknown(u8, [u8; 8])` and are logged at debug.

### 4.3 Codec API

```rust
// crates/crank-fec/src/lib.rs (sketch) — no I/O, no async, no allocation
pub enum MsgType { Broadcast = 0x4E, Acknowledged = 0x4F }
pub struct AntFrame { pub msg_type: MsgType, pub channel: u8, pub page: Page }

pub enum Page {
    GeneralFe(GeneralFe), Trainer(TrainerData), Capabilities(FeCapabilities),
    CommandStatus(CommandStatus), Manufacturer(ManufacturerInfo), Product(ProductInfo),
    Unknown(u8, [u8; 8]),
}
pub enum ControlPage {
    BasicResistance { pct: f32 }, TargetPower { watts: f32 },
    Wind { cw_kg_per_m: f32, speed_kmh: f32, drafting: f32 },
    Track { grade_pct: f32, crr: f32 },
    UserConfig { rider_kg: f32, bike_kg: f32, wheel_m: f32 },
    Request { page: u8, times: u8 },
}
pub fn encode(p: &ControlPage) -> Result<[u8; 13], EncodeError>;   // range-checked, never clamps silently
pub fn decode(b: &[u8]) -> Result<AntFrame, FrameError>;           // Sync | Length | Checksum | Truncated
```

Values that do not fit a page field (target above 16 383.75 W, grade outside ±200 %, Crr above 0.01275,
negative masses) are `EncodeError`s; clamping to trainer capabilities is the engine's explicit job (§7).

### 4.4 Command pacing and acknowledgement

- **One writer task per connection**, ticking at **4 Hz** (Tacx's rate). It holds coalescing slots,
  not a queue: `control` (latest `ControlMode`), `rider` (page 55), `requests` (FIFO of page-70 requests).
  Per tick it writes at most one message, priority `control` > `rider` > `requests`. A newer target
  replaces an unsent older one, so the trainer always gets the latest target and nothing piles up.
- `ControlMode::Sim` needs two pages (50 then 51); they go out on consecutive ticks.
- **Keep-alive:** the current control page is re-sent every 2 s even if unchanged. Writes are idempotent;
  HIL check 7 measures whether the NEO needs it.
- **Transport acknowledgement:** writes use GATT write-with-response; a GATT error becomes
  `TrainerEvent::Fault(DeviceError::Gatt)`. One write in flight at a time.
- **No command-specific confirmation.** FE-C over BLE gives no verifiable way to tie a reply to one
  command: page 71 reports the last received control page (byte 1), a sequence byte (byte 2) whose
  semantics are defined in the unretrieved ANT+ profile, a status (byte 3) and an echo of that command's
  data (bytes 4–7) (layout as implemented by pycycling), and there is no verified bound on how late a reply
  may arrive. Tags in the data bytes wrap and cannot identify a delayed reply either. crank therefore keeps
  two separate, honest signals:
  - **Delivery (transport):** every write uses GATT write-with-response. Success →
    `TrainerEvent::Delivered { mode }`, meaning the bytes reached the peripheral and nothing more; a GATT
    error → `Fault(DeviceError::Gatt)`.
  - **Observation (trainer state):** while a controlled mode (ERG, SIM, resistance, `Free`) is active the
    writer requests page 71 once per second through the `requests` slot (and once right after a mode
    change). Each reply becomes `TrainerEvent::Observed` with the echoed control page decoded into the mode
    it describes (`Erg { target_w }`, grade, resistance %), plus the status byte as reported. Page 25's
    target-limit flags are observations too. An observation says "the trainer reported state X at time t",
    never "command N was applied".
- **How the engine uses observations** (`crank-engine::erg`): it compares the latest observed mode with the
  desired mode. Equal → `trainer_following = yes`. Different, or status *fail / not supported / rejected*,
  for longer than 2 s (8 ticks) after the last desired-mode change → `trainer_following = no`, shown in the
  ride status and excluded from compliance (§7.8). No page-71 reply at all for 5 s → `unknown` (the UI shows
  no claim either way). Because a mismatch must persist across repeated 1 Hz observations, one delayed or
  stale reply can't flip the state; the worst effect of a stale reply is one second of a wrong observation.
  The engine never retries a write because of an observation; the 2 s keep-alive is unchanged.
- Pages 48, 50 and 55 are observed through the page-71 echo when they were the last control page, and
  otherwise through page 25 behaviour and page 54.
- **Hardware gate (#44, check 3):** does the NEO answer page 70 → page 71, and does it echo bytes 4–7 of
  pages 49 and 51? If not, `Observed` comes only from page 25 flags, `trainer_following` is `unknown` except
  for target-limit flags, and the ADR is amended before #43 implements the step. **Enhancement, gated on
  #44:** if the check shows that the sequence byte increments verifiably per received control page,
  command-specific correlation may be added later as a new ADR amendment; v1 does not depend on it.
- **Connect sequence:** connect → discover → subscribe `fec2` → read DIS → page 55 (rider config) →
  request page 54 (capabilities) → `TrainerLink` returned with `Link(Ready)` → engine sets the mode. The
  driver never chooses a control mode itself; restoring control is always the engine's `SetMode`.

## 5. Decision: sensor sources, discovery, pairing memory

- **One telemetry source per ride, never mixed.** For an FE-C-controlled trainer the source is FE-C
  **page 25** (power, cadence, accumulated power, trainer status, target-limit flags) plus **page 16**
  (speed, distance). Reasons: it is the link we must hold anyway for control; it carries the target-limit
  flags the ERG policy needs (§7); accumulated power + event count survive dropped notifications (below).
  **Gap filling:** between two received page-25 frames, `Δaccumulated / Δevents` (both modulo their
  rollover: 65 536 W and 256 events) is the exact *average* power of the missed events, not their
  individual values, so kJ stay exact but per-second means inside the gap are estimates. The engine fills
  only gaps shorter than the 2.0 s loss threshold (§6) with that average and marks those seconds
  `estimated` in the record (ADR-0001 §6); seconds wholly covered by received frames are exact. Longer gaps
  are dropouts: `null`, no filling. After a reconnect or an event-count jump that is not plausible for the
  elapsed time (more than 4 events per 250 ms), the first frame starts a new baseline; no delta is taken
  across it. Cycling Power 2A63 and CSC 2A5B
  are **not subscribed** during FE-C rides. The source is recorded in `ride.telemetry_source`.
  Acceptance in HIL check 5: page 25 arrives at ≥ 2 Hz and its per-second mean power agrees with 2A63
  within ±1 W; otherwise the ADR is amended to use 2A63 for telemetry.
- **Heart rate:** Heart Rate Service `0x180D`, characteristic 2A37 (bpm, sensor contact, RR intervals in
  1/1024 s converted to ms). The HR byte in FE-C page 16 is ignored (it relays ANT HR, not our strap).
- **Discovery:** btleplug scan with a service filter `[6e40fec1…, 0x1826, 0x1818, 0x180D]`, then
  post-filtered and classified on every platform: FE-C service → `FeC` trainer; FTMS without FE-C →
  `Ftms` (M3, #45; shown as "not yet supported" in v1); HRS → HR strap. Cycling-Power-only devices are
  ignored in v1.
- **Pairing memory** (`device` table, ADR-0001 §6): platform peripheral id (CoreBluetooth UUID on macOS,
  stable per host), advertised name, and DIS serial (2A25) + model. Reconnect tries the stored platform id
  first (CoreBluetooth can connect a known peripheral without scanning), then a scan matched by name +
  serial. No BLE bonding: the NEO and standard HR straps work without it (to confirm in #44).
- **Single central:** while crank is connected, no other app can control the NEO, and while another app
  (Zwift, Tacx app, a second crank process) is connected, crank cannot. crank therefore connects when the
  rider enters pre-ride or ride, stays connected while the app is open, and offers "Release trainer" in
  the devices view. If a remembered trainer is not seen during a scan, the UI says so with the likely
  causes (another app connected, trainer asleep or unpowered) while the scan continues. Within crank,
  the ride lock (ADR-0001 §2) prevents two crank processes from competing.

## 6. Decision: reconnection and dropout in a running workout

- **Detection:** a btleplug `DeviceDisconnected` event, or no trainer sample for 8 ticks (2.0 s at 4 Hz),
  marks the trainer `Lost`. The engine enters `paused(trainer_lost)` (ADR-0001 §5): from the detecting tick
  workout time and the block timer freeze while session time continues; the record gets `null` (no
  interpolation). The ≤ 2 s detection window before that stays workout time with `null` record values
  (ADR-0001 §5).
- **Reconnect loop** (driver task, active while a ride is loaded or the ride view is open): connect to the
  known peripheral, backoff 0.5 s, 1 s, 2 s, 4 s, then every 5 s, until the rider ends the ride or releases
  the trainer. Each attempt is logged; the UI shows "reconnecting (attempt n)".
- **On reconnect**, in this order: (1) the driver re-subscribes `fec2`, re-sends page 55 and re-checks page
  54, then emits `Link(Ready)`; (2) on `Ready` the engine issues `SetMode` with the current mode (in ERG via
  the soft start of §7, because the trainer may have been in its standalone mode while disconnected,
  measured in HIL check 11); (3) the ride resumes when cadence > 0 (ADR-0001 §5). The driver never re-sends
  a control mode on its own, so there is exactly one owner of control state.
- **HR strap dropout:** same reconnect loop; the ride does not pause; HR metrics are `null`; HR-dependent
  blocks and judge features behave as in ADR-0001 §5.
- **Packet loss without disconnect:** handled by accumulated power (§5); single missing pages are not a
  dropout.

## 7. Decision: ERG behaviour (from first principles)

Physics: P = τ·ω. In ERG the trainer holds power by setting brake torque τ = P/ω. If cadence falls, torque
rises; higher torque makes the pedals harder to turn, so cadence falls further, until the rider stalls
(the "spiral"). In the other direction a cadence surge lowers torque, so ERG feels "soft" right after a
step-down. Changing the target in a step forces an instantaneous torque change at constant cadence. The
policy below lives in `crank-engine::erg`, so it applies to every driver and is tested with `SimTrainer`.

1. **Controlled target** = block target × (1 + manual offset + coach change), rounded to 1 W, clamped to
   `[0, caps.max_target_w]` with the clamp shown as a validation warning in the editor (compile time) and a
   status flag in the ride. The controlled target is never smoothed and never derived from measured power.
2. **Steps:** the new target is sent on the block-boundary tick (coalescing in the `control` slot,
   §4.4, means the newest mode always wins). Smoothing a step is the trainer's job; its
   response time (t90 for 100→150→250→100 W) is measured in HIL check 6. A "lead" (send the next target L
   seconds early) defaults to 0 and is set to the measured trainer latency only by an ADR amendment
   backed by the HIL numbers.
3. **Ramp targets** (`ramp a-b%`): updated every 1 s (4 ticks) along the line, 1 W resolution; never more
   than one control write per tick (§4.4).
4. **Low-cadence release (spiral guard):** in ERG, cadence < 40 rpm for 2 s with target > 0 → switch to
   `Resistance { pct: 0 }` and show "ERG released, spin up". When cadence ≥ 60 rpm for 2 s → re-engage ERG
   with a soft start: the target ramps from the current 3 s power to the block target over 5 s (1 Hz
   updates). The block timer keeps running (the rider is still riding). Released seconds are excluded
   from compliance. Thresholds are initial values checked in HIL check 9.
5. **Soft start** is also used when ERG is (re-)engaged after a reconnect or after leaving `free`/`sim`.
6. **Trainer target-limit flags** (page 25: speed too low / too high) are shown in the ride status;
   they do not change the target by themselves.
7. **HR-target steps** (`140bpm` in the DSL): the engine steers the ERG target with a bounded integral
   controller: an update every 5 s, at most ±5 W per update, bounded to 40–90 % FTP, frozen while HR is
   `null`.
8. **Shown vs controlled power:** the UI shows filtered power per widget (default `power_3s`); `delta_target`
   uses `power_3s` against the **controlled** target; the ride record stores unfiltered per-second means of
   the source; coach features use their own windows from the knowledge pack. No display filter ever
   feeds control.
   **Compliance** = share of ERG seconds with `power_3s` within ±5 % of the controlled target, excluding
   *settle* seconds. Settle is triggered only by **discontinuities**, never by continuous target updates:
   (a) the first 5 s of each block whose start target differs from the previous block's end target (the
   mock's per-block settle); (b) a manual ± or coach change: 5 s; (c) an ERG (re-)engagement: the soft
   start's 5 s plus 5 s. Within `ramp` blocks and HR-steered steps the target moves continuously and is
   compared second by second with no settle; released (§7.4), paused and `trainer_following = no` seconds
   (§4.4) are excluded; compliance consumes observations, never command acknowledgements. Compliance
   is computed only in the engine (#13).
9. **SIM, resistance and free:** `Sim` sends page 50 (wind; defaults: 0.51 kg/m, 0 km/h, drafting 1.0) and
   page 51 (grade from the free-ride/sim control, Crr default 0.004) at ≤ 1 Hz on change; page 55 carries
   rider mass from the athlete profile and bike mass and wheel size from equipment settings. `Free` is
   **sent explicitly** as flat-road SIM (page 50 defaults, page 51 grade 0 %), never as "send nothing": an
   FE-C trainer keeps its last control page, so silence after ERG could leave the ERG target active.
   HIL check 12 (#44) confirms that ERG → `Free` releases the target on the NEO.

## 8. Decision: macOS TCC and the dev-time BLE path

- **App:** a real bundle from day 1: `crank.app` (bundle id `io.github.gerchowl.crank`) with
  `NSBluetoothAlwaysUsageDescription` = "crank connects to your smart trainer and heart-rate strap over
  Bluetooth.", assembled by Nix and signed by `just bundle` on sage with the self-signed `nix-tcc`
  identity (ADR-0001 §9.3, #37). TCC keys the grant on the designated requirement (bundle id + signing
  certificate), so the owner grants Bluetooth once and it survives rebuilds; ad-hoc-signed builds lose
  it on every rebuild (`spec/TRAINER-BLE.md`). Developer ID signing is for distribution only (#47).
- **CLI / HIL scripts:** the grant belongs to the terminal app that launched the process (TCC responsible
  process). On sage, flock panes are denied (responsible process `sshd-keygen-wrapper`), so hardware runs
  go through an Alacritty window: a wrapper script launched with
  `alacritty msg --socket <sock> create-window -e /abs/path/wrapper.sh` that writes its output to a file.
  Nobody modifies TCC settings.
- **Denied or off:** btleplug adapter errors map to `DeviceError::PermissionDenied` / `AdapterUnavailable`
  and reach the UI as `ApiError` with instructions. How btleplug reports CoreBluetooth's *unauthorized*
  state is checked by #43 with the grant revoked (owner step in #44).
- **No BLE in CI:** CI uses `SimTrainer` and `.fecap` replays (§9).

## 9. Decision: hardware-in-the-loop test plan

- **Scripts** in `hil/`: Rust examples over `crank-ble` (`cargo run -p crank-ble --example hil_fec -- <check>`),
  each writing a `.fecap` capture (timestamped raw frames, both directions) and a JSON summary.
- **Captures become fixtures:** each capture is committed under `crates/crank-ble/tests/fixtures/` and
  replayed in CI through `ReplayTrainer`, so every hardware fact stays tested.
- **Session 1 (#44), batched:**
  - Part A, no pedalling (an agent can run it via the Alacritty path while the trainer is powered): checks
    1–4 (checksum, message type, page 71, pages 54/80/81) and the idle notification pattern.
  - Part B, owner, about 20 min of pedalling in one sitting, script-guided with on-screen prompts: checks
    5–9, 11 and 12 (page 25 rate and agreement with 2A63; ERG step response at 90 rpm; cadence steps at
    200 W; keep-alive need; SIM grades; low-cadence stall point; dropout and reconnect; ERG → `Free`
    releases the target).
  - Check 3 also records whether page 71 echoes the command's data bytes and advances its sequence byte,
    which the acknowledgement correlation of §4.4 relies on.
  - Check 10 (single central) needs a second central device; done with the owner's phone in the same
    sitting.
  - Results go to `docs/hil/<date>-fec.md`; any contradiction with this ADR amends it before #43 merges.
- Later sessions only when a change touches device behaviour; each is requested as one batch with a time
  estimate.

## 10. Alternatives considered (summary)

| Decision | Alternative | Why rejected |
|---|---|---|
| btleplug in the Rust core | tauri-plugin-blec, bluest, CoreBluetooth via objc2, Web Bluetooth | §2 |
| Driver = connection factory + channel pair | `async fn` trait methods on a `dyn Trainer` | needs `async_trait` boxing per call and shared mutable access from engine and notification handler; channels match the actor engine |
| FE-C page 25 as telemetry | Cycling Power 2A63 | needs a second subscription and loses the target-limit flags; kept as the HIL-gated alternative |
| Coalescing 4 Hz writer | write on every change | can exceed the trainer's 4 Hz rate and queue stale targets |
| ERG policy in the engine | per-driver ERG logic | would differ per trainer and be untestable without hardware |
| FTMS later (M3) | FTMS first | the NEO has none (`spec/TRAINER-BLE.md`) |

## 11. Consequences

- The NEO works in v1 through one code path (FE-C) that is fully testable in CI via simulator and replays.
- Several codec details rest on a 2015 community copy of the Tacx document and on pycycling; HIL session 1
  (#44) must pass before the FE-C driver (#43) merges. The codec (#42) can be built and property-tested
  first.
- crank holds the trainer exclusively while open; the devices view must make "release" obvious.
- Real-trainer rides cannot fast-forward (ADR-0001 §5).
- Adding FTMS (#45) is a new driver only; engine, ERG policy, UI and storage are unchanged.

## 12. Open questions and follow-ups (filed)

| Item | Issue | Milestone |
|---|---|---|
| `crank-fec` codec + property tests + fuzzing | #42 | M1 |
| `crank-ble`: discovery, pairing memory, FE-C + HRS drivers, reconnect, captures | #43 | M1 |
| HIL session 1: checksum, message type, page 71, rates, ERG response, keep-alive, SIM, stall point, single central, dropout; request current Tacx doc | #44 | M1 |
| `SimTrainer`/`SimHeart` port (with the `shared/` port) | #31 | M1 |
| ERG policy and dropout pause in the engine | #32 | M1 |
| Signed `crank.app` via `just bundle` (`nix-tcc`) | #37 | M1 |
| Developer ID + notarization (distribution) | #47 | M3 |
| FTMS driver | #45 | M3 |
| Windows/Linux BLE behaviour | #41 | M3 |

#23's checklist maps to: Tacx doc → §4.1 + #44; probe script → #44; telemetry source → §5; trait + FE-C
backend + mock → §3, #42, #43, #31; pairing test FE-C vs FTMS → §5 discovery classification.

## 13. References

- Tacx, "FE-C over BLE — Simple how-to for engineers" v1.0.0 (2015), community copy:
  https://github.com/abellono/tacx-ios-bluetooth-example/blob/master/How-to%20FE-C%20over%20BLE%20v1_0_0.pdf
- Garmin Tacx trainer developer page (documentation on request): https://developer.garmin.com/tacx-trainer/
- ANT+ Device Profile — Fitness Equipment, thisisant.com (adopter login; not retrieved):
  https://www.thisisant.com/resources/fitness-equipment-device/
- pycycling, `tacx_trainer_control.py` (FE-C over BLE implementation tested with a NEO):
  https://github.com/zacharyedwardbull/pycycling
- btleplug README: https://github.com/deviceplug/btleplug ; crates.io btleplug 0.13.5 (2026-10-10)
- tauri-plugin-blec README: https://github.com/MnlPhlp/tauri-plugin-blec ; crates.io 0.17.0
- bluest on crates.io: 0.6.9 (2025-06-27)
- MDN, Web Bluetooth API (browser compatibility): https://developer.mozilla.org/en-US/docs/Web/API/Web_Bluetooth_API
- In-repo: `spec/TRAINER-BLE.md`, `shared/src/sim.ts`, `shared/README.md` (compliance definition),
  `spec/knowledge-pack/rules.md`
