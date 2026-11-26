# 3DS and Azahar: proposed transport design

**Status: design only, not implemented, not hardware-verified.**

This is a design for [#300](https://github.com/iannitram/save-relay/issues/300), not a shipped feature. Implementation is tracked by [#319](https://github.com/iannitram/save-relay/issues/319). No real 3DS, emulator, TLS, or account validation has been performed for this proposal.

## Route and trust boundary

The proposed route uses a small homebrew client built with devkitPro/libctru on a custom-firmware 3DS and a companion bridge on a trusted computer. The console connects to the bridge over Wi-Fi; the bridge uses the configured save-relay service to exchange data with the selected Azahar save folder. This avoids putting the provider's refresh token or account password on the console. The bridge is a new component, not an existing save-relay server; this document does not specify an implementation or claim it exists.

The device and bridge must authenticate each other before any save transfer. Pairing must bind the expected bridge identity and a revocable, narrowly scoped device credential; it must be explicit, resistant to an unauthenticated network peer substituting a bridge, and never log secrets. The connection must verify the bridge certificate and hostname or a securely paired public-key identity, with encryption and integrity. It must fail closed on authentication, certificate, or TLS errors: no plain HTTP, disabled certificate checks, or silent fallback. The bridge must restrict filesystem access to explicitly configured roots and title mappings. Revocation, re-pairing and loss of a device must be supported without exposing provider credentials.

The [libctru HTTP interface](https://github.com/devkitPro/libctru/blob/master/libctru/include/3ds/services/httpc.h) exposes HTTP, timeout and certificate APIs. Their presence does not establish usable TLS versions, certificate validation behavior, pairing security, transfer limits or target-device compatibility. A feasibility prototype must verify these on a real device and the selected firmware before this protocol is considered implementable. If it cannot meet the trust requirements, stop and revise the design; do not weaken verification. The exact wire protocol, secure enrollment mechanism, credential storage and supported TLS stack remain design decisions for that work.

## Mapping and transfer

The person selects a title and explicitly configures one reviewed mapping among the 3DS save archive, the Azahar save location and a distinct relay game identity. Do not infer paths or titles from untrusted input, mix title data, or include unrelated archives. The [libctru filesystem interface](https://github.com/devkitPro/libctru/blob/master/libctru/include/3ds/services/fs.h) identifies several archive types and a save commit operation; access to a particular title's archive, commit semantics, permissions and Azahar's actual layout require validation for the chosen title. No game or save files are shipped or committed.

Transfer a bounded manifest and complete snapshot, with validated relative paths, sizes and hashes. Stage and verify the whole incoming snapshot before replacing a save. Retain a recoverable copy before replacement, commit the target using its platform's supported mechanism, and verify the outcome. An interruption, bad hash, wrong mapping, authentication failure or unavailable provider must not publish a partial save. Resume and recovery behavior must be tested for both the bridge filesystem and console archive; do not assume desktop rename semantics apply to a 3DS save archive.

## Conflict policy for this route

When both copies changed, the owner's rule for this 3DS route is newest timestamp wins, with the losing copy durably retained as a timestamped backup before replacement. This rule does not change ordinary save-relay conflict behavior. The implementation must define the timestamp source, granularity, skew tolerance, backup identity and a durable ordering that survives a crash. Equal or untrustworthy timestamps require an explicit choice rather than silently selecting a winner. A clock comparison cannot establish recency until the clocks and timestamp provenance have been verified.

## Acceptance and open gates

1. Pin the toolchain and prototype pairing, certificate validation, TLS interoperability, bounded requests, credential storage and safe save-archive access on the target. Document results and choose a concrete protocol only after these checks.
2. Exercise synthetic snapshots and failure cases: both directions, interruption and recovery, path escape, wrong title, corrupted data, invalid credentials, TLS failure, clock ties and skew, and two changed copies with a preserved loser. Do not include proprietary files.
3. With privately held real saves for at least one explicitly mapped title, the owner verifies a 3DS → Azahar round trip and the reverse, launches both sides to confirm the saves load, and verifies both-changed resolution and backup recovery. Record versions, steps and outcomes without publishing save contents or credentials.

Neither source review nor synthetic tests satisfy the real-device gate. Keep #300 open until its complete acceptance criteria, including this owner-run check, are met.

---

# Transport and bridge protocol specification (proposal for #319)

Everything below is a proposal an implementation can be built from. Nothing here has been run on a 3DS, in Azahar, or in any TLS stack. Claims about console capability are tagged **unverified**, and each one is a gate in the feasibility prototype (see "Stop and revise rules"). Terms: *console* is the libctru client; *bridge* is the host program; *host copy* is the Azahar save folder as tracked through the existing save-relay service; *tid* is a 3DS title id.

Changes to the text above: (1) the earlier proposal said "mutually authenticated HTTPS"; this spec authenticates the bridge with TLS plus a pinned private CA, and the console with a pairing-derived MAC at the application layer, not a TLS client certificate (reasons in section 1). (2) The wire protocol, enrollment, credential storage and TLS stack, left open above, are decided here, still subject to the hardware gates.

## 1. Transport decision

The console speaks HTTPS to the bridge on the LAN. No plain HTTP, no certificate-check bypass (such as libctru's skip-verify SSL option), no fallback to a weaker mode, ever.

| | (a) mTLS, bridge-issued client certificate | (b) pinned server trust plus pairing-derived MAC |
|---|---|---|
| Console proves identity with | TLS client cert and private key (`httpcSetClientCert`; **unverified** that sslc accepts it, and which key types and sizes) | HMAC-SHA-256 over each request, key derived at pairing |
| Bridge proved by | server cert chained to a CA added with `httpcAddTrustedRootCA` | same |
| Depends on console TLS stack for | server validation and client auth | server validation only |
| Revocation | no CRL or OCSP on the console; the bridge keeps an allowlist anyway | delete the device record; effective on the next request |
| If client certs are broken | the design is blocked | no effect: the MAC is independent of TLS |
| Defense in depth | TLS only | TLS plus MAC; a TLS validation flaw alone does not let a peer act as the console |

**Decision: (b) is the baseline.** It needs the fewest unverified console features (one: validate a private root, ideally with name checking), keeps authentication sound if sslc proves weak, and gives per-device revocation without a PKI. mTLS may be added as hardening only if the prototype shows client certs work; the bridge then requires both and the MAC stays.

### Console TLS capabilities (all **unverified** until a real console run)

| Capability needed | libctru surface it would use | Status |
|---|---|---|
| HTTPS requests with bounded request and response bodies | `httpcOpenContext`, `httpcAddPostDataRaw`, `httpcBeginRequest`, `httpcDownloadData` | unverified at 64 KiB; method support may be limited, so every mutating verb is `POST` (section 3) |
| Private root CA accepted | `httpcAddTrustedRootCA` (DER) | unverified: a leaf from that CA must validate and a leaf from any other CA must be refused |
| Hostname or IP SAN validation | none documented in the header | unverified: if names are not checked, the trust anchor is a CA used for this one bridge only, so no other cert chains to it; test with a wrong-SAN leaf |
| TLS version and suites | not selectable | unverified: assume TLS 1.2 at best, RSA-2048 with SHA-256, common AES suites; the bridge leaf is RSA-2048 and its profile includes a TLS 1.2 RSA or ECDHE-RSA suite |
| Client certificate | `httpcSetClientCert` | unverified; optional (decision above) |
| Validity dates vs console clock | sslc may check against the RTC | unverified: CA valid 10 years, leaf 2 years, both backdated 1 day; test with a console clock a year off |
| No redirects followed | httpc redirect handling | unverified; the client treats any 3xx as `server_error` |

**Fallback: mbedTLS from devkitPro portlibs over `soc` sockets** (own TCP and TLS 1.2 client). It lets the client pin `sha256(SubjectPublicKeyInfo)` directly and ignore names and dates, and it fixes the suites, at the cost of RAM, more console code, and a random source (**unverified** which one: PS service random bytes are preferred; anything weaker needs owner review). The protocol above the transport does not change. The SPKI pin is part of the pairing bundle either way.

### Stop and revise rules

Stop the console route and revise this design, without weakening any check, if on the pinned firmware:

1. Neither httpc nor mbedTLS validates the bridge against the paired private CA or pinned key and refuses another.
2. No TLS 1.2 suite shared with a current server stack can be negotiated. The only remaining option would be a deliberately old bridge TLS profile; that needs the owner's explicit written decision, not a developer one.
3. The console cannot produce 128-bit random values of acceptable quality.
4. Verification only works with certificate checks disabled, or over plaintext.
5. Bodies of 64 KiB cannot be moved reliably (shrink the chunk bound and retest; stop if 16 KiB fails).

## 2. Pairing, identity, scopes, revocation, rotation

### Identities and storage

| Item | Bridge | Console |
|---|---|---|
| Bridge CA | private key in the bridge data directory, user-only permissions, never leaves the host | public cert (DER) in the pairing file on the SD card |
| Bridge leaf cert | issued by that CA; SAN lists the configured host name and IP | not stored |
| Device record | `device_id` (random 128-bit hex), label, scopes, created, last seen, state `active` or `revoked` | `device_id`, `device_key` (32 bytes), CA, bridge address, SPKI pin |
| Device key | in the host OS credential store through the existing save-relay credential store pattern (M6), else a user-only file; the bridge needs the key to verify MACs | `sdmc:/3ds/save-relay/device.bin`. The SD card is **not** a secret store (threat model): the key is scoped and revocable, nothing more |
| Relay provider credentials | stay in the host credential store | never present |

### Pairing flow (host-initiated, one console at a time)

1. Owner runs `bridge pair --scope <tid>[,<tid>] --name "<label>"` on the host. The bridge opens a 5-minute window accepting only `POST /v1/pair`.
2. The host prints a **pairing bundle**: CA cert, bridge address and port, SPKI pin, and a one-time 128-bit `pair_secret`. Delivery, in order of preference: (i) a QR code on the host screen scanned by the console (**unverified**: 3DS camera plus a QR decoder), (ii) the bundle as a file the owner copies to the SD card, (iii) typing the base32 secret and pin on the console keyboard, with the CA copied as a file. The bundle is the trust anchor and must reach the console out of band, never over the network.
3. The host also prints a **short code**: 6 digits, for human verification only, not a secret.
4. The console makes `device_id` and a 16-byte `client_nonce`, then calls `POST /v1/pair` over TLS validated against the bundle's CA with `{device_id, client_nonce, proof}`, where `proof = HMAC-SHA-256(pair_secret, "sr-pair-1" || device_id || client_nonce)`.
5. The bridge checks the proof, the window and the attempt count (5 wrong proofs close the window), and replies `{server_nonce}`. Both sides derive `device_key = HKDF-SHA-256(pair_secret, salt = client_nonce || server_nonce, info = "sr-device-key-1" || device_id, 32 bytes)`. The key never crosses the wire.
6. Both show `sas = int(HMAC-SHA-256(device_key, "sr-sas-1")[0..4]) mod 10^6`. The owner compares the two and confirms on the host; only then the bridge sets the device `active`. A mismatch or no confirmation discards the record. `pair_secret` is single use and erased on success, expiry or window close.

Why not a short code alone: 6 digits are guessable, and cannot authenticate the bridge to a console that does not yet know its CA. The out-of-band bundle carries the trust anchor; the short code only lets the human confirm that both ends derived the same key.

### Scopes, revocation, rotation

- **Scope**: per device, a list of titles each with `read` and/or `write`. Changing scope is a host command, effective on the next request. An out-of-scope request is `scope_denied` and never reveals whether the title exists.
- **Revocation**: `bridge device revoke <id>` marks the record revoked and drops its sessions; every later request is `device_revoked` (HTTP 401). A lost console is revoked at the host; no provider credential was ever on it. `bridge device list` shows label, scopes, last seen.
- **Device key rotation**: `POST /v1/rotate` in an authenticated session exchanges nonces; both derive `device_key' = HKDF(device_key, client_nonce || server_nonce, "sr-rotate-1")`. The bridge accepts the old key until the new one has signed one request or 10 minutes pass; the console replaces its file only after that request succeeds. Recommended every 90 days, a warning rather than a block.
- **Leaf rotation**: reissue under the same CA, no console change. **CA rotation**: re-pair (no overlap scheme in v1).
- **Re-pair** for an existing `device_id` happens only through the host `pair` command, never from the network.

### Sessions (replay bounds without trusting the console clock)

`POST /v1/session` with `{device_id, client_nonce, rtc_ms}`, authenticated by `sig = HMAC(device_key, "sr-session-0" || device_id || client_nonce || rtc_ms)`, returns `{session_id, server_nonce, server_time_ms, expires_in_s}` (default 600). The session key is `HMAC(device_key, "sr-session-1" || client_nonce || server_nonce)`. Each later request carries `Authorization: SR1 session=<id>, seq=<n>, sig=<b64>` with `seq` strictly increasing from 1 and `sig = HMAC-SHA-256(session_key, method "\n" path_and_query "\n" seq "\n" X-SR-Body-SHA256)`. The bridge rejects a repeated or lower `seq`, an expired session or a bad MAC with `auth_failed`. Responses carry `X-SR-Resp-MAC = HMAC(session_key, seq "\n" status "\n" sha256(body))`, which the console verifies before trusting the body. The console clock never authenticates anything; it only dates snapshots (section 5). The only unauthenticated routes are `GET /v1/hello` and `POST /v1/pair`. Failed authentication is limited to 10 per minute per source IP and per device id. HMAC and SHA-256 on the console come from a vendored implementation or mbedTLS (**to pin**).

## 3. Bridge API

Base path `/v1`. JSON uses `application/json; charset=utf-8`; chunks use `application/octet-stream`. Every mutating verb is `POST`, so the console need not rely on `PUT` or `DELETE` (**unverified**). `X-SR-Body-SHA256` is the lowercase hex SHA-256 of the body (of the empty string when none). `Idempotency-Key` (128-bit hex, client-random) is required on `stage` and `commit`.

### Limits (the bridge enforces them; `GET /v1/hello` reports them)

| Limit | Default | Hard maximum |
|---|---|---|
| Files per snapshot | 64 | 256 |
| Per-file size | 2 MiB | 16 MiB |
| Total snapshot size | 4 MiB | 16 MiB (mapping may set lower; real 3DS saves are mostly at most 1 MiB) |
| Manifest JSON | 32 KiB | 64 KiB |
| Chunk body | 64 KiB | 256 KiB |
| Request headers | 4 KiB | 8 KiB |
| Open stages | 1 per title per device | 2 per device |
| Stage time to live | 15 min idle | 1 h |
| Idempotency record kept | 24 h | 7 d |
| Snapshots listed | 50 | 200 |

Path rule for every `path` in a manifest: UTF-8 NFC, `/` separators, relative, no empty, `.` or `..` component, no leading `/`, no `\`, no NUL or control characters, each component at most 255 bytes, total at most 1024, no two paths equal after case folding, regular files only. A violation is `manifest_invalid`. Paths are joined under the mapped root only after this check, and the result must still resolve inside the root.

### Verbs

| Verb | Auth | Purpose |
|---|---|---|
| `GET /v1/hello` | none | `{"proto":[1],"bridge_id","limits":{...},"server_time_ms"}`; the client refuses a bridge whose `proto` lacks 1 (`unsupported_version`) |
| `POST /v1/pair`, `/v1/session`, `/v1/rotate` | section 2 | identity |
| `GET /v1/titles` | session | titles in this device's scope |
| `POST /v1/titles/{tid}/plan` | read | the bridge decides what to do |
| `GET /v1/titles/{tid}/snapshots` | read | list, newest first |
| `GET /v1/titles/{tid}/snapshots/{sid}` | read | the manifest |
| `GET /v1/titles/{tid}/snapshots/{sid}/files/{n}?offset=&len=` | read | bytes of file `n` (manifest index) |
| `POST /v1/titles/{tid}/stage` | write | open an upload |
| `POST /v1/titles/{tid}/stage/{stage}/chunk?file=&offset=` | write | send bytes |
| `POST /v1/titles/{tid}/stage/{stage}/verify` | write | the bridge hashes everything staged |
| `POST /v1/titles/{tid}/stage/{stage}/commit` | write | publish (section 5) |
| `POST /v1/titles/{tid}/stage/{stage}/abort` | write | discard |
| `POST /v1/titles/{tid}/ack` | write | the console reports it applied a snapshot |

`{tid}` is the 16-hex-digit lowercase title id. Byte ranges are query parameters, so no `Range` header support is assumed.

### JSON shapes

`GET /v1/titles`:

```json
{"ok":true,"data":{"titles":[{"title_id":"0004000000abcd00","game_id":"example-game","name":"Example",
  "access":["read","write"],"max_total_bytes":4194304,
  "head":{"snapshot_id":"20261125T101500Z-0007","seq":7,"manifest_sha256":"<hex>"}}]}}
```

`plan` request, then response:

```json
{"console":{"manifest_sha256":"<hex or null when the archive is empty>","total_bytes":524288,
  "captured_rtc_ms":3500000000000,
  "last_synced":{"snapshot_id":"20261125T101500Z-0007","manifest_sha256":"<hex>"}}}
```
```json
{"ok":true,"data":{"action":"noop|push|pull|conflict_auto|needs_choice","reason":"console_changed",
  "snapshot_id":"<id, for pull and conflict_auto>",
  "decision":{"rule":"newest_wins","offset_s":-3600,"residual_s":2,"tolerance_s":300,
              "console_time_ms":0,"host_time_ms":0,"winner":"console|host|null"}}}
```
`push`: the console is ahead, stage then commit. `pull`: the host is ahead, download `snapshot_id`. `conflict_auto`: both changed and the clocks are trusted; the named winner applies and the loser is kept. `needs_choice`: both changed with an untrusted or tied clock, so the console shows both sides and the owner picks. The names mirror save-relay's `Noop / Pull / Push / Conflict` plan; `Adopt` is folded into `noop` when both manifests hash equal. `plan` changes nothing.

`stage` request and response:

```json
{"base_snapshot_id":"<head the plan saw, or null>","manifest":{"...":"section 4"}}
{"ok":true,"data":{"stage_id":"<hex>","chunk_bytes":65536,"received":[0,0,0],"expires_in_s":900}}
```
A repeat of `stage` with the same `Idempotency-Key` and identical body returns the same `stage_id` and the current `received` array (bytes accepted per file, contiguous from 0): this is the resume mechanism. Same key with a different body is `usage`.

`chunk?file=2&offset=65536`: the body is raw bytes with `X-SR-Body-SHA256`. `offset` must equal `received[file]`; otherwise the bridge answers 409 `state_invalid` with `data.received` so the client resumes from the truth. A good chunk returns `{"ok":true,"data":{"received":[...]}}`; a chunk beyond the declared file size is `payload_too_large`.

`verify` re-hashes every staged file against the manifest and returns `{"ok":true,"data":{"manifest_sha256":"<hex>","files_ok":3}}`; a mismatch is `manifest_invalid` naming the file index (never content) and drops the stage. `commit` takes `{"resolve":null|"keep_console"|"keep_host"}` and returns:

```json
{"ok":true,"data":{"outcome":"committed|noop|conflict_resolved","snapshot_id":"<new head>","seq":8,
  "loser_snapshot_id":"<id or null>"}}
```
`ack` takes `{"snapshot_id","applied_manifest_sha256"}` and returns `{"ok":true,"data":{}}`.

### Errors

Envelope: `{"ok":false,"error":{"code":"<code>","message":"<short>","retryable":true|false}}`. A message never contains a key, MAC, token, save content or a host path outside the mapping. Codes reuse save-relay's vocabulary (`docs/code-map.md`, "Error codes"); the last four are **proposed additions** (code-map allows adding, never renaming):

| Code | HTTP | Meaning on this route |
|---|---|---|
| `usage` | 400 | malformed request, idempotency key reused with another body |
| `manifest_invalid` | 422 | path rule, count, size or hash check failed |
| `game_unknown` | 404 | the title has no reviewed mapping |
| `not_found`, `no_such_entry` | 404 | unknown stage or snapshot |
| `auth_failed` | 401 | bad MAC, expired session, replayed `seq` |
| `not_set_up` | 409 | the bridge has no relay service configured |
| `reauth_needed`, `credentials` | 503 | the host's relay sign-in needs the owner at the host, not the console |
| `server_unreachable` | 502 | the bridge cannot reach the relay service; the console must not treat this as an offline win |
| `server_error` | 500 | bridge or provider fault |
| `state_invalid` | 409 | stage state or `offset` mismatch; `data.received` carries the truth |
| `changed_on_server` | 409 | the head moved since `base_snapshot_id`; run `plan` again |
| `io` | 500 | host filesystem failure |
| `scope_denied` | 403 | **new**: title or access not in the device scope |
| `device_revoked` | 401 | **new** |
| `unsupported_version` | 400 | **new**: no common `proto` |
| `payload_too_large` | 413 | **new**: a limit above |

A conflict is not an error: it is the `plan` outcome `needs_choice`, as in save-relay where `conflict` changes nothing until the player chooses. Console retry rule: retry only on `retryable: true` or a timeout, with the same `Idempotency-Key`, at most 3 attempts after 1, 3 and 9 s; never retry a bad MAC. Timeouts: connect 5 s, no progress 30 s, no total deadline (as `UreqTransport`).

## 4. Snapshot and mapping schema

### Snapshot manifest (format 1)

```json
{"format":1,"title_id":"0004000000abcd00","game_id":"example-game",
 "origin":"console","device_id":"<hex, or host>",
 "created":{"source":"console_rtc","unix_ms":0},
 "files":[{"path":"00000001.sav","size":524288,"sha256":"<64 hex>"}],
 "total_bytes":524288}
```
`files` are sorted bytewise by `path`. `manifest_sha256` is the SHA-256 of the manifest in canonical form (sorted keys, no insignificant whitespace, integers only) **with `created` and `origin` removed**, so identical files have one identity whatever the clock. The bridge stores each snapshot as `snapshots/<snapshot_id>/manifest.json` plus `files/`; `snapshot_id = <UTC stamp>-<seq, 4 digits>`.

What a file is on the console is **unverified**: whatever the client enumerates in the title's save archive through libctru's FS (`FSUSER_OpenArchive`, `FSUSER_ControlArchive` for commit). Whether a homebrew app may open another title's save archive, and with what permissions on the pinned custom firmware, is a prototype gate. The manifest does not depend on the answer; an empty archive is an empty `files`.

### Title to relay game mapping (host file, owner-authored and reviewed)

```json
{"format":1,"titles":[{"title_id":"0004000000abcd00","game_id":"example-game","name":"Example",
  "azahar_save_dir":"<absolute path to the title's Azahar save directory>",
  "console_archive":{"kind":"savedata","media":"sd"},"max_total_bytes":4194304}]}
```
Rules: each `title_id` and each `game_id` appears once; `game_id` must satisfy save-relay's game id rules (**confirm against the manifest module at implementation**); `azahar_save_dir` exists, is a directory, and overlaps no other mapping; no path is derived from a title string or from anything the console sends: the console sends only a `title_id`, and an unmapped one is `game_unknown`. The Azahar layout (Citra lineage, `sdmc/Nintendo 3DS/<id0>/<id1>/title/<high>/<low>/data/00000001/`) and whether Azahar picks up files written there without a restart are **unverified** for the pinned Azahar. No game or save content is committed; fixtures are synthetic.

Relay side: each mapped title is one save-relay game. The bridge treats the Azahar directory as the local folder and uses the existing engine (pull, push, conflict, history) against the relay service. The console is a second peer of the bridge: its snapshots reach the Azahar directory by the commit protocol in section 5, then go up by the normal engine path. A relay conflict with another device follows save-relay's ordinary policy; this route's newest-wins applies only between console and host copy.

## 5. Ordering, conflicts and the commit record

### Detect change first, compare clocks only when needed

Each side stores `last_synced = {snapshot_id, manifest_sha256}`: the console in `sdmc:/3ds/save-relay/state.json`, the bridge per device and title in `head.json`. `console_changed` means the hash of the console files differs from `last_synced`; `host_changed` means the host head differs from it. Only one changed: that side wins and no clock is involved. Both changed with equal manifest hashes: `noop`, record `last_synced`. Both changed and different: the clock rule below. A missing `last_synced` counts as changed on that side, so a first sync with data on both sides is a conflict; an empty side loses without a clock.

### Timestamp sources

| Side | Source | Notes |
|---|---|---|
| Host copy | host clock: newest file mtime in the Azahar directory, else the relay head time | assume NTP; the bridge status shows whether the host clock is synced |
| Console | console RTC at capture, mapped to host time by the session offset | the 3DS RTC is local time with no zone and user-settable (**unverified**: `osGetTime` meaning and precision); the archive may expose no per-file time (**unverified**), so the value means "when the client captured it", not "when the game saved" |

Offset: the session request carries `rtc_ms`; the bridge computes `d = server_time - rtc` and sets `offset_s = round(d to the nearest 900 s)` (a quarter hour absorbs the zone) with residual `r = d - offset_s`. It is measured every session and never stored, so a daylight-saving step between sessions is absorbed.

### Rule (this route only)

1. `|r| > 60 s`, or `|offset_s| > 26 h`: `needs_choice`, reason `clock_unreliable`.
2. Else with `t_c = captured_rtc + offset_s` and `t_h` the host time: `|t_c - t_h| <= tolerance` (default 300 s, configurable 60 to 3600 s) is a tie: `needs_choice`, reason `tie`.
3. Else the later one wins and the loser is retained.

`needs_choice` is resolved by the owner on the console UI (`keep_console` or `keep_host`) and sent as `commit.resolve`; the choice is written into the commit record. Skew inside the tolerance is the owner's call by design.

### Loser retention and naming

Before the overwritten side changes, it is stored as snapshot `<stamp>-<seq>-loser` with the `.save-relay-conflict-loser` marker that retention already honours (`docs/code-map.md`). The bridge's retention keeps the newest 10 other snapshots per title (as M4) and never prunes losers. A host loser also enters the relay history through the normal engine, so `save-relay restore` can bring it back. On the console, the archive being overwritten is first copied to `sdmc:/3ds/save-relay/backup/<tid>/<stamp>/`; the bridge never deletes console backups.

### Crash-safe commit record

Order is a per-title integer `seq` in `head.json`, never a timestamp, so two commits cannot tie. A commit replaces `journal/<tid>.json` atomically (write temp, fsync, rename, fsync the directory) at each step:

```json
{"format":1,"title_id":"...","device_id":"...","seq":8,"op":"console_to_host|host_to_console|conflict",
 "state":"staged|verified|loser_saved|applied|published|complete",
 "from_snapshot":"...","to_snapshot":"...","loser_snapshot":"... or null",
 "decision":{"rule":"newest_wins|owner_choice|one_side_changed","offset_s":0,"residual_s":0,
             "tolerance_s":300,"winner":"console|host"}}
```

Console to host: `staged` (all chunks present) -> `verified` (re-hash equals manifest) -> `loser_saved` (previous host copy stored, only if both changed) -> `applied` (new files written to a sibling directory and swapped in by rename; the previous directory is kept until `published`) -> `published` (`head.json` written with the new `seq`, `last_synced` updated) -> `complete` (relay push attempted; a relay failure is reported later like save-relay's `deferred` and is not a failed commit). Recovery at bridge start, idempotent: `staged` and `verified` discard; `loser_saved` restores the pre-state and keeps the loser; `applied` rolls forward when the Azahar directory hashes to `to_snapshot`, else restores the kept previous directory; `published` and later finish. Replaying `commit` with the same `Idempotency-Key` after a crash returns the recorded result.

Host to console: the console downloads into `sdmc:/3ds/save-relay/stage/<tid>/`, verifies hashes, saves its backup, writes the files into the save archive, commits the archive with `FSUSER_ControlArchive`, re-reads and hashes, writes `last_synced`, and only then calls `ack`. It keeps its own journal (`staged`, `verified`, `backed_up`, `written`, `committed`, `acked`). The bridge counts a pull as finished only on `ack`; an un-acked pull shows as `pending_console_ack` and is offered again next session. Whether the archive commit is atomic, and whether power loss before it leaves the old archive intact, is **unverified**: the prototype cuts power at each console step. Do not assume desktop rename semantics.

## 6. Threat model

| Threat | Capability | Mitigation | Residual risk |
|---|---|---|---|
| LAN attacker, passive | sees traffic | TLS to a private CA; no plaintext route | packet sizes and timing |
| LAN attacker, fake bridge | spoofs DNS or IP, serves a certificate | console trusts only the bundle CA or SPKI pin; responses carry a MAC it cannot form | relies on unverified console chain and name validation (stop rule 1) |
| LAN attacker, fake console | connects to the bridge | nothing without a pairing or session MAC; pairing only in a 5-minute host-opened window with a 128-bit secret and 5 attempts; per-IP and per-device limits | an attacker who also captures the bundle (shoulder-surf, QR photo) in the window must still pass the owner's short-code confirmation on the host |
| Stolen console or SD card | reads `device.bin` | scoped to named titles; revoke at the host; no provider credential on the console; rotate on suspicion | the thief can read and write those titles until revoked; every overwrite keeps a loser or earlier snapshot, so data is recoverable but not private; no PIN in v1 |
| Malicious bridge peer | a rogue LAN host answering at the bridge address, or a process on the host | CA key and device keys are host-user-only; the console checks pin and MAC; the bridge binds only the configured interface | a process running as the host user can read everything: out of scope |
| Replay | resends captured requests | TLS; per-session increasing `seq`; session expiry; MAC over method, path, seq and body hash; idempotency keys only return the recorded result | none beyond a TLS break plus session key theft |
| Path escape, hostile manifest | `..`, absolute paths, case collisions, huge counts | path rule and limits; mapped roots only; no symlinks | implementation bugs, covered by synthetic tests |
| Resource exhaustion | many stages, big bodies | limits, quotas, stage TTL | LAN denial of service stays possible |
| Wrong title or mixed saves | bad mapping | reviewed mapping; console sends only a `tid`; scope, `base_snapshot_id` and head checks | owner mapping error |
| Bad clock | wrong winner | change detection first; clocks only for both-changed; reliability and tie rules force a choice | owner picks wrong; the loser is kept |
| Secrets in logs | | `Authorization` and MAC headers redacted as `http::Request`'s `Debug` does; messages carry no key, path or save content | |

## 7. Acceptance split

**Host-side automated tests (pure Rust bridge logic, synthetic fixtures, loopback only, no console, no real saves):**

- Manifest parse, canonical hash, path rules (traversal, absolute, backslash, case-fold collision, over-long, NUL, symlink), every limit.
- Pairing derivation and short code against fixed test vectors; window expiry, attempt limit, single use; scopes; revoke; rotate; session `seq` replay and expiry; bad MAC; response MAC.
- `plan` decision table: one side changed (both directions), equal hashes, first sync, empty side, tie, reliable newest-wins both ways, unreliable offset, a 3600 s DST step, large residual, owner choice.
- Stage, chunk, resume at each offset, wrong offset (`state_invalid` with `received`), idempotent replays, verify mismatch, abort, TTL.
- Commit journal with crash injection after every state on a fake filesystem, roll back and forward, loser retained and exempt from retention, naming and marker, `ack` and `pending_console_ack`.
- Mapping validation (overlap, duplicates, missing directory) and the error-code table.
- Wire fixtures for every JSON shape, shared with the console client's parser so both sides read the same files.
- Bridge-side TLS only: a loopback server with a generated test CA accepts a client trusting that CA and refuses one that does not.

**Only a real 3DS plus Azahar round trip proves (#300):** that httpc or the fallback stack negotiates with the bridge on the pinned firmware; private-root, name or IP validation, suites and clock behavior; client certs (optional); sustained 64 KiB chunks; QR scan or manual code entry; RTC meaning, zone and precision; whether the title's save archive can be opened, enumerated, written and committed, and whether power loss before commit is safe; the real file set of a title; Azahar reading the written folder and the game loading it, both directions; both-changed resolution and loser recovery on real data; host-side revocation of a real console. Record versions and outcomes without publishing save contents or credentials.

Neither these tests nor a source review close the real-device gate. The stop rules in section 1 apply the moment a prototype contradicts a capability tagged **unverified**.

## 8. Pinned toolchain (to pin at implementation time)

No versions are asserted here. Each row is filled, with its source, when the prototype is built; the reproducible build is documented then.

| Component | Version or commit | Source of the pin |
|---|---|---|
| devkitPro and devkitARM | to pin | |
| libctru | to pin | |
| Portlibs used (mbedTLS if the fallback is taken; zlib if used) | to pin | |
| Cross-build host (OS; container image digest if used) | to pin | |
| Target console model and system firmware | to pin | |
| Custom firmware and launcher | to pin | |
| Azahar release | to pin | |
| Bridge Rust toolchain and server TLS crate | to pin | |
| Licenses of every vendored piece linked into the console client | to confirm | |
