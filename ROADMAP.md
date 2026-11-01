# Roadmap

From `SPEC.md` (re-approved 2026-10-31) to 1.0: Uncharted on the AMD handheld, the Strix Halo tablet and the RTX 3090 Ti desktop sharing saves through Play alone, through the service chosen at setup: Nextcloud, Google Drive or Dropbox (`docs/adr/0002-storage-backends.md`). How the code is laid out: `docs/code-map.md`. Each feature below is one GitHub issue (number in brackets), sized by the pipeline's rubric; Easy ones carry a Qwen brief and locked tests.

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
| F27 | Service names, the config's service field, token accounts | `service` (pure part) | M6, M9 | #27 |
| F28 | PKCE verifier and S256 challenge | `oauth` (pure) | M6, M10 | #28 |
| F29 | Sign-in URL with the narrow scope; the loopback redirect | `oauth` (pure) | M6, M10 | #29 |
| F30 | Token answers and refresh timing (`reauth_needed`) | `oauth` (pure) | M6 | #30 |
| F31 | Dropbox request arguments (rev-checked upload) and the answer table | `dropbox` (pure) | M9 | #31 |
| F32 | Dropbox list and metadata answers | `dropbox` (pure) | M9 | #32 |
| F33 | Drive queries, file answers, oldest-wins | `gdrive` (pure) | M9, M10 | #33 |
| F34 | The Drive guard's decision table | `gdrive::guard` | M9 | #34 |
| F35 | `setup --service`, `--no-browser`, `--finish` parsing | `cli::parse` | M6 | #35 |
| F36 | The HTTP transport (ureq + rustls) | `http::UreqTransport` | M1-M6, M9 | #37 |
| F37 | `TokenSource`: access tokens from the stored refresh token | `oauth::TokenSource` | M6 | #38 |
| F38 | `FakeDropbox` | `fake` | M9 | #39 |
| F39 | `FakeDrive` (with Drive's 409 on a used id) | `fake` | M9 | #40 |
| F40 | The Dropbox backend | `dropbox::Dropbox` | M6, M9, M10 | #41 |
| F41 | The Google Drive backend and its M9 guard | `gdrive::Drive` | M9, M10 | #42 |
| F42 | Sign-in flows: browser, Dropbox code, Google device code, Game Mode | `oauth` | M6, M10 | #43 |
| F43 | The M9 suite: the same tests against every service | `tests/remote_contract.rs`, `tests/sync_services.rs` | M1-M5, M9 | #44 |
| F44 | The service choice in setup and status | `service::open`, `cli::run` | M6, M8, M10 | #45 |
| F45 | Choosing and setting up a service | `docs/services.md` | M6, M10 | #46 |
| F46 | Real round trips on Dropbox and Google Drive; Game Mode sign-in (`needs-owner-device`) | `tests/live_services.rs` | M6, M9, M10 | #47 |
| F47 | Register the app with Google and Dropbox (`needs-owner-device`, the owner's clicks) | `src/clients.rs` | M6, M10 | #36 |

Every Must has an issue: M1 (F07, F11, F19, F20, F23, F43), M2 (F02, F03, F20, F23, F43), M3 (F07, F19, F20, F23, F43), M4 (F08, F21, F23, F43), M5 (F07, F12, F20, F23, F43), M6 (F13, F14, F18, F22, F27-F30, F35, F37, F42, F44-F47), M7 (F01, F05, F23, F25), M8 (F09, F10, F22, F24, F26, F44), M9 (F12, F27, F31-F34, F36, F38-F41, F43, F46), M10 (F28, F29, F33, F40-F42, F44-F47).

## Mac, PC and Linux (R04)

- **Linux** (SteamOS on the two handhelds) and **Windows** (the desktop) are the 1.0 platforms; **macOS** is a Won't for 1.0, but CI builds and tests it on every push because it is free on a public repo, so the core stays ready for the 2.0 Mac port. All three run the same `cargo fmt`, `clippy` (default and no features), `build` and `test`.
- What differs per OS is kept in three places: `platform` (folders), `creds::KeyringStore` (Secret Service, Credential Manager, Keychain), and the file system edge of `scan` and `transfer` (paths in a save set are always `/`; Windows file names that cannot exist are an `io` error, not a silent skip).
- **Linux risk:** a launcher started from Steam's Game Mode may find no unlocked Secret Service. F18 finds out on the handhelds first; if the store is missing there, it stops and asks, because a fallback store would change M6. The same store holds the Google Drive and Dropbox refresh tokens, so the answer covers every service.
- **Game Mode sign-in risk (Linux):** a browser started from a Game Mode launcher may never show on screen, so Drive and Dropbox setup there defaults to the code flows (`--no-browser`, then `--finish`; Google: a short code typed on a phone; Dropbox: a code typed back, or Steam's own browser). Recommended: set up once in Desktop Mode. F42 builds it, F46 checks it on a handheld.
- **Browser start per OS:** `xdg-open` (Linux), `rundll32 url.dll,FileProtocolHandler` (Windows), `open` (macOS), no shell; the loopback redirect is plain `std::net`, the same on all three.
- **Android and iOS** (2.0): no platform-only dependency in the core; the network and the credential store are traits (`http::Transport`, `CredentialStore`) with their real implementations behind the `http` and `keyring` features, and the three backends are plain Rust over `Transport`, so a mobile app links the library and brings its own transport, store and browser session.

## Open-source pieces

| Project | License | Use |
|---|---|---|
| mtkennerly/ludusavi | MIT | **Learn**, maybe adopt small pieces with credit: its wrap-the-launch flow (back up after quit, restore before start), its cloud conflict check (compare local and cloud against the last sync) and its backup retention (full backups capped by count). Our core stays small instead of depending on it: it is a whole app (GUI, rclone, the big manifest), and we need one game folder and one server. Placeholder ideas for manifests (`<home>`, `<winAppData>`) informed `{home}`, `{data}`, `{config}`, `{documents}`. |
| mtkennerly/ludusavi-manifest | MIT | **Learn**: the shape of a per-game save location record. Ours is one small JSON file per port, not a shared database. |
| rclone/rclone | MIT | **Learn** its WebDAV behaviour with Nextcloud: set the modification time on upload (`X-OC-Mtime`), read `getetag` from PROPFIND, retry on 423/5xx, chunked upload only for big files (saves are small: after 1.0). **Use** `rclone serve webdav` as a test server on the Windows and macOS CI jobs (F23). |
| sigoden/dufs | MIT / Apache-2.0 | Alternative single-binary WebDAV test server, if `rclone serve webdav` misses a method we need. |
| nextcloud/docker (the `nextcloud` image) | AGPL-3.0 (server; used, not linked) | The truest test server: a service container on the Linux CI job (F23), and the test container on the RTX 3090 Ti desktop for the R12 check of F17. |
| algesten/ureq + rustls | MIT / Apache-2.0 | The HTTP client behind the `http` feature (`http::UreqTransport`, F36), shared by all three backends and the sign-in (pure Rust TLS: no OpenSSL to find on Windows or SteamOS). |
| rust-random/getrandom | MIT / Apache-2.0 | The OS's secure random source for the PKCE verifier and `state` (F42); pure Rust, supports Android and iOS too. |
| Dropbox HTTP API v2, Google Drive REST API v3, Google and Dropbox OAuth | the services' terms (used, not linked) | Called over HTTPS with our own small request builders and parsers (F31-F33), not an SDK: the official SDKs are not small pure-Rust crates, and we need eight endpoints each. The app registrations are the owner's (F47). |
| rclone/rclone (again) | MIT | **Learn** its Drive and Dropbox backends: Drive's repeated names and folder resolution, Dropbox's `Dropbox-API-Arg` escaping and rate-limit retries. Not used as a program (ADR 0002, ruled out). |
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
6. **The other two services** (F27-F47, M6, M9, M10; `docs/adr/0002-storage-backends.md`), in parallel with stages 3-5 because the backends meet the engine only at the `Remote` trait:
   - **6a, pure parts, Easy, free now:** F27 (service names), F28-F30 (PKCE, sign-in URL, token answers), F31-F32 (Dropbox), F33-F34 (Drive and its guard table); F35 (parsing) after #10; F45 (docs) after F35.
   - **6b, seams and fakes, Medium:** F36 (the transport, also unblocks F17), F37 (`TokenSource`), F38-F39 (the two fakes).
   - **6c, backends:** F40 Dropbox (Medium), F41 Drive with the M9 guard (Hard), F42 the sign-in flows with Game Mode (Hard).
   - **6d, together:** F43 the M9 suite against every backend (after F20, F21), F44 setup and status for every service (after F22).
   - **6e, the owner's part:** F47 registering the app (any time; nothing else waits on it until F46), then F46 the real round trips and Game Mode sign-in on a PC and a handheld (`needs-owner-device`). F46 is where Drive's 409-on-a-used-id, which the guard rests on, is checked for real.
7. **1.0 acceptance** (F26): uncharted_pc's launcher calls the released binary around Play (uncharted_pc #163); a save made on each of the three PCs loads on the others (`needs-owner-device`).
8. **Design pass** (R04, last before 1.0): **skipped, no UI.** save-relay has no screen; its only face is CLI text and the `--json` contract, and the prompts a player sees (the conflict choice, the service choice and the Game Mode sign-in code) are drawn by each port's launcher, so it gets its look in that port's own design pass. The wording that comes from here (the `message` lines and the prompt text in `docs/code-map.md` § API shape) is fixed by the API shape and reviewed with F22 and F44; the sign-in page the loopback shows is one plain sentence (F42). A CHANGELOG line records the skip when the roadmap is built.

## After 1.0

Android and iOS bindings (with the game ports' 2.0), with sign-in through the platform's browser session; signed macOS release; moving saves from one service to another (a Won't for 1.0: switching starts fresh); more services (OneDrive, S3) behind the same `Remote`; upload only changed files (server-side COPY of unchanged ones from the previous entry); Nextcloud chunked upload for large save sets; a `save-relay forget <game>`; pruning old conflict backups separately from pushes.
