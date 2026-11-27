// Real typing in the editor's text box (#759, ADR-0011 §2.1, §2.2): jsdom cannot type into CodeMirror's contenteditable
// (the Vitest tests use the `setText` seam), so the harness's Chromium does, on the `workouts-editor` fixture (the core's
// `sweet-spot` draft; the mock parses only the texts of editor.json, so each step types its way to one of them). One
// project is enough: the keys, not the layout, are under test, and the real app's WKWebView runs the same in `just e2e`.
import { expect, test, type Page } from "@playwright/test";

test.beforeEach(() => {
  test.skip(
    test.info().project.name !== "1024x768-light",
    "keys, not layout: one project",
  );
});

async function openEditor(page: Page) {
  await page.goto("/?scenario=workouts-editor");
  await page.getByRole("option", { name: "Sweet spot 3x12" }).first().click();
  await page.getByRole("button", { name: "Duplicate & edit" }).click();
  const box = page.getByRole("textbox", { name: "Text" });
  await expect(box).toBeVisible();
  await page.evaluate(() => window.__fixture.settled());
  return box;
}
const repeatRow = (page: Page) =>
  page.getByRole("listbox", { name: "Outline" }).getByRole("option").nth(1);
/**
 * Select the repeat's count, the first character of the second line, through the DOM selection (CodeMirror reads it on
 * `selectionchange`, as after a click): the keys that move to a line's start differ per platform.
 */
async function selectCount(page: Page) {
  await page.evaluate(() => {
    const line = document.querySelectorAll("#editor-dsl .cm-line")[1]!;
    const walk = document.createTreeWalker(line, NodeFilter.SHOW_TEXT);
    const node = walk.nextNode()!;
    getSelection()!.setBaseAndExtent(node, 0, node, 1);
  });
  await expect
    .poll(() => page.evaluate(() => getSelection()!.toString()))
    .toBe("3");
}

test("typed keys reach the core; Cmd/Ctrl+Z in the box undoes the burst in the text and the outline", async ({
  page,
}) => {
  const box = await openEditor(page);
  await box.click();
  await selectCount(page);
  await page.keyboard.type("4");
  await expect(repeatRow(page)).toHaveText("4 ×");
  await expect(box).toHaveText(/^"Warm-up" 10m ramp 45-70%4x\(/);
  await page.keyboard.press("ControlOrMeta+z");
  await expect(repeatRow(page)).toHaveText("3 ×");
  await expect(box).toBeFocused();
  await page.keyboard.press("ControlOrMeta+Shift+z");
  await expect(repeatRow(page)).toHaveText("4 ×");
  // Tab leaves the text box (ADR-0011 §2.7): it is not captured, and the text is unchanged.
  await page.keyboard.press("Tab");
  await expect(box).not.toBeFocused();
  await expect(repeatRow(page)).toHaveText("4 ×");
});

test("a composition reaches the model when it ends, never part-way", async ({
  page,
}) => {
  const box = await openEditor(page);
  await box.click();
  await selectCount(page);
  // An IME composition over the selected "3" (CDP's `Input.imeSetComposition`, as an input method drives the page).
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Input.imeSetComposition", {
    text: "4",
    selectionStart: 1,
    selectionEnd: 1,
  });
  // The composing "4" is on screen, but nothing was sent: the outline still shows the saved tree.
  await expect(box).toHaveText(/4x\(/);
  await page.evaluate(() => window.__fixture.settled());
  await expect(repeatRow(page)).toHaveText("3 ×");
  await cdp.send("Input.insertText", { text: "4" });
  await expect(repeatRow(page)).toHaveText("4 ×");
  await expect(box).toHaveText(/4x\(/);
});
