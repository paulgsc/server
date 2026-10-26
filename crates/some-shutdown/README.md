# some-shutdown

`shutdown_signal()`, which every service binary waits on before its cleanup: it
resolves on Ctrl-C or SIGTERM ([#482](https://github.com/paulgsc/server/issues/482)).
See the crate docs for why and how.
