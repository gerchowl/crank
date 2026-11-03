# Direction A "Telemetry": reference mockup

The owner chose this direction on 2026-11-02 (#438); [DESIGN.md §18](../../DESIGN.md#18-amendment-452-design-direction-a-telemetry)
is the spec. These files are the reference it was chosen from, kept as reviewed; where they differ from §18, §18 wins.

| File | What |
|---|---|
| [`a-telemetry.html`](a-telemetry.html) | Both screens in one page: open `a-telemetry.html#ride` or `a-telemetry.html#workouts` |
| [`sim.js`](sim.js) | Deterministic data: FTP 250 W, the built-in sensor check, the ride frozen at 1:22 |
| [`a-telemetry.ride.1440x900.png`](a-telemetry.ride.1440x900.png) | The ride screen at 1440×900 |
| [`a-telemetry.workouts.1440x900.png`](a-telemetry.workouts.1440x900.png) | Workouts at 1440×900 |

The page is self-contained: no remote fonts or scripts. The original loaded Inter and JetBrains Mono from Google
Fonts; that link is removed, so the page uses them when they are installed locally and the system fallbacks in
`--font-ui` / `--font-num` otherwise. The PNGs were rendered with both fonts loaded.

## Licences

The icons in `a-telemetry.html` are paths from [Lucide](https://lucide.dev), under the ISC licence:

> ISC License. Copyright (c) for portions of Lucide are held by Cole Bemis 2013-2022 as part of Feather (MIT). All
> other copyright (c) for Lucide are held by Lucide Contributors 2022.
>
> Permission to use, copy, modify, and/or distribute this software for any purpose with or without fee is hereby
> granted, provided that the above copyright notice and this permission notice appear in all copies.
>
> THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS SOFTWARE INCLUDING ALL
> IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT,
> INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN
> AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR
> PERFORMANCE OF THIS SOFTWARE.

The PNGs show Inter and JetBrains Mono (both SIL Open Font License 1.1); no font file is checked in here.
