# 0027 — The title bar is chrome above the scrolling body

- Status: accepted
- Date: 2026-10-21
- Supersedes in part: ADR 0025 (the title bar was made `sticky` inside the scroll container)

## Context

ADR 0025 pinned the title bar with `position: sticky; top: 0` because it was a child of
`.app-surface`, which was itself the scroll container (`overflow-y: auto`). That solved the problem
it was asked to solve — a long page no longer carried the only drag handle off the top of the
screen — but it left the two pieces of chrome disagreeing about where the top of the page is.

`sticky` only pins an element inside a scrollport. The scrollbar still belongs to the scroll
container, and the scroll container still contained the bar, so the bar ran the window's full height
and drew over the bar's right end. Reported as "the scroll bar takes up the entire height. Instead,
the title bar should take up the entire width, with the scroll bar starting below it." Measured on
the installed build, on the window's right-hand column:

| row range | pixels | what it is |
| --- | --- | --- |
| 0–43 | 44 | title bar (its bottom rule is the 1px row at 44) |
| 45–578 | 533 | scrollbar thumb, starting level with the bar's bottom rule |

Before the change the scrollbar began at row 0, so it also ran the height of the title bar. This is
a fault in the *structure*, not in the styling: no combination of `sticky`, `inset`, padding or
negative margins can make a scrollbar stop partway down its own scroll container.

## Decision

Split the scroll container from the window chrome. `.app-surface` stays `position: fixed; inset: 0`
(required — under a transparent webview a *root* background is not painted, ADR 0017, so this
element is the window's background) but becomes a flex **column** with no overflow of its own. Its
two children are:

1. `TitleBar.svelte` — a plain `flex h-9 shrink-0` item.
2. `.app-scroll` — `flex: 1 1 auto; min-height: 0; overflow-y: auto`, the only thing that scrolls.

The bar is now structurally outside the scrollport: it cannot scroll away, and because the
scrollbar belongs to `.app-scroll`, it begins where the bar ends. The bar stops being `sticky`,
which also removes the `z-20` stacking requirement — nothing scrolls under it any more.

The bar is rendered by `src/routes/+layout.svelte` rather than `(app)/+layout.svelte`. Two reasons,
both structural: it must be a SIBLING of the scroll area (inside it, it scrolls), and it must
outlive route changes — a bar inside the page body is destroyed and rebuilt on every navigation,
losing the maximised state it tracks.

`min-height: 0` on `.app-scroll` is load-bearing. A flex item's default `min-height: auto` refuses
to shrink below its content, so without it the body grows to the height of the page and the overflow
escapes to the window instead of scrolling in the new container.

## Consequences

`src/lib/layout.test.ts` asserts the arrangement from the source that defines it, since a unit test
cannot lay out CSS: `.app-surface` is a flex column that does not scroll, `.app-scroll` owns the
overflow and the `min-height: 0`, the root layout renders `<TitleBar />` before `.app-scroll`, the
`(app)` layout renders neither, and the bar is not `sticky`. Verified by reverting `.app-surface` to
`overflow-y: auto`, which fails the first assertion.

The parser in that test strips CSS comments before reading declarations, and that is not tidiness.
The rule bodies in `app.css` are half prose, and the comment on `.app-surface` quotes the
`overflow-y: auto` that caused this bug — an unstripped parser reads the explanation as the code and
the test fails on the prose instead of the behaviour.

Verified on the installed build by reading the compositor screenshot's right-hand column: the bar
occupies rows 0–43 with its rule at 44, and the scrollbar thumb begins at row 45.

The bar no longer needs a stacking context, so `z-20` is gone. `.window-resize-edge` keeps
`z-index: 40` — it is viewport-anchored and must stay above the bar, which it does (ADR 0026).
