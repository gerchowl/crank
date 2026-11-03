// Shared deterministic sim data: FTP 250 W, "Sensor check (3 min)" = 1:00 @ 125 W, 1:00 @ 200 W, 1:00 @ 100 W.
// The ride is frozen at 1:22 (22 s into "Work", 0:38 left).
window.SIM = (() => {
  const FTP = 250, NOW = 82;
  const blocks = [
    { name: "Warm-up", phase: "warmup", s: 0, d: 60, w: 125 },
    { name: "Work", phase: "work", s: 60, d: 60, w: 200 },
    { name: "Cool-down", phase: "cooldown", s: 120, d: 60, w: 100 },
  ];
  const n = (t, a, f) => Math.sin(t * f) * a + Math.sin(t * f * 2.7 + 1.3) * a * 0.5;
  const target = (t) => blocks.find((b) => t >= b.s && t < b.s + b.d)?.w ?? 100;
  const series = [];
  for (let t = 0; t <= NOW; t += 1) {
    const tg = target(t);
    // 5 s soft start into Work
    const ss = t >= 60 && t < 65 ? 125 + (200 - 125) * ((t - 60) / 5) : tg;
    const p = Math.round(ss + n(t, 5, 0.9));
    const hr = Math.round(98 + (t < 60 ? t * 0.25 : 15 + (t - 60) * 0.95) + n(t, 0.8, 0.4));
    const cad = Math.round(88.5 + n(t, 1.6, 0.7));
    series.push({ t, tg, p, hr, cad });
  }
  const last = series[series.length - 1];
  const workouts = [
    { id: "builtin-sensor-check", name: "Sensor check (3 min)", dur: 180, tss: 3, if: 0.62, tag: "built-in", dsl: '"Warm-up" 1m 50%, "Work" 1m 80%, "Cool-down" 1m 40%', blocks: [[60, 0.5], [60, 0.8], [60, 0.4]] },
    { id: "builtin-endurance-45", name: "Endurance 45", dur: 2700, tss: 33, if: 0.64, tag: "built-in", dsl: '"Warm-up" 5m ramp 40-60%, "Endurance" 35m 65%, "Cool-down" 5m ramp 60-40%', blocks: [[300, 0.4, 0.6], [2100, 0.65], [300, 0.6, 0.4]] },
    { id: "builtin-sweet-spot-2x10", name: "Sweet spot 2x10", dur: 2700, tss: 47, if: 0.79, tag: "built-in", dsl: '"Warm-up" 10m ramp 45-70%, 2x("Sweet spot" 10m 90%, "Rest" 5m 55%), "Cool-down" 5m ramp 60-40%', blocks: [[600, 0.45, 0.7], [600, 0.9], [300, 0.55], [600, 0.9], [300, 0.55], [300, 0.6, 0.4]] },
  ];
  const fmt = (s) => `${Math.floor(s / 60)}:${String(Math.round(s % 60)).padStart(2, "0")}`;
  return { FTP, NOW, blocks, series, last, workouts, fmt, target };
})();
