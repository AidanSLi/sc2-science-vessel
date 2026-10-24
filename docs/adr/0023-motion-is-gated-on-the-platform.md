# 0023 — Ambient motion is gated on the platform, not decided once for everyone

- **Status:** accepted
- **Date:** 2026-10-24
- **Supersedes part of:** [0021](0021-the-space-field-is-static.md)

## Context

ADR 0021 removed the Space theme's ambient animation because it cost **100% of a CPU core** on
the reference Linux machine, measured on the release build in steady state against **4%** with
every animation deleted. That ADR also states the reason plainly: the cost is not a property of
the effect but of the renderer underneath it. This app is forced onto WebKitGTK's **software**
compositor (`WEBKIT_DISABLE_DMABUF_RENDERER=1`), which is mandatory here because the NVIDIA driver
rejects the buffers WebKit asks for (ADR 0005, root-caused in ADR 0022). A software compositor
repaints the whole window on the CPU every frame, so an animating element saturates a core
whatever its size — six 4x4px dots measured the same 100% as two full-window gradient layers.

Two things followed from that, and only one of them was acted on at the time:

1. On Linux the field has to be static. Done in ADR 0021.
2. **On a platform with a working GPU compositor it does not have to be.** Not acted on, because
   the only machine available for measurement was the Linux one.

The product owner then established the constraint that makes this worth revisiting: most of the
app's users are on **Windows 11**, where Tauri renders through **WebView2** — Chromium — where a
`transform`/`opacity` animation is a composited GPU layer and costs approximately nothing. The
Linux measurement does not transfer to that renderer, and it never did.

## Decision

**The ambient animations are kept in the stylesheet but gated behind `html[data-motion='on']`.**
Only Windows sets that attribute, via a new shell command:

```rust
#[tauri::command]
fn animations_supported_command() -> bool {
    cfg!(windows)
}
```

The frontend asks once, at startup, before applying the first theme (`initAnimations`, awaited by
`initTheme` in `src/lib/theme.ts`), and caches the answer in a module-level flag. `applyTheme`
then writes `data-theme` and `data-motion` together in one function, so the two can never
disagree.

Three details are load-bearing:

- **`app.html` ships `data-motion="off"` in the markup**, exactly like `data-theme="terran"`. The
  first painted frame must never be the expensive one, and the failure mode of the shell not
  answering — a dev server, an older binary, an IPC error — is then a *static* background rather
  than a laggy app. Every rule is opt-in for the same reason.
- **The gate is a `data-` attribute and not a class**, because Tailwind 4's `@theme` defaults live
  on `:root` and the theme blocks already key off `html[data-theme=…]`; a second attribute keeps
  the two concerns (which palette, whether it moves) independently switchable.
- **The animations stay in `src/app.css` unguarded by any JS.** Switching theme or motion never
  re-renders a component tree, which matters because ADR 0019 measured a tree-wide JS repaint at
  12.6% painted under a transparent view.

Also fixed here, found while verifying the gate: the drift keyframes were shifting by **50% of the
layer height**, not by a whole star tile, so the field **snapped visibly** every 240s and 520s.
The distance is now the layer's own `background-size` (200px dense, 380px sparse), which lands the
repeating pattern back on itself and makes the restart invisible — verified by parsing the built
stylesheet and asserting each drift distance equals its tile height. Nebula breathing uses
`transform: scale()` rather than the standalone `scale` property, because `transform` is the form
every compositor is guaranteed to keep on the GPU.

## Consequences

- Windows users get the drifting starfield and breathing nebulae; Linux users get the same
  composition standing still. Both render identically otherwise — the layers, colours and blur are
  unchanged, because a *static* blurred layer is rasterised once and costs nothing.
- The warning in ADR 0021 ("do not reintroduce motion without measuring it") still applies to
  Linux, and the gate is what enforces it. **Do not move these declarations up into
  `.theme-sky`/`.theme-nebulae` unguarded.**
- The Windows branch is **unverified on hardware** — this repo has never run on Windows, exactly as
  `docs/packaging.md` already notes for the rest of the Windows path. The claim that motion is
  cheap there rests on WebView2 using the same compositor as Chrome, which is documented behaviour
  rather than something measured here. The first Windows run should confirm it with the app's own
  CPU counter: the expectation is a few percent, and anything near a whole core means the gate is
  not working and the field should be reverted to static for everyone.
- If the Linux renderer ever gets fixed — a WebKitGTK release with the NVIDIA path working, or a
  per-machine renderer probe as ADR 0022 suggests — the gate becomes unnecessary. It is one
  constant (`cfg!(windows)`) and one attribute, so removing it is a small change; ADR 0019's
  `nudgeRepaint` and ADR 0017's styled scrollbar stand to become unnecessary at the same time.
- ADR 0021's *measurements* and *ruled-out list* remain correct and worth keeping. Only its
  conclusion "the field is static" is now conditional, hence this ADR and the pointer added there.

## Verification

- `html[data-motion='on']` → `theme-sky-drift-dense 240s` / `theme-nebula-breathe 90s`;
  `off` → `none 0s`; attribute absent → animations apply (the gate is opt-in, so an absent
  attribute is *on* only if a page forgets to set it — `app.html` always does).
- Built CSS contains no ungated `.theme-sky`/`.theme-nebulae` animation rule.
- Drift distances equal tile heights: 200px/200px and 380px/380px, asserted against
  `build/_app/immutable/assets/*.css`.
- `pnpm check` 0/0, `pnpm test` 113 passed (7 files), Rust 56 passed / 1 ignored, clippy clean.
- On the Linux reference machine, motion stays off, so the capture for this ADR was taken from the
  built stylesheet with `data-motion="on"` forced, to confirm the composition renders as intended
  when a platform that can afford it does enable it.
