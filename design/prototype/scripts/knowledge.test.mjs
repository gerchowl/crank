// pnpm test:knowledge — the build-time check of the Why?/Background layer (DESIGN §15.3), in prototype
// form. The UI reads the knowledge pack itself (src/knowledge.ts); this fails unless every rule and every
// citation it can show resolves: rule refs and topic-file citations exist in references.bib, every
// reference has a DOI or PMID or OpenAlex ID, every grade is A–D and every rule ID the prototype names
// exists. It also compiles every wish-matrix cell at every time choice (DESIGN §15.2), and checks that the
// planner's reasons name only inputs crank has and rest on A/B rules (a cut-off may be D, labelled).
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";

const root = fileURLToPath(new URL("..", import.meta.url));
const pack = fileURLToPath(new URL("../../../spec/knowledge-pack/", import.meta.url));
const results = [];
const check = (name, ok, detail = "") => results.push({ name, ok, detail });
const server = await createServer({ root, logLevel: "error", server: { middlewareMode: true } });

try {
  const k = await server.ssrLoadModule("/src/knowledge.ts");
  const w = await server.ssrLoadModule("/src/mock/wish.ts");
  const ruleIds = new Set(k.rules.map((r) => r.id));
  const declared = (readFileSync(pack + "rules.md", "utf8").match(/^- id: /gm) ?? []).length;
  check("every rule in rules.md parses (id, scope, rule, rationale, grade, refs)", k.rules.length === declared && declared > 0, `${k.rules.length} of ${declared}`);
  check("grades A–D each have a meaning and a treatment (README table)", ["A", "B", "C", "D"].every((g) => k.GRADES[g]?.meaning && k.GRADES[g]?.treatment));

  const missingRef = k.rules.flatMap((r) => r.refs.filter((key) => !k.refs.has(key)).map((key) => `${r.id} → ${key}`));
  check("every rule citation resolves in references.bib", missingRef.length === 0, missingRef.join(", "));
  const topicCites = readdirSync(pack)
    .filter((f) => /^\d\d-.*\.md$/.test(f))
    .flatMap((f) => [...readFileSync(pack + f, "utf8").matchAll(/@([a-z][\w]*)/g)].map((m) => [f, m[1]]));
  const missingCite = topicCites.filter(([, key]) => !k.refs.has(key)).map(([f, key]) => `${f} → ${key}`);
  check("every topic-file citation resolves in references.bib", missingCite.length === 0, [...new Set(missingCite)].join(", "));
  const unlinked = [...k.refs.values()].filter((r) => !r.doi && !r.pmid && !/OpenAlex: W\d+/.test(readFileSync(pack + "references.bib", "utf8").split(`{${r.key},`)[1]?.split("\n@")[0] ?? ""));
  check("every reference has a DOI, PMID or OpenAlex ID", unlinked.length === 0, unlinked.map((r) => r.key).join(", "));
  check("every topic has a title, an overall confidence and graded findings", k.topics.length === 13 && k.topics.every((t) => t.title && t.confidence && t.findings.length > 0), k.topics.filter((t) => !t.findings.length).map((t) => t.no).join(","));
  check("every rule belongs to a topic (general R-GEN rules may cut across)", k.rules.every((r) => r.topics.length > 0 || r.id.startsWith("R-GEN-")), k.rules.filter((r) => !r.topics.length && !r.id.startsWith("R-GEN-")).map((r) => r.id).join(", "));

  const src = (dir) => readdirSync(dir, { withFileTypes: true }).flatMap((d) => (d.isDirectory() ? src(`${dir}/${d.name}`) : /\.tsx?$/.test(d.name) ? [`${dir}/${d.name}`] : []));
  const named = [...new Set(src(root + "src").flatMap((f) => readFileSync(f, "utf8").match(/R-[A-Z]+-\d\d/g) ?? []))];
  const unknown = named.filter((id) => !ruleIds.has(id));
  check("every rule ID the UI names exists in rules.md", unknown.length === 0 && named.length > 0, unknown.join(", ") || `${named.length} named`);

  const bad = [];
  for (const intensity of w.INTENSITIES)
    for (const format of w.FORMATS) {
      const c = { intensity, format };
      if (!w.coherent(c)) continue;
      for (const m of [30, 45, 60, 90, 120, 180])
        try {
          w.proposalFor(c, m, 280);
        } catch (e) {
          bad.push(`${intensity}.${format} @ ${m} min: ${e.message}`);
        }
    }
  check("every wish cell compiles at every time choice (30 min – 3 h)", bad.length === 0, bad.join("; "));

  // "What crank uses here" may only name inputs crank has (§15.3 check 5): never HRV or α1
  const inputs = Object.keys(w.INPUTS);
  check("crank's input registry names nothing it can't measure (HRV, α1, lactate)", !inputs.some((i) => /hrv|alpha|α1|dfa|lactate/i.test(i + w.INPUTS[i])));
  const reasons = [];
  for (const wish of [null, ...w.ALL_CELLS])
    for (const [legs, sleep] of [[1, 2], [3, 4], [3, 1]])
      for (const minutes of [45, 120])
        for (const capped of [false, true]) reasons.push(...w.planWish(wish, { legs, sleep, minutes, capped, coachPick: { intensity: "tempo", format: "intervals" } }).reasons);
  const effects = [w.profileFixture, w.defaultProfile, { ...w.profileFixture, returning: { since: "x" } }, { ...w.profileFixture, focus: "racing" }].flatMap((p) => w.profileEffects(p));
  const wrongUse = reasons.filter((r) => r.uses.some((u) => !inputs.includes(u)));
  const wrongRule = [...reasons, ...effects].filter((r) => r.rule && !ruleIds.has(r.rule));
  check("every planner reason uses only registered inputs and resolvable rules", wrongUse.length === 0 && wrongRule.length === 0 && reasons.length > 0, [...wrongUse, ...wrongRule].map((r) => r.text).join("; "));
  const looseCutoff = reasons.filter((r) => r.kind !== "safety" && r.rule && k.rules.find((x) => x.id === r.rule)?.grade > "B");
  check("a deviation rests on A/B evidence; its cut-off may be D and says so", looseCutoff.length === 0 && reasons.filter((r) => r.cutoff).every((r) => r.cutoff.grade && r.cutoff.what), looseCutoff.map((r) => r.rule).join(", "));
} finally {
  await server.close();
}
for (const r of results) console.log(`${r.ok ? "✓" : "✗"} ${r.name}${r.detail ? ` (${r.detail})` : ""}`);
const failed = results.filter((r) => !r.ok).length;
console.log(`${results.length - failed}/${results.length} passed`);
if (failed) process.exit(1);
