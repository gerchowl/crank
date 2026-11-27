// The in-page half of the WKWebView run (#630, runner.swift): the body of an async function called with `job` (a
// scenario of scenarios.ts), `audit` (layout-audit.js's source) and `opts`. It opens the fixture's state as design.pw.ts
// `open` does (picks, clicks, rows, fills, after, scrollTo, each followed by `settled()`), then returns the audit's result as JSON.
/* global job, audit, opts -- the async function's arguments (runner.swift `callAsyncJavaScript`). */
const until = async (what, ok) => {
  for (let t = 0; t < 200; t++) {
    const v = ok();
    if (v) return v;
    await new Promise((r) => setTimeout(r, 50));
  }
  throw new Error(`timed out waiting for ${what}`);
};
// By accessible name, as Playwright's `getByRole(role, { name })`: a case-insensitive substring of the label or text.
const named = (selector, name) =>
  [...document.querySelectorAll(selector)].find((e) =>
    (e.getAttribute("aria-label") ?? e.textContent)
      .toLowerCase()
      .includes(name.toLowerCase()),
  );
await until("the fixture", () => window.__fixture);
await window.__fixture.settled();
const press = async (selector, names) => {
  for (const name of names) {
    (await until(name, () => named(selector, name))).click();
    await window.__fixture.settled();
  }
};
await press("[role=option]", job.picks ?? []);
await press("button", job.clicks ?? []);
await press("[role=option]", job.rows ?? []);
// Fills as Playwright's `fill`. A text box is found by its label, as `getByRole("textbox", { name })`: an input by its
// `<label>`, the DSL text (CodeMirror's contenteditable, ADR-0011 §2.1) by the element its `aria-labelledby` names.
// An input gets its own value setter (React tracks the value it last rendered), then one input event; the
// contenteditable gets its content selected and replaced by `insertText`, the editing command a paste or a keystroke
// runs, which CodeMirror reads back from the DOM as it reads typing.
const labelled = (e, name) =>
  [
    ...(e.labels ?? []),
    ...(e.getAttribute("aria-labelledby") ?? "")
      .split(" ")
      .map((id) => document.getElementById(id))
      .filter(Boolean),
  ].some((l) => l.textContent.toLowerCase().includes(name.toLowerCase()));
for (const f of job.fills ?? []) {
  const box = await until(f.label, () =>
    [
      ...document.querySelectorAll("textarea, input, [contenteditable=true]"),
    ].find((e) => labelled(e, f.label)),
  );
  if (box.isContentEditable) {
    box.focus();
    document.execCommand("selectAll");
    document.execCommand("insertText", false, f.text);
  } else {
    const proto = Object.getPrototypeOf(box);
    Object.getOwnPropertyDescriptor(proto, "value").set.call(box, f.text);
    box.dispatchEvent(new Event("input", { bubbles: true }));
  }
  await window.__fixture.settled();
}
await press("button", job.after ?? []);
if (job.scrollTo) {
  document.querySelector(job.scrollTo)?.scrollIntoView({ block: "nearest" });
  await window.__fixture.settled();
}
(0, eval)(audit);
return JSON.stringify(window.__layoutAudit({ ...opts, full: true }));
