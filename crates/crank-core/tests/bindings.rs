//! Every command DTO exports to TypeScript in the format crank-desktop's bindings use
//! (`tauri-specta` default: `specta_serde::PhasesFormat`), so a type specta cannot
//! express fails here, in the crate that owns it, not when the desktop is wired (#34).
use crank_core::{
    ApiError, CoreInfo, DeviceLinkInfo, PairedDevice, ScannedDevice, WorkoutDetail, WorkoutSummary,
};
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
        .register::<WorkoutDetail>();
    let ts = Typescript::default()
        .export(&types, specta_serde::PhasesFormat)
        .unwrap_or_else(|e| panic!("TypeScript export failed: {e}"));
    for name in ["ApiError", "CoreInfo", "WorkoutDetail", "WorkoutBlock"] {
        assert!(ts.contains(name), "{name} missing from the export");
    }
}
