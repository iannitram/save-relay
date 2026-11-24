# Roadmap

From `SPEC.md` (re-approved 2026-11-17, with M17 and M18) to 1.0: Uncharted on the AMD handheld, the Strix Halo tablet and the RTX 3090 Ti desktop sharing saves through Play alone, through the service chosen at setup: Nextcloud, Google Drive or Dropbox (`docs/adr/0002-storage-backends.md`); a game still open on another machine said before I play; any Steam game synced with one launch option; each save shown as a card in a Game Mode panel (`docs/adr/0003-baton-wrap-cards-panel.md`); and the same saves on a Mac, through a supported, signed build, and in an iPhone app, through the iOS library (M17, M18). How the code is laid out: `docs/code-map.md`. Each feature below is one GitHub issue (number in brackets), sized by the pipeline's rubric; Easy ones carry a Qwen brief and locked tests.

Updated: 2026-11-24

features:
- sync-engine: The sync engine, WebDAV client and CLI (F01-F26) (release: 1.0)
- services: Google Drive and Dropbox beside Nextcloud, with their sign-in flows (F27-F47) (release: 1.0)
- wrap: The open-elsewhere mark, wrap, install-wrap, save cards and the ludusavi data (F48-F61, F65, F66) (release: 1.0)
- panel: The panel, its Game Mode shell and the Add games view (F62-F64, F68, F72-F78) (release: 1.0)
- hardening: The audit's blockers and should-fix items (B1-B6, F79-F87) (release: 1.0)
- release: Release candidates, device checks and the 1.0 acceptance (F24, F26, F67, F71) (release: 1.0)
- mac-build: The supported macOS build and Apple signing (F88, F91) (release: 1.0)
- mobile-bindings: The iOS library and Android bindings (F89, F92, F93) (release: 1.0)
- thin-app-install: Shared resumable Android/iOS chunk installation through Dropbox, Nextcloud and Google Drive (#305, #309, #310; owner direction 2026-11-24) (release: 1.0)
- save-slots: Follow the selected save slot (#240, built; its milestone is After 1.0) (release: after-1.0)
- launcher-reuse: craft-launcher starts the Decky plugin from this repo's `decky/` (#292) (release: after-1.0)

One line per feature, in the pipeline's block format: `- <id>: <title> (release: 1.0|after-1.0)`. An id is the `feature:<id>` label once issues carry it; today only `mac-build`, `mobile-bindings` and `save-slots` exist as labels, and the others are labelled when I next touch their issues. `Updated:` is the date this file was last brought in line with the code and the issues.

## Decisions

Owner rulings that change this roadmap, newest first. Earlier rulings are quoted where they apply (`SPEC.md`'s approval line, "Where it stands", `CHANGELOG.md`).

- **Owner direction 2026-11-24:** Save Relay is the shared hub for Android/iOS chunks that thin apps pull, install and resume, alongside the save relay itself. Support Dropbox, Nextcloud and Google Drive. This is not an app store; the launcher repo is a separate component and is out of this implementation scope. Use ACNH's current downloader as the consumer reference. #305 defines the contract, #309 implements the core, #310 exposes CLI/mobile calls. For #300, use devkitPro/libctru and design the relay transport first.
- **Owner ruling 2026-11-17:** "The ipad os and mac versions are obviously going to need to support an external controller. Make sure that's in the road map"; confirmed in the question tool: "Yes, 1.0, Mac and iPad (Recommended)". Each game port's roadmap gets a 1.0 step proving its Mac app and its iPad app play with an external controller (Xbox, PlayStation and MFi through Apple's GameController framework). save-relay has no Mac or iPad app of its own (on the Mac it is a CLI and a library a port bundles; on iOS and iPadOS, a library inside a port's app), so it adds no row here.
- **Owner ruling 2026-11-17**, to the Project Manager: "The save relay Mac and iOS functions need to be brought into 1.0" and "Revise the roadmap"; confirmed in the question tool: "Yes, all into 1.0 (Recommended)". macOS and iOS move from the Shoulds to Musts M17 and M18 in `SPEC.md` (approved 2026-11-17), and step 15 (`v1.0.0`) waits on rows A1-A4. This supersedes the "Ships with iOS app (Recommended)" placement below.
- **Owner ruling 2026-11-17**, question tool: "Fold the save relay into the Mac app via a step in the roadmap. Should be a 1.0 that exists in every universal app with and ios and Mac component. Revise all roadmaps if you need to." Rows A1-A4 are save-relay's part. Earlier the same day, where a port's iPhone/iPad app comes after its 1.0: "Ships with iOS app (Recommended)" (superseded by the ruling above); then "Update the mac and ios save relay connection in all of the universal apps".

## Where it stands (2026-11-24)

**Audited main: `7bfe1df`; you are here: Dev Loop, rc.2 published, acceptance incomplete.** The audit inspected the current modules, CLI/FFI surfaces, frontend, workflows, release assets and open issues. All 11 checks on this commit succeeded: Rust on Linux/Windows/macOS, mobile cross-builds and wrapper builds, frontend, and WebDAV integration on all three desktop OSes. This is build/test evidence, not owner-device proof. Active PRs below are not counted as merged code.

- Stages 1-7, M16 implementation (F72-F78), six audit blockers B1-B6, and the split hardening implementations are merged. #212 remains open for the handheld Steam-client check; #73 remains open for controller-only design acceptance. #249 is an umbrella whose split implementation and device evidence must be reconciled, not a new unsplit coding task.
- `Cargo.toml`, panel and Decky are at `1.0.0-rc.2`. Published releases include rc.1 (2026-11-17) and rc.2 (2026-11-18). rc.2 has Linux, Windows, Mac arm64, iOS arm64 library zip, Decky zip, panel zip and `SHA256SUMS`. #288 is implemented and shipped. #205 is a stale release checklist to verify and close with evidence, not a missing packaging implementation.
- All three save backends are present. Dropbox works according to the owner; automated suites use fake Dropbox/Drive, while CI uses Nextcloud on Linux and rclone on Windows/macOS. #47 remains the live Dropbox/Drive/Game Mode acceptance matrix. Source builds without the Google client secrets cannot sign in to Drive; tagged release builds enforce them.
- Mac and iOS remain Musts M17/M18. CI and release asset presence do not prove Apple signing or a real device round trip: #287/#238 and #289 remain open. `bindings/ios/example/` does not yet exist at this checkpoint.
- **New thin-app scope is not implemented in Save Relay.** ACNH main `4529d152` downloads prepared game files on Android using Save Relay's Dropbox app credentials; that code is in ACNH, not this library. Its `acnh-universal-game/android` root must be accounted for in migration. #305 designs the common three-service contract; #309 and #310 build it. No standalone app catalog is planned here.
- Open integration work not covered by the old feature table: #301 slot contract safeguards, #302 launcher setup/status/sign-out, #303 shared Uncharted save folders, #306 Android OAuth foreground retry (PR #307), #293 reusable Steam VDF edits, and #300 3DS/Azahar. The latest scope and dependency table below is authoritative.

### Run order to 1.0

Independent agent rows run in parallel. An owner/device gate blocks its dependents, not unrelated coding. A Terra agent gets each implementation issue; the coordinating agent reviews, fixes, checks the exact pushed head and merges ready PRs. A blocked acceptance issue stays open until its actual evidence exists.

| Order | Issue | State at audited checkpoint | Next action / dependency |
|---|---|---|---|
| D1 | #308 | audit in progress | revise this roadmap and code map before counting new implementations |
| D2 | #306 / PR #307 | code/docs and regression on feature branch | Terra review, privacy fixes and exact-head CI, then merge |
| D3 | #302 | missing launcher contract | backward-compatible local global status, setup aliases/challenges, sign-out; test all services |
| D4 | #303 | shared-folder acceptance absent | prove per-game prefix isolation through sync/history/restore; protect other files |
| D5 | #301 | missing reusable slot fixture | two populated slots and empty third, conflict retries and suffixed JSON IDs; no folder-name inference |
| D6 | #293 | modules public, lossless edit missing | preserve all bytes except edited VDF value; no-default-features proof |
| D7 | #305 | installer contract absent | document ACNH-compatible private chunk installation and provider constraints |
| D8 | #309 | no shared installer core | after D7: manifest/path safety, bounded ranges on all providers, durable checkpoints and verified publication |
| D9 | #310 | no installer CLI/mobile API | after D8: session lifecycle, progress/pause/cancel and Rust/C/Kotlin/Swift contracts |
| D10 | #311 | audit defects found | after #302 CLI edits: private terminal password input and stable release installation instructions |
| D11 | #205 | rc.1 and rc.2 already released | check release workflow/assets, download/checksum/version proof, close checklist with evidence |
| D12 | #249 | split implementations merged | reconcile #266/#267/#268 against source; retain #73 device evidence gate |
| D13 | #292 | reuse coordination, no code move requested | preserve generic Decky bridge; record handoff instead of inventing implementation |
| D14 | #300 | SDK choice settled, transport undecided | devkitPro/libctru transport design first, then console/Azahar implementation and real 3DS gate |
| A1 | #287 -> #238 | owner signing and Mac round trip open | Apple signing secrets and real Nextcloud test; then mark Mac supported and name signed release pin |
| A2 | #288 | done: rc.2 iOS asset published | preserve archive layout and wrapper build checks |
| A3 | #289 | device acceptance open; test-app preparation possible | prepare iOS test app independently; real Mac/iPhone matching hashes after #287/#288 |
| A4 | #47 | live service acceptance open | owner sign-in; real Dropbox/Drive round trips, Drive reused-ID guard and handheld code flows |
| A5 | #212 -> #73 | automated Add games and fixes present | handheld Steam callback/echo, then controller-only design re-check |
| A6 | #72 | device acceptance open | suspend handoff, wrap/install-wrap and Game Mode; after A5 |
| A7 | Uncharted-Universal #163 | downstream integration, tracked there | shared CLI/slot contracts here; consumer integration stays in its repo |
| A8 | #26 | final three-PC acceptance open | after A4/A6/downstream integration, and retain Mac/iOS Must gates |
| Final | new release issue after acceptance | no final 1.0 release yet | all Musts, including new chunk scope, verified; tag `v1.0.0`, move stage and record CHANGELOG |

Historical milestones are preserved in the feature table and development-stage history: F01-F25, F27-F45, F48-F66/F69, M16 F72-F78, audit blockers #223-#228, hardening F79-F87, Mac F88/F91 and mobile F70/F89/F92/F93. Their merged code is not undone by remaining acceptance. A new defect gets its own issue/PR and ships in a later RC before acceptance is repeated. Neither a green fake test nor a published archive closes an owner-device check.

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
| F70 | Android and iOS library builds with a C interface, for game ports that embed save-relay (TotK) | `ffi`, `include/save_relay.h` | M18 (iOS); Should (Android) | #49 |
| F71 | The first release candidate, `v1.0.0-rc.1`, for the device checks | `Cargo.toml`, `.github/workflows/release.yml` (run) | M8 | #205 |
| F72 | Steam store answer parser (Steam Cloud yes, no, not a game, unknown); a game's name from its appmanifest | `steam::cloud` (pure), `steam` | M16 | #207 |
| F73 | The Steam Cloud answer cache: one request per installed game, kept a week | `steam::cloud::Cache` | M16 | #208 |
| F74 | `save-relay steam-games`: installed games with Steam Cloud state, saves, wrapped | `cli`, `api`, `steam` | M16 | #209 |
| F75 | `install-wrap --no-steam-cloud`, with `skipped` | `cli`, `steam::launch` | M16 | #210 |
| F76 | `save-relay launch-option`; the panel API and the Decky backend run the M16 commands | `cli`, `panel` (Rust), `decky/` | M16 | #211 |
| F77 | The panel's Add games view, in the browser and in Game Mode | `panel/`, `decky/` | M16 | #212 |
| F78 | M16 integration test and docs | `tests/`, `docs/wrap.md`, `docs/panel.md` | M16 | #213 |
| F79 | WebDAV claim recovery after a dropped push; `repair` against a paused push (audit S1, S2) | `sync` | M5, M9 | #242 |
| F80 | `wrap` handles termination signals: push and release the mark (audit S3) | `wrap` | M11, M12 | #243 |
| F81 | A wrapped game receives saves on a machine that never ran it; no holes in "refuse when unclear" (audit S4, S5) | `ludusavi`, `cli` | M12, M13 | #244 |
| F82 | Decky 60-second kill and the whole-request HTTP deadline (audit S6, S7) | `decky/`, `http` | M9, M15 | #245 |
| F83 | A retried write after a 5xx keeps the live entry; record first, prune best-effort (audit S8, S9) | `dropbox`, `gdrive`, `sync` | M1, M9 | #246 |
| F84 | Durable and atomic downloads; case-only names on Windows; include/exclude drift (audit S10-S12) | `transfer`, `sync`, `plan` | M1-M3, M7 | #247 |
| F85 | Pin release-workflow actions to SHAs; `permissions:` in `check.yml` (audit S13) | `.github/workflows/` | M8 | #248 |
| F86 | Panel token and API surface, restore in-flight state, Game Mode B and D-pad, `install-wrap` edge cases (audit S14-S17) | `panel`, `panel/`, `decky/`, `steam::launch` | M13, M15 | #266 (S14, S15), #267 (S16), #268 (S17); split from #249 |
| F87 | The audit's Minor list, split into four issues (owner ruling 2026-11-16) | various | various | #269 (items 2, 3, 5), #270 (4, 8, 9), #271 (1, 6, 7, 14), #272 (11, 12, 13); split from #250 |
| F88 | Supported macOS build (signed CLI and library) for the Mac game ports | `.github/workflows/release.yml`, `README.md` | M17 | #238 |
| F89 | Android and iOS bindings with platform browser sign-in | `bindings/`, `ffi`, `oauth` | M18 (iOS); Should (Android) | #254 |
| F91 | Apple signing secrets and a real Mac round trip against Nextcloud (`needs-owner-device`, the owner's clicks) | — | M17 | #287 |
| F92 | The iOS library as a release asset | `.github/workflows/release.yml` | M18 | #288 |
| F93 | iOS device round trip through the C interface (`needs-owner-device`) | `bindings/ios/example/` | M18 | #289 |

Every Must has an issue: M1 (F07, F11, F19, F20, F23, F43), M2 (F02, F03, F20, F23, F43), M3 (F07, F19, F20, F23, F43), M4 (F08, F21, F23, F43), M5 (F07, F12, F20, F23, F43), M6 (F13, F14, F18, F22, F27-F30, F35, F37, F42, F44-F47), M7 (F01, F05, F23, F25, F69), M8 (F09, F10, F22, F24, F26, F44, F71), M9 (F12, F27, F31-F34, F36, F38-F41, F43, F46), M10 (F28, F29, F33, F40-F42, F44-F47), M11 (F48-F52, F65, F67), M12 (F52-F54, F57, F58, F65-F67), M13 (F52-F56, F59, F65-F67), M14 (F52, F54, F60, F61, F65-F67), M15 (F52, F62-F64, F66-F68), M16 (F67, F72-F78), M17 (F88, F91), M18 (F70, F89, F92, F93). The Should: Android (F70, F89).

## Mac, PC and Linux (R04)

- **Linux** (SteamOS on the two handhelds) and **Windows** (the desktop) are the 1.0 platforms, and **macOS** is one too (M17, owner ruling 2026-11-17; a Should from 2026-11-12 until then; a supported, signed Mac build is #238, run-order rows A1 and A2); CI builds and tests it on every push because it is free on a public repo. All three run the same `cargo fmt`, `clippy` (default and no features), `build` and `test`.
- What differs per OS is kept in three places: `platform` (folders), `creds::KeyringStore` (Secret Service, Credential Manager, Keychain), and the file system edge of `scan` and `transfer` (paths in a save set are always `/`; Windows file names that cannot exist are an `io` error, not a silent skip).
- **Linux risk:** a launcher started from Steam's Game Mode may find no unlocked Secret Service. F18 finds out on the handhelds first; if the store is missing there, it stops and asks, because a fallback store would change M6. The same store holds the Google Drive and Dropbox refresh tokens, so the answer covers every service.
- **Game Mode sign-in risk (Linux):** a browser started from a Game Mode launcher may never show on screen, so Drive and Dropbox setup there defaults to the code flows (`--no-browser`, then `--finish`; Google: a short code typed on a phone; Dropbox: a code typed back, or Steam's own browser). Recommended: set up once in Desktop Mode. F42 builds it, F46 checks it on a handheld.
- **Browser start per OS:** `xdg-open` (Linux), `rundll32 url.dll,FileProtocolHandler` (Windows), `open` (macOS), no shell; the loopback redirect is plain `std::net`, the same on all three.
- **The M11-M15 parts:** the mark, the cards and the ludusavi translation are pure and the same on all three. `wrap` is checked on Linux (native games and Proton, Steam native and Flatpak) and Windows; macOS builds and tests it (Steam for Mac's folder is known); real Mac save round trips are a 1.0 gate in #287/#238. `install-wrap` is checked on Linux and Windows; the macOS path is handled but untried on a device. The Decky plugin is Linux Game Mode only; `save-relay panel` runs on all three and opens the browser the same way sign-in does.
- **Proton risk (Linux):** a Windows game's saves live inside its Proton prefix, whose user folder is always `steamuser`; F55 maps there, and F67 checks one Proton game on a handheld. A game that keeps saves in the prefix's registry is not covered (ludusavi's registry entries are not read).
- **Steam file risk:** `install-wrap` edits `localconfig.vdf`, which Steam rewrites when it runs; so it refuses while Steam runs (`steam_running`) and keeps a dated backup.
- **Android and iOS** (iOS a Must for 1.0, M18, owner ruling 2026-11-17; Android a Should, my ruling of 2026-11-12: "Bindings into 1.0 too"; #49 and #254): no platform-only dependency in the core; the network and the credential store are traits (`http::Transport`, `CredentialStore`) with their real implementations behind the `http` and `keyring` features, and the three backends are plain Rust over `Transport`, so a mobile app links the library and brings its own transport, store and browser session. For iOS, the release asset is #288 and the first device round trip #289 (run-order rows A3 and A4).

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
| libretro/RetroArch | GPL-3.0 | **Ideas only** (license): its WebDAV cloud sync on PC, Android and iOS is the precedent for the Android and iOS bindings. |
| zdyloves/game-save-sync-webdav, sparksquidremain/game-save-cloud-sync | MIT | **Learn**: the simple version of the same job; checked weekly in the peer review. |

## Development stages

Stages 1-7 are done (2026-11-06); the "free now" and "after" notes below are the order they were built in, kept as history. These numbered stages are historical build order. Current acceptance and new integration/installer work are in "Run order to 1.0" above.

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

#240 (save slots: a changed saves folder under one game id must not push or pull across slots; M7) is closed and built; it has no F row, and its milestone is still After 1.0 (2026-11-18: not moved, since it is done).

#292 tracks the craft-launcher Decky scaffold reuse coordination. #295 is closed: local state writes already use pinned `craft-atomic`; it is not pending work. #300 adds the separately scoped 3DS/Azahar route after transport design, with its own real-device gate.

The panel's views on Android and iOS, after the bindings' sign-in through the platform's browser session (the bindings themselves are in 1.0, #254: iOS a Must, M18; Android a Should); moving saves from one service to another (a Won't for 1.0: switching starts fresh); more services (OneDrive, S3) behind the same `Remote`; upload only changed files (server-side COPY of unchanged ones from the previous entry); Nextcloud chunked upload for large save sets; a `save-relay forget <game>`; pruning old conflict backups separately from pushes; saves kept in the Windows registry for wrapped games (ludusavi's registry entries); wrapping games from launchers other than Steam.
