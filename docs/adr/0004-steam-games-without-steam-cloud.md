# 0004: Steam games without Steam Cloud

**Decided:** 2026-11-10, with M16. My rulings behind it, question tool: "List + add, CLI and panel", "Show as unknown, not auto-added", "In 1.0", "Steam store, cached".

## What was decided

- **Where the yes or no comes from: Steam's public store.** `GET https://store.steampowered.com/api/appdetails?appids=<id>&filters=basic,categories`, no sign-in. A game whose `categories` hold id 23 has Steam Cloud; a game without it does not; an answer of another `type` (tool, dlc, demo) is not a game; `success: false` is unknown. Checked against the real store on 2026-11-10.
- **Only the app id leaves the machine**, one request per installed game, at most 100 per run. Answers are kept a week in `<data>/save-relay/steam/cloud.json` (`steam::cloud::Cache`). A 429 or no network stops the run and keeps what it has: it is never an error.
- **Unknown stays unknown.** A game with no stored answer is listed as unknown and is never wrapped by `install-wrap --no-steam-cloud` or by the panel's "Add all". I can still add it by app id. This is the spec's no-guessing rule (M13) applied to Steam Cloud.
- **`save-relay steam-games`** lists the installed games (store type `game`, or unknown and in the ludusavi index) with `steam_cloud`, `saves` (`manifest`, `ludusavi`, `none`) and `wrapped`. **`install-wrap --no-steam-cloud`** wraps those with `steam_cloud: no` and a known saves folder, and reports the rest in `skipped`.
- **Adding from Game Mode.** `install-wrap` edits `localconfig.vdf` and refuses while Steam runs (ADR 0003), and in Game Mode Steam always runs. So the Decky shell sets the launch option through Steam's own client; the text comes from `save-relay launch-option <appid> --current <text>`, which uses the same `steam::launch::wrapped` and touches no file. The browser shell calls `install-wrap` and says "Close Steam, then press Add again" on `steam_running`.

## Why

- The ludusavi data cannot answer it: of 53,520 games it marks 14,649 `cloud.steam: true` and none `false`, so "not marked" means "not known", for about 70% of Steam games.
- A game with Steam Cloud that is also wrapped would sync twice, and the two could disagree; so the bulk add must be sure of a "no".

## Ruled out

- **Steam's local `appinfo.vdf`** (the cloud quota): offline and sends nothing, but the file is binary and its format changes between Steam versions.
- **The ludusavi flag alone**, treating "not marked" as no Steam Cloud: would wrap games that do have it.
- **Wrapping by itself at setup**: I pick what is added.

## Risk

- Steam's client call for launch options in Game Mode is not documented. F77 checks it on a handheld first; if it does not hold, the Game Mode view only lists and says to add from Desktop Mode with Steam closed.
