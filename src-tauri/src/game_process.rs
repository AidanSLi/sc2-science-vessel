//! Detects whether the StarCraft II *game* is running, as distinct from whether a *match* is
//! being played.
//!
//! ## Why this is a separate question from the match
//!
//! The user's vocabulary separates two things that a lazier design would conflate:
//!
//! - **The game** is the StarCraft II process. It starts when the player launches SC2 from
//!   Battle.net and exits when they quit. It is running for the menus, the lobby, a match and
//!   the score screen alike.
//! - **The match** is one game inside SC2. It starts when the loading screen clears and the
//!   3-2-1 countdown begins, and ends when the player presses the score screen button.
//!
//! The watcher needs the first, because paying for a web view is only justified while SC2 is
//! running at all. The window mode needs the second, which comes from the client API's screen
//! list (ADR 0010). Only one of the two is a polling problem, and it is this one: the client
//! API does not exist until SC2 starts, so "is SC2 running" cannot be asked of it.
//!
//! ## How the process is recognised
//!
//! By the executable's reported name, matched case-insensitively against a small set of
//! spellings, because SC2 runs under Wine and the name that reaches the kernel depends on how
//! the launcher started it. Observed on the reference machine
//! (`~/Faugus/battlenet`, a Wine prefix started by Faugus):
//!
//! - the launcher is Battle.net, whose process is `Battle.net.exe`;
//! - the game proper is `StarCraft II.exe`, but Wine also spawns `StarCraft II Editor.exe` and
//!   versioned copies under `Versions/Base*/`;
//! - the editor must **not** count, because opening the Galaxy editor is not playing.
//!
//! Matching is deliberately *not* by window title: the window title poller already exists for
//! anchoring, but a title is only present once the game has created a window, which is later
//! than the process itself — and the watcher's whole job is to be early enough that the web
//! view is already warm when a match starts.
//!
//! ## Cost
//!
//! One directory read of `/proc` per poll. The caller polls about once a second, so this is a
//! few hundred `readlink` calls a second at worst; measured on the reference machine at 0.00%
//! of a core (see `docs/adr/0013-a-watcher-owns-the-web-view.md`).

/// Names that count as "StarCraft II is running", lowercased without the extension.
///
/// `starcraft ii` covers `StarCraft II.exe` and the versioned `StarCraft II.exe` under
/// `Versions/Base*`. `starcraft 2` is accepted as well because older shortcuts and Wine
/// wrappers use it, and the difference is invisible to a player.
///
/// `sc2_x64` and `sc2` cover what the game is actually called **under Wine**, which is the only
/// form that reaches a Linux process table. Measured on the reference machine
/// (`~/Faugus/battlenet`): the running game is `SC2_x64.exe`, and its kernel `comm` is
/// `Play Main Threa` — the name of a thread the game renamed, not the executable at all. Neither
/// of those would ever have matched `starcraft ii`, so the watcher could not see the game it was
/// built for, which showed up as an autostart launch that stayed silent forever.
///
/// `cfg(target_os = "linux")` because the matching below exists for Linux's `/proc` view of a
/// Wine process. Windows will need its own implementation (`CreateToolhelp32Snapshot` or a WMI
/// query) when the watcher is ported there, and until then this whole cluster would be dead code
/// on that platform — which is a `dead_code` warning under `-D warnings`, and was how this was
/// found rather than on a user's machine. The tests are gated with it because they exercise the
/// matching directly rather than through `game_is_running`.
#[cfg(target_os = "linux")]
const GAME_PROCESS_NAMES: [&str; 4] = ["starcraft ii", "starcraft 2", "sc2_x64", "sc2"];

/// Substrings that identify the game when the executable name is useless.
///
/// Under Wine the kernel's `comm` can be a thread name (`Play Main Threa`), so the name-based
/// match above is not sufficient on its own. The **command line** survives intact — Wine passes
/// the Windows path through — so it is consulted as a second signal. These are the fragments
/// that appear in the game's own path but not in Battle.net's, the editor's, or a launcher's.
#[cfg(target_os = "linux")]
const GAME_COMMAND_FRAGMENTS: [&str; 2] = ["sc2_x64.exe", "sc2.exe"];

/// Substrings that disqualify a command line even when it names the game.
///
/// The editor and the launcher share the game's directory, so their command lines contain
/// `StarCraft II` too. Editor and launcher are excluded here for the same reason they are
/// excluded by name.
#[cfg(target_os = "linux")]
const DISQUALIFYING_COMMAND_FRAGMENTS: [&str; 2] = ["editor.exe", "launcher.exe"];

/// Name fragments that disqualify a process even when it also matches a game name.
///
/// The editor ships in the same directory and shares the game's name prefix, so it would
/// otherwise be indistinguishable by name alone.
#[cfg(target_os = "linux")]
const DISQUALIFYING_FRAGMENTS: [&str; 2] = ["editor", "launcher"];

/// The maximum length of the kernel's `comm` field, in bytes.
///
/// Linux stores at most 15 bytes plus a terminator, so a longer executable name arrives
/// truncated and the tail is simply gone. That is not a Wine quirk to work around; it is how
/// `/proc/<pid>/comm` has always behaved, and it is why comparison cannot be equality against a
/// full filename.
#[cfg(target_os = "linux")]
const COMM_MAX_BYTES: usize = 15;

/// Truncates a name the way the kernel does when filling `comm`.
///
/// Byte-oriented because that is what the kernel does, but cut on a character boundary so a
/// non-ASCII name cannot produce invalid UTF-8. Process names in practice are ASCII; this is
/// defensive rather than load-bearing.
///
/// @param name A process name.
/// @returns The prefix the kernel would report.
#[cfg(target_os = "linux")]
fn kernel_comm(name: &str) -> &str {
	if name.len() <= COMM_MAX_BYTES {
		return name;
	}
	let mut end = COMM_MAX_BYTES;
	while end > 0 && !name.is_char_boundary(end) {
		end -= 1;
	}
	&name[..end]
}

/// Normalises a process name for comparison: lowercased, `.exe` removed, whitespace trimmed.
///
/// Wine and Linux disagree about how much of the filename survives, so comparison happens on
/// the stem rather than trusting any one source to be consistent.
///
/// @param name A process name as reported by the operating system.
/// @returns A comparable stem, e.g. `starcraft ii`.
#[cfg(target_os = "linux")]
fn normalise_process_name(name: &str) -> String {
	let lowered = name.trim().to_ascii_lowercase();
	let stem = lowered.strip_suffix(".exe").unwrap_or(&lowered);
	stem.trim().to_string()
}

/// Reports whether a process name looks like the StarCraft II game.
///
/// Exposed for tests, which is the only way to exercise the name matching without a real game
/// running. The disqualifying check runs first so `StarCraft II Editor.exe` cannot match by
/// accident even if a future name list grows a prefix that would otherwise admit it.
///
/// @param name A process name as reported by the operating system.
/// @returns `true` when the process is the game rather than the editor, launcher or anything
///   else.
#[cfg(target_os = "linux")]
pub fn is_game_process_name(name: &str) -> bool {
	let normalised = normalise_process_name(name);
	if normalised.is_empty() {
		return false;
	}
	if DISQUALIFYING_FRAGMENTS
		.iter()
		.any(|fragment| normalised.contains(fragment))
	{
		return false;
	}
	// Two spellings of the same process are legitimate: the full name, and what the kernel
	// actually hands back for it. `StarCraft II.exe` becomes `StarCraft II.ex` in `comm`, so
	// accepting only the full name would never match a real running game — which is precisely
	// the bug this comparison had before a test caught it.
	GAME_PROCESS_NAMES.iter().any(|candidate| {
		if normalised == *candidate {
			return true;
		}
		let with_extension = format!("{candidate}.exe");
		normalised == kernel_comm(&with_extension)
	})
}

/// Reports whether a process's command line looks like the StarCraft II game.
///
/// Needed because the kernel's `comm` cannot be trusted for this process: Wine runs the game as
/// `wine64-preloader` and the game renames its main thread, so `comm` reads `Play Main Threa`.
/// The command line still contains the real Windows path, so it is the reliable signal.
///
/// The disqualifying check runs first, for the same reason as in [`is_game_process_name`]: the
/// editor lives beside the game and its path contains `StarCraft II`.
///
/// @param command_line The process's arguments joined by spaces.
/// @returns `true` when the command line names the game rather than the editor or a launcher.
#[cfg(target_os = "linux")]
pub fn is_game_command_line(command_line: &str) -> bool {
	let lowered = command_line.to_ascii_lowercase();
	if lowered.is_empty() {
		return false;
	}
	if DISQUALIFYING_COMMAND_FRAGMENTS
		.iter()
		.any(|fragment| lowered.contains(fragment))
	{
		return false;
	}
	GAME_COMMAND_FRAGMENTS
		.iter()
		.any(|fragment| lowered.contains(fragment))
}

/// Reports whether the StarCraft II process is running right now.
///
/// On Linux this walks `/proc` and compares the `comm` field, which is the kernel's truncated
/// view of the executable name. Truncation is why matching is by name *stem* rather than exact
/// equality with a full filename: `comm` is capped at 15 bytes, so `StarCraft II.exe` arrives
/// as `StarCraft II.ex` and a strict comparison would never match. Normalising strips the
/// partial extension away, and the remaining `StarCraft II` is well inside the limit.
///
/// A game launched through a Wine prefix still appears here, because Wine runs the game as an
/// ordinary Linux process — it is a PE loaded by a Linux binary, not a virtual machine.
///
/// @returns `true` when SC2 is running.
pub fn game_is_running() -> bool {
	#[cfg(target_os = "linux")]
	{
		let Ok(entries) = std::fs::read_dir("/proc") else {
			return false;
		};
		for entry in entries.flatten() {
			// Only numeric directories are processes; everything else in /proc is not.
			if entry.file_name().to_string_lossy().parse::<u32>().is_err() {
				continue;
			}
			// The command line is read first and is the authoritative signal: under Wine the
			// kernel's `comm` is a thread name (`Play Main Threa`), so a name-only match never
			// finds the real game. Reading it first also keeps the common case to one read.
			if let Ok(raw) = std::fs::read(entry.path().join("cmdline")) {
				let command_line = String::from_utf8_lossy(&raw).replace('\0', " ");
				if is_game_command_line(&command_line) {
					return true;
				}
			}
			let Ok(comm) = std::fs::read_to_string(entry.path().join("comm")) else {
				continue;
			};
			if is_game_process_name(&comm) {
				return true;
			}
		}
		false
	}

	#[cfg(not(target_os = "linux"))]
	{
		// Windows and macOS get their own implementation when the watcher is ported there; the
		// task list is reported as `false` rather than guessed, so the watcher stays in its idle
		// state instead of spawning web views it cannot retire.
		false
	}
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
	use super::*;

	#[test]
	fn recognises_the_game_under_its_install_directory_name() {
		assert!(is_game_process_name("StarCraft II.exe"));
		assert!(is_game_process_name("StarCraft II"));
		assert!(is_game_process_name("starcraft ii.exe"));
		assert!(is_game_process_name("StarCraft 2.exe"));
	}

	#[test]
	fn recognises_the_truncated_name_the_kernel_reports() {
		// `comm` is capped at 15 bytes, so this is what a real /proc entry contains.
		// `StarCraft II.exe` is 17 bytes; the kernel keeps the first 15.
		assert!(is_game_process_name("StarCraft II.ex"));
		assert!(is_game_process_name("StarCraft II"));
		assert_eq!(kernel_comm("starcraft ii.exe"), "starcraft ii.ex");
	}

	#[test]
	fn short_names_are_not_truncated() {
		assert_eq!(kernel_comm("starcraft ii"), "starcraft ii");
		assert_eq!(kernel_comm("a"), "a");
	}

	#[test]
	fn the_editor_is_not_the_game() {
		assert!(!is_game_process_name("StarCraft II Editor.exe"));
		assert!(!is_game_process_name("StarCraft II Editor_x64.exe"));
	}

	#[test]
	fn the_launcher_is_not_the_game() {
		assert!(!is_game_process_name("Battle.net Launcher.exe"));
		assert!(!is_game_process_name("Battle.net.exe"));
	}

	#[test]
	fn recognises_the_wine_executable_name() {
		// Measured on the reference machine: the running game is `SC2_x64.exe`.
		assert!(is_game_process_name("SC2_x64.exe"));
		assert!(is_game_process_name("SC2.exe"));
	}

	#[test]
	fn recognises_the_game_from_a_wine_command_line() {
		// The real command line, verbatim from `/proc/8298/cmdline` on the reference machine.
		// The name-based match cannot see this process at all, so this path is load-bearing.
		let command_line = r"C:\Program Files (x86)\StarCraft II\Versions\Base98310\SC2_x64.exe -sso=1 -launch -uid s2";
		assert!(is_game_command_line(command_line));
	}

	#[test]
	fn a_thread_name_is_not_taken_for_the_game() {
		// Wine renames the game's main thread, so `comm` reads this on the reference machine. If it
		// ever started matching, the watcher would fire on unrelated processes.
		assert!(!is_game_process_name("Play Main Threa"));
	}

	#[test]
	fn the_editor_command_line_is_not_the_game() {
		let editor = r"C:\Program Files (x86)\StarCraft II\StarCraft II Editor.exe";
		assert!(!is_game_command_line(editor));
	}

	#[test]
	fn battle_net_is_not_the_game() {
		let agent = r"C:/ProgramData/Battle.net/Agent/Agent.9824/Agent.exe --session=1";
		assert!(!is_game_command_line(agent));
	}

	#[test]
	fn unrelated_processes_do_not_match() {
		assert!(!is_game_process_name("sc2-science-vessel"));
		assert!(!is_game_process_name("konsole"));
		assert!(!is_game_process_name(""));
		assert!(!is_game_process_name("   "));
	}

	#[test]
	fn a_name_merely_containing_the_game_name_does_not_match() {
		// Guards the exactness of the comparison: a hypothetical helper called
		// `StarCraft II Helper` must not be treated as the game itself.
		assert!(!is_game_process_name("StarCraft II Helper.exe"));
	}

	#[test]
	fn detection_is_answerable_without_a_game_running() {
		// Not asserting `false` — the test suite may run while a game happens to be open. The
		// contract under test is that this always answers rather than panicking on an
		// unreadable /proc.
		let _ = game_is_running();
	}
}
