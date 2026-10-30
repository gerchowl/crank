# ADR-0002: Bluetooth stack and Trainer abstraction (FE-C first)

- **Status:** Proposed, Amended (#54, #250, #264, #266)
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

pub enum TrainerCommand { SetMode { mode: ControlMode, seq: CommandSeq }, SetRider(RiderConfig), Disconnect }
// `seq` comes from the link's shared `SeqSource` (`TrainerLink::seq`), so every sender's commands are distinct (#345)

pub enum TrainerEvent {
    Sample(TrainerSample),
    Delivered { mode: ControlMode, seq: CommandSeq }, // GATT write-with-response succeeded: bytes reached the peripheral, nothing more; confirms only the SetMode of `seq` (#345)
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

**HIL result (#44 Part A, 2026-10-14, `docs/hil/2026-10-14-fec.md`).** The NEO answered a page-70 request
under all four checksum rules, including a deliberately wrong one: it **does not verify the checksum on
writes** (11 of 12 requests were answered; the one lost reply is the §4.4 effect, not a rejection). It also accepted
message type `0x4E` as well as `0x4F`, with and without GATT response; every effect check (page 49 followed by
the page-71 echo) matched. The decisions above stand: the NEO's own frames always carry XOR including sync,
and that is what crank sends and verifies. Nothing about the NEO's tolerance is relied on.

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

Unused bytes are `0xFF`. Unknown pages decode to `Page::Unknown { page, raw }` and are logged at debug.

**Tentative pending HIL #44** (the ANT+ FE profile was not retrievable, §4.1; the items below are codec
policy, either raw values kept without meaning or a mask or validity rule, no semantics invented). Part A
results (2026-10-14, NEO fw 0.7.4) are marked; the rest stays tentative:

- Page 16: `equipment_type` is masked with `0x1F` (bits 5–7 reserved). **Part A:** bits 5–7 were 0 in all
  456 captured page-16 frames (type `0x19`), so the mask never mattered on this NEO; it stays tentative
  because zero bits do not prove the bits are reserved.
- Page 54: `max_resistance_n` is the raw `u16`; whether `0xFFFF` means unavailable is undecided and left to
  the consumer (`crank-ble` currently maps it to `None`, covered by the driver test
  `page_54_max_resistance_0xffff_is_unavailable_and_other_values_are_kept`). **Part A:** the NEO reports `200` N and modes
  `0x07` (basic resistance, target power, simulation) in every reply; `0xFFFF` was never seen. Whether
  `0xFFFF` means unavailable stays tentative.
- Page 55: wheel offset nibble 0–9 is millimetres and `0xF` means no offset; 10–14 decode to `None` pending
  confirmation that 10 is not valid. **Part A:** the NEO accepts the corrected layout (708 mm sent as byte 6 =
  70, nibble 8; read back byte for byte), and stores and reports nibbles 10, 12 and 14 verbatim; it reports a
  sent `0xF` as `0`. The NEO therefore gives no evidence either way about 10–14, and the decode to `None`
  stays a codec policy, tentative.
- Page 70: a count of 0 encodes `0x80` and decodes to `times: 0, acknowledged: true`. **Part A (n = 1 trial
  per count):** `times` = 1 gave one reply, 3 gave three, and 0 gave five replies within 0.82 s of the
  request (an immediate pair, then one per ~0.24 s) and then none within the 4 s window. So the NEO
  repeats a count-0 request a bounded number of times (five in the one trial), which fits "until
  acknowledged" with a cap; the cap itself is one observation and stays tentative.
- Requests for a page the NEO does not serve as data (16 and 25, one trial each). **Part A:** it answers
  with page 71 whose last-page byte is the *requested* page (`0x10`, `0x19`) and status `2` (not
  supported): `47 10 DD 02 FF FF FF FF`. An unsupported request therefore overwrites page 71's
  last-page field, so the next poll no longer echoes the last control page (it reads `NotControl`, see
  rule R1 in §4.4.1, engine routing tracked in #226). Whether a *supported* request (54, 80, 81, 55) also overwrites it is not
  established by Part A (no request-then-poll capture); crank sends only 54 (at connect, before any
  poll) and 71.
- Pages 48/50 endpoints. **Part A (settled):** 100 % (`200`) and page 50 at 1.86 / +127 km/h / 1.0 are
  applied as numbers (echo `C8` and `BA FE 64`).
- Pages 49 and 51 endpoints. **Part A (settled, contradicts the earlier "numeric, not sentinels"):** the
  NEO treats page 49 `0xFFFF` and page 51 Crr `0xFF` as invalid. The echo of `0xFFFF` reads `0x0000` (the
  previous target is replaced by 0 W); the echo of Crr `0xFF` keeps the previous Crr. **Confirmed by HIL
  Part A** (`docs/hil/2026-10-14-fec.md`, check 3, "Endpoints"): `encode` rejects both with
  `EncodeError` (`Field::Watts`, `Field::Crr`), judged on the wire value after rounding, so the largest valid
  values are 16383.5 W (`0xFFFE`) and Crr 0.0127 (`0xFE`) (#215; these two are inferred from the sentinels,
  not measured: Part A sent only `0xFFFF` and `0xFF`).
- Page 49 resolution. **Part A:** the NEO quantises the target to whole watts: 200.25 W is echoed as 200 W
  and 200.5 W as 201 W, so §7.1's 1 W grid is the real resolution and 0.25 W is wire format only.
- Pages 54/80/81 layout. **Part A (settled):** page 80 reads hardware revision 1, manufacturer 89
  (`0x59`, Tacx), model 2800 (`0x0AF0`); page 81 reads supplemental revision 0, main software revision 7 (DIS
  firmware reads 0.7.4) and a serial; all as the codec decodes them.

### 4.3 Codec API

```rust
// crates/crank-fec/src/lib.rs (sketch) — no I/O, no async, no allocation
pub enum MsgType { Broadcast = 0x4E, Acknowledged = 0x4F }
pub struct AntFrame { pub msg_type: MsgType, pub channel: u8, pub page: Page }

pub enum Page {
    GeneralFe(GeneralFe), Trainer(TrainerData),
    BasicResistance { pct: Option<f32> }, TargetPower { watts: f32 },
    Wind { cw_kg_per_m: Option<f32>, speed_kmh: Option<f32>, drafting: Option<f32> },
    Track { grade_pct: Option<f32>, crr: f32 },
    Capabilities(FeCapabilities),
    UserConfig { rider_kg: Option<f32>, bike_kg: Option<f32>, wheel_m: Option<f32>, gear_ratio: Option<f32> },
    Request { page: u8, times: u8, acknowledged: bool, command_type: u8 },
    CommandStatus(CommandStatus), Manufacturer(ManufacturerInfo), Product(ProductInfo),
    Unknown { page: u8, raw: [u8; 8] },     // all eight page bytes verbatim
}
// Control pages round-trip through decode, so the control variants above are also decoded.
pub enum ControlPage {
    BasicResistance { pct: f32 }, TargetPower { watts: f32 },
    Wind { cw_kg_per_m: f32, speed_kmh: f32, drafting: f32 },
    Track { grade_pct: f32, crr: f32 },
    UserConfig { rider_kg: f32, bike_kg: f32, wheel_m: f32 },
    Request { page: u8, times: u8 },
}
#[non_exhaustive] pub enum Field { Pct, Watts, CwKgPerM, SpeedKmh, Drafting, GradePct, Crr, RiderKg, BikeKg, WheelM, Times }
pub struct EncodeError { pub field: Field }
pub fn encode(p: &ControlPage) -> Result<[u8; 13], EncodeError>;   // range-checked, never clamps silently
pub fn decode(b: &[u8]) -> Result<AntFrame, FrameError>;           // Sync | Length | Checksum | Truncated
```

Values that do not fit a page field (target at or above 16 383.625 W (rounds to `0xFFFF`), grade outside ±200 %, Crr at or above 0.012725 (rounds to `0xFF`),
negative masses) are `EncodeError`s; clamping to trainer capabilities is the engine's explicit job (§7).
`crank-devices` rejects SIM's Crr above `CRR_MAX` = 0.0127 (unit `0xFE`) at the mode boundary (`ControlMode::grid_key`),
so the same typed error surfaces there and not late from the BLE writer (#237).

### 4.4 Command pacing and acknowledgement

- **One writer task per connection**, ticking at **4 Hz** (Tacx's rate). It holds coalescing slots,
  not a queue: `control` (latest `ControlMode`), `rider` (page 55), `requests` (FIFO of page-70 requests).
  Per tick it writes at most one message, priority reserved page-71 poll (subject to the SIM-pair exception below) > `control` > `rider` > other `requests`. A newer target
  replaces an unsent older one, so the trainer always gets the latest target and nothing piles up.

**Amendment (#43): reserved poll tick (2026-10-13).** A page-71 request due on the nominal
1 s grid reserves the next writer tick, even with single-page control pending; it cannot occupy two
consecutive ticks or defer the same pending value twice. This prevents observation starvation while
keeping single-page control latency within two ticks. A pending SIM pair (50 then 51) instead runs on
two consecutive ticks before the poll; that poll may be late by two ticks, with no change to nominal
due times. Steady SIM alternates keep-alive pages one second apart, polling on the following tick.
For continuously superseded SIM pairs, finish the current pair then service an overdue poll before
starting another pair. An in-flight pair finishes its track page with the newest value before a
coalesced wind update, so a superseding SIM update can be delivered track then wind. A poll already
deferred by one SIM pair takes the next tick even if a newer pair is pending; polls are never more
than two ticks late against the nominal grid and never occupy consecutive ticks. Both SIM pages are
written within two ticks of the desired change, except when the pair's first free tick goes to such
an already-deferred poll: then completion is within three ticks (0.75 s), never more. ERG, resistance,
and single-page control remain within two ticks. This bounds observation starvation, which affects
safety-relevant trainer_following. The extra SIM tick arises under back-to-back changes faster than
real terrain; the writer's share of `L` is 0.5 s nominal. #44 check 3 measured the age of stale echoes
including slot/coalescing delay (§4.4.1: at most about 0.8 s against `L` = 2 s).
Latency is measured per page from the moment it first becomes pending; coalescing replaces the value, not the deadline.
Rider configuration is written on connection; explicit SetRider on a live connection returns a
protocol fault and requires a new connection. Reserving polls avoids five-second observation expiry;
one tick of single-page delay remains inside the control latency budget.

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
    `TrainerEvent::Delivered { mode, seq }`, meaning the bytes of the `SetMode` of sequence `seq` reached the
    peripheral and nothing more (a mode replaced before its last page is never confirmed); a GATT
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
  **Result (#44 Part A, 2026-10-14):**
  - *Gate passes.* The NEO answers page 70 → page 71 (request-to-reply p50 0.06 s) and echoes bytes 4–7 of
    pages 48, 49, 50 and 51 exactly, with status `0` (pass) and the last control page in byte 1. Page 55 is
    **not** echoed: after a page-55 write byte 1 still names the previous control page. Before any control
    page, byte 1 is `0x30` and the echo is `FF FF FF 00`, which is a valid resistance echo of 0 %. Callers
    must therefore not treat that echo as a trainer-following disagreement.
  - *Sequence byte.* It increments by 1 for each control page (48–51) and for each page-70 request the
    NEO receives (not for page 55), wrapping at 256. It does increment verifiably, so the gated enhancement
    is feasible later (a reply whose sequence is exactly 1 above that of a request/control pair proves no
    other message intervened); v1 stays as decided.
  - *Lost replies.* A page-70 request for page 71 is sometimes never answered, with the sequence byte
    showing it was received. Counted by sequence number over the Part A captures
    (`frames::reply_accounting`, offline test with duplicates and gaps): 5 of 79 requests sent in a row
    (250/500/1000/2000 ms apart), 4 of 40 requests 0.1–1 s after a control write, and 27 of 344 (7.8 %) of
    the driver's own 1 s polls; 0 of 12 in check 2 and 3 of 32 in check 3. The loss does not depend on
    spacing (2/19 at 250 ms, 0/19 at 500 ms, 3/19 at 1000 ms, 0/22 at 2000 ms) or envelope (§4.1). A
    reply that does arrive does so within 0.12 s (n = 317); a lost one is not answered later. The next
    poll is the recovery, which is already the design (never one reply). In SIM it costs one reply period
    (2 s) for the affected component.
  - *Repeated replies.* Of 397 page-71 frames in the driver run, 80 are `0x4E` (broadcast) copies of a
    `0x4F` reply with the same sequence and bytes: 66 within 1 ms of the original, the rest up to one
    4 Hz slot (0.24 s) later. Replies are not always `0x4F`. The driver turns every frame into its own
    `Observed` event ("one reply, one observation", no deduplication). That is harmless to
    §4.4.1: a copy carries the same value and sequence, so it can never disagree with its original, and
    `T_verify` requires the agreeing replies of a verification to be spread over at least 2 s, so a burst
    of copies cannot verify a component. A copy does count toward `N_follow`, so a verification may rest
    on two distinct replies at least 2 s apart plus a copy; the sequence byte would allow deduplicating
    copies exactly, which is the same gated enhancement as above.
- **Connect sequence:** connect → discover → subscribe `fec2` → read DIS → page 55 (rider config) →
  request page 54 (capabilities) → `TrainerLink` returned with `Link(Ready)` → engine sets the mode. The
  driver never chooses a control mode itself; restoring control is always the engine's `SetMode`.

#### 4.4.1 Trainer-following state machine (amendment, #54)

**Why it was restructured.** The first rule (round 3 of the #46 pre-mortem) had two defects found by the
astra pre-mortem. The mismatch grace restarted on every 1 Hz ramp update, and one stale agreeing reply
flipped the state. Its fixes in rounds 1–2 of #54 kept one aggregate machine with per-component clauses
added in prose, and the review kept finding interactions between them:

- grace renewed by transition storms;
- stale SIM components blocking each other's recovery;
- lagging replies bypassing the missing-component check;
- a verification deadline that assumed quiet segments.

The design is therefore split in two:

- **one small machine per commanded component**, with its own evidence, expiry and verdict;
- an **aggregate verdict that is a pure function** of the component verdicts and holds no memory of its own.

The transition tables below are implemented literally by an executable model,
`docs/adr/models/trainer_following.py` (Python, stdlib only). The model explores the discretised state
space exhaustively and replays every pre-mortem counterexample as a named scenario. The `following-model`
flake check runs it in CI. The Rust implementation (`crank-engine::erg::following`, #32) must agree with
it.

**What it can and cannot claim.** A reply's content cannot show how old it is (§4.4). The machine therefore
never decides on one reply.

- A component is `following` after at least 3 consecutive agreeing replies about it, spread over at least
  2 s.
- It is `not_following` after a disagreement that has lasted at least 2 s, confirmed by a second
  disagreeing reply, with no verification in between.
- The aggregate is `following` only when every component of the desired mode is.

Stale replies are not detected. A trainer that keeps sending stale agreeing replies for 2 s or more is
reported `following`. Only a verified sequence byte (§4.4 enhancement, gated on #44) could close this gap,
and v1 claims nothing beyond these definitions.

**Components and grid.**

- Values are compared as integers at the wire resolution, never as floats. `ControlMode::grid_key()` in
  `crank-devices` gives ERG in 0.25 W, grade in signed 0.01 % (without page 51’s offset), Crr in 5×10⁻⁵, resistance in
  0.5 %, and wind in 0.01 kg/m, 1 km/h and 0.01.
- The engine emits desired modes already on that grid. §7.1 rounds ERG to 1 W, which is exact in 0.25 W.
  Grade, Crr, resistance and wind are rounded to the grid where the engine produces them.
- A desired mode has one **component** per control page that carries it:

  | Desired mode | Components (page) | Reply period `P_c` |
  |---|---|---|
  | ERG | `erg` (49) | 1 s |
  | resistance | `resistance` (48) | 1 s |
  | SIM and `Free` (flat-road SIM, §7.9) | `track` (51: grade + Crr), `wind` (50) | 2 s each |

- A page-71 echo describes one page, so it is a reply about exactly one component.

**SIM echo schedule (driver, §4.4).** In SIM the writer alternates the keep-alive pages: page 50 and page 51,
each every 2 s and 1 s apart. The 1 s page-71 request follows each of these writes on the next free tick.
Each SIM component therefore gets a reply every 2 s. A changed SIM page is still written at once (on
consecutive ticks if both changed), and the alternation continues from there.

**Activity.** The machine is *active* exactly while two things hold:

- the trainer link is `Ready`;
- the engine has a current desired `ControlMode` (`Some`).

`Activate` fires whenever both start to hold, whatever the cause: first connect, reconnect (§6), the start of
a ride on a connection that stayed open, or control resuming after "Release trainer". `Deactivate` fires the
moment either stops holding, and clears every component. A second ride on the same connection therefore
activates exactly like the first.

**Desired history.** For each component the engine keeps the values it desired in the last `L`, with the
instant each stopped being desired. A changed desired mode updates that history and runs C8/C9 for
components that stop or start being required. **Desired-state changes are not otherwise an input to any
machine.** Transitions and continuous updates are treated identically: lag acceptance runs from when a value
stopped being desired, never from a transition, so neither a storm of transitions nor a ramp can defer or
renew anything.

**Constants.**

| Constant | Value | Source |
|---|---|---|
| `T_poll` page-71 request period | 1 s | §4.4. Gives `P_c` above (SIM: alternation) |
| `L` lag acceptance: how long after a value stopped being desired an echo of it still agrees | 2 s | Bounds the age of a **stale echo**: an echo of a value that has been superseded by a newer desired value. Budget: the writer delay (≤ 2 ticks (0.5 s) nominal; ≤ 3 ticks (0.75 s) worst case for a SIM pair behind a deferred poll, §4.4) during which polls still read the old value, plus reply latency (≤ 0.12 s). **HIL calibration (#44 check 3, Part A, 2026-10-14, `docs/hil/2026-10-14-fec.md`)**: in the driver run (60 ERG and 30 SIM changes, 397 page-71 frames, about 6 min) 2 echoes were stale after the new value was written, 16 ms and 22 ms after the write (both ERG; none in SIM track), so the age from the desired change is at most 0.75 s + 0.022 s ≈ 0.8 s. `L` = 2 s keeps 1.2 s of margin. (The first-echo latency of a new value, ERG ≤ 2.2 s and SIM ≤ 4.3 s, is **not** what `L` bounds; it is checked against `T_unknown(c)` below.) Reproduce: `hil_fec analyse` |
| `N_follow` agreeing replies to verify a component | 3 | Design: a single or duplicated reply never decides |
| `T_verify` minimum span of those replies | 2 s | Design: a burst of delayed replies arriving together never verifies |
| `T_mismatch` minimum span of a disagreement episode before `not_following` | 2 s | The previous 2 s rule (8 ticks) |
| `T_unknown(c)` silence: required time without a reply about component c | `4·P_c + 1 s` = 5 s (ERG, resistance), 9 s (SIM components) | **Per component (HIL #44 Part A).** A reply gap is `(m + 1)·P_c` after `m` consecutive lost replies; c is `silent` (C7) once the gap reaches `T_unknown(c)`, so `m` = ⌈`T_unknown(c)` / `P_c`⌉ − 1 losses in a row make a healthy component silent and then `not_following` (A3). The NEO never answered 27 of 344 polls (p = 7.8 %, `latency.analysis.json`; check 2: 0 of 12, check 3: 3 of 32 = 9.4 %, spacing: 5 of 79 = 6.3 % in a row and 4 of 40 = 10 % just after a control write). With independent losses a component slot of period `P_c` starts such a run at rate `p^m` per slot, i.e. `3600 / P_c · p^m` per hour. **Flat 5 s:** ERG `m` = 4, 0.14 per hour; one SIM component `m` = 2 (gap 6 s), `p²·1800` = 11 per hour, and a SIM ride has two (track, wind), so 22 per hour: unacceptable. **4·P_c + 1 s:** ERG `m` = 4 (gap 5 s), 0.14 per hour; one SIM component `m` = 4 (gap 10 s; three losses give 8 s and are tolerated), 0.068 per hour, 0.14 per hour per ride. At the highest rate measured after a control write (4 of 40 = 10 %) the new rule gives 0.36 per hour (ERG, or SIM per ride), at the in-a-row rate (5 of 79 = 6.3 %) 0.06. Measured, no component ever exceeded 4.05 s (SIM) or 2.04 s (ERG), and the largest first echo of a changed value was 4.21 s (SIM track) and 2.19 s (ERG). **Independence is assumed, not shown:** the 27 losses of the latency capture were all isolated (no two requests in a row), where independence expects 2.1 adjacent pairs (probability of none 0.12); the spacing capture (`spacing.analysis.json`, 9 of 120 lost) has one adjacent pair (observed 1, expected 0.67); and in the SIM phase no two replies of the same component were lost in a row (135 requests, 0.82 expected, probability of none 0.44). The data neither support nor contradict a burst-free loss process, so Part B counts `silent` episodes on a healthy NEO over a long SIM ride; the rule changes only if they exceed the rate above |
| `T_decide(c)` decision deadline (derived, not tuned) | `T_verify + T_mismatch + 3·P_c` = 7 s (ERG, resistance), 10 s (SIM components) | Checked by the model's fair walk |

A change to `L`, `T_unknown(c)` or the reply periods is an ADR amendment backed by #44 numbers. The model's
constants must be changed in the same commit; the model fails when its constants differ from this table
(`check_constants`).

**Reply routing.** A reply (`TrainerEvent::Observed`) arrives at `t` = `at`. It is routed by the first
matching rule:

| # | Reply | Effect |
|---|---|---|
| R1 | Any of: page-25 flags; `control = None` or `NotControl` (e.g. page 55 or an unsupported request was the last page received; it says nothing about any component), **whatever the status** (an unsupported request gives `NotControl` with status `NotSupported`, #44 Part A); status `Pending` or `Other` | Neutral: no effect on anything |
| R2 | About a required component c: status `Pass` and the echo equals c's current desired value, or a value c had at some instant in `[t − L, t]` | **Agree** for c |
| R3 | About a required component c: anything else (other value; status `Fail`, `NotSupported` or `Rejected`, even with an equal echo) | **Disagree** for c |
| R4 | About a component that is not required but was required at some instant in `[t − L, t]` | Neutral: the trainer still reports the previous mode |
| R5 | About a component that was not required at any instant in `[t − L, t]` | **Disagree** for every required component. The trainer reports a control page crank is not sending |

`TrainerEvent::Delivered` and `Fault(Gatt)` are not inputs. Delivery says nothing about what the trainer
does (§4.4).

**Component machine.** Each component c has:

- a state ∈ {`unverified`, `following`, `not_following`, `silent`};
- `streak`: the consecutive Agree replies since the last Disagree, as a count and the arrival time of the
  first;
- `dis_since`: the start of the open disagreement episode, or none;
- `last_evidence`: the arrival of the last Agree/Disagree, or the start, counted in required time;
- `not_required_since`: none while c is required.

Each Agree or Disagree for c is evaluated in row order:

1. **C2 is a pre-step.** If its guard holds it applies, and evaluation continues.
2. Then **the first matching row among C3–C6** for the reply's class wins.

Valid evidence for c is recorded whatever the other components are doing. The model evaluates exactly this
order: it walks `REPLY_ROWS` in sequence, and `check_table_order` fails CI if the row order of this table
and the model's ever diverge.

| # | When | Guard | Next state | Actions |
|---|---|---|---|---|
| C1 | c starts: `Activate`, or required again after more than `T_unknown(c)` not required | none | `unverified` | `streak` = empty, `dis_since` = none, `last_evidence` = now |
| C2 | **pre-step** for any Agree/Disagree: c is `silent`, or `t − last_evidence ≥ T_unknown(c)` (evidence too old even if C7 has not run, e.g. c stopped being required at that instant) | none | `unverified`, then continue with C3–C6 | `streak` = empty, `dis_since` = none |
| C3 | Agree | after adding it, `streak` has ≥ `N_follow` replies and `t − streak.first ≥ T_verify` | `following` | add to `streak`, `last_evidence` = t, `dis_since` = none |
| C4 | Agree | otherwise | unchanged | add to `streak` (first = t if it was empty), `last_evidence` = t |
| C5 | Disagree | `t − (dis_since or t) ≥ T_mismatch` | `not_following` | `streak` = empty, `last_evidence` = t, `dis_since` = `dis_since` or t |
| C6 | Disagree | otherwise | `following` → `unverified`; others unchanged | as C5 |
| C7 | tick, c required and not `silent` | `now − last_evidence ≥ T_unknown(c)` | `silent` | `streak` = empty, `dis_since` = none |
| C8 | c stops being required | none | `following` → `unverified`; others unchanged | `streak` = empty, `not_required_since` = now. `dis_since` and `not_following` are **kept** |
| C9 | c required again within `T_unknown(c)` | none | unchanged | `last_evidence` += now − `not_required_since` (silence counts required time only), `not_required_since` = none |

Properties of this table:

- Verification never carries over to a new requirement (C8).
- A disagreement episode survives short mode toggles (C8/C9).
- Only C3 ends an episode (or C1, C2 and C7 reset it). No transition can.

**Aggregate verdict.** The engine maps the required components' states to `TrainerFollowing`, a field of
`RideStatus`. The mapping is a pure function with no memory of its own:

| # | Condition (first match) | `TrainerFollowing` (serde) | UI | Compliance (§7.8) |
|---|---|---|---|---|
| A1 | machine inactive | `unknown` | no claim | counted |
| A2 | every required component `silent` | `unknown` | no claim | counted |
| A3 | any required component `not_following` or `silent` | `not_following` | "Trainer is not following the target", naming the component (e.g. "grade not confirmed") | **excluded** |
| A4 | every required component `following` | `following` | none needed (normal state) | counted |
| A5 | any required component with an open episode (`dis_since` ≠ none) | `suspect` | no claim | **excluded** |
| A6 | otherwise | `checking` | no claim | counted |

`RideStatus` carries the verdict and the per-component states (`components: Vec<(Component,
ComponentState)>`), so the UI can name the missing or disagreeing component. The UI sees them in the
`status` of a `StreamFrame` (ADR-0001 §4.3): sent on change, so within one frame, coalesced to the latest
value, and re-read by `ride_status()` after a `resync`.

A record second is excluded from compliance when any of its four ticks is `suspect` or `not_following`.
Excluding `suspect` covers the detection delay, so nothing is rewritten afterwards (no rollback, ADR-0001
§5). The page-25 target-limit flags are a separate `RideStatus` field (§7.6) and never enter this machine.

**Guarantees.** These hold for replies arriving with latency ≤ r. Each one is checked by the model, either
as an invariant over every reachable state or as a named scenario.

1. **Stuck component.** Suppose replies about c arrive every `P_c` and echo a value V that c has not
   desired in the last `L`. Then the aggregate is `not_following` no later than `L + 2·P_c + T_mismatch + r`
   after the later of activation and the last instant c desired V: 6 s + r for ERG, 8 s + r for a SIM
   component. Desired changes in between, at any rate, change nothing.
2. **Missing component.** A required component without a reply for `T_unknown(c)` of required time is
   `silent`. The aggregate is then `not_following` if any sibling is not silent, and `unknown` if all are.
   It is never `following` (A3/A4).
   - Mode toggling cannot restart this clock (C9) unless the component stays unrequired for more than
     `T_unknown(c)`. Then it is a new requirement and must verify from scratch, so it can still never be
     masked as `following`.
   - Lagging replies are not a separate class: an echo of a value desired within `L` is an Agree for its
     own component. It never refreshes another component.
3. **Decision deadline.** A required component that receives a reply at least every `P_c` reaches
   `following` or `not_following` within `T_decide(c)` of becoming undecided. That moment is its start, or
   the first unmet desired value after it was `following`. Since desired changes are not inputs, no
   transition rate can defer the deadline. The model's fair component walks measure a maximum of 6 s
   (`P_c` = 1 s) and 10 s (`P_c` = 2 s) against bounds of 7 s and 10 s.
4. **Recovery.** Suppose that, from any state, every required component c again gets agreeing replies
   exactly every `P_c`, the first arriving at `first_c`. Then:
   - Component c is `following` no later than `first_c + V_c`, where
     `V_c = P_c · max(N_follow − 1, ⌈T_verify / P_c⌉)`. That is the arrival of the `N_follow`-th reply
     once the span reaches `T_verify`: `V` = 2 s for ERG and resistance, and 4 s for each SIM component.
   - The aggregate is `following` no later than `max_c(first_c + V_c)`.
   - The bound is reached exactly when no agreeing streak precedes the run, e.g. after silence.

   In ERG this is 2 s after the first reply. In SIM the echo schedule puts the two components' first
   replies at most `T_poll` apart. The aggregate is therefore `following` no later than
   `T_poll + 4 s` = **5 s** after the first reply of either component, and exactly 5 s after silence.
   Example: wind at 10, 12, 14 s and track at 11, 13, 15 s give `following` at 15 s. Arrival jitter of
   the last reply adds to the bound directly.

   Stale or silent components do not block each other, because each records its own evidence (C2). The
   model checks this bound as an aggregate-deadline invariant in its fair walks and asserts it exactly in
   two scenarios.
5. **Single replies.**
   - One observation never produces `following`, from any state and whatever its content.
   - Agreeing observations that all arrive within less than `T_verify` never produce it either.
   - One Disagree never produces `not_following`.
6. **Leaving `not_following`.** A component leaves `not_following` only through its own verification (C3),
   silence (C7) or `Deactivate`. A new target does not end it.
7. **Silence.** A trainer that sends no reply is `checking` for `T_unknown(c)` (5 s for ERG and resistance, 9 s for SIM
   components, so 9 s in SIM where both components must be silent) after each `Activate`, then `unknown` (A2)
   until a reply arrives or the machine deactivates. A trainer that never answers page 70 (the #44 check-3
   fallback) therefore shows `checking` for the first 5 s (ERG) or 9 s (SIM) of every activation and `unknown`
   after that.
8. **Repeated rides.** Activity depends only on the link and the desired mode, so a second ride on an open
   connection activates and verifies exactly like the first.

**Desired-state classes (for settle only).** The engine still classifies each desired change once, but
only §7.8 settle uses the class. The following machine ignores it (see Desired history above).

| Change | Class |
|---|---|
| Mode kind change (ERG ↔ SIM ↔ resistance ↔ `Free`), including ERG release to `Resistance { pct: 0 }` (§7.4) and ERG (re-)engagement, which starts the soft start | `Transition` |
| Block boundary whose start target differs from the previous block's end target (§7.8 a) | `Transition` |
| Manual ± or coach change (§7.8 b) | `Transition` |
| Rider-set SIM grade or resistance from the free-ride/sim control (§7.9) | `Transition` |
| Ramp update inside a block (§7.3), soft-start update after its engagement (§7.4–7.5), HR-steering update (§7.7) | `Continuous` |
| Block boundary with an equal target, keep-alive re-send, `SetRider` (page 55) | none (the desired mode is unchanged) |

**Tests required.** These are listed on #32 (machine and engine) and #43 (driver), and form the acceptance
for those issues.

- *Model (this PR):* `python3 docs/adr/models/trainer_following.py` (about 40 s). CI runs it as the
  `following-model` flake check, which passes it this ADR file; locally it is `just model`. Because the
  components are independent by construction, it verifies the design compositionally, and every walk is
  exhaustive:
  - **Table order.** It parses the component table of this section. The check fails unless:
    - the row ids are C1–C9 in order;
    - the reply rows C2–C6 appear in the order of the model's `REPLY_ROWS`, which drives the model's
      evaluation;
    - the input classes (Agree, Disagree) match;
    - exactly C2 is marked as the pre-step.

    A literal implementation of this table is therefore the checked model.
  - **Component walk.** One component machine (C1–C9) runs to the fixpoint. At every step the requirement
    can start, stop or resume, and the reply can be none, Agree or Disagree. An independent monitor, driven
    only by the raw inputs, checks in every reachable state:
    - **(a)** two Disagree replies `T_mismatch` apart with no verification between them give
      `not_following`;
    - **(b)** `following` requires `N_follow` Agree replies spanning `T_verify` since the last
      disagreement, silence or re-requirement, so a single stale reply or a burst never verifies;
    - **(c)** that evidence while the component is required and not silent gives `following`, so
      recovery always completes;
    - **(d)** `silent` holds exactly after `T_unknown(c)` of required time without a reply, and requirement
      toggles cannot restart that clock;
    - a single Disagree never gives `not_following`;
    - `not_following` ends only through verification, silence or a fresh start.
  - **Fair component walks**, with replies every `P_c` = 1 s and 2 s, check the decision deadline. The
    maximum measured is 6 s against a bound of 7 s, and 10 s against 10 s.
  - **Aggregate check.** A1–A6 hold over every combination of component states. `following` is reached
    only when every component is; a silent component next to a talking one gives `not_following`; all
    silent gives `unknown`.
  - **Routing check.** R2–R5 hold over every pair of modes, every history age and every reply: a reply
    changes only its own component (R2/R3), every required component (R5) or none (R4), and it agrees
    exactly when its value is current or was desired within `L`.
  - **Whole-machine walks** compose the components. In every state they check activity **(e)**, the
    end-to-end versions of (a)–(d), and the deadline:
    - "erg" runs to the fixpoint, with link loss and `Ready`, ride start and stop, ERG changes, failing
      replies, foreign pages and silence;
    - "erg" fair also runs to the fixpoint;
    - "sim" fair runs to the fixpoint, with grade changes, ERG ↔ SIM toggles and per-component replies;
    - both fair walks also check the **aggregate recovery deadline** (guarantee 4): in an unbroken run of
      agreeing replies with an unchanged set of required components, the aggregate is `following` once
      every component's first reply of the run is `V_c` old. They also check that the SIM components join
      such a run at most `T_poll` apart;
    - "sim" free covers every input sequence of length ≤ 6. The free SIM space has about 1 M states at
      the fixpoint, too slow for CI. The components it composes are each covered to the fixpoint above.
  - **Named regression scenarios**, one per pre-mortem counterexample:
    - #46 r2 `200→250→200` stale reply;
    - #54 r1 ramp with a stuck trainer, single stale reply, and burst;
    - #54 r2 transition storm, second ride without reconnect, and SIM with wind echoes only;
    - #54 r3 stale components recover, lagging track echoes cannot mask missing wind, and transitions do
      not block verification;
    - #54 r4 SIM recovery with phase offset (astra's case: following exactly at 15 s), and ERG recovery
      exact;
    - ERG ↔ SIM toggling cannot mask track, silent trainer, and reconnect mid-ramp.

    `r54_3_stale_components_recover` and both r4 scenarios assert the guarantee-4 bound with equality.
  - **Mutation check (done once, for this PR):** a single reply verifying, dropping the evidence-age half
    of C2, and C8 clearing the episode are each caught by the walks.
- *#32, Rust implementation of the machine:*
  1. The model's named scenarios, ported as unit tests with the same inputs and the same expected
     verdict sequence.
  2. Proptest refinement: for generated input sequences (the model's alphabets, plus sub-second jitter and
     latency 0..r), the Rust verdict sequence equals the model's. The model is invoked as a test oracle
     through a fixture file generated by the model, not at run time.
  3. Every guarantee above (1–8) as a property, including transition storms at every tick, ERG ↔ SIM
     toggles faster than `T_unknown(c)`, and recovery after silence.
  4. Compliance: exactly the seconds with a `suspect` or `not_following` tick are excluded.
  5. Classification: every desired change gets exactly one class, and the `Transition` set equals the §7.8
     settle triggers.
  6. Determinism and catch-up: the same timestamped inputs give the same outputs whether ticks run live or
     as a burst after a stall (ADR-0001 §5).
- *#32, engine scenarios with `SimTrainer`.* Its `Observed` emission gets fault modes: stuck echo, echo lag
  d, injected stale replies, silent page 71, and a dropped SIM page 50 or 51. The scenarios:
  - a ramp with a stuck echo reaches `not_following`, and compliance excludes it;
  - with lag < `L` the ramp stays `following`;
  - a reconnect mid-ramp goes `unknown` → `checking` → `following`;
  - a second ride without reconnecting verifies;
  - ERG → free ride with page 51 dropped reaches `not_following` naming `track`.
- *#43, driver (FE-C):*
  1. **Poll cadence.** For arbitrary command sequences through the 4 Hz writer, a page-70 request for page
     71 is written every `T_poll` (± one tick; −1/+2 ticks with a pending SIM pair) while a controlled
     mode is active, never otherwise. At most one is written per tick, never on consecutive ticks; a
     pending single-page control value is deferred at most one tick.
  2. **SIM echo schedule.** In steady SIM, page 50 and page 51 are each written every 2 s, 1 s apart, and
     each is followed by the request on the next free tick. Replies from a faithful fake transport alternate
     between wind and track echoes.
  3. **Write latency.** A desired-mode change reaches its control write within 2 ticks (both SIM pages
     within 2 ticks when both changed). This is the writer's share of the `L` budget.
  4. **One reply, one observation.** Every received page 71 gives exactly one `Observed`, in arrival order,
     with `at` = arrival `MonoTime`. The driver never synthesizes, deduplicates, reorders or drops
     observations.
  5. **Echo decoding round trip.** For pages 48/49/50/51, `encode(page)` → echo bytes 4–7 →
     `ObservedControl` equals the `grid_key` projection of the mode that produced the page. Page 55 and
     unknown pages give `NotControl(page)`. All status byte values decode, and unknown ones become `Other`.
  6. **Replay.** The #44 check-3 capture, replayed through `ReplayTrainer` into the machine, gives
     `following` for its steady ERG and SIM segments. Every stale echo in it (an echo of a superseded value) is at
     most `L` old, measured from the first write of the superseding value plus the writer delay, and the first
     echo of each changed value arrives within `T_unknown(c)`; each SIM component gets a reply within
     `T_unknown(c)`.

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
  first, then a scan matched by name + serial. **#44 Part A:** btleplug resolves only peripherals the central has
  already seen in the current process (a fresh process with the stored id gave `NotFound`; after a scan the
  same id resolved), so the id-first path works only after a scan and the scan is the normal path. The id
  was identical across separate processes. No BLE bonding: connect, subscribe and control writes worked
  with no pairing prompt (confirmed on the NEO; HR strap still to confirm).
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
  observations before it are ignored; (3) the ride resumes when cadence > 0 (ADR-0001 §5). Until the first pedalling sample the re-issued mode is `Erg { target_w: 0 }`, then the soft start of §7.5 runs from the current 3 s power (#231). The driver never re-sends
  a control mode on its own, so there is exactly one owner of control state.
- **Implementation (#147):** `crank_ble::Reconnecting` / `ReconnectingHr` wrap any driver and keep the
  consumer's channels across losses. The first connect is not retried (its error reaches the UI). The
  link may be held while the app is open (§5), but the backoff loop is enabled only while a ride is
  active: the supervisors take an explicit `RideGate` input (wiring from engine/core: #208). Outside a
  ride a loss is surfaced (`Link(Lost)`, stream ends) and normal rediscovery reconnects when the device
  is seen again; a gate that goes inactive during backoff stops the loop. While active, each attempt
  waits `backoff(n)` (0.5, 1, 2, 4 s, then 5 s) on the injected clock, emits
  `TrainerEvent::Reconnecting { attempt }`, and calls the inner `connect` with the original rider
  config. The loop also ends on `Disconnect` or when the link is dropped. Commands sent while
  disconnected are discarded, never replayed. A unit that merely shares the advertised name is rejected
  by `Pairing::verify` after connecting (serial and model, §5). A `device` row is one (peripheral, role)
  pairing, unique per `(platform_id, kind)`: one peripheral may be both trainer and strap.
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
   from compliance. "Cadence ≥ 60 rpm for 2 s" is judged on the held cadence (ADR-0001 §5: a trainer value is
   usable for its tick and the next 4), so four real samples and four silent ticks satisfy it (architect ruling,
   #250 round 2). Thresholds are initial values checked in HIL check 9.
5. **Soft start** is also used when ERG is (re-)engaged after a reconnect or after leaving `free`/`sim`.
   **Ride start and user pause (amendment, #250 / #264, 2026-10-14).** A ride start is treated as a
   reconnect: on the NEO, ERG at the target torque with a stopped flywheel is the low-cadence hazard of §7.4
   (the rider pushes off against the full target at near-zero cadence). So the engine holds `Erg { target_w: 0 }`
   from the first tick until the first sample with cadence > 0, then runs this soft start from the 3 s power.
   A first block that is `free` or `sim` sends no ERG and is unchanged; the soft start is then owed at the
   next ERG block (the paragraph below). A block change inside a running soft start continues the ramp at its progress towards the new target (DESIGN §17.3). Resuming from a **user pause** is the same engagement: while paused the
   engine commands `Resistance { pct: 0 }` (DESIGN §5.2 "Paused (user)": "the trainer is at 0 % resistance"),
   and the resume owes the soft start with the 0 W hold until cadence. A release in force at the pause
   stays in force after the resume until §7.4 leaves it (60 rpm for 2 s). Stopped pedalling (auto-pause, ADR-0001 §5)
   needs nothing of its own: cadence 0 for 3 s is past the 2 s release of §7.4, which already commands
   `Resistance { pct: 0 }`.
   **Origin and 0 W (amendment, #231).** The origin is the 3 s power (12 ticks, pruned by tick time, so
   nothing older than 3 s counts) at the tick the first trainer sample with cadence > 0 of the new link
   epoch arrives. After `Link(Ready)`, or after a loss detected by silence, until that sample the engine
   re-issues the mode as `Erg { target_w: 0 }`, never the block target: no torque is applied to a rider
   who is not pedalling (§7.4). Once samples flow the ramp runs from that origin to the target in five
   1 Hz steps; it holds its last step while samples are missing and restarts from the then-current 3 s
   power when they return. Pre-loss power is never an origin: all soft-start state (3 s window, flow start,
   origin) is reset on every **applied** link transition and on every 8-tick silence, also one that
   starts while already paused. Every applied transition away from `Ready` (`Lost` or `Connecting`) makes
   the next engagement a soft start, so `Connecting` → `Ready` without a `Lost` is covered. If the new epoch has cadence > 0 but no power value, the
   origin is 0 W (the ramp never jumps to the target).
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
   start's 5 s plus 5 s (**amended, #250 / #264, 2026-10-14:** a ride start and a resume from a user pause are
   (re-)engagements, §7.5; the first block has no previous block for (a), but its start is an engagement under (c):
   the 0 W hold, then 10 s from the first pedalling sample are settle). These are the `Transition` class of §4.4.1, which only settle uses (the
   trainer-following machine ignores the class). Within `ramp` blocks and HR-steered steps the target moves continuously
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
   **Ride end (amendment, #266, 2026-10-14; end mode revised 2026-10-14, round 2).** The same reasoning
   applies when the ride ends: after a natural finish, `Stop` or the loss of every handle, the engine sends
   `Resistance { pct: 0 }`, not `Free`: the same single-page mode as the user pause (DESIGN §5.2) and the §7.4
   release, an unambiguous non-torque state that one page-71 echo verifies, independent of HIL check 12.
   (`Free` is the two SIM pages 50 and 51; the trainer is not out of ERG before both are written.) It does so
   before it clears the ride-active signal (`RideGate`). **What the gate waits for:** the signal stays up until
   the driver reports `Delivered { Resistance { 0 }, seq }` for the end mode's own sequence (the mode's only
   page written), the link leaves `Ready` (seen while no ride is running), or 2 s (eight writer ticks) pass,
   whichever is first; a `Delivered` of any other command does not release it, not even one of an earlier,
   equal `Resistance { 0 }` (a pause's, #345): correlation is by sequence, never by value.
   `Delivered { mode }` means every control page of `mode` has been written, for every multi-page mode.
   The end mode is always sent, also when the last command is already `Resistance { pct: 0 }`: that command
   (a pause, a release) may still be queued and unwritten. **A `Load` does not cancel it (amendment, #279,
   2026-10-14):** a previous ride's end mode stays owed across `Load`, `Start` and the unload of a loaded ride;
   the ride-active signal is cleared by that delivery (or by link loss while no ride is running, or by the
   2 s deadline), not by the unload, and the engine awaits it on shutdown. While it is owed, a newly started
   ride sends nothing, so its own engagement rules (§7.5: `Erg { 0 }` until cadence, then the soft start)
   begin only after page 48 has been written and no torque can overtake or coalesce away the end mode.
   Silence at the end would leave the last `Erg { target_w }` applied at 0 rpm.

   **Link loss while a new ride runs (active-ride exception, #288).** A link event that arrives while the
   new ride is running is queued in tick order like any other trainer event and does not settle the owed
   end mode; only the delivery or the 2 s deadline does. That is the safe direction: the signal stays up
   for at most 2 s longer. The behaviour is kept and the text aligned to it.

   **Residual (FM-FIN-02).** If a GATT write stalls for more than 2 s, the deadline releases the
   hold-back and the new ride's first ERG page can replace the still-queued page 48, so 48 is never
   written. The deadline exception allows this: a trainer that stalls this long is not taking writes
   reliably.

   **App quit is a ride end (amendment, #34, 2026-10-28).** The desktop process does not exit on the
   first quit request (Cmd+Q, last window closed): it holds the exit back, calls `Core::shutdown`, which ends
   the loaded ride like `ride_end` (so the engine sends the end mode above) and waits until the devices are
   released, at most 3 s (above the 2 s deadline), then exits. It cannot rely on dropping `Core`: on macOS the
   event loop ends the process without running destructors, so the "loss of every handle" path never runs and
   ERG could stay applied (S1). A trainer that never confirms costs the cap, not a hang.
   **"Released" means disconnected, also for a cancelled connect (#304).** A reconnect supervisor that is
   shut down mid-connect (service discovery, subscribe) disconnects the half-open peripheral, bounded
   (2 s), before it drops its event stream; the driver arms that cleanup before the transport connects,
   and every driver disconnect is bounded, so a hung platform disconnect neither hangs the release nor
   keeps the driver's admission held.
   The bound has a cost, and it bounds the **wait**, not the uncertainty: after a disconnect that timed out
   (`DISCONNECT_TIMEOUT`, 2 s; plus 1 s for the supervisor's wait on a cancelled connect) "released" is
   signalled while the peripheral **may still be connected**, and nothing retries, so it can stay connected
   indefinitely. The visible consequence: the NEO (or the strap) may still show as connected, and another
   central (a phone app) may find it busy, until the peripheral drops the link itself or the process exits
   (process exit releases the radio, which is why the quit path does not wait). Whether our own next
   connect to it works is unverified until hardware (HIL #44). We accept this: a platform disconnect that
   hangs is not something the app can fix, and the alternative is a ride end or a pairing that hangs with
   it. A best-effort re-disconnect at the next connect and at quit is tracked in #335, not done here.
   This holds on every path that drops a link, a ride end the engine cannot carry out (#331) included: the
   end mode that path sends itself comes before the link is dropped, and the disconnect after it is bounded
   in the same way.
   After the 2 s our own admission lock is free (no `AlreadyConnected` from us); the outcome of a subsequent
   connect is up to btleplug/CoreBluetooth, and a failure surfaces as `ConnectFailed`.
   **A ride end the engine cannot carry out (#331).** `ride_end` bounds its `Stop` (1 s, below the 3 s
   quit cap; the replacing `ride_load` and `discard` too). **Threading (#344):** the application runs each
   ride's engine on its own OS thread with its own current-thread tokio runtime (`EngineHost::Thread`,
   `crates/crank-core/src/core.rs`), so
   an engine blocked in synchronous code (a store write) holds only that thread: the core's runtime, its
   workers and its timer, on which every deadline below runs, stay free. (On the core's runtime a blocked
   engine held a worker and with it the timer: the deadlines did not fire, End took as long as the stall.)
   The in-runtime host (`EngineHost::Task`) is for tests on paused time only. Recorder writes stay on the
   engine's thread; the core's own store call below runs on the blocking pool. An engine that does not
   answer, or whose task is gone, is ended by force, in this order. (1) The engine is **cut off**: the
   relays between the drivers and the engine stop passing events to it, and its ride-active signal is
   ignored from then on, and the trainer's **command gate** drops its commands (below). (2) **The end mode is
   the core's**: it has its own sender through that gate, independent of the engine, and sends `Resistance{0}`
   itself, with a sequence of its own, waiting for the `Delivered` of that sequence for at
   most 500 ms (about two writer ticks; with the 1 s `Stop` bound, the abort and the record close it keeps a
   forced end at about 2.25 s worst case, inside the quit cap), before the link is dropped; a driver that
   returns on a closed control channel writes no end mode itself. (3) **The core releases the devices**
   through what it owns, not by dropping the engine's links (which an engine wedged on its thread still
   holds): both relays stop and close the drivers' event streams (a closed stream makes the trainer's
   driver return and the strap's disconnect), and the trainer is sent `Disconnect` on the core's control
   sender. (4) The engine is aborted (a task is aborted; a thread is signalled and drops the engine at its
   next await) and given 250 ms to end; an engine blocked in synchronous code cannot be interrupted and is
   detached, never awaited. (5) The ride's record is closed as **aborted** (the seconds persisted so far
   are kept, there is no summary, as for a ride closed by crash recovery), on the blocking pool and waited
   for at most 500 ms; if the engine finished the ride after all (a late `Stop`), that record wins and the
   end is reported as an ordinary one. (6) The ride-active signal is cleared, **if it is still this ride's**:
   the signal (the command gate and `active`) is one per core and belongs to the engine built last, which takes
   it over before its `Load` raises it (the check and the clear hold the same lock); a forced end that runs late (an End and a replacing
   load both force the same stuck ride, and the load's finishes first and builds the newer ride) clears neither
   the newer ride's signal nor its last-end record (`Core::clear_signal`, `signal_of`, #370). A load that
   builds no session (its `Load` unanswered or refused, or the load cancelled meanwhile) cuts its engine off and
   gives the claim back, clearing the signal (`SignalClaim`). So no load, failed or not, leaves it up. The
   engine sets the signal only while it is not cut off **and** its ride owns the signal (`ride_activity`, under
   both locks), so an engine that was reaped without a forced end (it dropped the signal itself) and is still
   ending, or detached, cannot drop or raise a newer ride's signal (#392). `settle`'s forced reap (an engine that
   answered `Stop` but never dropped the signal within `REAP_TIMEOUT`) clears the signal the same way, and cuts
   the engine off before it reaps, as a forced end does, so an engine it detaches cannot raise the signal after the
   clear (#392). One way remains for it to stay up with no ride loaded: a forced end or `settle`'s forced reap
   whose future is dropped between its `reap` and its clear. In the app only `shutdown` drops it (its cap is a
   `timeout` on the end), and the process exits right after; the Tauri commands run to completion. The next
   load's claim takes the signal over from it. The drivers' release is
   awaited (`RELEASE_TIMEOUT`, 3 s, `core.rs`), and a new ride can be loaded at once. A detached engine that wakes later changes
   nothing: its record writes are refused by the store (`ride is not live`), so the ride stays aborted, and
   its ride-active signal no longer reaches the core, so the next ride's is not touched. **The command gate
   (#345):** the engine's control sender does not reach the driver directly but through a relay the core
   owns; from the cut-off on, every engine command (one sent after it, or one queued before it and not yet
   forwarded, or one waiting on a full driver channel) is dropped and logged with the ride id, never
   forwarded (`Takeover` and the command relay `gate_commands`, `core.rs`), while the core's own sender is never blocked. One task forwards both senders and reads the
   cut-off after taking each engine command, and the core raises the cut-off before it sends: nothing of the
   engine's can follow the core's end mode. A stale `Delivered` of an earlier `Resistance{0}` does not confirm
   the core's (sequence correlation, above). The residual is FM-FIN-02's stalled
   write: if the trainer does not take the end mode within the 500 ms the core gives up on it and the link
   is dropped. `ride_end` reports `{rideId, forced}` (`RideEnded`, `crates/crank-core/src/dto.rs`; `forced` is true
   when the core closed the record without the engine's `Stop` confirming; if a late `Stop` finishes the record
   first, the core has taken over but reports `forced: false`, `core.rs` `end_ride` / `force_end`; a repeat End
   after a forced end answers `forced: true` again from `last_forced`); the id is returned either way. **The UI keys
   the forced-end note on `forced`, never on the recorded outcome (#346):** an `aborted` record is also what crash
   recovery writes, and a late `Stop` that finishes the ride after all is reported `forced: false`. The fact is on
   the answer only, not in the stored `RideRecord`, so the ride screen hands it to the summary for the window
   (`app/src/ride/forcedEnds.ts`; a reload loses it, the record does not carry it). Quitting inherits the bound. **No request waits on the
   engine under the state lock (#351):** End takes that lock, so a request holding it while a wedged engine
   does not answer would hold End. The three `Stop`s above (`end_ride`, the replacing `ride_load`, `discard`) are the only engine requests made under it, by
   design (no `ride_start` may reach the engine first), and are bounded by `STOP_TIMEOUT`. Every other
   request (`Load`, start/pause/resume/skip, and the stream requests: open, subscribe, unsubscribe, ack,
   snapshot, a window's close) is sent without the lock and waited for at most 1 s (`REQUEST_TIMEOUT`, the bounded wait in `core.rs`); the
   caller then gets the typed `engine_unresponsive` (outcome unknown, the request may still apply). The
   ride view says the engine is not answering and offers End; nothing is retried. A `Load` the new engine
   does not take releases the devices it was built with, as a forced end does. FMEA row FM-FIN-05.

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
    and 51, which gives the SIM reply periods behind `T_unknown(c)`. These calibrate the constants of §4.4.1.
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
