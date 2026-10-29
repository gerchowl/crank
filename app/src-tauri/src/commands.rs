//! The Tauri commands: one thin wrapper per `crank-core` facade method (ADR-0001 §4).
//! They hold no logic; the window label of the caller is the only thing added, and
//! [`ApiError`] is the only error that crosses.
use crank_core::{
    ApiError, Core, CoreInfo, DeviceLinkInfo, DeviceRole, FrameSink, PairedDevice, RideEnded,
    RideLoaded, RideRecord, RideStatusView, ScannedDevice, SinkClosed, WorkoutDetail,
    WorkoutSummary,
};
use crank_engine::{MetricId, StreamFrame, StreamId, StreamSnapshot, SubId, SubscribeOpts};
use tauri::{State, ipc::Channel};

/// Frames go to the window's IPC channel; a failed send ends the stream.
struct ChannelSink(Channel<StreamFrame>);
impl FrameSink for ChannelSink {
    fn send(&self, frame: StreamFrame) -> Result<(), SinkClosed> {
        self.0.send(frame).map_err(|_| SinkClosed)
    }
}

/// Versions and mode of the core.
///
/// # Errors
/// As [`Core::core_ready`].
#[tauri::command]
#[specta::specta]
pub async fn core_ready(core: State<'_, Core>) -> Result<CoreInfo, ApiError> {
    core.core_ready().await
}

/// Scan for trainers and straps.
///
/// # Errors
/// As [`Core::devices_scan`].
#[tauri::command]
#[specta::specta]
pub async fn devices_scan(
    core: State<'_, Core>,
    for_ms: Option<u32>,
) -> Result<Vec<ScannedDevice>, ApiError> {
    core.devices_scan(for_ms).await
}

/// Pair a device in a role.
///
/// # Errors
/// As [`Core::device_pair`].
#[tauri::command]
#[specta::specta]
pub async fn device_pair(
    core: State<'_, Core>,
    id: String,
    role: DeviceRole,
) -> Result<PairedDevice, ApiError> {
    core.device_pair(id, role).await
}

/// Forget the pairing of a role.
///
/// # Errors
/// As [`Core::device_forget`].
#[tauri::command]
#[specta::specta]
pub async fn device_forget(core: State<'_, Core>, role: DeviceRole) -> Result<(), ApiError> {
    core.device_forget(role).await
}

/// Pairings and live links.
///
/// # Errors
/// As [`Core::device_links`].
#[tauri::command]
#[specta::specta]
pub async fn device_links(core: State<'_, Core>) -> Result<Vec<DeviceLinkInfo>, ApiError> {
    core.device_links().await
}

/// The workout library.
///
/// # Errors
/// As [`Core::workouts_list`].
#[tauri::command]
#[specta::specta]
pub async fn workouts_list(core: State<'_, Core>) -> Result<Vec<WorkoutSummary>, ApiError> {
    core.workouts_list().await
}

/// One workout with its compiled view.
///
/// # Errors
/// As [`Core::workout_get`].
#[tauri::command]
#[specta::specta]
pub async fn workout_get(core: State<'_, Core>, id: String) -> Result<WorkoutDetail, ApiError> {
    core.workout_get(id).await
}

/// Load a workout, or a free ride for `None`.
///
/// # Errors
/// As [`Core::ride_load`].
#[tauri::command]
#[specta::specta]
pub async fn ride_load(
    core: State<'_, Core>,
    workout_id: Option<String>,
) -> Result<RideLoaded, ApiError> {
    core.ride_load(workout_id).await
}

/// Start the loaded ride.
///
/// # Errors
/// As [`Core::ride_start`].
#[tauri::command]
#[specta::specta]
pub async fn ride_start(core: State<'_, Core>) -> Result<(), ApiError> {
    core.ride_start().await
}

/// Pause the ride.
///
/// # Errors
/// As [`Core::ride_pause`].
#[tauri::command]
#[specta::specta]
pub async fn ride_pause(core: State<'_, Core>) -> Result<(), ApiError> {
    core.ride_pause().await
}

/// Resume a paused ride.
///
/// # Errors
/// As [`Core::ride_resume`].
#[tauri::command]
#[specta::specta]
pub async fn ride_resume(core: State<'_, Core>) -> Result<(), ApiError> {
    core.ride_resume().await
}

/// Skip to the next block.
///
/// # Errors
/// As [`Core::ride_skip`].
#[tauri::command]
#[specta::specta]
pub async fn ride_skip(core: State<'_, Core>) -> Result<(), ApiError> {
    core.ride_skip().await
}

/// End the ride: the id of the finished ride (`null` for an unloaded one) and whether the
/// core had to end it by force because the engine could not.
///
/// # Errors
/// As [`Core::ride_end_outcome`].
#[tauri::command]
#[specta::specta]
pub async fn ride_end(core: State<'_, Core>) -> Result<RideEnded, ApiError> {
    core.ride_end_outcome().await.map(Into::into)
}

/// The ride's state.
///
/// # Errors
/// As [`Core::ride_status`].
#[tauri::command]
#[specta::specta]
pub async fn ride_status(core: State<'_, Core>) -> Result<RideStatusView, ApiError> {
    core.ride_status().await
}

/// Open the calling window's multiplexed stream on the loaded ride.
///
/// # Errors
/// As [`Core::open_stream`].
#[tauri::command]
#[specta::specta]
pub async fn open_stream<R: tauri::Runtime>(
    core: State<'_, Core>,
    window: tauri::Window<R>,
    channel: Channel<StreamFrame>,
) -> Result<StreamId, ApiError> {
    core.open_stream(Some(window.label().to_owned()), ChannelSink(channel))
        .await
}

/// Subscribe to a metric on a stream of the calling window.
///
/// # Errors
/// As [`Core::subscribe`].
#[tauri::command]
#[specta::specta]
pub async fn stream_subscribe<R: tauri::Runtime>(
    core: State<'_, Core>,
    window: tauri::Window<R>,
    stream: StreamId,
    metric: MetricId,
    opts: SubscribeOpts,
) -> Result<SubId, ApiError> {
    core.subscribe(Some(window.label()), stream, metric, opts)
        .await
}

/// End a subscription of a stream of the calling window.
///
/// # Errors
/// As [`Core::unsubscribe`].
#[tauri::command]
#[specta::specta]
pub async fn stream_unsubscribe<R: tauri::Runtime>(
    core: State<'_, Core>,
    window: tauri::Window<R>,
    stream: StreamId,
    sub: SubId,
) -> Result<(), ApiError> {
    core.unsubscribe(Some(window.label()), stream, sub).await
}

/// The window applied every frame up to `seq`.
///
/// # Errors
/// As [`Core::stream_ack`].
#[tauri::command]
#[specta::specta]
pub async fn stream_ack<R: tauri::Runtime>(
    core: State<'_, Core>,
    window: tauri::Window<R>,
    stream: StreamId,
    seq: u32,
) -> Result<(), ApiError> {
    core.stream_ack(Some(window.label()), stream, seq).await
}

/// Latest sample of every subscription, the status and the links.
///
/// # Errors
/// As [`Core::stream_snapshot`].
#[tauri::command]
#[specta::specta]
pub async fn stream_snapshot<R: tauri::Runtime>(
    core: State<'_, Core>,
    window: tauri::Window<R>,
    stream: StreamId,
) -> Result<StreamSnapshot, ApiError> {
    core.stream_snapshot(Some(window.label()), stream).await
}

/// Stored rides, newest first.
///
/// # Errors
/// As [`Core::rides_list`].
#[tauri::command]
#[specta::specta]
pub async fn rides_list(core: State<'_, Core>) -> Result<Vec<RideRecord>, ApiError> {
    core.rides_list().await
}

/// One stored ride with its summary.
///
/// # Errors
/// As [`Core::ride_summary`].
#[tauri::command]
#[specta::specta]
pub async fn ride_summary(core: State<'_, Core>, id: String) -> Result<RideRecord, ApiError> {
    core.ride_summary(id).await
}
