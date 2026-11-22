//! The workout library (DESIGN §5.7, #110): crank's built-in workouts, parsed and compiled at
//! start-up and read-only, then the rider's own, stored (`crank-store`'s `workout` table) and
//! compiled when read. One library: `workouts_list`, `workout_get` and `ride_load` all read it.
use crate::dto::{CompiledView, WorkoutDetail, WorkoutDraft, WorkoutSummary};
use crate::error::{ApiError, ErrorCode};
use crank_domain::workout::{
    CompileOptions, CompiledWorkout, Severity, Workout, compile_workout, parse_dsl, print_dsl,
};
use crank_store::{Store, StoredWorkout, WorkoutTreeJson};
use std::time::{SystemTime, UNIX_EPOCH};

/// Reference FTP of the built-in workouts and of a new workout, W. Athlete inputs are #247.
const FTP_W: f64 = 250.0;
/// The longest workout name, in characters, once trimmed.
pub(crate) const WORKOUT_NAME_MAX: usize = 120;
/// The name a new workout starts with.
const NEW_NAME: &str = "New workout";
/// Every built-in id starts with it, and no stored id does: a stored id is `workout-<hex>`.
const BUILTIN_PREFIX: &str = "builtin-";

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
#[derive(Clone)]
pub(crate) struct Entry {
    pub(crate) workout: Workout,
    pub(crate) compiled: CompiledWorkout,
    pub(crate) builtin: bool,
}
impl Entry {
    fn summary(&self) -> WorkoutSummary {
        WorkoutSummary {
            id: self.workout.id.clone(),
            name: self.workout.name.clone(),
            total_s: self.compiled.total_s,
            tss: self.compiled.tss,
            ftp: self.workout.ftp,
            builtin: self.builtin,
        }
    }
}

/// The first error-severity issue of a compiled workout, as the refusal `ride_load` and
/// `workout_save` answer with.
pub(crate) fn refuse_errors(compiled: &CompiledWorkout) -> Result<(), ApiError> {
    match compiled
        .issues
        .iter()
        .find(|i| i.severity == Severity::Error)
    {
        Some(issue) => Err(ApiError::new(
            ErrorCode::InvalidInput,
            issue.message.clone(),
        )),
        None => Ok(()),
    }
}

/// A stored workout, compiled. A stored tree that no longer compiles (only a newer, stricter
/// domain can make one: saving refuses it) is a store failure, as a row that does not parse.
fn stored(s: &StoredWorkout) -> Result<Entry, ApiError> {
    let workout = s.tree.workout().clone();
    let compiled = compile_workout(&workout, &CompileOptions::default()).map_err(|e| {
        ApiError::new(
            ErrorCode::Storage,
            format!("stored workout {:?} does not compile: {e}", workout.id),
        )
    })?;
    Ok(Entry {
        workout,
        compiled,
        builtin: false,
    })
}

/// The built-ins; the rider's workouts are read from the store on each call.
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
                refuse_errors(&compiled).map_err(|e| internal(id, e.message))?;
                Ok(Entry {
                    workout,
                    compiled,
                    builtin: true,
                })
            })
            .collect::<Result<_, _>>()
            .map(Self)
    }
    fn is_builtin(id: &str) -> bool {
        id.starts_with(BUILTIN_PREFIX)
    }
    /// The entry for `id`: a built-in, else the rider's.
    ///
    /// # Errors
    /// `NotFound`; `Storage` for a stored row that does not read or compile.
    pub(crate) fn get(&self, store: &Store, id: &str) -> Result<Entry, ApiError> {
        let not_found = || ApiError::new(ErrorCode::NotFound, format!("no workout {id:?}"));
        if Self::is_builtin(id) {
            return self
                .0
                .iter()
                .find(|e| e.workout.id == id)
                .cloned()
                .ok_or_else(not_found);
        }
        store
            .workout(id)?
            .map_or_else(|| Err(not_found()), |s| stored(&s))
    }
    /// The built-ins in their order, then the rider's workouts in the order first saved.
    ///
    /// # Errors
    /// `Storage` for a stored row that does not read or compile.
    pub(crate) fn summaries(&self, store: &Store) -> Result<Vec<WorkoutSummary>, ApiError> {
        let mut out: Vec<_> = self.0.iter().map(Entry::summary).collect();
        for s in store.workouts()? {
            out.push(stored(&s)?.summary());
        }
        Ok(out)
    }
    /// One workout with its canonical text and compiled view.
    ///
    /// # Errors
    /// As [`Library::get`]; `Internal` if a block index exceeds `u32`.
    pub(crate) fn detail(&self, store: &Store, id: &str) -> Result<WorkoutDetail, ApiError> {
        let e = self.get(store, id)?;
        Ok(WorkoutDetail {
            dsl: print_dsl(&e.workout.nodes),
            compiled: CompiledView::try_from(&e.compiled)?,
            workout: e.workout,
            builtin: e.builtin,
        })
    }
    /// The draft a new workout starts from: no steps, the reference FTP.
    pub(crate) fn new_draft() -> WorkoutDraft {
        WorkoutDraft {
            name: NEW_NAME.into(),
            ftp: FTP_W,
            nodes: Vec::new(),
        }
    }
    /// Store `draft` as the rider's workout `id` (an update), or as a new one when `id` is
    /// `None`; answers the stored id. The name is trimmed.
    ///
    /// # Errors
    /// `InvalidInput` for a blank or too long name, a built-in id, a tree that does not
    /// compile, or an error-severity issue (its message); `NotFound` for
    /// an id that is no stored workout; `Storage` when the store fails.
    pub(crate) fn save(
        store: &Store,
        id: Option<&str>,
        draft: WorkoutDraft,
        now: i64,
    ) -> Result<String, ApiError> {
        let name = draft.name.trim();
        if name.is_empty() {
            return Err(ApiError::new(
                ErrorCode::InvalidInput,
                "a workout needs a name",
            ));
        }
        if name.chars().count() > WORKOUT_NAME_MAX {
            return Err(ApiError::new(
                ErrorCode::InvalidInput,
                format!("a workout name is at most {WORKOUT_NAME_MAX} characters"),
            ));
        }
        if let Some(id) = id.filter(|id| Self::is_builtin(id)) {
            return Err(Self::read_only(id));
        }
        let workout = Workout {
            id: id.map_or_else(new_id, str::to_owned),
            name: name.into(),
            ftp: draft.ftp,
            nodes: draft.nodes,
        };
        refuse_errors(&compile_workout(&workout, &CompileOptions::default())?)?;
        let dsl = print_dsl(&workout.nodes);
        let tree = WorkoutTreeJson::new(&workout)?;
        let saved = workout.id;
        if id.is_some() {
            if !store.update_workout(tree, dsl)? {
                return Err(ApiError::new(
                    ErrorCode::NotFound,
                    format!("no workout {saved:?}"),
                ));
            }
        } else {
            store.insert_workout(tree, dsl, now)?;
        }
        Ok(saved)
    }
    /// Delete the rider's workout `id`.
    ///
    /// # Errors
    /// `InvalidInput` for a built-in id; `NotFound` for an id that is no stored workout;
    /// `Storage` when the store fails.
    pub(crate) fn delete(store: &Store, id: &str) -> Result<(), ApiError> {
        if Self::is_builtin(id) {
            return Err(Self::read_only(id));
        }
        if store.delete_workout(id)? {
            Ok(())
        } else {
            Err(ApiError::new(
                ErrorCode::NotFound,
                format!("no workout {id:?}"),
            ))
        }
    }
    fn read_only(id: &str) -> ApiError {
        ApiError::new(
            ErrorCode::InvalidInput,
            format!("workout {id:?} is built in and read-only: duplicate it to change it"),
        )
    }
}

/// A fresh stored-workout id, `workout-<hex nanoseconds>` as ride ids are; the store refuses a
/// taken one rather than overwrite it.
fn new_id() -> String {
    format!(
        "workout-{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    )
}
