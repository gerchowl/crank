//! The stream wire types export to TypeScript the way `export-bindings` does
//! (`tauri-specta`'s default `specta_serde::PhasesFormat`). specta-typescript forbids
//! `u64`; ids, `seq` and `late_samples` are `u32` for that reason (#271).
#![cfg(feature = "specta")]
use crank_engine::{StreamFrame, StreamId, StreamSnapshot, StreamStatus, SubId, SubscribeOpts};
use specta::Types;
use specta_typescript::Typescript;

#[test]
fn stream_types_export_to_typescript() {
    let types = Types::default()
        .register::<StreamId>()
        .register::<SubId>()
        .register::<SubscribeOpts>()
        .register::<StreamFrame>()
        .register::<StreamStatus>()
        .register::<StreamSnapshot>();
    let ts = Typescript::default()
        .export(&types, specta_serde::PhasesFormat)
        .unwrap_or_else(|e| panic!("TypeScript export failed: {e}"));
    for name in [
        "StreamFrame",
        "StreamStatus",
        "StreamSnapshot",
        "SubscribeOpts",
    ] {
        assert!(ts.contains(name), "{name} missing");
    }
}
