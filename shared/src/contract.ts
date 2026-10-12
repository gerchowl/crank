// Contract between UI and backend. In the spikes this is implemented by the TS mock
// (`createMockBackend`); in the real app by the Rust core behind Tauri commands/channels.
// Spikes must only use what is exported here.

// ---------- metrics ----------

export type MetricId =
  | "power" | "power_3s" | "power_10s" | "power_30s" | "power_lap" | "np" | "if" | "tss" | "kj"
  | "w_per_kg" | "pct_ftp" | "target_power" | "delta_target" | "power_zone" | "wbal_pct"
  | "hr" | "hr_zone" | "hr_pct_max" | "hr_vs_typical" | "hr_drift" | "decoupling"
  | "rr" | "rmssd" | "dfa_a1"
  | "cadence" | "target_cadence"
  | "speed" | "distance"
  | "elapsed" | "remaining" | "block_remaining" | "block_index" | "compliance";

export interface MetricInfo {
  id: MetricId;
  label: string;
  unit: string;          // "W", "bpm", "rpm", "%", "s", ...
  sourceHz: number;      // native update rate; display rates above this add nothing
  zones?: Zone[];        // for colouring
}

export interface Zone { name: string; min: number; max: number; color: string }

export type Filter =
  | { kind: "sma"; windowS: number }
  | { kind: "ema"; tauS: number }
  | { kind: "median"; n: number }
  | { kind: "one_euro"; minCutoff: number; beta: number }
  | { kind: "hold" };

export interface SubscribeOpts { rateHz: number; filter?: Filter[] }

export interface Sample { t: number; v: number | null } // t = ride time in seconds; null = no data

export type Unsubscribe = () => void;

// ---------- multi-resolution history (log-time charts) ----------

export interface Bucket { tAgo: number; dur: number; mean: number; min: number; max: number }

export interface HistoryQuery {
  metric: MetricId;
  spanS: number;         // e.g. 7200
  /** target number of buckets; backend picks resolutions so recent = fine, old = coarse */
  buckets: number;
  scale: "log" | "fisheye";
}

// ---------- distributions ----------

export interface Histogram { metric: MetricId; binWidth: number; bins: { lo: number; seconds: number }[]; typical?: number[] }
export interface Heatmap2D {
  x: MetricId; y: MetricId; xBin: number; yBin: number;
  xLo: number; yLo: number; cols: number; rows: number;
  seconds: number[];     // row-major, rows * cols
}
export interface MmpCurve { durationsS: number[]; today: number[]; best90d: number[]; allTime: number[] }

// ---------- HR recovery ----------

export interface HrRecoveryState {
  active: boolean;
  startedT: number;
  peakT: number;
  targetBpm: number;
  observed: Sample[];                 // since block start
  fit?: { hrInf: number; amp: number; tauS: number; usualTauS: number };
  predicted?: { t: number; mean: number; lo: number; hi: number }[]; // future curve + band
  readyInS?: { mean: number; lo: number; hi: number };
  unreachable: boolean;               // hrInf >= target
  minS: number; maxS: number;
}

// ---------- workout model ----------

export type PowerTarget =
  | { kind: "pct_ftp"; value: number }
  | { kind: "watts"; value: number }
  | { kind: "range"; lo: number; hi: number }        // pct FTP
  | { kind: "ramp"; from: number; to: number }       // pct FTP
  | { kind: "hr"; bpm: number }
  | { kind: "free" };

export type StepEnd =
  | { kind: "time"; s: number }
  | { kind: "kj"; kj: number }
  | { kind: "lap" }
  | { kind: "hr_recovery"; targetBpm: number; minS: number; maxS: number;
      onUnreachable: "lower_power" | "cap" | "ask_coach" };

export interface StepNode {
  type: "step"; id: string; label?: string;
  end: StepEnd; target: PowerTarget;
  cadence?: { lo: number; hi: number };
}

export interface RepeatNode {
  type: "repeat"; id: string; label?: string;
  count: number;
  children: WorkoutNode[];
  progression?: { wattsPerRep?: number; pctPerRep?: number };
  skipLastRest?: boolean;
}

export type WorkoutNode = StepNode | RepeatNode;

export interface Workout { id: string; name: string; ftp: number; nodes: WorkoutNode[] }

export interface FlatBlock {
  path: number[];                 // indices into the tree
  iteration: number[];            // rep counters along the path
  nodeId: string;
  startS: number; durS: number;   // expected
  durRangeS?: [number, number];   // for open-ended blocks
  powerFrom: number; powerTo: number; // watts (ramp if different)
  phase: Phase;
  aiChanged?: boolean;
}

export type Phase = "warmup" | "work" | "recovery" | "cooldown" | "free";

export interface ValidationIssue { severity: "error" | "warning"; nodeId?: string; message: string }

export interface CompiledWorkout {
  blocks: FlatBlock[];
  totalS: number; totalRangeS: [number, number];
  tss: number; if: number;
  timeInZoneS: number[];          // by power zone index
  issues: ValidationIssue[];
}

export interface DslError { message: string; offset: number; length: number }

// ---------- coach ----------

export interface Checkin {
  sleep: 1 | 2 | 3 | 4 | 5; legs: 1 | 2 | 3 | 4 | 5; stress: 1 | 2 | 3 | 4 | 5; motivation: 1 | 2 | 3 | 4 | 5;
  minutes: number; note?: string;
}

export type Band = "strong" | "lean" | "uncertain";

export interface CoachNudge {
  callId: string; t: number;
  verdict: "ease_off" | "hold" | "push" | "replan";
  band: Band; p: number;
  message: string;               // human explanation
  proposedPctChange?: number;    // e.g. -5
  status: "pending" | "auto_applied" | "accepted" | "declined" | "expired";
}

export interface Proposal { workout: Workout; rationale: string }

export type RewriteIntent = "vary" | "harder" | "easier" | "shorter_same_stimulus";

// ---------- layouts ----------

export type WidgetKind =
  | "big_number" | "number_sparkline" | "zone_bar" | "log_time_chart" | "histogram"
  | "heatmap2d" | "mmp_curve" | "workout_strip" | "hr_recovery" | "coach_card";

export interface Widget {
  id: string; kind: WidgetKind;
  x: number; y: number; w: number; h: number;   // grid units (12 cols)
  metrics: MetricId[];                           // 1 for numbers, n for charts, [x,y] for heatmap
  rateHz: number;
  filter?: Filter[];
  colorByZone?: boolean;
  spanS?: number;
  scale?: "log" | "fisheye";
}

export interface Page { id: string; name: string; autoPhase?: Phase[]; widgets: Widget[] }
export interface Layout { id: string; name: string; device: "laptop" | "tablet" | "tv"; pages: Page[] }

// ---------- ride control ----------

export interface RideStatus {
  state: "idle" | "riding" | "paused" | "finished";
  t: number; speed: 1 | 10 | 60;
  mode: "erg" | "sim" | "free";
  trainer: "connected" | "connecting" | "lost";
  hrSensor: "connected" | "connecting" | "lost";
  blockIndex: number; phase: Phase;
  manualOffsetPct: number;
}

// ---------- the backend ----------

export interface Backend {
  metrics: MetricInfo[];
  zones: { power: Zone[]; hr: Zone[] };
  athlete: { ftp: number; weightKg: number; maxHr: number; restHr: number };

  // live data
  subscribe(metric: MetricId, opts: SubscribeOpts, cb: (s: Sample) => void): Unsubscribe;
  history(q: HistoryQuery): Bucket[];
  histogram(metric: MetricId, binWidth: number): Histogram;
  heatmap(x: MetricId, y: MetricId, xBin: number, yBin: number): Heatmap2D;
  mmp(): MmpCurve;
  hrRecovery(): HrRecoveryState;
  onStatus(cb: (s: RideStatus) => void): Unsubscribe;
  onNudge(cb: (n: CoachNudge) => void): Unsubscribe;

  ride: {
    load(w: Workout): void;
    start(): void; pause(): void; resume(): void; stop(): void;
    setSpeed(s: 1 | 10 | 60): void;
    adjust(pct: number): void;       // manual ± (cumulative)
    skipBlock(): void;
    current(): { workout: Workout; compiled: CompiledWorkout } | null;
  };

  // workout tooling (pure)
  compileWorkout(w: Workout): CompiledWorkout;
  parseDsl(text: string, ftp: number): { nodes: WorkoutNode[] } | { error: DslError };
  printDsl(nodes: WorkoutNode[]): string;
  snippets(): { name: string; nodes: WorkoutNode[] }[];

  // coach
  coach: {
    respond(callId: string, accept: boolean): void;
    rate(callId: string, rating: "right" | "wrong" | "unsure", useful: boolean): void;
    decisions(): CoachNudge[];
  };
  planner: {
    propose(c: Checkin): Promise<Proposal>;                       // mock: ~800 ms latency
    rewrite(nodes: WorkoutNode[], intent: RewriteIntent): Promise<WorkoutNode[]>;
  };

  // layouts
  layouts: {
    list(): Layout[];
    get(id: string): Layout | undefined;
    save(l: Layout): void;           // persisted (localStorage in mock)
    reset(id: string): Layout;       // back to built-in default
  };
}
