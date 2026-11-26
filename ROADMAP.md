# Roadmap

From `SPEC.md` to `v1.0.0`. Save Relay does two jobs for the owner's game ports: it syncs saves between machines through the service chosen at setup (Nextcloud, Google Drive or Dropbox), and it supplies resumable private game-file chunks that thin apps pull, verify and install. It is not an app store. How the code is laid out: `docs/code-map.md`.

Updated: 2026-11-25

features:
- sync-engine: The sync engine, WebDAV client, CLI and launcher contract (F01-F23, F25, F69, #302, #311) (release: 1.0)
- services: Google Drive and Dropbox beside Nextcloud, with their sign-in flows (F27-F47) (release: 1.0)
- wrap: The open-elsewhere mark, wrap, install-wrap, save cards, the ludusavi data and the reusable Steam modules (F48-F61, F65, F66, #293) (release: 1.0)
- panel: The panel, its Game Mode shell and the Add games view (F62-F64, F68, F72-F78) (release: 1.0)
- hardening: The audit's blockers and should-fix items, and the shore-up steps of this roadmap (B1-B6, F79-F87, F90, #329) (release: 1.0)
- release: Release candidates, device checks and the 1.0 acceptance (F24, F26, F67, F71) (release: 1.0)
- mac-build: The supported macOS build, the CLI on Apple Silicon only, and Apple signing (F88, F91) (release: 1.0)
- mobile-bindings: The iOS library, the Android bindings with a release asset, and the iOS verify-and-publish entry point (F70, F89, F92, F93, #306) (release: 1.0)
- save-slots: Follow the selected save slot (#240, #301, #313) (release: 1.0)
- thin-app-install: Shared resumable game-file installation through Dropbox, Nextcloud and Google Drive, with the desktop `save-relay game-files` command (#305, #309, #310, #325) (release: 1.0)
- publisher: Chunked resumable uploads of prepared game files, so Google Drive installs work with `drive.file` (#328) (release: 1.0)
- threeds-bridge: 3DS and Azahar saves through a paired bridge (#300, #319) (release: after-1.0)
- launcher-reuse: craft-launcher starts the Decky plugin from this repo's `decky/` (#292) (release: after-1.0)

One line per feature, in the pipeline's block format: `- <id>: <title> (release: 1.0|after-1.0)`. `publisher` moved into 1.0 and `threeds-bridge` stays after-1.0 by the owner rulings of 2026-11-25. Labels that exist today: `feature:mac-build`, `feature:mobile-bindings`, `feature:save-slots`; the rest are housekeeping step H1.

## Status terms

Every status in this file and in `docs/code-map.md` uses these terms and no others:

| Term | Means | Evidence it needs |
|---|---|---|
| implemented | the code is on `main` | a path |
| tested | automated tests on `main` exercise it | the test file or CI job |
| released | it is in a tagged release | the tag |
| device-verified | the owner ran it on a real device or account | the issue holding the owner's output |
| in review (#PR) | on a branch with an open PR, not on `main` | the PR |
| planned (#issue) | not started, or design only | the issue |

A green fake-backed test, a published archive, a source inspection or a sentence in a doc is not device verification. **No capability in this repo is device-verified today:** no issue holds owner-run output from a handheld, a Mac, an iPhone, an Android phone or a live Dropbox or Google Drive account.

## Decisions

Owner rulings that shape this roadmap, newest first. The eleven rulings of 2026-11-25 answer the roadmap's former owner questions Q1-Q11 (#308); the Q number is kept so earlier references still resolve.

- **Owner ruling 2026-11-25 (Q1): the installer is a 1.0 Must, new M19.** A thin app downloads, resumes and verifies prepared files from Dropbox, Nextcloud and Google Drive. Device proof on all three services gates 1.0. `SPEC.md` is amended in step P12.
- **Owner ruling 2026-11-25 (Q2): Android becomes a 1.0 Must, with a release asset** (`.aar` or `.so`). Android is the installer's 1.0 reference consumer.
- **Owner ruling 2026-11-25 (Q3): the publisher (#328) is in 1.0,** so Google Drive installs work with `drive.file`.
- **Owner ruling 2026-11-25 (Q4): the desktop `save-relay game-files` command is in 1.0.**
- **Owner ruling 2026-11-25 (Q5): M17 is amended to the CLI on Apple Silicon only.** No Mac library asset and no Intel build for 1.0.
- **Owner ruling 2026-11-25 (Q6): Windows is unsigned by decision for 1.0.** The README covers SmartScreen; signing comes after 1.0.
- **Owner ruling 2026-11-25 (Q7): the two #324 C functions stay** as deprecated compatibility wrappers on the new engine, removed after 1.0.
- **Owner ruling 2026-11-25 (Q8): iOS** gets, before the installer C interface freezes, an entry point that verifies and publishes an externally downloaded file (background `URLSession`).
- **Owner ruling 2026-11-25 (Q9): 3DS.** An automatic newest-wins pick, with the loser kept and uploaded, satisfies M3 ("not silent"). The 3DS bridge is an After 1.0 feature.
- **Owner ruling 2026-11-25 (Q10): payload selection.** `game-files/<app>/<os>/` is the standard layout. ACNH's existing `acnh-universal-game/android` is read in place as a legacy layout, with no migration.
- **Owner ruling 2026-11-25 (Q11): phones write the open-elsewhere mark** at launch and clear it on a successful push when the app pauses or goes to the background. A failed push leaves the mark up as a warning.
- **Owner direction 2026-11-24:** Save Relay is the shared hub for Android/iOS chunks that thin apps pull, install and resume, alongside the save relay itself. Support Dropbox, Nextcloud and Google Drive. This is not an app store; the launcher repo is a separate component and is out of this implementation scope. Use ACNH's current downloader as the consumer reference. #305 defines the contract, #309 implements the core, #310 exposes CLI/mobile calls. For #300, use devkitPro/libctru and design the relay transport first.
- **Owner ruling 2026-11-17:** "The ipad os and mac versions are obviously going to need to support an external controller. Make sure that's in the road map"; confirmed: "Yes, 1.0, Mac and iPad (Recommended)". Save Relay has no Mac or iPad app of its own, so it adds no step here; each port's roadmap carries it.
- **Owner ruling 2026-11-17:** "The save relay Mac and iOS functions need to be brought into 1.0" and "Revise the roadmap"; confirmed: "Yes, all into 1.0 (Recommended)". macOS and iOS are Musts M17 and M18 in `SPEC.md`; `v1.0.0` waits on gates G1-G4 below (R43).
- **Owner ruling 2026-11-17:** "Fold the save relay into the Mac app via a step in the roadmap. Should be a 1.0 that exists in every universal app with and ios and Mac component. Revise all roadmaps if you need to."
- **Owner ruling 2026-11-16:** the audit's Minor list was split into four issues (#269-#272).
- **Owner ruling 2026-11-12:** Android bindings are a Should for 1.0 ("Bindings into 1.0 too"). Superseded by the ruling of 2026-11-25 (Q2): Android is a Must.

`SPEC.md` was approved on 2026-11-17 and edited on 2026-11-24 (installer scope, a changed Won't) without a new approval line. The rulings above (Q1, Q2, Q5) change it again: the amendment is owed under #308 as step P12, and this roadmap does not edit `SPEC.md`.

## Where it stands (2026-11-25)

**Audited main: `b0ce9dc`. Stage: Dev Loop.** Tags: `v1.0.0-rc.1` (2026-11-17) and `v1.0.0-rc.2` (`961513c`, 2026-11-18), both pre-releases. `main` is 33 commits past rc.2; `Cargo.toml`, `panel/package.json` and `decky/package.json` still say `1.0.0-rc.2`. The last three changes (#327, #322, #332) were pushed to `main` as squashes with their PRs closed, not merged, and `check.yml` cancels superseded runs on `main`, so 9 of the 14 pushes to `main` since rc.2 had their check cancelled; the audited commit had no completed check at audit time (step P6).

| Capability | implemented | tested | released | device-verified |
|---|---|---|---|---|
| Save sync core: pull, push, conflict, offline, history, restore, repair (M1-M5, M7, M8) | yes | `tests/sync_*.rs`, `tests/remote_contract.rs`, `tests/cli_run.rs`; CI `rust` x3 | rc.2 | none (#26) |
| Nextcloud (M6, M9) | yes | CI `integration`: real Nextcloud on Linux, `rclone serve webdav` on Windows and macOS (`tests/cli_webdav.rs`) | rc.2 | none (#26, #287) |
| Dropbox, Google Drive (M9, M10) | yes | fakes only: `tests/sync_services.rs`, `tests/dropbox_backend.rs`, `tests/drive_backend.rs`, `tests/gdrive_guard.rs`; `tests/live_services.rs` is ignored and has no recorded run | rc.2 | none (#47 has no owner run) |
| Open-elsewhere mark, wrap, install-wrap, save cards (M11-M14) | yes | `tests/baton.rs`, `tests/wrap*.rs`, `tests/steam_launch.rs`, `tests/sync_cards.rs` | rc.2 | none (#72) |
| Panel and Decky plugin (M15) | yes | `tests/panel.rs`, panel and decky vitest suites (CI `frontend`); `decky/test_main.py` is not run in CI | rc.2 | none (#72, #73) |
| Add games (M16) | yes | `tests/steam_games_flow.rs`, `tests/cli_steam_games.rs`, panel `add-games.dom.test.tsx` | rc.2 | none (#212: the Steam client call was source-inspected only) |
| macOS build (M17) | yes; signing is wired (`scripts/macos-sign.sh`) | CI `rust` on macOS, unsigned | rc.2, **unsigned** arm64 CLI; no Mac library asset | none (#287, #238) |
| iOS library and C interface (M18) | yes | `tests/ffi.rs`, `tests/ffi_header.rs` on desktop hosts; CI `mobile` and `bindings` are build and type-check only | rc.2: arm64 device static library zip | none (#289) |
| Android bindings (Must since 2026-11-25) | yes | as iOS; no emulator run | no Android asset in any release (a release asset is now required, P5) | none (no gate issue yet; G4 and G6 cover it) |
| Save slots | desktop `--slot`: yes; slot-suffixed manifest ids through the C interface (#313): yes | `tests/cli_run.rs`, `tests/ffi.rs`, `examples/slot-contract-v1.0.0-rc.2.json` | `--slot` in rc.1 and rc.2; the C-interface part is not released | none |
| Game-file installer, interim (#324) | Dropbox only: `src/game_files.rs`, two C functions | one unit test | not released | none |
| Game-file installer, three services (#309, #310) | in review (#331, draft) | on the branch only | no | none |
| Publisher (#328), in 1.0 | planned (#328) | - | - | - |
| 3DS bridge (#300, #319) | planned; design in `docs/3ds.md`, transport design in review (#330, draft) | - | - | - |

### Implemented and tested on main since rc.2, not released

| Issue / PR | What | Tested by |
|---|---|---|
| #306 / #307 | Android OAuth finish retries after the app returns to the foreground | `tests/ffi.rs` |
| #302 / #314 | launcher contract: positional `setup`, `status` with no game, `sign-out` | `tests/cli_services.rs` |
| #303 / #318 | manifest format 2 with `strict_scope` for games sharing a saves folder; rc.2 and older reject format 2 | `tests/shared_save_layout.rs` |
| #301 / #315 | slot contract example and safeguards | `tests/cli_run.rs`, `examples/slot-contract-v1.0.0-rc.2.json` |
| #293 / #320 | `steam::vdf::replace_string`, byte-preserving | `tests/steam_vdf.rs` |
| #311 / #317 | a password typed at a terminal is refused; stable install instructions | `tests/cli_terminal_password.rs` |
| #305 / #312 | installer contract, `docs/assets.md` (document only) | - |
| #321 | `docs/3ds.md` (document only) | - |
| #324 | interim Dropbox-only downloader and two C exports; does not meet `docs/assets.md` (Known risks K1) | one unit test in `src/game_files.rs` |
| #313 / #327 | slot-suffixed manifest ids through the C interface | `tests/ffi.rs` |
| #322 | `bindings/ios/example/` harness for #289 | CI `bindings` type-check only |
| #329 / #332 | debug-only `SAVE_RELAY_TEST_STEAM_RUNNING` seam so tests do not depend on a running Steam | `tests/cli_install_wrap_bulk.rs`, `tests/steam_games_flow.rs` |
| #205 | closed with rc.1 checksum and `version` output from Linux and Windows; both were satellite runs, not owner runs | - |

### Open pull requests

| PR | State | Disposition |
|---|---|---|
| #331 | draft | the installer; lands #309 and #310 (step P1) |
| #323 | open, conflicting | superseded by #331; close when #331 lands |
| #326 | draft | superseded by #331; close when #331 lands |
| #330 | draft | 3DS transport design; After 1.0 (ruling Q9), not a 1.0 blocker |

## Owner questions

None open. Q1-Q11 were ruled on 2026-11-25 and are written into Decisions above; the steps they governed are updated below.

## Run order to 1.0

Order: agent work (P) lands, then `v1.0.0-rc.3` (R), then the owner gates G1-G7 run against that tag, then the design pass (D), then the 1.0 acceptance (G8 and G9, which depend on D1) and the final tag. **rc.3 is not cut while the interim #324 downloader is exported in the header as it is:** #331 must have replaced its engine, and the two C functions remain only as deprecated compatibility wrappers on the new engine (Q7). P rows are independent unless a row says otherwise; an owner gate blocks only its dependents. A defect found at a gate gets its own issue, a fix and a new RC before the gate is repeated.

"needs issue" marks a step with no issue yet, with the title to file.

### P: agent work before rc.3

| Step | Issue | What | Blocker | Done when |
|---|---|---|---|---|
| P0 | #308 | this roadmap and the code map rewritten against `b0ce9dc` | agent | merged; `PIPELINE.md` Notes updated (H5) |
| P1 | #309, #310 / PR #331 | one installer engine for the three services with the CLI lane included; replaces `src/game_files.rs`; closes #323 and #326 | agent | on `main`; `docs/assets.md` holds without exception (no overwrite of a player file, containment and no-follow on every path, bounded range answers, SHA-256 before publication); tests on all three providers through fakes; the desktop `save-relay game-files` command is in (Q4); the interim downloader's behaviour is gone and the two #324 C functions are deprecated wrappers on the new engine, removed after 1.0 (Q7); #331 is rebased on P0 and moves the code map rows from "in review (#331)" to present in the same PR (it also edits `CHANGELOG.md` and `docs/code-map.md`) |
| P2 | needs issue: installer contract gaps before the C interface is versioned | the standard layout `game-files/<app>/<os>/` and a host-chosen layout argument so a legacy root (ACNH's `acnh-universal-game/android`) is read in place, with no migration (Q10) and a test over its partial folder; the changed-manifest update path (today it ends in `destination_conflict`); a free-space check and a storage-full error; the wording of pause (cancel keeps the checkpoint; one download per process) made the same in `SPEC.md`, `docs/assets.md` and the header | agent; after P2a | decided in #331 or split; the header documents each |
| P2a | needs issue: iOS verify-and-publish entry point (Q8) | a C entry point that verifies and publishes a file the app downloaded outside the library (background `URLSession`), with the Swift wrapper; written and documented before the installer C interface is frozen or versioned | agent; before P2 and P3 | in the header, `src/ffi.rs` and the Swift wrapper, with a test through `tests/ffi.rs`; the header documents it |
| P3 | needs issue: header and bindings parity test | Kotlin, JNI and Swift wrap every exported C function; a test fails when the header, `src/ffi.rs`, a `repr(C)` layout or a wrapper drifts | agent; after P1 | CI `bindings` fails on a missing wrapper |
| P4 | needs issue: C interface covers M18 | play-anyway, history, restore, where, sign-out and account status through the C interface; `held` through `tests/ffi.rs`; the phone rules for the mark (Q11: write it at launch; clear it on a successful push when the app pauses or goes to the background; a failed push leaves it up as a warning) written into `docs/android.md` and an iOS guide | agent | a phone app can answer every envelope the desktop launcher can |
| P5 | needs issue: release workflow gates | secret-bearing steps in a protected environment, tag-only, before any Apple secret is added; a final (non-rc) tag fails when the Mac binary is unsigned; the tagged commit must be on `main` with a successful check; deployment targets set for macOS and iOS; version equality across the three version files checked; release notes name the tag Mac ports pin; NOTICE and licences in every asset; an Android release asset (`.aar` or `.so`, Q2); the Mac asset is the arm64 CLI only, with no library asset and no Intel build (Q5); Windows stays unsigned by decision, so the unsigned check applies to the Mac binary only (Q6) | agent | a pull-request run of `release.yml` sees no secret; a dry tag run fails unsigned |
| P6 | needs issue: CI on main and coverage | the stale comment at `check.yml:18-19` corrected (it calls Linux and Windows "the 1.0 platforms" and macOS a matter for "the game ports' 2.0"; macOS is a Must, M17); no cancel-in-progress on `main`; `main` protected with required checks and no direct pushes; `decky/test_main.py` run; the release configuration (musl, `--release`, `panel-dist` on Windows and macOS, Decky and iOS zips) built on pull requests; tests in the `--no-default-features --features ffi` configuration; job timeouts; pinned toolchain or MSRV; checksummed Kotlin and rclone downloads; dependency audit and an update bot; the duplicate `mobile` builds removed | agent | every `main` commit has a completed check |
| P7 | needs issue: stored-data and C ABI compatibility rule | what a 1.x build promises to read (server tree, `state.json`, `playing.json`, cards, local files, manifest formats 1 and 2, installer checkpoints); a locked test reading a tree written by rc.2; what an older build does with format 2; one version call for the whole C interface | agent | rule in `SPEC.md` or an ADR; the test on `main` |
| P8 | needs issue: split what is left of #249 | (a) the panel serves its token on an unauthenticated page and sends no anti-framing headers; (b) the Decky backend accepts commands and flags the panel audit removed | agent | two issues filed and fixed; then #249 closes (H2) |
| P9 | needs issue: save-sync safety | symlinks treated the same on upload and download; bounded response sizes on every backend read; a local safety copy before a pull replaces saves; one lock per game across CLI, panel, wrap and the C interface; a rotated refresh token stored when it is issued, and its failure reported | agent | each has a test; see Known risks K4-K8 |
| P10 | needs issue: docs sync for the installer and the launcher contract | `docs/PRIVACY.md` (what is uploaded, mobile apps, game-file downloads, the shared Dropbox app folder); `README.md`; `docs/services.md` "Planned game-file downloads"; `docs/assets.md` contradictions; `docs/android.md` "verified on Android 15" without a recorded run; slot-suffixed ids in the header and `docs/manifest.md`; `GLOSSARY.md` (app id and game id, new terms); ADRs for the installer, slot ids, strict scope, the launcher contract; an iOS integration guide; supported OS versions and CPU types; `SECURITY.md` (new) and a threat model; the README's SmartScreen note for the unsigned Windows binary (Q6); how to remove Save Relay from a machine; doc tests for `docs/assets.md`, `docs/android.md` and the header | agent; after P1 | each doc matches `main`; doc tests in CI |
| P11 | needs issue: `CHANGELOG.md` by version | entries for everything since the #308 audit; a released / unreleased split; the misplaced top line fixed | agent | rc.3 notes can be cut from it |
| P12 | #308 (owed; `SPEC.md` is not edited here) | amend `SPEC.md` for the rulings of 2026-11-25: add M19 (a thin app downloads, resumes and verifies prepared files from Dropbox, Nextcloud and Google Drive; device proof on all three), make Android a Must, amend M17 to the CLI on Apple Silicon only; cite the owner ruling and add the approval line | agent, with the owner's approval line | `SPEC.md` carries M19, the Android Must and the amended M17; before the G6 and G2 rows are used |
| P13 | #328 | the publisher: chunked resumable uploads of prepared game files to the three services, so Drive installs work with `drive.file` (Q3) | agent; after P1 | on `main` with tests on all three providers through fakes; label and milestone 1.0 set (H1) |

### R: the release candidate

| Step | Issue | What | Blocker | Done when |
|---|---|---|---|---|
| R1 | needs issue: `v1.0.0-rc.3` | version bump in the three files; tag after P1, P2a, P5 and P13 (P3, P4 and P7 if they change the C interface); every asset downloaded and checked against `SHA256SUMS`; the run URL, checksums and `version` output copied into the issue, because rc.2's run evidence can no longer be retrieved | agent; after P1, P2a, P5, P13 | the release exists; consumers (Uncharted-Universal, ACNH-Universal) pin the tag, not a git commit |
| R2 | - | each later fix that a gate turns up ships in a further RC (rc.4, ...) under the same checklist | agent | - |

### G: owner gates

Each needs the owner's device, account or signing identity. An agent prepares; only the owner's recorded output closes it. G1-G7 run before the design pass; G8 and G9 depend on D1 and are the 1.0 acceptance after it.

| Step | Issue | What the owner does | Depends on | Done when |
|---|---|---|---|---|
| G1 | #287 | sets the six Apple secrets the release workflow reads (`MACOS_CERTIFICATE_P12_BASE64`, `MACOS_CERTIFICATE_PASSWORD`, `MACOS_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_APP_PASSWORD`; today only the two Google secrets are set); runs a pull and push on a Mac against Nextcloud | P5 first; then an RC | a signed, notarized arm64 Mac CLI asset in an RC, and the round-trip output on #287. Longest chain to 1.0: start first |
| G2 | #238 | - (agent closes) | G1; P12 | README names the Apple Silicon Mac CLI as supported and the tag ports pin; M17 (as amended) device-verified by #287 |
| G3 | #289 | runs `bindings/ios/example/` on an iPhone: a save pushed from the Mac pulls on the phone and back, matching hashes | G1, R1 | output on #289 |
| G4 | needs issue: mobile sign-in on real phones | Dropbox and Google Drive sign-in on one Android phone and one iPhone; records how each OS receives the redirect and whether Google accepts the desktop client from a phone. A new client type or redirect is an owner console step beside #36 | R1 | output on the issue; never run so far |
| G5 | #47 | real Dropbox and Drive round trips (`tests/live_services.rs`), Drive's 409 on a reused id, Game Mode code flows on a handheld | R1 | output on #47 |
| G6 | needs issue: installer acceptance on devices | downloads, interrupts, resumes and verifies prepared files on each of Dropbox, Nextcloud and Google Drive (Drive through the publisher), on Android (the reference consumer), on iOS (through P2a) and with the desktop `save-relay game-files` command | P1, P2a, P13, R1; P12 | output on the issue for all three services; gates 1.0 |
| G7 | #212 | on a handheld: Add games sets a launch option through the Steam client | R1 | pass: close #212. Fail: ship the list-only and Desktop Mode fallback the issue documents (needs issue), new RC, repeat. #212 needs milestone 1.0 (H1) |
| G8 | #72 | suspend hand-off; wrap and install-wrap on a PC and a handheld, one Proton game; the panel in Game Mode; whether Game Mode has an unlocked Secret Service; the musl binary on SteamOS | D1 | output on #72 |
| G9 | #26 | Uncharted on the three PCs through Play; the launcher's bundled Save Relay version recorded | G5, G8, D1 | output on #26. Uncharted-Universal #163 (launcher integration) is closed; its open consumers of an RC are tracked there |

### D: design pass (last stage before 1.0, R04)

| Step | Issue | What | Blocker | Done when |
|---|---|---|---|---|
| D1 | #73 | A first pass ran on 2026-11-06 in desktop Big Picture with fixture output; its findings are implemented and released in rc.1 and rc.2. What remains: the re-check on a SteamOS handheld at 1280x800 with a controller only, covering Add games, B and D-pad pass-through (#267) and the QR sign-in with a phone. The installer adds no Save Relay screen; its prompts belong to each thin app's design pass | owner device; after G7 | findings filed as `design-pass` issues and built; #73 closed |

### Final

| Step | Issue | What | Done when |
|---|---|---|---|
| F1 | needs issue: v1.0.0 release checklist | one list of blocking evidence, below | every row links owner output or a named test |
| F2 | same | tag `v1.0.0` from a commit whose check passed; signed Mac asset; `PIPELINE.md` stage moved; `CHANGELOG.md` entry | release published |

| Must | Automated evidence | Gate that closes it |
|---|---|---|
| M1-M5, M7, M8 | sync, contract and CLI suites | G9 |
| M6, M9, M10 | service suites over fakes; Nextcloud in CI | G5, G4 (phones) |
| M11-M14 | `tests/baton.rs`, `tests/wrap*.rs` | G8 |
| M15 | panel and decky suites | G8, D1 |
| M16 | `tests/steam_games_flow.rs` | G7 |
| M17 (CLI on Apple Silicon, ruling Q5) | CI macOS | G1, G2 |
| M18 | `tests/ffi*.rs` | G3, G4, P4, P2a |
| M19 installer (ruling Q1; `SPEC.md` amended by P12) | P1's and P13's tests | G6 |
| Android (a Must, ruling Q2) | CI `mobile` and `bindings`; the Android asset check (P5) | G4, G6 |

### H: housekeeping for the Project Manager

| Step | What |
|---|---|
| H1 | Labels and milestones: set milestone 1.0 and label `feature:publisher` on #328; most issues carry no `feature:<id>` label and no milestone; ten of the thirteen feature ids have no label. Set milestone 1.0 on #212, #309, #310, #308; move #240 out of After 1.0 (it shipped in rc.1); label and milestone #300 and #319 (After 1.0) and #325 (1.0); relabel #305 and #310 from `feature:mobile-bindings` to `feature:thin-app-install`, and label #309 the same; label #313 `feature:save-slots` (it has no feature label) |
| H2 | #249 stays open until P8's two issues exist, then closes |
| H3 | #325: part (a) is #309/#310, part (b) is #328, part (c) is outside this repo; rewrite or close it with those links |
| H4 | #26's body still cites the closed Uncharted-Universal #163 and omits the Mac, iOS and installer gates |
| H5 | `PIPELINE.md` Notes still name `7bfe1df` and point to the wrong place for the peer list; `PEERS.md` "Imported code" is empty while `NOTICE` records imported ludusavi code |
| H6 | Changes go through a merged PR with a completed check; #327, #322 and #332 did not |
| H7 | R42: artifact retention is in place; write it down in `docs/code-map.md` § Tests and CI and watch cache growth |

## F-number index

F numbers are historical ids in issue titles. All are closed and released in rc.2 except the open gates in the last column. B1-B6 = #223-#228 (the audit's blockers, closed).

| Ids | Issues | What | Modules | Musts | Open |
|---|---|---|---|---|---|
| F01-F10 | #1-#10 | pure core | `glob`, `stamp`, `layout`, `platform`, `manifest`, `state`, `plan`, `retention`, `api`, `cli` (parse) | M1-M8 | - |
| F11-F15 | #11-#15 | local I/O and the in-memory server | `scan`, `remote`, `creds`, `local`, `webdav` (pure part) | M1-M6 | - |
| F16-F19 | #16-#19 | PROPFIND, the WebDAV client, the system credential store, safe transfers | `webdav`, `creds`, `transfer` | M1-M6 | - |
| F20, F21 | #20, #21 | pull, push, status, history, restore | `sync` | M1-M5 | - |
| F22-F26 | #22-#26 | the CLI end to end, integration tests, release binaries, the manifest guide, the 1.0 acceptance | `cli`, `tests/`, `release.yml`, `docs/manifest.md` | M1-M8 | #26 (G9) |
| F27-F35 | #27-#35 | service names, PKCE, sign-in URLs, tokens, Dropbox and Drive request and answer tables, the Drive guard, setup parsing | `service`, `oauth`, `dropbox`, `gdrive`, `gdrive::guard`, `cli` | M6, M9, M10 | - |
| F36-F46 | #37-#47 | the HTTP transport, `TokenSource`, the two fakes, the two backends, sign-in flows, the same suite on every service, the services guide, live round trips | `http`, `oauth`, `fake`, `dropbox`, `gdrive`, `service`, `docs/services.md`, `tests/live_services.rs` | M6, M9, M10 | #47 (G5) |
| F47 | #36 | the app registrations with Google and Dropbox | `clients` | M6, M10 | - |
| F48-F52 | #53-#57 | the open-elsewhere mark, `Held`, `--anyway`, `where`, parsing of the new commands | `presence`, `plan`, `api`, `layout`, `sync`, `cli` | M11 | - |
| F53-F59 | #58-#64 | VDF, Steam facts, the ludusavi data, pending choices, `wrap`, `install-wrap` | `steam::vdf`, `steam`, `ludusavi`, `pending`, `wrap`, `steam::launch` | M12, M13 | - |
| F60, F61 | #65, #66 | save cards, and cards travelling with entries | `card`, `transfer`, `scan`, `sync` | M14 | - |
| F62-F64 | #67-#69 | the panel front end, `save-relay panel`, the Decky plugin | `panel/`, `panel`, `decky/` | M15 | - |
| F65-F68 | #70-#73 | M11-M14 integration tests, docs, device checks, the panel's design pass | `tests/`, `docs/wrap.md`, `docs/panel.md` | M11-M15 | #72 (G8), #73 (D1) |
| F69 | #48 | manifest `saves_from: "launcher"` | `manifest`, `cli` | M7 | - |
| F70 | #49 | Android and iOS library builds with the C interface | `ffi`, `include/save_relay.h` | M18; Android (Must) | - |
| F71 | #205 | the first release candidate | `Cargo.toml`, `release.yml` | M8 | - |
| F72-F78 | #207-#213 | Steam Cloud answers and their cache, `steam-games`, `install-wrap --no-steam-cloud`, `launch-option`, the Add games view, its tests and docs | `steam::cloud`, `cli`, `api`, `steam::launch`, `panel`, `panel/`, `decky/` | M16 | #212 (G7) |
| F79-F85 | #242-#248 | the audit's should-fix items S1-S13 | `sync`, `wrap`, `ludusavi`, `cli`, `decky/`, `http`, `dropbox`, `gdrive`, `transfer`, `plan`, workflows | M1-M3, M5, M7-M9, M11-M13, M15 | - |
| F86 | #249 (split into #266, #267, #268) | panel token and API surface, in-flight restore state, Game Mode B and D-pad, `install-wrap` edge cases (S14-S17) | `panel`, `panel/`, `decky/`, `steam::launch` | M13, M15 | #249 (P8, H2) |
| F87 | #250 (split into #269-#272) | the audit's Minor list | various | various | - |
| F88 | #238 | the supported macOS build | `release.yml`, `README.md` | M17 | #238 (G2) |
| F89 | #254 | Android and iOS bindings with platform browser sign-in | `bindings/`, `ffi`, `oauth` | M18; Android (Must) | - |
| F90 | #280 | the slow-upload test flake on macOS, closed by #284 | `tests/` | - | - |
| F91 | #287 | Apple signing secrets and a Mac round trip | - | M17 | #287 (G1) |
| F92 | #288 | the iOS library as a release asset | `release.yml` | M18 | - |
| F93 | #289 | the iOS device round trip | `bindings/ios/example/` | M18 | #289 (G3) |

## Mac, PC and Linux (R04)

| Platform | Save sync | Installer | Release asset | Open risk |
|---|---|---|---|---|
| Linux x86_64 (SteamOS handhelds) | implemented, tested (CI `rust`, `integration` with real Nextcloud), released rc.2 | interim: none on the CLI; in review (#331): C interface and engine, no CLI command yet; the desktop `save-relay game-files` command is in 1.0 (Q4, P1) | static musl binary; Decky zip | Game Mode may have no unlocked Secret Service and may show no browser (code flows are the default there); the musl binary and the Decky plugin have build-only automation; Proton saves kept in the registry are not covered; no aarch64 build |
| Windows x86_64 (desktop) | implemented, tested (CI `rust`, `integration` with rclone), released rc.2 | as Linux | unsigned `.exe`, unsigned by owner decision for 1.0; the README covers SmartScreen (Q6) | names that cannot exist on Windows are an `io` error, not a skip; no arm64 build |
| macOS arm64 | implemented, tested (CI `rust`, `integration` with rclone), released rc.2 unsigned | as Linux | arm64 CLI only by owner decision (Q5); no library asset and no Intel build for 1.0 | signing has not started (G1); `install-wrap` and the Steam-running check are untried on a Mac; no minimum macOS version is set |
| iOS arm64 | implemented; tested on desktop hosts only | interim C functions have no Swift wrapper; in review (#331); blocking download does not fit background transfers; the verify-and-publish entry point is P2a (Q8) | device static library zip in rc.2; no simulator slice or XCFramework | no device run (G3, G4); no minimum iOS version is set in the build |
| Android arm64 | implemented; tested on desktop hosts only | as iOS; the installer's 1.0 reference consumer (Q2); the legacy ACNH layout is read in place (Q10) | a release asset (`.aar` or `.so`) is required for 1.0 (P5); none today | no device gate; minimum API (24) is visible only in CI's linker name; no reference credential store |

What differs per OS lives in `platform` (folders), `creds::KeyringStore` (Secret Service, Credential Manager, Keychain; never exercised in CI), the file-system edge of `scan` and `transfer`, signal handling in `wrap`,. The browser is opened with `xdg-open`, `rundll32 url.dll,FileProtocolHandler` or `open`, with no shell. `install-wrap` edits `localconfig.vdf`, which Steam rewrites while it runs, so it refuses then and keeps a dated backup. The Decky plugin is Linux Game Mode only; `save-relay panel` runs on all three desktops. Mobile builds leave out the `http` and `keyring` features: the app supplies transport, credential store and browser session through callbacks.

## Known risks

Verified security and correctness findings with no issue of their own. Each is a step above or needs one.

| K | Risk | Where | Step |
|---|---|---|---|
| K1 | The interim downloader on `main` is exported over the C ABI and breaks `docs/assets.md`: it renames over existing files, has no symlink or containment guard, accepts a 200 answer to a range request, reads an unbounded body, and uses 4 MiB chunks against a 1 MiB contract. Not released; must not be. The two C functions stay only as deprecated compatibility wrappers on the new engine, removed after 1.0 (Q7) | `src/game_files.rs`, `include/save_relay.h` | P1 before R1 |
| K2 | `release.yml` passes the OAuth and signing secrets to pull-request and manual runs with no environment gate; a branch that edits the workflow can read them. Today that exposes the two Google secrets; it gets worse the moment G1 adds Apple's | `.github/workflows/release.yml` | P5 before G1 |
| K3 | A tag ships an unsigned Mac binary with only a warning; nothing stops `v1.0.0` doing the same | `scripts/macos-sign.sh` | P5 |
| K4 | Symlinks are skipped silently on upload and followed on download | `src/scan.rs`, `src/transfer.rs` | P9 |
| K5 | Backend reads are unbounded in memory, and whole files are held in memory; a save over 256 MiB cannot upload | `src/http.rs`, backends | P9 |
| K6 | A pull replaces local saves with no local safety copy and no guard against the game writing during the pull | `src/transfer.rs` | P9 |
| K7 | No local lock: CLI, panel, wrap and the C interface can sync one game at once; the threading contract of the C interface is unwritten | `src/sync.rs`, `src/ffi.rs` | P9 |
| K8 | Secrets are not zeroed; a rotated refresh token is stored only at command end and a failed store is ignored through the C interface | `src/creds.rs`, `src/oauth.rs`, `src/ffi.rs` | P9 |
| K9 | The panel serves its token on an unauthenticated page and sends no anti-framing headers | `src/panel.rs` | P8 |
| K10 | The Decky backend accepts commands and flags the panel audit removed | `decky/main.py` | P8 |
| K11 | Every stored format is "equals 1 or fail"; nothing tests that a newer build reads what rc.2 wrote; manifest format 2 is already rejected by rc.2 | `src/state.rs`, `src/manifest.rs` | P7 |
| K12 | A pull through the C interface writes a mark that never expires; a phone has no play-anyway and may be ended before it pushes | `src/presence.rs`, `src/ffi.rs` | P4 (rule: Q11) |
| K13 | The save-sync C interface has no version or struct-size negotiation; struct layouts and wrapper mirrors are not tested; no run on a mobile target, no sanitizer or Miri job | `src/ffi.rs`, `bindings/` | P3, P7 |
| K14 | 9 of the 14 pushes to `main` since rc.2 had their check cancelled; the audited commit had no completed check at audit time; `main` is unprotected; the tag workflow does not require a passing check | `.github/workflows/check.yml` | P6, P5 |
| K15 | WebDAV over plain `http://` sends the app password with only a text warning; a self-hosted server with a private CA cannot connect over HTTPS (bundled roots only); undocumented | `src/webdav.rs`, `src/http.rs` | P10 documents; After 1.0 A6 |
| K16 | No dependency audit, no update bot, floating toolchain, unverified CI downloads | workflows | P6 |
| K17 | Releases carry only `SHA256SUMS`: no signature or provenance; no licences inside the assets | `release.yml` | P5; After 1.0 A7 |
| K18 | A full cloud account has no error code and no free-space check; multi-GB game files sit in each player's own account | backends, installer | P2 |
| K19 | Removing the binary or the Decky plugin leaves Steam launch options pointing at a missing program; `localconfig.vdf` backups are never pruned | `src/steam/launch.rs` | P10 documents; needs issue: uninstall command and backup pruning |
| K20 | `sign-out` deletes local tokens but does not revoke them at the provider | `src/cli.rs` | After 1.0 A8 |
| K21 | Mobile sign-in uses the desktop Google client and a loopback redirect; how an iPhone receives the redirect is undocumented and unrun; Android has no reference credential store | `src/clients.rs`, `bindings/` | G4, P10 |
| K23 | The Windows binary is unsigned by owner decision for 1.0: SmartScreen warns on first run | `release.yml`, `README.md` | P10 (README note); signing After 1.0, A7 |
| K22 | A source build needs a second repository (`craft-libs`, pinned by commit) and, for Drive, build-time Google secrets it does not have | `Cargo.toml`, `src/clients.rs` | R1 (consumers pin release assets) |

Statements removed from the earlier roadmap because no evidence backs them: "Dropbox works according to the owner" (#47 has no owner run) and the Android 15 device verification in `docs/android.md` (P10).

## After 1.0

| A | Item | Issue |
|---|---|---|
| A1 | Remove the two deprecated #324 C functions kept as compatibility wrappers in 1.0 (Q7) | no issue yet |
| A2 | 3DS and Azahar through a paired bridge (owner ruling Q9: an automatic newest-wins pick with a kept and uploaded loser satisfies M3); other emulators; an SD-card-only route | #300, #319, PR #330 |
| A3 | craft-launcher reuses `decky/` | #292 |
| A4 | Linux cloud-build hosting outside this repo | #325 part (c) |
| A5 | iOS simulator slice and XCFramework; Intel Mac, Linux aarch64 and Windows arm64 binaries; a Mac library asset | no issue yet |
| A6 | A private CA or the system trust store for self-hosted Nextcloud | no issue yet |
| A7 | Signed checksums and build provenance; Windows signing (unsigned for 1.0 by owner decision, Q6) | no issue yet |
| A8 | Revoke tokens at the provider on sign-out | no issue yet |
| A9 | One sign-in shared by several thin apps on a phone (account, device name, root) | no issue yet |
| A10 | Observability: a persistent log, a log hook in the C interface, installer progress by callback | no issue yet |
| A11 | Slots in the panel | no issue yet |
| A12 | Installer: small-file parallelism, network-policy guidance (metered, Wi-Fi only) | no issue yet |
| A13 | Refactors with no behaviour change: a structured HTTP status in `Error` instead of message prefixes; one shared retry and refresh layer for the three providers, with `Retry-After` on WebDAV; `Remote` without Drive- and WebDAV-specific hooks; `cli.rs` split; the mobile library without wrap, panel, Steam, ludusavi and the fakes; `Transport` and `Remote` with cancel, progress and `Send` for uploads and parallel chunks; the injected environment used everywhere instead of `std::env` | no issue yet |
| A14 | The panel's views on Android and iOS; moving saves between services; more services (OneDrive, S3); upload only changed files; Nextcloud chunked upload (overlaps the publisher, P13); `save-relay forget <game>`; pruning old conflict backups; registry saves for wrapped games; launchers other than Steam | no issue yet |

Closed and not pending: #240 (save slots, released in rc.1) and #295 (local writes use `craft-atomic`).

## Open-source pieces

| Project | License | Use |
|---|---|---|
| mtkennerly/ludusavi | MIT | learn its wrap flow, conflict check and retention; small imported pieces are recorded in `NOTICE` |
| mtkennerly/ludusavi-manifest | MIT | its data file for wrapped Steam games, downloaded with its ETag and cached, never bundled |
| saphyr-rs/serde-saphyr | MIT / Apache-2.0 | YAML reader for that data |
| craft-libs (`craft-atomic`) | `Cargo.toml` and `NOTICE` say MIT OR Apache-2.0; the repository's detected licence is Apache-2.0 only | atomic local writes; a git dependency pinned by commit, so a source build fetches a second repository |
| windows-sys | MIT / Apache-2.0 | the Windows Documents folder (`src/platform.rs`) |
| algesten/ureq + rustls (`ring`) | MIT / Apache-2.0, ISC | the HTTP client behind feature `http`; a build machine needs a C compiler |
| open-source-cooperative/keyring-rs | MIT / Apache-2.0 | the credential store behind feature `keyring` |
| rust-random/getrandom; RustCrypto `sha2`; `serde`, `serde_json`; `quick-xml` | MIT / Apache-2.0; MIT | PKCE randomness, hashes, JSON, PROPFIND |
| in review (#331): `caseless`, `unicode-normalization` | MIT / Apache-2.0 | installer path comparison |
| SteamDeckHomebrew decky-loader | GPL-2.0, host, used not linked | runs the Game Mode panel |
| @decky/ui, @decky/api; @decky/rollup | LGPL-2.1; BSD-3-Clause | the Decky plugin and its build |
| React 18, Vite, vitest, TypeScript, `qrcode` | MIT; Apache-2.0 (TypeScript) | the panel |
| rclone | MIT | learn its WebDAV, Drive and Dropbox behaviour; `rclone serve webdav` is the CI test server on Windows and macOS |
| nextcloud/docker | AGPL-3.0, server, used not linked | the CI test server on Linux |
| starship (signing script) | ISC | `scripts/macos-sign.sh` is adapted from it |
| Dropbox API v2, Google Drive v3, their OAuth | the services' terms | called over HTTPS with our own request builders, no SDK |
| libretro/RetroArch | GPL-3.0 | ideas only |
| zdyloves/game-save-sync-webdav, sparksquidremain/game-save-cloud-sync | MIT | learn; reviewed weekly (`PEERS.md`) |

## Development stages

1. Pure core (F01-F10). Done.
2. Local I/O and fakes (F11-F15). Done.
3. The server: PROPFIND, WebDAV client, credential store, safe transfers (F16-F19). Done.
4. The engine: pull, push, conflict, offline, status, history, restore (F20, F21). Done.
5. The tool: CLI, integration tests on three OSes, release binaries, manifest guide (F22-F25). Done.
6. The other two services (F27-F47). Done, except the live acceptance (G5).
7. Mark, wrap, cards, panel, Add games, audit hardening, Mac and mobile builds (F48-F93). Done, except the owner gates G1-G8 and the design pass D1.
8. The installer, the publisher, the iOS entry point and the steps P1-P13, then `v1.0.0-rc.3` (R1). In progress.
9. Owner gates G1-G7 against the release candidate.
10. **Design pass** (R04; D1, #73): the last stage before 1.0. Save Relay has a screen, the panel, in Game Mode and in the browser, so the pass is not skipped.
11. 1.0 acceptance (G8, #72; G9, #26) and the `v1.0.0` tag (F1, F2).
