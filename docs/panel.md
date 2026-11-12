# The panel

See your games, their save history and any pending choices, in a browser or in Game Mode.

## Open the panel in a browser

`save-relay panel` starts a local server on 127.0.0.1 only (no other computer can reach it) and opens the page in the default browser. `--port` picks the port (1 to 65535; without it any free port is used). `--no-open` prints the address and does not open a browser. The address has a token in it (`?token=`) that changes every run; every request needs it. The browser itself is opened with a one-use code instead, and the page removes the token from the address bar. Close the window or press Ctrl+C to stop.

```text
save-relay panel
save-relay panel --no-open
save-relay panel --port 8765
save-relay panel --port 8765 --no-open
```

## Game Mode

The Decky Loader plugin "Save Relay" shows the same panel in Game Mode. Install it from the release zip with Decky Loader's option to install a plugin from a zip file. The plugin runs its own copy of save-relay from its `bin` folder, or the one on your PATH if it has none. Linux only.

### Controller

Use the controller: the D-pad moves between buttons, A presses the focused button, B goes back.

## What each view shows

- **Games**: every game, where it is open now, which device saved last, and a "Choice waiting" badge when a pending choice is waiting. The buttons at the bottom open **Pending choices** and **Sign in**.
- **A game's history**: each save card, with its picture and place. "Current" marks the saves in use. **Restore** asks "Restore these saves?" first; changes on this machine that were not synced are kept as a backup.
- **Two versions**: pick **Keep server** or **Keep local**.
- **Open elsewhere**: **Wait** for the other machine, or **Play anyway**. Playing anyway can lose the other machine's newest saves.
- **Pending choices**: every choice waiting for an answer.

## Add games

Choose **Add games…** from the Games view to see installed Steam games, their Steam Cloud state, whether save-relay knows a saves folder, and whether a game is already added. The list puts games with no Steam Cloud first. A game with an unknown store answer stays marked **Unknown**; unknown is never treated as no. Add all selects only games with a definite no and a known saves folder, then asks you to confirm the number before it changes their launch options. A known-save game marked Unknown can still be added by its app id. Only Steam app ids are sent to the public store lookup.

In the browser, Add and Remove call `install-wrap` for the selected app ids. Bulk Add confirms the selected eligible ids before calling it. Steam must be closed; if Steam is running, the panel says **Close Steam, then press Add again.**

In Game Mode, the Decky bridge reads the current Launch Options through Steam's app-details callback, asks `save-relay launch-option` for the updated text, then sends it to Steam's client and waits for Steam to echo the requested value. The panel marks the row Added only after that echo. This callback and echo flow has automated tests; checking that Steam accepts it on a handheld is still pending.

## Sign-in for Google Drive or Dropbox

Pick Google Drive or Dropbox in the panel. It shows a QR code and the sign-in address. Open the address on a phone or computer and sign in; for Google Drive, type the code the panel shows.

The panel cannot finish the sign-in. Finish it in a terminal on this machine with the `--finish` flag for your service. For Dropbox, type the code Dropbox showed you when the command asks.

```text
save-relay setup --service gdrive --finish
save-relay setup --service dropbox --finish
```

## Limits

The panel never runs `wrap`, `install-wrap` or `panel`. Everything it does, the command line can do too. It only listens on 127.0.0.1 and answers one request at a time. To set up Nextcloud, use the command line.
