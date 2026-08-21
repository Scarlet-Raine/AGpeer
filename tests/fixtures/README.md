# Test Fixtures

`tests/fixtures/` holds deterministic, legal test data used by integration
and end-to-end tests. Nothing here may be downloaded user content, torrent
payload data from the wild, or anything that would put CI on the public
network.

## Contents (planned)

- **Local test torrents** — locally generated `.torrent` files over a few
  small, synthetic files (e.g. text fixtures), used for add → complete
  integration tests.
- **Archives for extraction** — small `.zip`, `.rar`, `.7z`, `.tar`,
  `.tar.gz`, `.tgz` fixtures, including multipart RAR/7z sets and a
  malicious-path fixture containing `../evil.exe`-style entries, to exercise
  path-traversal sanitization.
- **Mock slskd API** — a mock slskd HTTP fixture covering login, search,
  incremental results, download, queue, completion, failure, and restart, so
  Soulseek-backed tests never depend on the public network.

## Conventions

- Fixtures are committed to the repository only when small and license-clean;
  generated fixtures must be reproducible from committed generator scripts.
- Large or downloadable fixtures are generated at test time, not committed.
- Never commit downloaded user content or third-party torrent payload data.

## Generating fixtures

Fixture generation is driven by scripts in this directory; running them must
not require the public Soulseek network or torrent swarms.
