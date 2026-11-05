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

`which <appid>` exits 2 with `save_location_unclear` when there is no single existing folder. `wrap` in that case prints the error and starts the game without sync (the game still starts; nothing is pushed). To fix it, write your own manifest for the game (see manifest.md) and pass it once: `wrap --manifest <file> -- %command%`. save-relay remembers it for that game.