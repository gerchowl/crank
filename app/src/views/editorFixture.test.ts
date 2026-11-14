// The editor mock answers only with the core's answers (editorFixture.ts, #111).
import { describe, expect, it } from "vitest";
import type { WorkoutNode } from "../bindings";
import { DRAFTS, editorCommands } from "./editorFixture";

const text = (name: string) => DRAFTS[name]!.text;
const ids = (nodes: readonly WorkoutNode[]): string[] =>
  nodes.flatMap((n) => [n.id, ...(n.type === "repeat" ? ids(n.children) : [])]);

describe("the editor mock", () => {
  it("gives every parse fresh dsl ids, as the core", () => {
    const ed = editorCommands();
    const a = ed.parse_dsl(text("sweet-spot"), 250);
    const b = ed.parse_dsl(text("sweet-spot"), 250);
    if (a.outcome !== "parsed" || b.outcome !== "parsed")
      throw new Error("parses");
    expect(ids(a.nodes).every((id) => id.startsWith("dsl-"))).toBe(true);
    expect(ids(b.nodes).some((id) => ids(a.nodes).includes(id))).toBe(false);
    expect(ed.print_dsl(b.nodes)).toBe(text("sweet-spot"));
  });

  it("answers a text that does not parse with the core's problem", () => {
    const ed = editorCommands();
    const r = ed.parse_dsl(text("missing-target"), 250);
    expect(r.outcome).toBe("invalid");
    if (r.outcome === "invalid")
      expect(r.problem.message).toMatch(/power target/);
  });

  it("compiles with the request's ids and refuses what the core refuses", () => {
    const ed = editorCommands();
    const r = ed.parse_dsl(text("sweet-spot"), 250);
    if (r.outcome !== "parsed") throw new Error("parses");
    const view = ed.compile_workout({
      id: "d",
      name: "D",
      ftp: 250,
      nodes: r.nodes,
    });
    expect(view.blocks.length).toBeGreaterThan(0);
    expect(view.blocks.every((b) => ids(r.nodes).includes(b.nodeId))).toBe(
      true,
    );
    const long = ed.parse_dsl(text("too-long"), 250);
    if (long.outcome !== "parsed") throw new Error("parses");
    expect(() =>
      ed.compile_workout({ id: "d", name: "D", ftp: 250, nodes: long.nodes }),
    ).toThrow(expect.objectContaining({ code: "invalid_input" }) as Error);
  });

  it("rejects a null FTP as Tauri does, before the core", () => {
    const ed = editorCommands();
    expect(() => ed.parse_dsl(text("sweet-spot"), null)).toThrow(
      /expected f64/,
    );
    expect(() =>
      ed.compile_workout({ id: "d", name: "D", ftp: null, nodes: [] }),
    ).toThrow(/expected f64/);
  });

  it("never guesses: a draft not in the core's file breaks the test", () => {
    const ed = editorCommands();
    expect(() => ed.parse_dsl("1m 50%", 250)).toThrow(/not in editor.json/);
    expect(() => ed.print_dsl([])).not.toThrow();
    expect(() =>
      ed.print_dsl([
        {
          type: "step",
          id: "x",
          end: { kind: "lap" },
          target: { kind: "free" },
        } as WorkoutNode,
      ]),
    ).toThrow(/no draft/);
  });
});
