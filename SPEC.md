# Spec

**1.0 means:** I finish a chapter of Uncharted on my AMD handheld, then start Uncharted on my Strix Halo tablet or my RTX 3090 Ti desktop, and the game loads that save, with no file copying by hand.

**Approved by me:** 2026-10-31, in the Project Manager session's question tool: "Approve (Recommended)". Recorded in `CHANGELOG.md` (2026-10-31).

A project is Spec'd only when the line above has a date and a link. Change a Must or a Won't after approval and the approval line goes back to "not yet" until I approve again.

## Musts

Each Must is something a person can do and see. Each one gets its own issue, and each issue names the check that proves it.

| # | Must | Done looks like | Check |
|---|---|---|---|
| M1 | WHEN I press Play in a game's launcher, THE LAUNCHER SHALL pull the newest saves for that game from my Nextcloud into the local saves folder before the game starts. | A save made on another machine is in the in-game load list. | integration test against a WebDAV test server; on the three PCs (`needs-owner-device`) |
| M2 | WHEN the game quits, THE LAUNCHER SHALL push the local saves to my Nextcloud. | The save is on the server, with the device and time recorded. | integration test; on the three PCs |
| M3 | IF both the server and this machine changed the saves since this machine last synced, THE LAUNCHER SHALL show both (device, time) and let me pick, keeping the other as a dated backup. | Nothing is overwritten without my choice; the loser is restorable. | integration test of the conflict path |
| M4 | EVERY push SHALL keep a dated copy, and the server SHALL keep the last 10 per game; I SHALL be able to list and restore any of them. | `save-relay history <game>` lists them; `restore` brings one back. | integration test |
| M5 | IF my Nextcloud is unreachable at launch, THE LAUNCHER SHALL start the game with the local saves, say so, and push at the next launch or exit that reaches the server, with the M3 check first. | The game starts offline; the save reaches the server later. | integration test with the server down |
| M6 | THE FIRST RUN on a machine SHALL ask once for my Nextcloud address and an app password, store the password in the system's credential store, and never write it to a file or a log. | One setup per machine; no password on disk. | unit test of the store; on each PC |
| M7 | A GAME SHALL join by adding one small manifest (game id, saves folder per platform, which files count as saves), with no code in save-relay. | A second game syncs with only a manifest. | test with a fixture game |
| M8 | THE CLI SHALL offer `pull`, `push`, `status`, `history`, `restore` for a game, each with `--json` for launchers, and uncharted_pc's launcher SHALL call it around Play on Linux and Windows. | Uncharted on the three PCs shares saves (the 1.0 sentence). | CLI tests in CI; on the three PCs |

## Won'ts

- Android and iOS bindings: they come with the game ports' 2.0 (Android, iOS with controller support, Mac), on this same core.
- macOS builds in 1.0 (the code stays portable; Mac comes with 2.0).
- Syncing while the game runs; only pull at launch and push at exit.
- Any storage other than Nextcloud over WebDAV; Steam Cloud.
- Settings, shader caches or logs: saves only.

## Prior art

Projects doing the same thing, approved by me at Spec and reviewed weekly. Their code comes in only when their license allows it, with credit; otherwise only their ideas.

peer: mtkennerly/ludusavi | MIT | the PC save backup tool: its wrap-the-launch flow, cloud conflict check and backup retention; main source to learn from (Rust, MIT, may be adopted with credit)
peer: mtkennerly/ludusavi-manifest | MIT | the shared database of where PC games keep saves; a model for our per-game manifest
peer: libretro/RetroArch | GPL-3.0 | WebDAV cloud sync of saves across PC, Android and iOS: the closest cross-platform precedent (ideas only unless the license fits)
peer: rclone/rclone | MIT | WebDAV/Nextcloud client behaviour: chunking, retries, modification times
peer: zdyloves/game-save-sync-webdav | MIT | a small Windows WebDAV save-sync tool; the simple version
peer: sparksquidremain/game-save-cloud-sync | MIT | multi-PC save sync tool

## Problem

My game ports keep saves in a folder on each machine. Moving from the handheld to the desktop means copying folders by hand, and I lose track of which copy is newest. Each port would otherwise grow its own sync code.

## Solution

One small library and command-line tool that every port's launcher calls: pull before Play, push after the game quits, through my own Nextcloud, with a conflict prompt, history and offline play. A game joins with a manifest. Android and iOS builds embed the same core later.

## User stories

1. As the owner, I want to continue my save on whichever machine I pick up, so that I never copy save folders by hand.
2. As the owner, I want to be asked when two machines both changed a save, so that I never lose progress silently.
3. As the owner, I want to restore an older save, so that a bad save or a wrong pick is recoverable.
4. As a port maintainer, I want to add a game with one manifest, so that every port shares the same pathway.

## Implementation decisions

- Rulings 2026-10-31 (question tool): Nextcloud over WebDAV with a per-device app password; pull at launch, push at exit; ask and keep both on conflict; shared library + CLI; 1.0 = Uncharted on the three PCs; last 10 versions per game; play local and push later when offline; new public repo; own small Rust core, with Ludusavi as the main peer to learn from.
- Core in Rust: a library crate and a `save-relay` binary. It must build for Linux, Windows and macOS now, and Android and iOS later (no platform-only dependencies in the core).
- Server layout under one Nextcloud folder: per game, the current saves, a small state file (revision, device, time, file list with hashes), and a history folder of dated copies. Each machine records the revision it last synced; a conflict is "server revision changed and local files changed since that revision".
- Pushes upload to a temporary name and rename, so a broken upload never replaces the current saves.
- The game manifest is a small data file shipped by each port.

## Testing decisions

- The sync logic (change detection, conflict decision, retention) is pure and unit-tested.
- Integration tests run the CLI against a local WebDAV server in CI, with no real Nextcloud and no real game.
- Device checks on the three PCs are marked `needs-owner-device`.
