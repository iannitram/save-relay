# 0001: The current saves are the history entry `state.json` names

**Decided:** 2026-10-31, at Roadmap.

**What.** A game's folder on the server holds `state.json` and `history/<stamp>-<device>/` entries. There is no separate `current/` folder: `state.json`'s `entry` field names the history entry that is current. A push uploads into `incoming/<entry>/`, MOVEs it into `history/<entry>/` when every file is there, then writes `state.json` with If-Match on the etag read at the start.

**Why.**
- Every push is its own dated copy, which is what M4 asks, with one upload instead of two.
- Switching revisions is one small write (`state.json`), so a broken upload never replaces the current saves, and no moment exists where the server has no current saves (a `current/` folder needs two MOVEs, with a gap between them).
- If-Match on `state.json` makes two machines pushing at once safe: one wins, the other gets `changed_on_server` and re-plans, which shows the conflict prompt.
- Each entry carries its own file list (`.save-relay-files.json`, written before the MOVE), so any entry can be restored or checked on its own.
- Restore is a server-side COPY of an entry to a new entry plus a `state.json` write, so history stays in time order.

**Ruled out.** A `current/` folder plus dated copies (twice the upload, or a COPY with a crash window); syncing file by file into one folder (no atomic switch, no history); Nextcloud's own file versions (not exposed the same way on every server, and the retention is the server admin's, not ours).

**Cost.** Someone browsing the Nextcloud folder by hand has to read `state.json` to know which entry is current; `save-relay history` marks it.
