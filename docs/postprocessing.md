# Post-Processing

Post-processing is rule-driven, conservative, and fully observable. It runs
after a download completes and turns raw downloads into organized, usable
files — without ever executing untrusted content automatically.

## Pipeline

```text
download completes
    ↓
classification
    ↓
optional verification
    ↓
extract
    ↓
inspect
    ↓
organize
    ↓
optional explicit executable action
    ↓
ready
```

Each step is individually observable and retryable. A failed step fails the
job; the job can be retried without redoing already-completed steps.

## Job and step model

A post-processing job belongs to one transfer and targets a specific file
(`target`). One transfer spawns 0..n jobs. Supported steps:

```text
verify
extract
flatten
rename
inspect_media
move
copy
hardlink
cleanup
run_installer
custom_hook
```

Step states: `pending`, `running`, `completed`, `failed`, `skipped`.
Job states: `pending`, `running`, `completed`, `failed`, `cancelled`.

## Classification

The classifier categorizes downloaded files using extension + metadata:

```text
audio
video
image
archive
document
software
unknown
```

Classification drives which pipeline applies and where files may be
organized.

## Archive handling

Recognized formats:

```text
.zip   .rar   .7z   .tar   .tar.gz   .tgz
```

- Multipart RAR/7z sets are handled as a single archive unit.
- Extraction goes through an `Extractor` adapter trait. The first
  implementation shells out to 7-Zip (`7z.exe` / `7zz`).
- **Path-traversal sanitization is implemented in our code**, not delegated to
  the extractor: archive entries such as `../../evil.exe` are normalized,
  canonicalized, and rejected if they would escape the assigned working
  directory.
- The source archive is **never deleted** until extraction succeeded and
  output validation passed.
- Archives are treated as hostile input; extracted content is validated before
  any further pipeline step runs.

## Media inspection

- `ffprobe` is used behind an adapter where available (duration, codec,
  resolution, tags).
- If ffprobe is absent, inspection is skipped — it never fails the job.

## Media organization

- Organization is configurable and conservative.
- Files are renamed/moved only within configured library roots; nothing is
  silently moved or renamed outside them.
- Categories map to configurable destinations (e.g. audio → Music,
  video → Movies).

## Installer execution policy

Downloaded executable content is untrusted. `run_installer` is a privileged
operation governed by hard rules:

1. Executables are **never** executed automatically, by default.
2. Extraction does **not** imply execution.
3. The UI must clearly display the executable path and the origin transfer
   before any launch.
4. API callers must request installer execution explicitly, passing a
   `confirmation_token`.
5. Remote API access cannot launch executables unless the permission is
   separately enabled.
6. A configurable confirmation policy gates execution.
7. An audit event is recorded whenever an executable is launched.
8. DRM bypass, crack application, license-key generation, and binary patching
   are never implemented as first-party features.

Denials answer `ProcessLaunchDenied`; missing/expired confirmation tokens
answer `PermissionDenied`.

## Retryability

Jobs and steps are persisted (`postprocess_jobs`, `postprocess_steps` in
SQLite). Completed steps are never re-run on retry; only `failed` steps and
their dependents re-run.
