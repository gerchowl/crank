// Ride sounds (DESIGN §7.3): short Web Audio tones, no assets. Interactive mode only (scenes are
// silent). Every sound duplicates something that is also on screen.
import { useEffect, useRef } from "react";
import type { CoachNudge, CompiledWorkout, RideStatus } from "@crank/shared";
import { sound, type SoundEvent } from "./persist";
import { sceneId } from "./session";

let ctx: AudioContext | null = null;
function tones(event: SoundEvent, notes: [freq: number, startS: number, durS: number][]) {
  const cfg = sound.get();
  if (sceneId || cfg.muted.includes(event) || cfg.volume <= 0) return;
  ctx ??= new AudioContext();
  const t0 = ctx.currentTime + 0.02;
  for (const [f, start, dur] of notes) {
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.type = "sine";
    osc.frequency.value = f;
    gain.gain.setValueAtTime(0, t0 + start);
    gain.gain.linearRampToValueAtTime(0.25 * cfg.volume, t0 + start + 0.01);
    gain.gain.exponentialRampToValueAtTime(0.0001, t0 + start + dur);
    osc.connect(gain).connect(ctx.destination);
    osc.start(t0 + start);
    osc.stop(t0 + start + dur + 0.02);
  }
}

export const play = {
  block: (harder: boolean | null) => tones("block", harder === null ? [[660, 0, 0.25]] : harder ? [[523, 0, 0.18], [784, 0.16, 0.25]] : [[784, 0, 0.18], [523, 0.16, 0.25]]),
  tick: () => tones("countdown", [[1000, 0, 0.06]]),
  ask: () => tones("coach", [[587, 0, 0.15], [880, 0.15, 0.22]]),
  applied: () => tones("applied", [[700, 0, 0.2]]),
  undone: () => tones("applied", [[392, 0, 0.25]]),
  warning: () => tones("warning", [[330, 0, 0.22], [262, 0.24, 0.3]]),
  reconnected: () => tones("reconnected", [[880, 0, 0.2]]),
  /** §7.3: the planner answered a wish — granted (three rising notes) or changed (two short + one low) */
  wish: (granted: boolean) => tones("wish", granted ? [[523, 0, 0.1], [659, 0.1, 0.1], [784, 0.2, 0.22]] : [[659, 0, 0.1], [659, 0.14, 0.1], [440, 0.28, 0.28]]),
};

/** Fire the §7.3 table from ride state changes. */
export function useRideSounds(status: RideStatus | null, compiled: CompiledWorkout | null, nudges: CoachNudge[], blockLeftS: number | null, alarm: boolean) {
  const prev = useRef<{ block: number; nudge: string; alarm: boolean; tick: number } | null>(null);
  useEffect(() => {
    if (!status || !compiled) return;
    const last = nudges.at(-1);
    const nudgeKey = last ? `${last.callId}:${last.status}` : "";
    const p = prev.current;
    if (p) {
      if (status.blockIndex !== p.block) {
        const a = compiled.blocks[p.block];
        const b = compiled.blocks[status.blockIndex];
        play.block(a && b ? (b.powerFrom === a.powerTo ? null : b.powerFrom > a.powerTo) : null);
      }
      if (nudgeKey !== p.nudge && last) {
        if (last.status === "pending") play.ask();
        else if (last.status === "auto_applied" || last.status === "accepted") play.applied();
      }
      if (alarm && !p.alarm) play.warning();
      if (!alarm && p.alarm) play.reconnected();
    }
    const tick = blockLeftS !== null && blockLeftS > 0 && blockLeftS <= 3.2 && status.state === "riding" ? Math.ceil(blockLeftS) : 0;
    if (tick && tick !== p?.tick) play.tick();
    prev.current = { block: status.blockIndex, nudge: nudgeKey, alarm, tick };
  }, [status, compiled, nudges, blockLeftS, alarm]);
  useEffect(() => {
    if (!alarm) return;
    const id = setInterval(() => play.warning(), 15000);
    return () => clearInterval(id);
  }, [alarm]);
}
