# Spec

**1.0 means:** I finish a chapter of Uncharted on my AMD handheld, then start Uncharted on my Strix Halo tablet or my RTX 3090 Ti desktop, and the game loads that save, with no file copying by hand. The saves travel through the service I picked at setup: my Nextcloud, Google Drive or Dropbox. If I suspend the handheld mid-chapter instead of quitting, the desktop tells me the game is still open there before I play. Any other Steam game with a known save folder does the same with one launch option, and in Game Mode the Save Relay panel shows each game's saves as cards (picture, playtime, where I was) that I can restore with one press.

**Approved by me:** 2026-11-10, in a session's question tool: "List + add, CLI and panel (Recommended)", "Show as unknown, not auto-added (Recommended)", "In 1.0 (Recommended)", "Steam store, cached (Recommended)" (M16 added: Steam games without Steam Cloud). Before that: 2026-11-01, in the Project Manager session's question tool: "Approve (Recommended)". Changed 2026-11-01 (question tool: "2: The Baton (Recommended)", "1: Any game via wrap", "3: Save cards + Decky UI"; "All three in 1.0 (Recommended)"): 1.0 sentence rewritten, M11-M15 added, Won'ts edited. Previous approval: 2026-10-31, re-approved in the Project Manager session's question tool: "Approve (Recommended)" (choice of save service: M6 rewritten, M9 and M10 added). Rulings behind it, question tool, 2026-10-31: Google Drive and Dropbox "In 1.0"; "One service per person, set at setup (Recommended)"; Drive "App's own folder (Recommended)". First approval 2026-10-31: "Approve (Recommended)", recorded in `CHANGELOG.md`. Changed 2026-11-12 by my own ruling, in the Project Manager session's question tool: "Make macOS a Should" (macOS builds in 1.0 moved from the Won'ts to a new Shoulds list). I made this change myself, so the approval above stands.

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
| M6 | THE FIRST RUN on a machine SHALL ask once which service holds my saves (Nextcloud, Google Drive or Dropbox) and sign in to it: for Nextcloud my address and an app password; for Google Drive and Dropbox a sign-in in the browser. It SHALL keep the password or sign-in token in the system's credential store and never write it to a file or a log. Every machine uses the one service I chose. | One setup per machine; no password or token on disk; the service is named in `save-relay status`. | unit test of the store; on each PC, once per service |
| M7 | A GAME SHALL join by adding one small manifest (game id, saves folder per platform, which files count as saves), with no code in save-relay. | A second game syncs with only a manifest. | test with a fixture game |
| M8 | THE CLI SHALL offer `pull`, `push`, `status`, `history`, `restore` and `repair` for a game, each with `--json` for launchers, and uncharted_pc's launcher SHALL call it around Play on Linux and Windows. | Uncharted on the three PCs shares saves (the 1.0 sentence); `repair` releases a stranded WebDAV claim and moves its complete, unpublished entry to `set-aside/` (kept, never deleted). | CLI tests in CI; on the three PCs |
| M9 | WITH ANY OF THE THREE SERVICES, M1-M5 SHALL behave the same, and a push SHALL never replace saves another machine pushed after this machine last read them. | The same tests pass against each service. | integration tests against a WebDAV test server and against fakes of the Google Drive and Dropbox APIs; one real round trip per service on a PC |
| M10 | ON Google Drive and Dropbox, SAVE-RELAY SHALL see only its own folder (Drive: files it created or its app folder; Dropbox: an app folder), never the rest of my files. | The sign-in screen asks only for that narrow access. | the requested scopes in the sign-in request; on each service once |
| M11 | WHEN `pull` runs, SAVE-RELAY SHALL mark the game as open on this machine on the server, and `push` SHALL clear that mark. IF another machine's mark is present at launch, THE LAUNCHER SHALL say which machine and since when, and let me Wait or Play anyway; it SHALL never expire the mark or block on its own. | Suspending the handheld mid-game, then pressing Play on the desktop, shows "open on amd-handheld since 7:12pm". `save-relay where` lists, per game, the machine that has it open. | integration test with two simulated machines; once on a handheld with a suspend |
| M12 | A GAME WITH NO MANIFEST SHALL sync through one Steam launch option, `save-relay wrap -- %command%`: pull, start the game, push when it quits, with the same conflict, offline and open-elsewhere handling as M1-M5 and M11. | Adding the launch option to a Steam game I never wrote a manifest for shares its saves between two PCs. | integration test wrapping a fixture program; on a PC and a handheld |
| M13 | FOR A WRAPPED GAME, SAVE-RELAY SHALL find the saves folder from its Steam app id in the ludusavi-manifest data, cached on the machine, and SHALL stop and tell me when the game has no entry or more than one possible folder, rather than guessing. A manifest of my own SHALL always win. `save-relay install-wrap` SHALL add the launch option to the Steam games I pick. | `save-relay which <appid>` names the folder; an unknown game is refused with a clear message. | unit tests against a fixture of the manifest data; on a PC |
| M14 | EVERY push SHALL carry a save card: playtime of the session, device, time, and when available a picture and a place name ("Chapter 7: The Fortress"). A port supplies the place and picture in a small sidecar file; for a wrapped game the picture is the newest Steam screenshot of that app taken during the session. The conflict prompt and `history --json` SHALL show the cards. | The conflict choice reads as two cards, not two device names. | unit tests of the card; integration test with a fixture sidecar |
| M15 | IN STEAM'S GAME MODE, A SAVE RELAY PANEL in the quick access menu SHALL show each game's cards as a timeline, restore one with one press, show the conflict and open-elsewhere choices, and show a QR code for the Google Drive and Dropbox sign-in. On the desktop, `save-relay panel` SHALL open the same views in the browser. The look follows the design system all my apps share. | On a handheld in Game Mode I restore yesterday's save without a keyboard. | front end tests; on a handheld; the design pass |
| M16 | SAVE-RELAY SHALL list the Steam games installed on this machine and say for each whether it has Steam Cloud, asking Steam's public store once per game (only the app id is sent) and keeping the answer a week. I SHALL be able to add the wrap to the games without Steam Cloud, one by one or all at once, from the command line and from the panel. A game whose answer is unknown SHALL be shown as unknown and never added in bulk. | `save-relay steam-games` shows a game without Steam Cloud as "no"; the panel's Add games view adds it with one press, and its saves reach my other PC. | integration test with a fixture Steam folder and a fake store; on a handheld in Game Mode (`needs-owner-device`) |

## Shoulds

- macOS builds in 1.0: a supported, signed and notarized Mac build of the CLI and the library, with a real pull and push round trip on a Mac. Moved here from the Won'ts by my ruling of 2026-11-12 (see the approval line above). Tracked in #238.

## Won'ts

- Android and iOS bindings: they come with the game ports' 2.0 (Android, iOS with controller support), on this same core.
- Syncing while the game runs; only pull at launch and push at exit (the open-elsewhere mark of M11 is written at launch and cleared at exit, nothing in between).
- Capturing the screen myself: a card's picture comes from the port or from Steam's own screenshots, or there is none.
- A guessed saves folder: a wrapped game with no clear entry is refused (M13).
- Any storage other than my Nextcloud (WebDAV), Google Drive and Dropbox; Steam Cloud.
- Moving saves from one service to another automatically: switching services starts fresh (I can push again from any machine).
- Settings, shader caches or logs: saves only (a save card's small picture and text travel with the save they describe).
- A panel outside Steam's Game Mode other than `save-relay panel` in the browser (no tray app, no desktop window).

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
5. As the owner, I want to be told a game is still open on another machine, so that a suspended handheld never hides my newest save.
6. As the owner, I want any Steam game to sync with one launch option, so that the tool covers my whole library, not only my ports.
7. As the owner, I want to see where I was and how long I played in each save, so that choosing or restoring a save is obvious at a glance.

## Implementation decisions

- Rulings 2026-10-31 (question tool): Nextcloud over WebDAV with a per-device app password; pull at launch, push at exit; ask and keep both on conflict; shared library + CLI; 1.0 = Uncharted on the three PCs; last 10 versions per game; play local and push later when offline; new public repo; own small Rust core, with Ludusavi as the main peer to learn from.
- Storage is one interface with three backends: WebDAV (a create-once `rev/<n+1>` claim guards servers whose etag can repeat before push or restore writes state with If-Match; #149; a crash can strand it and `repair` releases it, moving a complete entry it names to `<game>/set-aside/`), Dropbox (uploads tied to the file revision) and Google Drive (a narrow-scope folder; its weaker overwrite check gets a guard so M9 holds). Google Drive and Dropbox sign in with OAuth in the browser (PKCE, loopback redirect); the app registrations on each service are made under my own accounts.
- Core in Rust: a library crate and a `save-relay` binary. It must build for Linux, Windows and macOS now, and Android and iOS later (no platform-only dependencies in the core).
- Server layout under one Nextcloud folder: per game, the current saves, a small state file (revision, device, time, file list with hashes), and a history folder of dated copies. Each machine records the revision it last synced; a conflict is "server revision changed and local files changed since that revision".
- Pushes upload to a temporary name and rename, so a broken upload never replaces the current saves.
- The game manifest is a small data file shipped by each port.
- Rulings 2026-11-01 (question tool): the Baton, `wrap` and save cards with a Game Mode panel, all in 1.0; the open-elsewhere mark "Never blocks, always asks (Recommended)"; the card picture "Port writes it, wrap grabs Steam's (Recommended)".
- The open-elsewhere mark is a small `playing.json` beside `state.json` (device, time, revision read), written at `pull` and removed at `push`; the plan table gains a row for it before Pull.
- `wrap` reuses the engine unchanged: it is `pull`, a child process, then `push`. The ludusavi-manifest data is MIT, downloaded and cached, never bundled as code.
- The panel is a Decky Loader plugin talking to the same binary's `--json`; `save-relay panel` serves the same views on loopback for the desktop. The design pass is no longer skipped.

## Testing decisions

- The sync logic (change detection, conflict decision, retention) is pure and unit-tested.
- Integration tests run the CLI against a local WebDAV server in CI, with no real Nextcloud and no real game.
- Device checks on the three PCs are marked `needs-owner-device`.
