# 0002: One storage interface, three backends, chosen once at setup

**Decided:** 2026-10-31, at the re-approved Spec (M6 rewritten, M9 and M10 added).

## What

The `Remote` trait (`src/remote.rs`: list, get, put with a `Precondition`, mkcol, copy, move_to, delete) is the storage interface. The sync engine, `transfer` and the layout of ADR 0001 stay as they are and never learn which service they talk to. Three backends implement it:

| Backend | Module | Sign-in | Its guard for `put(state.json, Match(etag))` | etag |
|---|---|---|---|---|
| Nextcloud (WebDAV) | `webdav::WebDav` | app password, HTTP Basic | create `rev/<n+1>` with `If-None-Match: *`, then `If-Match` on state (412 is `Changed`; #149) | the server's ETag |
| Dropbox (HTTP API v2) | `dropbox::Dropbox` | OAuth, App folder | `files/upload` with `mode: {".tag": "update", "update": <rev>}` and `strict_conflict: true`; `mode: add` for `Absent` (409 `path/conflict` is `Changed`) | the file's `rev` |
| Google Drive (REST v3) | `gdrive::Drive` | OAuth, `drive.file` | the claim guard below (`gdrive::guard`) | `v<version>.<claim id>` |

All three send HTTP through one seam, `http::Transport`. The real transport (`http::UreqTransport`, ureq with rustls) is the only code behind a feature (`http`); the backends are plain Rust and always built. So CI tests them against in-memory fakes of the Dropbox and Drive APIs (`fake::FakeDropbox`, `fake::FakeDrive`), the M9 suite runs the same tests against every backend, and a mobile build (2.0) brings its own transport. The core keeps no platform-only dependency.

The service is chosen once per machine at setup (`save-relay setup --service nextcloud|gdrive|dropbox`, or `setup` with no flag asks) and is the same on every machine (M6). It is recorded in `config.json`'s `server` field: the Nextcloud address (`https://...`), or the service key `gdrive` or `dropbox` (`service::Service::from_server`). Keeping format 1's shape means no config migration and no change to the issued locked tests. `status` and `setup` name the service in their `--json` (`service`).

## WebDAV revision markers (#149)

A WebDAV server's ETag can repeat for same-second, same-size writes (an ETag built from the
modification time in whole seconds and the size). Nextcloud 33 does not do this: every write gets
a fresh ETag, so there `If-Match` alone already refuses a stale push. The claim guards any WebDAV
server whose ETag can repeat, and costs one small file per revision on Nextcloud. Before either push or restore
writes revision `n+1` to `state.json`, the engine creates `<game>/rev/<n+1>` with
`Precondition::Absent` (`If-None-Match: *`); its body is the new history entry's name. A refused
create is `Error::Changed` (exit 3), and the losing entry is removed. The state write still uses
its original precondition. A reported state-write failure attempts to delete its marker; the
complete history entry is retained unless the write was explicitly refused. Successful push
and restore prune markers numerically, keeping the newest 10 through their published revision,
never deleting that revision's marker or later claims, and ignoring odd names. Drive, Dropbox
and MemoryRemote create no `rev/` paths.

`save-relay repair <game>` lists the claim files above the current revision `n`. It acts only on
`rev/<n+1>`, and only after `state.json` is confirmed unchanged. Two cases:

- The entry the claim names is missing or incomplete: nothing was published under it, so the claim is released.
- The entry is complete. This is the usual crash (the push moves its entry into `history/` before it claims the revision): the entry is moved to `<game>/set-aside/rev<n+1>-<entry>`, never deleted, and then the claim is released. `state.json` is checked again after the move; if it moved, the entry goes back to `history/` and nothing is released.

Claims above `n+1`, and claims whose body is not an entry name, are kept. Text mode asks before any change, and `--yes` approves. `--json` without `--yes` only reports; each marker carries `status` and, for a set-aside entry, `set_aside`. A server change during repair returns `changed_on_server` (exit 5). Repair is available for Nextcloud; Drive, Dropbox and MemoryRemote report "nothing to repair." Repair assumes no machine is pushing to that game: a writer that is still alive and has not written `state.json` can publish after its claim is released, and repair cannot stop a write that lands after its last check. There is no timer-based expiry: a claim is not removed merely because it is old. The rules are covered by `sync::repair_tests` and by `tests/repair.rs` (a push that dies at its state write, then repair, then a push; and the CLI prompt, `--json` and `--yes`), #159.

## Dropbox

- A **scoped app with App folder access**: everything lives in `Apps/<app name>/` and the token reaches nothing else (M10). Paths map one to one (`"a/b"` is `"/a/b"`, `""` the App folder). Scopes asked in the sign-in: `account_info.read files.metadata.read files.metadata.write files.content.read files.content.write`.
- `Dropbox-API-Arg` headers must be ASCII, so non-ASCII path characters are written as `\uXXXX` (`dropbox::header_json`).
- Dropbox creates missing parent folders on its own; the backend checks the parent first (cached), so the `Remote` contract's `NotFound(parent)` holds the same as on WebDAV and `MemoryRemote`.
- `move_v2` and `copy_v2` work on whole folders server-side, so ADR 0001's "incoming, then MOVE into history" is as atomic as on Nextcloud.

## Google Drive

- **Scope `drive.file`**: the app sees only files it created (M10). Its folder is a normal `save-relay` folder in My Drive that the person can open and rescue saves from by hand, as on Nextcloud.
- Drive is id-based and allows two files with one name. A path resolves one segment at a time from `root`; when names repeat, **the oldest wins** (`createdTime`, then id: `gdrive::pick`) on every machine, so two machines that both created `uncharted-1/` at once still agree on one.
- `move_to` is one metadata update (new parent and name), so an entry appears in `history/` whole. Drive copies files only, not folders: `copy` of a folder is file by file and not atomic, so callers copy into `incoming/` and then `move_to` (restore, #21).
- `put` with `Absent` (the first `state.json` of a game) creates the file, lists the name, and keeps only the oldest; a machine whose copy is not the oldest deletes it and gets `Changed`.

### The M9 guard on Drive

Drive v3 has no conditional upload (no `If-Match`), so "a push never replaces saves another machine pushed after this machine last read them" (M9) needs its own one-winner step. Drive gives one: `files.create` with a client-chosen id (from `files.generateIds`) is refused with 409 when that id is already in use. So every file the backend writes carries a fresh **claim id** in its `appProperties`, and a conditional write of a file read at version `V` with claim `C`:

1. reads the file's metadata; a version other than `V` is `Changed`;
2. **claims**: creates `.claim-<C>` with id `C`, holding a fresh `next` id. Only one machine can; the others get 409;
3. reads the version again and asks the pure decision table `gdrive::guard::decide`:
   - version moved: `Changed`;
   - won, held under 60 s: write; held 60 s or more (the machine stalled): give up as unreachable, so the push is retried at the next launch or exit;
   - claim taken, under 120 s old (Drive's clock): `Changed` (another machine is writing; sync re-plans and M3 asks);
   - claim taken, older than 120 s, file unchanged (the claimer died mid-push): **break the lease** by claiming that claim's `next` id, again one winner, at most 3 hops;
4. uploads the new content with a fresh claim id;
5. reads the metadata again: version and claim must be the ones just written (`check_written`), else `Server("state changed during the write")`.

Claim files are kept for a day, so an id is never free again while a slow machine may still hold an etag that names it.

**The risk that remains, stated plainly.**

- The guard rests on Drive refusing a second create with the same id. Drive documents the 409; whether an id generated on one machine can be used by another (same account, same app) is checked first in the real round trip on a PC (`needs-owner-device`). If either fails, the fallback is a named lock file where the oldest `createdTime` wins after a re-list, and this ADR is amended.
- A machine that stalls for over two minutes **inside** its single state upload request (a handheld suspended mid-request), while another machine breaks its lease and pushes the same game, can have one of the two revisions replaced by the other. The replaced push's files stay in `history/` (nothing is deleted, `restore` brings them back), but the conflict prompt may be skipped once. Step 5 catches it whenever the other write lands first. Nextcloud and Dropbox do not have this hole.
- Drive's lists can lag a write by moments; the oldest-wins rule and the claim make that safe for `state.json`, but a just-created folder may be created twice (harmless: the oldest wins and the other stays empty).
- Edits made by hand in the Drive web page bypass the guard, as edits in the Nextcloud web page bypass save-relay's own records.

## Sign-in for Google Drive and Dropbox (OAuth)

- **OAuth 2.0 for a desktop app with PKCE** (`S256`) and a **loopback redirect** on `127.0.0.1`: Google desktop clients accept any port; Dropbox needs the exact registered URI (`clients::DROPBOX_REDIRECT`, port 53710). The narrow scope is in every sign-in request, so M10's check reads it there. `oauth` holds the pure parts (PKCE, URLs, the redirect, token answers, expiry) apart from the flow.
- **Tokens**: the refresh token goes to the credential store through the existing `CredentialStore` (`KeyringStore`), filed under `service::token_account` (`"gdrive:<e-mail>"`); access tokens stay in memory (`oauth::TokenSource`), refreshed a minute before expiry and once after a 401. A refused refresh (`invalid_grant`) is the new error `reauth_needed` (exit 4): the launcher offers to sign in again. A sign-in that does not finish is `sign_in_failed` (exit 4).
- **Client ids are shipped in the build** (`src/clients.rs`): a desktop app cannot keep a secret, which is why PKCE exists. Google issues a "client secret" for desktop clients too, but documents that it is not treated as a secret for installed apps; it is shipped beside the id. Dropbox's PKCE flow needs the app key only. The registrations are made under the owner's own accounts (the `needs-owner-device` issue "Register the save-relay app with Google and Dropbox"); agents never sign in.

### Steam's Game Mode

A launcher started from SteamOS Game Mode runs inside gamescope with no desktop session, so a browser opened with `xdg-open` may never appear on screen. Setup is once per machine (M6) and pull/push afterwards need no browser (the refresh token works silently), so:

1. **Recommended:** run setup once in Desktop Mode (a terminal, or the launcher there).
2. **In Game Mode** (`XDG_CURRENT_DESKTOP=gamescope` or `SteamGamepadUI` set, `oauth::in_game_mode`), setup defaults to the code flows, in two steps so a launcher can draw them: `setup --service <s> --no-browser --json` returns a `SignInPrompt` (a URL and, for Google, a short code); the person opens it on a phone; `setup --service <s> --finish --json` completes it. **Google** uses its device flow (a code like `ABCD-EFGH` typed at google.com/device; needs a second Google client of type "TVs and Limited Input devices", and `drive.file` is on Google's list of scopes that flow allows). **Dropbox** has no device flow: its code flow shows a long code that has to be typed back on the handheld (Steam's on-screen keyboard), which is clumsy; the launcher may instead open the URL in Steam's own browser (`steam://openurl/<url>`), whose loopback redirect reaches the same machine.
3. The pending sign-in (PKCE verifier or device code) waits in the credential store between the two steps, never in a file.

**Game Mode risks:** whether the Secret Service is unlocked from Game Mode at all (already open in #18: no store means no token, whatever the service); Google refuses sign-in inside embedded browsers, so Steam's browser is not an option for Google, only the device flow; Google apps left in "Testing" get refresh tokens that expire after 7 days, which would show as `reauth_needed` every week. The owner's registration issue publishes the app instead (`drive.file` alone needs no Google review).

## Why

- One interface means M1-M5 behave the same on every service by construction (M9), and one test suite proves it.
- Each service's own compare-and-swap is used where it exists (Dropbox rev); WebDAV supplements `If-Match` with a create-once revision marker, and Drive uses its claim guard with a pure, separately tested decision table.
- The transport seam keeps the network behind one feature and lets the Drive and Dropbox backends be tested in CI with no account, which agents cannot have.

## Ruled out

- **Drive `appDataFolder`** (hidden app data): the person could not see or rescue saves by hand, and the folder disappears when the app's access is removed. The owner's ruling was "App's own folder" with narrow access; `drive.file` gives both.
- **Full-Drive or full-Dropbox scopes**: against M10.
- **rclone as the backend for all three**: a second program to ship and configure on every machine, and its overwrite semantics are not the compare-and-swap M9 needs.
- **Drive's revision list as the guard** (upload, then check that the revision before ours is the one we read): it detects a lost race only after the overwrite, and Drive may merge or prune revisions.
- **One shared OAuth flow in a web service**: there is no server of ours, and a desktop app with PKCE needs none.
- **Moving saves between services**: a Won't; switching services starts fresh.

## Cost

- Three backends to keep in step, and two fakes that must follow the real APIs; the real round trips (`needs-owner-device`) are the check on the fakes.
- Drive pushes make about five extra small requests for the guard.
- The config's `server` field doubles as the service name, which a person reading `config.json` has to know (this ADR, `docs/code-map.md`).
