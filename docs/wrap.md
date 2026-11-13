# Wrap a Steam game

Sync a game's saves every time Steam starts it.

## wrap

`wrap` pulls the saves from the server, runs the game as a child process with this terminal's input and output, and pushes after the game ends. The game's own exit code is the exit code. The game always starts. A conflict or held game becomes a pending choice; with no answer in 120 seconds the game starts on this machine's saves, and nothing on the server is overwritten.

```text
save-relay wrap -- %command%
```

Put that line in the game's Launch Options in Steam, or let `install-wrap` do it for you. `install-wrap` writes the full path to the program in place of `save-relay`. Options for `wrap` (`--game`, `--manifest`, `--saves-dir`) go before the `--`.

## install-wrap

`install-wrap` adds the wrap to the Launch Options of the games you name by Steam app id, or of every installed game save-relay knows with `--all-known`. `--dry-run` shows what would change and writes nothing. `--remove` takes the wrap back out.

Steam must be closed: save-relay refuses to run while Steam is open, and never closes Steam for you. Before the first change to each file it writes a backup beside it, named `localconfig.vdf.save-relay-<stamp>.bak`. Only one `install-wrap` runs at a time.

```text
save-relay install-wrap --dry-run 1234560
save-relay install-wrap 1234560
save-relay install-wrap --all-known --dry-run
save-relay install-wrap --remove 1234560
```

## which

`which` shows the saves folder save-relay will use for a game, and where it came from: your own manifest, or the ludusavi data. It works offline if the ludusavi data was fetched before. With no copy and no network it fails with `server_unreachable` (exit 5).

```text
save-relay which 1234560
```

## Games without Steam Cloud

`steam-games` lists installed Steam games, whether the public store says they use Steam Cloud, whether save-relay knows their saves folder, and whether they are already wrapped. Steam receives only the app ids needed for the store lookup. `unknown` means the store did not give a yes-or-no game answer; it is never treated as `no`.

`--no-steam-cloud` shows only games with a known `no` answer. `install-wrap --no-steam-cloud` wraps only those games whose saves folder is known. Unknown games and games without a known saves folder are reported as skipped. The command needs Steam closed, as described above, and does not wrap anything when the store answer is unknown.

```text
save-relay steam-games
save-relay steam-games --no-steam-cloud
save-relay install-wrap --no-steam-cloud --dry-run
save-relay install-wrap --no-steam-cloud
```

## Your own manifest

A manifest you wrote (`--manifest`) always wins over the ludusavi data. `--manifest` is remembered for that game id. `--saves-dir` only applies to ludusavi-found games or to a game without a manifest.

## Proton

On Linux, saves under the Proton prefix `steamapps/compatdata/<appid>/pfx` are read with the Windows folders mapped (`<winAppData>`, `<winDocuments>` and so on). The user is `steamuser`. Saves a game keeps in the Windows registry are not covered.

## Pending choices and answer

When the server has saves this machine does not, or another machine still has the game open, `wrap` stops and asks before the game starts. That is a pending choice. Answer it from another terminal or from the panel:

- two versions: `keep-server` uses the server's saves, `keep-local` keeps this machine's saves.
- open elsewhere: `wait` waits for the other machine, `play-anyway` starts now (you can lose the other machine's newest saves).

With no answer in 120 seconds the game starts on this machine's saves, and nothing on the server is overwritten. `answer` with no pending choice exits 2.

```text
save-relay answer steam-1234560 keep-server
save-relay answer steam-1234560 play-anyway
```

## When the saves folder is unclear

`which <appid>` exits 2 with `save_location_unclear` when there is no single folder to use. `wrap` in that case prints the error and starts the game without sync (the game still starts; nothing is pushed). To fix it, write your own manifest for the game (see manifest.md) and pass it once: `wrap --manifest <file> -- %command%`. save-relay remembers it for that game.

A save path that cannot be resolved on this machine counts toward "exactly one" too: an unknown placeholder, a `<storeUserId>` with more than one Steam user, or a `<base>` with no appmanifest. With one resolvable folder and such a path beside it, the location is unclear, not that folder.

## A machine that has never run the game

The ludusavi data names the game's saves folder even where the game has never run. When a save path with a glob is the only one named (Hades on Linux: `~/.local/share/Hades/*.sav`), save-relay uses its folder without requiring it to exist. The first `wrap` (or `pull`) on that machine downloads the server's saves into the folder, creating it, and that first launch is not a conflict. A folder that did not exist when it was located is not remembered for the game: the next launch locates it again.

A save path with no glob that is not on disk yet (Hades' `Saved Games/Hades` on a machine that has not run it) stays `save_location_unclear` until the game has run once: it could be a folder or a file, and save-relay does not guess from the name.

Whether a Windows game runs under Proton comes from the launch (Steam sets `STEAM_COMPAT_DATA_PATH`), not from the prefix folder being on disk. If a Proton prefix folder and a native saves folder are both on disk, the location is unclear and the person says which with their own manifest; a leftover prefix never wins.

## When Steam force-closes the game (Linux, macOS)

On SIGTERM or SIGHUP (Steam's force exit, a logout, a shutdown) `wrap` stops waiting, passes the signal on to the game, runs the exit push and clears this device's `playing.json` mark, then exits with 128 plus the signal number. A second signal while it does this ends `wrap` at once and leaves the mark, so the next launch handles it as an interrupted session.

What is not covered yet: `wrap` does not wait for a game that ignores the signal it was sent, so the exit push can run while that game is still writing (a half-written set can be pushed); a signal during the pull or the choice wait before the game starts is not handled; and on Windows the console close, logoff and shutdown events are not handled (#264 tracks the Windows half and a bounded wait).
