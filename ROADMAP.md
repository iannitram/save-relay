# Roadmap

From `SPEC.md` (re-approved 2026-11-01) to 1.0: Uncharted on the AMD handheld, the Strix Halo tablet and the RTX 3090 Ti desktop sharing saves through Play alone, through the service chosen at setup: Nextcloud, Google Drive or Dropbox (`docs/adr/0002-storage-backends.md`); a game still open on another machine said before I play; any Steam game synced with one launch option; and each save shown as a card in a Game Mode panel (`docs/adr/0003-baton-wrap-cards-panel.md`). How the code is laid out: `docs/code-map.md`. Each feature below is one GitHub issue (number in brackets), sized by the pipeline's rubric; Easy ones carry a Qwen brief and locked tests.

## Where it stands (2026-11-11)

**Audited checkpoint:** main at `a8e07ff` has Stages 1–7 merged (F01–F25, F27–F45, F48–F66 and F69). **You are here: Dev Loop, M16 implementation through F78 (#213), with Game Mode handheld acceptance still pending.** The stacked M16 branches add the Steam Cloud parser and cache, CLI, Add games view, integration coverage and usage docs.

- **Verified on main:** the [check run for `a8e07ff`](https://github.com/iannitram/save-relay/actions/runs/42939286063) passed Rust and WebDAV integration on Linux, Windows and macOS, plus the frontend build and tests. Dropbox and Drive have automated fake-service coverage. Live sign-in and round trips remain in #47.
- **Stage 8 remains open:** the 20 design findings (#164–#179, #197, #198, #201, #202) are fixed. #73 tracks the controller-only Game Mode re-check once the Add games view is ready.
- **M16 progress at the tip:** F72–F78 are assembled in the stacked implementation branches. The Add games UI and Steam-client echo path have automated coverage, but Game Mode has not been checked on a handheld. Finish that check before #73, then #36 registration can proceed in parallel; #205 waits on both #36 and #213.
- **Release and acceptance:** Cargo is at `0.0.0` and GitHub has no release. The owner can register Google and Dropbox in #36 while M16 proceeds. The release candidate (#205) waits for both. The device and Uncharted acceptance steps below remain open.

### Run order to 1.0

Each step is one issue. An automated run takes the **agent** steps whose "Waits on" is done, in this order, and stops at an **owner** step. The owner can do step 8 independently of steps 1–7.

| Step | Who | Issue | What | Waits on | Done when |
|---|---|---|---|---|---|
| 1 | agent | #207 (F72) | Steam store answer parser; a game's name from its appmanifest | nothing | its tests pass |
| 2 | agent | #208 (F73) | The Steam Cloud answer cache | step 1 | its tests pass against a fake transport |
| 3 | agent | #209 (F74) | `save-relay steam-games` | step 2 | the fixture Steam folder lists yes, no, unknown |
| 4 | agent | #210 (F75) | `install-wrap --no-steam-cloud` | step 3 | only the no-cloud game is wrapped; unknown is skipped |
| 5 | agent | #211 (F76) | `save-relay launch-option`; the panel API runs the M16 commands | step 3 | the three routes answer |
| 6 | agent, check on a SteamOS handheld | #212 (F77) | The panel's Add games view | steps 3, 4, 5 | vitest and the contract fixtures pass; the Game Mode add is tried on a handheld |
| 7 | agent | #213 (F78) | M16 integration test and docs | steps 4, 6 | the flow test and the doc test pass |
| 8 | owner | #36 (F47) | Register the app with Google and Dropbox; set the five values as this repo's Actions variables and secrets (a tagged release refuses to build without them) and put them in `src/clients.rs` | nothing, any time | `gh variable list` and `gh secret list` show the five names; the values are on `main` |
| 9 | agent, on a SteamOS handheld | #73 (F68) | Re-check the panel in Game Mode after #201 and #202, with the Add games view; file and fix any new `design-pass` finding; record the pass in `PIPELINE.md` and `CHANGELOG.md` | step 6 handheld acceptance | every view re-checked at 1280x800, controller only; #73 closed |
| 10 | agent | #205 (F71) | Cut `v1.0.0-rc.1`: version bump, release dry run, tag, checksum check | steps 7, 8 | the GitHub release holds the three binaries, `SHA256SUMS`, the Decky zip and the panel zip |
| 11 | owner, an agent may drive with the owner signing in | #47 (F46) | Real round trips on Dropbox and Google Drive; Game Mode sign-in | step 10 | `cargo test --test live_services -- --ignored` passes on Windows and a handheld; written up on #47 |
| 12 | owner | #72 (F67) | Device checks: suspend hand-off, wrap and install-wrap, the panel in Game Mode, adding a game without Steam Cloud | steps 9, 10 | written up on #72 |
| 13 | agent | Uncharted-Universal #163 | The launcher calls the released binary around Play. Its own note (2026-11-06) holds it until the integration contract is ready: the `--json` API shape in `docs/code-map.md` at the `rc.1` tag is that contract | step 10 | merged there, bundling a save-relay release |
| 14 | owner | #26 (F26) | 1.0 acceptance on the three PCs | steps 11, 12, 13 | written up on #26 |
| 15 | agent | new issue when step 14 passes | Tag `v1.0.0`; `PIPELINE.md` moves to Prototype / Beta with a CHANGELOG line | step 14 | the 1.0 release is out |

A bug found in steps 11, 12 or 14 becomes its own issue, is fixed by an agent, and ships in the next `rc` before the step is run again. Before step 10, a device can try a build from the release workflow's dry run (`gh workflow run release.yml --ref main`): Nextcloud works in it, Google Drive and Dropbox say they are not registered.

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
| F26 | 1.0 acceptance: the Uncharted launcher on the three PCs | Uncharted-Universal #163 | M8 (all) | #26 |
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
| F48 | `playing.json` mark: shape, checks, `presence::check` | `presence` (pure) | M11 | #53 |
| F49 | Plan row for `Held` (`decide_with_mark`); `Action::Held`, exit 6; `layout::playing_path` | `plan`, `api`, `layout` | M11 | #54 |
| F50 | Write and clear the mark in `pull`/`push`; `--anyway`; `held` in the data and `status` | `sync` | M11 | #55 |
| F51 | `save-relay where` | `sync::where_open`, `cli` | M11 | #56 |
| F52 | Parse `wrap`, `which`, `install-wrap`, `answer`, `where`, `picture`, `panel`, `pull --anyway` | `cli::parse` | M11-M15 | #57 |
| F53 | KeyValues (VDF) reader and writer | `steam::vdf` (pure) | M12, M13 | #58 |
| F54 | Steam facts: folders per OS, libraries, an app's library, user ids, screenshot pick | `steam` | M12-M14 | #59 |
| F55 | ludusavi entry to our manifest: placeholders, `when`, save tags, Proton prefix, refuse unclear | `ludusavi::translate` (pure) | M13 | #60 |
| F56 | ludusavi data: download with ETag, weekly refresh, cache, Steam index; `which` | `ludusavi::Cache` | M13 | #61 |
| F57 | Pending choices: file shape, `answer`, the 120-second wait | `pending`, `local` | M12 | #62 |
| F58 | `wrap`: pull, the game as a child, push, the child's exit code, pending choices | `wrap` | M12 | #63 |
| F59 | `install-wrap`: `LaunchOptions` in `localconfig.vdf`, backup, Steam-running check, `--remove` | `steam::launch` | M13 | #64 |
| F60 | Save card: shape, sidecar, picture check, playtime rule | `card` (pure) | M14 | #65 |
| F61 | Cards travel with entries: `put_card`/`read_card`, the session file, `server_card`/`local_card`, `HistoryEntry.card`, `picture`; `scan` and `transfer` skip `.save-relay-*` | `transfer`, `sync`, `local`, `scan` | M14 | #66 |
| F62 | Panel front end: views, `Bridge`, TypeScript types with contract fixtures, QR code, controller focus | `panel/` | M15 | #67 |
| F63 | `save-relay panel`: loopback server, token, embedded front end, `/api/*` | `panel` (Rust) | M15 | #68 |
| F64 | Decky plugin: `plugin.json`, Python backend over `--json`, quick access entry | `decky/` | M15 | #69 |
| F65 | M11-M14 integration tests: two simulated machines, a wrapped fixture program, fixture ludusavi data, a fixture sidecar | `tests/` | M11-M14 | #70 |
| F66 | Docs: wrap, which, install-wrap and Proton; the panel; the sidecar in the manifest guide | `docs/wrap.md`, `docs/panel.md`, `docs/manifest.md` | M12-M15 | #71 |
| F67 | Device checks: suspend hand-off on a handheld, wrap and install-wrap on a PC and a handheld, the panel in Game Mode (`needs-owner-device`) | — | M11-M15 | #72 |
| F68 | Design pass on the panel (stage 8, `design-pass`) | `panel/`, `decky/` | M15 | #73 |
| F69 | Manifest "saves_from": "launcher" (TotK) | manifest, cli | M7 | #48 |
| F71 | The first release candidate, `v1.0.0-rc.1`, for the device checks | `Cargo.toml`, `.github/workflows/release.yml` (run) | M8 | #205 |
| F72 | Steam store answer parser (Steam Cloud yes, no, not a game, unknown); a game's name from its appmanifest | `steam::cloud` (pure), `steam` | M16 | #207 |
| F73 | The Steam Cloud answer cache: one request per installed game, kept a week | `steam::cloud::Cache` | M16 | #208 |
| F74 | `save-relay steam-games`: installed games with Steam Cloud state, saves, wrapped | `cli`, `api`, `steam` | M16 | #209 |
| F75 | `install-wrap --no-steam-cloud`, with `skipped` | `cli`, `steam::launch` | M16 | #210 |
| F76 | `save-relay launch-option`; the panel API and the Decky backend run the M16 commands | `cli`, `panel` (Rust), `decky/` | M16 | #211 |
| F77 | The panel's Add games view, in the browser and in Game Mode | `panel/`, `decky/` | M16 | #212 |
| F78 | M16 integration test and docs | `tests/`, `docs/wrap.md`, `docs/panel.md` | M16 | #213 |

Every Must has an issue: M1 (F07, F11, F19, F20, F23, F43), M2 (F02, F03, F20, F23, F43), M3 (F07, F19, F20, F23, F43), M4 (F08, F21, F23, F43), M5 (F07, F12, F20, F23, F43), M6 (F13, F14, F18, F22, F27-F30, F35, F37, F42, F44-F47), M7 (F01, F05, F23, F25, F69), M8 (F09, F10, F22, F24, F26, F44, F71), M9 (F12, F27, F31-F34, F36, F38-F41, F43, F46), M10 (F28, F29, F33, F40-F42, F44-F47), M11 (F48-F52, F65, F67), M12 (F52-F54, F57, F58, F65-F67), M13 (F52-F56, F59, F65-F67), M14 (F52, F54, F60, F61, F65-F67), M15 (F52, F62-F64, F66-F68), M16 (F67, F72-F78).

## Mac, PC and Linux (R04)

- **Linux** (SteamOS on the two handhelds) and **Windows** (the desktop) are the 1.0 platforms; **macOS** is a Won't for 1.0, but CI builds and tests it on every push because it is free on a public repo, so the core stays ready for the 2.0 Mac port. All three run the same `cargo fmt`, `clippy` (default and no features), `build` and `test`.
- What differs per OS is kept in three places: `platform` (folders), `creds::KeyringStore` (Secret Service, Credential Manager, Keychain), and the file system edge of `scan` and `transfer` (paths in a save set are always `/`; Windows file names that cannot exist are an `io` error, not a silent skip).
- **Linux risk:** a launcher started from Steam's Game Mode may find no unlocked Secret Service. F18 finds out on the handhelds first; if the store is missing there, it stops and asks, because a fallback store would change M6. The same store holds the Google Drive and Dropbox refresh tokens, so the answer covers every service.
- **Game Mode sign-in risk (Linux):** a browser started from a Game Mode launcher may never show on screen, so Drive and Dropbox setup there defaults to the code flows (`--no-browser`, then `--finish`; Google: a short code typed on a phone; Dropbox: a code typed back, or Steam's own browser). Recommended: set up once in Desktop Mode. F42 builds it, F46 checks it on a handheld.
- **Browser start per OS:** `xdg-open` (Linux), `rundll32 url.dll,FileProtocolHandler` (Windows), `open` (macOS), no shell; the loopback redirect is plain `std::net`, the same on all three.
- **The M11-M15 parts:** the mark, the cards and the ludusavi translation are pure and the same on all three. `wrap` is checked on Linux (native games and Proton, Steam native and Flatpak) and Windows; macOS builds and tests it (Steam for Mac's folder is known) but is no 1.0 device check. `install-wrap` is checked on Linux and Windows; the macOS path is handled but untried on a device. The Decky plugin is Linux Game Mode only; `save-relay panel` runs on all three and opens the browser the same way sign-in does.
- **Proton risk (Linux):** a Windows game's saves live inside its Proton prefix, whose user folder is always `steamuser`; F55 maps there, and F67 checks one Proton game on a handheld. A game that keeps saves in the prefix's registry is not covered (ludusavi's registry entries are not read).
- **Steam file risk:** `install-wrap` edits `localconfig.vdf`, which Steam rewrites when it runs; so it refuses while Steam runs (`steam_running`) and keeps a dated backup.
- **Android and iOS** (2.0): no platform-only dependency in the core; the network and the credential store are traits (`http::Transport`, `CredentialStore`) with their real implementations behind the `http` and `keyring` features, and the three backends are plain Rust over `Transport`, so a mobile app links the library and brings its own transport, store and browser session.

## Open-source pieces

| Project | License | Use |
|---|---|---|
| mtkennerly/ludusavi | MIT | **Learn**, maybe adopt small pieces with credit: its wrap-the-launch flow (back up after quit, restore before start), its cloud conflict check (compare local and cloud against the last sync) and its backup retention (full backups capped by count). Our core stays small instead of depending on it: it is a whole app (GUI, rclone, the big manifest), and we need one game folder and one server. Placeholder ideas for manifests (`<home>`, `<winAppData>`) informed `{home}`, `{data}`, `{config}`, `{documents}`. |
| mtkennerly/ludusavi-manifest | MIT | **Learn**: the shape of a per-game save location record. Ours is one small JSON file per port, not a shared database. **Use** its data (`data/manifest.yaml`) for wrapped Steam games (F55, F56): downloaded with its ETag, cached, never bundled as code; a manifest of my own always wins. |
| saphyr-rs/serde-saphyr | MIT / Apache-2.0 | The YAML reader for the ludusavi data (F56): a pure-Rust serde deserializer, so the 17 MB file reads into typed structs without a whole tree (ADR 0003 has the crates ruled out). Added by its issue. |
| SteamDeckHomebrew decky-loader | GPL-2.0 (the host; used, not linked) | Runs the Game Mode panel (F64) in Steam's quick access menu. |
| @decky/ui, @decky/api | LGPL-2.1 | The Decky plugin's components and its calls to the Python backend (F64). |
| @decky/rollup | BSD-3-Clause | The Decky plugin's build (with rollup, MIT). |
| React (`react`, `react-dom`) | MIT | The panel's views, React 18 like Steam's own, which Decky supplies at run time (F62). |
| Vite, `@vitejs/plugin-react` | MIT | The desktop panel's build into `panel/dist` (F62, F63). |
| vitest | MIT | The front-end tests (F62, F64). |
| TypeScript | Apache-2.0 | The front ends' language and `typecheck`. |
| soldair/node-qrcode (`qrcode`) | MIT | The QR code of the Drive and Dropbox sign-in on the panel (F62). |
| rclone/rclone | MIT | **Learn** its WebDAV behaviour with Nextcloud: set the modification time on upload (`X-OC-Mtime`), read `getetag` from PROPFIND, retry on 423/5xx, chunked upload only for big files (saves are small: after 1.0). **Use** `rclone serve webdav` as a test server on the Windows and macOS CI jobs (F23). |
| sigoden/dufs | MIT / Apache-2.0 | Alternative single-binary WebDAV test server, if `rclone serve webdav` misses a method we need. |
| nextcloud/docker (the `nextcloud` image) | AGPL-3.0 (server; used, not linked) | The truest test server: a service container on the Linux CI job (F23), and the test container on the RTX 3090 Ti desktop for the R12 check of F17. |
| algesten/ureq + rustls | MIT / Apache-2.0 | The HTTP client behind the `http` feature (`http::UreqTransport`, F36), shared by all three backends and the sign-in (rustls with its `ring` crypto, which bundles its own C and assembly: no OpenSSL to find on Windows or SteamOS; a build machine needs a C compiler). |
| rust-random/getrandom | MIT / Apache-2.0 | The OS's secure random source for the PKCE verifier and `state` (F42); pure Rust, supports Android and iOS too. |
| Dropbox HTTP API v2, Google Drive REST API v3, Google and Dropbox OAuth | the services' terms (used, not linked) | Called over HTTPS with our own small request builders and parsers (F31-F33), not an SDK: the official SDKs are not small pure-Rust crates, and we need eight endpoints each. The app registrations are the owner's (F47). |
| rclone/rclone (again) | MIT | **Learn** its Drive and Dropbox backends: Drive's repeated names and folder resolution, Dropbox's `Dropbox-API-Arg` escaping and rate-limit retries. Not used as a program (ADR 0002, ruled out). |
| open-source-cooperative/keyring-rs | MIT / Apache-2.0 | The credential store behind `keyring`. |
| RustCrypto `sha2`, `serde`, `serde_json` | MIT / Apache-2.0 | Already in the skeleton. `quick-xml` (MIT) for F16. |
| libretro/RetroArch | GPL-3.0 | **Ideas only** (license): its WebDAV cloud sync on PC, Android and iOS is the precedent for the 2.0 mobile builds. |
| zdyloves/game-save-sync-webdav, sparksquidremain/game-save-cloud-sync | MIT | **Learn**: the simple version of the same job; checked weekly in the peer review. |

## Development stages

Stages 1-7 are done (2026-11-06); the "free now" and "after" notes below are the order they were built in, kept as history. What is left, M16 (F72-F78) included, is in "Run order to 1.0" above.

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
7. **The open-elsewhere mark, wrap, save cards and the panel** (F48-F68, M11-M15; `docs/adr/0003-baton-wrap-cards-panel.md`), in parallel lanes like stage 6:
   - **7a, pure parts:** F53 (VDF, Medium) free now; Easy F48 (the mark) and F60 (the card) after #2 (`stamp`), F49 after #3, #7, #9; Medium F57 (pending choices) after #2, #14, F55 (ludusavi translation) after #5, and F52 (parsing) after #10. F52, F53 and F57 were planned Easy and filed Medium (their tests run past the Easy limit).
   - **7b, Steam and the data, Medium:** F54 (Steam facts, after F53 and #4), F56 (the ludusavi data, after #37, F54, F55).
   - **7c, in the engine, Medium:** F50 (the mark in `pull`/`push`, after #20, F48, F49), F51 (`where`, after F50 and #22), F61 (cards with entries, after #19, #20, #21, F60).
   - **7d, wrap, Hard:** F58 `wrap` (after #20, F50, F52, F56, F57), F59 `install-wrap` (after F53, F54).
   - **7e, the panel:** F62 the front end (Hard; free now, on the fixtures and the `Bridge`), then F63 `save-relay panel` (Hard, after #22, F62) and F64 the Decky plugin (Medium, after F62).
   - **7f, together:** F65 the integration tests (after F50, F58, F61), F66 the docs (after F58, F63).
   - **7g, the owner's part:** F67 the device checks (`needs-owner-device`, after all of the above), F68 the design pass (stage 8).
8. **Design pass** (R04, before the 1.0 acceptance; F68): **no longer skipped**, because save-relay now has a screen: the panel (M15), in Game Mode and in the browser. The pass checks its views (the games list, the card timeline, restore, the conflict and held choices, pending choices, the QR sign-in) against the design system all my apps share, at 1280x800 with a controller only, and its findings become `design-pass` issues. The prompts a port's launcher draws (the conflict and held choices, the service choice and the sign-in code) still get their look in that port's own design pass; the wording from here (the `message` lines and the prompt text in `docs/code-map.md` § API shape) is fixed by the API shape and reviewed with F22, F44 and F62.
9. **1.0 acceptance** (F26, and F67 for M11-M15), last: the Uncharted launcher calls the released binary around Play (Uncharted-Universal #163); a save made on each of the three PCs loads on the others (`needs-owner-device`).

## After 1.0

#49 (F70: Android and iOS library builds with a C interface, for game ports that embed save-relay; TotK) is after-1.0.

Android and iOS bindings (with the game ports' 2.0), with sign-in through the platform's browser session (the panel's views can follow them there); signed macOS release; moving saves from one service to another (a Won't for 1.0: switching starts fresh); more services (OneDrive, S3) behind the same `Remote`; upload only changed files (server-side COPY of unchanged ones from the previous entry); Nextcloud chunked upload for large save sets; a `save-relay forget <game>`; pruning old conflict backups separately from pushes; saves kept in the Windows registry for wrapped games (ludusavi's registry entries); wrapping games from launchers other than Steam.
