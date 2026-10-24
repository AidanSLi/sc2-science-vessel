# 0024. Two-tone palettes are carried by edges, headings and controls

Status: accepted
Date: 2026-10-24

## Context

ADR 0020 gave each race a palette of two colours: `--color-accent` (the console's own glow) and
`--color-accent-2` (its secondary, e.g. Protoss gold beside blue). The tokens existed and were
correct, but the app did not *look* two-tone, and the reason was not in the tokens:

- Every card in the app was `rounded-lg border border-surface-800 bg-surface-900/60 p-4` — a
  neutral hairline and a neutral fill. Twenty-six call sites. The accent appeared only on the
  wordmark tick, the nav's active pill and a couple of links, so on a page of cards the scheme
  read as "a dark app with one coloured element".
- Every button was `bg-surface-800`, i.e. neutral, so a page of controls had no theme in it.
- Every section heading was plain ink.

Measured on the Terran palette, the worst case: `--color-accent-2` was `oklch(0.72 0.038 246)`,
a near-grey. Against the surface ramp it is at (chroma 0.03–0.05) it was not perceptibly a second
colour at all. The palette declared two colours and rendered one.

## Decision

Put the second colour on the things the user cannot avoid seeing, and give the two hues separate
*duties* so the scheme reads as a system rather than as decoration:

| Element | Colour | Why |
| --- | --- | --- |
| Card border, table rules, input/placeholder edges | `accent-2` at low mix | Every panel is edged in the second hue |
| Card top edge (`::before` gradient) | `accent-2` → `accent` → `accent-2` | The two hues in one place, on every screen |
| Section headings (`h2`) | `accent-2` left rule | Seventeen headings; cheapest way to theme every screen |
| Page title (`h1`) | `accent` → `accent-2` gradient text, `accent-2` underline | The one place both hues sit adjacent |
| Buttons | `accent-2` edge, neutral-ish fill | Buttons are chrome |
| Active nav item | `accent` wash + `accent` underline | "Where I am" is identity |
| Focus ring | `accent-2` | Secondary means "the keyboard is here"; accent means "this is interactive" |

The split — **chrome takes the secondary, identity takes the primary** — is the load-bearing idea.
It means the two colours are always both present but never competing, and it gives a rule for the
next component instead of a judgement call.

Three CSS classes carry it (`.panel`, `.section-heading`/`.page-heading`, `.button`/`.nav-active`),
so the panel look lives in one place instead of twenty-six.

### The overlay is exempt, and not for convenience

The guide is a transparent, click-through panel over unpredictable game imagery. A tinted edge
strong enough to read over a dark map is a bright slab over snow, so the overlay carries the theme
in **text** colour instead (title in `accent`, matchup and "Next:" in `accent-2`, the current row's
number in `accent`, a `accent` rule down its left edge), and it uses its own text ramp
(`.guide-ink*`) rather than the app's `ink-*` scale.

That last point was measured, not assumed. Compositing the window's real depth-32 buffer over a
bright band (`.guide-halo` experiment, `/tmp/ovcomp.py`) showed rows drawn in `ink-500` (0.747
lightness) and `ink-300` disappearing entirely over a lit slope. Two changes fixed it:

1. **A text halo** — hard 1px `text-shadow` in eight directions plus a 2px drop. Soft blurred
   shadows were tried first and were not enough: they spread the darkening over 2–6px, so the
   glyph still sat on a band as bright as itself. Crisp offsets darken only the rim.
   Footnote: this is the standard HUD solution for exactly this problem, and it fails only when
   it is blurred.
2. **A brighter ink ramp** for the overlay, so the glyph is light enough for the halo to protect.

## Consequences

- Adding a card is now `class="panel"`; the look cannot drift between screens.
- A new theme must supply a secondary with chroma ≥ 0.06 and a hue ≥ 40° from its accent.
  `src/lib/theme.test.ts` reads `src/app.css` and enforces both, because the Terran regression was
  invisible in code review — the token was present and well-formed, just grey.
- Gradient text needs the `forced-colors` fallback in `app.css`, or a title is invisible in
  forced-colours mode. `-webkit-text-fill-color: transparent` is a real accessibility hazard.
- The overlay now has two defences (halo, ramp) where it previously relied on the theme's ink being
  dark enough over the game. Either alone is insufficient; the halo makes a light glyph survive a
  bright background, the ramp keeps the glyph light enough to matter.
- Twenty-six utility strings became one class. That is the change most likely to be reverted by a
  future edit that "just needs a slightly different card" — hence this ADR.
