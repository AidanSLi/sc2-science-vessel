//! Runs the Python analysis sidecar and hands its JSON to the frontend.
//!
//! The shell never parses a replay itself (ADR 0001). Replay parsing is `sc2reader`'s job and
//! it lives in Python, so this module exists to do exactly three things: find the sidecar
//! executable, run it, and turn its stdout into something the frontend can use — including
//! turning a failure into a readable message rather than an empty result.
//!
//! Every call here is **offline**. Nothing in this module opens a socket; the only networked
//! component in the project is the build-order snapshot scraper, and that runs at development
//! time (`tools/refresh_build_snapshot.py`, ADR 0002).
//!
//! Two things about the sidecar's contract matter enough to state here:
//!
//! - It writes one JSON document to stdout and **diagnostics to stderr**. So stdout is the
//!   result and stderr is the error channel, and a non-zero exit with a JSON body on stderr
//!   is normal.
//! - `scan` and `analyze` are slow by frontend standards — `analyze` decodes tens of thousands
//!   of events and takes roughly 0.3-1 s per replay. They are run synchronously on the command
//!   thread rather than the main thread, which Tauri already does for commands, so the UI does
//!   not freeze while one runs.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use tauri::{AppHandle, Manager};

/// Name of the sidecar binary, as declared in `tauri.conf.json`'s `externalBin`.
///
/// Tauri appends the platform's executable suffix, so the file on disk is
/// `sc2sv-sidecar-x86_64-unknown-linux-gnu` in development and `sc2sv-sidecar` once bundled.
pub const SIDECAR_NAME: &str = "sc2sv-sidecar";

// There is deliberately no timeout on a sidecar run. `analyze` on a 40-minute 4v4 is the
// slowest thing the app does, and a timeout that fires mid-analysis would report a working
// replay as broken. A timeout would also need the process killed to be meaningful, and killing
// it mid-write can leave a partially written cache behind. A hang is better diagnosed from the
// CLI (`sc2sv analyze <replay> --summary`), which is why that exists.

/// Finds the sidecar executable.
///
/// Three layouts have to work, because this is run from `tauri dev`, from a bundled app, and
/// from `cargo run` during development, and they put the binary in different places:
///
/// 1. Next to the current executable, which is where Tauri's bundler and `tauri dev` both put
///    it (`externalBin` is resolved relative to the exe).
/// 2. `<repo>/sidecar/` run directly as a Python module, so a developer without a built binary
///    can still exercise the app without packaging anything.
/// 3. The repo's `.venv` interpreter running `-m sc2sv.cli`, for the same reason.
///
/// Args:
///     app: The running app handle, used to locate the current executable.
///
/// Returns:
///     The command to run and the arguments that precede the sidecar's own, or an `Err`
///     explaining which paths were tried. The error names the paths because "sidecar not
///     found" without them is unactionable.
fn locate_sidecar(app: &AppHandle) -> Result<(PathBuf, Vec<String>), String> {
    let mut tried: Vec<String> = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(SIDECAR_NAME);
            if candidate.is_file() {
                return Ok((candidate, Vec::new()));
            }
            tried.push(candidate.display().to_string());
            // `tauri dev` writes the sidecar with the target triple still in the name.
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if name.starts_with(SIDECAR_NAME) {
                        return Ok((entry.path(), Vec::new()));
                    }
                }
            }
        }
    }

    // Development fallbacks. `app.path().resource_dir()` is where a bundled app keeps
    // resources, so it is checked before the source tree even though the source tree is what
    // usually answers in practice.
    if let Ok(resources) = app.path().resource_dir() {
        let candidate = resources.join(SIDECAR_NAME);
        if candidate.is_file() {
            return Ok((candidate, Vec::new()));
        }
        tried.push(candidate.display().to_string());
    }

    // The Python fallback walks **upward** from the current directory rather than looking only
    // in it, because `tauri dev` runs the app with `src-tauri` as its working directory, so a
    // single-level check never finds the repo root. Both `.venv` and `sidecar/` must be present
    // before the interpreter is trusted: that combination is what distinguishes this project's
    // virtualenv from an unrelated `.venv` further up the tree, whose Python would not have
    // `sc2reader` installed and would fail with an import error instead of a clear message.
    if let Ok(cwd) = std::env::current_dir() {
        for ancestor in cwd.ancestors().take(4) {
            let venv = ancestor.join(".venv/bin/python");
            let package = ancestor.join("sidecar");
            if venv.is_file() && package.join("sc2sv/cli.py").is_file() {
                return Ok((
                    venv,
                    vec![
                        "-m".to_string(),
                        "sc2sv.cli".to_string(),
                        "--package-root".to_string(),
                        package.display().to_string(),
                    ],
                ));
            }
        }
        tried.push(format!(
            "{}/.venv/bin/python (searched upward)",
            cwd.display()
        ));
    }

    Err(format!(
        "the analysis sidecar was not found. Looked in: {}. \
		 Build it with `pnpm tauri build` or run the app from the repo root.",
        tried.join(", ")
    ))
}

/// Runs the sidecar and returns its parsed JSON output.
///
/// The `--package-root` argument is stripped before the call: it is a locator hint for this
/// module, not a sidecar option, so it is translated into `PYTHONPATH` instead. Passing a flag
/// the CLI does not define would make `argparse` exit 2 and look like a sidecar crash.
///
/// Args:
///     app: The running app handle.
///     args: The sidecar's own arguments, e.g. `["analyze", path, "--json"]`.
///     working_dir: Directory to run in, which also puts `sidecar/` on `PYTHONPATH` for the
///         module-based layout.
///
/// Returns:
///     The parsed JSON document the sidecar printed, or an `Err` naming what went wrong —
///     including the sidecar's own stderr, because that is where it explains a parse failure.
pub fn run_sidecar(app: &AppHandle, args: &[String], working_dir: &Path) -> Result<Value, String> {
    let (program, prefix) = locate_sidecar(app)?;

    let mut command = Command::new(&program);
    let mut package_root: Option<PathBuf> = None;
    let mut index = 0;
    while index < prefix.len() {
        if prefix[index] == "--package-root" {
            package_root = prefix.get(index + 1).map(PathBuf::from);
            index += 2;
            continue;
        }
        command.arg(&prefix[index]);
        index += 1;
    }
    command.args(args);
    command.current_dir(working_dir);

    if let Some(root) = &package_root {
        let mut path = root.to_string_lossy().into_owned();
        if let Some(existing) = std::env::var_os("PYTHONPATH") {
            path.push(':');
            path.push_str(&existing.to_string_lossy());
        }
        command.env("PYTHONPATH", path);
    }

    let output = command.output().map_err(|error| {
        format!(
            "could not run the analysis sidecar at {}: {error}",
            program.display()
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "the analysis sidecar failed ({}){}",
            output.status,
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", stderr.trim())
            }
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(stdout.trim()).map_err(|error| {
        format!(
            "the analysis sidecar returned output that is not JSON ({error}). \
			 The first 200 bytes were: {}",
            stdout.chars().take(200).collect::<String>()
        )
    })
}

/// Walks a directory for replay files.
///
/// Done in Rust rather than by asking the sidecar, because it is a directory walk and spawning
/// a Python interpreter to do it would cost more than the walk itself. Header parsing is the
/// part that needs `sc2reader`; finding files does not.
///
/// The walk is depth-limited and skips dot-directories, because a registered root may be a
/// whole Wine prefix (`.../drive_c/users/steamuser/Documents/StarCraft II/`) and descending into
/// every nested `Banks`/`Hotkeys` directory for nothing is wasted time on a real library.
///
/// Args:
///     root: The directory to search.
///     limit: Stop after this many replays.
///
/// Returns:
///     Replay paths, sorted, or an `Err` when the root cannot be read.
fn collect_replays(root: &Path, limit: usize) -> Result<Vec<String>, String> {
    if !root.is_dir() {
        return Err(format!("{root:?} is not a directory"));
    }
    let mut found: Vec<String> = Vec::new();
    let mut queue: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];

    while let Some((directory, depth)) = queue.pop() {
        if depth > 6 || found.len() >= limit {
            continue;
        }
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if !name.starts_with('.') {
                    queue.push((path, depth + 1));
                }
            } else if name.ends_with(".SC2Replay") {
                found.push(path.to_string_lossy().into_owned());
                if found.len() >= limit {
                    break;
                }
            }
        }
    }

    found.sort();
    Ok(found)
}

/// Lists the replay files under a root, without parsing them.
///
/// This is what the match list is built from. It is deliberately separate from the header scan:
/// listing is instant and the scan is not, so the UI can show "found 783 replays" immediately
/// and fill in map and matchup as the scan completes.
///
/// @param root Directory to search.
/// @param limit Maximum number of replays to return.
/// @returns The replay paths, or an error naming the directory.
#[tauri::command]
pub fn list_replays(root: String, limit: Option<usize>) -> Result<Vec<String>, String> {
    collect_replays(Path::new(&root), limit.unwrap_or(5000))
}

/// Parses the headers of every replay in a library.
///
/// Header-level parsing is ~3 ms per replay (measured across 783), so this returns a full
/// library in a second or two and there is no need for a background indexing service.
///
/// @param app The running app handle.
/// @param root Directory to scan.
/// @param limit Maximum number of replays to parse.
/// @returns The sidecar's scan payload: one record per replay with build, matchup, length,
///     speed and MMR presence.
#[tauri::command]
pub fn scan_library(app: AppHandle, root: String, limit: Option<usize>) -> Result<Value, String> {
    let mut args = vec!["scan".to_string(), root.clone(), "--json".to_string()];
    if let Some(limit) = limit {
        args.push("--limit".to_string());
        args.push(limit.to_string());
    }
    let working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    run_sidecar(&app, &args, &working_dir)
}

/// Produces the full metric payload for one replay.
///
/// This is the expensive call: it decodes every event, which takes roughly 0.3-1 s for a long
/// game. The result is every prepared series the analysis view renders, plus the guide-shaped
/// build order from `spawningtool`.
///
/// @param app The running app handle.
/// @param path The replay file to analyse.
/// @returns The analysis payload, or an error naming the replay and what failed.
#[tauri::command]
pub fn analyze_replay(app: AppHandle, path: String) -> Result<Value, String> {
    if !Path::new(&path).is_file() {
        return Err(format!("{path} is not a file"));
    }
    let args = vec!["analyze".to_string(), path, "--json".to_string()];
    let working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    run_sidecar(&app, &args, &working_dir)
}

/// Extracts one player's build order from a replay.
///
/// Split out from `analyze_replay` because the overlay only needs the step list, and analysing
/// a replay to show a build order in the overlay would make toggling the overlay cost a full
/// event decode.
///
/// @param app The running app handle.
/// @param path The replay file to read.
/// @param pid The player whose build order to return.
/// @returns The build-order payload.
#[tauri::command]
pub fn replay_build_order(app: AppHandle, path: String, pid: Option<i64>) -> Result<Value, String> {
    if !Path::new(&path).is_file() {
        return Err(format!("{path} is not a file"));
    }
    let mut args = vec!["build-order".to_string(), path, "--json".to_string()];
    if let Some(pid) = pid {
        args.push("--pid".to_string());
        args.push(pid.to_string());
    }
    let working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    run_sidecar(&app, &args, &working_dir)
}
