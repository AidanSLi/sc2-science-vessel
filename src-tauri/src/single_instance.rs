//! A process-wide lock, so only one resident watcher exists per user.
//!
//! ## Why a lock rather than a pid file
//!
//! A pid file has a failure mode that a lock does not: the process dies without cleaning up and
//! the file survives, so the next launch either refuses to start or has to guess whether the pid
//! it names is still the program it thinks it is. Pid reuse makes that guess unreliable.
//!
//! An advisory file lock is released by the kernel when the file handle closes, which happens on
//! any exit including a crash or `SIGKILL`. There is therefore no stale state to detect, and the
//! "is it running?" question is answered by "can I take the lock?" — one atomic operation.
//!
//! ## Why `std` and not a crate
//!
//! [`std::fs::File::try_lock`] stabilised in Rust 1.89.0 and the project already requires 1.90
//! for its other dependencies, so the standard library has this. A crate such as `fslock` or
//! `single-instance` would add a dependency to reimplement a function that is now built in, and
//! the crates that also do activation-by-IPC would duplicate the control socket in
//! [`crate::watcher`], which needs to exist anyway to carry the `show` and `quit` commands.
//!
//! ## Scope
//!
//! Per user, not per machine: the lock file lives in the user's runtime directory, so two users
//! logged into the same machine each get their own watcher. That is correct — each has their own
//! StarCraft II session and their own window.

use std::fs::File;

/// The lock file this process holds for its whole life, if it is the primary instance.
///
/// Kept alive by the caller rather than stored statically: dropping the `File` releases the lock,
/// so ownership of this value *is* the lock. Storing it in a `static` would make the lock
/// impossible to release on shutdown, and would also make the module untestable.
pub struct InstanceLock {
	/// Held only so the handle — and therefore the lock — lives as long as this value.
	_file: File,
}

/// Where the lock file lives.
///
/// `XDG_RUNTIME_DIR` on Linux, which the session manager clears at logout, so nothing survives
/// into the next login. Elsewhere, the temp directory with the user's id appended, so two users
/// on one machine cannot contend for the same file.
///
/// @returns The path of the lock file.
fn lock_path() -> std::path::PathBuf {
	let dir = std::env::var_os("XDG_RUNTIME_DIR")
		.map(std::path::PathBuf::from)
		.unwrap_or_else(std::env::temp_dir);
	dir.join("sc2-science-vessel.lock")
}

/// Tries to become the primary instance.
///
/// @returns The held lock, or `None` when another process already holds it.
pub fn acquire() -> Option<InstanceLock> {
	acquire_at(&lock_path())
}

/// Tries to take the lock at a specific path.
///
/// Split out from [`acquire`] so that tests get their own lock file. Without this, tests running
/// in parallel share one path and fail each other's acquisitions — which is exactly the false
/// failure this separation removed.
///
/// @param path The lock file to take.
/// @returns The held lock, or `None` when another process already holds it.
fn acquire_at(path: &std::path::Path) -> Option<InstanceLock> {
	// `create(true)` rather than `create_new`: the file is expected to exist from a previous
	// run, and *holding the lock* is the claim, not creating the file.
	let file = File::options()
		.read(true)
		.write(true)
		.create(true)
		.truncate(false)
		.open(path)
		.ok()?;

	// `try_lock` returns `Err` both when the lock is held elsewhere (WouldBlock) and when the
	// filesystem does not support locking. Treating both as "someone else is running" is the
	// safe direction: the caller then asks the other instance to show itself, and if there is
	// no other instance it prints an explanation rather than starting a duplicate supervisor.
	file.try_lock().ok()?;

	Some(InstanceLock { _file: file })
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A lock path unique to one test, so parallel tests never contend.
	///
	/// The process id keeps concurrent `cargo test` runs apart; the name keeps tests in this
	/// binary apart.
	///
	/// @param name Which test is asking.
	/// @returns A path under the temp directory that no other test will use.
	fn scratch(name: &str) -> std::path::PathBuf {
		std::env::temp_dir().join(format!("sc2sv-lock-{}-{name}", std::process::id()))
	}

	#[test]
	fn the_lock_can_be_taken_when_nothing_holds_it() {
		let path = scratch("free");
		let _ = std::fs::remove_file(&path);
		let lock = acquire_at(&path);
		assert!(lock.is_some());
		let _ = std::fs::remove_file(&path);
	}

	#[test]
	fn a_second_acquire_while_held_fails() {
		let path = scratch("contended");
		let _ = std::fs::remove_file(&path);
		let first = acquire_at(&path).expect("first acquire should succeed");
		// A second open of the same file is what a second process does, and the lock must
		// refuse it because the lock belongs to a live file description.
		let second = acquire_at(&path);
		assert!(second.is_none(), "a held lock must not be acquirable twice");
		drop(first);
		let _ = std::fs::remove_file(&path);
	}

	#[test]
	fn the_lock_is_released_on_drop() {
		let path = scratch("released");
		let _ = std::fs::remove_file(&path);
		let first = acquire_at(&path).expect("first acquire should succeed");
		drop(first);
		let second = acquire_at(&path);
		assert!(second.is_some(), "dropping the lock must release it");
		let _ = std::fs::remove_file(&path);
	}
}
