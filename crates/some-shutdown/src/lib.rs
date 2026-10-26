//! The one shutdown signal every service binary waits on — see [#482][issue-482].
//!
//! Each service runs as PID 1 in its container, and `docker stop` sends
//! SIGTERM. A PID-1 process with no handler for SIGTERM ignores it, so Docker
//! waits out its grace period and sends SIGKILL, and the service's cleanup
//! never runs. [`shutdown_signal`] resolves on SIGTERM as well as on Ctrl-C.
//!
//! [issue-482]: https://github.com/paulgsc/server/issues/482

use std::future::Future;
use tokio::signal::unix::{signal, Signal, SignalKind};

/// Install handlers for SIGINT (Ctrl-C) and SIGTERM, and return a future that
/// resolves when either arrives.
///
/// The handlers are installed by this call, not when the future is first
/// polled, so a signal sent after it returns is never missed: it reaches the
/// future instead of taking its default action, which for both is to end the
/// process.
///
/// A handler that fails to install is logged and that signal is left
/// unwatched; the future waits on the other one alone. The failure is never
/// read as a signal, which would stop a service that has only just started.
/// If neither installs, the future never resolves.
///
/// # Panics
///
/// When called outside a Tokio runtime.
pub fn shutdown_signal() -> impl Future<Output = ()> + Send + 'static {
	let interrupt = listen(SignalKind::interrupt(), "SIGINT");
	let terminate = listen(SignalKind::terminate(), "SIGTERM");
	async move {
		let received = tokio::select! {
			() = next(interrupt) => "SIGINT",
			() = next(terminate) => "SIGTERM",
		};
		tracing::info!(signal = received, "shutdown signal received");
	}
}

fn listen(kind: SignalKind, name: &'static str) -> Option<Signal> {
	match signal(kind) {
		Ok(signal) => Some(signal),
		Err(error) => {
			tracing::error!(%error, signal = name, "failed to install the signal handler; this signal will not stop the service");
			None
		}
	}
}

/// Resolve on the next delivery, or never: when the handler was not installed,
/// or when the stream ends because the runtime is shutting down.
async fn next(signal: Option<Signal>) {
	if let Some(mut signal) = signal {
		if signal.recv().await.is_some() {
			return;
		}
	}
	std::future::pending::<()>().await;
}
