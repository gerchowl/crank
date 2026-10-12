import type { DslError, PowerTarget, RepeatNode, StepEnd, StepNode, WorkoutNode } from "../contract";
import { uid } from "../util";

/*
 * Workout text DSL — grammar and examples in shared/DSL.md.
 *
 *   10m ramp 45-75%
 *   "VO2" 3x[+5w, !last](4x(30s 120%, 15s 50%), 3m 50%)
 *   hr<120 2m..4m 40%
 *   10m 55%
 */

type TokKind = "word" | "string" | "(" | ")" | "[" | "]" | "sep" | "eof";
interface Tok { kind: TokKind; text: string; offset: number; length: number }

class DslFail extends Error {
  constructor(message: string, readonly offset: number, readonly length: number) {
    super(message);
  }
}

const fail = (msg: string, tok: { offset: number; length: number }): never => {
  throw new DslFail(msg, tok.offset, Math.max(0, tok.length));
};

function lex(src: string): Tok[] {
  const toks: Tok[] = [];
  let i = 0;
  const isWordChar = (c: string) => !/[\s()[\],;"#]/.test(c);
  while (i < src.length) {
    const c = src[i] as string;
    if (c === "#") {
      while (i < src.length && src[i] !== "\n") i++;
      continue;
    }
    if (c === "\n" || c === "," || c === ";") {
      toks.push({ kind: "sep", text: c, offset: i, length: 1 });
      i++;
      continue;
    }
    if (/\s/.test(c)) {
      i++;
      continue;
    }
    if (c === "(" || c === ")" || c === "[" || c === "]") {
      toks.push({ kind: c, text: c, offset: i, length: 1 });
      i++;
      continue;
    }
    if (c === '"') {
      const start = i;
      i++;
      let s = "";
      while (i < src.length && src[i] !== '"') {
        if (src[i] === "\\" && i + 1 < src.length) {
          s += src[i + 1];
          i += 2;
        } else if (src[i] === "\n") {
          break;
        } else {
          s += src[i];
          i++;
        }
      }
      if (src[i] !== '"') fail("Unterminated label string", { offset: start, length: i - start });
      i++;
      toks.push({ kind: "string", text: s, offset: start, length: i - start });
      continue;
    }
    const start = i;
    while (i < src.length && isWordChar(src[i] as string)) i++;
    toks.push({ kind: "word", text: src.slice(start, i), offset: start, length: i - start });
  }
  toks.push({ kind: "eof", text: "", offset: src.length, length: 0 });
  return toks;
}

const NUM = String.raw`\d+(?:\.\d+)?`;
const RE_DUR = /^(?:(\d+)h)?(?:(\d+)m)?(?:(\d+)s)?$/i;
const RE_REPEAT = /^(\d+)x$/i;
const RE_KJ = new RegExp(`^(${NUM})kj$`, "i");
const RE_HR_END = /^hr<(\d+)$/i;
const RE_PCT = new RegExp(`^(${NUM})%$`);
const RE_RANGE = new RegExp(`^(${NUM})-(${NUM})%$`);
const RE_WATTS = /^(\d+)w$/i;
const RE_BPM = /^(\d+)bpm$/i;
const RE_RPM = /^(\d+)(?:-(\d+))?rpm$/i;
const RE_PROG_W = new RegExp(`^([+-]${NUM})w$`, "i");
const RE_PROG_P = new RegExp(`^([+-]${NUM})%$`);

export function parseDurationWord(w: string): number | null {
  if (!w) return null;
  const m = RE_DUR.exec(w);
  if (!m || (!m[1] && !m[2] && !m[3])) return null;
  return Number(m[1] ?? 0) * 3600 + Number(m[2] ?? 0) * 60 + Number(m[3] ?? 0);
}

class Parser {
  private i = 0;
  constructor(private toks: Tok[], private ftp: number) {}

  private peek(): Tok {
    return this.toks[this.i] as Tok;
  }
  private next(): Tok {
    return this.toks[this.i++] as Tok;
  }

  parseTop(): WorkoutNode[] {
    const nodes = this.parseSeq(null);
    const t = this.peek();
    if (t.kind !== "eof") fail(`Unexpected '${t.text}'`, t);
    return nodes;
  }

  private parseSeq(open: Tok | null): WorkoutNode[] {
    const items: WorkoutNode[] = [];
    for (;;) {
      while (this.peek().kind === "sep") this.next();
      const t = this.peek();
      if (t.kind === "eof") {
        if (open) fail("Unclosed '(' — missing ')'", open);
        break;
      }
      if (t.kind === ")") {
        if (!open) fail("Unexpected ')'", t);
        break;
      }
      items.push(this.parseItem());
      const after = this.peek();
      if (after.kind !== "sep" && after.kind !== "eof" && after.kind !== ")") {
        fail(`Unexpected '${after.text}' — separate items with ',', ';' or a newline`, after);
      }
    }
    return items;
  }

  private parseItem(): WorkoutNode {
    let label: string | undefined;
    if (this.peek().kind === "string") label = this.next().text;
    const t = this.next();
    if (t.kind !== "word") {
      if (t.kind === "eof") fail(label !== undefined ? "Expected a step or repeat after the label" : "Unexpected end of input", t);
      fail(`Unexpected '${t.text}'`, t);
    }
    const rm = RE_REPEAT.exec(t.text);
    if (rm) return this.parseRepeat(Number(rm[1]), t, label);
    return this.parseStep(t, label);
  }

  private parseRepeat(count: number, countTok: Tok, label: string | undefined): RepeatNode {
    if (count < 1) fail("Repeat count must be ≥ 1", countTok);
    const node: RepeatNode = { type: "repeat", id: uid("r"), count, children: [] };
    if (label !== undefined) node.label = label;
    if (this.peek().kind === "[") {
      const open = this.next();
      const prog: { wattsPerRep?: number; pctPerRep?: number } = {};
      for (;;) {
        const o = this.next();
        if (o.kind === "]") break;
        if (o.kind === "sep") continue;
        if (o.kind !== "word") fail(o.kind === "eof" ? "Unclosed '[' — missing ']'" : `Unexpected '${o.text}' in repeat options`, o.kind === "eof" ? open : o);
        let m: RegExpExecArray | null;
        if ((m = RE_PROG_W.exec(o.text))) prog.wattsPerRep = Number(m[1]);
        else if ((m = RE_PROG_P.exec(o.text))) prog.pctPerRep = Number(m[1]);
        else if (o.text.toLowerCase() === "!last") node.skipLastRest = true;
        else fail(`Unknown repeat option '${o.text}' (use +5w, +2%, !last)`, o);
      }
      if (prog.wattsPerRep !== undefined || prog.pctPerRep !== undefined) node.progression = prog;
    }
    const open = this.next();
    if (open.kind !== "(") fail(`Expected '(' after '${countTok.text}'`, open);
    node.children = this.parseSeq(open);
    const close = this.next(); // ')'
    if (node.children.length === 0) fail("Empty repeat", { offset: countTok.offset, length: close.offset + 1 - countTok.offset });
    return node;
  }

  private parseStep(t: Tok, label: string | undefined): StepNode {
    let end: StepEnd;
    const w = t.text;
    const dur = parseDurationWord(w);
    let m: RegExpExecArray | null;
    if (dur !== null) {
      if (dur <= 0) fail("Duration must be > 0", t);
      end = { kind: "time", s: dur };
    } else if ((m = RE_KJ.exec(w))) {
      end = { kind: "kj", kj: Number(m[1]) };
    } else if (w.toLowerCase() === "lap") {
      end = { kind: "lap" };
    } else if ((m = RE_HR_END.exec(w))) {
      const rt = this.peek();
      const parts = rt.kind === "word" ? rt.text.split("..") : [];
      const lo = parts.length === 2 ? parseDurationWord(parts[0] as string) : null;
      const hi = parts.length === 2 ? parseDurationWord(parts[1] as string) : null;
      if (lo === null || hi === null) fail(`Expected min..max duration after '${w}', e.g. 2m..4m`, rt.kind === "word" ? rt : t);
      this.next();
      if ((lo as number) > (hi as number)) fail("Min duration is longer than max", rt);
      end = { kind: "hr_recovery", targetBpm: Number(m[1]), minS: lo as number, maxS: hi as number, onUnreachable: "lower_power" };
    } else if (/^\d+$/.test(w)) {
      return fail(`'${w}' needs a unit: 30s, 4m, 1h, 10kj — or '${w}x(...)' for a repeat`, t);
    } else {
      return fail(`Expected a duration (30s, 4m, 1m30s), kJ (10kj), 'lap', 'hr<N' or a repeat (3x(...)); got '${w}'`, t);
    }

    let target: PowerTarget | undefined;
    let targetTok: Tok | undefined;
    let cadence: { lo: number; hi: number } | undefined;
    const setTarget = (pt: PowerTarget, tok: Tok) => {
      if (target) fail("Duplicate power target", tok);
      target = pt;
      targetTok = tok;
    };
    while (this.peek().kind === "word") {
      const a = this.next();
      const x = a.text;
      if (x.toLowerCase() === "ramp") {
        const r = this.next();
        const rm = r.kind === "word" ? RE_RANGE.exec(r.text) : null;
        if (!rm) fail("Expected a range after 'ramp', e.g. ramp 50-75%", r.kind === "eof" ? a : r);
        setTarget({ kind: "ramp", from: Number((rm as RegExpExecArray)[1]), to: Number((rm as RegExpExecArray)[2]) }, r);
      } else if ((m = RE_RANGE.exec(x))) {
        const lo = Number(m[1]);
        const hi = Number(m[2]);
        if (lo > hi) fail("Range low end is above high end (use 'ramp 75-50%' for a descending ramp)", a);
        setTarget({ kind: "range", lo, hi }, a);
      } else if ((m = RE_PCT.exec(x))) {
        setTarget({ kind: "pct_ftp", value: Number(m[1]) }, a);
      } else if ((m = RE_WATTS.exec(x))) {
        setTarget({ kind: "watts", value: Number(m[1]) }, a);
      } else if ((m = RE_BPM.exec(x))) {
        setTarget({ kind: "hr", bpm: Number(m[1]) }, a);
      } else if (x.toLowerCase() === "free") {
        setTarget({ kind: "free" }, a);
      } else if ((m = RE_RPM.exec(x))) {
        if (cadence) fail("Duplicate cadence", a);
        const lo = Number(m[1]);
        const hi = m[2] !== undefined ? Number(m[2]) : lo;
        if (lo > hi) fail("Cadence range low end is above high end", a);
        cadence = { lo, hi };
      } else if (/^!(cap|lower|coach)$/i.test(x)) {
        if (end.kind !== "hr_recovery") fail(`'${x}' only applies to hr< recovery steps`, a);
        const k = x.slice(1).toLowerCase();
        (end as Extract<StepEnd, { kind: "hr_recovery" }>).onUnreachable = k === "cap" ? "cap" : k === "coach" ? "ask_coach" : "lower_power";
      } else {
        fail(`Unknown token '${x}'`, a);
      }
    }
    if (!target) return fail("Missing power target (e.g. 120%, 250w, 50-75%, ramp 50-75%, 140bpm, free)", t);
    const pct = maxPct(target, this.ftp);
    if (pct > 400) fail(`${Math.round(pct)} % FTP is implausible (max 400 %)`, targetTok as Tok);
    const node: StepNode = { type: "step", id: uid("s"), end, target };
    if (label !== undefined) node.label = label;
    if (cadence) node.cadence = cadence;
    return node;
  }
}

function maxPct(t: PowerTarget, ftp: number): number {
  switch (t.kind) {
    case "pct_ftp":
      return t.value;
    case "watts":
      return ftp > 0 ? (t.value / ftp) * 100 : 0;
    case "range":
      return t.hi;
    case "ramp":
      return Math.max(t.from, t.to);
    default:
      return 0;
  }
}

/** Parse DSL text into a fresh tree (new ids every call). Errors carry offset/length into `text`. */
export function parseDsl(text: string, ftp: number): { nodes: WorkoutNode[] } | { error: DslError } {
  try {
    return { nodes: new Parser(lex(text), ftp).parseTop() };
  } catch (e) {
    if (e instanceof DslFail) return { error: { message: e.message, offset: e.offset, length: e.length } };
    throw e;
  }
}

// ---------- printing ----------

const fmtNum = (x: number): string => String(Math.round(x * 1000) / 1000);

export function formatDuration(totalS: number): string {
  const s = Math.max(0, Math.round(totalS));
  if (s === 0) return "0s";
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  return `${h ? `${h}h` : ""}${m ? `${m}m` : ""}${sec ? `${sec}s` : ""}`;
}

const quote = (s: string) => `"${s.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}" `;

function printTarget(t: PowerTarget): string {
  switch (t.kind) {
    case "pct_ftp":
      return `${fmtNum(t.value)}%`;
    case "watts":
      return `${Math.round(t.value)}w`;
    case "range":
      return `${fmtNum(t.lo)}-${fmtNum(t.hi)}%`;
    case "ramp":
      return `ramp ${fmtNum(t.from)}-${fmtNum(t.to)}%`;
    case "hr":
      return `${Math.round(t.bpm)}bpm`;
    case "free":
      return "free";
  }
}

function printStep(s: StepNode): string {
  let end: string;
  let flag = "";
  switch (s.end.kind) {
    case "time":
      end = formatDuration(s.end.s);
      break;
    case "kj":
      end = `${fmtNum(s.end.kj)}kj`;
      break;
    case "lap":
      end = "lap";
      break;
    case "hr_recovery":
      end = `hr<${Math.round(s.end.targetBpm)} ${formatDuration(s.end.minS)}..${formatDuration(s.end.maxS)}`;
      flag = s.end.onUnreachable === "cap" ? " !cap" : s.end.onUnreachable === "ask_coach" ? " !coach" : "";
      break;
  }
  const cad = s.cadence ? ` ${s.cadence.lo === s.cadence.hi ? s.cadence.lo : `${s.cadence.lo}-${s.cadence.hi}`}rpm` : "";
  return `${s.label !== undefined ? quote(s.label) : ""}${end} ${printTarget(s.target)}${cad}${flag}`;
}

function printNode(n: WorkoutNode): string {
  if (n.type === "step") return printStep(n);
  const opts: string[] = [];
  const sign = (x: number) => (x >= 0 ? `+${fmtNum(x)}` : fmtNum(x));
  if (n.progression?.wattsPerRep !== undefined) opts.push(`${sign(n.progression.wattsPerRep)}w`);
  if (n.progression?.pctPerRep !== undefined) opts.push(`${sign(n.progression.pctPerRep)}%`);
  if (n.skipLastRest) opts.push("!last");
  const o = opts.length ? `[${opts.join(", ")}]` : "";
  return `${n.label !== undefined ? quote(n.label) : ""}${n.count}x${o}(${n.children.map(printNode).join(", ")})`;
}

/** Canonical text for a tree: top-level items one per line, nested items comma-separated. */
export function printDsl(nodes: readonly WorkoutNode[]): string {
  return nodes.map(printNode).join("\n");
}
