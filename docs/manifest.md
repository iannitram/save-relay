# Manifest Format

A manifest is a small JSON file that each game port ships so a game joins with no code changes in save-relay.
There is one manifest per game, because each game has its own revision, history and conflicts.

## Fields

| Field | Description |
|-------|-------------|
| `format` | Always 1 |
| `id` | The game id: 1-64 characters of `a-z`, `0-9`, `-`, starting with a letter or digit. It names the game's folder on the server |
| `name` | A display name, not empty |
| `saves` | The saves folder template per platform; at least one, unless `saves_from` is set |
| `saves_from` | The only value is `"launcher"`: the launcher always passes `--saves-dir`, so `saves` may be `{}` |
| `include` | Glob patterns of the files that count as saves, relative to the saves folder; at least one |
| `exclude` | Patterns left out even when included |

## Patterns

Glob patterns use these rules:
- `*` matches any characters inside one folder level
- `?` matches one character
- `**` matches any number of folder levels
- Patterns are relative to the saves folder, using `/` only
- Patterns may not start with `/`, contain `\`, or contain a `..` segment

## Machines with different patterns (S12)

The patterns are per machine: each machine reads its own manifest, and the server keeps only the
file list of the current revision. When two machines' `include` or `exclude` differ, a file one
machine does not scan can still be in the revision another machine pushed:

- A pull writes every file of the current revision to the saves folder, whatever this machine's
  patterns say, so the file is on disk here.
- Each sync then carries such a file into this machine's local set when it is still on disk with
  the revision's hash. A push keeps it, and a launch does not read its absence as a change.
- If such a file is missing from the saves folder (deleted by hand, or never pulled), the push
  does not go ahead: it fails with a message naming the files, and the server is left alone. Pull
  again, or make the two manifests agree.

Keep the `include` and `exclude` lists the same on every machine that shares a game, so no file
is ever left out of a push.

## Placeholders

The following placeholders are supported:
- `{home}` - expands to the home directory
- `{data}` - expands to the data directory
- `{config}` - expands to the config directory
- `{documents}` - expands to the documents directory

### Linux
- Home: `$HOME`
- Data: `$XDG_DATA_HOME`, else `~/.local/share`
- Config: `$XDG_CONFIG_HOME`, else `~/.config`
- Documents: `~/Documents`

### Windows
- Home: `%USERPROFILE%`
- Data: `%LOCALAPPDATA%`
- Config: `%APPDATA%`
- Documents: `%USERPROFILE%\Documents`

### macOS
- Home: `$HOME`
- Data and config: `~/Library/Application Support`
- Documents: `~/Documents`

Any other `{...}` placeholder is an error.

## --saves-dir Override

A launcher that lets the player move the saves folder passes `--saves-dir <folder>`.
It wins over the template.
A `saves_from` game requires it.

### Ports with save slots

Pass the selected slot's folder as `--saves-dir <slot-folder>` and pass `--slot <n>` (1 through 10)
on every save-relay command for that game. Each slot then has an independent server id such as
`<game>-slot-3`, with its own current save and history. The slot folder is also recorded with the
local last-sync state; if the launcher changes folders under one id, save-relay treats the old
record as absent and uses the first-sync rules rather than comparing saves across folders.

## Sidecar

The sidecar file is `.save-relay-meta.json`, in the saves folder. It is read at the start of each sync. If it is missing, nothing happens.

Fields are `place` (text) and `picture` (path relative to the saves folder). Unknown fields are ignored.

The `place` field has whitespace (including new lines) converted to one space, trimmed, and cut to 80 characters. An empty value means no place.

The `picture` field must be a JPEG or PNG file (checked by the file's first bytes, not its name). It must be at most 512 KiB; larger files are left out with a note. The path must be inside the saves folder: no leading `/`, no `\`, no `:`, no `..`.

The picture is copied into the save card in the history. It never stays in the saves folder.

Files named `.save-relay-*` are never treated as saves: scan and transfer skip them. Use `.save-relay-` as the prefix for the picture file so it is never synced as a save. Example file: `.save-relay-cover.jpg`.

A bad sidecar never fails a push: bad JSON or a bad picture path is left out and shows as a note in the message the push prints. A picture that cannot be read is left out with a note too.

```jsonc
{
  "place": "Example place",
  "picture": ".save-relay-cover.jpg"
}
```

## Examples

```json
{
  "format": 1,
  "id": "uncharted-1",
  "name": "Uncharted: Drake's Fortune",
  "saves": { "linux": "{data}/uncharted-pc/user", "windows": "{data}/uncharted-pc/user" },
  "include": ["Uncharted 1/SAVEDATA*/**"],
  "exclude": ["**/*.tmp"]
}
```

```json
{
  "format": 1,
  "id": "second-game",
  "name": "Second Game",
  "saves": { 
    "linux": "{documents}/second-game",
    "windows": "{documents}/second-game", 
    "mac": "{documents}/second-game"
  },
  "include": ["**"]
}
```

```json
{
  "format": 1,
  "id": "launcher-game",
  "name": "Launcher Game",
  "saves": {},
  "saves_from": "launcher",
  "include": ["**"]
}
```
