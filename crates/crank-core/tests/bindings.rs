//! Every command DTO exports to TypeScript in the format crank-desktop's bindings use
//! (`tauri-specta` default: `specta_serde::PhasesFormat`), so a type specta cannot
//! express fails here, in the crate that owns it, not when the desktop is wired (#34).
use crank_core::{
    ApiError, CoreInfo, DeviceLinkInfo, PairedDevice, RideLoaded, RideRecord, ScannedDevice,
    WorkoutDetail, WorkoutSummary,
};
use crank_engine::{StreamFrame, StreamId, StreamSnapshot, StreamStatus, SubId, SubscribeOpts};
use specta::Types;
use specta_typescript::Typescript;

#[test]
fn every_command_dto_exports_to_typescript() {
    let types = Types::default()
        .register::<ApiError>()
        .register::<CoreInfo>()
        .register::<ScannedDevice>()
        .register::<PairedDevice>()
        .register::<DeviceLinkInfo>()
        .register::<WorkoutSummary>()
        .register::<WorkoutDetail>()
        .register::<RideLoaded>()
        .register::<RideRecord>();
    let ts = Typescript::default()
        .export(&types, specta_serde::PhasesFormat)
        .unwrap_or_else(|e| panic!("TypeScript export failed: {e}"));
    for name in [
        "ApiError",
        "CoreInfo",
        "WorkoutDetail",
        "WorkoutBlock",
        "RideRecord",
    ] {
        assert!(ts.contains(name), "{name} missing from the export");
    }
}

/// The wire types of the stream channel are `crank-engine`'s. They hold `u64` ids and
/// counters, which specta-typescript forbids; the engine moves them to `u32` in a
/// separate PR (#271), which un-ignores this test.
#[test]
#[ignore = "blocked on #271: u64 stream ids/seq are BigInt-forbidden"]
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
    assert!(ts.contains("StreamFrame"));
}
