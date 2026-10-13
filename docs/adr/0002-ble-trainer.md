# ADR-0002: Bluetooth stack and Trainer abstraction (FE-C first)

- **Status:** Proposed, Amended (#54)
- **Date:** 2026-10-13
- **Issue:** #27 (decides the design questions of #23); amendment #54 (trainer-following state machine, §4.4.1)
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
    pub at: MonoTime,                           // arrival time of the reply (ADR-0001 §5 tick assignment)
    pub source: ObservationSource,              // Page71Echo | Page25Flags
    pub control: Option<ObservedControl>,       // page-71 echo of the last received control page, on the grid (§4.4.1)
    pub status: Option<CommandStatus>,          // page-71 status byte: Pass | Fail | NotSupported | Rejected | Pending | Other(u8)
    pub target_limit: Option<TargetLimit>,      // page 25 flags
}

/// One echoed control page, as integers at the wire resolution (`ControlMode::grid_key`, §4.4.1).
/// A page-71 echo describes one page only: SIM is two pages (50, 51), so an echo shows half of it.
pub enum ObservedControl {
    Erg { quarter_w: u16 },                                  // page 49
    Track { grade_centi_pct: i32, crr_units: u8 },           // page 51 (Crr in 5×10⁻⁵)
    Wind { cw_centi: u8, speed_kmh: i16, drafting_centi: u8 }, // page 50
    Resistance { half_pct: u8 },                             // page 48
    NotControl(u8),                                          // e.g. page 55 was the last page received
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
- `ControlMode::Sim` needs two pages (50 then 51). When both change they go out on consecutive ticks.
- **Keep-alive:** the current control page is re-sent every 2 s even if unchanged. In SIM, pages 50 and 51
  alternate 1 s apart, each followed by the page-71 request (the SIM echo schedule, §4.4.1). Writes are idempotent;
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
    writer requests page 71 every `T_poll` = 1 s through the `requests` slot (fixed cadence, aligned in SIM
    to the page alternation of §4.4.1; no extra request after a mode change, because no single reply
    decides anything). Each reply becomes
    `TrainerEvent::Observed` with the echoed control page as an `ObservedControl` on the wire grid, the status
    byte as reported, and its arrival time. Page 25's target-limit flags are observations too. An
    observation says "the trainer reported state X when this reply arrived", never "command N was applied",
    and nothing bounds how old the reported state is.
- **How the engine uses observations:** the trainer-following state machine of §4.4.1
  (`crank-engine::erg::following`). It turns the stream of observations into *verified following*, never
  into a claim from one reply. The engine never retries a write because of an observation; the 2 s
  keep-alive is unchanged.
- Pages 48, 50 and 55 are observed through the page-71 echo when they were the last control page, and
  otherwise through page 25 behaviour and page 54.
- **Hardware gate (#44, check 3):** does the NEO answer page 70 → page 71, and does it echo bytes 4–7 of
  pages 49 and 51? If not, `Observed` comes only from page 25 flags, `trainer_following` is permanently
  `unknown` (the target-limit flags are still shown, §7.6), and the ADR is amended before #43 implements the
  step. The same check measures page-71 reply latency (request written → reply arrived) and the end-to-end
  latency from a desired change to the first reply echoing it, which calibrate the constants of §4.4.1. **Enhancement, gated on
  #44:** if the check shows that the sequence byte increments verifiably per received control page,
  command-specific correlation may be added later as a new ADR amendment; v1 does not depend on it.
- **Connect sequence:** connect → discover → subscribe `fec2` → read DIS → page 55 (rider config) →
  request page 54 (capabilities) → `TrainerLink` returned with `Link(Ready)` → engine sets the mode. The
  driver never chooses a control mode itself; restoring control is always the engine's `SetMode`.

#### 4.4.1 Trainer-following state machine (amendment, #54)

The previous rule (round 3 of the #46 pre-mortem) had two defects found by the astra pre-mortem: the 2 s
mismatch grace restarted on every desired-mode change, so during a ramp (1 Hz target updates) a trainer that
ignored every command never reached `no`; and "observed = desired → `yes`" let one stale agreeing reply flip
the state at once. Its claim that a stale reply costs at most one second was unsupported, since nothing
bounds a reply's age. The machine below replaces it. It lives in `crank-engine::erg::following`. It is a
pure function of its inputs (events in tick order, timestamps from the `Clock`, ADR-0001 §5), with no I/O.
The machine has **no grace period**. A reply that shows a value desired within the last `L` is *Lagging*
(neutral), so a trainer that is still applying an earlier value never counts as disagreeing. Because nothing
can be renewed, frequent transitions cannot postpone detection (#54 review, round 2).

**What it can and cannot claim.** A reply's content cannot show how old it is (§4.4). The machine therefore
never decides on one reply. `following` means *at least 3 consecutive agreeing replies about the current
segment, spread over at least 2 s, covering every component of the desired mode*. `not_following` means
*disagreement that has lasted at least 2 s, confirmed by a second disagreeing reply, with no verification in
between*. Stale replies are not detected. A trainer that keeps sending stale agreeing replies for 2 s or
more is reported `following`. Only a verified sequence byte (§4.4 enhancement, gated on #44) could close this
gap, and v1 claims nothing beyond the two definitions.

**Activity.** The machine is *active* exactly while two things hold: the trainer link is `Ready`, and the
engine has a current desired `ControlMode` (`Some`). It becomes active whenever both start to hold, whatever
the cause: first connect, reconnect (§6), the start of a ride on a connection that stayed open, or control
resuming after "Release trainer". It becomes inactive the moment either stops holding. The engine derives
`Activate`/`Deactivate` from these two facts alone, so a second ride on the same connection activates the
machine like the first.

**Comparison grid and components.** Desired and observed values are compared as integers at the wire
resolution, never as floats. `ControlMode::grid_key()` in `crank-devices` gives ERG in 0.25 W, grade in 0.01 %
(offset as on page 51), Crr in 5×10⁻⁵, resistance in 0.5 %, and wind in 0.01 kg/m, 1 km/h and 0.01. The
engine emits desired modes already on that grid (§7.1 rounds ERG to 1 W, which is exact in 0.25 W; grade,
Crr, resistance and wind are rounded to the grid where the engine produces them). A desired mode has
**components**, one per control page that carries it: ERG has `target` (page 49), resistance has `resistance`
(page 48), and SIM and `Free` (flat-road SIM, §7.9) have `track` (page 51: grade + Crr) and `wind` (page 50).
A page-71 echo describes one page, so it says something about exactly one component. The engine compares
the echo with the desired mode *projected onto that component*. An echo of a page that is not a component of
the desired mode (an ERG echo while SIM is desired) matches no value of the current segment.

**SIM echo schedule (driver, §4.4).** So that both SIM components produce evidence, in SIM the writer
alternates the keep-alive pages: page 50 and page 51 each every 2 s, 1 s apart, and the 1 s page-71 request
follows each of these writes on the next free tick. In steady SIM the replies therefore alternate between
wind and track echoes. A changed SIM page is still written at once (on consecutive ticks if both changed),
and the alternation continues from there.

**Inputs.**

1. *Desired-state changes* from the engine, each classified exactly once:

   | Change | Class |
   |---|---|
   | The machine becomes active (see Activity) | `Activate` |
   | Mode kind change (ERG ↔ SIM ↔ resistance ↔ `Free`), including ERG release to `Resistance { pct: 0 }` (§7.4) and ERG (re-)engagement, which starts the soft start | `Transition` |
   | Block boundary whose start target differs from the previous block's end target (§7.8 a) | `Transition` |
   | Manual ± or coach change (§7.8 b) | `Transition` |
   | Rider-set SIM grade or resistance from the free-ride/sim control (§7.9) | `Transition` |
   | Ramp update inside a block (§7.3), soft-start update after its engagement (§7.4–7.5), HR-steering update (§7.7) | `Continuous` |
   | Block boundary with an equal target, keep-alive re-send, `SetRider` (page 55) | none (the desired control mode is unchanged) |
   | The machine stops being active: link `Lost`/`Connecting`, `Disconnect`, "Release trainer", or the engine has no desired mode any more | `Deactivate` |

   `Transition` is the §7.8 discontinuity set plus mode-kind and rider-set changes. Settle (§7.8) and this
   machine use the same classification, which the engine computes once per change. A `Transition` starts a
   new **segment** (`seg_start` = now).
2. *Observations* (`TrainerEvent::Observed`), classified when they arrive, at time `t` = `at`:
   - **Neutral**: page-25 flags, `control = None` or `NotControl`, status `Pending` or `Other`. A neutral
     observation changes no state and no timer.
   - **Disagree**: status `Fail`, `NotSupported` or `Rejected` (even when the echo equals the desired
     value); or status `Pass` with an echo that matches no desired value at any instant in `[t − L, t]`; or
     an *incomplete* reply (below).
   - **Agree**: status `Pass` and the echo equals the component projection of some desired mode that was
     current at some instant in `[max(seg_start, t − L), t]`, **and** the desired mode is complete (below).
     The lag window `L` lets a trainer that echoes the previous ramp step agree.
   - **Lagging**: status `Pass` and the echo equals the projection of a desired value from before
     `seg_start` that was current at some instant in `[t − L, t]`. The trainer is still applying the
     previous segment. Lagging changes neither the streak nor the episode. It refreshes `last_decisive` and
     the component's `last_evidence`.
   - **Complete / incomplete** (multi-component modes, i.e. SIM and `Free`): a required component c is
     *stale* at t when `t − max(required_since[c], last_evidence[c]) ≥ T_component`, where `last_evidence[c]`
     is the arrival of the last Agree or Lagging reply for c. `required_since[c]` is set when c becomes
     required (on `Activate`, or a `Transition` into a mode that has c), **unless c was already required at
     some instant in the last `T_component`**, in which case it keeps its value. Neither a storm of grade
     changes nor rapid mode-kind toggling can therefore restart the clock. A reply that would be Agree while
     another required component is stale is *incomplete* and counts as Disagree. A trainer that applied the
     wind page but never the track page goes to `not_following`; it never stays `checking`.
3. *Ticks* (4 Hz, ADR-0001 §5) for the timeouts.

`TrainerEvent::Delivered` and `Fault(Gatt)` are not inputs. Delivery says nothing about what the trainer
does (§4.4).

**State and variables.** State ∈ {`Inactive`, `Unverified`, `Following`, `Mismatch`, `Unknown`}. Variables:
`seg_start` (time of the last `Activate`/`Transition`), the desired history over the last `L` with its
segment boundaries, `streak` (the consecutive Agree replies since the last Disagree: count, arrival time of
the first, and the set of components seen), `last_evidence[c]` and `required_since[c]` per component,
`dis_since` (start of the open
disagreement episode, or none), and `last_decisive` (arrival time of the last Agree/Disagree/Lagging reply,
or the activation time).

**Constants.**

| Constant | Value | Source |
|---|---|---|
| `T_poll` page-71 request period | 1 s | §4.4 |
| `L` lag window: how long a superseded desired value is still accepted as the trainer catching up | 2 s | Budget: desired change → control write (≤ 2 writer ticks, 0.5 s; SIM may need two pages), plus the trainer applying it, plus the reply latency. **HIL calibration (#44 check 3)**: L ≥ p99 of (desired change → arrival of the first reply that echoes it), measured end to end through the writer, so the slot and coalescing delay are included |
| `N_follow` agreeing replies to verify | 3 | Design: more than one reply, so a single or duplicated reply never decides |
| `T_verify` minimum span from first to last of those replies | 2 s | Design: three replies at `T_poll` = 1 s. A burst of delayed replies arriving together never verifies |
| `T_component` maximum age of a SIM component's last evidence | 4 s | The SIM echo schedule gives each component a reply every 2 s, plus `L`. **HIL calibration (#44 check 3)**: SIM replies must alternate, as the schedule above requires |
| `T_mismatch` disagreement duration | 2 s | The previous 2 s rule (8 ticks) |
| `T_unknown` without a decisive reply | 5 s | The previous 5 s rule |

A change to a value in this table is an ADR amendment backed by #44 numbers.

**Transition table.** Rows are applied in order. The first matching row wins.

| # | State | Input | Guard | Next state | Actions |
|---|---|---|---|---|---|
| 1 | any | `Deactivate` | none | `Inactive` | clear all variables |
| 2 | `Inactive` | `Activate(D)` | none | `Unverified` | `seg_start` = now, history = [D], `streak` = empty, `last_evidence` = none, `required_since[c]` = now for D's components, `dis_since` = none, `last_decisive` = now |
| 3 | `Inactive` | any other | none | `Inactive` | ignore. `Transition`/`Continuous` cannot reach an inactive machine, because the first desired mode while the link is `Ready` is `Activate` (see Activity). Observations are ignored |
| 4 | `Following` | `Transition(D)` | none | `Unverified` | `seg_start` = now, append D to history, `streak` = empty, update `required_since` per the rule above |
| 5 | `Unverified`, `Mismatch`, `Unknown` | `Transition(D)` | none | unchanged | as row 4. `dis_since` is **kept**: a disagreement episode ends only through verification, silence or `Deactivate`, so neither a new target nor a storm of them can hide a trainer that ignores commands, and `Mismatch` is not cleared by a new target |
| 6 | any active | `Continuous(D)` | none | unchanged | append D to history. **No timer, streak or episode is reset** |
| 7 | any active | Agree | after adding it, `streak` has ≥ `N_follow` replies spanning ≥ `T_verify` and covers every component of the current desired mode | `Following` | add to `streak`, `last_evidence[c]` = t, `dis_since` = none, `last_decisive` = t |
| 8 | any active | Agree | otherwise | `Unknown` → `Unverified`; others unchanged | add to `streak` (first = t if it was empty), `last_evidence[c]` = t, `last_decisive` = t |
| 9 | any active | Lagging | none | `Unknown` → `Unverified`; others unchanged | `last_decisive` = t, `last_evidence[c]` = t |
| 10 | any active | Disagree | `dis_since` ≠ none and `t − dis_since ≥ T_mismatch` | `Mismatch` | `streak` = empty, `last_decisive` = t |
| 11 | any active | Disagree | otherwise | `Mismatch` stays; others → `Unverified` | `streak` = empty, `last_decisive` = t, `dis_since` = `dis_since` or t |
| 12 | any active | Neutral | none | unchanged | none |
| 13 | `Unverified`, `Following`, `Mismatch` | tick | `now − last_decisive ≥ T_unknown` | `Unknown` | `streak` = empty, `dis_since` = none |

Transitions only move the segment boundary. The time a value stays acceptable is measured from when it
stopped being desired, never from a transition, so a transition storm cannot extend it.

**Outputs.** The engine maps state to `TrainerFollowing`, a field of `RideStatus`. The UI sees it in the
`status` of a `StreamFrame` (ADR-0001 §4.3): sent on change, so within one frame, coalesced to the latest
value, and re-read by `ride_status()` after a `resync`.

| State | `TrainerFollowing` (serde) | UI | Compliance (§7.8) |
|---|---|---|---|
| `Inactive`, `Unknown` | `unknown` | no claim | counted |
| `Unverified`, `dis_since` = none | `checking` | no claim | counted |
| `Unverified`, `dis_since` ≠ none | `suspect` | no claim | **excluded** |
| `Following` | `following` | none needed (normal state) | counted |
| `Mismatch` | `not_following` | "Trainer is not following the target" in the ride status | **excluded** |

A record second is excluded from compliance when any of its four ticks is `suspect` or `not_following`.
Excluding `suspect` covers the detection delay before `not_following`, so nothing is rewritten afterwards
(no rollback, ADR-0001 §5). The page-25 target-limit flags are a separate `RideStatus` field (§7.6) and
never enter this machine.

**Guarantees.** These assume a reply arrives every `T_poll` with latency ≤ r. They are what the tests
assert.

- **Stuck trainer.** Let V be the value its echo shows. It reaches `not_following` no later than
  `L + 2·T_poll + T_mismatch + r` (6 s + r) after the later of activation and the last instant V was
  desired. Any number of `Transition` and `Continuous` inputs in between changes nothing, including a
  `Transition` every tick.
- **SIM component never applied.** With SIM or `Free` desired, a trainer that never echoes an agreeing
  value for one component reaches `not_following` no later than
  `T_component + T_mismatch + 2·T_poll + r` (8 s + r) after `required_since` for that component. It never
  reaches `following` in between, because verification needs every component. Grade changes and mode-kind
  toggles do not move `required_since` (rule above).
- **Lagging trainer.** A trainer whose echo lags the desired value by less than `L` never produces a
  Disagree. It reaches `following` no later than `2 ticks + 2·T_poll + T_verify + r` after a `Transition`
  in ERG or resistance (4.5 s + r), and one `T_poll` later in SIM (5.5 s + r). During `Continuous` updates
  it never leaves `following`.
- **Single replies.** One observation never produces `following`, from any state and whatever its
  content. Agreeing observations that all arrive within less than `T_verify` never produce it either. One
  disagreeing observation never produces `not_following`. That needs two Disagree observations at least
  `T_mismatch` apart, with no verification between them.
- **Leaving `not_following`.** It ends only through verification (`following`), `T_unknown` of silence, or
  `Deactivate`. A new target does not end it.
- **Silence.** A trainer that sends no decisive reply is `checking` for `T_unknown` (5 s) after each
  `Activate`. It is then `unknown` until a decisive reply arrives or the machine deactivates; `Transition`
  inputs do not leave `unknown`. A trainer that never answers page 70 (the #44 check-3 fallback) therefore
  shows `checking` for the first 5 s of every activation and `unknown` after that.
- **Repeated rides.** Every activation starts from the same initial state, so the second ride on an open
  connection behaves exactly like the first.

**Tests required.** These are listed on #32 (machine and engine) and #43 (driver), and form the acceptance
for those issues:

- *#32, property tests on the pure machine (proptest, `VirtualClock`), with generated desired-change
  sequences (all classes) and generated observation streams (arrival jitter, latency 0..r, neutral
  observations interleaved):*
  1. **Ramp, stuck trainer.** For any ramp whose segment values never equal the stuck echo, the state is
     `not_following` within the stuck-trainer bound. `Continuous` updates never delay it.
  2. **Transition storm.** From `Unverified` (with or without an open episode) and from `Following`, a
     `Transition` every 1 s or every tick, with a stuck echo, reaches `not_following` within the stuck-trainer
     bound and never returns to `checking`.
  3. **Single stale agreeing observation.** In any stream where at most one observation agrees (the rest
     disagree, lag, are neutral or absent), `following` never occurs.
  4. **Burst.** Any number of agreeing observations whose arrivals span less than `T_verify` never causes
     `following` on their own.
  5. **Verification shape.** Every entry into `following` is preceded by ≥ `N_follow` consecutive Agree
     observations (neutral and lagging ones skipped) about the current segment, spanning ≥ `T_verify` and
     covering every component of the desired mode.
  6. **Single disagreeing observation.** In any stream with at most one Disagree, `not_following` never
     occurs. Every entry into `not_following` is preceded by two Disagree observations ≥ `T_mismatch`
     apart, with no verification between them.
  7. **Lagging but following trainer.** If the echo lags desired by less than `L`, across any mix of
     `Transition` and `Continuous` (ramp, soft start, HR steering, rapid manual ±), no Disagree is produced
     and the output is never `suspect` or `not_following`.
  8. **Time to verify.** After any `Transition`, a lagging-but-following trainer reaches `following` within
     the bounds above (ERG/resistance and SIM).
  9. **Transitions do not clear an episode.** In `Mismatch`, any sequence of `Transition`/`Continuous`
     without verification stays `not_following`. Transitions never reset `dis_since`.
  10. **SIM components.** ERG → SIM (and ERG → `Free`) where only page-50 echoes agree and page 51 is never
      echoed, or is echoed with a wrong grade: never `following`, and `not_following` within the SIM
      component bound. The same holds under a storm of grade `Transition`s every 1 s and under ERG ↔ SIM
      toggling faster than `T_component`. With alternating agreeing echoes of both pages it reaches
      `following` within the SIM bound, including right after ERG → SIM when ERG lasted ≥ `T_component`.
  11. **Neutral and silence.** Neutral observations change no state or variable. After `T_unknown` without
      a decisive observation, any active state becomes `unknown` and stays there across `Transition`s. A
      silent trainer is `checking` for exactly `T_unknown` after `Activate`.
  12. **Activity and lifecycle.** `Deactivate` gives `unknown` (`Inactive`) at once from any state.
      Observations while inactive are ignored. Activation happens on any start of (link `Ready` ∧ desired
      mode `Some`): connect, reconnect, a new ride on an open connection, control after "Release trainer".
  13. **Status codes.** `Fail`/`NotSupported`/`Rejected` disagree even with an equal echo. `Pending`/`Other`
      are neutral.
  14. **Grid comparison.** For every `ControlMode` the engine can emit, `grid_key` matches its FE-C
      projection exactly, per component.
  15. **Determinism and catch-up.** The same timestamped inputs give the same outputs whether ticks run
      live or as a catch-up burst after a stall (ADR-0001 §5).
  16. **Classification.** For generated workouts with manual and coach changes, every desired change gets
      exactly one class per the table above, and the `Transition` set equals the settle triggers of §7.8.
  17. **Compliance.** Seconds with a `suspect` or `not_following` tick are excluded. No other second is
      affected by the machine.
- *#32, engine scenarios with `SimTrainer`* (its `Observed` emission gets fault modes: stuck echo, echo
  lag d, injected stale replies, silent page 71, SIM page 51 dropped):
  - A 20-min ramp with a stuck echo reaches `not_following` within the bound of test 1, and compliance
    excludes it. The same ramp with lag < `L` stays `following`.
  - A reconnect mid-ramp goes `unknown` → `checking` → `following`.
  - **Second ride without reconnecting:** finish ride 1, keep the link `Ready`, load and start ride 2. The
    machine deactivates when the engine drops its desired mode (if it does) and activates on ride 2's first
    `SetMode`, reaching `following`. If the engine keeps a desired mode between rides, the change is a
    `Transition` and the machine stays active.
  - ERG → free ride (SIM) with page 51 dropped reaches `not_following`.
- *#43, driver (FE-C):*
  1. **Poll cadence.** For arbitrary command sequences through the 4 Hz writer, a page-70 request for page 71
     is written every `T_poll` (± one tick) while a controlled mode is active, never otherwise. It is never
     written more than once per tick and never delays a `control` write.
  2. **SIM echo schedule.** In steady SIM, page 50 and page 51 are each written every 2 s, 1 s apart, and
     each is followed by the page-71 request on the next free tick. Consecutive replies from a faithful fake
     transport alternate between wind and track echoes.
  3. **Write latency.** For any command sequence, a desired-mode change reaches its control write within
     2 ticks (both SIM pages within 2 ticks when both changed). This is the writer's share of the `L` budget.
  4. **One reply, one observation.** Every received page 71 gives exactly one `Observed`, in arrival order,
     with `at` = arrival `MonoTime`. The driver never synthesizes, deduplicates, reorders or drops
     observations.
  5. **Echo decoding round trip.** For pages 48/49/50/51, `encode(page)` → echo bytes 4–7 →
     `ObservedControl` equals the `grid_key` projection of the mode that produced the page. Page 55 and
     unknown pages give `NotControl(page)`. All status byte values decode, and unknown ones become `Other`.
  6. **Replay.** The #44 check-3 capture replayed through `ReplayTrainer` into the machine gives `following`
     for its steady ERG and SIM segments. Its measured desired-change → echo latency is below `L`, and its SIM
     replies alternate within `T_component`.

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
  marks the trainer `Lost`, which deactivates the trainer-following machine (§4.4.1, `unknown`). The engine enters `paused(trainer_lost)` (ADR-0001 §5): from the detecting tick
  workout time and the block timer freeze while session time continues; the record gets `null` (no
  interpolation). The ≤ 2 s detection window before that stays workout time with `null` record values
  (ADR-0001 §5).
- **Reconnect loop** (driver task, active while a ride is loaded or the ride view is open): connect to the
  known peripheral, backoff 0.5 s, 1 s, 2 s, 4 s, then every 5 s, until the rider ends the ride or releases
  the trainer. Each attempt is logged; the UI shows "reconnecting (attempt n)".
- **On reconnect**, in this order: (1) the driver re-subscribes `fec2`, re-sends page 55 and re-checks page
  54, then emits `Link(Ready)`; (2) on `Ready` the engine issues `SetMode` with the current mode (in ERG via
  the soft start of §7, because the trainer may have been in its standalone mode while disconnected,
  measured in HIL check 11); this `SetMode` makes the following machine active again (`Activate`, §4.4.1), and
  observations before it are ignored; (3) the ride resumes when cadence > 0 (ADR-0001 §5). The driver never re-sends
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
   start's 5 s plus 5 s. These are the `Transition` inputs of the trainer-following machine (§4.4.1), from
   the same classification. Within `ramp` blocks and HR-steered steps the target moves continuously
   (`Continuous`) and is compared second by second with no settle. Released (§7.4) and paused seconds are
   excluded, and so is every second with a `suspect` or `not_following` tick (§4.4.1). Compliance consumes
   the state machine's output, never command acknowledgements or single replies. Compliance
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
  - Check 3 also records whether page 71 echoes the command's data bytes (the observation model of §4.4
    relies on it), whether its sequence byte advances (the gated enhancement of §4.4), and the
    distribution of the latency from a desired change, through the writer's slot delay and the control write,
    to the first reply echoing it (end to end). It also checks that SIM replies alternate between pages 50
    and 51. These calibrate `L` and `T_component` (§4.4.1).
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
