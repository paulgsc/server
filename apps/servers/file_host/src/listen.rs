//! Where `file_host` listens, and the dev-only port juggle.
//!
//! Production listens on exactly the port it is given (`FILE_HOST_PORT`,
//! 3000 by default) or fails to start: its container publishes `3000:3000`
//! and its healthcheck asks 3000, so a server that quietly moved would be
//! up, unreachable, and marked unhealthy for a reason nobody could see.
//!
//! A server run from source beside that container opts into the juggle
//! (`make dev`): `FILE_HOST_PORT_FALLBACK` takes the next free port when the
//! asked-for one is taken, and `FILE_HOST_PORT_FILE` records the port it
//! actually got, so `paulgsc/some-ui`'s `vite dev` can find it without anyone
//! choosing a port. The file is JSON, `{ "port": 3001, "pid": 12345 }`: the
//! pid lets a reader ignore a file left behind by a process that no longer
//! exists (a `kill -9` never reaches the cleanup below).

use std::io;
use std::path::{Path, PathBuf};
use tokio::net::TcpListener;

/// How many ports past the asked-for one the fallback tries.
pub const FALLBACK_ATTEMPTS: u16 = 20;

/// Listen on `host:port`, or — with `fallback` — on the first free port of
/// the [`FALLBACK_ATTEMPTS`] from `port` up.
///
/// Only a port in use moves on to the next; any other failure (a bad host, a
/// privileged port) is returned as it is, since the next port would fail the
/// same way.
///
/// # Errors
/// `AddrInUse` when every port tried is taken, or the first other bind error.
pub async fn bind(host: &str, port: u16, fallback: bool) -> io::Result<TcpListener> {
	let attempts = if fallback { FALLBACK_ATTEMPTS } else { 1 };
	let mut in_use = None;
	for offset in 0..attempts {
		let Some(candidate) = port.checked_add(offset) else {
			break;
		};
		match TcpListener::bind((host, candidate)).await {
			Ok(listener) => return Ok(listener),
			Err(err) if err.kind() == io::ErrorKind::AddrInUse => in_use = Some(err),
			Err(err) => return Err(err),
		}
	}
	Err(in_use.unwrap_or_else(|| io::Error::from(io::ErrorKind::AddrInUse)))
}

/// The listener `main` serves on, from `config`, and the port file recording
/// it when `config.port_file` asks for one.
///
/// # Errors
/// A bind failure (see [`bind`]) or a port file that cannot be written.
pub async fn from_config(config: &crate::Config) -> io::Result<(TcpListener, Option<PortFile>)> {
	let listener = bind(&config.host, config.port, config.port_fallback).await?;
	let bound = listener.local_addr()?;
	if bound.port() != config.port {
		tracing::warn!(
			requested = config.port,
			bound = bound.port(),
			"port taken; listening on the next free one (FILE_HOST_PORT_FALLBACK)"
		);
	}
	tracing::info!("listening on {bound}");
	let port_file = config.port_file.as_deref().map(|path| PortFile::write(path, bound.port())).transpose()?;
	Ok((listener, port_file))
}

/// The record of the port this process listens on; removed when dropped.
#[derive(Debug)]
pub struct PortFile {
	path: PathBuf,
}

impl PortFile {
	/// Write `{ port, pid }` to `path`, creating its directory.
	///
	/// Written to a sibling and renamed into place, so a reader never sees a
	/// half-written file.
	///
	/// # Errors
	/// Any filesystem failure.
	pub fn write(path: &Path, port: u16) -> io::Result<Self> {
		if let Some(dir) = path.parent() {
			std::fs::create_dir_all(dir)?;
		}
		let contents = serde_json::json!({ "port": port, "pid": std::process::id() }).to_string();
		let mut staging = path.as_os_str().to_owned();
		staging.push(".tmp");
		std::fs::write(&staging, contents)?;
		std::fs::rename(&staging, path)?;
		Ok(Self { path: path.to_owned() })
	}
}

impl Drop for PortFile {
	/// Remove the file, unless another instance has written its own port
	/// there since: the newest dev server is the one a reader should find.
	fn drop(&mut self) {
		let ours = std::fs::read_to_string(&self.path)
			.ok()
			.and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
			.and_then(|record| record.get("pid").and_then(serde_json::Value::as_u64))
			== Some(u64::from(std::process::id()));
		if ours {
			let _ = std::fs::remove_file(&self.path);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{bind, PortFile};

	/// Production's rule: a taken port is a failure, not a move.
	#[tokio::test]
	async fn without_fallback_a_taken_port_is_an_error() {
		let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
		let port = taken.local_addr().unwrap().port();
		let err = bind("127.0.0.1", port, false).await.unwrap_err();
		assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
	}

	/// Dev's rule: a taken port moves up to the next free one.
	#[tokio::test]
	async fn with_fallback_a_taken_port_moves_up() {
		let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
		let port = taken.local_addr().unwrap().port();
		let listener = bind("127.0.0.1", port, true).await.unwrap();
		let bound = listener.local_addr().unwrap().port();
		assert!(bound > port && bound < port + super::FALLBACK_ATTEMPTS, "{port} -> {bound}");
	}

	/// A free port is taken as asked, fallback or not.
	#[tokio::test]
	async fn a_free_port_is_used_as_asked() {
		let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
		let listener = bind("127.0.0.1", free, true).await.unwrap();
		assert_eq!(listener.local_addr().unwrap().port(), free);
	}

	/// The file names the port and this process, and goes away with it —
	/// unless a newer instance has claimed it.
	#[test]
	fn the_port_file_records_this_process_and_is_removed_on_drop() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("nested").join("dev-port.json");
		let file = PortFile::write(&path, 3001).unwrap();
		let record: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
		assert_eq!(record["port"], 3001);
		assert_eq!(record["pid"], u64::from(std::process::id()));
		drop(file);
		assert!(!path.exists(), "removed on drop");

		let file = PortFile::write(&path, 3001).unwrap();
		std::fs::write(&path, r#"{"port":3002,"pid":1}"#).unwrap();
		drop(file);
		assert!(path.exists(), "another instance's record is left alone");
	}
}
