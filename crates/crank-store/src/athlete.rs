//! The athlete model's rows (ADR-0009 §2.1.1, §2.2.4, §2.5; #714): the append-only estimate history
//! (`athlete_estimate`) and the proposals (`athlete_proposal`, `athlete_proposal_param`).
//!
//! **One writer of estimates.** A row is appended only through [`EstimateOrigin`], one variant per path ADR-0009
//! §2.2.4 "Never silent" allows to write the model (the rider's set, revert and clear, onboarding, an import, and
//! accepting a proposal); this module holds the only `INSERT` into the table. Every appended row passes
//! [`check_entry`] against the model as stored, in the transaction that appends it. A path other than accept marks
//! every open proposal of its parameter `superseded` in that transaction ("Manual entry wins"); accept writes one row
//! per proposed value and marks the proposal `accepted`, also in one transaction.
//!
//! The rules that raise, stale and re-raise proposals are the core's (#720); here a proposal is inserted, listed and
//! moved out of `open` once.
use crate::{JsonColumn, ProposalEvidenceJson, StoreError};
use crank_domain::athlete::{
    AthleteError, AthleteParam, AthleteSnapshot, Cleared, Entry, Estimate, METHOD_MANUAL, Source,
    check_entry, check_value,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

/// The method of an estimate row copied by migration 0005 from the table before it (ADR-0009 §2.5).
pub const METHOD_LEGACY: &str = "legacy";
/// The method of the weight row migration 0005 moved out of `athlete_profile.weight` (ADR-0009 §2.5).
pub const METHOD_PROFILE: &str = "profile";

/// The path that appends an estimate row (ADR-0009 §2.2.4 "Never silent"): the store appends a row only for one of
/// these ([`crate::Store::append_athlete`]).
#[derive(Clone, Debug, PartialEq)]
pub enum EstimateOrigin {
    /// `athlete_set`: a value the rider typed in (source `self_report`, method [`METHOD_MANUAL`], n 1).
    Set {
        /// The parameter.
        param: AthleteParam,
        /// The value, in the parameter's unit.
        value: f64,
        /// Its sd.
        sd: f64,
    },
    /// `athlete_revert`: a copy of an earlier value row (source, method, ride, proposal and n kept, a new time).
    /// Refused for a prior row (that is Clear) and for a cleared row (`InvalidInput`).
    Revert {
        /// The row copied.
        estimate_id: i64,
    },
    /// `athlete_clear`: the prior's row, or a cleared row for a parameter with no prior ([`Entry::clear`], HR max's
    /// prior from `athlete_profile.age`).
    Clear {
        /// The parameter.
        param: AthleteParam,
    },
    /// Onboarding: a self-reported number (source `self_report`, method [`METHOD_MANUAL`], n 1) with the sd of its
    /// stated origin (`crank_domain::athlete::prior::SelfReportOrigin`).
    OnboardingSelfReport {
        /// The parameter.
        param: AthleteParam,
        /// The value.
        value: f64,
        /// Its sd.
        sd: f64,
    },
    /// Onboarding: the parameter's prior row; `InvalidInput` when it has none.
    OnboardingPrior {
        /// The parameter.
        param: AthleteParam,
    },
    /// An import from another service (ONBOARDING §2): source `imported`.
    Import {
        /// The parameter.
        param: AthleteParam,
        /// The value.
        value: f64,
        /// Its sd.
        sd: f64,
        /// How the service derived it; not empty.
        method: String,
        /// Number of efforts or rides behind it.
        n: u32,
    },
    /// `athlete_proposal_accept`: one row per proposed value, with the proposal's method and triggering ride and
    /// the proposal's id; the proposal is marked `accepted` in the same transaction. `values` are what the core
    /// recomputed from the frozen evidence (#720) and must name exactly the proposal's parameters. `Conflict` when
    /// the proposal is not `open`, or was raised before the current row of one of its parameters (ADR-0009 §2.2.4
    /// "Manual entry wins"); `NotFound` when there is none.
    ProposalAccept {
        /// The proposal.
        proposal_id: i64,
        /// `test` or `rides` (ADR-0009 §2.2.4: every proposal is one of them).
        source: Source,
        /// The values to write, one per parameter of the proposal.
        values: Vec<ProposedValue>,
        /// Number of efforts or rides behind them.
        n: u32,
    },
}

/// One proposed value (`athlete_proposal.values_json`).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ProposedValue {
    /// The parameter.
    pub param: AthleteParam,
    /// The value, in the parameter's unit.
    pub value: f64,
    /// Its sd.
    pub sd: f64,
}

/// Why an open proposal went stale (ADR-0009 §2.2.4 "Evidence and acceptance"; `athlete_proposal.stale_reason`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaleReason {
    /// An `ANALYSIS_VERSION` bump re-analysed its rides.
    Version,
    /// A ride of its frozen evidence was deleted.
    RideDeleted,
    /// P8, P9: the CP estimate it read is no longer current.
    CpReplaced,
    /// An FTP proposal displaced by one of higher priority.
    Displaced,
}

impl StaleReason {
    const ALL: [Self; 4] = [
        Self::Version,
        Self::RideDeleted,
        Self::CpReplaced,
        Self::Displaced,
    ];

    /// The stored name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Version => "version",
            Self::RideDeleted => "ride_deleted",
            Self::CpReplaced => "cp_replaced",
            Self::Displaced => "displaced",
        }
    }
}

/// A proposal's status (`athlete_proposal.status`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProposalStatus {
    /// Waiting for the rider.
    Open,
    /// Accepted: its values were written as estimates.
    Accepted,
    /// Dismissed by the rider.
    Dismissed,
    /// Lost to a newer value or a manual entry.
    Superseded,
    /// Its evidence no longer holds.
    Stale(StaleReason),
}

impl ProposalStatus {
    const fn columns(self) -> (&'static str, Option<&'static str>) {
        match self {
            Self::Open => ("open", None),
            Self::Accepted => ("accepted", None),
            Self::Dismissed => ("dismissed", None),
            Self::Superseded => ("superseded", None),
            Self::Stale(r) => ("stale", Some(r.as_str())),
        }
    }

    fn from_columns(status: &str, reason: Option<&str>) -> Result<Self, StoreError> {
        let corrupt =
            || StoreError::InvalidInput(format!("stored proposal status {status:?} / {reason:?}"));
        Ok(match (status, reason) {
            ("open", None) => Self::Open,
            ("accepted", None) => Self::Accepted,
            ("dismissed", None) => Self::Dismissed,
            ("superseded", None) => Self::Superseded,
            ("stale", Some(r)) => Self::Stale(
                StaleReason::ALL
                    .into_iter()
                    .find(|v| v.as_str() == r)
                    .ok_or_else(corrupt)?,
            ),
            _ => return Err(corrupt()),
        })
    }
}

/// How an open proposal is closed without accepting it ([`crate::Store::close_proposal`]). Accepting goes through
/// [`EstimateOrigin::ProposalAccept`], so an `accepted` proposal always wrote its estimates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProposalClose {
    /// The rider dismissed it.
    Dismissed,
    /// A newer proposal for the same parameter replaced it.
    Superseded,
    /// Its evidence no longer holds.
    Stale(StaleReason),
}

impl From<ProposalClose> for ProposalStatus {
    fn from(c: ProposalClose) -> Self {
        match c {
            ProposalClose::Dismissed => Self::Dismissed,
            ProposalClose::Superseded => Self::Superseded,
            ProposalClose::Stale(r) => Self::Stale(r),
        }
    }
}

/// A proposal to raise ([`crate::Store::insert_proposal`]); it is stored `open`.
#[derive(Clone, Debug, PartialEq)]
pub struct NewProposal {
    /// The triggering ride; the proposal is deleted with it.
    pub ride_id: String,
    /// The rule's method (`peak_hr_v1`, `cp_ols_v1`, …); not empty.
    pub method: String,
    /// 1, or one more than the stale row it re-raises (§2.2.4 "Never raised twice").
    pub generation: u32,
    /// One value, or two (CP and W′ of P5 and P6), each of another parameter.
    pub values: Vec<ProposedValue>,
    /// Everything the values were computed from.
    pub evidence: ProposalEvidenceJson,
    /// The `ANALYSIS_VERSION` it was computed at.
    pub analysis_version: u32,
    /// UTC seconds.
    pub created_at: i64,
}

/// A stored proposal.
#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    /// The row.
    pub id: i64,
    /// The triggering ride.
    pub ride_id: String,
    /// The rule's method.
    pub method: String,
    /// Its generation for (`ride_id`, `method`).
    pub generation: u32,
    /// The proposed values.
    pub values: Vec<ProposedValue>,
    /// The frozen evidence.
    pub evidence: ProposalEvidenceJson,
    /// The `ANALYSIS_VERSION` it was computed at.
    pub analysis_version: u32,
    /// Its status.
    pub status: ProposalStatus,
    /// When it was raised, UTC seconds.
    pub created_at: i64,
    /// When it left `open`, UTC seconds.
    pub decided_at: Option<i64>,
}

/// `values_json`'s envelope.
#[derive(serde::Serialize, serde::Deserialize)]
struct ValuesJson {
    v: u32,
    values: Vec<ProposedValue>,
}

const ESTIMATE_SELECT: &str = "SELECT id,param,kind,value,sd,source,method,ride_id,proposal_id,n,updated_at \
     FROM athlete_estimate";
const PROPOSAL_SELECT: &str = "SELECT p.id,p.ride_id,p.method,p.generation,p.values_json,p.evidence_json,\
     p.analysis_version,p.status,p.stale_reason,p.created_at,p.decided_at FROM athlete_proposal p";
/// "Open proposals touching param X" (ADR-0009 §2.5): an index search on `athlete_proposal_param(param, …)`, then
/// each proposal by its id.
pub(crate) const OPEN_FOR_PARAM: &str = "SELECT p.id,p.ride_id,p.method,p.generation,p.values_json,\
     p.evidence_json,p.analysis_version,p.status,p.stale_reason,p.created_at,p.decided_at \
     FROM athlete_proposal_param pp JOIN athlete_proposal p ON p.id = pp.proposal_id \
     WHERE pp.param = ?1 AND p.status = 'open' ORDER BY pp.proposal_id";

type EstimateRow = (
    i64,
    String,
    String,
    Option<f64>,
    Option<f64>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    i64,
);

fn estimate_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<EstimateRow> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
        r.get(9)?,
        r.get(10)?,
    ))
}

fn decode_entry(
    (id, param, kind, value, sd, source, method, ride_id, proposal_id, n, updated_at): EstimateRow,
) -> Result<Entry, StoreError> {
    let param: AthleteParam = param.parse()?;
    let corrupt = || {
        StoreError::InvalidInput(format!(
            "athlete_estimate row {id} breaks its kind {kind:?}"
        ))
    };
    match (kind.as_str(), value, sd, source, method, n) {
        ("estimate", Some(value), Some(sd), Some(source), Some(method), Some(n)) => {
            Ok(Entry::Estimate(Estimate {
                id,
                param,
                value,
                sd,
                source: source.parse()?,
                method,
                ride_id,
                proposal_id,
                n: u32::try_from(n).map_err(|_| corrupt())?,
                updated_at,
            }))
        }
        ("cleared", None, None, None, None, None) if ride_id.is_none() && proposal_id.is_none() => {
            Ok(Entry::Cleared(Cleared {
                id,
                param,
                updated_at,
            }))
        }
        _ => Err(corrupt()),
    }
}

/// The rider's age (`athlete_profile.age`), which HR max's prior needs.
fn age(db: &Connection) -> Result<Option<u32>, StoreError> {
    let age: Option<Option<i64>> = db
        .query_row("SELECT age FROM athlete_profile WHERE id = 1", [], |r| {
            r.get(0)
        })
        .optional()?;
    age.flatten()
        .map(|a| {
            u32::try_from(a)
                .map_err(|_| StoreError::InvalidInput(format!("stored athlete_profile.age {a}")))
        })
        .transpose()
}

/// A row by its id.
pub(crate) fn entry(db: &Connection, id: i64) -> Result<Option<Entry>, StoreError> {
    db.query_row(
        &format!("{ESTIMATE_SELECT} WHERE id = ?1"),
        [id],
        estimate_row,
    )
    .optional()?
    .map(decode_entry)
    .transpose()
}

/// Every row of `param`, newest first.
pub(crate) fn history(db: &Connection, param: AthleteParam) -> Result<Vec<Entry>, StoreError> {
    db.prepare(&format!(
        "{ESTIMATE_SELECT} WHERE param = ?1 ORDER BY updated_at DESC, id DESC"
    ))?
    .query_map([param.as_str()], estimate_row)?
    .map(|r| decode_entry(r?))
    .collect()
}

/// The newest row of each parameter that has one, in [`AthleteParam::ALL`]'s order: the largest `updated_at`, then
/// the largest id (as [`AthleteSnapshot::resolve`] picks it).
pub(crate) fn current(db: &Connection) -> Result<Vec<Entry>, StoreError> {
    let mut stmt = db.prepare_cached(&format!(
        "{ESTIMATE_SELECT} WHERE param = ?1 ORDER BY updated_at DESC, id DESC LIMIT 1"
    ))?;
    let mut out = Vec::new();
    for param in AthleteParam::ALL {
        if let Some(row) = stmt.query_row([param.as_str()], estimate_row).optional()? {
            out.push(decode_entry(row)?);
        }
    }
    Ok(out)
}

/// The model as stored: [`AthleteSnapshot::resolve`] over [`current`] with the profile's age.
pub(crate) fn snapshot(db: &Connection) -> Result<AthleteSnapshot, StoreError> {
    Ok(AthleteSnapshot::resolve(&current(db)?, age(db)?))
}

fn value_row(
    param: AthleteParam,
    value: f64,
    sd: f64,
    source: Source,
    method: &str,
    n: u32,
    now: i64,
) -> Estimate {
    Estimate {
        id: 0,
        param,
        value,
        sd,
        source,
        method: method.to_owned(),
        ride_id: None,
        proposal_id: None,
        n,
        updated_at: now,
    }
}

/// The rows accepting `proposal_id` appends ([`EstimateOrigin::ProposalAccept`]).
fn accept_rows(
    tx: &Transaction<'_>,
    proposal_id: i64,
    source: Source,
    values: Vec<ProposedValue>,
    n: u32,
    now: i64,
) -> Result<Vec<Entry>, StoreError> {
    let invalid = |m: &str| StoreError::InvalidInput(m.to_owned());
    if !matches!(source, Source::Test | Source::Rides) {
        return Err(invalid("a proposal's value is from a test or from rides"));
    }
    let p = proposal(tx, proposal_id)?
        .ok_or_else(|| StoreError::NotFound(format!("proposal {proposal_id}")))?;
    if p.status != ProposalStatus::Open {
        return Err(StoreError::Conflict(format!(
            "proposal {proposal_id} is not open"
        )));
    }
    // "Manual entry wins" (ADR-0009 §2.2.4): a proposal raised before its parameter's current value was
    // written is refused, whatever wrote that value.
    if let Some(newer) = current(tx)?
        .iter()
        .find(|e| p.values.iter().any(|v| v.param == e.param()) && e.updated_at() > p.created_at)
    {
        return Err(StoreError::Conflict(format!(
            "proposal {proposal_id} was raised before the current {}",
            newer.param()
        )));
    }
    let mut want: Vec<_> = p.values.iter().map(|v| v.param).collect();
    let mut got: Vec<_> = values.iter().map(|v| v.param).collect();
    want.sort_unstable();
    got.sort_unstable();
    if want != got {
        return Err(invalid(
            "the accepted values must name the proposal's parameters",
        ));
    }
    let rows = values
        .into_iter()
        .map(|v| {
            Entry::Estimate(Estimate {
                ride_id: Some(p.ride_id.clone()),
                proposal_id: Some(proposal_id),
                ..value_row(v.param, v.value, v.sd, source, &p.method, n, now)
            })
        })
        .collect();
    Ok(rows)
}

/// The rows `origin` appends (ids still 0), and the proposal it accepts, if any.
fn rows(
    tx: &Transaction<'_>,
    origin: EstimateOrigin,
    now: i64,
) -> Result<(Vec<Entry>, Option<i64>), StoreError> {
    let invalid = |m: &str| StoreError::InvalidInput(m.to_owned());
    Ok(match origin {
        EstimateOrigin::Set { param, value, sd }
        | EstimateOrigin::OnboardingSelfReport { param, value, sd } => (
            vec![Entry::Estimate(value_row(
                param,
                value,
                sd,
                Source::SelfReport,
                METHOD_MANUAL,
                1,
                now,
            ))],
            None,
        ),
        EstimateOrigin::Revert { estimate_id } => {
            match entry(tx, estimate_id)?
                .ok_or_else(|| StoreError::NotFound(format!("estimate {estimate_id}")))?
            {
                Entry::Cleared(_) => {
                    return Err(invalid("a cleared row has no value to revert to"));
                }
                Entry::Estimate(e) if e.source == Source::Prior => {
                    return Err(invalid("a prior row is not reverted to: that is Clear"));
                }
                Entry::Estimate(e) => (
                    vec![Entry::Estimate(Estimate {
                        id: 0,
                        updated_at: now,
                        ..e
                    })],
                    None,
                ),
            }
        }
        EstimateOrigin::Clear { param } => (vec![Entry::clear(0, param, age(tx)?, now)], None),
        EstimateOrigin::OnboardingPrior { param } => (
            vec![Entry::Estimate(
                Estimate::prior_row(0, param, age(tx)?, now)
                    .ok_or_else(|| invalid("the parameter has no prior"))?,
            )],
            None,
        ),
        EstimateOrigin::Import {
            param,
            value,
            sd,
            method,
            n,
        } => {
            if method.is_empty() {
                return Err(invalid("an imported value needs a method"));
            }
            let row = value_row(param, value, sd, Source::Imported, &method, n, now);
            (vec![Entry::Estimate(row)], None)
        }
        EstimateOrigin::ProposalAccept {
            proposal_id,
            source,
            values,
            n,
        } => (
            accept_rows(tx, proposal_id, source, values, n, now)?,
            Some(proposal_id),
        ),
    })
}

/// Appends the rows of `origin` in one transaction (see the module doc). A row's `updated_at` is `now`, or its
/// parameter's newest `updated_at` when the clock went back, so the appended row is always the current one.
pub(crate) fn append(
    tx: &Transaction<'_>,
    origin: EstimateOrigin,
    now: i64,
) -> Result<Vec<Entry>, StoreError> {
    let model = snapshot(tx)?;
    let newest = current(tx)?;
    let (mut entries, accepted) = rows(tx, origin, now)?;
    let mut insert = tx.prepare_cached(
        "INSERT INTO athlete_estimate (param,kind,value,sd,source,method,ride_id,proposal_id,n,updated_at) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
    )?;
    for e in &mut entries {
        check_entry(e, &model)?;
        let at = newest
            .iter()
            .filter(|n| n.param() == e.param())
            .map(Entry::updated_at)
            .fold(now, i64::max);
        match e {
            Entry::Estimate(e) => {
                e.updated_at = at;
                insert.execute(params![
                    e.param.as_str(),
                    "estimate",
                    e.value,
                    e.sd,
                    e.source.as_str(),
                    e.method,
                    e.ride_id,
                    e.proposal_id,
                    e.n,
                    at
                ])?;
                e.id = tx.last_insert_rowid();
            }
            Entry::Cleared(c) => {
                c.updated_at = at;
                insert.execute(params![
                    c.param.as_str(),
                    "cleared",
                    None::<f64>,
                    None::<f64>,
                    None::<String>,
                    None::<String>,
                    None::<String>,
                    None::<i64>,
                    None::<u32>,
                    at
                ])?;
                c.id = tx.last_insert_rowid();
            }
        }
    }
    if let Some(id) = accepted {
        tx.execute(
            "UPDATE athlete_proposal SET status = 'accepted', decided_at = ?2 WHERE id = ?1",
            params![id, now],
        )?;
    } else {
        for e in &entries {
            tx.execute(
                "UPDATE athlete_proposal SET status = 'superseded', decided_at = ?2 \
                 WHERE status = 'open' AND id IN \
                   (SELECT proposal_id FROM athlete_proposal_param WHERE param = ?1)",
                params![e.param().as_str(), now],
            )?;
        }
    }
    Ok(entries)
}

type ProposalRow = (
    i64,
    String,
    String,
    i64,
    String,
    String,
    i64,
    String,
    Option<String>,
    i64,
    Option<i64>,
);

fn proposal_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ProposalRow> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
        r.get(9)?,
        r.get(10)?,
    ))
}

fn decode_proposal(
    (
        id,
        ride_id,
        method,
        generation,
        values,
        evidence,
        version,
        status,
        reason,
        created_at,
        decided_at,
    ): ProposalRow,
) -> Result<Proposal, StoreError> {
    let bad_values = |reason: String| StoreError::InvalidJson {
        table: "athlete_proposal",
        column: "values_json",
        reason,
    };
    let values: ValuesJson =
        serde_json::from_str(&values).map_err(|e| bad_values(e.to_string()))?;
    if values.v != 1 {
        return Err(bad_values(format!("unknown version {}", values.v)));
    }
    let number = |n: i64| {
        u32::try_from(n)
            .map_err(|_| StoreError::InvalidInput(format!("athlete_proposal row {id}: {n}")))
    };
    Ok(Proposal {
        id,
        ride_id,
        method,
        generation: number(generation)?,
        values: values.values,
        evidence: ProposalEvidenceJson::parse(&evidence)?,
        analysis_version: number(version)?,
        status: ProposalStatus::from_columns(&status, reason.as_deref())?,
        created_at,
        decided_at,
    })
}

/// A proposal by its id.
pub(crate) fn proposal(db: &Connection, id: i64) -> Result<Option<Proposal>, StoreError> {
    db.query_row(
        &format!("{PROPOSAL_SELECT} WHERE p.id = ?1"),
        [id],
        proposal_row,
    )
    .optional()?
    .map(decode_proposal)
    .transpose()
}

/// Every open proposal, oldest first.
pub(crate) fn open_proposals(db: &Connection) -> Result<Vec<Proposal>, StoreError> {
    db.prepare(&format!(
        "{PROPOSAL_SELECT} WHERE p.status = 'open' ORDER BY p.id"
    ))?
    .query_map([], proposal_row)?
    .map(|r| decode_proposal(r?))
    .collect()
}

/// The open proposals with a value for `param`, oldest first ([`OPEN_FOR_PARAM`]).
pub(crate) fn open_proposals_for(
    db: &Connection,
    param: AthleteParam,
) -> Result<Vec<Proposal>, StoreError> {
    db.prepare(OPEN_FOR_PARAM)?
        .query_map([param.as_str()], proposal_row)?
        .map(|r| decode_proposal(r?))
        .collect()
}

/// Stores `new` as an open proposal and returns its id.
pub(crate) fn insert_proposal(tx: &Transaction<'_>, new: &NewProposal) -> Result<i64, StoreError> {
    let invalid = |m: &str| StoreError::InvalidInput(m.to_owned());
    if new.method.is_empty() || new.generation == 0 {
        return Err(invalid("a proposal needs a method and a generation from 1"));
    }
    if !(1..=2).contains(&new.values.len())
        || new.values.first().map(|v| v.param) == new.values.get(1).map(|v| v.param)
    {
        return Err(invalid(
            "a proposal has one value, or two of different parameters",
        ));
    }
    for v in &new.values {
        check_value(v.param, v.value, v.sd).map_err(StoreError::Athlete)?;
    }
    let ride: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM ride WHERE id = ?1)",
        [&new.ride_id],
        |r| r.get(0),
    )?;
    if !ride {
        return Err(StoreError::NotFound(format!("ride {:?}", new.ride_id)));
    }
    let taken: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM athlete_proposal WHERE ride_id = ?1 AND method = ?2 AND generation = ?3)",
        params![new.ride_id, new.method, new.generation],
        |r| r.get(0),
    )?;
    if taken {
        return Err(StoreError::Conflict(format!(
            "ride {:?} already has generation {} of {}",
            new.ride_id, new.generation, new.method
        )));
    }
    let values = serde_json::to_string(&ValuesJson {
        v: 1,
        values: new.values.clone(),
    })
    .map_err(|e| invalid(&e.to_string()))?;
    tx.execute(
        "INSERT INTO athlete_proposal \
         (ride_id,method,generation,values_json,evidence_json,analysis_version,status,created_at) \
         VALUES (?1,?2,?3,?4,?5,?6,'open',?7)",
        params![
            new.ride_id,
            new.method,
            new.generation,
            values,
            new.evidence.as_str(),
            new.analysis_version,
            new.created_at
        ],
    )?;
    let id = tx.last_insert_rowid();
    for v in &new.values {
        tx.execute(
            "INSERT INTO athlete_proposal_param (proposal_id,param) VALUES (?1,?2)",
            params![id, v.param.as_str()],
        )?;
    }
    Ok(id)
}

/// Moves an open proposal to `close` at `now`.
pub(crate) fn close_proposal(
    tx: &Transaction<'_>,
    id: i64,
    close: ProposalClose,
    now: i64,
) -> Result<(), StoreError> {
    let p = proposal(tx, id)?.ok_or_else(|| StoreError::NotFound(format!("proposal {id}")))?;
    if p.status != ProposalStatus::Open {
        return Err(StoreError::Conflict(format!("proposal {id} is not open")));
    }
    let (status, reason) = ProposalStatus::from(close).columns();
    tx.execute(
        "UPDATE athlete_proposal SET status = ?2, stale_reason = ?3, decided_at = ?4 WHERE id = ?1",
        params![id, status, reason, now],
    )?;
    Ok(())
}

impl From<AthleteError> for StoreError {
    fn from(e: AthleteError) -> Self {
        Self::Athlete(e)
    }
}

#[cfg(test)]
mod tests {
    use super::OPEN_FOR_PARAM;
    use rusqlite::Connection;

    /// "Open proposals touching param X" is an index search (ADR-0009 §2.5), never a scan of either table.
    #[test]
    fn open_proposals_for_a_param_is_an_indexed_query() {
        let dir = tempfile::tempdir().expect("temp dir");
        drop(crate::Store::open_owner(dir.path()).expect("migrated store"));
        let db = Connection::open(dir.path().join("crank.db")).expect("database");
        let plan: Vec<String> = db
            .prepare(&format!("EXPLAIN QUERY PLAN {OPEN_FOR_PARAM}"))
            .expect("plan")
            .query_map(["ftp"], |r| r.get::<_, String>(3))
            .expect("plan rows")
            .collect::<Result<_, _>>()
            .expect("plan text");
        assert!(
            plan.iter()
                .any(|s| s.contains("athlete_proposal_param_by_param") && s.contains("param=?")),
            "{plan:?}"
        );
        assert!(
            plan.iter()
                .any(|s| s.starts_with("SEARCH p USING INTEGER PRIMARY KEY")),
            "{plan:?}"
        );
        assert!(!plan.iter().any(|s| s.starts_with("SCAN")), "{plan:?}");
    }
}
