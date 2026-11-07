# Architecture

```
┌──────────────────────── apps/desktop (Tauri 2) ────────────────────────┐
│  React UI (src/)            IPC (invoke / events)       src-tauri/     │
│  tokens · components ──────────────────────────────▶  commands.rs     │
│  zustand stores      ◀── install://progress (≤10 Hz)  state.rs        │
│                      ◀── game://log (batched 100 ms)                   │
└──────────────────────────────────────────┬─────────────────────────────┘
                                           │ plain Rust calls
┌──────────────────── crates/nc-launcher (services, async) ──────────────┐
│ download   meta   install   java   auth   process   fsutil   layout    │
└──────────────────────────────────────────┬─────────────────────────────┘
                                           │
┌──────────────────── crates/nc-core (pure, no I/O) ─────────────────────┐
│ manifest   rules   maven   libraries   profile   launch   java   paths │
│ secret                                                                 │
└────────────────────────────────────────────────────────────────────────┘
```

## Principles

1. **Pure core.** `nc-core` has no Tokio, no network and no file system. It is fast to test and reusable.
2. **Thin shell.** `src-tauri` validates input, calls a service and maps errors to `{ code, message }`.
   No business logic lives there.
3. **Untrusted metadata.** Version JSON, loader profiles, runtime manifests and (later) mod metadata
   are treated as hostile: every path is validated, Maven coordinates are checked segment by segment,
   and download URLs must be HTTPS.
4. **Secrets are typed.** Tokens live in `nc_core::Secret`, whose `Debug`/`Display` are redacted.
   `LaunchCommand` keeps its secrets separately so it can always produce redacted arguments.
5. **Throttle IPC.** Progress is sampled at 10 Hz and logs are batched every 100 ms, so heavy work never
   floods the WebView.

## Data directory (`%APPDATA%\NightClient`)

| Path | Contents | Shared? |
|---|---|---|
| `cache/` | version manifest (offline fallback) | — |
| `versions/<id>/` | version JSON, client jar, `.installed` marker | across instances |
| `libraries/` | Maven-layout libraries | across instances (acts as the cache) |
| `assets/` | `indexes/`, `objects/`, `log_configs/` | across instances |
| `runtimes/<component>/` | official Mojang Java runtimes | across instances |
| `instances/<name>/` | game directory, `.natives/<version>/` | per instance |

A version counts as installed only when its `.installed` marker exists. The marker is written atomically
after every file has verified.

## Download engine invariants

- A destination file is either absent or complete and verified (`.part` → verify → `rename`).
- `.part` files are resumed with `Range`. A complete `.part` (interrupted just before the rename) is
  verified without a network request.
- Retryable failures: network errors, timeouts, 408/429/5xx, size or hash mismatch. Not retryable:
  other 4xx, I/O errors, insecure URLs, insufficient disk space.
- The first non-retryable failure cancels the whole batch. The root cause is reported, not the
  cancellations it triggered.

## Loader extension point

Loaders produce a version JSON that `inheritsFrom` a vanilla version. `nc_core::profile::merge`
combines them: child scalars win, arguments are appended, and child libraries override parent
libraries with the same `group:artifact[:classifier]`.

- **Fabric:** profile JSON from Fabric Meta, merged as-is. The loader and intermediary jars carry no
  hash in the profile, so the installer must fetch their Maven `.sha1` files (Stage 4).
- **NeoForge:** needs installer processors to produce the patched client. See
  [spikes/neoforge.md](spikes/neoforge.md).
- **Forge and Quilt:** same shapes as NeoForge and Fabric respectively. Not implemented yet.

## Planned additions

- `nc-store`: SQLite with versioned migrations (Stage 1).
- DPAPI token vault in `src-tauri` (Stage 3).
- Modrinth client and resolver (Stage 5).
- Updater with signed artifacts (Stage 8).
