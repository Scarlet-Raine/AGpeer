# Integration Tests

`tests/integration/` hosts end-to-end tests that exercise the running core:
the API, a transfer backend, persistence, and post-processing together. They
must run without the public Soulseek network and without torrent swarms.

## Canonical E2E path

1. add a local test torrent;
2. complete the download;
3. extract the fixture archive;
4. organize the file;
5. observe all expected API events.

The test asserts that each stage produces the documented API events
(`transfer.added`, `transfer.started`, `transfer.progress`, `transfer.completed`,
`postprocess.*`, …) and that SQLite state matches after a restart where
applicable.

## Soulseek coverage

Soulseek-backed tests run against the **mock slskd API fixture**
(`tests/fixtures/`), covering login, search, incremental results, download,
queue, completion, failure, and restart. Live-network Soulseek tests are
opt-in and excluded from default CI.

## Harness

Tests boot the `agpeer` core on an ephemeral port with a temporary data
directory, read the generated bearer token, and drive the public API only.
Fixtures referenced here live in `tests/fixtures/`; see its README for the
catalog and generation rules.
