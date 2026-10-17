//! A tiny loopback control channel, shared by the watcher and the GUI.
//!
//! ## Why loopback TCP and not something more natural
//!
//! The app needs two processes to talk to each other:
//!
//! - a second launch of the app tells the running watcher **"show yourself"**, so clicking the
//!   app icon while it is already resident does not start a second copy;
//! - the watcher tells a running GUI **"show yourself"**, because a GUI spawned when StarCraft II
//!   started is deliberately hidden and the player may now be asking to see it.
//!
//! The portable options for that are limited. Unix domain sockets do not exist on Windows; named
//! pipes do not exist on Unix; and a crate that abstracts both would still need a protocol on top.
//! A TCP socket on `127.0.0.1` is `std::net` on every platform, needs no dependency, and the only
//! thing it costs is a listening port that is bound to the loopback interface and therefore
//! unreachable from off the machine.
//!
//! ## The protocol
//!
//! One line in, one line out, then the connection closes. A command is a lowercase word; the
//! reply is `ok` or `error`. There is no framing, no length prefix and no state: both sides send
//! exactly one request per connection, which is all this needs and is why it fits in a page.
//!
//! The read is bounded by [`MAX_COMMAND_BYTES`] so a peer that connects and streams forever
//! cannot make either process allocate without limit, and by a timeout so a peer that connects and
//! says nothing cannot hold a thread.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

/// The name the *watcher* publishes its control port under.
///
/// Declared here rather than in `watcher.rs` because both halves of the app need it and only one
/// of them includes `watcher.rs`: the GUI's tray "Quit" has to reach the watcher, since the watcher
/// supervises the GUI and would otherwise start a fresh window the next time StarCraft II appeared.
/// Putting the strings next to the transport that carries them keeps them in one place both sides
/// compile against, so neither can drift.
pub const WATCHER_CONTROL_NAME: &str = "watcher";

/// The control command that brings the app up, spawning a GUI when there is none.
///
/// Only the watcher handles it, and only the GUI sends it, so each of the two binaries sees one
/// half of this constant's life as dead code. That is exactly why it lives here: a per-crate
/// definition would be a definition per crate to keep in step.
#[allow(dead_code)]
pub const SHOW: &str = "show";

/// The control command that stops the watcher and the GUI it owns.
pub const QUIT: &str = "quit";

/// The largest single command accepted, in bytes.
///
/// Every real command is a handful of bytes; this is a bound on a hostile or broken peer, not a
/// protocol limit.
pub const MAX_COMMAND_BYTES: u64 = 256;

/// How long a peer has to speak, and how long to wait for a reply.
///
/// Two seconds is far longer than a loopback round trip and far shorter than a person's patience,
/// so it never fires in normal use and never hangs the caller for long when it does.
const IO_TIMEOUT: Duration = Duration::from_secs(2);

/// The pause before re-reading a port file whose port did not answer.
///
/// Long enough for a listener that has just bound to have published, short enough that a caller
/// asking a dead process for something does not notice.
const RETRY_DELAY: Duration = Duration::from_millis(250);

/// Reads the port published by a process, if it published one.
///
/// @param name The state file's base name, distinct per role so the watcher and the GUI cannot
///   read each other's file.
/// @returns The port, or `None` when there is no readable state file.
pub fn read_published_port(name: &str) -> Option<u16> {
	std::fs::read_to_string(state_path(name)).ok()?.trim().parse().ok()
}

/// The path of a role's port file.
///
/// Under `XDG_RUNTIME_DIR` on Linux, which the session manager clears at logout, so a stale file
/// cannot survive into the next login and point at a port that now belongs to something else.
///
/// @param name The role's state file name.
/// @returns The path of the state file.
pub fn state_path(name: &str) -> std::path::PathBuf {
	let dir = std::env::var_os("XDG_RUNTIME_DIR")
		.map(std::path::PathBuf::from)
		.unwrap_or_else(std::env::temp_dir);
	dir.join(format!("sc2-science-vessel-{name}.port"))
}

/// Binds a listening socket on an ephemeral loopback port and publishes it.
///
/// Port 0 asks the operating system for a free port, so two instances can never collide and no
/// fixed number has to be reserved anywhere.
///
/// @param name The state file name to publish the port under.
/// @param handler Called once per connection with the received line; returns the reply to send.
///   Returning `None` closes the connection without a reply.
/// @param should_stop Polled before each connection is served, so the caller can end the thread.
/// @returns The bound port, or `None` when no port could be bound.
pub fn listen<H, S>(name: &str, handler: H, should_stop: S) -> Option<u16>
where
	H: Fn(&str) -> Option<String> + Send + 'static,
	S: Fn() -> bool + Send + 'static,
{
	// A loopback-only bind: the address is explicit, so this cannot accidentally listen on a
	// public interface the way binding to a bare port sometimes can.
	let listener = TcpListener::bind(("127.0.0.1", 0)).ok()?;
	let port = listener.local_addr().ok()?.port();

	std::thread::spawn(move || {
		for incoming in listener.incoming() {
			if should_stop() {
				return;
			}
			let Ok(stream) = incoming else { continue };
			let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
			let _ = stream.set_write_timeout(Some(IO_TIMEOUT));

			let mut line = String::new();
			// `take` before `read_line` is what bounds the read: without it a peer that never
			// sends a newline would be read until the connection closed.
			if BufReader::new(&stream)
				.take(MAX_COMMAND_BYTES)
				.read_line(&mut line)
				.is_err()
			{
				continue;
			}

			if let Some(reply) = handler(&line) {
				let _ = (&stream).write_all(reply.as_bytes());
			}
		}
	});

	// Published after the thread starts, so a peer that reads the file and connects immediately
	// finds a listener already accepting.
	if std::fs::write(state_path(name), port.to_string()).is_err() {
		return None;
	}
	Some(port)
}

/// Sends one line to a listening process and waits for its reply.
///
/// @param port The peer's port.
/// @param command The line to send, without a trailing newline.
/// @returns The reply line, or `None` when the peer could not be reached or did not answer.
pub fn send(port: u16, command: &str) -> Option<String> {
	let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
	let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
	let _ = stream.set_read_timeout(Some(IO_TIMEOUT));

	stream.write_all(format!("{command}\n").as_bytes()).ok()?;

	let mut reply = String::new();
	BufReader::new(&stream)
		.take(MAX_COMMAND_BYTES)
		.read_line(&mut reply)
		.ok()?;
	Some(reply.trim().to_string())
}

/// Sends a command to a process identified by its published port file.
///
/// The state file survives a process that was killed outright (`SIGKILL` cannot run cleanup), so
/// a port read from it may belong to a process that no longer exists. One re-read after a failed
/// send distinguishes a genuinely absent peer from a file that was stale by milliseconds: a live
/// listener rewrites the file before it starts accepting, so the second read finds the real port.
/// Without this, `request_show` could answer `false` for a GUI that was in fact listening.
///
/// @param name The peer's state file name.
/// @param command The line to send.
/// @returns `true` when the peer replied `ok`.
pub fn send_to(name: &str, command: &str) -> bool {
	let attempt = |port: u16| send(port, command).map(|reply| reply == "ok").unwrap_or(false);

	let first = read_published_port(name).map(attempt).unwrap_or(false);
	if first {
		return true;
	}

	std::thread::sleep(RETRY_DELAY);
	read_published_port(name).map(attempt).unwrap_or(false)
}

/// Removes a role's published port file, so a later launch does not try to reach a dead process.
///
/// @param name The state file name.
pub fn unpublish(name: &str) {
	let _ = std::fs::remove_file(state_path(name));
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::atomic::{AtomicBool, Ordering};
	use std::sync::Arc;

	/// A name unique to this test process, so parallel test runs cannot share a port file.
	///
	/// @param base The test's own label.
	/// @returns A file name no other test uses.
	fn scratch_name(base: &str) -> String {
		format!("test-{}-{base}", std::process::id())
	}

	#[test]
	fn a_command_round_trips_and_gets_its_reply() {
		let name = scratch_name("roundtrip");
		let stop = Arc::new(AtomicBool::new(false));
		let stop_for_listener = stop.clone();
		let port = listen(
			&name,
			|line| {
				if line.trim() == "ping" {
					Some("ok\n".to_string())
				} else {
					Some("error\n".to_string())
				}
			},
			move || stop_for_listener.load(Ordering::Relaxed),
		)
		.expect("listener should bind");

		assert_eq!(send(port, "ping").as_deref(), Some("ok"));
		assert_eq!(send(port, "nonsense").as_deref(), Some("error"));
		assert_eq!(read_published_port(&name), Some(port));

		stop.store(true, Ordering::Relaxed);
		unpublish(&name);
	}

	#[test]
	fn sending_to_a_dead_port_fails_quietly() {
		// Nothing is listening on this name, so `send_to` must report failure rather than panic
		// or block — this is the ordinary case of clicking the app when nothing is running.
		assert!(!send_to(&scratch_name("nothing-listening"), "show"));
	}

	#[test]
	fn a_published_port_can_be_read_back() {
		let name = scratch_name("publish");
		std::fs::write(state_path(&name), "45678").unwrap();
		assert_eq!(read_published_port(&name), Some(45678));
		unpublish(&name);
		assert_eq!(read_published_port(&name), None);
	}
}
