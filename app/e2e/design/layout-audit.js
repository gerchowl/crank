// crank's DOM layout audit (#457, from the 2026-11-02 walkthrough of #438). Browser-side: defines
// `window.__layoutAudit(opts)`, which returns the violations per rule, their counts and the worst offenders.
// Inject it with `page.addScriptTag({ path })`, then `page.evaluate((o) => __layoutAudit(o), opts)`.
//
// Direction A (DESIGN §18): the scales are read from `theme.css` on `:root` (`--space-*`, `--text-*`, `--tier-*`,
// `--control-h*`), so the audit follows the tokens. Each rule names the §18.7 design rule it checks.
// opts:
//   controls  [{ selector, token }]: where a control may be another `--control-h*` height than `--control-h`
//             (32 px, §18.4). Default: none; the spec passes the rail's `--control-h-rail`. A control under an
//             allowance is its own height variant (`<variant>@<token>`), so it never clashes with the default.
//   keyed     selector of the ride and saddle actions that show their key in a `kbd` (rule 9). Default ".ride button".
//   rail      selector inside which icons are 20 px instead of 16 (rule 4). Default ".rail".
//   metrics   the metric column's grid (DESIGN §18.3 "Metric grid", rule 6): { slot, label, value, unit } selectors.
//             Default: the ride's `.ride-metrics > .readout`, `.readout-head`, `[data-tier]`, `.readout-value > .tile-unit`.
//   strip     the block strip's labels (§18.3 "Strip labels", rule 6): { label, block, bar } selectors; a label's
//             `data-block-label` names the `data-block` it belongs to. Default: the ride's timeline.
//   top       offenders kept per rule (default 3); full: keep every message under `rules`.
(() => {
  const sel = (el) => {
    if (el.id) return `#${el.id}`;
    const parts = [];
    for (
      let e = el;
      e && e.nodeType === 1 && parts.length < 4;
      e = e.parentElement
    ) {
      let s = e.tagName.toLowerCase();
      const cls = [...e.classList].slice(0, 2).join(".");
      if (cls) s += "." + cls;
      for (const a of ["data-tile", "data-slot", "aria-label", "data-testid"]) {
        const v = e.getAttribute(a);
        if (v) {
          s += `[${a}="${v.slice(0, 24)}"]`;
          break;
        }
      }
      parts.unshift(s);
      if (e.id) {
        parts[0] = `#${e.id}`;
        break;
      }
    }
    return parts.join(" > ");
  };
  const visible = (el) => {
    const r = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    return (
      r.width > 0 &&
      r.height > 0 &&
      cs.visibility !== "hidden" &&
      cs.display !== "none" &&
      +cs.opacity > 0
    );
  };
  const px = (v) => parseFloat(v) || 0;
  /** The px values of the `:root` custom properties whose names match `re`, by name. */
  const tokens = (re) => {
    const root = getComputedStyle(document.documentElement);
    const names = new Set();
    for (const s of document.styleSheets)
      for (const r of s.cssRules)
        if (r.style) for (const p of r.style) if (re.test(p)) names.add(p);
    return Object.fromEntries(
      [...names].map((n) => [n, px(root.getPropertyValue(n))]),
    );
  };
  // --- colour + contrast ---
  const parse = (c) => {
    const m = c.match(/rgba?\(([^)]+)\)/);
    if (!m) return null;
    const p = m[1]
      .split(/[ ,/]+/)
      .filter(Boolean)
      .map(Number);
    return { r: p[0], g: p[1], b: p[2], a: p.length > 3 ? p[3] : 1 };
  };
  const lum = ({ r, g, b }) => {
    const f = (v) => {
      v /= 255;
      return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
    };
    return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
  };
  const over = (fg, bg) => ({
    r: fg.r * fg.a + bg.r * (1 - fg.a),
    g: fg.g * fg.a + bg.g * (1 - fg.a),
    b: fg.b * fg.a + bg.b * (1 - fg.a),
    a: 1,
  });
  const bgOf = (el) => {
    const stack = [];
    for (let e = el; e; e = e.parentElement) {
      const c = parse(getComputedStyle(e).backgroundColor);
      if (c && c.a > 0) {
        stack.push(c);
        if (c.a >= 1) break;
      }
    }
    let bg = { r: 255, g: 255, b: 255, a: 1 };
    for (let i = stack.length - 1; i >= 0; i--) bg = over(stack[i], bg);
    return bg;
  };
  const ratio = (a, b) => {
    const [x, y] = [lum(a), lum(b)].sort((m, n) => n - m);
    return (x + 0.05) / (y + 0.05);
  };

  window.__layoutAudit = function (opts = {}) {
    const vw = document.documentElement.clientWidth;
    const out = {
      viewport: `${innerWidth}x${innerHeight}`,
      rules: {},
      counts: {},
      worst: {},
    };
    const add = (rule, el, msg) => {
      (out.rules[rule] ||= []).push(`${sel(el)} :: ${msg}`);
    };
    const all = [...document.querySelectorAll("body *")].filter(visible);
    const space = tokens(/^--space-/);
    const SCALE = new Set([0, ...Object.values(space)]);
    const TYPE = new Set(Object.values(tokens(/^--(text|tier)-/)));
    const control = tokens(/^--control-h/);
    const TIER = tokens(/^--tier-/);
    out.scales = {
      space: [...SCALE].sort((a, b) => a - b),
      type: [...TYPE].sort((a, b) => a - b),
      control,
    };

    // 1. Buttons. Rule 9 / §18.4: exactly `--control-h` unless an allowance names another control height.
    const buttons = all.filter((e) =>
      e.matches(
        "button,[role=button],a.button,input[type=button],input[type=submit]",
      ),
    );
    const heights = {};
    for (const b of buttons) {
      const r = b.getBoundingClientRect();
      const cs = getComputedStyle(b);
      const h = Math.round(r.height);
      const allowances = (opts.controls ?? []).filter((c) =>
        b.matches(c.selector),
      );
      const variant =
        ([...b.classList]
          .filter((c) => /primary|ride-btn|secondary|ghost|icon/.test(c))
          .join(".") || "default") +
        allowances.map((c) => `@${c.token}`).join("");
      (heights[variant] ||= new Set()).add(h);
      const allowed = allowances.map((c) => control[c.token]);
      const want = allowed.length ? allowed : [control["--control-h"]];
      if (!want.includes(h))
        add("button.height", b, `${h}px not ${want.join("/")}`);
      for (const side of ["Top", "Right", "Bottom", "Left"]) {
        const v = Math.round(px(cs["padding" + side]));
        if (!SCALE.has(v)) {
          add(
            "button.padding",
            b,
            `padding-${side.toLowerCase()} ${v}px off-scale`,
          );
          break;
        }
      }
      const parentW = b.parentElement.getBoundingClientRect().width;
      if (r.width > 320 && r.width >= parentW - 2)
        add("button.fullWidth", b, `${Math.round(r.width)}px full-width`);
      if (b.matches(opts.keyed ?? ".ride button") && !b.querySelector("kbd"))
        add("button.noKey", b, "no kbd hint");
    }
    for (const [v, s] of Object.entries(heights))
      if (s.size > 1)
        out.rules["button.inconsistentHeight"] = [
          ...(out.rules["button.inconsistentHeight"] || []),
          `variant ${v}: ${[...s].sort((a, b) => a - b).join("/")}px`,
        ];

    // 2. Spacing (rule 2): margins/gaps off the `--space-*` scale, inconsistent sibling gaps
    for (const e of all) {
      const cs = getComputedStyle(e);
      for (const p of [
        "marginTop",
        "marginBottom",
        "marginLeft",
        "marginRight",
        "rowGap",
        "columnGap",
      ]) {
        const raw = cs[p];
        if (raw === "normal" || raw === "auto") continue;
        const v = Math.abs(Math.round(px(raw)));
        if (v && !SCALE.has(v)) {
          add("spacing.offScale", e, `${p} ${raw}`);
          break;
        }
      }
      // A data graphic's marks (a trace's plan blocks) are drawn at their data's positions, not laid out.
      const kids = [...e.children].filter(visible);
      if (
        kids.length >= 3 &&
        /flex|grid/.test(cs.display) === false &&
        !e.closest("svg[data-graphic]")
      ) {
        const gaps = [];
        for (let i = 1; i < kids.length; i++)
          gaps.push(
            Math.round(
              kids[i].getBoundingClientRect().top -
                kids[i - 1].getBoundingClientRect().bottom,
            ),
          );
        const pos = gaps.filter((g) => g >= 0);
        if (pos.length >= 2 && Math.max(...pos) - Math.min(...pos) > 8)
          add(
            "spacing.inconsistentSiblings",
            e,
            `vertical gaps ${pos.join(",")}`,
          );
      }
    }

    // 3. Typography: sizes on the type scale (rule 2), exact tiers (rule 16), tabular numbers (rule 5)
    const textEls = all.filter((e) =>
      [...e.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim()),
    );
    const sizes = new Set(),
      lhRatios = new Set(),
      weights = new Set();
    for (const e of textEls) {
      const cs = getComputedStyle(e);
      const fs = px(cs.fontSize);
      sizes.add(fs);
      weights.add(cs.fontWeight);
      if (cs.lineHeight !== "normal")
        lhRatios.add((px(cs.lineHeight) / fs).toFixed(2));
      if (fs < 12) add("type.tooSmall", e, `${fs}px`);
      if (!TYPE.has(fs)) add("type.offScale", e, `${fs}px off the type scale`);
      const txt = [...e.childNodes]
        .filter((n) => n.nodeType === 3)
        .map((n) => n.textContent)
        .join("")
        .trim();
      if (/\d/.test(txt) && !/tabular-nums/.test(cs.fontVariantNumeric))
        add(
          "type.noTabularNums",
          e,
          `"${txt.slice(0, 20)}" ${cs.fontVariantNumeric}`,
        );
    }
    for (const e of all.filter((x) => x.matches("[data-tier]"))) {
      const want = TIER[`--tier-${e.dataset.tier}`];
      const fs = px(getComputedStyle(e).fontSize);
      if (fs !== want)
        add("type.tier", e, `${e.dataset.tier} ${fs}px not ${want}px`);
    }
    out.typeScale = [...sizes].sort((a, b) => a - b);
    out.lineHeightRatios = [...lhRatios];
    out.fontWeights = [...weights];

    // 4. Layout (rule 6)
    if (document.documentElement.scrollWidth > vw + 1)
      out.rules["layout.hOverflow"] = [
        `scrollWidth ${document.documentElement.scrollWidth} > ${vw}`,
      ];
    for (const e of all) {
      const p = e.parentElement;
      if (!p || p === document.body) continue;
      const pcs = getComputedStyle(p);
      if (/hidden|clip|auto|scroll/.test(pcs.overflowX + pcs.overflowY)) {
        if (
          e.scrollWidth > e.clientWidth + 1 &&
          getComputedStyle(e).overflow === "hidden"
        )
          add(
            "layout.clipped",
            e,
            `content ${e.scrollWidth} > box ${e.clientWidth}`,
          );
        continue;
      }
      const r = e.getBoundingClientRect(),
        pr = p.getBoundingClientRect();
      if (r.right > pr.right + 2 || r.left < pr.left - 2)
        add(
          "layout.overflowParent",
          e,
          `x ${Math.round(r.left)}–${Math.round(r.right)} vs parent ${Math.round(pr.left)}–${Math.round(pr.right)}`,
        );
    }
    const main = document.querySelector("main .page, main .ride, main > *");
    if (main) {
      const r = main.getBoundingClientRect();
      out.contentBox = `${Math.round(r.left)}..${Math.round(r.right)} (${Math.round(r.width)}px)`;
      // stretched single column: text blocks wider than ~90ch
      for (const e of textEls) {
        const w = e.getBoundingClientRect().width,
          fs = px(getComputedStyle(e).fontSize);
        if (/^(P|LI|DD)$/.test(e.tagName) && w / (fs * 0.5) > 100)
          add("layout.measureTooLong", e, `${Math.round(w / (fs * 0.5))}ch`);
      }
      // used horizontal area at 1440: share of content width actually covered by elements
      const leaves = all.filter(
        (x) => main.contains(x) && x.children.length === 0,
      );
      const rights = leaves.map((x) => x.getBoundingClientRect().right);
      const used = rights.length ? Math.max(...rights) - r.left : 0;
      out.usedWidthPct = Math.round((used / r.width) * 100);
      if (innerWidth >= 1400 && out.usedWidthPct < 60)
        out.rules["layout.stretchedEmpty"] = [
          `content uses ${out.usedWidthPct}% of ${Math.round(r.width)}px`,
        ];
    }

    // 5. Hit targets / names / contrast (rule 7; axe is the authority, these are the audit's own reads)
    for (const e of all.filter((x) =>
      x.matches(
        "button,a[href],input,select,textarea,[role=button],[tabindex]:not([tabindex='-1'])",
      ),
    )) {
      const r = e.getBoundingClientRect();
      if (e.matches("input[type=radio],input[type=checkbox]")) {
        if (r.width < 24 && !e.closest("label"))
          add(
            "a11y.hitTarget",
            e,
            `${Math.round(r.width)}x${Math.round(r.height)}`,
          );
      } else if (r.width < 24 || r.height < 24)
        add(
          "a11y.hitTarget",
          e,
          `${Math.round(r.width)}x${Math.round(r.height)}`,
        );
      const name = (
        e.getAttribute("aria-label") ||
        e.textContent ||
        e.getAttribute("title") ||
        ""
      ).trim();
      if (e.matches("button") && (!name || /^[^\w]{1,3}$/u.test(name)))
        add("a11y.iconButtonNoLabel", e, `name "${name}"`);
    }
    // focus-visible rule present? (rule 8's focus test is #458)
    let fv = false;
    for (const s of document.styleSheets)
      for (const r of s.cssRules)
        if (r.selectorText && r.selectorText.includes(":focus-visible"))
          fv = true;
    if (!fv) out.rules["a11y.noFocusVisible"] = ["no :focus-visible rule"];
    for (const e of textEls) {
      const cs = getComputedStyle(e);
      const fg = parse(cs.color);
      if (!fg) continue;
      const bg = bgOf(e);
      let op = 1;
      for (let x = e; x; x = x.parentElement)
        op *= +getComputedStyle(x).opacity;
      const c = ratio(over({ ...fg, a: fg.a * op }, bg), bg);
      const fs = px(cs.fontSize),
        bold = +cs.fontWeight >= 700;
      const need = fs >= 24 || (bold && fs >= 18.66) ? 3 : 4.5;
      if (c < need && !e.closest("button:disabled"))
        add("a11y.contrast", e, `${c.toFixed(2)}:1 < ${need} (${fs}px)`);
    }

    // 6. Icons (rule 4): Lucide at 16 px (20 in the rail), stroke 1.5, with a word or an accessible name. A data
    // graphic (`svg[data-graphic]`: a trace, the block timeline) is no icon; it names itself with `role="img"`.
    for (const svg of all.filter((x) => x.matches("svg:not([data-graphic])"))) {
      const r = svg.getBoundingClientRect();
      const want = svg.closest(opts.rail ?? ".rail") ? 20 : 16;
      if (Math.round(r.width) !== want || Math.round(r.height) !== want)
        add(
          "icon.size",
          svg,
          `${Math.round(r.width)}x${Math.round(r.height)} not ${want}`,
        );
      const stroke =
        svg.getAttribute("stroke-width") ?? getComputedStyle(svg).strokeWidth;
      if (px(stroke) !== 1.5) add("icon.stroke", svg, `stroke ${stroke}`);
      const host =
        svg.closest("button,a,label,li,p,span,div") ?? svg.parentElement;
      const named =
        svg.getAttribute("aria-label") ||
        svg.querySelector("title")?.textContent ||
        host?.getAttribute("aria-label") ||
        host?.textContent;
      if (!named?.trim()) add("icon.name", svg, "no word or accessible name");
    }

    // 7. Colour never alone (rule 11) and no tweening (rule 12), on their hooks
    const labelled = (e, inner) => e.querySelector(inner)?.textContent.trim();
    for (const e of all.filter((x) => x.matches("[data-series]")))
      if (!labelled(e, "[data-series-label]"))
        add("colour.seriesLabel", e, `series ${e.dataset.series} has no label`);
    for (const e of all.filter((x) => x.matches("[data-state]")))
      if (!/\S/.test(labelled(e, "[data-state-label]") ?? ""))
        add(
          "colour.stateLabel",
          e,
          `state ${e.dataset.state} has no word or glyph`,
        );
    for (const e of all.filter((x) => x.matches("[data-delta]"))) {
      const t = e.textContent.trim();
      if (t && e.dataset.delta !== "in-band" && !/^[▲▼]/.test(t))
        add("colour.deltaGlyph", e, `"${t}" has no ▲/▼`);
    }
    for (const e of all.filter((x) => x.matches("[data-live]"))) {
      const cs = getComputedStyle(e);
      if (
        cs.transitionDuration.split(",").some((d) => px(d) > 0) ||
        cs.animationName !== "none"
      )
        add(
          "motion.tween",
          e,
          `transition ${cs.transitionDuration}, animation ${cs.animationName}`,
        );
    }

    // 8. Consistency: distinct colours, radii, shadows
    const colours = new Set(),
      radii = new Set(),
      shadows = new Set(),
      borders = new Set();
    for (const e of all) {
      const cs = getComputedStyle(e);
      colours.add(cs.color);
      if (parse(cs.backgroundColor)?.a) colours.add(cs.backgroundColor);
      if (px(cs.borderTopWidth)) borders.add(cs.borderTopColor);
      if (cs.borderTopLeftRadius !== "0px") radii.add(cs.borderTopLeftRadius);
      if (cs.boxShadow !== "none") shadows.add(cs.boxShadow);
    }
    out.distinct = {
      colours: colours.size,
      borderColours: borders.size,
      radii: [...radii],
      shadows: shadows.size,
    };
    if (radii.size > 3)
      out.rules["consistency.radii"] = [
        `${radii.size} radii: ${[...radii].join(", ")}`,
      ];
    if (!Object.keys(space).length)
      out.rules["tokens.noSpacingTokens"] = ["no --space-* tokens"];

    // What an element draws: its text and its leaf boxes (a leader), never the accessible-only text.
    const inkOf = (el) => {
      const ink = [];
      const walk = document.createTreeWalker(el, NodeFilter.SHOW_ALL);
      for (let n = walk.currentNode; n; n = walk.nextNode()) {
        if (n.nodeType === 1 && n.closest(".sr-only")) continue;
        if (n.nodeType === 3 && !n.parentElement.closest(".sr-only")) {
          const range = document.createRange();
          range.selectNodeContents(n);
          ink.push(...range.getClientRects());
        } else if (
          n !== el &&
          n.nodeType === 1 &&
          !n.children.length &&
          visible(n)
        )
          ink.push(n.getBoundingClientRect());
      }
      return ink.filter((r) => r.width > 0 && r.height > 0);
    };

    // 9. The metric grid (§18.3 "Metric grid", #581): in every slot the label-to-value gap is the same, every unit
    // starts in one column, and the gap from a slot's last inked line to the next slot's label is the same.
    const mg = {
      slot: ".ride-metrics > .readout",
      label: ".readout-head",
      value: "[data-tier]",
      unit: ".readout-value > .tile-unit",
      ...opts.metrics,
    };
    const slots = all.filter((e) => e.matches(mg.slot));
    const spread = (xs) => Math.max(...xs) - Math.min(...xs);
    const labelGaps = [],
      unitXs = [],
      slotGaps = [];
    slots.forEach((slot, i) => {
      const label = slot.querySelector(mg.label);
      const value = slot.querySelector(mg.value);
      if (label && value)
        labelGaps.push([
          slot,
          value.getBoundingClientRect().top -
            label.getBoundingClientRect().bottom,
        ]);
      const unit = slot.querySelector(mg.unit);
      if (unit && visible(unit))
        unitXs.push([slot, unit.getBoundingClientRect().left]);
      const next = slots[i + 1]?.querySelector(mg.label);
      // From the last line that draws ink: an empty box (a reserved strip) cannot hide an unequal gap.
      const last = [...slot.children].filter((c) => inkOf(c).length).at(-1);
      if (next && last)
        slotGaps.push([
          slot,
          next.getBoundingClientRect().top -
            last.getBoundingClientRect().bottom,
        ]);
    });
    for (const [name, list] of [
      ["metrics.labelGap", labelGaps],
      ["metrics.unitColumn", unitXs],
      ["metrics.slotGap", slotGaps],
    ])
      if (list.length > 1 && spread(list.map(([, v]) => v)) > 1)
        for (const [slot, v] of list)
          add(
            name,
            slot,
            `${Math.round(v * 10) / 10}px of ${list.map(([, x]) => Math.round(x)).join("/")}`,
          );

    // 10. The strip's labels (§18.3 "Strip labels", #581): each is flush left with its block (±1 px), its text stays
    // inside the block's width, no two overlap, every label sits one scale step under the bar, and a label too long for
    // its block is a leader with the full label in the accessible text.
    const st = {
      label: "[data-block-label]",
      block: "[data-block]",
      bar: ".ride-timeline svg",
      leader: ".timeline-leader",
      full: ".sr-only",
      ...opts.strip,
    };
    const bar = document.querySelector(st.bar)?.getBoundingClientRect();
    const inked = [];
    // Every label, drawn or not: an empty one has no box, so it is not among the visible elements.
    const labels = [...document.querySelectorAll(st.label)];
    for (const b of document.querySelectorAll(st.block))
      if (!labels.some((l) => l.dataset.blockLabel === b.dataset.block))
        add("strip.label", b, `block ${b.dataset.block} has no label`);
    for (const l of labels) {
      const b = document
        .querySelector(`${st.block}[data-block="${l.dataset.blockLabel}"]`)
        ?.getBoundingClientRect();
      if (!b) {
        add("strip.label", l, "no block of its own");
        continue;
      }
      const ink = inkOf(l);
      // A label with no text draws the leader and carries its full label in the accessible text; neither is a hole.
      if (!ink.length || l.querySelector(st.leader)) {
        const leader = l.querySelector(st.leader);
        if (!leader || !visible(leader))
          add("strip.label", l, "draws neither its text nor a leader");
        if (!l.querySelector(st.full)?.textContent.trim())
          add("strip.label", l, "no full label in the accessible text");
        if (!ink.length) continue;
      }
      const left = Math.min(...ink.map((r) => r.left));
      const right = Math.max(...ink.map((r) => r.right));
      if (Math.abs(left - b.left) > 1)
        add(
          "strip.label",
          l,
          `starts at ${Math.round(left)}, its block at ${Math.round(b.left)}`,
        );
      if (right > b.right + 1)
        add(
          "strip.label",
          l,
          `ends at ${Math.round(right)}, past its block's ${Math.round(b.right)}`,
        );
      // The gap is the label's lane box to the bar (the glyphs' own extent is the font's).
      const gap = Math.round(
        l.getBoundingClientRect().top - (bar?.bottom ?? 0),
      );
      if (bar && (gap <= 0 || !SCALE.has(gap)))
        add("strip.label", l, `${gap}px under the bar, not a --space-* step`);
      for (const o of inked)
        if (left < o.right && o.left < right)
          add("strip.label", l, `overlaps the label of block ${o.block}`);
      inked.push({ left, right, block: l.dataset.blockLabel });
    }

    for (const [k, v] of Object.entries(out.rules)) {
      out.counts[k] = v.length;
      const grouped = new Map();
      for (const m of v) grouped.set(m, (grouped.get(m) || 0) + 1);
      out.worst[k] = [...grouped.entries()]
        .slice(0, opts.top ?? 3)
        .map(([m, n]) => (n > 1 ? `${m} (x${n})` : m).replace(/^#root > /, ""));
    }
    out.total = Object.values(out.counts).reduce((a, b) => a + b, 0);
    if (!opts.full) delete out.rules;
    return out;
  };
})();
