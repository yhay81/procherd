# ProcHerd store compatibility corpus

This corpus freezes durable run-store documents emitted by the published
ProcHerd contract. Current and future binaries must reopen every store version
and reject the corruptions declared by its manifest with the stable integrity
exit-code class.

The fixture is synthetic and contains no real process identity, secret, log,
or third-party content. Tests materialize the platform lock file at runtime
because lock contents are not durable data.

When the store contract intentionally changes:

1. preserve the existing version directory byte-for-byte;
2. add a new version directory and digest manifest;
3. retain an old-version reader or add a no-clobber migration;
4. test interruption and rollback before documenting the migration.

The integration test checks fixture SHA-256 values, exact JSON and NDJSON
round trips, status/log/list/lease/wait/stop/GC reads, stable corruption
signals, and manifest coverage. Repository attributes preserve LF bytes on
Windows, macOS, and Linux.
