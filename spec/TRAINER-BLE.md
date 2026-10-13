# crank — trainer BLE link

Probed 2026-10-13 against the user's NEO (gen 1) from macOS with bleak
(scan + GATT dump + Cycling Power notify).

## Device as found

- Advertises as `Tacx Neo 55624`.
- Firmware 0.3.2, updated via Tacx Utility (iOS) to **0.7.4**. Hardware rev `0`.
- **No FTMS (0x1826) on either firmware.** The GATT list did refresh after the
  update (new firmware string, new 2A64), so this is not a stale CoreBluetooth cache.

| Service | Characteristics | Use in crank |
|---|---|---|
| `6e40fec1-b5a3-f393-e0a9-e50e24dcca9e` Tacx FE-C over BLE | `…fec2` notify (trainer → app), `…fec3` write / write-without-response (app → trainer) | **Trainer control + trainer data** |
| `0x1818` Cycling Power | 2A63 Measurement (notify), 2A64 Vector (notify, 0.7.4 only), 2A65 Feature, 2A5D Location, 2A66 Control Point | Power/cadence telemetry (standard, works today) |
| `0x1816` Cycling Speed & Cadence | 2A5B Measurement (notify), 2A5C Feature, 2A5D Location | Speed/cadence fallback |
| `0x180A` Device Information | manufacturer, model, serial, hw/fw revision | Equipment screen, diagnostics |
| `669aa501-0c08-969e-e211-86ad5062675f` | `669aac01…` read/write | Unknown Tacx service — do not touch |

Cycling Power notifications reach the Mac (verified, hand-rotated flywheel → 0–1 W).

## Decision

Trainer control goes through a `Trainer` trait in the Rust core with **FE-C over BLE
as the first (and for this NEO, only) control backend**. FTMS becomes a second
backend for other trainers, not the primary path. Telemetry may come from FE-C
page 25 or from Cycling Power 2A63; pick one source per ride, never mix.

## FE-C over BLE (to verify against Tacx's "FE-C over BLE" doc before coding)

- Payload on fec2/fec3 is an ANT message: `A4 | len=09 | 4E (ack/broadcast data) | ch=05 | 8-byte FE-C page | XOR checksum`.
- Pages needed for v1:
  - 16 general FE data, 25 specific trainer data (power, cadence, trainer status) — from trainer
  - 49 target power (ERG; 0.25 W units)
  - 51 track resistance (SIM; grade in 0.01 % with 0x4E20 offset, rolling-resistance coefficient)
  - 50 wind resistance, 55 user configuration (rider + bike mass, wheel size)
  - 70 request data page (to read e.g. capabilities page 54)
- Expected NEO behaviour to confirm empirically: one central connection only;
  ERG target held across cadence changes; how quickly it applies a target change.

## macOS permissions (dev)

Bluetooth access is TCC-gated per *responsible process*. nix-built apps are
ad-hoc signed, so a grant is lost on every rebuild. For dev probing, run BLE
scripts from a terminal that has the grant; the durable path is the crank `.app`
signed with an Apple Development identity plus `NSBluetoothAlwaysUsageDescription`.
