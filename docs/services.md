# Choose and set up a save service

Choose one service when you first set up save-relay on a machine. Set it up once on each machine, and choose the same service on every machine that shares saves. Setup is not repeated for each game.

Use `--service` to name the service. Nextcloud is for a server you or your group already use. Google Drive and Dropbox sign in through a browser. Each has its own narrow access to save-relay's files.

After setup, the password or refresh token stays in the system credential store. The refresh token lets Drive and Dropbox sign in again quietly when needed; it is not a password you need to enter for each game.

## Choosing a service

Pick the service where you want to keep your saves. Use that same choice on every machine. You can also run setup without a service name and answer the question it asks.

```text
save-relay setup --service nextcloud --server https://cloud.example.org --user me
save-relay setup --service gdrive
save-relay setup --service dropbox
```

Nextcloud asks for an address and user name, then an app password. Google Drive and Dropbox open a browser so you can sign in and approve access. Their sign-in uses PKCE and returns to save-relay through a small local web page. The browser page is on this machine; it passes the sign-in result back to the app.

## Nextcloud

Use your Nextcloud address and user name. Create an app password in Nextcloud and give it to save-relay when asked. An app password is separate from your normal account password and can be revoked in Nextcloud.

```text
save-relay setup --service nextcloud --server https://cloud.example.org --user me
```

The password is read privately during setup and saved in the system credential store. Nextcloud stores save-relay's files in its `save-relay` folder.

## Google Drive

Setup opens your browser. Sign in to the Google account you want to use and approve access. Drive access is limited to `drive.file`: save-relay can see files it created, not your other Drive files. Its files are in a normal `save-relay` folder in My Drive, so you can find and rescue them yourself.

```text
save-relay setup --service gdrive
```

The browser returns to a small page on this machine when sign-in is done. The refresh token is kept in the system credential store.

## Dropbox

Setup opens your browser. Sign in to the Dropbox account you want to use and approve access. Dropbox access is limited to the app's App folder, `Apps/<app name>/`; save-relay cannot see your other Dropbox files.

```text
save-relay setup --service dropbox
```

The browser returns to a small page on this machine when sign-in is done. The refresh token is kept in the system credential store.

## Steam's Game Mode

If possible, set up Drive or Dropbox once in Desktop Mode. A browser opened in Game Mode might not appear on screen. After setup, playing and syncing do not need a browser.

If you must set up in Game Mode, first run setup with `--no-browser`. It gives you a sign-in to finish on a phone or another computer. For Google Drive, enter the short code at google.com/device. For Dropbox, open the sign-in page, copy its code, and type it back on the machine running save-relay.

```text
save-relay setup --service gdrive --no-browser
save-relay setup --service gdrive --finish
save-relay setup --service dropbox --no-browser
save-relay setup --service dropbox --finish
```

The `--finish` step completes the sign-in on the machine where setup began. Keep the same service choice on this machine as on your other machines.

## How to run the live checks

The live tests run the real round trip on Dropbox and Google Drive with your own sign-in. They are ignored by default, so a normal test run never touches the services. Run them after `save-relay setup --service <s>` on the machine, pointing each variable at that service's config folder (the `save-relay` folder that holds `config.json`).

```bash
SAVE_RELAY_LIVE_GDRIVE_CONFIG=/path/to/config/save-relay cargo test --test live_services gdrive -- --ignored
SAVE_RELAY_LIVE_DROPBOX_CONFIG=/path/to/config/save-relay cargo test --test live_services dropbox -- --ignored
```

Each test works in its own `save-relay-live-test-<name>` folder on the service and clears it before and after the test, including after an assertion fails. Cleanup failures are reported; rerun once the service is reachable to remove leftovers. Your configured saves root is never used. A variable that is not set skips its tests with a message (add `--nocapture` to see it). Tokens are never printed. Tests serialize sign-ins within a run; do not run separate live-test processes against the same account at once. On Windows, set the variable with `$env:SAVE_RELAY_LIVE_GDRIVE_CONFIG = "..."` before the same `cargo test` line.

The Drive 409 check uses two token-source sessions on this machine. Using an id generated on another physical machine, the consent screens, and Game Mode sign-in remain owner device checks.

## Switching services

Switching services starts fresh. Saves in the old service are not copied to the new one. Set up the new service on every machine. Then push again from the machine that has the newest saves, before syncing from the other machines.

To check which service this machine uses, run `save-relay status` for a game. If Drive or Dropbox reports `reauth_needed`, sign in again with the same setup for that service. If setup reports `sign_in_failed`, start that service's setup again and finish the sign-in. These errors do not mean you should choose a different service.
