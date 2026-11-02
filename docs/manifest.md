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
- Documents: `~/Documents`

### macOS
- Home: `$HOME`
- Data and config: `~/Library/Application Support`
- Documents: `~/Documents`

Any other `{...}` placeholder is an error.

## --saves-dir Override

A launcher that lets the player move the saves folder passes `--saves-dir <folder>`.
It wins over the template.
A `saves_from` game requires it.

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