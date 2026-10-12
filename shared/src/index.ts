// @neo/shared — mock backend for the neo-trainer UI spikes.
// The contract types are the only API the spikes should depend on; the helpers below are
// pure functions the UIs may use for presentation (axis mapping, formatting, ids).

export type * from "./contract";

export { createMockBackend, type MockBackend, type MockBackendOptions } from "./backend";

// athlete, zones, metric catalog
export {
  type Athlete,
  DEFAULT_ATHLETE,
  metricCatalog,
  powerZones,
  hrZones,
  pctZones,
  zoneIndex,
  hrSteady,
  powerForHr,
  physioFor,
  type Physio,
} from "./athlete";

// axis scales for log_time_chart (must match history() bucketing)
export { timeAgoToU, uToTimeAgo, timeAgoEdges, timeAgoTicks, fisheyeFocusS, type TimeScale } from "./scales";

// workout tooling (pure)
export { compileWorkout, normalizedPower, TRAINER_MAX_W, LAP_EXPECTED_S, LAP_RANGE_S, type CompileOptions } from "./workout/compile";
export { parseDsl, printDsl, formatDuration, parseDurationWord } from "./workout/dsl";
export { seedSnippets, demoWorkout, DEMO_DSL, SNIPPET_DSL } from "./workout/library";
export { reId, findNode, withoutIds, targetPct, targetWatts, isWorkStep, shiftTarget, WORK_THRESHOLD_PCT } from "./workout/model";
export { rewriteNodes } from "./workout/rewrite";

// building blocks (exported for tests / reuse)
export { createFilterChain, createFilter, type FilterChain, type FilterStage } from "./filters";
export { MultiResSeries, HistoryStore, HISTORY_LEVELS } from "./history";
export { wbalStep, wbalSeries } from "./wbal";
export { rmssd, dfaAlpha1 } from "./hrv";
export { fitMonoExp, predictAt, buildHrRecoveryState, type MonoExpFit, type RecoveryPrior, type RecoveryInput } from "./hrRecovery";
export { histogramFrom, heatmapFrom, mmpFrom, syntheticMmp, MMP_DURATIONS_S } from "./distributions";
export { bandOf, NUDGE_TTL_S, NUDGE_COOLDOWN_S } from "./coach";
export { proposeWorkout, plannerLatencyMs, readiness } from "./planner";
export { defaultLayouts, LayoutStore, memoryStorage, LAYOUT_STORAGE_KEY, type StorageLike } from "./layouts";
export { VirtualClock, realClock, type Clock } from "./clock";
export { Rng } from "./rng";
export { SIM_DT } from "./sim";
export { uid } from "./util";
