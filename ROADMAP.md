# Roadmap

From `SPEC.md` (approved 2026-10-31) to 1.0: Uncharted on the AMD handheld, the Strix Halo tablet and the RTX 3090 Ti desktop sharing saves through Play alone. How the code is laid out: `docs/code-map.md`. Each feature below is one GitHub issue (number in brackets), sized by the pipeline's rubric; Easy ones carry a Qwen brief and locked tests.

## Features

| Id | Feature | Module | Musts | Issue |
|---|---|---|---|---|
| F01 | Glob matching for include / exclude | `glob` | M7 | #1 |
| F02 | UTC time text (RFC 3339, entry stamps) | `stamp` | M2, M4 | #2 |
| F03 | Server paths, entry names, device names | `layout` | M2, M4 | #3 |
| F04 | OS folders from the environment | `platform` | M6, M7 | #4 |
| F05 | Manifest parse, validate, expand | `manifest` | M7 | #5 |
| F06 | `state.json` and last-synced JSON with checks | `state` | M1-M3 | #6 |
| F07 | The sync decision table | `plan` | M1, M2, M3, M5 | #7 |
| F08 | Keep the last 10 per game | `retention` | M4 | #8 |
| F09 | The `--json` envelope and exit codes | `api` | M8 | #9 |
| F10 | Argument parsing | `cli::parse` | M8 | #10 |
| F11 | Local save set with SHA-256 | `scan` | M1-M3 | #11 |
| F12 | In-memory test server | `remote::MemoryRemote` | M1-M5 | #12 |
| F13 | Memory credential store and account names | `creds` | M6 | #13 |
| F14 | This machine's config and records | `local` | M1-M6 | #14 |
| F15 | WebDAV URLs and path encoding | `webdav` (pure part) | M1-M6 | #15 |
| F16 | PROPFIND response parser | `webdav::parse_propfind` | M1-M5 | #16 |
| F17 | The WebDAV client for Nextcloud | `webdav::WebDav` | M1-M6 | #17 |
| F18 | System credential store | `creds::KeyringStore` | M6 | #18 |
| F19 | Safe upload and download of a save set | `transfer` | M1-M3 | #19 |
| F20 | `pull` and `push`: sync, conflict, offline | `sync` | M1, M2, M3, M5 | #20 |
| F21 | `status`, `history`, `restore` | `sync` | M4 | #21 |
| F22 | The CLI end to end, with `setup` | `cli::run` | M6, M8 | #22 |
| F23 | Integration tests against a WebDAV server in CI, with a fixture game | `tests/` | M1-M5, M7, M8 | #23 |
| F24 | Release binaries for Linux, Windows (and macOS) | `.github/workflows/release.yml` | M8 | #24 |
| F25 | Manifest guide for port maintainers | `docs/manifest.md` | M7 | #25 |
| F26 | 1.0 acceptance: uncharted_pc's launcher on the three PCs | uncharted_pc #163 | M8 (all) | #26 |

Every Must has an issue: M1 (F07, F11, F19, F20, F23), M2 (F02, F03, F20, F23), M3 (F07, F19, F20, F23), M4 (F08, F21, F23), M5 (F07, F12, F20, F23), M6 (F13, F14, F18, F22), M7 (F01, F05, F23, F25), M8 (F09, F10, F22, F24, F26).

## Mac, PC and Linux (R04)

- **Linux** (SteamOS on the two handhelds) and **Windows** (the desktop) are the 1.0 platforms; **macOS** is a Won't for 1.0, but CI builds and tests it on every push because it is free on a public repo, so the core stays ready for the 2.0 Mac port. All three run the same `cargo fmt`, `clippy` (default and no features), `build` and `test`.
- What differs per OS is kept in three places: `platform` (folders), `creds::KeyringStore` (Secret Service, Credential Manager, Keychain), and the file system edge of `scan` and `transfer` (paths in a save set are always `/`; Windows file names that cannot exist are an `io` error, not a silent skip).
- **Linux risk:** a launcher started from Steam's Game Mode may find no unlocked Secret Service. F18 finds out on the handhelds first; if the store is missing there, it stops and asks, because a fallback store would change M6.
- **Android and iOS** (2.0): no platform-only dependency in the core; the network and the password store are traits with their real implementations behind the `webdav` and `keyring` features, so a mobile app links the library and brings its own.

## Open-source pieces

| Project | License | Use |
|---|---|---|
| mtkennerly/ludusavi | MIT | **Learn**, maybe adopt small pieces with credit: its wrap-the-launch flow (back up after quit, restore before start), its cloud conflict check (compare local and cloud against the last sync) and its backup retention (full backups capped by count). Our core stays small instead of depending on it: it is a whole app (GUI, rclone, the big manifest), and we need one game folder and one server. Placeholder ideas for manifests (`<home>`, `<winAppData>`) informed `{home}`, `{data}`, `{config}`, `{documents}`. |
| mtkennerly/ludusavi-manifest | MIT | **Learn**: the shape of a per-game save location record. Ours is one small JSON file per port, not a shared database. |
| rclone/rclone | MIT | **Learn** its WebDAV behaviour with Nextcloud: set the modification time on upload (`X-OC-Mtime`), read `getetag` from PROPFIND, retry on 423/5xx, chunked upload only for big files (saves are small: after 1.0). **Use** `rclone serve webdav` as a test server on the Windows and macOS CI jobs (F23). |
| sigoden/dufs | MIT / Apache-2.0 | Alternative single-binary WebDAV test server, if `rclone serve webdav` misses a method we need. |
| nextcloud/docker (the `nextcloud` image) | AGPL-3.0 (server; used, not linked) | The truest test server: a service container on the Linux CI job (F23), and the test container on the RTX 3090 Ti desktop for the R12 check of F17. |
| algesten/ureq + rustls | MIT / Apache-2.0 | The HTTP client behind `webdav` (pure Rust TLS: no OpenSSL to find on Windows or SteamOS). |
| open-source-cooperative/keyring-rs | MIT / Apache-2.0 | The credential store behind `keyring`. |
| RustCrypto `sha2`, `serde`, `serde_json` | MIT / Apache-2.0 | Already in the skeleton. `quick-xml` (MIT) for F16. |
| libretro/RetroArch | GPL-3.0 | **Ideas only** (license): its WebDAV cloud sync on PC, Android and iOS is the precedent for the 2.0 mobile builds. |
| zdyloves/game-save-sync-webdav, sparksquidremain/game-save-cloud-sync | MIT | **Learn**: the simple version of the same job; checked weekly in the peer review. |

## Development stages

1. **Pure core** (F01-F10): all Easy, no dependencies between them except F03 (stamp) and F08 (layout). Satellites can take them in parallel.
2. **Local I/O and fakes** (F11-F15): the save set scan, the in-memory server, the memory credential store, this machine's files, WebDAV URLs. Easy.
3. **The server** (F16-F19): PROPFIND parsing, the real WebDAV client (checked on the Nextcloud test container, R12), the system credential store, safe transfers. Medium, one Hard (F18).
4. **The engine** (F20, F21): `pull`, `push`, conflict, offline, then `status`, `history`, `restore`, all against `MemoryRemote`. Medium.
5. **The tool** (F22-F25): the CLI end to end, integration tests against real WebDAV servers in CI on all three OSes, release binaries, the manifest guide.
6. **1.0 acceptance** (F26): uncharted_pc's launcher calls the released binary around Play (uncharted_pc #163); a save made on each of the three PCs loads on the others (`needs-owner-device`).
7. **Design pass** (R04, last before 1.0): **skipped, no UI.** save-relay has no screen; its only face is CLI text and the `--json` contract, and the one prompt a player sees (the conflict choice) is drawn by each port's launcher, so it gets its look in that port's own design pass. The wording that comes from here (the `message` lines and the prompt text in `docs/code-map.md` § API shape) is fixed by the API shape and reviewed with F22. A CHANGELOG line records the skip when the roadmap is built.

## After 1.0

Android and iOS bindings (with the game ports' 2.0); signed macOS release; upload only changed files (server-side COPY of unchanged ones from the previous entry); Nextcloud chunked upload for large save sets; a `save-relay forget <game>`; pruning old conflict backups separately from pushes.
