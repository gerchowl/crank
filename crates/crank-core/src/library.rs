//! The workout library (DESIGN §5.7, #110): crank's built-in workouts, parsed and checked at
//! start-up and read-only, then the rider's own, stored (`crank-store`'s `workout` table). One
//! library: `workouts_list`, `workout_get` and `ride_load` all read it.
//!
//! **FTP (ADR-0009 §2.1.4).** Workouts are in % FTP and are compiled when read, at the FTP the
//! caller passes: the athlete model's current FTP for the library and the editor's previews
//! (labelled with it, `CompiledView::ftp_w`), the ride's frozen snapshot for `ride_load`. A
//! stored tree's own `ftp` (`workout.ftp_at_creation`) is informative only.
use crate::dto::{CompiledView, WorkoutDetail, WorkoutDraft, WorkoutSummary};
use crate::error::{ApiError, ErrorCode};
use crank_domain::athlete::{AthleteParam, prior};
use crank_domain::workout::{
    CompileOptions, CompiledWorkout, Severity, Workout, compile_workout, parse_dsl, print_dsl,
};
use crank_store::{Store, StoredWorkout, WorkoutTreeJson};
use std::time::{SystemTime, UNIX_EPOCH};

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

/// A library entry, compiled at one FTP: `workout.ftp` is the FTP it was compiled at.
#[derive(Clone)]
pub(crate) struct Entry {
    pub(crate) workout: Workout,
    pub(crate) compiled: CompiledWorkout,
    pub(crate) builtin: bool,
}
impl Entry {
    /// Compile `workout` at `ftp`, which replaces its own.
    ///
    /// # Errors
    /// [`crank_domain::workout::CompileError`] as `InvalidInput`.
    pub(crate) fn at(mut workout: Workout, ftp: f64, builtin: bool) -> Result<Self, ApiError> {
        workout.ftp = ftp;
        let compiled = compile_workout(&workout, &CompileOptions::default())?;
        Ok(Self {
            workout,
            compiled,
            builtin,
        })
    }
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

/// A stored workout, compiled at `ftp`. A stored tree that no longer compiles (only a newer,
/// stricter domain can make one: saving refuses it) is a store failure, as a row that does not
/// parse.
fn stored(s: &StoredWorkout, ftp: f64) -> Result<Entry, ApiError> {
    let workout = s.tree.workout().clone();
    let id = workout.id.clone();
    Entry::at(workout, ftp, false).map_err(|e| {
        ApiError::new(
            ErrorCode::Storage,
            format!("stored workout {id:?} does not compile: {}", e.message),
        )
    })
}

/// The FTP prior (ADR-0009 §2.1.2): the reference the built-ins are parsed and checked at once,
/// at start-up. Their steps are in % FTP, so the tree does not depend on it; every read compiles
/// them again at the caller's FTP.
fn reference_ftp() -> Result<f64, ApiError> {
    prior::prior(AthleteParam::Ftp, None)
        .map(|p| p.value)
        .ok_or_else(|| ApiError::new(ErrorCode::Internal, "FTP has no prior"))
}

/// The built-ins, parsed; the rider's workouts are read from the store on each call.
pub(crate) struct Library(Vec<Workout>);
impl Library {
    /// Parse every built-in, and check that it compiles without an error-severity issue at the
    /// FTP prior.
    pub(crate) fn builtin() -> Result<Self, ApiError> {
        let ftp = reference_ftp()?;
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
                let doc = parse_dsl(dsl, ftp, &mut ids).map_err(|e| internal(id, e.message))?;
                let workout = Workout {
                    id: (*id).into(),
                    name: (*name).into(),
                    ftp,
                    nodes: doc.nodes,
                    vars: doc.vars,
                };
                let checked =
                    Entry::at(workout.clone(), ftp, true).map_err(|e| internal(id, e.message))?;
                refuse_errors(&checked.compiled).map_err(|e| internal(id, e.message))?;
                Ok(workout)
            })
            .collect::<Result<_, _>>()
            .map(Self)
    }
    fn is_builtin(id: &str) -> bool {
        id.starts_with(BUILTIN_PREFIX)
    }
    /// The entry for `id` compiled at `ftp`: a built-in, else the rider's.
    ///
    /// # Errors
    /// `NotFound`; `Storage` for a stored row that does not read or compile; `InvalidInput`
    /// for a built-in that does not compile at `ftp`.
    pub(crate) fn get(&self, store: &Store, id: &str, ftp: f64) -> Result<Entry, ApiError> {
        let not_found = || ApiError::new(ErrorCode::NotFound, format!("no workout {id:?}"));
        if Self::is_builtin(id) {
            let workout = self.0.iter().find(|w| w.id == id).ok_or_else(not_found)?;
            return Entry::at(workout.clone(), ftp, true);
        }
        store
            .workout(id)?
            .map_or_else(|| Err(not_found()), |s| stored(&s, ftp))
    }
    /// The built-ins in their order, then the rider's workouts in the order first saved, each
    /// compiled at `ftp`.
    ///
    /// # Errors
    /// As [`Library::get`].
    pub(crate) fn summaries(
        &self,
        store: &Store,
        ftp: f64,
    ) -> Result<Vec<WorkoutSummary>, ApiError> {
        let mut out = Vec::new();
        for w in &self.0 {
            out.push(Entry::at(w.clone(), ftp, true)?.summary());
        }
        for s in store.workouts()? {
            out.push(stored(&s, ftp)?.summary());
        }
        Ok(out)
    }
    /// One workout with its canonical text and compiled view, at `ftp`.
    ///
    /// # Errors
    /// As [`Library::get`]; `Internal` if a block index exceeds `u32`.
    pub(crate) fn detail(
        &self,
        store: &Store,
        id: &str,
        ftp: f64,
    ) -> Result<WorkoutDetail, ApiError> {
        let e = self.get(store, id, ftp)?;
        Ok(WorkoutDetail {
            dsl: print_dsl(&e.workout.vars, &e.workout.nodes),
            compiled: CompiledView::new(&e.compiled, ftp)?,
            workout: e.workout,
            builtin: e.builtin,
        })
    }
    /// The draft a new workout starts from: no steps, at `ftp` (the athlete model's current FTP).
    pub(crate) fn new_draft(ftp: f64) -> WorkoutDraft {
        WorkoutDraft {
            name: NEW_NAME.into(),
            ftp,
            nodes: Vec::new(),
        }
    }
    /// Store `draft` as the rider's workout `id` (an update), or as a new one when `id` is
    /// `None`; answers the stored id. The name is trimmed. The draft's own `ftp` is stored as
    /// the informative `ftp_at_creation`, so the tree is checked at it, and again at `ftp`, the
    /// athlete model's current FTP (the editor previewed it there and `ride_load` compiles it
    /// at the model's FTP too).
    ///
    /// # Errors
    /// `InvalidInput` for a blank or too long name, a built-in id, a tree that does not
    /// compile, or an error-severity issue (its message); `NotFound` for
    /// an id that is no stored workout; `Storage` when the store fails.
    pub(crate) fn save(
        store: &Store,
        id: Option<&str>,
        draft: WorkoutDraft,
        ftp: f64,
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
            // A draft carries no declarations yet (#744): a tree that uses a variable is
            // refused by the compile below as `Resolve(UnknownVariable)`.
            vars: Vec::new(),
        };
        // At its own FTP (stored, so it must be one: positive and finite), then at the model's.
        refuse_errors(&compile_workout(&workout, &CompileOptions::default())?)?;
        refuse_errors(&Entry::at(workout.clone(), ftp, false)?.compiled)?;
        let dsl = print_dsl(&workout.vars, &workout.nodes);
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
