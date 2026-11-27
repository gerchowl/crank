# ADR-0002: Bluetooth stack and Trainer abstraction (FE-C first)

- **Status:** Proposed, Amended (#54, #250, #264, #266, #435, #588, #589, #585)
- **Date:** 2026-10-13
- **Issue:** #27 (decides the design questions of #23); amendment #54 (trainer-following state machine, §4.4.1); amendment #435 (connection lifecycle, §5.1); amendment #585 (eased ERG target, §7.10); amendment #610 (phase-locked page-71 request, §4.4, proposed)
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
on the shared host [mac-studio] are denied Bluetooth inside flock panes, so hardware runs go through an Alacritty
window and need the owner for anything involving pedalling.

## 2. Decision: BLE library and where the stack runs

**btleplug** (0.13.5, released 2026-10-10 on crates.io) in the Rust core, crate `crank-ble`. The UI
never talks to Bluetooth; it sees device state through the window's snapshot channel (`UiFrame.devices`, ADR-0001
§4.3, #588) and device commands (ADR-0001 §4). The CLI uses the same crate, so `crank ride` works with a real trainer.

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

**Amendment (#610): phase-locked page-71 request. Status: proposed, pending the `hil_fec phase`
sweep.** Production still runs the free 1 s `T_poll` grid above. PR 2 of #610 implements this once
the owner's sweep has fixed `D`.

*Evidence (NEO 55624, firmware 0.7.4).* The NEO stops broadcasting page 16 (speed, distance) for
22–25 s at a time when the page-71 request keeps one relation to the page-16 slot. In
`docs/hil/captures/latency.fecap` (#44 Part A, crank's own writer, 1 Hz polls, 348 s):

- **Matched rates.** The NEO's broadcast frames sit on a 250.015 ms grid, four frames per
  1000.06 ms. That equals the poll period (1000.08 ms, 344 polls). So the request keeps its place
  among the trainer's frames (always about 0.8–0.88 of a frame slot); what moves is page 16's slot.
- **Plateaus.** Page 16's slot moves relative to the request in steps of one frame slot. Between
  steps the request sits 12–24 s on one offset after page 16 (plateaus near 470, 980, 720, 500 and
  200 ms).
- **Outages.** After 21–24 s on the plateau near 200 ms (one slot after page 16), page 16 stopped
  entirely: 25.23 s from 108.6 s (ERG) and 22.23 s from 240.6 s (SIM). The NEO kept sending at its
  normal rate and put page 25 in the slots page 16 was due in. Page 16 came back at a new slot.
  Inside the outages the request's offset, extrapolated from the last page-16 slot, ran from
  192–193 ms to 393–424 ms.
- **Thinning.** Near 940 ms (the request just before page 16), page 16 thins out to about one per
  2 s: the reply takes its slot.
- **The owner's ride (#610, 2026-11-08).** Speed was blank for 21–22 s every 131–133 s; here the two
  outages start 132.0 s apart. The `speed` hold of 6 s (ADR-0001 §5, #437) is not meant to bridge an
  outage; widening it would hide one, not follow a slower page.
- **Without polls** (`idle.fecap`), page 16 is every fifth frame (gaps of 1.23–1.26 s) and there are
  no outages.

Binned by offset (requests followed by page 16 within 1.5 s; `hil_fec/phase.rs`, whose tests assert
these values on the capture):

- 250–400 ms: 0–18 % followed. Unsafe.
- 900–950 ms: 36 %. Unsafe.
- 650–750 ms: ≥ 90 %. Safe.
- The other offsets are mixed or were never visited.

*Decision (proposed).*

- **Lock.** While page 16 arrives, the page-71 request is due at `last16 + D + n·T_poll`: the first
  such instant (n ≥ 0) at least `T_poll`/2 after the previous request. `last16` is the arrival of
  the last page 16; `T_poll` = 1 s. This is `hil_fec`'s `PhaseLock`, so the sweep measures this
  request timing. It writes nothing else: no SIM keep-alive or control page is interleaved, so the
  sweep does not test the SIM alignment below. PR 2's acceptance ride does.
- **`D` is TBD by the sweep.** It will be the centre of the widest safe band, which must be
  ≥ 150 ms wide: 75 ms to either edge against the ~30 ms BLE connection-interval quantisation.
- **Provisional band from the capture:** 650–750 ms safe. Avoid 150–450 ms and 900–1000 ms.
- **Absent page 16.** The same formula with the last page 16 seen gives one request per `T_poll`,
  still `D` after the last known slot. A missing page 16 therefore never moves the request into the
  band relative to where page 16 was. Before a connection's first page 16 (the connect sequence),
  the free 1 s grid applies; page 16 arrives within about 1.25 s.
- **Writing the request.** It is written at its due instant, not on the next 4 Hz writer tick. A
  tick-quantised poll would need a safe band ≥ 250 ms plus jitter. One write stays in flight at a
  time: a poll due while a control write is in flight goes out when that write completes. The
  reserved-poll-tick amendment above becomes "a poll is never more than one write late against its
  due time". Control pages keep the 4 Hz tick grid and lose no tick to a poll.
- **SIM alignment.** Today each SIM keep-alive page (50, 51; alternating, each every 2 s) is followed
  by the poll on the next free tick. Under the lock the poll's time is set by page 16, so the
  alternating keep-alive page moves instead: it is written on the last writer tick before the poll's
  due time. Each SIM component still gets a reply every 2 s (`P_c` unchanged), and ERG and
  resistance every 1 s. A changed SIM page is still written at once, as before.
- **Reply timeouts (§4.4.1).** `T_poll` stays 1 s on average, so `P_c`, `T_unknown(c)` = `4·P_c` +
  1 s and the per-hour silent rates keep their form. A step of page 16's slot stretches one reply
  interval, by up to `T_poll`/2 plus one slot under the half-period rule. PR 2 measures the
  reply-interval distribution under the lock from the sweep capture and re-derives the silent rates
  of the `T_unknown` row from it before this amendment is accepted. If they exceed the rates stated
  there, `T_unknown` changes in the same PR.
- **Acceptance.** A `hil_fec phase` PASS (runbook `docs/hil/phase.md`) sets `D`; only a complete
  sweep has a verdict (a link drop or a partial capture is INCOMPLETE). Then a ride captured with
  `CRANK_CAPTURE_DIR` under PR 2, in ERG and in SIM (which verifies the SIM alignment with control
  and keep-alive writes interleaved), must show no page-16 gap over 2.5 s: `hil_fec analyse` gives
  `page16_phase.page16_gaps_over_2_5_s` empty (`max_page16_gap_ms` ≤ 2500). `outages` lists only
  gaps of 6 s or more and does not decide acceptance.
- **Firmware.** This is NEO fw 0.7.4 behaviour. Another firmware or trainer may rotate its pages
  differently; the lock only assumes that the trainer broadcasts page 16 periodically.

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
`status` of every snapshot frame (`UiFrame.ride.status`, ADR-0001 §4.3, #588), coalesced to the latest value at
the frame's send. Until #588 step 3 it is the `status` of a `StreamFrame`, re-read by `ride_status()` after a
`resync`.

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
| Ramp update inside a block (§7.3, per tick since #585), soft-start update after its engagement (§7.4–7.5), HR-steering update (§7.7), every eased update of the commanded target (§7.10, #585) | `Continuous` |
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
  (Zwift, Tacx app, a second crank process) is connected, crank cannot. When crank connects, holds and
  releases the devices is §5.1 (amendment, #435); the devices view offers "Release trainer". If a remembered trainer is not seen during a scan, the UI says so with the likely
  causes (another app connected, trainer asleep or unpowered) while the scan continues. Within crank,
  the ride lock (ADR-0001 §2) prevents two crank processes from competing.

### 5.1 Connection lifecycle and speculative connect (amendment, #435, 2026-11-02)

**Context.** `ride_load` connects the paired devices before it builds the engine (`core.rs` `Core::connect_links`). It
connects the trainer, then the strap, one after the other, each bounded by `CONNECT_TIMEOUT` (15 s). Each connect
resolves its peripheral first (`central.rs` `resolve`): a lookup of the adapter's list, then a scan of up to
`RESOLVE_SCAN` (4 s) when the adapter does not list it, which after launch it never does (#206-3: btleplug resolves only
peripherals this process has already seen). Then GATT connect, discovery, subscribe, the DIS read, page 55, a fixed
250 ms wait and the page-54 reply, awaited for up to 2 s (`driver.rs` `FecTrainer`). From code reading, not yet
measured: about 4–8 s typical and about 30 s worst case, while the UI shows "Loading ride…". The owner's direction on
#435 is speculative connect: connect while a ride is likely, as a photo uploads while the caption is still being
written, and let go when it is not. The constraints are those of §5: one central at a time on the NEO (other apps, HIL
check 10's second central), and the strap's coin cell. This section replaces the "connects when the rider enters
pre-ride or ride, stays connected while the app is open" of "Single central" above.

#### 5.1.1 States and who owns a link

Each paired device (trainer, strap) has one **link phase**, owned by a new core component, the **link keeper**
(`crank-core`, beside the session; one per `Core`):

| Phase | Meaning |
|---|---|
| `cold` | not connected, no attempt running |
| `searching` | resolving the peripheral (adapter lookup, then the shared scan) |
| `connecting` | GATT connect, discovery, subscribe, DIS read and identity check (`Pairing::verify`) |
| `handshake` | trainer only: page 55 written, page-54 reply awaited |
| `warm` | `Link(Ready)` reached; the keeper holds the link and drains its events |
| `not_found` | the last attempt failed (not seen in the scan, connect failed, identity mismatch, timeout); kept until the next attempt |
| `riding` | the link was handed to a ride's session (§5.1.4); the keeper no longer owns it |

Ownership is one way: **keeper → session**, at `ride_load`. A session never hands a link back. The ride's end
releases its links exactly as today (end mode, release, disconnect: §7.9, `LinkRelease`), and the phase becomes
`cold`. If a warm view is shown again, the keeper warms up again from `cold`.

**The ride's hold makes the keeper active again only after the ride's links are handed back** (#573). The session
keeps the keeper's hold until its drivers have closed their event streams (the peripherals are handed back; at most
`RELEASE_TIMEOUT`, as for pairing); only then is the hold given back (`Released`). This holds for every way a session
ends: an End, a discard, a forced end, a dead engine, and a load that fails, is cancelled or panics after the links were
handed to it. Giving the hold back at the reap instead let a load racing a "Release trainer" discard start an attempt
while the ride's link was still connected, which the driver refuses ("driver already connected") and which left the
trainer `not_found`. Whoever waits for a ride's release (a replacing load, `ride_end`, the keeper's discard) therefore
acts on a keeper that has the `Released` report before anything it sends next, and the keeper applies its own reports
queued before a core message ahead of that message. All of §6 and §7 stays attached to the
session and is not touched: reconnect backoff, `RideGate`, end mode, forced end, command gate.

**The keeper's ride states (implementation, #469).** Beside the per-device phases, the keeper is in one of three ride
states, and every input has a defined transition in each (the state machine's matches have no catch-all arm, and a
test enumerates every state × input):

| Ride state | Meaning | Leaves it |
|---|---|---|
| `Free` | no ride: the keeper is active (warm-up, cool-down, scan) | the handover → `Handed` |
| `Handed` | the links are handed to a load that has not installed its ride yet (its engine's `Load` is under way, bounded by 1 s) | `Loaded` (the ride is installed) → `Loaded`; `Refused` (the load was cancelled before it took the links: they come back `warm`) → `Free`; `Released` (the load failed after taking them: they are gone, `cold`) → `Free` |
| `Loaded` | a ride holds the links, started or not | `Released` (the ride's end or discard, once its links are handed back) → `Free`, every link `cold` |

In `Handed` an intent is **recorded**, never dropped, and applied the moment the load resolves: on `Loaded` as in
`Loaded` (`Sleep` discards the unstarted ride; `RiderRelease` unloads it, releases and latches), on `Refused` or
`Released` as in `Free` (a release, the latch, or the intent and its timer). Which recorded intent stands (architect
decision on #520, 2026-11-06; this is the one statement of the rule):

- the latest intent that is no release wins;
- of two releases the stronger is kept (`RiderRelease`, then `Sleep`, then `Devices`, then `Hidden`), whatever their
  order;
- a later intent that is not a rider action (visibility or focus: `Likely`, `Left`, `Background`) never overrides a
  recorded release;
- a later **rider action**, "Ride this" (a load) or Retry, **supersedes** it: the recorded release, a recorded
  `RiderRelease` and its latch included, is cancelled and answered `Conflict` ("superseded"). The rider action then
  proceeds as in `Free`. This is the latch's own rule (§5.1.3: it lasts until the rider asks for a ride).

What is order-independent, and what is not: a rider action supersedes a **recorded** release; it cannot undo an
**applied** one. So a load gets its ride whether it reaches the keeper before or after the links come back (either the
release is applied first and the load then clears the latch and connects again, or the load supersedes it). But a
release that resolution applied stays applied. Written out for a `Sleep` around the load's resolution:

- `Sleep`, then Retry, then `Loaded`: the Retry superseded the recorded `Sleep` (the rider acted, so the Mac was awake),
  and the unstarted ride is kept;
- `Loaded`, then `Sleep`, then Retry: the `Sleep` was applied on `Loaded` and discarded the unstarted ride; the later
  Retry does not bring it back.

Both follow the rule; neither is a race the keeper resolves.

A pairing change in `Handed` marks the device's handed link stale: if it comes back (`Refused`), it is released, and
the new pairing starts `cold`. A change of the rider configuration in `Handed` is applied when the trainer comes back:
a trainer link connected with the old configuration is reconnected (a ride that loaded keeps its link). A quit in
`Handed` releases links that come back. A load that arrives in `Handed` waits for the links to come back or to be
released; if the ride is installed instead, it fails.

#### 5.1.2 Warm-up: when and how

**Intent.** The UI reports the rider's intent; the core owns every timer and decision. `links_intent(intent)` is a
facade command (ADR-0001 §4) with `LinkIntent`:

- `Likely`: the window is visible and focused, and it shows a **warm view**: Today, Workouts (list or a selected
  workout) or the Ready sheet (DESIGN §5.1, §5.4, §5.7). The UI sends it when one of these appears or gains focus,
  and again on rider input while it is shown, at most once per 30 s. On the Ready sheet, while no ride is loaded
  (the load is in flight or failed), it is `Likely { ready: true }`, which holds without input (§5.1.3).
- `Left`: the window now shows another view (History, Review, Settings other than Devices, Onboarding). This starts
  the idle timer (§5.1.3).
- `Background`: crank is no longer the frontmost app while its window stays visible. This starts a 60 s grace
  timer (§5.1.3); a `Likely` within it cancels it.
- `Release(reason)`: cool down now (§5.1.3). The reasons are `Hidden`, `Sleep`, `Devices` and `RiderRelease`.
- `Retry`: the rider pressed Retry on a chip (§5.1.4): attempt every paired device that is `cold` or `not_found` at
  once, scan continuously again, and clear the `RiderRelease` latch. It is a `Likely` too, keeping a standing
  `ready`.

**While a ride is loaded or active the keeper is inactive.** It ignores every `Likely`, whatever any window shows,
and warms up no device. The session owns its links (§5.1.4). A strap that missed the load stays out of that ride
(#450). The core still counts the throttled `Likely` refresh of the Ready sheet as rider input for `T_abandon`
(§5.1.3): only the keeper ignores it.

On `Likely` the keeper starts an attempt for every paired device that is `cold` or `not_found`, **trainer and strap in
parallel** (two tasks, never one after the other). A `not_found` device is attempted only by a **fresh** `Likely`, one
that follows another intent (or none); the throttled refresh of a `Likely` that stands retries nothing, so rider input
never becomes a connect timer against an absent device (implementation, #469). Each attempt is one `connect` of the existing driver, bounded by
`CONNECT_TIMEOUT`. A device with no pairing has no phase and gets no attempt.

**Handle reuse (an assumption, to be measured).** Both attempts resolve through `resolve`, which looks the
peripheral up in the adapter's list before it scans. #206-3 showed only that an id resolves after a scan **in the same
process**. It did not test the lookup after the device was disconnected, or after the scan that saw it stopped, and
`resolve`'s own documentation (`central.rs` `resolve`) says the adapter lists a peripheral "only while it is
connected or was seen by a running scan". So the lookup may well fail after a cool-down or once a Devices scan has
ended, and the attempt then scans. The design assumes nothing beyond that: a failed lookup costs one scan of latency
(about 0.3 s typical for the NEO, at most `RESOLVE_SCAN`), never safety, because identity is checked after connecting
either way (`Pairing::verify`). The `via=lookup|scan` field of §5.1.6 measures how often the lookup works, and
sub-issue 1 (§5.1.7) verifies it on hardware. The NEO advertises about 3.5 times per second (Part A, #206-1: 104–108 advertisements in 30 s), so that scan
normally ends well inside `RESOLVE_SCAN`. The two attempts share one adapter scan (`central.rs` `RESOLVE_SCAN` notes
that the scan is shared). No platform id is "connected without a scan" from a previous process: #206-3 showed that
btleplug cannot do that, and it is not attempted.

**Retries while likely.** A `not_found` device is retried while intent is `Likely`. The trigger is the device's next
advertisement in the shared scan, not a connect timer, so no connect loops against an absent device. The scan runs
only while a paired device is `not_found` and intent is `Likely`, and it is **capped**:

- it scans continuously for the first `SCAN_CONTINUOUS` = 30 s after a device becomes `not_found`;
- then it scans in windows of `RESOLVE_SCAN` (4 s) every `SCAN_PERIOD` = 30 s, a duty cycle of about 13 %. The first
  window opens one period after the continuous scan ended (at 60 s, 90 s, … after the device became `not_found`);
- it stops when intent is no longer `Likely`, when every paired device is `warm`, or at cool-down;
- an advertisement starts at most one attempt per device per scan window (the continuous scan counting as one
  window), and the failure of an attempt an advertisement started does not restart the cap. Only a device that becomes
  `not_found` otherwise (a failed attempt of a `Likely`, a load or Retry, or a lost warm link) restarts it. A device that
  advertises but never connects therefore costs at most one connect per window, not one per advertisement (#469).

**Cost of the cap.** An unbounded scan is a real cost while a NEO is held by another app and a warm view stays open.
Active scanning keeps the Mac's Bluetooth radio receiving, and every scan response is a request from crank. On a
MacBook, the radio is shared with Wi-Fi in the 2.4 GHz band and with the other app's connection to the NEO. Its
power draw is small next to the display, but it is not zero, and it is spent for hours on a device that cannot come.
The cap keeps detection prompt where it matters: a strap put on, or a NEO the other app just let go of, is seen within
30 s plus one window, at most about 34 s. A rider who knows the device is back presses Retry, which scans
continuously again. These values are starting values, measured in Part B (§5.1.6).

A strap that sleeps without skin contact therefore warms up when
the rider puts it on. A trainer held by another app does not advertise, so it stays `not_found` (§5.1.5).

**What warm-up runs: the full connect, up to `Link(Ready)`.** This includes the DIS identity check, page 55 and the
page-54 capabilities. It is not a GATT connection alone. Pages 80/81 are not requested, today or here: identity is
the DIS serial and model (§5). The reasons:

- **Latency.** GATT alone would leave subscribe, page 55, the fixed 250 ms and up to 2 s of page 54 on the ride-start
  path, which rules out a start under 1 s.
- **Honesty.** A `warm` chip that says "Trainer: ready" must mean that this is the paired unit (serial checked) and
  that it answers FE-C (page 54 replied). A GATT connection to a same-named unit, or to a trainer that does not answer,
  is not ready.
- **Cost.** Once the GATT connection exists, holding the handshake's result costs nothing more. The NEO's single slot
  is taken either way, and a subscribed but silent link has no writes (§5.1.3). The strap's HR notifications (about
  1 Hz) feed the Ready sheet's HR row (DESIGN §5.4), which is wanted.

The rider configuration goes out as page 55 at connect, and `SetRider` on a live link is a protocol fault (§4.4). So
a change to rider or bike mass, or to wheel size, while the trainer is `warm` makes the keeper reconnect it: release
it, then warm it up again.

#### 5.1.3 Connected is not controlling; cool-down

**Nothing is written while warm.** After `Link(Ready)` the keeper sends no `TrainerCommand` and the driver writes
nothing:

- no control page (48–51);
- no page-70 request (polls run only while a controlled mode is active, §4.4);
- no page-55 re-send, and no keep-alive (there is no current control page to keep alive).

The connect handshake's page 55 and page-70 request for page 54 are the only writes of a warm period. They are
configuration and a request, not control. In particular, **page 48 at 0 % is not written**. Writing it would make
`warm` a controlled state with an owner of control, a keep-alive and an active trainer-following machine, which is
exactly what "connected ≠ controlling" excludes. It would also protect nothing:

- a fresh connection already reports page 48 at 0 % before any control page (Part A, §4.4: echo `FF FF FF 00`);
- a crank ride normally ends with `Resistance { pct: 0 }` delivered before its links go (§7.9). **The exceptions:**
  - a forced end whose end mode is not taken within its 500 ms (FM-FIN-02's stalled write);
  - a disconnect that timed out (#335), where the end mode may or may not have reached the trainer;
  - a crashed or killed process, which sends nothing.

  In each case the trainer keeps whatever it last applied, and a link is never handed back to the keeper anyway
  (§5.1.1);
- whatever another app or an exception above left behind is the trainer's state without crank, the same as with
  crank closed.

The conclusion does not rest on those endings. Nothing is written while warm, so a warm link never adds resistance or
torque to anything: it leaves the trainer exactly as it found it. Clearing a leftover state is the job of the next
ride's own engagement, which is unchanged (§7.5: `Erg { target_w: 0 }` from the first tick until cadence).

Every safety rule of §6 and §7 arms when the session takes the link, exactly as today:

- the engine's first `SetMode` (`Erg { target_w: 0 }` until cadence, then the soft start, §7.5);
- `Activate` of §4.4.1, because the session receives the link together with its current state, `Ready`, and never
  infers that state;
- the reconnect loop behind `RideGate` (§6).

The keeper reads only telemetry while warm (the latest sample per device, for the chips). It drains both event
streams: the drivers `try_send` into bounded channels, and a full channel would become a fault.

**Cool-down.** A `warm`, `searching`, `connecting` or `handshake` device is released, and becomes `cold`:

- **after `T_cool` = 5 min without a `Likely`**: after `Left`, or on a warm view with no rider input (the throttled
  refresh above) for 5 min. **The Ready sheet is exempt from the input rule**, but only for the keeper's own links:
  while the load is in flight or has failed (`Likely { ready: true }`), the links are held without input, because the
  rider is about to ride. Hiding, the background grace and sleep still apply to those keeper-owned links. Once the
  load succeeds the links are the session's, and the rules below do not apply;
- **after the 60 s grace on `Background`**: crank is no longer the frontmost app, also while its window stays
  visible. A `Likely` within 60 s cancels it, so a quick switch to another app costs no reconnect;
- **at once on `Release`**, with these reasons:
  - `Hidden`: the window is minimised or the app is hidden.
  - `Sleep`: the Mac is about to sleep. `crank-desktop` observes `NSWorkspaceWillSleepNotification`; other platforms
    are #41.
  - `Devices`: Settings → Devices is opened. Pairing needs the devices free (`device_pair` connects itself), and a
    connected NEO does not advertise, so the Devices scan would not list it.
  - `RiderRelease`: the Devices view's "Release trainer" (#314 point 2). It also **latches**: no automatic warm-up
    of that device until the rider asks for a ride ("Ride this", Retry) or relaunches the app.
    **With a ride loaded (architect decision on #435, 2026-11-05; the owner may override).** On the Ready sheet with a
    ride loaded but not started, "Release trainer" is reachable: it unloads that ride (back to choosing a workout,
    the path of Cancel, as `T_abandon` below), its links are released, and the latch is set, so there is no silent
    re-acquire until the rider asks for a ride again. During a started ride (riding or paused by the rider or by
    the engine) it is **not** reachable: the ride owns the trainer and the keeper is inactive, so `links_intent`
    refuses it with `Conflict` and changes nothing; the rider ends the ride first, and the ride's own end releases
    the trainer (§7.9). A ride whose Start was sent counts as started, also when the Start races the release past
    the core's own check: the discard then finds it started, and the release answers `Conflict`.

**What `links_intent` answers (#469).** It answers once the intent is applied and what it releases is released: the
keeper's links (each release bounded by 2 s), a cancelled attempt (once its connect is dropped), or a loaded ride it
discards (the bounds of the end path, §7.9). An intent recorded in `Handed` (§5.1.1) is answered when the load resolves
and the intent is applied; that wait is bounded by the load's own bounds (`Load` within 1 s). It is never answered `Ok`
for an intent it has not applied. It answers `Conflict` for `RiderRelease` while the ride holds the trainer, for one
whose discard found the ride started, for `Likely` or `Retry` after the quit, and for a release recorded in `Handed`
that a later rider action superseded (§5.1.1; it was never applied).

**After the quit nothing acquires a link (#469).** `Shutdown` lets every keeper link go, and from then on no transition
may connect or hand over: the keeper's single point of emission drops any attempt and turns a handover back, and a load
still waiting gets its failure. A load that waited in `Handed` and links released after the quit connect nothing.

The release is the driver's bounded disconnect (§7.9, #304). A disconnect that timed out leaves the same residual as
there (#335). An attempt that is still connecting is cancelled, and the cancelled connect disconnects the half-open
peripheral (#304).

**A loaded ride owns its links (architect decision on #451, 2026-11-03; the owner may override).** When `ride_load`
succeeds, the links are `riding` and the **session** owns them until one of three things happens: the ride is
discarded (Cancel on the Ready sheet), it ends, or the app quits. Focus, hiding, the background grace and `T_cool`
never release a loaded ride. Holding them writes nothing, the same as warm: nothing is armed before Start (the
ride's first `SetMode`). Two rules bound an **unstarted** loaded ride:

- **Abandoned load.** It is discarded, and its links released, after `T_abandon` = 10 min without Start and without
  rider input on the Ready sheet (the throttled refresh above). The core runs this timer, not the UI.
- **Sleep.** `Release(Sleep)` discards it, because a load that sleeps through is stale.

Both go through the same path as Cancel (`ride_end` on a loaded ride, §7.9). That discard checks, under the same lock as
its `Stop`, that the ride is still loaded and that no Start was sent for it (`ride_start` marks it under that lock before
it sends Start), so a discard and a Start that race never end a ride the rider started. A discard that does not go through
(the Start came first, then failed, or the ride was gone) re-arms `T_abandon` from then, and a later `Sleep` discards
again; a `RiderRelease` whose discard did not go through answers `Conflict` and does not latch. A **started** ride follows §6 and §7
unchanged: it keeps its links through focus changes, hiding and background (sleep during a ride is #51).

**Quit, `Core` drop and crash while warm.** Warm links are the keeper's, so no ride's end mode covers them, and none is
needed: nothing was written.

- **Quit.** `Core::shutdown` (the quit path, §7.9) releases the keeper's links in parallel with ending any loaded ride.
  It is an orderly, bounded driver disconnect (`DISCONNECT_TIMEOUT`, 2 s) with **no write** of any kind, inside the 3 s
  quit cap. An attempt still connecting is cancelled and its half-open peripheral released (#304). The `Exit`
  backstop runs the same shutdown.
- **`Core` drop.** Dropping the keeper drops its links, which disconnects them through the drivers' cleanup with no
  write. On macOS, where the event loop may exit without destructors, process exit releases the radio.
- **Crash.** A crashed or killed process writes nothing. The OS drops the link, and the NEO stays as it was. While
  warm, that is the state it was in before crank connected.

**Background grace and the timer values (architect decision on #451, 2026-11-03; the owner may override).** The idle
timer runs only while crank is frontmost. Going to the background releases after 60 s (above). So `T_cool` never holds
the NEO against the phone app or check 10's second central for more than that grace. A rider who wants the trainer
for another app at once uses "Release trainer" or hides crank (`Hidden`); check 10's procedure hides crank. The 60 s
grace trades the other app's wait of up to a minute against the reconnect that every quick switch would otherwise
cost. The values for `T_cool`, the grace and the scan cap are recorded as decided, and each is a constant the owner
can change. For `T_cool`, what remains is weighed below:

- **Reconnect cost.** Going cold costs one reconnect: GATT, the handshake and its fixed 250 ms, plus a scan if the
  lookup fails (the assumption above). This is estimated at 1–3 s (to be measured, §5.1.6) and shown on the Ready
  sheet (§5.1.4).
- **Strap battery.** A connection costs some of the coin cell's life. Bounded at 5 min per idle period, it is small
  next to a ride.
- **Rider behaviour.** A rider who glances at History or reads a workout for a few minutes and then rides should not
  wait.

1 min would reconnect on most such detours; 15 min would hold the strap through an abandoned session. 5 min is a
starting value: HIL #44 Part B logs the keeper's timings (§5.1.6), and the owner may change it.

#### 5.1.4 `ride_load`, failure UX and status

**`ride_load` uses the keeper.** For each paired device, `ride_load` acts on its phase:

- `warm`: the link is taken. No connect and no wait.
- `searching`, `connecting` or `handshake`: that attempt is awaited, never a second connect.
- `cold` or `not_found`: an attempt starts now, in parallel with the other device's.

The trainer is required, as today (`DeviceUnavailable` when it fails). The strap is optional: one that is not
`warm` once its attempt has ended gives a ride without HR, as today. A strap that is found later joining the running
ride is not decided here (#450).

Details of the implementation (#469):

- The links are handed over once the trainer is `warm` and the strap's attempt has ended, both in one step, with the
  link's current state (`Ready`). When the trainer fails, the load fails at once and nothing is handed over: a strap
  that linked beside it stays the keeper's, `warm`, under the rules of §5.1.3 (the Ready sheet holds it).
- A load is a `Likely` (it keeps a standing `ready`, and starts `T_cool` otherwise) and clears the latch.
- A cancelled load that never took its links gives them back to the keeper, `warm`.
- A load that comes while the links are still handed to a load that has ended (its release or their return not yet
  reported to the keeper) waits for them; it never fails on that.
- The `device{kind,id}` span of a ride (ADR-0001 §7.1) opens at the handover; a warm-up's attempt runs before any ride
  exists.
- Pairing, forgetting and the Devices scan release the keeper's links first, as `Release(Devices)` does, whether or
  not the UI sent it: a connected NEO does not advertise, and pairing connects itself. A changed pairing makes the
  device `cold` (a forgotten one has no phase).
- A connect that panics fails its attempt like any other failure.

So the worst case of a load is one `CONNECT_TIMEOUT` (the longer of two parallel attempts), no longer the sum of two.
With both devices `warm`, `ride_load` performs no Bluetooth I/O. It must return in under 1 s, the target for "Ride
this" with warm links.

**Load phases.** `LoadProgress { trainer: LinkPhase, strap: Option<LinkPhase> }` holds each device's phase during a
load (`strap` is absent when none is paired). It is updated as each phase changes, so a Ready sheet opened over a cold
load shows per-device progress (DESIGN §5.4): "Trainer: searching… / connecting… / ready", "HR: not found — keeps
looking".

- **Today** (#561): it is `ride_status().load` (`crates/crank-core/src/dto.rs:193-199`, `:428-434`).
- **From #588 step 3:** the window's snapshot channel carries `loading: bool`, and the progress is the devices' own
  phases (`UiFrame.devices`, ADR-0001 §4.3.6). `LoadProgress` stays for the `ride_load` log line (§5.1.6) and the
  keeper's own use.

**Phases outside a ride, and one source per device (amendment 2026-11-09, #588).** From #588 step 3 the chips on
Today, Workouts, the Ready sheet and Settings → Devices read each device from **one** place: the `devices` of the
window's snapshot channel (ADR-0001 §4.3). Each entry is `DeviceView { paired, phase, link, sample }`:

- `phase` is the keeper's phase (§5.1.1); a device that is not `warm` has no sample.
- A device a ride holds has `phase: riding`, and the **core overlays** the ride's link state onto it (`link`:
  `connecting`, `ready` or `lost`). It reads it from the engine status that the same frame's `ride.status` comes from,
  so a loss and the pause it brings are in the same frame. From the hand-over until the session is installed there is
  no engine status yet: `link` is then `ready`, the state the link was handed over in, and the frame says
  `loading: true`, `ride: null`. The ride's values reach the page as the snapshot's series, not as a sample.
- The rule is per device: a strap that was not `warm` at `ride_load` stays the keeper's during the ride, in its own
  phase (`not_found`), beside the trainer's `riding`.
- When the ride's hold is released (an unload, a discard, the ride's end), the devices it held come back `cold`, with
  no sample; a device it never held keeps its phase.

Until step 3 the code follows #561's rule: a riding device is read from `StreamFrame.links` and
`device_links().link`, every other device from `LinkFrame`, and the Ready sheet combines both with
`RideLoaded.hr_linked` (`app/src/ride/ReadySheet.tsx:65-80`). The second writer the old text feared is not needed:
the core computes the overlay from the two inputs it already has (the keeper's view and the session's status watch).

**`links_intent` answers nothing (#588 step 2, #597).** `links_intent(intent)` returns `Result<(), ApiError>`. It is
answered at the same moment as today, once the intent is applied and what it releases is released ("What
`links_intent` answers", §5.1.3), with the same refusals (`Conflict`, `Storage`). Until #597 it answered the `LinkFrame`
its resolving step published (`app/src-tauri/src/commands.rs:93-98`), and the page had to order those answers against
the channel's frames by `seq` (`app/src/ride/linkOrder.ts:7-10`, deleted in #597), which is how #577 arose. With no state in the answer the
**ordering rule is deleted**: the page applies and acks every frame. After an intent the UI changes on the next link
frame (within about 100 ms plus one IPC hop), not on the answer; #561's per-device split (`StreamFrame.links` for a
riding device, `LinkFrame` for the others) stays until step 3.

**The link channel until #588 step 3.** `links_open` opens one `Channel<LinkFrame>` per window (#561,
`crates/crank-core/src/keeper/channel.rs`): an ack window of 1 (`links_ack(id, seq)` for every frame received; an ack
of another `seq` is ignored, one for a channel the window does not hold is `NotFound`); each frame the complete state
when sent, so coalescing loses nothing; at most 10 Hz on the unscaled clock, the first frame at once; under 300 bytes
(`keeper/channel.rs:337-358`); `seq` the keeper state's number. Step 3 replaces it with ADR-0001 §4.3's snapshot
channel and deletes `links_open`, `links_ack`, `LinkFrame` and `device_links`.

**Failure UX.** Failures show as quiet chips, never as modal errors or toasts:

- "Trainer: connecting…", "Trainer: ready", "Trainer: not found — is another app using it?" (hedged until #314);
- the same for "HR".

Retry starts an attempt at once and clears a `RiderRelease` latch. The rider meets an error only when they ask for a
ride: a trainer that cannot be connected fails `ride_load` with `DeviceUnavailable`, and the Ready sheet shows it with
Retry (DESIGN §5.4). The visual design is #438.

#### 5.1.5 A trainer held by another app (#314)

A NEO connected to another central does not advertise, so its attempt ends `not_found` after the scan window. It is
retried on its next advertisement, which appears when the other app lets go. The keeper never tries to take a
trainer: there is no forced connect and no retry timer. Without #314's typed signal, "held by another app" and
"asleep or off" look the same, so the chip says "not found" with the hedged hint. When #314 adds the signal, the chip
names the other app's hold, and nothing else in this section changes. The other direction is §5.1.3: "Release trainer" or
hiding crank frees the NEO at once, and going to the background frees it after the 60 s grace.

#### 5.1.6 Observability

Every phase change is logged at INFO under the target `crank_core::links`, so a normal launch writes it to `crank.log`
(AGENTS.md, #411). Each line has `device` (`trainer`/`strap`), `from`, `to`, `ms` (time in the phase being left) and
`attempt` (per device, from 1). The phases timed are these:

- `resolve`, with `via=lookup|scan` (it measures the handle-reuse assumption of §5.1.2);
- `gatt` (connect and discovery);
- `identity` (subscribe and DIS read; for the strap also the identity check, `Pairing::verify`);
- `handshake` (trainer only: page 55 to the page-54 reply, then the trainer's identity check, which runs after it);
- `ready_total`.

Beside the phase lines:

- `intent` lines carry the intent and reason;
- `cool` lines carry the reason (`idle`, `background`, or a `Release` reason) and `held_s`;
- `scan` lines carry `mode=continuous|window` and `ms`, so the scan cap's radio time is measurable;
- `ride_load` logs `trainer=warm|awaited|cold`, the same for `strap`, and its own `ms` from request to answer.

HIL measures warm-up and load latency from these lines (Part B, #44): a warm "Ride this" under 1 s on the NEO, and
cold warm-up per phase. No timing is derived from the UI.

#### 5.1.7 Implementation sequence (sub-issues of #435, filed when this is accepted)

1. **Parallel connect.** `connect_links` runs the trainer and strap connects concurrently, and the phase log lines of
   §5.1.6 are added. This gives the cold-path win alone, with no lifecycle. It also **verifies the handle-reuse
   assumption** (§5.1.2) on the NEO and the strap. It records `via` for a lookup in each of three cases: right after a
   Devices scan, after that scan stopped, and after a disconnect. Results go to the HIL doc. If the lookup fails, the
   assumption is reworded and nothing else changes.
2. **Link keeper** (#469). The keeper's state machine (`crank-core` `keeper`): pure transitions over intent, the clock, link events, pairing and
   rider-config changes, and load or release. Also `links_intent`, `T_cool`, the background grace, the Ready-sheet
   exemption, the keeper's inactivity during a ride, `T_abandon` and the sleep discard of an unstarted ride, the scan cap, release on shutdown, the latch, handover to `ride_load` with
   the link's current state, event draining, and the sim latency model below. `Core::links_intent` is the facade
   method; its Tauri command and binding are wired with the contract (step 3), and the rider-config change input waits
   for athlete inputs (#247, #518): until then the configuration is the constant `RIDER`, and a load passes it.
3. **Contract.** `LoadProgress` in `ride_status`, `links_open` / `LinkFrame`, and the regenerated bindings (shipped,
   #561). **Amended 2026-11-09 (#588):** the contract is ADR-0001 §4.3's snapshot channel, reached in #588's steps:
   - step 2 (#597, shipped): `links_intent` answers `()`, and the `seq` ordering rule is deleted;
   - step 3 (#598): `UiFrame.devices` and `loading` replace `links_open` / `LinkFrame`, `device_links` and
     `ride_status().load`, with one source per device (the core's overlay, §5.1.4); `RideLoaded.hr_linked` is
     deleted.
4. **UI** (#471). Intent reporting (warm views, focus, visibility, throttled input), `crank-desktop`'s sleep observer, the
   chips, and Ready-sheet progress, styled per #438. It reads devices from their one source (§5.1.4). It starts after
   #588 step 2, and preferably after step 3, so that it is written once, against `UiFrame.devices`.

#### 5.1.8 Tests required (acceptance of the sub-issues)

`SimTrainer`/`SimHeart` get a **latency model**: a delay per phase (resolve, GATT, handshake), "never answers", and
"not advertising". Tests run on paused tokio time.

- **Warm start under 1 s.** Phase latencies are 3 s for the trainer and 2 s for the strap. `Likely`, then advance
  until both are `warm`; `ride_load` then answers within 1 s of virtual time with zero `connect` calls.
  `searching` / `connecting` at load awaits the running attempt (connect count unchanged).
- **Parallel bound.** With latencies a and b, a cold load takes max(a, b) ± one tick, not a + b. With the strap
  never answering, the load ends at `CONNECT_TIMEOUT` with the trainer linked and no HR, not later.
- **Idle cool-down disconnects.** After `Left` the devices are still warm at `T_cool` − 1 s and released at `T_cool`
  (driver disconnect observed, phase `cold`). A `Likely` in between restarts the timer. Every `Release` reason
  disconnects at once, mid-attempt included (the half-open peripheral is released). The latch blocks `Likely` until
  Retry or "Ride this".
- **Background grace.** After `Background` the devices are still warm at 59 s and released at 60 s. A `Likely` at 30 s
  cancels the release.
- **The Ready sheet holds.** With a failed load, `Likely { ready: true }` and no input for 30 min of virtual time,
  the keeper's links stay `warm`.
- **A loaded ride keeps its links.** With a loaded, unstarted ride, `Hidden`, `Background` past 60 s and `T_cool`
  past 5 min each leave the links connected, and nothing is written (recording transport: no write after `Ready`).
- **An abandoned load releases.** A ride loaded but unstarted, with no input, is discarded at `T_abandon` = 10 min
  (links released, phase `cold`). Input at 9 min restarts the timer.
- **Sleep discards an unstarted ride.** `Release(Sleep)` with a loaded, unstarted ride discards it and releases its
  links. With a started ride, the ride and its links are untouched.
- **`Likely` during a ride warms nothing.** With a ride loaded or active and the strap not linked, `Likely` from any
  view starts no attempt (connect count unchanged).
- **Scan cap.** With the trainer never advertising and intent `Likely` for 10 min, the scan is continuous for
  `SCAN_CONTINUOUS`, then on for `RESOLVE_SCAN` in every `SCAN_PERIOD`. It stops on `Left` / cool-down. An
  advertisement in a window starts the attempt.
- **Quit while warm.** `Core::shutdown` with both devices `warm`, and again with one still `connecting`: both are
  disconnected, the write log has nothing after `Ready`, and shutdown returns within 3 s even when one disconnect
  hangs (bounded by `DISCONNECT_TIMEOUT`).
- **Nothing is written while warm.** A recording fake transport under the real FE-C driver is held `warm` for 10 min
  of virtual time. Its write log holds only the connect's page 55 and the page-70 request for page 54. The first
  control page is the session's first `SetMode`. At core level, no `TrainerCommand` reaches the control channel
  before the handover.
- **Handover.** The engine's following machine activates on the handed-over `Ready` link without a new `Link` event.
  A link lost while warm makes the load connect cold. A ride's end leaves the phase `cold`, never `warm`. A full
  event channel never faults a warm link.

#### 5.1.9 Alternatives, consequences, limits

| Alternative | Why rejected |
|---|---|
| Connect at launch and hold while the app is open (the old §5 text) | Holds the NEO's only slot and the strap's battery for hours, and blocks other apps and check 10 |
| No speculation: only a parallel connect with handle reuse at "Ride this" | Still an estimated 1–3 s, plus a scan when the lookup fails, so not under 1 s; the owner asked for speculative connect |
| Warm with a GATT connection only, handshake at `ride_load` | Leaves up to about 2.5 s on the start path, and a "ready" chip that may name the wrong or a silent unit (§5.1.2) |
| Write page 48 at 0 % while warm | Makes warm a controlled state, and protects nothing (§5.1.3) |
| Hand a ride's links back to the keeper after the ride ends | Two owners across the §7.9 end and release machinery; "released means disconnected" is relied on by pairing and quit |
| Release at once on `Background` (round 1 of this amendment) | Every quick switch to another app would cost a reconnect; replaced by the 60 s grace (architect decision) |
| An unbounded scan while a device is `not_found` | Hours of radio time for a NEO held by another app; replaced by the scan cap (§5.1.2) |

**Consequences.** A warm "Ride this" does no Bluetooth work. A cold one costs one `CONNECT_TIMEOUT` at worst, not
two. Before a ride, crank holds the NEO only in these cases: while it is frontmost on a ride-likely view, within 5 min of
one, for up to 60 s after going to the background, or with a loaded, unstarted ride until `T_abandon` (10 min
without Start or rider input) discards it. The core gains a
second owner of links before a ride, with a one-way handover. The UI must report intent faithfully: an intent it
fails to send costs latency (no warm-up) or the slot (no release), never safety, because nothing is written while
warm.

**Known limits.**

- The first warm-up after launch scans (#206-3), and later ones may too: handle reuse is an assumption (§5.1.2).
- Another app waits up to 60 s for the NEO after crank goes to the background, unless the rider uses "Release
  trainer" or hides crank.
- A loaded but unstarted ride holds the NEO whatever the focus, including while crank is hidden or in the
  background, until 10 min pass without Start or rider input (`T_abandon`): active use of the Ready sheet holds it for
  as long as that use continues. It is released on Cancel, on `T_abandon`, on sleep or on quit.
- A Ready sheet over a failed load holds the keeper's links without input, until crank is hidden, goes to the
  background or the Mac sleeps. Leaving the sheet does not release them at once: it starts `T_cool` (§5.1.3).
- A device that returns is seen within about 34 s under the scan cap, or at once with Retry.
- Until #314, "held by another app" cannot be told from "off".
- A strap missing at load stays missing for that ride (#450).
- A disconnect that timed out may leave a peripheral connected (#335).
- The sleep observer is macOS only (#41).

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
  link may be held before a ride (the link keeper, §5.1), but the backoff loop is enabled only while a ride is
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
   (Amended 2026-11-11, #585: what is *sent* is the **commanded** target, the controlled target eased by §7.10;
   the controlled target itself stays as stated here and is what compliance compares against.)
2. **Steps:** the new target is sent on the block-boundary tick (coalescing in the `control` slot,
   §4.4, means the newest mode always wins). ~~Smoothing a step is the trainer's job~~ superseded by §7.10 (#585):
   the controlled target changes on that tick and the commanded target eases towards it from that tick. The trainer's
   response time (t90 for 100→150→250→100 W) is measured in HIL check 6. A "lead" (send the next target L
   seconds early) defaults to 0 and is set to the measured trainer latency only by an ADR amendment
   backed by the HIL numbers.
3. **Ramp targets** (`ramp a-b%`): ~~updated every 1 s (4 ticks) along the line~~ evaluated on every tick along the
   line (amended 2026-11-11, #585, §7.10: the eased command follows a ramp exactly only on a continuous line), 1 W
   resolution; never more than one control write per tick (§4.4).
4. **Low-cadence release (spiral guard):** in ERG, cadence < 40 rpm for 2 s with target > 0 → switch to
   `Resistance { pct: 0 }` and show "ERG released, spin up". When cadence ≥ 60 rpm for 2 s → re-engage ERG
   with a soft start: the target ramps from the current 3 s power to the block target ~~over 5 s (1 Hz
   updates)~~ (amended 2026-11-11, #585: by the §7.10 engagement, §7.10.3). The block timer keeps running (the rider is still riding). Released seconds are excluded
   from compliance. "Cadence ≥ 60 rpm for 2 s" is judged on the held cadence (ADR-0001 §5: a trainer value is
   usable for its tick and the next 4), so four real samples and four silent ticks satisfy it (architect ruling,
   #250 round 2). Thresholds are initial values checked in HIL check 9.
   **Amendment 2026-11-09 (#589): gentler defaults, and the thresholds are rider tuning.** The four numbers
   and an on/off switch are `ErgTuning` (ADR-0001 §7.2): `release_enabled`, `release_rpm`, `release_after_s`,
   `reengage_rpm`, `reengage_after_s`. The rule above is unchanged in form; its **defaults** become:
   release when cadence < **30 rpm** for **5 s** with a target > 0; re-engage when cadence ≥ **45 rpm** for
   **1.5 s**, then the soft start (§7.5; its ramp is now the eased engagement of §7.10.3, #585). `release_enabled` defaults to on.
   *Evidence (the owner's ride of 2026-11-09, #589).* Six cadence dips to 0 rpm for 1–3 s in the recovery
   blocks (137 W), the flywheel keeping 12–13 km/h: saddle readjustments, no Bluetooth loss. Each released
   ERG under the old 40 rpm / 2 s rule; re-engaging needed ≥ 60 rpm for 2 s plus the 5 s soft start, so power
   sat at 0–60 W for 5–18 s. Once the rider came back at 58–60 rpm, just under 60, and ERG stayed released
   for ~17 s. The release was too eager, and the re-engage cadence sat above this rider's recovery cadence.
   *Why these values.* A readjustment is up to ~3 s at 0 rpm plus well under 1 s on each side of it below
   30 rpm (cadence fell from ~70 to 0 within 1 s): about 4 s, inside the 5 s dwell, so it never releases. A
   rider back at a recovery cadence (the owner's 58–60 rpm) is above 45 rpm, so a release that did happen is
   left. The 15 rpm gap between 30 and 45 keeps hysteresis. 1.5 s (6 ticks) is longer than one held sample (5
   ticks), so re-engaging needs at least two trainer samples at ≥ 45 rpm, never one stray value.
   *Safety argument.* A held ERG target on a rider who stops is a torque τ = P/ω that rises without limit as
   cadence falls: a rider who stopped for good and pushes off again from a standing crank meets the full
   target at near-zero cadence (the §7.5 low-cadence hazard), and a rider fading under load is pushed further
   into the spiral. The release exists for both. With the new defaults a real stop (cadence 0, or no cadence,
   which counts as 0) still releases, 5 s after cadence fell below 30 rpm instead of 2 s after it fell below
   40: a rider who has stopped for good gets 0 % resistance after 5 s, before a dismount or a deliberate
   restart. A restart inside those 5 s is the readjustment case, and the evidence shows it on a moving
   flywheel at the rider's normal cadence. The cost is in the spiral: a fading rider is now held from 40 down
   to 30 rpm and then for 5 s instead of 2. That is accepted on the owner's judgement that the NEO's ERG is
   not harsh, and the bounds keep it limited: `release_rpm` 20–50, `release_after_s` 1–8 s (so no tuning holds
   a stopped rider under ERG for more than 8 s), `reengage_rpm` 30–90 and above `release_rpm`,
   `reengage_after_s` 1.5–5 s; durations on the 0.25 s tick grid (`ErgTuning::BOUNDS`, `Tuning::validate`).
   Every in-bounds tuning with the release on releases a sustained stop (a property test). With
   `release_enabled = false` the release never happens: that is the rider's explicit choice, and the rider's
   pause (DESIGN §5.2) and the ride end (§7.9) still command `Resistance { pct: 0 }`.
   HIL Part B's copies of the old values (`hil_fec` `RELEASE_WAIT`, `STALL_FLOOR_RPM`) move to the recorded
   tuning in #593.
5. **Soft start** is also used when ERG is (re-)engaged after a reconnect or after leaving `free`/`sim`.
   **Amended 2026-11-11 (#585):** the ramp is the §7.10 easer started at the origin (§7.10.3); the "five 1 Hz
   steps" and "over 5 s" below are superseded by it. The hold, origin and reset rules below are unchanged.
   **Ride start and user pause (amendment, #250 / #264, 2026-10-14).** A ride start is treated as a
   reconnect: on the NEO, ERG at the target torque with a stopped flywheel is the low-cadence hazard of §7.4
   (the rider pushes off against the full target at near-zero cadence). So the engine holds `Erg { target_w: 0 }`
   from the first tick until the first sample with cadence > 0, then runs this soft start from the 3 s power.
   A first block that is `free` or `sim` sends no ERG and is unchanged; the soft start is then owed at the
   next ERG block (the paragraph below). A block change inside a running soft start continues the ramp at its progress towards the new target (DESIGN §17.3). Resuming from a **user pause** is the same engagement: while paused the
   engine commands `Resistance { pct: 0 }` (DESIGN §5.2 "Paused (user)": "the trainer is at 0 % resistance"),
   and the resume owes the soft start with the 0 W hold until cadence. A release in force at the pause
   stays in force after the resume until §7.4 leaves it (its re-engage criterion). Stopped pedalling (auto-pause, ADR-0001 §5)
   needs nothing of its own: it changes no command, and the §7.4 release commands `Resistance { pct: 0 }`
   once its dwell has passed. (Amended 2026-11-09, #589: with the 5 s default dwell the 3 s auto-pause comes
   first and the release follows during the pause; under the old 2 s dwell the release came first.)
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
   start's 5 s plus 5 s (**amended, #585, 2026-11-11:** each of (a), (b) and (c) runs from its discontinuity or
   engagement until the commanded target is on the plan line, plus 5 s, and every tick off the line is settle, §7.10.5; **amended, #250 / #264, 2026-10-14:** a ride start and a resume from a user pause are
   (re-)engagements, §7.5; the first block has no previous block for (a), but its start is an engagement under (c):
   the 0 W hold, then ~~10 s from the first pedalling sample~~ (amended, #585: the engagement until the command is on
   the line, plus 5 s, §7.10.5) are settle). These are the `Transition` class of §4.4.1, which only settle uses (the
   trainer-following machine ignores the class). Within `ramp` blocks and HR-steered steps the target moves continuously
   (`Continuous`) and is compared second by second with no settle (amended with §7.10, round 5: a ramp within the
   rider's rates is tracked on the line, except for the few ticks of its eased corners, which are settle, at most
   2 s for a 5 W/s ramp at the defaults; so is a ramp steeper than the rider's rate while the command is behind it,
   §7.10.5). Released (§7.4) and paused seconds are
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
   ending, or detached, cannot drop or raise a newer ride's signal (#392). Nor can it raise the signal before a
   newer claim: the reaper's reap (not forced) clears the signal for its ride as it takes the session out, under
   the same lock, and from then on that ride's engine cannot set it (`Core::reap`, `SignalOwner`, #399), so P2
   after a normal End does not rest on the engine never calling `set_active(true)` again. That reap does not cut
   the engine off, which would drop the engine's own end mode still queued in the command gate. `settle`'s forced reap (an engine that
   answered `Stop` but never dropped the signal within `REAP_TIMEOUT`) clears the signal the same way, and cuts
   the engine off before it reaps, as a forced end does, so an engine it detaches cannot raise the signal after the
   clear (#392). One way remains for it to stay up with no ride loaded: a forced end or `settle`'s forced reap
   whose future is dropped between its `reap` and its clear. In the app only `shutdown` drops it (its cap is a
   `timeout` on the end), and the process exits right after; the Tauri commands run to completion. The next
   load's claim takes the signal over from it. A `settle` dropped between its cut-off and its `reap` leaves the
   ride loaded, its engine cut off and the signal up: the signal still has its owner, a loaded ride, so no load is
   needed to clear it, and the next End or load settles that ride again (its engine can no longer drop the signal, so
   that `settle` reaps it by force after `REAP_TIMEOUT` and clears it, #399). The drivers' release is
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

### 7.10 Amendment 2026-11-11 (#585): the commanded ERG target is eased (simple easing: rate, acceleration and jerk limits)

*Amends items 1, 2, 3, 4, 5 and 8 above and the class table of §4.4.1. Implementation: #620–#624 (§7.10.10).*

*History* (short; every superseded round is in git: this section at `4b958c87`, PR #712, and PR #625's rounds):
- *Rounds 1–3 (PR #625): a hand-written rate and acceleration limiter. Three safety falsifications, each finding new
  edges (§7.10.9).*
- *Rounds 4–18 (2026-11-14 to 2026-11-26): the motion was Ruckig's (pantor/ruckig 0.19.4, a time-optimal
  jerk-limited trajectory generator; the port was to use `rsruckig`). crank's rules around it, kept below, came from
  those rounds' falsifiers: the frame, the canonical line, the lead, the meet, safe braking, the cuts and their
  domains, the input layer. The rest was about the library's numerical edges: refused inputs (upstream #269), slower
  valid answers on its velocity boundary, a segfault, refusals on degenerate frames near the rate. Each fix round
  added machinery (the twin with limits 2⁻²⁰ tighter, degraded mode, a slow-answer rule, `answer_bound`, a band, then
  `landable`, a fault gate), and each falsifier found the next edge: at `4b958c87` (f712c) a fault-free plan still
  raised the degraded status (ramps 10⁻⁶ W/s apart) and a degenerate lead frame was refused for 17 ticks.*
- *Round 19 (2026-11-27), the owner: "Simple easing, no Ruckig." The closed-form, rate- and jerk-limited easing
  that rounds 11–18 already ran for every stop, probe, brake, hold and chase, and for degraded mode, becomes the
  easer, with one more closed-form profile for the landing and the meet. Ruckig, `rsruckig`, the twin, degraded
  mode, the slow-answer rule, `landable` and the fault gate are deleted. Every rider-facing promise is kept and
  re-proven in closed form (§7.10.6); what is lost against the hybrid is measured (§7.10.2, "What simple easing
  gives up").*

*The design's model: `docs/adr/models/erg_slew.py`, Python's standard library only.*

**Context.** After a real ride on 2026-11-09 (Sweet Spot 2×10, #589) the owner found the step from the end of a ramp
into the next, higher block abrupt. Item 2 left smoothing a step to the trainer, and the only ramp crank sends is
the §7.5 soft start, used only on (re-)engagement: five 1 Hz steps over 5 s, whatever the step's size. The owner's
direction (#585, 2026-11-09), binding:
- separate rising and falling rate limits as rider settings, the falling rate twice the rising by default;
- a limit on the second derivative as well: an S-curve that eases into and out of the ramp, so a step never feels
  like "falling off the stairs";
- one mechanism with the soft start, not two.

The owner's later decisions: 2026-11-14, spike Ruckig and redesign on it (rounds 4–18); 2026-11-27, simple easing,
no Ruckig (round 19, binding). The standing rule to prefer tested library primitives yields here to the ruling: the
primitives are a handful of closed-form polynomials, each property-tested against dense sampling (§7.10.6).

**Terms.**

- The **plan line** `R` is item 1's controlled target before its 1 W rounding: block target × (1 + manual offset +
  coach change), clamped to `[0, M]` with `M = min(max_target_w, cap)` (the health cap when active). A `ramp` block's
  line is evaluated **on every tick** (item 3 as amended below), so it moves continuously. §7.10.2 ("The line on the
  tick grid") fixes how it is sampled and its pieces.
- A **discontinuity** is a jump of the plan line's value: a block boundary whose start differs from the previous
  block's end (§7.8 a), a skip or an edit that enters a block, a manual ± or a coach change (§7.8 b), and a raised cap
  that lets the line jump up. Neither of these is one:
  - a ramp's own motion;
  - a **vertex**, where the line's slope changes at the value the line already has: a ramp's start or end, ramp to
    ramp, or where the clamp to 0 W or `M` starts or ends.
- The **commanded target** `y` is the value sent in `Erg { target_w: round(y) }`, the position of the trajectory of
  §7.10.2. It is **on the line** on a tick when `|R − y| ≤ ε` (ε = 0.5 W, half the 1 W resolution). "Caught up" means
  on the line, nothing else.
- Item 1's rule stands for the plan line: it is never smoothed and never derived from measured power, and compliance
  compares against it (rounded, as today).

#### 7.10.1 Feed-forward on the target, never a loop on measured power

**Decision:** the easer (§7.10.2) shapes the setpoint (feed-forward). It reads only the plan line ahead and its own state,
never measured power, except for the engagement origin (§7.10.3), which is item 5's 3 s power as today.

**Why not a closed loop.** The NEO already closes the power loop in ERG: it sets brake torque τ = P/ω to hold the
page-49 target. An outer loop on measured power (a PID that moves the target until page-25 power follows a smooth
reference) would be a cascade around an inner loop whose dynamics are not yet measured (check 6's t90, #44 Part B, has
not been run), fed by a 4 Hz page-25 signal that lags by up to the 2 s of `L` (§4.4.1). Two loops on one quantity
with unknown inner bandwidth can fight and oscillate. Worse, it is unsafe in the one situation that matters: when a
rider fades, measured power falls below the target, and an outer loop would *raise* the command to catch up. That is
the §7 spiral, driven by crank. Item 1's rule exists against that; a feed-forward ramp keeps it. The one closed loop
crank has, HR steering (item 7), is slow (5 s, ±5 W) on a different quantity and stays as it is. Its output becomes a
line: each ±5 W update ramps over its 5 s interval (at most 1 W/s), so it is no discontinuity and the command stays on
it (no settle, as item 8 has it for HR-steered steps; not yet implemented, so this binds its implementation).

**Write rate (an assumption, checked in #624).** The easer runs on every engine tick (4 Hz) and the engine sends
`Erg { round(y) }` whenever the rounded value differs from the last one sent, as today. The writer (§4.4) bounds what
reaches the NEO: at most one message per tick, the newest value replacing an unsent one in the `control` slot, and
the reserved page-71 poll taking its tick each second. During an ease, and on a ramp steeper than 1 W per tick
(4 W/s), page 49 is therefore written on about three ticks in four, for as long as the ease or the ramp lasts (a ramp
for minutes). **That the NEO takes page 49 at that rate rests on the Tacx document's 4 Hz, the premise of §4.4, not on
a capture:** Part A's driver run made 60 ERG changes in about 6 minutes, never a sustained per-tick sequence. #624
measures it: a `hil_fec` observation of page-71 echo latency and agreement under per-tick page-49 writes (an ease and a
2-minute ramp), and the same figures from a `.fecap` of the owner's ride.
*If echoes lag past `L`.* Nothing in the control path changes: the following machine (§4.4.1) does not alter commands.
An ERG component whose echoes stop agreeing goes `not_following` within its documented bound (`L + 2·P_c + T_mismatch
+ r`, 6 s + r for ERG), the ride status says the trainer is not following, and those seconds are excluded from
compliance (item 8). That is the honest reading of a trainer that is not taking the writes. A measured lag past `L`
amends this section (for example a lower write cadence for eased values); there is no fallback in code.
1 Hz updates (item 3 as it was) were rejected: at the default rates (§7.10.4) they are 20–40 W steps every second
during an ease, and on a ramp they make the easer's input a staircase it would ease every second (the round-1
defect: 59 of 60 s of a ramp excluded from compliance).

**Following (§4.4.1).** The desired mode the following machine sees is the commanded one. Desired changes are not
inputs to it (guarantee 3: "desired changes in between, at any rate, change nothing"), and an echo agrees with any
value desired within `L` = 2 s, so an echo of a value written up to two ticks earlier agrees. The desired history
gains at most one value per tick. The class table is amended: every eased update and every per-tick ramp update is
`Continuous`; only the discontinuities and the engagement are `Transition` (unchanged rows).

#### 7.10.2 The easer: closed-form simple easing, and crank's rules around it

**Decision: the motion is crank's own closed form.** Every tick the engine takes the commanded target's state
`(y, v, a)`, the plan line ahead and the rider's limits, and sends the position one tick later of a trajectory it
computed in closed form when its input last changed (step 7). There is no planning library: no calculation can be
refused, so there is no fallback, no retry and no degraded mode. The trajectories are three profiles, all under one
pair of limits `a_max`, `j_max` and the frame's rates:

- **The slope change** `slope_change(v, a, s*)`: the time-optimal jerk-limited change of slope from `(v, a)` to
  `(s*, 0)` (Biagiotti and Melchiorri 2008, §3.4, for the slope): the curvature ramps at `±j_max` to a peak
  `a_p ≤ a_max`, holds, and ramps back to 0. With `s* = 0` it is **the stop**. It is the chase (step 4), the brake and
  the hold (step 6), X5's stop, and the probe of safe braking, the floor and the lead's timing, exactly as in rounds
  11–18.
- **The landing** (new in round 19): from `(v, a)`, a move of displacement `d` in the line's frame that ends at rest
  in that frame (on the line, moving with it), its slopes within the frame's rates `[−r_dn, r_up]`:
  1. the slope change to a **peak slope** `p` in the direction the move still needs, `g = d − x_s` (`x_s` the stop's
     own displacement), then 2. a cruise at `p`, then 3. the stop from `p`.

  With `p = 0` the profile is the stop itself. The peak that lands exactly is a root of the displacement as a
  function of `p`, on a bracket, and a move longer than the profile at the rate cruises at the rate for the rest:
  - beyond the unwound slope `v_u = v + a·|a|/(2·j_max)` in the move's direction (`p ≥ max(0, ±v_u)`), every slope
    change starts with jerk in that direction, so a larger peak gives a pointwise larger slope and the displacement is
    monotone (self-tested on 4 000 random states): the root is unique. That branch reaches every `d` past its start;
  - only a `d` between the stop's displacement and that of unwinding and then stopping (they differ only where the
    curvature carries the slope onwards, by about `a³/j_max²`) needs a peak between 0 and `v_u`. Slowing to such a
    peak and then stopping pauses the curvature at 0, and the displacement is not monotone there (the model found
    three roots from `(2 W/s, 9 W/s²)`); the root exists by continuity, and the root finder's fixed bracket makes it
    deterministic.
  - *Not chosen: unwinding the curvature first*, which makes the whole family monotone. It delays a braking
    curvature: after X3 under a fall at (50, ×4, 4) the landing undershot the 300 W line by 34 W (K12's suite).

  From rest it is the time-optimal rest-to-rest S-curve (`|d|/r + r/a_max + a_max/j_max` where it reaches the rate,
  self-tested). From a moving state it is the slope change, the cruise and the stop: never refused, at most three
  phases long. Where the frame allows no motion in the direction needed (a rate of 0), it is the stop.
- **The meet** (steps 5 and 6): at rest on a moving line. In the absolute frame it is the landing onto a rest point
  `D` away; the line moves at `s` meanwhile, so the end lies on it iff `h(D) = rel + D − s·T(D) = 0` (`rel` the
  command relative to the line now, `T` the landing's duration). From above a rising line, `h(x_s) > 0` when the
  stop alone ends above the line and `h(−rel) = −s·T < 0`, and `h` is monotone between (a rest point further from
  the stop takes longer), so the root exists and is found; if the stop alone reaches the line, the meet is the stop
  (the line passes the command at rest). Below a falling line, the mirror; there a stop that reaches the line is no
  meet, and step 6 brakes.

**The root finder** (`root` in the model) is the Illinois variant of regula falsi on the bracket the profile's
monotonicity gives: a secant step that never leaves the bracket (a bisection step if it would), halving the stale
end's value when one end repeats. It converges whatever the kinks of the function (a slope change switches shape
where its peak curvature reaches `a_max`), stops at `|f| ≤ 10⁻¹²` relative or a bracket a few ulps wide, and is
capped at 200 steps. Self-tested: over 20 000 random inputs (states, displacements, frame rates down to 10⁻⁹ W/s) the
landing ends at its displacement within 10⁻⁶ relative (plus floating point times a cruise of up to 10⁶ s; a cruise
at 10⁻⁹ W/s can take 10¹² s, longer than any plan, and is checked over its first 2 hours), at rest, its slopes within
the frame's rates, its curvature within `a_max`; over 3 000 random meets from moving states it ends at rest on the
moving line (from above within 10⁻⁶ W; from below at or under it by at most 10⁻³ W relative, where the falling line
then comes down onto the command; the worst measured is 1.5·10⁻⁶ W).

*Why simple easing is enough.* Ruckig's one advantage was time-optimality from any moving state. From rest the
landing is the same time-optimal S-curve, which is where every quoted figure is measured: they are unchanged (below).
From a moving state the landing is never more than the slope change, the cruise and the stop. The rules of this
section never needed more: every rider-facing promise (§7.10.6) is a bound that a feasible profile meets, and the
landing is a feasible profile by construction. What is lost is measured below.

**Determinism.**
- Every profile is closed form: polynomial segments, square roots, and a bounded root finder with a fixed start (the
  bracket's ends), no iteration seeded by time or randomness. The same input on the same build gives the same
  trajectory.
- Across platforms and between the model and the Rust port, results agree to floating point, not bit for bit (`sqrt`
  is exact, but the root finder's last step and `libm` may differ by an ulp). So #620's golden vectors compare with a
  tolerance of 10⁻⁶ W, W/s and W/s². The model and the port must agree on every vector within it.

**The mapping.**

| Our quantity | Limit |
|---|---|
| commanded target `y` (W) | 0 W and `M` (X3, X4) |
| its slope `v` (W/s) | `r↑ = rise` rising, `−r↓ = −rise·factor` falling |
| its curvature `a` (W/s²) | `±a_max`, `a_max = 2·rise/τ` |
| the ease, its jerk (W/s³) | `±j_max`, `j_max = 4·rise/τ²` |

`τ = ease` is the **ease time**: from rest to the full rising rate in exactly τ, as one S-curve. The acceleration is a
triangle rising to `a_max` in τ/2 and back to 0. A fall uses the same acceleration and jerk limits, so it reaches the
falling rate `r↓ = f·r↑` in `τ·(1 + f)/2`.
- One pair of limits for every move: a change of direction never changes the limits under a moving state (rounds 4–18
  found that per-direction limits leave a state a previous trajectory produced outside the new, smaller limits).
- Velocity, acceleration and jerk hold inside every profile by construction (self-tested). The properties below check
  them on every tick anyway, on the sent positions.

**The line on the tick grid** (round 3's sampling, kept):
- Tick `k` starts at `t_k = k·Δt`, with Δt = 0.25 s. The plan as a function of time, `P(t)`, is each block's value on
  `[start, start + dur)`: a steady block's target, or a ramp's `from + (to − from)·(t − start)/dur`.
- Block durations are on the 0.25 s grid, so every boundary is a tick.
- `R_k = clamp(P(t_k)·(1 + offset + coach), 0, M)`, sampled at the tick's start. This is dev's
  `frac = elapsed/dur` (`crates/crank-engine/src/erg/policy.rs:219`): a ramp's first tick carries its `from`, and
  the next block's first tick carries its `to`.
- The line is a list of **pieces**, built in canonical form from tick 0 every time, so it never depends on the tick
  that asks:
  1. each block's line (multiplier applied), joined if it starts within 10⁻⁹ W of where the previous block ended;
  2. joined lines whose slopes agree within `SLOPE_TOL` = 10⁻⁹ relative (max(1, |slope|)) are **merged into one
     line**, its slope recomputed from the merged endpoints;
  3. a merged slope within `SLOPE_TOL` of 0 is exactly 0 (round 7: a slope of 7.7·10⁻¹⁴ W/s read as rising held the
     command 0.75 W above a 0 W line);
  4. each line is split where the clamp to `[0, M]` starts or ends, and a clamped piece is flat at 0 W or `M`;
  5. **after the clamp, a piece is joined to the previous one only if the clamped line continues there**: its first
     tick's value equals the previous piece's value at that tick within `VALUE_TOL` = 10⁻⁶ W; inside one line, where
     the clamp starts or ends, a piece is joined (a vertex); a block that started where the unclamped line ended can
     still jump after the clamp (a ramp that falls from above `M` within one tick): a discontinuity;
  6. adjacent pieces are **merged after the clamp** only if they are joined in this sense, start where the previous
     one ends, and have the same slope within `SLOPE_TOL` (two blocks both held at `M`, or both at 0 W; round 6 merged
     on slope alone, and a flat piece at `M` swallowed a lower flat block: 232.9 W above the plan);
  7. **only then** are the pieces that end at or before tick `k` dropped (round 10; round 7 dropped them first, so a
     merged piece's start depended on the tick that asked). The model checks it as a property: for 2 000 random
     plans, every multiplier and cap, the line at tick `k` is exactly the tail of the line at tick 0, and canonical.

  So a ramp split into blocks of one slope (an editor's split, a file format's, endpoints with float noise) gives the
  same pieces as the single block: exactly at multiplier 1, within 5·10⁻¹³ W otherwise. Hence the same trajectories
  and commands (checked at multiplier 1 on 300 random splits: difference 0 W).
- A piece's start is a **vertex** when the line is continuous there and the slope changes (not collinear), and a
  **discontinuity** when it jumps.
- The easer's frame and lead keys are compared within floating point (10⁻⁹ relative; equal values first, so a cap of
  +∞, none, is the same input as +∞: round 19's invalid-input generator found `∞ − ∞` = NaN making every tick under no
  cap a new input, re-planned from the sampled state, and P4 missed; latent since round 4), so two representations of
  one line never count as a new input. A lead is identified by the vertex it leads into, and its timing test admits
  ties (+10⁻⁹ s).
- The line ahead is known up to the plan's end or the next event (skip, ±, coach, cap, engagement, edit). Each event
  re-plans.

**Per tick.** The engine computes tick `k`'s command from the state `(y, v, a)` at tick `k − 1`, in this order:

1. **Cap (X3).** If `y > M`: `y ← M`, keeping only the falling slope from which the stop still ends at or above the
   line there, `min(line, M)`, and no negative curvature once the slope is reduced (rounds 15–16, closed form), so a
   cap below a falling command never stops a fall it can still stop on the line and never undershoots it. The cap is
   never eased.
2. **The frame is the piece in force over the step.** The step from `t_{k−1}` to `t_k` belongs to the piece in force
   on tick `k − 1`: the backward-slope convention of the line on the tick grid.
   - At a vertex on tick `k` (a kink, the line continuous there), the step still uses the previous piece, and the new
     piece takes over from the next step. Where the clamp starts or ends, and at a discontinuity, the new piece is
     used at once.
   - The landing works in the frame of that piece, a line `c + ŝ·t` moving at its slope `ŝ`: the state relative to it
     is `(y − line, v − ŝ, a)`, and the target is `(0, 0, 0)`, "on the line, moving with it". The frame's rates are
     `r↑ − ŝ` up and `r↓ + ŝ` down, so the absolute slope stays within `[−r↓, r↑]`.
   - *Round-4 fix:* the new piece extended back one tick put the command "ahead" of a steeper line at a ramp-to-ramp
     vertex, so it landed from above while rising and the rise was cut (2 701 slope pairs).
3. **Lead into a vertex (when the plan says so).** At the next vertex:
   - **Where:** where the slope falls (a rising ramp's end, a falling ramp's start, ramp to falling ramp), or where a
     fall ends without a rise starting (a falling ramp's end into a flat or gentler fall).
   - **When:** from the tick where the time to the vertex is at most half the slope change between the two slopes
     (`slope_change` from the current state) plus one tick.
   - **How:** the frame becomes the next piece's line, extended back. The slope is capped at the current piece's: at
     most `ŝ` before a slope fall, at least `ŝ` before a fall's end (the landing's frame rate in that direction is
     the difference of the two slopes).
   - **Effect:** the command eases into the vertex on the side that lowers the load: below the line before a rise
     ends, braking the fall early, above a falling line, before a fall ends.
   - **Where the slope falls, only from on the current line** (within ε at tick `k − 1`). The next line extended back
     lies above the current one; from above, aiming at it would raise the command above the plan (seed 3, plan 15889,
     pinned); from below, the cap would hold it back from the line (round 6). A fall's end has no such condition.
   - **Never at a vertex that starts or steepens a rise:** leading there would raise the load before the plan does.
   - *Round 19:* a lead between slopes 10⁻⁶ W/s apart has a frame rate of 10⁻⁶ W/s; its landing cruises at it, never
     refused (Ruckig refused such degenerate lead frames in fault-free riding: f712c's F3–F5, golden).
4. **A line at or beyond the rate is chased.** A line strictly within the rates (`−r↓ < ŝ < r↑`) is landed on (the
   frame of step 2). Where `ŝ ≥ r↑` or `ŝ ≤ −r↓`, "on the line, moving with it" is infeasible after an eased start
   (it would need a slope beyond the rate), so the command **chases it at the full rate**, towards where the line is
   on tick `k`: `+r↑` at or below a rising line, `−r↓` at or above a falling one. "At" means within ε: on the line
   counts as behind it (round 6; a golden plan pins it). If the line moves towards the command, the target slope is 0:
   the command holds and the line comes to it. The chase and the hold are `slope_change` to the target slope.
   - *Round 19: no band near the rate.* Rounds 18's fix rounds chased pieces near the rate too (`landable`), because
     Ruckig refused their degenerate frames. The landing's frame rate there is small (`r↑ − ŝ`), and it simply
     cruises at it: the command lags an eased start by the slope change's lag and closes it at `r↑ − ŝ`, exactly as
     the chase would. P7 and D5 apply on every line strictly within the rates again, as their bound `ramp_bound` is
     built for (it is finite for every such slope, and long where the rate is small).
5. **Above a rising line, come down and meet it (round 14).** If the command is above a rising line by more than ε on
   tick `k`, the trajectory is **the meet**: the command comes down within the rates, never rises while above the
   line, and comes to rest on it; from on the line, the next tick lands on it from below (step 4). This covers a
   downward step onto a rising ramp (skip, ±/coach, cap), a fall that meets a rising ramp, and an engagement above one
   (seed 585's plan 442, pinned). Rounds 1–13 held at rest and let the line come up (f625i: a −50 % offset onto a
   0.17 W/s ramp held 151 W high for 280 s); the repro is pinned and is at the line within its P7 bound.
6. **Safe braking.** A step after which the command **rises or will rise** is taken only if the new state itself, on
   its own tick, and then the stop from it stay at or below the plan line known ahead, tick by tick, until the command
   is at rest (round 6 added the candidate's own tick: seed 1, plan 10743).
   - "Will rise" means `v > 0`, or a braking fall whose curvature will carry it past rest: `a > 0` and
     `v + a²/(2·j_max) > 0` (round 5: checking `v > 0` alone let a braking fall's own curvature force a rise one tick
     later).
   - The stop may pass the line by at most ε/2, strictly inside X2's ε, so an accepted braking path is never cut
     (seed 2, plan 19521, pinned), and `M` by at most `ROUND_SNAP` = 10⁻⁹ W, what step 8 snaps (seed 585, plan 7929,
     pinned: a ramp into the cap brakes to rest exactly at `M`).
   - A step that leaves the command falling is taken only if its stop stays at or above 0 W.
   - **Below a falling line, it meets the line at rest (rounds 16–17).** Where a rising step is rejected below a line
     falling within the rates (not leading into a vertex), the trajectory is the meet onto the line **one tick of its
     fall early** (`|ŝ|·Δt` below it), checked by safe braking like any step; each tick tries the ordinary landing
     first. Braking to rest and re-planning climbed in pulses (A353: a D5 landing in 6.25 s against 5.29 s); meeting
     the line itself left the rest point up to `|ŝ|·Δt` above the line at the next tick boundary (f625l: 7 of 400
     recoveries missed D5). Both are pinned, and 360 recoveries at (50, ×1, 1) miss none.
   - Otherwise **the brake**: the stop from the current state, the very profile the probe accepted that state on
     (round 18, kept: what the probe vouches for is what runs).
   - This eases without a cut: a short block followed by a lower one (at rest by the boundary, below the plan), a fall
     that meets a drop, a falling ramp that ends at 0 W.
7. **Re-plan only on a change.** A trajectory is computed when its input changes and otherwise sampled tick by tick:
   a new frame or piece; a lead, a chase, a meet or a brake starting; a cut; an event or an engagement. The input is
   the frame (the line `c + ŝ·k·Δt`, the limits, the target kind, the cap); a lead's frame is the next line's, so when
   that line takes over nothing is computed, and the lead's vertex is kept apart, so a lead continues only into its
   own vertex. The model counts computations on an unchanged input, and they must be 0. Sampling the trajectory in
   force keeps the command what the probes vouched for, and makes it independent of when it is sampled.
8. **After the step.** A cut is named (an exception) only if it changes the state by more than floating point:
   10⁻⁶ W, 10⁻⁵ W/s, 10⁻⁴ W/s². A landing within `ROUND_SNAP` outside `[0, M]` is snapped, not cut.
   - **X2:** a step that would end above the line and higher than it started stops instead (`y` kept, `v = a = 0`).
   - **X4:** `y < 0` gives `(0, 0, 0)`.
   - **X3:** `y > M` gives `(M, 0, 0)`.
   - Every named cut forces a re-plan.
9. **Command:** `Erg { target_w: round(y) }`.

**X5: an input that is not a number (the input layer).** The only time the easer does not follow the plan. A plan or cap
that is not a number (NaN or ±∞ in the line before the clamp, in a block or an offset; a NaN or −∞ cap; round 14), or a
state that is not a number (a NaN or ±∞ origin, engagement, initial value or slope; round 15), is a fault:
- **The command comes to rest smoothly, then holds.** Each such tick takes one tick of the stop from the state, then
  holds at rest. A state that is not a number is first replaced at rest: its value if that is a number, else the last
  command sent, else 0 W. The output is a finite number in `[0, max(M, 0)]` on every tick. The cuts apply against the
  cap in force, and X2 against the last line known whose pieces are numbers (an offset or a cap not a number leaves
  the line of the last valid ones), so an unforeseeable drop on the same tick stops a rise as it does outside X5
  (round 19: the invalid-input generator found a skip and a NaN offset on one tick rising 30 W above the line, D3; a
  block not a number gives no line, and no X2).
- **The status `erg_input_invalid`** (`ergInputInvalid`, §7.10.5) is raised on exactly those ticks, and the log records
  the input. Round 19 keeps the status for this alone and renames it: rounds 13–18's `erg_easer_degraded` also
  reported the library's refusals, and with no library there is nothing else to report. Normal planning resumes on
  the first tick whose input is valid.
- **The cap comes first (round 15).** A valid cap (a number, or +∞ for none, 0 W if below 0 W) takes effect on the
  tick it arrives, whatever else is a fault (round 14 lost a cap of 100 W during a NaN offset: 62.5 s at 300 W). A NaN
  or −∞ cap leaves the last valid cap in force, in the easer and in the line alike.
- **The input layer** (round 16, the rule the port mirrors). The cap is valid if it is a number or +∞, and is 0 W if
  below 0 W; the ± offset and an engagement are valid if they are finite. For several events on one tick, each input
  takes them in the order they arrive: the **last valid value wins**, and **any invalid value raises the fault** on
  that tick, which stays raised while that input's latest value is invalid. The initial cap counts as the first value
  (round 17). Every order is tested for the cap, the offset and the engagement.
- **The hold through a valid lower offset while a cap or offset fault persists** is by design (round 17): while any
  input is invalid, the easer does not know the plan it would follow, and planning towards a line built on a cap it
  does not have could raise the load past it. The valid offset is in the line and takes effect on the first valid
  tick.
- *Why it is safe.* The state X5 starts from was accepted by safe braking, which accepts a rising state only if the
  stop from it stays at or below the line known ahead (by at most ε/2), and a falling one only if it stays at or above
  0 W. X5 *is* that stop. Where an unforeseeable event moves the line or the cap under it, the cuts apply in their
  domain as everywhere.

**The remaining exceptions.** The rate (P2) holds on every tick. The acceleration limit and the jerk limit (P3) hold on
every tick, judged on the **sent positions**' finite differences (slope, curvature, jerk), except where one of these
cuts breaks them. Each cut is allowed only in its domain and within its bound, and the model fails otherwise:

| | Domain (anywhere else the model fails) | Bound on the slope change | Worst seen (round 19, the suite: 21 756 plans) | Why it is safe |
|---|---|---|---|---|
| X2 | Only while an **unforeseeable event** (skip, ±, coach, cap, engagement) is pending, until the command is next on the line and moving with it | `r↑ + a_max·Δt` | 50.0 W/s | It only stops a rise; continuing would raise the load above a plan that was just lowered (P5) |
| X3 | The same | a position step down to `M`; the slope changes by at most `max(r↑, r↓) + a_max·Δt` (step 1; after X3 the command never falls below the line it capped, `min(line, M)`, by more than ε, until it has landed on the line or an event; a lead into the line ahead excepted) | 7.6 W/s | The cap is a limit; it only lowers the load, and never below the line it caps |
| X4 | The same | `r↓ + a_max·Δt` | none | Nothing is below 0 W, and there is no load to feel at 0 W |

**No corner is left in event-free riding:** X2, X3 and X4 never break the band without an unforeseeable event, and the
model fails if one does. That covers every vertex (a ramp's start or end, ramp to ramp: 1 421 slope pairs over the
defaults and the bound corners, within, at and beyond the rates; the clamp), every jump, every short block followed by
a lower one, and every engagement.

**Figures** (defaults unless named; `erg_slew.py --report`; round 19's closed form gives every figure rounds 4–18
quoted, unchanged: each is a landing from rest or a chase, where the landing is the time-optimal S-curve). On the line
after a jump onto a held line:

| Jump | Rising | Falling |
|---|---|---|
| 20 W (150 ↔ 170) | 2.5 s | 2.5 s |
| 83 W (137 ↔ 220) | 5.5 s | 4.5 s |
| 150 W (100 ↔ 250) | 8.75 s | 6.0 s |
| 300 W (100 ↔ 400) | 16.25 s | 9.75 s |
| engagement 0 → 137 W, 0 → 250 W | 8.25 s, 13.75 s | — |

- **Durations.** A rise from rest lands within `Δ/r↑ + τ`. A fall is at or below the line within `Δ/r↓ + τ·(1 + f)/2`.
  Over 3 000 held-line cases across the tuning grid, the largest excess over these was −0.21 s and +0.01 s, against 0.75 s
  allowed (P4, P7).
- **The owner's case.** A ramp 100 → 180 W over 60 s, then a 220 W block, lands 3.25 s after the boundary.
- **Steep ramps.** 100 → 400 W over 10 s lands 16.5 s after its start, with slopes 0…20 W/s. 400 → 100 W over 5 s
  lands 10.0 s after its start, with slopes −40…0 W/s.
- **First second.** The first second of an engagement adds 3.33 W (`j_max·t³/6` with t = 1 s), and 25.0 W at rise 50
  and ease 1. Round 3's figures were 6.25 W and 31.25 W; today's soft start adds 27 W.
- **Ramps within the rates** are followed with both corners eased:

  | Ramp | Off-line ticks | Excluded seconds | Largest gap |
  |---|---|---|---|
  | 100 → 400 W over 60 s (5 W/s) | 6 | 2 | 2.35 W |
  | 50 → 150 W over 30 s | 4 | 2 | 1.25 W |
  | 100 → 400 W over 120 s | 3 | 1 | 0.83 W |
  | 300 → 100 W over 60 s | 0 | 0 | 0.23 W |

  - A rising ramp's start lags below the line: the S-curve starts on the vertex's tick, with no lead into a rise.
  - Its end is led, and so is a falling ramp's start, so the falling ramp has no off-line tick.
  - A ramp at the rider's rate (100 → 300 W over 10 s) is chased, and lags up to 20 W.

**What simple easing gives up** (round 19, measured). Ruckig's landing was time-optimal from any state; the landing is
time-optimal from rest and otherwise the slope change, the cruise and the stop. Measured against round 18's model (`4b958c87`, Ruckig 0.19.4 in the loop, no injected fault) on the same plans,
as off-line time (the ticks the command is more than ε off the line, so the time to the target summed over a plan):
- **the golden plans with valid inputs** (45): 37 identical; 1 longer by one tick (0.25 s, 18585 + 12602); 7 shorter
  by 0.75–2.25 s, where round 18 re-planned around a refusal or chased a near-rate piece (PR #693's finding 2, f712c's
  F4, the ramps near either rate, the lead from a moving state); in all, 2 504 s against 2 493.25 s;
- **2 000 random plans** (round 7's generator, seed 585): 1 728 identical; 131 longer (median 0.25 s, 90th
  percentile 1.75 s, worst 3.25 s); 141 shorter (best 9.25 s); in all, 56 585.75 s against 56 557.5 s.

Where the landing is longer, it starts from a moving state: an event mid-ease, a re-plan after a cut, a lead's
handover. There it changes the slope to its peak, cruises and stops, where Ruckig found the single time-optimal
profile. Every figure this section quotes is a landing from rest or a chase, and is unchanged. Nothing rider-facing
is given up: every property of §7.10.6 holds as before, P7 holds on more lines (step 4), and nothing can fail to
compute.

**Measurements (round 19, on the final model; every run sequential under the host lock, 0 check failures in all of
them).**
- The suite, `erg_slew.py --report`: exit 0 in 118 s. 21 756 plans, 54 golden plans matching, 7 821 plans with every
  input valid and never held (P8), `erg_input_invalid` on 263 512 ticks, each with an input not a number.
- Round 7's generator, every input valid: seeds 1001–1020, 101–110, 301–304 and 3001–3004 at 5 000 plans each
  (190 000 plans, 56.2 M ticks): never held, no failure.
- The falsifiers' generators of PR #712, adapted (their refusals dropped, since nothing can refuse):
  - the band sweep (band.py, slopes at fix round 1's band edge, seeds 501–516, 24 000 plans), where round 18 at
    `047fe9f` had episodes of up to 54 ticks;
  - the edge sweep (f712c's edge.py, slopes at `landable`'s old edge, seeds 11–16 and 21–23, 36 000 plans);
  - f712c's lead grid (vertices between slopes 2·10⁻⁹ to 10⁻³ relative apart, over the defaults, the corners and a
    twelfth of the grid: 15 288 plans), the long near-rate ramps at M = 65 535 W (big.py: 72 plans) and 30 min block
    alternations (gate.py: 18 plans).
- Inputs not a number (seeds 1–10, 50 000 plans): X5 on 2.29 M ticks, every one an invalid input's, D1–D5 held.

#### 7.10.3 The soft start is the easer's engagement

An ERG (re-)engagement (item 5: ride start, resume from a user pause, leaving a release, leaving `Free`/`Sim`, a new
link epoch) starts the easer from `(y, v, a) = (origin, 0, 0)`. `origin` is item 5's 3 s power at the epoch's first
pedalling sample (0 W without a power value), clamped to `[0, M]`.

Everything else of item 5 is unchanged:
- `Erg { 0 }` from the engagement until that sample, so no torque reaches a rider who is not pedalling;
- while samples are missing, the easer holds its state and the last command;
- when samples return, it restarts from the then-current 3 s power;
- all engagement state is reset on every applied link transition and every 8-tick silence;
- pre-loss power is never an origin.

An engagement in the middle of a plan (a reconnect on a ramp, for example) is the same: the easer lands from
`(origin, 0, 0)` on the line in force, by the rules of §7.10.2.
- An origin above a falling line is excess while the line falls (P6). At 260 W on a −20 W/s ramp: at most 14.2 W
  above it, on the line after 15.5 s.
- An origin above a rising line comes down and meets it at rest, under step 5, and is tracked from below.

`SOFT_START_TICKS` and the five 1 Hz steps go.

**When it completes.** The engagement (the soft start no longer owed) completes on the tick the command is on the line.
Behind a line the command cannot follow (step 4), it completes on the first tick the command moves towards the line at
the full rate. From then on the chase is settle under item 8 as amended (§7.10.5), not engagement.

It is gentler at the start and longer overall than today's 5 s: 8.25 s from 0 to 137 W, with 3.33 W in the first second.
A block change during an engagement continues it towards the new line, as DESIGN §17.3 5a requires, with no rule of its
own: it is a re-plan.

#### 7.10.4 Settings, defaults, bounds, short blocks

A new tuning group (ADR-0001 §7.2), `Tuning.slew: SlewTuning`, frozen at `ride_load` like `erg`:

| Field (JSON) | Type and grid | Meaning | Default | Bounds |
|---|---|---|---|---|
| `slew.riseWPerS` | integer W/s (`u16`) | rising rate `rise` (`r↑`) | 20 | 5–50 |
| `slew.fallFactor` | number on a 0.25 grid (`f64`) | falling rate `r↓` = rise × factor | 2.0 | 1–4 |
| `slew.easeS` | seconds on the 0.25 s grid (`f64`) | ease time `τ`: `a_max = 2·rise/τ`, `j_max = 4·rise/τ²` | 2.0 | 1–4 |

**Cross-field rule** (`Tuning::validate`): `rise × factor ≥ 20 W/s`, the falling rate's floor. This is what bounds the
time above the plan (§7.10.6). An off-grid factor or ease is refused like any off-grid duration (ADR-0001 §7.2). The
engine never uses other limits.

*Evidence and status of the defaults.* The ride of 2026-11-09 (#589: recovery blocks at 137 W, no Bluetooth loss)
shows that the step at the end of the ramp was felt. It does not give the step's size or a rate that would have felt
right: the record has no target column yet (#622), and the NEO's own step response (check 6) is not measured.

**The three defaults are guesses to tune from rides (#624):**
- 20 W/s makes the 83 W step of a recovery-to-work transition a 5.5 s S-curve;
- the factor 2 is the owner's;
- an ease of 2 s spreads each corner over a second of rising and a second of falling acceleration.

The bounds are design choices:
- `rise ≤ 50` and `ease ≥ 1 s` keep every engagement a ramp: the first second adds at most 25 W;
- `rise ≥ 5` and `ease ≤ 4` keep a 300 W rise to about a minute: `300/5 + 4` = 64 s;
- the factor is at least 1, so a fall is never slower than a rise.

*No off switch.* The engagement is the same mechanism and must stay a ramp. A rider who wants crisper steps sets 50 W/s
and 1 s: a 200 W step in about 5 s.

**Short blocks: the rates hold.** The rider's limits are limits (the owner's direction), so a block too short for them
is not reached:
- the command eases towards the block's line for as long as it can;
- before a known lower block it eases out to rest by the boundary (step 6), so a short rep peaks lower than it would
  with an un-eased cut;
- block time is unchanged;
- every tick off the line is settle (§7.10.5), excluded from compliance, not counted as a miss;
- after a boundary onto a held line, the excluded seconds are at most `⌈Δ/r + τ_r + 0.75⌉ + 5` s, with
  `τ_r = τ` rising and `τ·(1 + f)/2` falling: 23 s for a 300 W rise at the defaults;
- the chart draws the hump it will be (DESIGN §19.2).

Figures for 100 ↔ 400 W (the 5th rep's hard block):

| | Defaults (20 W/s, ×2, 2 s) | Fastest (50 W/s, ×4, 1 s) |
|---|---|---|
| 30/30: peak, mean | 400 W, 316 W | 400 W, 366 W |
| 15/15: peak, mean | 360 W, 232 W | 400 W, 332 W |
| 5 s at 600 W after 100 W: peak | 160 W | 300 W |

Whether that is right for short intervals is an owner question (PR #625). The recommendation is:
- keep the limits binding;
- tune the defaults on #624 with a short-interval session too;
- add a compile-time warning in the editor for a block the rider's tuning cannot reach (#627), like the cap warning of
  item 1.

#### 7.10.5 Interactions: block time, compliance, mode changes, status

- **Block time.** Unchanged. The block timer and workout time are the plan's; the eased seconds belong to the block
  they are in. A rise costs the entered block some work, and a fall gives the previous block's load a little longer
  (bounded, §7.10.6). The ease of a jump starts at the event, never before it (item 2's "lead" stays 0). The only
  things done before a known boundary lower the load:
  - the lead into a vertex;
  - the ease-out before a known drop.
- **Compliance (item 8, amended).** It still compares `power_3s` with the rounded plan line (the controlled target). A
  tick is **settle**:
  - when the command is not on the line;
  - for 5 s after it lands following a discontinuity or an engagement.

  These are the new settle (a), (b) and (c), replacing their fixed 5 s and the soft start's "5 s plus 5 s". A second
  with a settle tick is excluded as today (`compliance.rs`: one excluded tick excludes its second). Ramps entered from
  an equal target, at the defaults:

  | Ramp | Round 1 (1 Hz line) | This design |
  |---|---|---|
  | 100 → 400 W over 60 s (5 W/s) | 59 of 60 s excluded | 2 (its eased corners, at most 2.35 W off) |
  | 50 → 150 W over 30 s | 29 of 30 | 2 (1.25 W off) |
  | 300 → 100 W over 60 s | 32 of 60 | 0 |
  | 100 → 400 W over 120 s | 34 of 120 | 1 (0.83 W off) |

  A ramp entered by a jump is settle until it lands, plus 5 s. A ramp the command cannot follow is settle while the
  command is behind it: correct, since the trainer is not asked for the plan the rider limited. Released, paused,
  `suspect` and `not_following` seconds are excluded as before.
- **Mode changes bypass the easer.** A user pause, a §7.4 release, a `Free` or `Sim` block, the ride end (§7.9) and a
  trainer loss are mode changes:
  - `Resistance { pct: 0 }` or `Free` is the desired mode on the tick it is decided, exactly as today;
  - the easer is not consulted, its state is dropped, and the return to ERG is an engagement (§7.10.3).

  **A downward step towards a release, a pause or a stop is never delayed by a ramp.** The release criterion is
  unchanged: it reads the last command sent (`Erg` above 0 counts as torque, eased or not), so an ease never hides a
  stall.
- **Skip, edits, ±, coach.** A discontinuity: a re-plan from the current state. An edit that enters ERG from `Free`
  (DESIGN §17.3 5b) is an engagement. A manual − mid-rise stops the rise at once (X2), then eases into the fall.
  Unforeseeable reversals are where X2 remains.
- **Status (#622).**
  - `easingToW`: the rounded plan line while the command is off it, else `null`.
  - `softStartTargetW`: keeps its meaning (an engagement after a recovered loss, until it completes, §7.10.3) and comes
    from the same state.
  - `ergInputInvalid` (round 19; rounds 13–18's `ergEaserDegraded`): `true` on a tick whose plan, cap or state is not
    a number (§7.10.2, X5), else `false`. The UI shows it as a fault, like `not_following`, and the log records
    `erg_input_invalid` with the input. Seconds while it is raised are settle, since the command holds off the line.
    With no planning library the easer has no other fault to report.
  - The commanded target is streamed and recorded per second as the metric `target`: W, the last tick of the second,
    `null` outside ERG.
- **Drawing.** The power trace draws the commanded target behind "now", and the predicted trajectory over the known
  plan ahead of it, with the ±5 % band around that line. The strip stays blocks (DESIGN §19).
  - **The core computes the prediction:** it runs the same easer, on a copy of its state, over the plan ahead. It sends
    the result on the ui channel as a per-second polyline on every re-plan (an event, an engagement).
  - So the app needs no port of the easer: #623's TS port is withdrawn (a second implementation to keep identical).

#### 7.10.6 Properties and the safety argument

The properties of the easer, for every in-bounds tuning and any plan. #620 tests them word for word as property tests
on the port; the model checks them on every tick.
- Every rate, curvature and jerk property is judged on the **sent positions** `y_k`: their first, second and third
  finite differences over Δt. So a cut cannot hide behind the easer's internal state.
- On a cut tick, the slope change must be within that cut's bound and the cut inside its domain.
- A window that contains a cut tick's slope may exceed the band by that cut's own slope change `Δv_c` (the larger of
  the sent slope's and the state's), and by no more: the curvature one tick after the cut by `Δv_c/Δt`, the jerk one
  and two ticks after by `2·Δv_c/Δt²`. A cap drop's position step exempts the windows that contain it, and its slope
  change is judged by X3's bound.
- **P4/P7 on every plan** (round 6). Wherever a flat piece starts, whether a step, a ramp's end, an event or an
  engagement, the command must reach it within `landing_bound`, if the piece and the plan last that long without a
  further event:
  - from rest: `Δ/r + T_e + 0.75 s`, with `T_e` = τ rising and τ(1 + f)/2 falling;
  - from a moving state, add the time and distance of easing it out: unwinding the curvature `|a|/j`, which adds
    `a²/(2j)` to the slope, then stopping the slope.
  Where X5 holds in the window, P4/P7 are not promised; D5 applies from the first valid tick.
- The sent state is the reported one: over a tick at piecewise-constant jerk, the position step is
  `Δt·(v₀ + v₁)/2 − (a₁ − a₀)·Δt²/12` within `j_max·Δt³/6`.
- Every X5 run is checked against the stop from the state it started from, tick by tick to 10⁻⁶ W (D4), and the
  landing after it against D5.

*Why the closed form keeps them* (round 19). The properties are bounds on what is sent; none was ever a property of
Ruckig's output. Each holds by construction of the profiles and by crank's rules, and is checked on every tick:
- **P2, P3:** every profile's slopes lie within its frame's rates, which keep the absolute slope within `[−r↓, r↑]`
  (the landing's phases are slope changes between slopes inside the rates and a cruise), and its curvature and jerk
  within `a_max`, `j_max` (self-tested on 20 000 random landings, and checked on the sent positions).
- **P5, P6:** safe braking (step 6) vouches for every rising step with the stop, the profile the brake then runs; the
  meet (step 5) never rises while above the line; X2 where an unforeseeable reversal leaves no room.
- **P4, P7, D5:** `landing_bound` and `ramp_bound` allow easing the state out, then covering the gap at the closing
  rate, plus the ease and 0.75 s. From rest the landing is the time-optimal S-curve, inside the bound
  (`|d|/r + r/a_max + a_max/j_max ≤ Δ/r + T_e`, since `a_max/j_max = τ/2` and `r↓/a_max + τ/2 = τ(1 + f)/2`). From a
  moving state the landing changes the slope to the peak directly instead of easing out to rest and starting again,
  and its cruise covers the rest at the rate. That it stays inside the bounds from every moving state is not proven
  in general here: it is checked on every plan (P4/P7 on every flat piece, P7 on every downward step, D5 after every
  X5 run; 10 073 downward steps and 11 032 landings after X5 in the suite alone, and every sweep), with no miss. The
  meet's descent is a landing in the absolute frame onto a point on the line it meets.
- **P8:** there is no calculation to refuse, so the easer is never held while every input is valid.

The model's runs (`erg_slew.py --report`, every check on every tick of every plan), with no violation:
- Random plans of round 7's generator (2–8 blocks over the tuning grid; steps and ramps within, at and beyond the
  rates, entered from the line and by jumps; blocks of 0.25–60 s; ± offsets, cap changes, skips and engagements;
  grid-exact steps Δ = rise·ease; ramps split into blocks with float noise; clamp then drop; ramps from exactly 0 W):
  the suite's 2 000 (seed 585) and the sweeps below.
- **Inputs not a number** (round 19's generator, `invalid_plan`; it replaces rounds 13–18's fault injection): 2 000
  random plans with 1–4 invalid inputs each (offsets NaN or ±∞ for 1–80 ticks or to the end, caps NaN or −∞ until a
  valid one, engagements NaN or ±∞): X5 exactly on those ticks (P8), D1–D5.
- X5 windows (the offset not a number for 1–40 ticks) at random places in 2 × 300 random plans (the falsifiers' seeds
  56000 + i and 70000 + i), on 400 planned falls, and around 400 cap events (round 13's scheme); X5 from a random tick
  to the end on 2 × 300 random plans (incl. the round-7 falsifier's q4v7); D5 after X5 over every grid tuning (9 792
  runs). These are rounds 13–18's refusal suites with the refusal replaced by an invalid input.
- The line's tick-independence and canonical form: 2 000 random plans (round 10).
- The falsifiers' repros as plans (f625b, f625c, f625d: 38 plans, incl. p3–p6), track 2's segfault plan, and every
  finding of rounds 18 and 19 (PR #693, PR #712's fix rounds, f712c's F1–F5: 20 plans), plain: no cut without an
  event, never held.
- Δ = rise·ease steps (1×, 2×, 3×, both directions) over every 35th of the 6 955 valid tunings: 1 068 plans.
- Split ramps against the single block, tick for tick: 300 plans, difference 0 W.
- Golden traces: Σ y, off-line ticks and settle ticks of 54 plans, pinned (`--golden`, regenerated in round 19), incl.
  X5 traces and every finding above. They pin what the properties leave free: the lead's timing and cap, the chase,
  the will-rise test, the landing's profile from a moving state, the meet.
- Ramp-to-ramp vertices: 1 421 slope pairs at the defaults and the 8 bound corners.
- 3 000 held-line cases, 80 runs at the corners of the bounds, and the P7 sweep: slopes from 10⁻⁴ to 0.99 of either
  rate, rising and falling, over the defaults, the corners and a sixth of the grid (2 064 plans).
- The closed form on its own: the landing on 20 000 random inputs, the monotone branch on 4 000, the meet on 3 000
  (`constants()`), and `profile_extrema` against dense sampling on 3 000.
- Pinned regressions: the round-5 stuck steps and the stuck mid-plan case (refused by Ruckig, landing within P4/P7),
  the split ramps of round 5's blocker 3, the ramp-to-ramp cuts, the X4 plan, a fall that meets a rising ramp, a cap
  drop mid-fall, a falling ramp to 0 W then a jump, a ramp into the cap, twelve random plans by (seed, plan) that
  broke an earlier rule (the hold of step 5, the cap term of step 6), X5's mechanism (an offset not a number on one
  tick: held on that tick only, the status on that tick only), and the hard clamp itself.
- The figures this section quotes, pinned: a change that moves one fails the model.
- The sweeps of round 19 (below, §7.10.2's measurements).

The properties:

- **P1 crossing.** The command never moves up past the line (P5). It passes below the line only by easing out of a
  motion it was in when the line moved: below the plan. Otherwise the line passes the command: a jump, or a line
  steeper than the rider's rate.
- **P2 rate.** The sent slope `(y_k − y_{k−1})/Δt` is in `[−r↓, r↑]` on every tick, except X3's position step down to
  a lowered cap. The profiles' slopes stay within the rates; the cuts only move the slope towards 0.
- **P3 ease.** On the sent positions, curvature ≤ `a_max` and jerk ≤ `j_max` on every tick, except where X2–X4 break
  them, each within its domain and bound (§7.10.2's table). No un-eased corner without an unforeseeable event, also
  during X5.
- **P4 duration.** A rise onto a held line lands within `landing_bound`: `Δ/r↑ + τ + 0.75 s` from rest. On every plan
  outside X5; after X5, D5.
- **P5 no rise above the plan.** A command more than ε above the line never rises. No step ends more than ε above the
  line higher than it started.
- **P6 excess and bounds.** `max(0, y − R − ε)` grows only on a tick where the line falls (a downward jump, a falling
  ramp, the clamp), and `0 ≤ y ≤ max(M, 0)` after every tick (a cap below 0 W is 0 W).
- **P7 time above the line after a downward step, on every line** (round 14). After a downward step (a skip, ±/coach,
  a cap, or a jump down of the line), a command above the line is at or below it:
  - onto a held line within `landing_bound`, `Δ/r↓ + τ·(1 + f)/2 + 0.75 s` from rest;
  - onto a line of slope `s` strictly within the rates, rising or falling, within `ramp_bound`. It closes the gap at
    the real rate `r↓ + s`, which is never below `r↓ − |s|` and is finite as `s → 0±`.

  The rider or coach lowering the load takes effect in bounded time, whatever the line's slope. On every plan outside
  X5; after X5, D5. The model checks it on every downward step (`check_above`), and a sweep covers slopes from `10⁻⁴`
  to 0.99 of either rate. Round 19 restores it on every line strictly within the rates (fix round 2 of round 18 had
  given it up above a rising line near r↑, where Ruckig refused the meet).
- **P8 never held while the input is valid** (round 19, replacing rounds 18's honest promise). The easer is held
  (X5) on exactly the ticks whose plan, cap or state is not a number, and `erg_input_invalid` is raised on exactly
  those ticks. On a plan whose every input is valid, it never holds and the status is never raised. Checked on every
  trace (`check_status`), fault-free and with invalid inputs. Rounds 18's P8 allowed degraded episodes of up to 2 s
  without a fault, because Ruckig's refusals at a state's numerical edge were not provably absent; with no library
  there is nothing to refuse.

**X5's own promises** (rounds 13–17's degraded mode, now the input layer's hold; §7.10.2, X5). While an input is not a
number there is no promise to follow the plan, and these hold instead, for any pattern of invalid inputs from any tick
and state:
- **D1 limits.** P2 and P3 on the sent positions, with no cut of X5's own: X2–X4 only in their domain.
- **D2 bounds.** `0 ≤ y ≤ max(M, 0)` after every tick, `M` the last valid cap; the output is a finite number.
- **D3 no rise above the line.** A command more than ε above the line never rises (P5), and the excess grows only where
  the line falls (P6).
- **D4 the stop.** From its first tick, the command follows the stop from the state it had (so it travels at most that
  stop's distance), then holds at rest; until the input is valid again, an unforeseeable event, an engagement or a
  cut. The status is raised on every such tick (P8).
- **D5 the landing after it.** On the first tick whose input is valid, normal planning resumes. From the state at that
  tick, the command is on a held line (a rise) or at or below it (a fall) within `landing_bound`, and on a ramp
  strictly within the rates within `ramp_bound`, if the piece and the plan last that long with no further event or
  invalid tick. `ramp_bound` is the same bound in the line's frame, with three differences:
  - the closing rates are the real ones, `r↑ − s` from below and `r↓ + s` from above (round 14);
  - the catch-up's rate follows the sign of the gap left once the relative slope is stopped (the slower rate where
    that sign is not known);
  - where the line comes to a command waiting for it (a falling line above it, under safe braking) or the command
    meets it at rest (a rising line, step 5), an overshoot of at most `|s|·T_s/2`, with
    `T_s = |s|/a_max + a_max/j_max`, is closed at the opposite relative rate (round 13b's two cases, pinned).

**Claim: no eased target holds the rider above the plan for longer than the documented bound.**

1. *A rise never commands more than the plan.*
   - A rising command never ends a tick more than ε above the line (P5): safe braking (step 6) and the meet above a
     rising line (step 5, which never rises while above it), with X2 where an unforeseeable reversal leaves no room.
     X5 is the very stop safe braking accepted, so an invalid input adds no rise.
   - On the line, it is the line.
   - The lead into a vertex only lowers it.
2. *A fall never commands more than the rider was just riding, and ends in bounded time.*
   - When the line falls below the command, the command never rises (P5), so it stays at or below the load the rider
     was holding. Its excess never grows unless the line keeps falling (P6).
   - Onto a ramp, rising or falling, it is at or below the line within `ramp_bound` (P7), closing at `r↓ + s`.
   - Onto a held line it is at or below the line within `Δ/r↓ + τ·(1 + f)/2 + 0.75 s` (P7). With the bounds
     (`r↓ ≥ 20 W/s`, `τ ≤ 4 s`, `f ≤ 4`), that is at most `Δ/20 + 10.75 s`. For a 300 W drop the bound is 11.25 s at the
     defaults, and the model's trace takes 9.75 s.
   - Behind a falling ramp steeper than `r↓`, the command eases into `r↓` and falls at it, never above the load at the
     descent's start.
3. *The cap is never exceeded:* the cap is applied before and after every tick (X3, P6).
4. *Stopping never waits.* Pause, release, end and every non-ERG block are mode changes sent on the tick they are
   decided (§7.10.5); ride end keeps §7.9 unchanged. A fading rider in a slow descent is released by §7.4 exactly as
   under a held target, since the release reads the last command.
5. *Engagement adds no torque before pedalling:* the `Erg { 0 }` hold and the origin rules of item 5 are unchanged,
   and the easer starts at the origin at rest.
6. *Nothing can fail to compute.* Every trajectory is closed form with a bounded root finder; there is no fallback
   path, so no seam between a planner and a fallback for a falsifier to find (rounds 10–18's recurring finding).

Against today, the change is a bounded extension of the previous block's load on a fall (item 2), and a slower rise.
The rider was riding the higher load the moment before, it only decreases, and every exit is immediate. The cost of
slow limits falls on the plan (short reps not reached), never on the rider's safety. The owner question of §7.10.4 is
about the workout, not the rider.

#### 7.10.7 What the safety falsifier must probe (#621)

1. **Stop paths.** Every path to `Resistance { 0 }` or `Free`, mid-ease and mid-engagement (pause, release, end,
   symptom stop, a `Free` block, a trainer loss):
   - the mode is desired on the same tick;
   - no later `Erg` write overtakes it (the newest mode wins the `control` slot, §4.4; the end mode ordering of §7.9).
2. **Reversals.** A skip into a lower block, a manual −, a coach decrease mid-rise, a jump down onto a rising ramp, a
   short block followed by a lower one. The command never rises while above the line. X2 appears only where an
   unforeseeable event leaves no room, within its bound.
3. **Caps.** A cap that appears or drops mid-ease or mid-ramp: the command is at or below it on that tick.
4. **Engagement:**
   - no torque before the first pedalling sample;
   - no origin from an old epoch;
   - a link epoch mid-ease resets the state;
   - a release during an engagement still fires on its dwell;
   - an engagement mid-plan above a falling line and below a rising one;
   - completion behind a line the command cannot follow.
5. **The properties at the bounds.** P1–P8, D1–D5 and the bounds of X2–X4 at the corners of the bounds. `validate`
   refuses a falling rate under 20 W/s, and an off-grid factor or ease.
6. **The closed form in the loop:**
   - re-plan only on a change, with no trajectory computed on an unchanged input;
   - the landing from any state: ends at its displacement, at rest, within the frame's rates, `a_max` and `j_max`;
     from rest, the time-optimal S-curve; a frame rate down to 10⁻⁹ W/s (a lead between near-equal slopes) still
     lands, by cruising;
   - the meet from above a rising line and below a falling one: at rest on the moving line, or the stop where the line
     passes the command;
   - the root finder: bounded steps, never leaves its bracket, deterministic;
   - the probes (the stop, the floor, the lead's timing), the brake and the chase are the closed-form slope change;
   - every finding of rounds 4–18's falsifiers that involved the library (its refusals, slower answers, the segfault
     inputs, degenerate and near-rate frames, the twin's limits) as a plain plan: no cut without an event, never held;
   - inputs that are not numbers anywhere in a plan (offsets, caps, engagements, blocks; windows of 1–80 ticks and to
     the end): X5 exactly on those ticks, D1–D5, the status on exactly those ticks;
   - the line returned for tick `k` is the same whichever tick asks;
   - no NaN or infinity for any input;
   - the golden vectors agree with `erg_slew.py` within 10⁻⁶.
7. **Following and writer.** Per-tick updates keep a healthy trainer `following` (the executable model, `just model`).
   The writer still serves the reserved poll on time during a long ramp (§4.4).
8. **Compliance.** Settle never ends before the command lands. An off-line tick is never counted, so a steep ramp, an
   eased corner or an unreached short block is excluded, never counted as a miss.
9. **Mutation.** Deleting or breaking any rule of §7.10.2 must fail the property tests, as it fails the model:
   `just slew-mutants` (`docs/adr/models/erg_slew_mutants.py`) breaks each rule in turn and requires a non-zero exit
   from the model. Round 19 rewrites the list for the closed form: 94 mutants. It deletes the 28 of Ruckig's machinery (the twin and its
   limits, the slow-answer rule and `answer_bound`, `landable` and the band, the fault gate, fault injection, the
   target-velocity guard, the on-limit routing, the chase and the brake through Ruckig). It keeps the 77 of crank's own
   rules (the frame, the line on the tick grid and its tolerances, the lead, the meet's rule, safe braking, the cuts and
   their bounds and domains, the input layer, the closed-form helpers, the checks), and retargets 8 to X5 and the meet.
   It adds 9: the landing without its cruise, its monotone branch's bracket inverted, the frame's cap ignored (the
   rider's rates in a lead), the rates not relative to the line, the stop's own displacement ignored, the meet
   ignoring the line's motion, the meet as the stop alone, X5 without X2 against the last line known, and `same_key`
   without equality first.
   - The run (`--random 400 --jobs 2`, 93 min): the model passes, and all 93 runnable mutants are killed, 86 on the
     first run. Three survived it, each a rule the closed form never needs on its own (its landings end on `M` and
     0 W to well within `ROUND_SNAP`, where Ruckig's ended 1 ulp outside): the snap at `M` and at 0 W, and safe
     braking's tolerance at `M`. `constants()` now pins each directly (a moving state 10⁻¹² W outside snapped with its
     slope and no cut; a stop 10⁻⁷ W past `M` refused). `same_key` without equality first was killed only by a pin
     too (`same_key(∞, ∞)`, and one step under no cap computed in at most 4 trajectories): re-planning every tick from
     the sampled state breaks no property with the closed form. Three texts moved with later edits and were
     re-targeted. Those 8 were re-run on the final model: all killed.
   - One is shown equivalent and listed, not run: a line exactly at the rate landed on instead of chased. There the
     landing frame's rate in that direction is 0, so the landing is the stop in the frame, the slope change to the
     rate: the chase's own profile (it survived the full run).
   - The lead's timing from a moving state changes traces without breaking a property, so golden traces pin it (random
     56000 + 87), and X5 traces (planned falls with invalid-input windows). #620 runs a mutation tool
     (`cargo mutants`) over `crank_engine::slew` with the same requirement.

#### 7.10.8 HIL Part B checks 6, 7 and 12

`hil_fec` writes its procedures to the trainer itself through `crank-ble`; the engine's policy is not in its path.
The checks measure the trainer, and check 6 measures the trainer's own step response (`t90`), so its steps must stay
raw. **The Part B programs are unchanged**: `PROGRAM_6`, `PROGRAM_9` and `PROGRAM_12` of #521, and #616's check 12
(`Release, SoftStart(200), Wind, Track(0), Release`, ADR-0007 §4), keep their own soft start, ERG 0 W and then six
writes 1 s apart on a line. Today `rider::SOFT_START_STEPS` is derived from the engine's `SOFT_START_TICKS`; #621 makes
it hil_fec's own constant (5) when the engine's goes, so neither the programs nor ADR-0007's tests
(`the_soft_start_is_erg_0_then_six_steps_on_the_line_to_its_target`) change. Check 7 (no keep-alive, from the 150 W
write at the end of that soft start) is unaffected for the same reason. The eased command and the per-tick write rate
are checked on hardware by a new check in #624 (§7.10.1).

#### 7.10.9 Alternatives considered

| Alternative | Why rejected |
|---|---|
| Leave smoothing to the trainer (item 2 as it was) | The owner felt the step on the NEO (#585) |
| Close the loop on measured power (outer PID) | It fights the NEO's own ERG loop through an unmeasured inner response and a lagging page 25, and it raises the command when a rider fades, which is the spiral (§7.10.1) |
| **A hand-written limiter (rounds 1–3)** | Three rounds of formulas each exposed new edges: the rate at `g = 0`, un-eased ramp corners, the model and the ADR disagreeing on skips and mid-plan engagements. Round 19's easer differs in kind: one third-order profile family (slope change, cruise, stop) whose properties are self-tested, under rules (the frame, the lead, safe braking, the meet, the cuts) that rounds 4–18's falsifiers already hardened |
| A second-order limiter only (rate and acceleration, round 3) | Its acceleration jumps, and its ramp corners went 0 → 20 W/s in one tick (X1). A third-order trajectory (jerk-limited) eases every corner |
| **Ruckig (rounds 4–18), with its twin, degraded mode, the slow-answer rule, `landable` and the fault gate** | Time-optimal from any state, but a numerical library at its edges: refused inputs (#269), slower valid answers on its velocity boundary, a segfault, refusals on degenerate frames near the rate and between near-equal slopes. Fifteen rounds each fixed the last edge and found the next one; at `4b958c87` a fault-free plan still raised the fault status (f712c F4) and a fault-free episode lasted 17 ticks (F5). The owner: "Simple easing, no Ruckig." (2026-11-27). What it gave over the closed form is measured in §7.10.2 |
| `rsruckig` (the Rust port of Ruckig) for the engine | Goes with Ruckig. It also had no randomized tests, and presumably shared #269 |
| Ruckig Pro (position limits, tracking, waypoints) | Closed source and paid; position limits only bound a constant, while our plan line moves |
| Re-plan every tick | It makes the command depend on sampling and leaves the trajectory in force unvouched for by its probes; re-planning on a change keeps what the probes accepted (step 7). (Rounds 4–18: Ruckig also failed on its own sampled state at a velocity-limit boundary) |
| On an invalid input, hold the command at once | An instant hold is an un-eased corner. X5 holds only after the closed-form jerk-limited stop |
| Round 4's "X2 before the step" | Redundant: deleting it changed no property in any check (§7.10.2 step 8) |
| Use the new piece, extended back a tick, for the step into a vertex (round 4) | Put the command "ahead" of a steeper line: un-eased cuts at ramp-to-ramp vertices (2 701 slope pairs) |
| Track 2: a trajectory towards the line with relaxed promises (round 8) | No lead, braking, merge or cuts, only a hard clamp. Measured excess above the plan: 180 W at the defaults, 1 543 W at the worst tuning |
| Treat slopes 10⁻¹³ apart as a vertex (round 5) | A split ramp became a lead with a velocity cap the state was outside: a 47 W undershoot. Collinear lines are merged (§7.10.2) |
| Per-direction acceleration and jerk limits | A change of direction then changes the limits under a moving state; one pair for every move (§7.10.2, the mapping) |
| Lead every vertex, or centre every ease on its boundary | It raises the load before the plan does at a rise's start, and it is impossible for a skip, a ±, the coach or an edit. Only vertices where the lead lowers the load are led (step 3) |
| Keep easing through a reversal (no X2) | The command would keep rising after the plan lowered it (P5). Planned reversals are eased out before they happen (step 6); unforeseeable ones keep the cut |
| Ease the clamp at 0 W by going below it | There is nothing below 0 W; safe braking against 0 W (step 6) eases into it |
| Ease mode changes (pause, release, end) | It would delay the stop; stopping resistance must never wait on a ramp |
| A TS port of the easer in the app (#623 as it was) | A second implementation to keep identical with the core. The core sends the predicted trajectory (§7.10.5) |
| Feed the easer a ramp's 1 Hz staircase (round 1) | Every 1 s step became a discontinuity to ease: 59 of 60 s of a 5 W/s ramp were excluded from compliance, contradicting item 8 |
| Compress the ease into a share of short blocks (round 1's κ) | It overrides the rider's rates (κ = 21.6, 432 W/s, for a 5 s block), and the owner's rates are limits |
| Sample a ramp's last tick at its end value, or set `ŝ = 0` on every block's first tick | The ramp reads as steep (20.25 W/s for a 20 W/s ramp), or its end reads as a gap of `ŝ·Δt` (round 3's sampling table) |
| 1 Hz or 2 Hz eased updates | Staircase steps of 20–40 W (or 10–20 W) at the default rates; the writer already caps writes at 4 Hz and coalesces |
| An explicit falling rate instead of a factor | The owner described a factor; with `factor ≥ 1` a fall is never slower than a rise |
| An off switch, or unbounded rates | The engagement is the same mechanism and must stay a ramp |
| Compliance against the commanded (eased) target | The trainer lags an ease, so seconds would miss spuriously; compliance is about the plan |
| Counting off-line seconds against the plan | It punishes the smoothing the rider asked for |
| Stretch the block by the ease | It changes the workout's length and every later block's time |

#### 7.10.10 Implementation plan (sub-issues of #585, in order)

1. **#620** `crank_engine::slew`, retitled by round 19 "the closed-form easer (ADR-0002 §7.10.2)". Draft PR #693
   (crank-620) **drops `rsruckig` and becomes a port of the model's closed-form easer**:
   - **delete:** the `rsruckig` dependency (and its `deny.toml`/licence entries, if any) and every call to it; the twin
     (`TWIN`, the tightened limits, `limit_twins`); degraded mode on a refusal, `plan_failures`, the slow-answer rule,
     `answer_bound`/`feasible_landing`, `TwinLimits`; `trackable`/`landable` and the band; the target-velocity guard
     (`TARGET_SCALE`) and the on-limit routing; `FaultGate`, the episodes and their counters; fault injection
     (`inject`, `refuse_ticks`, `twin_answers`, the SplitMix injection draws) and `fail_brake`; the tests of all of
     these: the bare #269 input, the segfault inputs against the library, the twin's limits, the slow-twin and
     velocity-boundary pins, `landable`'s edge, `intermittent_refusals_keep_every_property_and_the_twin_alone_never_degrades`,
     the persistent-refusal and X5-mechanism tests, the fault gate's recurrence and episode tests;
   - **keep, unchanged:** `SlewTuning` (Default, BOUNDS, validate) and the mapping to `a_max`, `j_max`; the line on the
     tick grid (`segments`, `SLOPE_TOL`, `VALUE_TOL`, the raw join, `same_key`, which must compare equal values first:
     round 19's `+∞` fix); steps 1–3 and 5–8 (the cap, the
     frame, the lead with its timing and its cap, the meet's rule, safe braking with its slack, the meet from below one
     tick early, the brake as the stop, re-plan on a change with the frame as the key, the cuts and `ROUND_SNAP`); the
     closed-form helpers it already has (`slope_change`, `profile_at`, `profile_extrema`, the stop); X3 before the
     step (`_at_cap`); the input layer and X5 for inputs that are not numbers; `port_plan` and the harness's
     generators; the checks of P1–P7 and D1–D5, `landing_bound`, `ramp_bound`, `check_cap_floor`, `check_state`;
   - **add:** `landing` (slope change to a peak, cruise, stop), `meet_profile`, `root` (Illinois, 200 steps,
     `10⁻¹²` relative) as the model has them, with their self-tests (the model's `constants()`: the landing on random
     inputs, from rest the time-optimal S-curve, monotone in the peak, the meet at rest on the moving line); step 4 as
     "strictly within the rates is landed on, else chased"; the status `ergInputInvalid` on exactly the invalid ticks
     (P8 as round 19 states it: `check_status`); X5's cuts against the last line known (X2 on the tick an unforeseeable
     drop meets an invalid offset); the invalid-input generator (`invalid_plan`) and the X5 windows that replace the
     refusal windows;
   - **tests to change:** P8 asserts "held exactly on the invalid ticks" (fault-free: never held, the status never
     raised); every test of a refusal becomes the same plan as a plain plan or with an invalid-input window
     (`nan_window`, as the model converts them); the finding tests (`finding_1_…`, `finding_2_…`) run their plans
     plain and assert no cut without an event and never held;
   - **golden vectors:** regenerated from round 19's model (`erg_slew.py --golden`, 54 plans, incl. f712c's F1–F5
     and every earlier finding as a plain plan), agreeing within 10⁻⁶;
   - property tests P1–P8 and D1–D5 (under invalid inputs), the bounds of X2–X4, and the line's tick-independence, over
     the model's cases and random plans.

   No wiring.
2. **#621** The ERG policy:
   - evaluates ramps per tick (tick-start sampling, pieces, §7.10.2) and commands the eased target;
   - the soft start becomes the engagement;
   - settle as amended;
   - mode changes bypass the easer;
   - hil_fec gets its own soft-start constant;
   - §4.4.1 classes and the model;
   - FMEA rows, including "an input that is not a number" (X5) and "eased descent holds load above the plan";
   - golden engine traces (#357);
   - **safety falsifier review** (§7.10.7).
3. **#622** Stream and record the commanded target (`target`), `easingToW`, `ergInputInvalid`, and the **predicted
   trajectory** message on the ui channel (a per-second polyline sent on every re-plan); bindings.
4. **#623** The app: the power trace draws the recorded target behind "now" and the predicted trajectory ahead. Also
   the TARGET sub-line, the banners (including the invalid-input fault), and the mock fed from the golden traces
   (#357). No port of the easer.
5. **#624** HIL:
   - eased steps, a long ramp, and page-71 echo latency under per-tick writes on the NEO;
   - owner rides (a steady and a short-interval workout) to tune the defaults;
   - a contradiction amends this section.
6. **#627** (pending the owner's answer on PR #625) An editor warning for a block the rider's tuning cannot reach.
7. **#634** Run the model in CI: with no Ruckig it needs only Python's standard library, so it can be a flake check
   (its default run takes about 2 minutes on one core; a reduced `--random` count fits a check).

The Settings rows (rise, fall factor, ease) are #595, beside the release rows.

## 8. Decision: macOS TCC and the dev-time BLE path

- **App:** a real bundle from day 1: `crank.app` (bundle id `io.github.gerchowl.crank`) with
  `NSBluetoothAlwaysUsageDescription` = "crank connects to your smart trainer and heart-rate strap over
  Bluetooth.", assembled by Nix and signed by `just bundle` on [mac-studio] with the self-signed `nix-tcc`
  identity (ADR-0001 §9.3, #37). TCC keys the grant on the designated requirement (bundle id + signing
  certificate), so the owner grants Bluetooth once and it survives rebuilds; ad-hoc-signed builds lose
  it on every rebuild (`spec/TRAINER-BLE.md`). Developer ID signing is for distribution only (#47).
- **CLI / HIL scripts:** the grant belongs to the terminal app that launched the process (TCC responsible
  process). On [mac-studio], flock panes are denied (responsible process `sshd-keygen-wrapper`), so hardware runs
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
