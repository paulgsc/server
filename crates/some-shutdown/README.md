# some-shutdown

One function, `shutdown_signal()`, that every service binary waits on before it
runs its cleanup ([#482](https://github.com/paulgsc/server/issues/482)).

Each service runs as PID 1 in its container. `docker stop` sends SIGTERM, and a
PID-1 process with no handler for it ignores it, so Docker waits out its grace
period and sends SIGKILL: the cleanup (closing the database, flushing NATS and
OpenTelemetry, draining WebSockets) never runs. `shutdown_signal()` resolves on
SIGTERM as well as on Ctrl-C (SIGINT).

- It installs both handlers when called, not when first polled, so a signal
  sent after it returns is never missed.
- A handler that fails to install is logged and that signal is left unwatched;
  the future waits on the other. It never resolves because of the failure.

`tests/sigterm.rs` sends SIGTERM to a child process waiting on it and requires
a clean exit within five seconds.
