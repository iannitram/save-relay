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
