# FE-C capture replay

`.fecap` is UTF-8 text, one ANT notification/write per record:

```text
# elapsed_us direction complete_frame_hex
0 rx A4 09 4E 05 10 19 04 00 00 00 FF 24 30
```

`elapsed_us` is an unsigned 64-bit count of microseconds from capture start,
monotonically nondecreasing (ties preserve file order). `rx` means trainer →
host (`fec2` notification); `tx` means host → trainer (`fec3` write).
Remaining whitespace-separated tokens are hexadecimal bytes. Blank lines and
`#` comments (including trailing comments) are allowed. Preserve each complete
GATT payload, even when its ANT framing/checksum is invalid. Never combine or
split notifications. This format records both directions and timing as required
by ADR-0002 §9. Decoding is immediate and deterministic; replay does not sleep.

`tests/replay.rs` loads every `.fecap`, passes each record through `decode`, and
compares it with its same-basename `.expected` typed snapshot. Snapshots include
arrival time, direction, full typed frame or framing error, and unknown raw bytes.
Write expectations independently from the layout; do not regenerate them from
the decoder under test. Errors remain records so subsequent frames still replay.
This is codec replay; the fake BLE transport/ReplayTrainer belongs to #43.

These fixtures are **synthetic**, not evidence of NEO behaviour:

- `tacx-worked`: Tacx FE-C over BLE v1.0.0 worked example in ADR-0002 §4.1.
- `synthetic`: hand-computed ADR-0002 §4.2 vectors for all six telemetry pages,
  time/distance/event counter rollover, invalid sensors, lap/status flags,
  raw pages 17/19, and an outgoing page 49. For example, `39 30` is 12345 mm/s,
  `BC 7A` is power 0xABC = 2748 W plus status 7, and `34 12` is 4660 N.

Real NEO captures will be added by [#44](https://github.com/gerchowl/crank/issues/44),
including confirmation of the tentative pages 54/80/81 layouts. Pages 17/19 remain
unknown pending [#139](https://github.com/gerchowl/crank/issues/139) and an ADR amendment.
