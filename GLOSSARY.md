# Glossary

The project's own words, one line each. `/grill-with-docs` adds to this during the interview; code, issues and specs use these words.

| Term | Means | Not to be confused with |
|---|---|---|
| save set | All of one game's save files on one machine, synced as a unit | a single save slot |
| revision | The server's version of a save set, recorded in its state file | a git commit |
| manifest | A port's small data file naming its game id and saves folder per platform | Ludusavi's manifest |
| service | Where the saves are kept, chosen once at setup and the same on every machine: Nextcloud, Google Drive or Dropbox | a system service or daemon |
| backend | The code that speaks one service's API behind the `Remote` trait (`webdav`, `dropbox`, `gdrive`) | a server |
| refresh token | The long-lived sign-in Google Drive or Dropbox gives at setup, kept only in the credential store | an access token (short-lived, memory only) |
| claim | On Google Drive, a small file created with a pre-chosen id so that only one machine may write `state.json` next (`gdrive::guard`) | a lock that someone must release |
| rev marker | On WebDAV, the create-once file `<game>/rev/<n>` a push or restore creates before it writes `state.json`, so only one writer may publish revision n when etags can repeat; `save-relay repair <game>` releases a stranded next marker; a complete entry it names is moved to `<game>/set-aside/` first, never deleted (`layout::rev_path`, `layout::set_aside_path`, `sync::claim_revision`, `sync::repair`, #149/#159) | the Drive claim file (`claim`), and the open-elsewhere mark |
| open-elsewhere mark (mark) | The small `playing.json` on the server naming the machine whose `pull` left a game open; `push` clears it (`presence`) | a lock: it never blocks and never expires, it only makes the launcher ask |
| held | The answer at launch when another machine's mark is present: nothing is touched and the player picks Wait or Play anyway (`Action::Held`, exit 6) | a conflict (both sides changed the saves) |
| wrap | Syncing a game through its Steam launch option, `save-relay wrap -- %command%`: pull, the game as a child process, push | `install-wrap`, which only writes that launch option |
| pending choice | A conflict or held answer waiting on this machine for `save-relay answer`, because a wrapped game has no launcher to ask (`pending`) | a pending upload or a deferred push |
| save card | The small record a push writes into its history entry: device, time, playtime, and when known a place name and a picture (`card`) | the save files themselves |
| sidecar | A port's `.save-relay-meta.json` in the saves folder giving the place name and picture for the next card; never part of the save set | the manifest |
| panel | The Save Relay views: in Game Mode as the Decky plugin, on the desktop as `save-relay panel` in the browser; one front end (`panel/`) | the `panel` Rust module alone, which is only the desktop server |
| ludusavi data | The ludusavi-manifest project's shared list of where PC games keep saves, downloaded and cached to find a wrapped game's saves folder (`ludusavi`) | a manifest (ours, which always wins) |
| Steam Cloud state | Whether Steam itself syncs a game's saves: yes, no or unknown, from Steam's public store and kept a week (`steam::cloud`) | the ludusavi data's `cloud.steam` flag, which only ever says yes |
