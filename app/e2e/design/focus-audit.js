// crank's focus probe (#458, DESIGN §18.7 rule 8). Browser-side half of focus.ts, which presses the keys: defines
// `window.__focusStart()` (focus a sentinel at the document start, so the next Tab reaches the first control) and
// `window.__focusProbe()`, which reads the focused element after each key press. Inject it with
// `page.addScriptTag({ path })`.
//
// Each probe returns the stop now focused (`id`, stable per element across probes; `kind`; document-coordinate box)
// and the ring verdict (`fault`, null for a visible ring) of the previous stop, read now that it has lost focus: the ring is its focused `outline`, or
// its `box-shadow` if it has no outline, and it must (WCAG 2.2 SC 2.4.11 / 2.4.13 intent) differ from the unfocused
// state, be at least 2 CSS px thick, lie in the viewport, and reach 3 : 1 against the background it is drawn on: the
// parent's for an outer ring, the element's own for an inset one (`outline-offset` < 0, an `inset` shadow).
// Background images are not read: the background is the stack of ancestor `background-color`s over white.
(() => {
  // --- colour + contrast: any CSS colour, through a canvas, so `color(srgb …)` or `oklch()` parse too ---
  const ctx = document.createElement("canvas").getContext("2d");
  const rgba = (c) => {
    ctx.clearRect(0, 0, 1, 1);
    ctx.fillStyle = "#000";
    ctx.fillStyle = c;
    ctx.fillRect(0, 0, 1, 1);
    const [r, g, b, a] = ctx.getImageData(0, 0, 1, 1).data;
    return { r, g, b, a: a / 255 };
  };
  const over = (fg, bg) => ({
    r: fg.r * fg.a + bg.r * (1 - fg.a),
    g: fg.g * fg.a + bg.g * (1 - fg.a),
    b: fg.b * fg.a + bg.b * (1 - fg.a),
    a: 1,
  });
  const lum = ({ r, g, b }) => {
    const f = (v) => {
      v /= 255;
      return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
    };
    return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
  };
  const ratio = (a, b) => {
    const [x, y] = [lum(a), lum(b)].sort((m, n) => n - m);
    return (x + 0.05) / (y + 0.05);
  };
  const bgOf = (el) => {
    const stack = [];
    for (let e = el; e; e = e.parentElement) {
      const c = rgba(getComputedStyle(e).backgroundColor);
      if (c.a > 0) {
        stack.push(c);
        if (c.a >= 1) break;
      }
    }
    let bg = { r: 255, g: 255, b: 255, a: 1 };
    for (let i = stack.length - 1; i >= 0; i--) bg = over(stack[i], bg);
    return bg;
  };

  const sel = (el) => {
    const parts = [];
    for (
      let e = el;
      e && e.nodeType === 1 && parts.length < 4;
      e = e.parentElement
    ) {
      if (e.id) {
        parts.unshift(`#${e.id}`);
        break;
      }
      let s = e.tagName.toLowerCase();
      const cls = [...e.classList].slice(0, 2).join(".");
      if (cls) s += "." + cls;
      const name =
        e.getAttribute("aria-label") ?? (e === el ? e.textContent?.trim() : "");
      if (name) s += `[${JSON.stringify(name.slice(0, 24))}]`;
      parts.unshift(s);
    }
    return parts.join(" > ");
  };

  /** What a Tab stop may be: a native control, an ARIA widget, or a scroll container (keyboard-scrollable). */
  const CONTROL =
    "a[href], area[href], button, input, select, textarea, summary, iframe, [contenteditable]:not([contenteditable='false'])";
  const WIDGET_ROLES = new Set(
    "button checkbox combobox gridcell link listbox menuitem menuitemcheckbox menuitemradio option radio scrollbar searchbox slider spinbutton switch tab textbox treeitem".split(
      " ",
    ),
  );
  const scroller = (el) => {
    const cs = getComputedStyle(el);
    return (
      (/(auto|scroll)/.test(cs.overflowY) &&
        el.scrollHeight > el.clientHeight) ||
      (/(auto|scroll)/.test(cs.overflowX) && el.scrollWidth > el.clientWidth)
    );
  };
  const kindOf = (el) => {
    if (
      !el ||
      el === document.body ||
      el === document.documentElement ||
      el === sentinel
    )
      return "lost";
    if (
      el.matches(CONTROL) ||
      WIDGET_ROLES.has(el.getAttribute("role")) ||
      scroller(el)
    )
      return "control";
    return "other";
  };

  /** The ring's style: its outline, else its box-shadow. */
  const ringStyle = (el) => {
    const cs = getComputedStyle(el);
    return {
      outline:
        cs.outlineStyle === "none"
          ? ""
          : `${cs.outlineStyle} ${cs.outlineWidth} ${cs.outlineColor}`,
      outlineWidth: parseFloat(cs.outlineWidth) || 0,
      outlineColor: cs.outlineColor,
      inset: (parseFloat(cs.outlineOffset) || 0) < 0,
      shadow: cs.boxShadow === "none" ? "" : cs.boxShadow,
    };
  };
  /** The first shadow's colour, its thickness (spread plus blur, the most a ring can be) and whether it is inset. */
  const shadowRing = (s) => {
    const first = s.split(/,(?![^(]*\))/)[0];
    const color =
      first.match(/(rgba?|color|oklch|oklab|lab|lch|hsla?)\([^)]*\)/)?.[0] ??
      "transparent";
    const lens = first
      .replace(color, "")
      .match(/-?[\d.]+px/g)
      ?.map(parseFloat) ?? [0, 0, 0, 0];
    return {
      color,
      width: Math.max(0, lens[3] ?? 0) + Math.max(0, lens[2] ?? 0),
      inset: /\binset\b/.test(first),
    };
  };
  /** Why `focused` (read while focused) is no visible ring on `el`, or null when it is one. */
  const ringFault = (el, focused, rect) => {
    const now = ringStyle(el);
    if (focused.outline === now.outline && focused.shadow === now.shadow)
      return "no change on focus";
    const ring = focused.outline
      ? {
          color: focused.outlineColor,
          width: focused.outlineWidth,
          inset: focused.inset,
        }
      : focused.shadow
        ? shadowRing(focused.shadow)
        : null;
    if (!ring) return "no outline or box-shadow when focused";
    if (ring.width < 2) return `ring ${ring.width}px < 2px`;
    const vw = innerWidth;
    const vh = innerHeight;
    if (
      rect.right <= 0 ||
      rect.bottom <= 0 ||
      rect.left >= vw ||
      rect.top >= vh
    )
      return "off screen when focused";
    const bg = bgOf(ring.inset ? el : el.parentElement);
    const c = ratio(over(rgba(ring.color), bg), bg);
    if (c < 3) return `ring contrast ${c.toFixed(2)}:1 < 3:1`;
    return null;
  };

  const ids = new Map();
  const els = new Map();
  let sentinel = null;
  let prev = null;

  window.__focusStart = () => {
    sentinel?.remove();
    sentinel = document.createElement("span");
    sentinel.tabIndex = -1;
    document.body.prepend(sentinel);
    sentinel.focus();
    prev = null;
  };

  window.__focusProbe = () => {
    const el = document.activeElement;
    const previous = prev && {
      sel: prev.sel,
      fault: ringFault(prev.el, prev.ring, prev.rect),
    };
    const kind = kindOf(el);
    prev = null;
    if (kind === "lost")
      return {
        previous,
        kind,
        id: 0,
        sel: el ? el.tagName.toLowerCase() : "none",
      };
    if (!ids.has(el)) {
      ids.set(el, ids.size + 1);
      els.set(ids.size, el);
    }
    const r = el.getBoundingClientRect();
    prev = { el, sel: sel(el), ring: ringStyle(el), rect: r };
    const href = el.matches("a[href^='#']") ? el.getAttribute("href") : null;
    return {
      previous,
      kind,
      id: ids.get(el),
      sel: prev.sel,
      box: {
        left: r.left + scrollX,
        top: r.top + scrollY,
        right: r.right + scrollX,
        bottom: r.bottom + scrollY,
      },
      /** For a same-page link: its target's selector, if the target exists (the skip link, §6.7). */
      skipTo:
        href && href.length > 1 && document.querySelector(href) ? href : null,
    };
  };

  /** Whether the focused element is `selector`'s element, and the first of the stops `order` (ids) inside it. */
  window.__focusWithin = (selector, order) => {
    const t = document.querySelector(selector);
    return {
      on: document.activeElement === t,
      first: order.find((id) => t?.contains(els.get(id))) ?? 0,
    };
  };
})();
