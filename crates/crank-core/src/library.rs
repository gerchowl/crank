//! The built-in workout library: DSL text parsed and compiled at start-up. User workouts
//! (store-backed library and editor) are #110/#111; until then this is the MVP's source of
//! rideable workouts (#256).
use crate::dto::{CompiledView, WorkoutDetail, WorkoutSummary};
use crate::error::{ApiError, ErrorCode};
use crank_domain::workout::{
    CompileOptions, CompiledWorkout, Severity, Workout, compile_workout, parse_dsl, print_dsl,
};

/// Reference FTP of the built-in workouts, W. Athlete inputs are #247.
const FTP_W: f64 = 250.0;

/// `(id, name, dsl)`.
const BUILTIN: [(&str, &str, &str); 3] = [
    (
        "builtin-sensor-check",
        "Sensor check (3 min)",
        "\"Warm-up\" 1m 50%, \"Work\" 1m 80%, \"Cool-down\" 1m 40%",
    ),
    (
        "builtin-endurance-45",
        "Endurance 45",
        "\"Warm-up\" 5m ramp 40-60%, \"Endurance\" 35m 65%, \"Cool-down\" 5m ramp 60-40%",
    ),
    (
        "builtin-sweet-spot-2x10",
        "Sweet spot 2x10",
        "\"Warm-up\" 10m ramp 45-70%, 2x(\"Sweet spot\" 10m 90%, \"Rest\" 5m 55%), \"Cool-down\" 5m ramp 60-40%",
    ),
];

/// A parsed, compiled library entry.
pub(crate) struct Entry {
    pub(crate) workout: Workout,
    pub(crate) compiled: CompiledWorkout,
}

/// The library; entries are in display order.
pub(crate) struct Library(Vec<Entry>);
impl Library {
    /// Parse and compile every built-in.
    pub(crate) fn builtin() -> Result<Self, ApiError> {
        let internal = |id: &str, e: String| {
            ApiError::new(ErrorCode::Internal, format!("built-in workout {id}: {e}"))
        };
        BUILTIN
            .iter()
            .map(|(id, name, dsl)| {
                let mut n = 0_u32;
                let mut ids = || {
                    n += 1;
                    format!("{id}-n{n}")
                };
                let nodes = parse_dsl(dsl, FTP_W, &mut ids).map_err(|e| internal(id, e.message))?;
                let workout = Workout {
                    id: (*id).into(),
                    name: (*name).into(),
                    ftp: FTP_W,
                    nodes,
                };
                let compiled = compile_workout(&workout, &CompileOptions::default())
                    .map_err(|e| internal(id, e.to_string()))?;
                if let Some(issue) = compiled
                    .issues
                    .iter()
                    .find(|i| i.severity == Severity::Error)
                {
                    return Err(internal(id, issue.message.clone()));
                }
                Ok(Entry { workout, compiled })
            })
            .collect::<Result<_, _>>()
            .map(Self)
    }
    pub(crate) fn get(&self, id: &str) -> Result<&Entry, ApiError> {
        self.0
            .iter()
            .find(|e| e.workout.id == id)
            .ok_or_else(|| ApiError::new(ErrorCode::NotFound, format!("no workout {id:?}")))
    }
    pub(crate) fn summaries(&self) -> Vec<WorkoutSummary> {
        self.0
            .iter()
            .map(|e| WorkoutSummary {
                id: e.workout.id.clone(),
                name: e.workout.name.clone(),
                total_s: e.compiled.total_s,
                tss: e.compiled.tss,
                ftp: e.workout.ftp,
            })
            .collect()
    }
    pub(crate) fn detail(&self, id: &str) -> Result<WorkoutDetail, ApiError> {
        let e = self.get(id)?;
        Ok(WorkoutDetail {
            workout: e.workout.clone(),
            dsl: print_dsl(&e.workout.nodes),
            compiled: CompiledView::try_from(&e.compiled)?,
        })
    }
}
