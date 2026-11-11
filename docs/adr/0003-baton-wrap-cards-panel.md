# 0003: The open-elsewhere mark, `wrap`, save cards and one panel with two shells

**Decided:** 2026-11-01, at Roadmap, after the re-approved Spec (M11-M15 added). My rulings behind it, question tool: all three features in 1.0; the mark "Never blocks, always asks"; the card picture "Port writes it, wrap grabs Steam's".

## The open-elsewhere mark (M11)

- A game's folder on the server gains `playing.json` beside `state.json`: `{"format":1,"device":"amd-handheld","since":"2026-11-01T19:12:00Z","revision_read":7}` (`presence::Mark`, `layout::Layout::playing_path`).
- `pull` writes it with `Precondition::Any`, after its own work, even on `noop`; an offline `pull` writes nothing. `push` removes it only when it names this device, never another machine's mark; a missing file on delete is fine. A `deferred` push leaves it, so the mark stays true while the saves are still only here.
- It **never expires** and never blocks: a mark from a crashed or suspended machine stays until I answer. It is advice, not a lock: the M3 conflict check and the M9 compare-and-swap still guard `state.json`, so a wrong mark can cost a question, never a save.
- The plan table gains one row, checked after `Unreachable` and before everything else, at launch only: another machine's mark and no `--anyway` is `Decision::Held`. The pure part is `presence::check` (`Free`, `Mine`, `Elsewhere { device, since }`) and `plan::decide_with_mark`; `plan::decide` stays as #7 locked it. At exit the mark is ignored.
- `Decision::Held` is `Action::Held` with **exit code 6**, a new code; no existing code changes. The `pull` data gains `held: {device, since}`, `status` gains `held` (a launcher that chose Wait polls `status` until it is `null`), and `pull --anyway` plays past the mark and writes this machine's own.
- `save-relay where` lists, for every game this machine remembers, the machine that has it open and the server's current side.

## `wrap` for any Steam game (M12, M13)

- `save-relay wrap [--game <id>] [--manifest <file>] [--saves-dir <dir>] -- <command>...`, set as a Steam launch option `save-relay wrap -- %command%`. The game id is `--game`, else `steam-<appid>` from the first of `SteamAppId`, `SteamGameId`, `STEAM_COMPAT_APP_ID`. It runs the same engine as the launcher flow: `pull`, then the game as a child process with inherited standard streams, then `push`, and exits with the **child's** exit code. The game always starts, even when `pull` fails: play the local saves, the same rule as M5.
- **Pending choices.** A wrapped game has no launcher to ask, so a `conflict` or `held` at `wrap` time is written to `<data>/save-relay/pending/<game>.json` (the kind, both sides with their cards, the time). `wrap` waits up to 120 seconds for `save-relay answer <game> keep-server|keep-local|play-anyway|wait`, which the panel calls. With no answer the game starts on the local saves and nothing on the server is overwritten; the push at exit meets the same conflict and leaves it pending again. An answer after the wait has ended is applied immediately by `answer` and clears the choice once settled. For a held game, `wait` keeps polling until the original 120-second deadline; it does not restart or extend that timer. While that snooze is active, another `play-anyway` answer is queued for `wrap` to apply; without another answer the game starts on local saves if the other machine is still holding it at the deadline. Never guessed.
- **The saves folder**, in order: a manifest of my own (`--manifest`, or the one remembered for that id) always wins; else the ludusavi-manifest data by Steam app id.
- **The ludusavi data** (`data/manifest.yaml` of mtkennerly/ludusavi-manifest, MIT) is downloaded over `http::Transport` with its ETag, at most once a week, and cached at `<data>/save-relay/ludusavi/` with a small `steam-index.json` (app id to name). An old cache keeps working offline. It is data, read, never bundled as code.
- **Translation** (`ludusavi::translate`, pure over `PathVars` and the Steam facts): the placeholders `<home>`, `<winAppData>`, `<winLocalAppData>`, `<winDocuments>`, `<xdgData>`, `<xdgConfig>`, `<storeUserId>`, `<osUserName>`, `<base>`, `<root>`, `<game>`; the `when: [{os, store}]` filter; only paths tagged `save`. **Proton on Linux:** a Windows-only entry maps into the prefix `<library>/steamapps/compatdata/<appid>/pfx/drive_c/users/steamuser/` (`AppData/Roaming`, `AppData/Local`, `Documents`). No candidate folder on disk, or more than one, is the new error `save_location_unclear` (exit 2): refused with the reason, never guessed. `save-relay which <appid>` prints the folder or the reason.
- **Steam facts** (`steam`, pure plus a little file reading): the Steam folder per OS (Linux `~/.steam/steam`, `~/.local/share/Steam`, Flatpak `~/.var/app/com.valvesoftware.Steam/.local/share/Steam`; Windows `C:\Program Files (x86)\Steam` with no registry; macOS `~/Library/Application Support/Steam`; `SAVE_RELAY_STEAM_DIR` overrides all), a small KeyValues (VDF) reader and writer (`steam::vdf`), `libraryfolders.vdf` to find an app's library, the `userdata/<id>` user ids, and the screenshot thumbnails folder. None found is the new error `steam_not_found` (exit 2).
- **`save-relay install-wrap [<appid>...] [--all-known] [--remove] [--dry-run]`** (`steam::launch`) writes `LaunchOptions` in every `userdata/<id>/config/localconfig.vdf`. Steam must not be running: it refuses with the new error `steam_running` (exit 2) and never stops Steam itself. It writes a dated backup beside the file first, keeps an existing launch option by wrapping it (`save-relay wrap -- <the old option>`, with `%command%` where it was), is idempotent, and `--remove` takes out only its own part. On Windows the binary path is quoted.
- `answer` with nothing pending is the new error `no_pending_choice` (exit 2).

## Save cards (M14)

- `card::Card`: `{format:1, device, time, playtime_secs, place, picture}`. The place is at most 80 characters; the picture is JPEG or PNG by its first bytes and at most 512 KiB, else it is left out with a note in `message`, never an error.
- A card lives **in its history entry**, beside the file list (`.save-relay-files.json`): `.save-relay-card.json` and `.save-relay-picture.jpg` or `.png`. `transfer::put_card` writes them after the MOVE into `history/` and before `state.json`; a crash in between leaves an entry with no card, which reads as `card: null`, never a broken revision. `upload_entry` is unchanged. `download_entry` never writes them into the saves folder. Entries pushed before cards have none.
- A port supplies the place and picture in a sidecar, `.save-relay-meta.json` in the saves folder: `{"place": "...", "picture": "<file in the saves folder>"}`. Every name starting `.save-relay-` is reserved and never part of a save set (`card::is_reserved`; `scan` and `transfer` skip them, F61).
- Playtime: `pull` (and so `wrap`) writes `<data>/save-relay/session/<game>.json` with the start time; `push` sets `playtime_secs` to now minus start, or `null` with no start or over 24 hours.
- For a wrapped game the picture is the newest Steam thumbnail of that app (`userdata/<id>/760/remote/<appid>/screenshots/thumbnails/`) whose time falls inside the session (`steam::pick_screenshot`, pure over names and times). save-relay never captures the screen.
- API: `HistoryEntry` gains `card`; the `pull` and `push` data gain `server_card` and `local_card` beside the sides; `save-relay picture <game> <entry> --out <file>` copies a picture out for the panel.

## The panel (M15): one front end, two shells

- **One front end**, `panel/` (TypeScript, React 18, Vite, vitest; a pnpm workspace with `decky/`), holds every view: games, a per-game timeline of cards, restore with one press and a confirm step, a conflict as two cards side by side, held ("open on amd-handheld since 7:12pm" with Wait and Play anyway), pending choices, and sign-in with a QR code of `SignInPrompt.url` plus the code (`qrcode`, MIT). Controller first: everything reachable with the D-pad, A and B, no hover-only actions, visible focus, text 16 px or larger at 1280x800.
- The views talk to one `Bridge` interface and get back the same `--json` envelopes as the command line. The TypeScript types are written by hand to mirror `src/api.rs`, and a contract test checks them against JSON fixtures that `cargo test` writes.
- **Game Mode shell**, `decky/`: a Decky Loader plugin in the quick access menu (`plugin.json`, a `main.py` backend that runs the `save-relay` binary with `--json`, a front end on `@decky/ui` and `@decky/api` that renders the shared views). Decky supplies Steam's own React at run time. Linux Game Mode only.
- **Desktop shell**, `save-relay panel [--port <n>] [--no-open]` (the `panel` Rust module): a tiny HTTP/1.1 server on `127.0.0.1` over `std::net`, no new dependency. It serves the built front end embedded in the binary and `POST /api/<command>`, which runs the same code as the command line. A random token is in the URL it opens and is checked on every request, so other pages in the browser cannot call it. It opens the browser like `oauth::open_browser`. Linux, Windows and macOS.
- **Embedding without a build script.** The built front end (`panel/dist`, fixed names `index.html`, `app.js`, `app.css`) is embedded with `include_bytes!` only under the Cargo feature `panel-dist`; without it the binary serves a checked-in placeholder page. A plain `cargo build` needs neither Node nor a C toolchain for a build script; CI's front-end job and the release build turn the feature on.
- The design pass (ROADMAP stage 8) is **no longer skipped**: the panel is checked against the design system all my apps share, and its findings become `design-pass` issues. The release (#24) also attaches the Decky plugin zip.

## YAML reader: serde-saphyr

The ludusavi data is one YAML file of about 17 MB. **serde-saphyr** (2.0, MIT OR Apache-2.0, pure Rust on the saphyr parser, released 2026-10, maintained) is a serde deserializer, so the file reads straight into typed structs and every field save-relay does not use is skipped without building a tree. Its default size budgets may need raising for a file this big; its issue sets them. It is added to `Cargo.toml` by that issue, not by the skeleton.

One licence comes with it that is not plain MIT OR Apache-2.0: serde-saphyr's `deserialize` feature always pulls in `encoding_rs_io`, which pulls in **encoding_rs** (0.8, `(Apache-2.0 OR MIT) AND BSD-3-Clause`). The BSD-3-Clause part covers only the WHATWG encoding data tables (`LICENSE-WHATWG`). It cannot be turned off, and I accepted it on 2026-11-03.

## Why

- The mark answers the one case M3 cannot see: a machine that has not pushed yet because the game is still open (a suspended handheld). Writing it at `pull` and clearing it at `push` needs no clock and no lease, and asking instead of expiring keeps every decision mine.
- `wrap` is the engine unchanged (pull, child, push), so M1-M5, M9 and M11 hold for wrapped games by construction, and one launch option covers my whole Steam library.
- Pending choices keep "never guess" true for a game nobody launched through a launcher, without blocking the game for long.
- Cards inside the history entry travel, restore and get pruned with the saves they describe, on every service, with no new server file to keep in step.
- One front end means one set of views to test and to design; the two shells are thin.

## Ruled out

- **A mark that expires, or a lock that blocks**: against my ruling. A suspended handheld can be days old and still hold the newest progress.
- **The mark inside `state.json`**: every `pull` would then write `state.json` and contend with pushes for its compare-and-swap.
- **Bundling the ludusavi data, or depending on ludusavi as a program**: the data changes weekly, and the program is a whole app (the peer note in ROADMAP).
- **Guessing among several candidate folders** (newest, biggest): a Won't (M13).
- **Capturing the screen** for a card's picture: a Won't; the port or Steam supplies it.
- **Card fields on `Side` and `SyncData`**: #9's locked tests build those structs whole, so new fields would break them; the cards ride beside them in `api::SyncReport`.
- **YAML crates:** `serde_yaml` (deprecated by its author); `serde_yaml_ng` (no release since 2024-05); `serde_yml` (soundness and advisory concerns); `serde_norway` (no release since 2024-12); `yaml-rust2` (maintained and pure Rust, but it builds a whole tree, which is heavy for 17 MB; kept as the fallback).
- **A desktop window or tray app**: a Won't; the browser is the desktop shell.
- **A web framework for `save-relay panel`** (axum, hyper, tiny_http): a few routes on loopback do not need one, and each pulls in a large tree.
- **A Cargo build script that embeds `panel/dist`**: it would fail where Node did not run first, and it needs a linker for itself; the `panel-dist` feature does the same with plain `include_bytes!`.

## Cost

- One more small server file per game (`playing.json`): a write at every `pull` and a delete at every `push`.
- Pending choices make a conflict on a wrapped game wait for me in the panel instead of in a launcher; with no panel open, the game starts on local saves after 120 seconds and the choice waits for later.
- `install-wrap` edits a Steam file Steam owns. The backup, the Steam-running refusal and `--remove` limit the harm; a Steam update that changes `localconfig.vdf` would need a fix.
- A second language (TypeScript) and a Python file for Decky, with their own CI job.
