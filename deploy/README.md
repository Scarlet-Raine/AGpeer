# Deployment assets

Shipable recipes for running the one-binary agpeer build
(`cargo build --release --features webui --bin agpeer`).

| File | Platform | What it does |
|---|---|---|
| `config.example.toml` | all | Example bootstrap config; mirrors the installed layout (Linux paths). `AGPEER_*` env vars overlay it per-run. |
| `service-install.ps1` | Windows (NSSM) | Installs `agpeer serve` as an auto-start service with crash restart and log rotation. Requires NSSM on PATH. |
| `systemd/agpeer.service` | Linux | systemd unit; runs as an unprivileged user with hardening and `Restart=on-failure`. |
| `../Dockerfile`, `../docker-compose.example` | Linux | Container build (multi-stage: WebUI → Rust → slim runtime) and a compose example. |

See `../README.md` (Environment variables) and `../docs/security.md` for the
token-bootstrap exposure notes.