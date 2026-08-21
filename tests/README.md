# Tests

Tests live alongside the code they cover plus the shared assets and harnesses
in this directory, per the testing strategy in `AGENTS.md`.

## Layers

- **Unit tests** — in-crate, next to the code: normalized state mapping, path
  validation, archive path sanitization, post-processing rules, API
  validation, database transitions, and backend error translation.
- **Integration tests** — `tests/integration/`: real transfers end-to-end
  against local fixtures and a mock slskd API; no public network required.
- **End-to-end** — the canonical E2E path is: add a local test torrent →
  complete the download → extract a fixture archive → organize the file →
  observe all expected API events.

## Network policy

- Torrent integration tests use known legal fixtures, locally generated
  torrents, or test data only.
- Soulseek tests prefer mocks/fixtures; normal CI must not depend on the
  public Soulseek network.
- Soulseek live-network tests are opt-in only.

## Shared fixtures

See [tests/fixtures/README.md](fixtures/README.md) for what lives in
`tests/fixtures/` and how it is generated.

## Integration harness

See [tests/integration/README.md](integration/README.md) for the harness and
the mock slskd fixture.

## Running

```bash
cargo test --workspace
```

Frontend checks run from `apps/desktop/package.json`
(`npm run typecheck`, `npm run build`).
