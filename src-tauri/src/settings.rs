//! User settings, persisted next to the app's other data.
//!
//! Deliberately a small JSON file rather than a config crate: the settings are two short
//! values, and a JSON blob that fails to parse falls back to defaults instead of refusing to
//! start. The file is written on change, not on exit, so a crash mid-game cannot lose a
//! setting the player just changed.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// The overlay toggle key used when the user has never chosen one.
///
/// Not `Ctrl+Alt+B`: that combination is taken on at least one real KDE session, and a
/// default that silently fails to register would make the overlay look unreachable on a
/// fresh install. `Ctrl+Alt+O` is unclaimed in a stock KDE keymap (which reserves only
/// `Ctrl+Alt+Del`) and does not collide with a StarCraft II default hotkey.
pub const DEFAULT_OVERLAY_SHORTCUT: &str = "Ctrl+Alt+O";

/// The appearance theme used when the user has never chosen one.
///
/// Terran: it is the neutral, low-chroma palette the rest of the app was designed against, and
/// it is the closest of the four to a plain dark UI, so a first run looks like an analysis tool
/// rather than like a game skin the user did not ask for.
pub const DEFAULT_THEME: &str = "terran";

/// Every theme id the shell will store.
///
/// Listed here and not just in the frontend because the shell is the one writing the value: a
/// typo or a stale frontend bundle would otherwise persist an id that no CSS rule matches, and
/// the app would come up on unstyled fallbacks with no error to explain it. Must stay in step
/// with `THEMES` in `src/lib/theme.ts`.
pub const THEMES: &[&str] = &["terran", "zerg", "protoss", "space"];

/// Returns the default theme as an owned string, for serde's `default` attribute.
fn default_theme() -> String {
    DEFAULT_THEME.to_string()
}

/// Keys tried, in order, when the configured key cannot be registered.
///
/// Registration fails when another application already owns the combination, and the
/// global-shortcut plugin cannot tell us that in advance — `is_registered` only reports the
/// plugin's own registrations, not the compositor's. So the only way to discover a conflict
/// is to try, and the only useful response is to try the next candidate. Falling through
/// this list means the overlay still has *a* working key rather than none, which matters
/// because the key is the whole point of the overlay: it is pressed mid-game, with the game
/// holding keyboard focus.
pub const FALLBACK_OVERLAY_SHORTCUTS: &[&str] = &[
    "Ctrl+Alt+O",
    "Ctrl+Alt+U",
    "Ctrl+Alt+F9",
    "Ctrl+Shift+F9",
    "Ctrl+Alt+Y",
];

/// The persisted settings document.
///
/// Every field carries `#[serde(default)]` so that a file written by an older build (or a
/// hand-edited one) still loads: a missing field becomes its default rather than a parse
/// error that would discard the whole document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Key combination that shows and hides the overlay, in accelerator form
    /// (`Ctrl+Alt+O`). Parsed by the global-shortcut plugin, which accepts `Ctrl`, `Alt`,
    /// `Shift`, `Super`/`Cmd` modifiers and a trailing key.
    #[serde(default = "default_overlay_shortcut")]
    pub overlay_shortcut: String,
    /// Directories the app scans for `.SC2Replay` files, in the order the user added them.
    ///
    /// Empty means "not configured yet", which the matches page turns into a first-run
    /// prompt rather than an error; `suggest_replay_roots` exists to make that prompt
    /// answerable in one click on a fresh install.
    #[serde(default)]
    pub replay_roots: Vec<String>,
    /// Build order to load per matchup, keyed by `ZvP`-style matchup.
    ///
    /// This is what makes the overlay open on the right build when a game starts. The key may
    /// also be a wildcard (`ZvX`), which the frontend resolves after an exact match fails, so a
    /// player who only has one build per race does not have to duplicate it across three
    /// matchups. An empty map means "no presets configured", which is the first-run state.
    #[serde(default)]
    pub build_presets: std::collections::BTreeMap<String, String>,
    /// Whether the app starts hidden and shows its popup when a game begins.
    ///
    /// On by default, which is the user's stated default: a resident helper that is invisible
    /// until the game starts. Turning it off leaves the app starting with its window visible,
    /// which is what someone treating it as an ordinary analysis application would want.
    #[serde(default = "default_true")]
    pub start_hidden: bool,
    /// Whether to launch the app when the user logs in.
    ///
    /// Off by default, and deliberately so: writing an autostart entry is a change to the
    /// user's desktop that they did not ask for unless they ask for it. It only has an effect
    /// for an installed build — see [`Settings::autostart_available`].
    #[serde(default)]
    pub launch_at_login: bool,
    /// The appearance theme: one of [`THEMES`].
    ///
    /// Stored as a plain string rather than an enum so that a theme this build does not know —
    /// written by a newer version, or hand-edited — degrades to the default appearance instead
    /// of failing to parse the whole settings document and losing every other setting with it.
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Where the analysis window was last placed and how big it was.
    ///
    /// `None` until the player has moved or resized the window, which is why it is optional
    /// rather than carrying a default rectangle: "never placed" and "placed at the default
    /// spot" have to be distinguishable, or a first run would restore a position it invented.
    /// See [`WindowGeometry`].
    #[serde(default)]
    pub analysis_geometry: Option<WindowGeometry>,
}

/// A saved window rectangle, in **physical** pixels.
///
/// Physical rather than logical on purpose. This is restored by the shell on a later launch,
/// possibly at a different display scale than the one it was saved under, and Tauri's
/// `PhysicalPosition`/`PhysicalSize` are the only units that mean the same thing on both
/// sides of a restart: a logical rectangle saved at 2.0 scale and restored at 1.0 would move
/// the window and halve it. The struct is unit-agnostic in name so that reading the field
/// does not invite the wrong assumption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowGeometry {
    /// Left edge, in physical pixels from the virtual desktop's origin.
    pub x: i32,
    /// Top edge, in physical pixels.
    pub y: i32,
    /// Outer width, in physical pixels.
    pub width: u32,
    /// Outer height, in physical pixels.
    pub height: u32,
}

impl WindowGeometry {
    /// Whether this rectangle is worth restoring on a given screen layout.
    ///
    /// A saved rectangle can stop making sense: the player unplugged the monitor it was on,
    /// or changed the resolution. Restoring it blindly would put the window off-screen with
    /// no way to drag it back — a frameless window has no title bar in the window manager
    /// sense, so "move it into view" is not something the desktop can do for the user.
    ///
    /// The test is deliberately loose in one direction and strict in the other: the window
    /// only has to *touch* a monitor (so a window half off the right edge still restores), but
    /// it must not be smaller than the app's own interactive floor, because a 12-pixel-wide
    /// window is indistinguishable from a crash.
    ///
    /// @param monitors The monitors currently attached, as (x, y, width, height) in physical px.
    /// @param min_width The smallest width the window may have.
    /// @param min_height The smallest height the window may have.
    /// @returns `true` when the rectangle should be restored.
    pub fn is_restorable(
        &self,
        monitors: &[(i32, i32, u32, u32)],
        min_width: u32,
        min_height: u32,
    ) -> bool {
        if self.width < min_width || self.height < min_height {
            return false;
        }
        // Overlap rather than containment: a window deliberately parked half off-screen is a
        // legitimate layout, and demanding full containment would silently undo it every launch.
        let left = self.x as i64;
        let top = self.y as i64;
        let right = left + self.width as i64;
        let bottom = top + self.height as i64;
        monitors.iter().any(|&(mx, my, mw, mh)| {
            let mleft = mx as i64;
            let mtop = my as i64;
            let mright = mleft + mw as i64;
            let mbottom = mtop + mh as i64;
            // A minimum visible area, so a window whose last 3 pixels touch a monitor counts as
            // lost rather than as placed.
            let overlap_w = right.min(mright) - left.max(mleft);
            let overlap_h = bottom.min(mbottom) - top.max(mtop);
            overlap_w > 64 && overlap_h > 64
        })
    }
}

/// Returns the default accelerator as an owned string, for serde's `default` attribute.
fn default_overlay_shortcut() -> String {
    DEFAULT_OVERLAY_SHORTCUT.to_string()
}

/// Returns `true`, for serde's `default` attribute on opt-out settings.
fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            overlay_shortcut: default_overlay_shortcut(),
            replay_roots: Vec::new(),
            build_presets: std::collections::BTreeMap::new(),
            start_hidden: true,
            launch_at_login: false,
            theme: default_theme(),
            analysis_geometry: None,
        }
    }
}

impl Settings {
    /// Loads settings from disk, falling back to defaults on any failure.
    ///
    /// A missing file is the normal first-run case. A *corrupt* file is reported on stderr
    /// and then ignored: refusing to start because a settings blob could not be parsed would
    /// trade a minor annoyance for a completely unusable app.
    ///
    /// @param app The running app handle, used to locate the config directory.
    /// @returns The loaded settings, or defaults.
    pub fn load(app: &AppHandle) -> Self {
        let path = match settings_path(app) {
            Ok(path) => path,
            Err(err) => {
                eprintln!("settings path unavailable ({err}); using defaults");
                return Self::default();
            }
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Self>(&text) {
                Ok(settings) => settings,
                Err(err) => {
                    eprintln!("{} is unreadable ({err}); using defaults", path.display());
                    Self::default()
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(err) => {
                eprintln!(
                    "{} could not be read ({err}); using defaults",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Writes settings to disk, creating the config directory if needed.
    ///
    /// @param app The running app handle, used to locate the config directory.
    /// @returns `Err` with a human-readable reason if the write failed.
    pub fn save(&self, app: &AppHandle) -> Result<(), String> {
        let path = settings_path(app)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("{}: {err}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|err| format!("settings could not be serialised: {err}"))?;
        std::fs::write(&path, text).map_err(|err| format!("{}: {err}", path.display()))
    }

    /// Whether "launch at login" can do anything useful for the running build.
    ///
    /// The autostart entry records **the path of the running executable**, so enabling it from
    /// a development build writes an entry pointing into `target/debug`. That entry breaks the
    /// moment the binary is rebuilt or cleaned, and it starts a bare binary rather than an app
    /// the desktop knows about. The settings page should therefore not offer the switch at all
    /// for such a build, rather than offering one that quietly produces a broken login entry.
    ///
    /// A bundled build shows a reason to believe it is installed: Tauri resolves resources
    /// from the app's own bundle directory, which does not exist next to a `cargo` binary.
    ///
    /// @param app The running app handle.
    /// @returns `true` when enabling autostart makes sense for this build.
    pub fn autostart_available(_app: &AppHandle) -> bool {
        crate::autostart::available()
    }
}

/// Normalises a per-matchup preset map.
///
/// Trims both sides and drops any entry whose key or value is blank, so clearing a dropdown in
/// the UI removes the preset rather than storing an empty id. An empty id is the dangerous
/// case: it *looks* configured, matches no build in the library, and the overlay would then
/// fall back while the settings page still showed a choice.
///
/// Neither keys nor ids are validated against the build library here. The library lives in the
/// frontend bundle, and a preset naming a build that a snapshot refresh removed is a normal
/// outcome the overlay already handles by falling back.
///
/// # Arguments
///
/// * `presets` - Matchup (or `ZvX` wildcard) to build order id.
///
/// # Returns
///
/// The cleaned map.
pub fn clean_presets(
    presets: std::collections::BTreeMap<String, String>,
) -> std::collections::BTreeMap<String, String> {
    presets
        .into_iter()
        .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        .filter(|(key, value)| !key.is_empty() && !value.is_empty())
        .collect()
}

/// Lists directories that look like StarCraft II replay folders on this machine.
///
/// Why this exists: a replay library is somewhere non-obvious — the game writes into the
/// user's Documents folder, and on Linux that folder is inside a Wine prefix whose layout
/// depends on the launcher (`Faugus`, `Lutris`, plain `~/.wine`, Bottles). Asking the user to
/// find and paste that path before the app can do anything is a poor first run, so the app
/// checks the layouts it knows about and offers whatever it finds.
///
/// Every candidate must be an existing directory to be returned, so an empty result is a
/// normal answer (the game may not be installed) rather than a failure. Results are deduped
/// because a prefix can be reachable by more than one route.
///
/// @returns Existing candidate directories, sorted for a stable order.
pub fn suggest_replay_roots() -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let home = match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home),
        None => return found,
    };

    // Native installs first: on Windows and macOS the game writes straight into Documents.
    for native in [
        home.join("Documents/StarCraft II/Accounts"),
        home.join("Documents/StarCraft II"),
    ] {
        collect_account_replays(&native, &mut found);
    }

    // Then the Wine prefixes, in the order of how likely they are to be where a Linux player
    // actually installed the game. Only the prefix marker directory is probed, so no prefix
    // that is not present costs more than one failed `read_dir`.
    for prefix in [
        home.join("Faugus"),
        home.join("Games"),
        home.join(".local/share/bottles/bottles"),
        home.join(".wine"),
        home.join(".local/share/lutris"),
    ] {
        collect_prefix_replays(&prefix, &mut found);
    }

    found.sort();
    found.dedup();
    found
}

/// Walks one Wine prefix for replay folders.
///
/// Windows paths live under `drive_c/users/<user>/`, so the user name is not known in
/// advance and has to be enumerated. Bottles nests a second level of prefix directories, so
/// that case is handled by trying the directory itself and then one level down.
///
/// @param prefix The prefix root, which may or may not exist.
/// @param found Accumulator for existing replay directories.
fn collect_prefix_replays(prefix: &Path, found: &mut Vec<String>) {
    // A Bottles prefix is `<bottles>/<BottleName>/drive_c`, and a plain Wine prefix is
    // `<prefix>/drive_c`; trying both costs one failed lookup each.
    let mut drive_candidates = vec![prefix.join("drive_c")];
    if let Ok(entries) = std::fs::read_dir(prefix) {
        for entry in entries.flatten() {
            drive_candidates.push(entry.path().join("drive_c"));
        }
    }

    for drive_c in drive_candidates {
        let users = drive_c.join("users");
        let Ok(entries) = std::fs::read_dir(&users) else {
            continue;
        };
        for entry in entries.flatten() {
            let documents = entry.path().join("Documents/StarCraft II");
            collect_account_replays(&documents, found);
        }
    }
}

/// Finds the `Replays` directories inside a StarCraft II `Accounts` folder.
///
/// The game's own documents folder is `<documents>/Accounts/<account id>/<profile id>/Replays`,
/// but some layouts drop a level, so both shapes are probed. Returning a `Replays` directory
/// rather than the `Accounts` root matters: pointing the scanner at a folder with no replays
/// in it produces an empty library and no explanation, while this returns nothing at all.
///
/// @param documents The StarCraft II documents folder, or its `Accounts` subfolder.
/// @param found Accumulator for existing replay directories.
fn collect_account_replays(documents: &Path, found: &mut Vec<String>) {
    // `documents` may already be the `Accounts` directory or its parent; probe both spellings
    // of the child so a caller does not have to know which it passed.
    for accounts in [documents.to_path_buf(), documents.join("Accounts")] {
        let Ok(entries) = std::fs::read_dir(&accounts) else {
            continue;
        };
        for entry in entries.flatten() {
            let account = entry.path();
            let Ok(profiles) = std::fs::read_dir(&account) else {
                // Some layouts put `Replays` directly under the account.
                push_if_dir(account.join("Replays/Multiplayer"), found);
                push_if_dir(account.join("Replays"), found);
                continue;
            };
            for profile in profiles.flatten() {
                push_if_dir(profile.path().join("Replays/Multiplayer"), found);
                push_if_dir(profile.path().join("Replays"), found);
            }
        }
    }
}

/// Pushes a path onto `found` when it is a directory.
///
/// @param path The candidate.
/// @param found Accumulator for existing directories.
fn push_if_dir(path: PathBuf, found: &mut Vec<String>) {
    if path.is_dir() {
        found.push(path.display().to_string());
    }
}

/// Resolves the settings file path inside the app's config directory.
///
/// Uses Tauri's own config directory rather than a hand-built path so it lands where the
/// platform expects (`~/.config/<identifier>` on Linux, `%APPDATA%` on Windows,
/// `~/Library/Application Support` on macOS).
///
/// @param app The running app handle.
/// @returns The full path to `settings.json`.
fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|err| format!("no config directory: {err}"))?;
    Ok(dir.join("settings.json"))
}

/// Registers the overlay shortcut, falling back through [`FALLBACK_OVERLAY_SHORTCUTS`].
///
/// Clears any previously registered shortcut first, so this doubles as the rebind path when
/// the user picks a new key from the settings page. Returns the accelerator that ended up
/// registered, which may differ from the requested one — the UI must show what it *got*, not
/// what it asked for, or a user whose key was taken would keep pressing a dead combination.
///
/// @param app The running app handle.
/// @param requested The accelerator the user configured.
/// @returns The accelerator actually registered, and an error string if none could be.
pub fn register_overlay_shortcut(app: &AppHandle, requested: &str) -> Result<String, String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;

    let manager = app.global_shortcut();
    // Unregistering everything is safe here: the overlay toggle is the app's only shortcut,
    // and a stale registration would make the next `register` fail as a duplicate.
    let _ = manager.unregister_all();

    let mut last_error = String::new();
    let mut candidates: Vec<&str> = vec![requested];
    for candidate in FALLBACK_OVERLAY_SHORTCUTS {
        if !candidates.contains(candidate) {
            candidates.push(candidate);
        }
    }

    for candidate in candidates {
        match manager.register(candidate) {
            Ok(()) => {
                // Logged on success too, not just failure: "nothing was printed" otherwise has
                // to mean either "the grab succeeded" or "this function never ran", and those
                // are indistinguishable from outside the process.
                eprintln!("registered the global shortcut {candidate}");
                return Ok(candidate.to_string());
            }
            Err(err) => {
                last_error = format!("{candidate}: {err}");
            }
        }
    }
    Err(last_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Builds a preset map from pairs, for terse test bodies.
    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn clean_presets_keeps_real_entries() {
        let cleaned = clean_presets(map(&[("ZvP", "bundled-130000"), ("TvZ", "bundled-9")]));
        assert_eq!(cleaned.len(), 2);
        assert_eq!(cleaned["ZvP"], "bundled-130000");
    }

    #[test]
    fn clean_presets_drops_blank_values() {
        // The case this exists for: a cleared dropdown must remove the key, because an empty
        // value would look configured while matching no build.
        let cleaned = clean_presets(map(&[("ZvP", ""), ("ZvT", "   "), ("ZvZ", "ok")]));
        assert_eq!(cleaned.len(), 1);
        assert!(!cleaned.contains_key("ZvP"));
        assert!(!cleaned.contains_key("ZvT"));
        assert_eq!(cleaned["ZvZ"], "ok");
    }

    #[test]
    fn clean_presets_drops_blank_keys() {
        let cleaned = clean_presets(map(&[("", "bundled-1"), ("  ", "bundled-2")]));
        assert!(cleaned.is_empty());
    }

    #[test]
    fn clean_presets_trims_surrounding_space() {
        // A trailing space in an id would not match the library's id and the preset would
        // silently fall back, which looks like the preset was never saved.
        let cleaned = clean_presets(map(&[(" ZvP ", " bundled-1 ")]));
        assert_eq!(cleaned["ZvP"], "bundled-1");
    }

    #[test]
    fn settings_without_presets_still_load() {
        // A settings file written before presets existed must not fail to parse, or upgrading
        // would silently reset the user's replay folders and shortcut.
        let json = r#"{"overlayShortcut":"Ctrl+Alt+U","replayRoots":["/replays"]}"#;
        let settings: Settings = serde_json::from_str(json).expect("old settings must parse");
        assert_eq!(settings.overlay_shortcut, "Ctrl+Alt+U");
        assert_eq!(settings.replay_roots, vec!["/replays".to_string()]);
        assert!(settings.build_presets.is_empty());
    }

    #[test]
    fn presets_round_trip_through_json() {
        let original = Settings {
            build_presets: map(&[("ZvP", "bundled-1"), ("ZvX", "bundled-2")]),
            ..Settings::default()
        };
        let text = serde_json::to_string(&original).expect("must serialise");
        let loaded: Settings = serde_json::from_str(&text).expect("must parse");
        assert_eq!(loaded.build_presets, original.build_presets);
        // Wildcards are just keys, so they survive alongside exact matchups.
        assert_eq!(loaded.build_presets["ZvX"], "bundled-2");
    }

    /// One 1920x1080 monitor at the origin, which is the common single-screen case.
    const ONE_SCREEN: &[(i32, i32, u32, u32)] = &[(0, 0, 1920, 1080)];

    #[test]
    fn geometry_round_trips_through_json() {
        let geometry = WindowGeometry {
            x: 120,
            y: 80,
            width: 1280,
            height: 820,
        };
        let original = Settings {
            analysis_geometry: Some(geometry),
            ..Settings::default()
        };
        let text = serde_json::to_string(&original).expect("must serialise");
        let loaded: Settings = serde_json::from_str(&text).expect("must parse");
        assert_eq!(loaded.analysis_geometry, Some(geometry));
    }

    #[test]
    fn geometry_is_absent_by_default() {
        // "Never placed" must be distinguishable from "placed at the default spot", or a first
        // run restores a rectangle it invented. An absent field has to deserialise to `None`.
        let loaded: Settings = serde_json::from_str(r#"{"theme":"zerg"}"#).expect("must parse");
        assert!(loaded.analysis_geometry.is_none());
    }

    #[test]
    fn geometry_is_restorable_on_the_monitor_it_was_saved_on() {
        let geometry = WindowGeometry {
            x: 200,
            y: 100,
            width: 1280,
            height: 820,
        };
        assert!(geometry.is_restorable(ONE_SCREEN, 720, 480));
    }

    #[test]
    fn geometry_off_every_monitor_is_not_restorable() {
        // The regression this guards: the monitor the window was on has been unplugged. A
        // frameless window has no native title bar, so restoring this would strand the window
        // off-screen with nothing to drag.
        let geometry = WindowGeometry {
            x: 4000,
            y: 100,
            width: 1280,
            height: 820,
        };
        assert!(!geometry.is_restorable(ONE_SCREEN, 720, 480));
    }

    #[test]
    fn geometry_half_off_screen_is_restorable() {
        // Parked half off the right edge is a legitimate layout the player chose; demanding full
        // containment would silently undo it on every launch.
        let geometry = WindowGeometry {
            x: 1400,
            y: 100,
            width: 1280,
            height: 820,
        };
        assert!(geometry.is_restorable(ONE_SCREEN, 720, 480));
    }

    #[test]
    fn geometry_barely_touching_a_monitor_is_not_restorable() {
        // A window whose last few pixels graze the screen is lost, not placed: there would be
        // nothing to grab.
        let geometry = WindowGeometry {
            x: 1900,
            y: 100,
            width: 1280,
            height: 820,
        };
        assert!(!geometry.is_restorable(ONE_SCREEN, 720, 480));
    }

    #[test]
    fn geometry_smaller_than_the_floor_is_not_restorable() {
        let geometry = WindowGeometry {
            x: 100,
            y: 100,
            width: 200,
            height: 150,
        };
        assert!(!geometry.is_restorable(ONE_SCREEN, 720, 480));
    }

    #[test]
    fn geometry_restores_onto_a_second_monitor() {
        // Two screens side by side: a rectangle on the right-hand one is valid, which the
        // containment check alone would get right only if it iterated the monitors.
        let monitors = [(0, 0, 1920, 1080), (1920, 0, 2560, 1440)];
        let geometry = WindowGeometry {
            x: 2400,
            y: 200,
            width: 1280,
            height: 820,
        };
        assert!(geometry.is_restorable(&monitors, 720, 480));
    }
}
