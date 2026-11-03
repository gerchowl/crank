// The Why? layer and Background read the knowledge pack itself (DESIGN §15.3): no rule, grade or citation
// is copied into the UI. Production generates the same bundle at build time and fails on any key that does
// not resolve (§15.3 "Build-time check"); `pnpm test:knowledge` is that check for the prototype.
import readme from "../../../spec/knowledge-pack/README.md?raw";
import rulesMd from "../../../spec/knowledge-pack/rules.md?raw";
import bib from "../../../spec/knowledge-pack/references.bib?raw";

const topicFiles = import.meta.glob<string>("../../../spec/knowledge-pack/[0-9][0-9]-*.md", { query: "?raw", import: "default", eager: true });

export type Grade = "A" | "B" | "C" | "D";
export interface Rule { id: string; scope: string; rule: string; rationale: string; grade: Grade; refs: string[]; topics: string[] }
export interface Ref { key: string; authors: string; year: string; title: string; journal: string; doi: string | null; pmid: string | null }
export interface Finding { grade: string; headline: string; cites: number }
export interface Topic { no: string; title: string; confidence: string; findings: Finding[] }

/** Grade meaning and how the app treats it: the README's table, verbatim. */
export const GRADES = Object.fromEntries(
  [...readme.matchAll(/^\| \*\*([A-D])\*\* \| (.+?) \| (.+?) \|$/gm)].map((m) => [m[1], { meaning: m[2] ?? "", treatment: (m[3] ?? "").replaceAll("`", "") }]),
) as Record<Grade, { meaning: string; treatment: string }>;
/** One word per grade for chips; the meaning above is always one tap away. */
export const GRADE_WORD: Record<Grade, string> = { A: "strong", B: "good", C: "limited", D: "convention" };

/** Rules belong to the topics in their own `# (NN)` note, else to their section's `(NN)`; general rules to none. */
export const rules: Rule[] = rulesMd.split(/^# -+ /m).flatMap((section) => {
  const sectionTopics = /^[^\n]*\((\d\d)\)/.exec(section)?.[1];
  return [...section.matchAll(/- id: (\S+)\n {2}scope: (\S+)\n {2}rule: (.+)\n {2}rationale: (.+)\n {2}grade: ([A-D])\n {2}basis: \S+\n {2}refs: \[(.*?)\](.*)/g)].map((m) => {
    const own = [...(m[7] ?? "").matchAll(/\d\d/g)].map((t) => t[0]);
    return {
      id: m[1] ?? "",
      scope: m[2] ?? "",
      rule: m[3] ?? "",
      rationale: m[4] ?? "",
      grade: (m[5] ?? "D") as Grade,
      refs: (m[6] ?? "").split(",").map((r) => r.trim()).filter(Boolean),
      topics: own.length ? own : sectionTopics ? [sectionTopics] : [],
    };
  });
});

const field = (entry: string, name: string) => new RegExp(`\\b${name} = \\{(.+?)\\},?\\n`).exec(entry)?.[1]?.replace(/[{}]/g, "").replace(/\\['"`^~]/g, "").replace(/\\(\w)/g, "$1") ?? null;
export const refs = new Map<string, Ref>(
  bib.split(/\n@/).slice(1).map((e) => {
    const key = /^\w+\{([^,]+),/.exec(e)?.[1] ?? "";
    const authors = (field(e, "author") ?? "").split(" and ");
    const first = authors[0]?.split(",")[0] ?? "";
    return [key, { key, authors: authors.length > 2 ? `${first} et al.` : authors.map((a) => a.split(",")[0]).join(" & "), year: field(e, "year") ?? "", title: field(e, "title") ?? "", journal: field(e, "journal") ?? "", doi: field(e, "doi"), pmid: field(e, "pmid") }];
  }),
);

export const topics: Topic[] = Object.entries(topicFiles)
  .sort(([a], [b]) => a.localeCompare(b))
  .map(([path, md]) => ({
    no: /\/(\d\d)-/.exec(path)?.[1] ?? "",
    title: /^# \d\d · (.+)$/m.exec(md)?.[1] ?? path,
    confidence: /^\*\*(?:Overall confidence|Bottom line):\*\* (.+)$/m.exec(md)?.[1] ?? "",
    // a finding is its bullet plus its indented sub-bullets; sources are counted over both
    findings: md
      .split(/^(?=- \*\*\[)/m)
      .slice(1)
      .map((block) => block.split(/\n(?=\S)/)[0] ?? "")
      .flatMap((b) => {
        const m = /^- \*\*\[([A-D][^\]]*)\] (.+?)\*\*/.exec(b);
        return m ? [{ grade: m[1] ?? "", headline: (m[2] ?? "").replace(/\.$/, ""), cites: new Set(b.match(/@\w+/g)).size }] : [];
      }),
  }));

/** Rules belong to a topic by their `# (NN)` note in rules.md. */
export const rulesOf = (no: string) => rules.filter((r) => r.topics.includes(no));
export function rule(id: string): Rule {
  const r = rules.find((x) => x.id === id);
  if (!r) throw new Error(`knowledge pack: unknown rule ${id}`); // the build-time check makes this unreachable
  return r;
}
export const compiled = /Pack compiled (\d{4}-\d\d-\d\d)/.exec(readme)?.[1] ?? "";
