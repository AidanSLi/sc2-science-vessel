# 22. The NVIDIA driver rejects WebKit's GBM buffers, and that is why the compositor is software

**Status:** accepted

**Date:** 2026-10-19

**Related:** ADR 0005 (which disables DMABUF), ADR 0019 (the transparent-view damage bug),
ADR 0021 (the static Space field, whose cost follows from this).

## Context

ADR 0021 established that any continuously animating element costs a full core, because the app
runs on WebKitGTK's software compositor. ADR 0005 turns that compositor on deliberately: the
DMABUF path failed with `Failed to create GBM buffer of size 1280x820: Invalid argument`.

This ADR records why it fails, because the answer decides whether cheap animation is reachable at
all. Two other workarounds in this codebase exist only because of the software path — ADR 0019's
`nudgeRepaint` and the styled scrollbar from ADR 0017 — so the same answer applies to all three.

## The failing call

`WEBKIT_DISABLE_DMABUF_RENDERER=0` on this machine logs the same line twice per window:

```
Failed to create GBM buffer of size 1280x820: Invalid argument
```

WebKit 2.54 does not use plain `gbm_bo_create`. An `LD_PRELOAD` shim that logs every GBM entry
point caught the exact call:

```
gbm_bo_create_with_modifiers2(dev, 640x420, fmt=XR24, mods=[0x0000000000000000], n=1, flags=0x4)
  -> NULL, errno=22 (EINVAL)
```

`0x4` is `GBM_BO_USE_RENDERING`; `0x0000000000000000` is `DRM_FORMAT_MOD_LINEAR`.

### Bisected against the driver directly

Calling the same function from Python (`ctypes`, no WebKit involved) isolates the argument the
driver objects to. On the NVIDIA RTX 4070 (render node `renderD128`, driver 615.78.08, open kernel
module):

| format | flags | modifiers | result |
| --- | --- | --- | --- |
| `XR24` / `AR24` | `GBM_BO_USE_RENDERING` | `[LINEAR]` | **EINVAL** |
| `XR24` / `AR24` | `GBM_BO_USE_RENDERING` | `NULL` (count 0) | OK |
| `XR24` / `AR24` | none | `[LINEAR]` | OK |
| `XR24` / `AR24` | none | `NULL` | OK |
| `XR24` / `AR24` | `RENDERING\|LINEAR` | either | **EINVAL** |

So the driver accepts a modifier list, and it accepts `GBM_BO_USE_RENDERING`, but **not both
together**. `gbm_surface_create_with_modifiers` behaves the same way: a modifier list of only
`DRM_FORMAT_MOD_INVALID` fails, `NULL` succeeds. The Mesa path on the same box (AMD Raphael iGPU,
`renderD129`) accepts every combination, with the sole exception of `INVALID` modifiers, which it
rejects for everyone.

WebKit always passes both — it has to, because a composited buffer needs a modifier to be
importable and needs `RENDERING` to be drawable. There is no argument combination left for it to
fall back to, which is why the failure is fatal rather than retried.

This matches the upstream record. WebKit bug 303811 (blank WebView on RTX 50 series, same
`Invalid argument`), tauri#13493 (closed as "not planned" — *"webkitgtk is acting up more on every
release especially with Nvidia hardware"*), tauri#9304, wry#1366, and the tauri docs page "Linux
Graphics Issues" all describe it. There is even a crate, `webkit2gtk-nvidia-quirk`, that exists
only to make the NVIDIA/Wayland/X11 call.

## The proof that it is the driver, not WebKit

Forcing the *other* GPU's EGL vendor makes the DMABUF path work on this machine:

```sh
__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json \
WEBKIT_DISABLE_DMABUF_RENDERER=0 \
./sc2-science-vessel --gui
```

Measured results:

| | software path (default) | Mesa GPU path |
| --- | --- | --- |
| GBM errors | 2 per window | **0** |
| Analysis window painted | 100% | 100% |
| Overlay (380x600) painted | 100% | 100% |
| **Animated Space theme** | **101% of a core** | **10.2%** |
| Static Space theme | 3.7% | 1.2% |

Same binary, same CSS. `DRI_PRIME=1` alone is *not* enough (it still reports 2 GBM errors and
paints 0%); the variable that matters is the EGL vendor override, i.e. which driver the `gbm`
device is created against.

Patching only the allocation is not enough either: an `LD_PRELOAD` shim that retried
`gbm_bo_create_with_modifiers2` without modifiers made the allocation succeed (0 GBM errors,
1795 successful retries) and the window still painted **0%** — the rest of that pipeline is bound
to the NVIDIA device, which is the device the compositor cannot import from.

## Decision

**Do not change the renderer selection yet.** Keep `suppress_broken_webview_renderer()` and its
`WEBKIT_DISABLE_DMABUF_RENDERER=1` default exactly as ADR 0005 left it, and keep the Space field
static as ADR 0021 decided. The `WEBKIT_DISABLE_DMABUF_RENDERER=0` escape hatch documented in the
README stays the way an affected user gets the GPU path.

Forcing an EGL vendor at start-up would fix animation on *this* machine and break the guarantee
the app currently makes. The vendor override names one specific driver stack:

- it hard-codes a Mesa-only path, so a machine with only the NVIDIA driver would get no renderer
  at all, where today it gets a working (if slow) one;
- `50_mesa.json` is a Mesa packaging detail, not an EGL API, so its path is not portable and its
  absence is a hard failure rather than a fallback;
- it changes which GPU does the compositing for a transparent, over-the-game window — the part of
  this app that has produced the most subtle bugs (ADR 0015, 0017, 0019) — and that has only been
  spot-checked on the Mesa path, not exercised over a live match.

Cheap animation is therefore a real, reachable thing, but shipping it means selecting a renderer
per machine rather than picking one for everybody.

## What shipping it would require

In rough order of risk:

1. **Detect, do not assume.** Ask the driver whether the exact WebKit call works — create a `gbm`
   device on each render node, try `gbm_bo_create_with_modifiers2` with `LINEAR` +
   `GBM_BO_USE_RENDERING`, and only then consider the DMABUF path. This is a ~40-line probe with
   no external dependency and it tests the actual failure, not a vendor string.
2. **Verify the overlay over a live game** on whichever device is selected. The transparent,
   override-redirect, click-through panel is the most fragile part of this app and it has not been
   tested end to end on Mesa here.
3. **Then, and only then**, re-add motion to the Space field and re-measure — and re-check whether
   ADR 0019's `nudgeRepaint` and ADR 0017's styled scrollbar are still needed, since both exist
   only to work around the software path.

## Consequences

- The reader should know that `Failed to create GBM buffer` is **not** a WebKit bug to wait out.
  It is the NVIDIA driver refusing modifier + `GBM_BO_USE_RENDERING` together, it has been
  reported upstream for years, and tauri has closed it as not planned. Waiting for a WebKitGTK
  release will not fix it.
- Animation being expensive is a consequence of the driver, not of the theme code. ADR 0021's
  numbers stand, and the static field stands with them.
- A user whose drivers handle the combination can already get the GPU path today with
  `WEBKIT_DISABLE_DMABUF_RENDERER=0`; the README says so.
- If cheap animation is wanted for everyone, the work is renderer *selection* (item 1 above), not
  more CSS.
