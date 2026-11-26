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

When both copies changed, the owner's stated rule for this 3DS route (#300) is newest timestamp wins, with the losing copy durably retained as a timestamped backup before replacement. This rule does not change ordinary save-relay conflict behavior. The console has no known source for the time a game last saved, so what "newest" means on the console is an open owner decision (section 9, question 6). Until the owner rules, this design asks the owner to choose whenever both copies changed (`needs_choice`), and it still retains the loser. The implementation must define the timestamp source, granularity, skew tolerance, backup identity and a durable ordering that survives a crash. Equal or untrustworthy timestamps require an explicit choice rather than silently selecting a winner. A clock comparison cannot establish recency until the clocks and timestamp provenance have been verified.

## Acceptance and open gates

1. Pin the toolchain and prototype pairing, certificate validation, TLS interoperability, bounded requests, credential storage and safe save-archive access on the target. Document results and choose a concrete protocol only after these checks.
2. Exercise synthetic snapshots and failure cases: both directions, interruption and recovery, path escape, wrong title, corrupted data, invalid credentials, TLS failure, clock ties and skew, and two changed copies with a preserved loser. Do not include proprietary files.
3. With privately held real saves for at least one explicitly mapped title, the owner verifies a 3DS → Azahar round trip and the reverse, launches both sides to confirm the saves load, and verifies both-changed resolution and backup recovery. Record versions, steps and outcomes without publishing save contents or credentials.

Neither source review nor synthetic tests satisfy the real-device gate. Keep #300 open until its complete acceptance criteria, including this owner-run check, are met.

---

# Transport and bridge protocol specification (proposal for #319)

Everything below is a proposal an implementation can be built from. Nothing here has been run on a 3DS, in Azahar, or in any TLS stack, and nothing described here is implemented. Claims about console capability are tagged **unverified**, and each one is a gate in the feasibility prototype (see "Stop and revise rules"). Terms: *console* is the libctru client; *bridge* is the host program; *host copy* is the Azahar save folder as tracked through the existing save-relay service; *tid* is a 3DS title id. Where the design depends on a decision that belongs to the owner, this spec states the safe default it holds until the owner rules, and section 9 lists every such question.

Changes to the text above: (1) the earlier proposal said "mutually authenticated HTTPS"; this spec authenticates the bridge with TLS plus a pinned private CA, and the console with a pairing-derived MAC at the application layer, not a TLS client certificate (reasons in section 1; owner question 4). (2) The wire protocol, enrollment, credential storage and TLS stack, left open above, are proposed here, still subject to the hardware gates.

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

**Proposal: (b) is the baseline.** It needs the fewest unverified console features (one: validate a private root), keeps authentication sound if sslc proves weak, and gives per-device revocation without a PKI. mTLS may be added as hardening only if the prototype shows client certs work; the bridge then requires both and the MAC stays. This departs from the mutually authenticated HTTPS suggested on #300, so it needs the owner's acceptance (section 9, question 4).

### Console TLS capabilities (all **unverified** until a real console run)

| Capability needed | libctru surface it would use | Status |
|---|---|---|
| HTTPS requests with bounded request and response bodies | `httpcOpenContext`, `httpcAddPostDataRaw`, `httpcBeginRequest`, `httpcDownloadData` | unverified at 64 KiB; method support may be limited, so every mutating verb is `POST` (section 3) |
| Private root CA accepted | `httpcAddTrustedRootCA` (DER) | unverified: a leaf from that CA must validate and a leaf from any other CA must be refused |
| Paired root is the only trusted root | none documented | unverified, and a hard requirement (see below): no default or Nintendo roots may remain in effect for this connection |
| Hostname or IP SAN validation | none documented in the header | unverified: see "Trust without name checking" |
| TLS version and suites | not selectable | unverified: see "Bridge TLS profile" |
| Client certificate | `httpcSetClientCert` | unverified; optional (decision above) |
| Validity dates vs console clock | sslc may check against the RTC | unverified: CA valid 10 years, leaf 2 years, both backdated 1 day; test with a console clock a year off |
| No redirects followed | httpc redirect handling | unverified; the client treats any 3xx as `server_error` |

**Trust without name checking.** If sslc does not check names, the fallback is safe only when the paired CA is the *sole* trust root for the request and signs nothing but this bridge's leaf. If sslc also trusts its built-in roots (any public CA, or the console vendor's roots) alongside the added one, a certificate for any name from those roots would be accepted, and the fallback is unsafe: that is stop rule 1. Prototype tests, all required: (1) the bridge leaf validates; (2) a leaf from an unrelated private CA is refused; (3) a real public-CA-issued certificate for an arbitrary name, served on the same address, is refused; (4) a leaf from the same CA with a wrong SAN is refused if names are checked, or the test is recorded as "names unchecked" and the CA is then used only for this one bridge (a new CA per pairing, never reused across bridges).

**Bridge TLS profile (proposed; confirm against the pinned rustls version).** The bridge is built on rustls, matching save-relay's existing HTTP client stack (pure Rust, no OpenSSL). Offered: TLS 1.2 and 1.3, with only ephemeral-key suites: for 1.2, ECDHE-RSA with AES-128-GCM, AES-256-GCM or ChaCha20-Poly1305; for 1.3 the standard AEAD suites. The leaf is RSA-2048 signed with SHA-256, so older console stacks can verify it. rustls does not offer static-RSA key exchange (no forward secrecy) or CBC suites. Whether sslc negotiates one of the offered suites is **unverified** and a prototype gate. If it cannot, the options are the mbedTLS fallback below, or a bridge profile with static RSA or a different TLS library, which weakens forward secrecy and is an exception to the no-OpenSSL rule: that is the owner's written decision (section 9, question 7), never a developer's.

**Fallback: mbedTLS from devkitPro portlibs over `soc` sockets** (own TCP and TLS 1.2 client). It lets the client pin `sha256(SubjectPublicKeyInfo)` directly and ignore names and dates, and it fixes the suites, at the cost of RAM, more console code, and a random source (**unverified** which one: PS service random bytes are preferred; anything weaker needs owner review). The protocol above the transport does not change. The SPKI pin is part of the pairing bundle either way.

### Stop and revise rules

Stop the console route and revise this design, without weakening any check, if on the pinned firmware:

1. Neither httpc nor mbedTLS validates the bridge against the paired private CA or pinned key *as the only trusted root*, and refuses another (including a public-CA certificate).
2. No TLS 1.2 or 1.3 ephemeral-key suite shared with the bridge profile can be negotiated. The only remaining option would be a deliberately old bridge TLS profile; that needs the owner's explicit written decision.
3. The console cannot produce 128-bit random values of acceptable quality.
4. Verification only works with certificate checks disabled, or over plaintext.
5. Bodies of 64 KiB cannot be moved reliably (shrink the chunk bound and retest; stop if 16 KiB fails).
6. Only for a cartridge or anti-rollback title: see section 4, "3DS save archives".

## 2. Pairing, identity, scopes, revocation, rotation

### Byte encodings (used by every MAC and hash below)

All MAC and hash inputs are the exact byte strings defined here. Hex in JSON and headers is lowercase; `sig` and MAC headers are standard base64 with padding. Concatenation (`||`) has no separators, in the order listed. MACs are compared in constant time.

| Notation | Bytes |
|---|---|
| `u8`, `u16`, `u32`, `u64` | unsigned integer, fixed width 1, 2, 4, 8, big-endian |
| `raw(n)` | exactly `n` bytes; a hex field decoded to bytes (device_id, nonces, session_id: 16; SHA-256: 32). Any other length is rejected before any MAC is computed |
| `lp(s)` | `u16` byte length, then the UTF-8 bytes of `s`, no terminator. Labels are ASCII. Strings other than labels are NFC |
| `H(x)` | SHA-256 of `x`, 32 raw bytes |
| `HMAC(k, x)` | HMAC-SHA-256, 32 raw bytes |
| method code | `u8`: GET = 1, POST = 2 |
| `path_and_query` | the exact ASCII request target as sent. Every value in it is lowercase hex or decimal, so there is nothing to percent-encode; parameters appear in the order the verb table gives. The bridge MACs the bytes it received and also rejects a target that is not in that canonical form |

Defined byte strings (every label is part of the input, so no two uses can collide):

| Name | Input | Total bytes |
|---|---|---|
| pair proof | `HMAC(pair_secret, lp("sr-pair-1") \|\| raw(16) device_id \|\| raw(16) client_nonce)` | 11 + 16 + 16 = 43 |
| device key | `HKDF-SHA-256(ikm = pair_secret, salt = raw(16) client_nonce \|\| raw(16) server_nonce, info = lp("sr-device-key-1") \|\| raw(16) device_id, L = 32)` | |
| SAS input | `HMAC(device_key, lp("sr-sas-1") \|\| raw(16) device_id \|\| raw(16) client_nonce \|\| raw(16) server_nonce)`; `sas = u32(first 4 bytes) mod 1000000`, shown zero-padded to 6 digits | 10 + 48 = 58 |
| session request sig | `HMAC(device_key, lp("sr-session-0") \|\| raw(16) device_id \|\| raw(16) client_nonce \|\| u32 proto_mask \|\| u64 rtc_ms)` | 14 + 16 + 16 + 4 + 8 = 58 |
| session key | `HMAC(device_key, lp("sr-session-1") \|\| raw(16) client_nonce \|\| raw(16) server_nonce)` | |
| session reply MAC | `HMAC(device_key, lp("sr-session-reply-0") \|\| raw(16) device_id \|\| raw(16) client_nonce \|\| raw(16) server_nonce \|\| raw(16) session_id \|\| u8 proto \|\| u64 server_time_ms \|\| u32 expires_in_s \|\| limits block)` | 20 + 64 + 1 + 8 + 4 + 24 = 121 |
| limits block | six `u32`, in this order: files per snapshot, per-file bytes, total snapshot bytes, manifest bytes, chunk bytes, header bytes (section 3 table, the values in force) | 24 |
| request MAC | `HMAC(session_key, lp("sr-req-1") \|\| u8 method \|\| lp(path_and_query) \|\| u64 seq \|\| H(body) \|\| idem)`, where `idem` is `u8 0` when there is no `Idempotency-Key`, else `u8 1 \|\| raw(16) key` | 10 + 1 + (2 + len) + 8 + 32 + 1 or 17 |
| response MAC | `HMAC(session_key, lp("sr-resp-1") \|\| u64 seq \|\| u16 http_status \|\| H(body))` | 10 + 8 + 2 + 32 = 52 |
| content hash | `H(lp("sr-3ds-content-1") \|\| u8 format \|\| raw(8) title_id \|\| u32 file_count \|\| for each file in bytewise `path` order: lp(path) \|\| u64 size \|\| raw(32) file_sha256)`; the identity of the files only (see section 4) | 18 + 1 + 8 + 4 + per file (2 + len + 8 + 32) |
| rotate key | `HKDF-SHA-256(ikm = device_key, salt = raw(16) client_nonce \|\| raw(16) server_nonce, info = lp("sr-rotate-1") \|\| raw(16) device_id, L = 32)` | |

`proto_mask` is a `u32` with bit `n` set when the client speaks protocol version `n` (version 1 is `0x00000002`). `rtc_ms` is the console clock as unsigned milliseconds, sent in JSON as a decimal integer with no leading zeros (the bridge refuses any other form); `server_time_ms` the same on the bridge.

**Worked layout: request MAC** for `POST /v1/titles/0004000000abcd00/plan`, `seq = 3`, no `Idempotency-Key`:

| Field | Width | Value | Bytes |
|---|---|---|---|
| `lp("sr-req-1")` | 10 | label `sr-req-1` (8 bytes) | `00 08 73 72 2d 72 65 71 2d 31` |
| method | 1 | POST | `02` |
| `lp(path_and_query)` | 2 + 32 | 32-byte target `/v1/titles/0004000000abcd00/plan` | `00 20` then the 32 ASCII bytes |
| `seq` | 8 | 3 | `00 00 00 00 00 00 00 03` |
| `H(body)` | 32 | SHA-256 of the request body | 32 raw bytes |
| `idem` | 1 | none | `00` |

The message is 86 bytes. **Worked layout: content hash** for one file `00000001.sav` (12 bytes of path), 524288 bytes, file hash `h`: `00 10 "sr-3ds-content-1"` (18 bytes) `01` (format) `00 04 00 00 00 ab cd 00` (title id as 8 raw bytes) `00 00 00 01` (one file) `00 0c "00000001.sav"` (14 bytes) `00 00 00 00 00 08 00 00` (size) `h` (32 bytes): 85 bytes in total.

**Test vectors.** The prototype must generate `test-vectors/3ds-wire-1.json` with one entry per row above, each `{name, key_hex, message_hex, expected_hex}` (for the content hash and HKDF rows, `ikm`, `salt`, `info` and `L`), produced by the bridge's Rust code and cross-checked against the console client's C code (and a third, independent implementation such as a scripting language's standard library) before either side is accepted. Every `expected_hex` is **to be generated by the prototype**; none appears in this document. The bridge's tests and the console client's tests both read that one file. Negative vectors are included: a field one byte longer or shorter, a swapped nonce order, a missing label, a leading-zero `rtc_ms`.

### Identities and storage

| Item | Bridge | Console |
|---|---|---|
| Bridge CA | private key in the bridge data directory, user-only permissions, never leaves the host | public cert (DER) in the pairing file on the SD card |
| Bridge leaf cert | issued by that CA; SAN lists the configured host name and IP | not stored |
| Device record | `device_id` (random 128-bit hex), label, scopes, created, last seen, state `pending`, `active` or `revoked`, `key` and an optional `pending_key` (rotation) | `device_id`, `device_key` (32 bytes), CA, bridge address, SPKI pin |
| Device key | in the host OS credential store through the existing save-relay credential store pattern (M6), else a user-only file; the bridge needs the key to verify MACs | `sdmc:/3ds/save-relay/device.bin`, plus `device.bin.new` during a rotation. The SD card is **not** a secret store (threat model): the key is scoped and revocable, nothing more |
| Relay provider credentials | stay in the host credential store | never present |

### Pairing flow (host-initiated, one console at a time)

1. Owner runs `bridge pair --scope <tid>[,<tid>] --name "<label>"` on the host. The bridge opens a 5-minute window accepting only `POST /v1/pair`.
2. The host prints a **pairing bundle**: CA cert, bridge address and port, SPKI pin, and a one-time 128-bit `pair_secret`. Delivery, in order of preference: (i) a QR code on the host screen scanned by the console (**unverified**: 3DS camera plus a QR decoder), (ii) the bundle as a file the owner copies to the SD card, (iii) typing the base32 secret and pin on the console keyboard, with the CA copied as a file. The bundle is the trust anchor and must reach the console out of band, never over the network. Any bundle file the host writes is deleted by the bridge on success, expiry or window close; the console deletes its copy after pairing succeeds. A leftover copy is harmless once the secret is spent, and SD deletion is not secure erasure.
3. The host also prints a **short code**: 6 digits, for human verification only, not a secret.
4. The console makes `device_id` and a 16-byte `client_nonce`, then calls `POST /v1/pair` over TLS validated against the bundle's CA with `{device_id, client_nonce, proof}`, the proof as defined in "Byte encodings".
5. The bridge checks the proof, the window and the attempt count (5 wrong proofs close the window). **The first valid proof consumes `pair_secret`**: the device record is created in state `pending` and the bridge replies `{server_nonce}`. A second valid proof in the same window, from any source, finds the secret spent and is `auth_failed`; it never replaces the pending record. Both sides derive `device_key` as defined above; it never crosses the wire.
6. Both show the `sas` value. The owner compares the two and confirms on the host; only then the bridge sets the device `active`. A mismatch or no confirmation discards the record, and the secret stays spent. `pair_secret` is single use and erased on success, expiry or window close.

Why not a short code alone: 6 digits are guessable, and cannot authenticate the bridge to a console that does not yet know its CA. The out-of-band bundle carries the trust anchor; the short code only lets the human confirm that both ends derived the same key. The window length and the typed-secret fallback are owner question 1.

### Scopes, revocation, rotation

- **Scope**: per device, a list of titles each with `read` and/or `write`. Changing scope is a host command, effective on the next request.
- **Check order on every titled request**: (1) session and MAC, (2) device not revoked, (3) **scope**, (4) mapping. An out-of-scope title is `scope_denied` whether or not it is mapped, so the error code never reveals whether an out-of-scope title exists. Only a title inside the scope can reach `game_unknown`.
- **Revocation**: `bridge device revoke <id>` marks the record revoked and drops its sessions; every later request is `device_revoked` (HTTP 401). A lost console is revoked at the host; no provider credential was ever on it. `bridge device list` shows label, scopes, last seen.
- **Device key rotation** (hygiene only; on suspicion of theft, revoke and re-pair instead, because a thief holding the old key can rotate first). `POST /v1/rotate` in an authenticated session exchanges nonces; both derive the rotate key defined above. The bridge stores it as `pending_key` and keeps the old key active. The console writes the new key to `device.bin.new` (fsync) *before* it uses it, then opens a session with it. The bridge accepts a session MAC under either key. When a session under `pending_key` succeeds, the bridge makes it the key and drops the old one, and the console then renames `device.bin.new` over `device.bin`. If 10 minutes pass with no use of the new key, the bridge drops `pending_key` and keeps the old key. A console that finds both files tries `device.bin.new` first, then `device.bin`; whichever the bridge accepts is kept and the other deleted. A crash at any step therefore leaves at least one key that the other side accepts. Recommended every 90 days, a warning rather than a block.
- **Leaf rotation**: reissue under the same CA, no console change. **CA rotation**: re-pair (no overlap scheme in v1; owner question 3).
- **Re-pair** for an existing `device_id` happens only through the host `pair` command, never from the network.

### Sessions (replay bounds without trusting the console clock)

`POST /v1/session` with `{device_id, client_nonce, proto_mask, rtc_ms, sig}` (the session request sig in the encoding table) returns `{session_id, server_nonce, server_time_ms, expires_in_s, proto, limits, mac}` (default `expires_in_s` 600). The **session reply MAC** covers every field of the reply, including the selected `proto` and the `limits` in force, so a peer without the device key cannot forge a session, downgrade the version, or raise a limit. The console verifies `mac` before using `server_nonce`, and takes limits only from this reply, never from the unauthenticated `hello`. The session key is defined in the table.

Each later request carries `Authorization: SR1 session=<hex id>, seq=<n>, sig=<b64>` where `sig` is the request MAC. `seq` is strictly increasing from 1 within a session; **gaps are allowed** (a lost response is retried with a new `seq` and the same `Idempotency-Key`), and the bridge keeps only the highest `seq` it accepted. A repeated or lower `seq`, an expired session or a bad MAC is `auth_failed`. **Requests in one session are serialized**: a second request that arrives while one is in flight is refused with `usage` and does not advance `seq`. Every reply sent inside a session, errors included, carries `X-SR-Resp-MAC` (the response MAC), and the console verifies it before trusting the status or body. An error reply with no valid MAC (for example a failed session setup) is **untrusted**: the console treats it only as "failed", shows a generic message, and changes no state because of its content. The console clock never authenticates anything; it only feeds the recorded times in section 5. The only unauthenticated routes are `GET /v1/hello` and `POST /v1/pair`; whatever `hello` returns is advisory (a cheap early check) and is superseded by the session reply. Failed authentication is limited to 10 per minute per source IP and per device id. HMAC and SHA-256 on the console come from a vendored implementation or mbedTLS (**to pin**).

## 3. Bridge API

Base path `/v1`. JSON uses `application/json; charset=utf-8`; chunks use `application/octet-stream`. Every mutating verb is `POST`, so the console need not rely on `PUT` or `DELETE` (**unverified**). `X-SR-Body-SHA256` is the lowercase hex SHA-256 of the body (of the empty string when none), and the request MAC covers it through `H(body)`. `Idempotency-Key` (128-bit hex, client-random) is required on `stage`, `commit` and `ack`, and is part of the request MAC.

**Idempotency scope.** A recorded result is keyed by (`device_id`, `tid`, verb, `Idempotency-Key`). The same key under another title or another verb is a different record, never a match; the same key with a different body on the same (device, title, verb) is `usage`. Records are kept for the period in the limits table.

### Limits (the bridge enforces them; the session reply reports them, authenticated)

| Limit | Default | Hard maximum |
|---|---|---|
| Files per snapshot | 64 | 256 |
| Per-file size | 2 MiB | 16 MiB |
| Total snapshot size | 4 MiB | 16 MiB (mapping may set lower; the sizes of real 3DS saves are **unverified**, so the default is a guess to be checked in the prototype) |
| Manifest JSON | 32 KiB | 64 KiB |
| Chunk body | 64 KiB | 256 KiB |
| Request headers | 4 KiB | 8 KiB |
| Open stages | 1 per title per device | 2 per device |
| Stage time to live | 15 min idle | 1 h |
| Idempotency record kept | 24 h | 7 d |
| Snapshots listed | 50 | 200 |

The default and maximum snapshot sizes are owner question 2. The retained-loser cap is in section 5.

**Bounds that apply to byte ranges.** A download's `len` must satisfy `1 <= len <= chunk_bytes` and `offset + len <= size of file n`, else `usage`; the bridge never reads more than `chunk_bytes` for one request. An upload chunk body must be at most `chunk_bytes` and `offset + body length` at most the declared file size, else `payload_too_large`.

**Path rule** for every `path` in a manifest: UTF-8 NFC, `/` separators, relative, no empty, `.` or `..` component, no leading `/`, no `\`, no NUL or control characters, none of `: < > " | ? *`, each component at most 255 bytes, total at most 1024, no two paths equal after Unicode case folding, regular files only. Because the host may be Windows (and an SD card is FAT), also: no component ends in `.` or a space, and no component is a Windows reserved device name with or without an extension, any case (`con`, `prn`, `aux`, `nul`, `com0` to `com9`, `lpt0` to `lpt9`). The implementation reuses save-relay's existing reserved-name checks, which today are private helpers (`is_windows_reserved` in `src/manifest.rs` for game ids, `windows_reserved_name` in `src/ludusavi.rs` for path segments); the bridge shares one of them rather than adding a third. A violation is `manifest_invalid`. Paths are joined under the mapped root only after this check, and the result must still resolve inside the root.

### Verbs

| Verb | Auth | Purpose |
|---|---|---|
| `GET /v1/hello` | none | `{"proto_mask","bridge_id","limits":{...},"server_time_ms"}`, advisory only; the client refuses a bridge whose mask lacks version 1 (`unsupported_version`) |
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
| `POST /v1/titles/{tid}/ack` | write | the console reports it applied a snapshot, and how a conflict was resolved |

`{tid}` is the 16-hex-digit lowercase title id. Byte ranges are query parameters, so no `Range` header support is assumed. A console-loser upload (`stage` with `"purpose":"console_loser"`) exists in the design but is **disabled** until the owner rules (section 9, question 8).

### JSON shapes

`GET /v1/titles`:

```json
{"ok":true,"data":{"titles":[{"title_id":"0004000000abcd00","game_id":"example-game","name":"Example",
  "access":["read","write"],"max_total_bytes":4194304,
  "head":{"snapshot_id":"20261125T101500Z-0007","seq":7,"content_sha256":"<hex>"}}]}}
```

`plan` request, then response. The console's `last_synced` is authoritative about what the console last applied; the bridge validates it and does not substitute its own record:

```json
{"console":{"content_sha256":"<hex, or null when the archive is empty>","total_bytes":524288,
  "changed_seen_rtc_ms":3500000000000,"captured_rtc_ms":3500000100000,
  "last_synced":{"snapshot_id":"20261125T101500Z-0007","content_sha256":"<hex>"}}}
```
```json
{"ok":true,"data":{"action":"noop|push|pull|needs_choice","reason":"console_changed",
  "snapshot_id":"<id, for pull>",
  "info":{"host_changed":true,"console_changed":true,"host_azahar_running":false,
          "console_time":{"meaning":"first_seen_changed","rtc_ms":0},
          "host_time":{"meaning":"newest_file_mtime","unix_ms":0},
          "offset_s":-3600,"residual_s":2}}}
```

How the bridge reads it: `last_synced` is checked against the bridge's snapshot history for that title. If `snapshot_id` is unknown there, or its `content_sha256` differs, the baseline is untrusted and a both-sides-differ situation becomes `needs_choice` with reason `last_synced_unknown`; the bridge never trusts its own per-device `head.json` as the base. A missing `last_synced` counts as changed on both sides that have files. **Host changed** means the SHA of the *live Azahar folder*, re-hashed by the bridge now, differs from `last_synced.content_sha256`; `head.json` is a record, not the test, so a change made by playing in Azahar, or by a relay pull, is seen. `push`: the console is ahead, stage then commit. `pull`: the host is ahead; the bridge has just snapshotted the live folder as `snapshot_id` (read twice, hashed before and after; a mismatch means the folder is being written and the reply is `host_busy`), and the console downloads it. `needs_choice`: both changed (or the baseline is unknown); the console shows both sides with the labelled `info` times and the owner picks. A `conflict_auto` action (a clock-decided winner) is **not offered** until the owner decides what "newest" means (section 5, question 6). The names mirror save-relay's `Noop / Pull / Push / Conflict` plan; `Adopt` is folded into `noop` when both content hashes are equal. `plan` changes nothing.

`stage` request and response:

```json
{"base_snapshot_id":"<head the plan saw, or null>","manifest":{"...":"section 4"}}
{"ok":true,"data":{"stage_id":"<hex>","chunk_bytes":65536,"received":[0,0,0],"expires_in_s":900}}
```
A repeat of `stage` with the same `Idempotency-Key` and identical body returns the same `stage_id` and the current `received` array (bytes accepted per file, contiguous from 0): this is the resume mechanism.

`chunk?file=2&offset=65536`: the body is raw bytes with `X-SR-Body-SHA256`. `offset` must equal `received[file]`; otherwise the bridge answers 409 `stage_mismatch` with `data.received` so the client resumes from the truth. A good chunk returns `{"ok":true,"data":{"received":[...]}}`.

`verify` re-hashes every staged file against the manifest and returns `{"ok":true,"data":{"content_sha256":"<hex>","files_ok":3}}`; a mismatch is `manifest_invalid` naming the file index (never content) and drops the stage. `commit` takes `{"resolve":null|"keep_console"}` and returns:

```json
{"ok":true,"data":{"outcome":"committed|noop|conflict_resolved","snapshot_id":"<new head>","seq":8,
  "loser_snapshot_id":"<id or null>"}}
```
`ack` takes `{"snapshot_id","applied_content_sha256","resolution":null|"keep_host","console_loser_content_sha256":"<hex or null>"}` and returns `{"ok":true,"data":{}}`.

**Both-changed resolution flows.**

- `keep_console`: the console stages and verifies its files, then `commit` with `resolve:"keep_console"`. The bridge first stores the previous host copy as a host loser (section 5) and only then applies.
- `keep_host`: nothing is staged and the host copy is not touched. The console downloads the host snapshot named by `plan`, saves its own archive as a console loser on the SD card (section 5), applies, and calls `ack` with `resolution:"keep_host"` and the content hash of the archive it replaced. The bridge records the decision and marks that snapshot `pending_console_ack` until this call arrives.

### Errors

Wire envelope: `{"ok":false,"error":{"code":"<code>","message":"<short>","retryable":true|false}}`. It differs from the CLI's `--json` envelope (`src/api.rs`: `command` field, no `retryable`) and the success envelope here likewise carries no `command` field. A message never contains a key, MAC, token, save content or a host path outside the mapping. Codes reuse save-relay's vocabulary (`docs/code-map.md`, "Error codes") with the meanings below; where a name is shared, the meaning on this route is stated so nothing is silently redefined. The rows marked **new** are proposed additions (code-map allows adding, never renaming). They would be added to `Error::code` in `src/error.rs`, to the exit-code table in `docs/code-map.md` and `exit_code` in `src/api.rs` only when the bridge is built; this document changes none of them. Proposed exit-code groups: `scope_denied` and `device_revoked` with 4, `unsupported_version` and `payload_too_large` with 2, `stage_mismatch`, `host_busy` and `losers_full` with 5.

| Code | HTTP | Meaning on this route |
|---|---|---|
| `usage` | 400 | malformed request, concurrent request in a session, idempotency key reused with another body, bad `offset` or `len` |
| `manifest_invalid` | 422 | path rule, count, size or hash check failed |
| `game_unknown` | 404 | the title is in scope but has no reviewed mapping |
| `not_found`, `no_such_entry` | 404 | unknown stage or snapshot |
| `auth_failed` | 401 | bad MAC, expired session, replayed `seq`, or a spent pairing secret (same family as the CLI's code for a refused sign-in) |
| `not_set_up` | 409 | the bridge has no relay service configured |
| `reauth_needed`, `credentials` | 503 | the host's relay sign-in needs the owner at the host, not the console |
| `server_unreachable` | 502 | the bridge cannot reach the relay service; the console must not treat this as an offline win |
| `server_error` | 500 | bridge or provider fault |
| `state_invalid` | 500 | unchanged meaning: a state file or journal on the bridge is unreadable |
| `changed_on_server` | 409 | the head moved, or the live host folder no longer hashes as `plan` saw it; run `plan` again |
| `io` | 500 | host filesystem failure |
| `scope_denied` | 403 | **new**: title or access not in the device scope |
| `device_revoked` | 401 | **new** |
| `unsupported_version` | 400 | **new**: no common `proto` |
| `payload_too_large` | 413 | **new**: a limit above |
| `stage_mismatch` | 409 | **new**: the stage state or `offset` disagrees with the bridge; `data.received` carries the truth |
| `host_busy` | 409 | **new**: Azahar is running, or the folder changed while it was being read (section 5) |
| `losers_full` | 409 | **new**: the retained-loser cap would be exceeded (section 5) |

A conflict is not an error: it is the `plan` outcome `needs_choice`, as in save-relay where `conflict` changes nothing until the player chooses. Console retry rule: retry only on `retryable: true` or a timeout, with the same `Idempotency-Key` and a fresh `seq`, at most 3 attempts after 1, 3 and 9 s; never retry a bad MAC. Timeouts mirror `UreqTransport` (`src/http.rs`): connect 5 s, idle (no read or write progress) 60 s (`IDLE_TIMEOUT`), no total deadline.

## 4. Snapshot and mapping schema

### Snapshot manifest (format 1)

```json
{"format":1,"title_id":"0004000000abcd00","game_id":"example-game",
 "origin":"console","device_id":"<hex, or host>",
 "created":{"source":"console_rtc","unix_ms":0},
 "files":[{"path":"00000001.sav","size":524288,"sha256":"<64 hex>"}],
 "total_bytes":524288,"content_sha256":"<64 hex>"}
```
`files` are sorted bytewise by `path`. **`content_sha256` is computed from the files alone**, with the byte layout in section 2 ("content hash"): format, title id and the sorted (path, size, file hash) list. `device_id`, `origin`, `created`, `game_id` and `total_bytes` are not inputs, so the same files hash identically on the console and on the host whatever the clock or the device, and "equal hash is a noop" holds. (The hash is a byte layout, not canonical JSON, so no JSON canonicalization scheme is needed on the 3DS.) The recipient recomputes it and rejects a manifest whose stated value differs. The bridge stores each snapshot as `snapshots/<snapshot_id>/manifest.json` plus `files/`; `snapshot_id = <UTC stamp>-<seq, 4 digits>`.

### 3DS save archives (all **unverified**)

What a file is on the console: whatever the client enumerates in the title's save archive through libctru's FS (`FSUSER_OpenArchive`, `FSUSER_ControlArchive` for commit). Whether a homebrew app may open another title's save archive, and with what permissions on the pinned custom firmware, is a prototype gate. The manifest does not depend on the answer; an empty archive is an empty `files`.

- **Scope: SD-installed (digital) titles' save data only.** Extdata (extra data, DLC data), system saves and other archives are out of scope and are never read or written.
- **Anti-rollback (secure value).** Some titles store a counter outside the save files, read and written with `FSUSER_GetSaveDataSecureValue` and `FSUSER_SetSaveDataSecureValue`, and refuse a save restored without it. Whether the pinned custom firmware lets the client read and set it, and which titles use it, are prototype gates. Until verified, the console client refuses to apply a snapshot to a title whose secure value reads as set, and tells the owner why. This is stop rule 6, scoped to those titles; it does not stop the route for others.
- **Cartridge saves** live in the card's SPI archive, not the SD archive, with different access and size rules. They are **out of scope** for v1 until the owner decides (section 9, question 5); the mapping's `console_archive.kind` accepts only `savedata` on `media` `sd`, and `gamecard` is rejected at validation. If the owner brings them in, they are a separate prototype gate with their own tests.

### Title to relay game mapping (host file, owner-authored and reviewed)

```json
{"format":1,"titles":[{"title_id":"0004000000abcd00","game_id":"example-game","name":"Example",
  "console_archive":{"kind":"savedata","media":"sd"},"max_total_bytes":4194304}]}
```
The mapping does **not** repeat the Azahar path. The Azahar side of a mapped title is the save path in the title's relay manifest (the existing `saves` template and the `{home}`, `{data}`, `{config}`, `{documents}` variables resolved through `PathVars`, `docs/manifest.md`), so the relay and the bridge cannot disagree about where the host copy lives. Rules: each `title_id` and each `game_id` appears once; `game_id` must equal the relay manifest `id` and satisfy save-relay's game id rules (`src/manifest.rs`); the title's relay manifest must exist and resolve a folder on the bridge host; no two mapped folders overlap; no path is derived from a title string or from anything the console sends: the console sends only a `title_id`, and an unmapped (in-scope) one is `game_unknown`. `bridge mapping check` verifies the agreement: the relay manifest's id equals `game_id`, the resolved folder exists and is a directory, and, when its path contains the 3DS `title/<high>/<low>` segments, they equal the high and low 32 bits of `title_id`.

**Manifest shape for a mapped title (illustrative, owner-reviewed, no real ids).** The relay manifest follows the Azahar folder and nothing on the 3DS:

```json
{"format":1,"id":"example-game","name":"Example",
 "saves":{"windows":"{data}/<azahar data dir>/sdmc/Nintendo 3DS/<id0>/<id1>/title/00040000/00abcd00/data/00000001",
          "linux":"{data}/<azahar data dir>/sdmc/Nintendo 3DS/<id0>/<id1>/title/00040000/00abcd00/data/00000001"},
 "include":["**"]}
```
`<azahar data dir>`, `<id0>` and `<id1>` (the per-console and per-profile hash folders) are filled in by the owner from a real Azahar install; the layout (Citra lineage) and the platform folders for `{data}` are **unverified** for the pinned Azahar, as is whether Azahar picks up files written there without a restart. This is the "relay manifest says which save path each side follows" of #300: the Azahar side is the manifest's `saves`; the console side is the mapping's `console_archive` plus the title id, and `bridge mapping check` keeps the two consistent. Adding a 3DS entry to the manifest format itself is not proposed here. No game or save content is committed; fixtures are synthetic.

Relay side: each mapped title is one save-relay game. The bridge treats the Azahar directory as the local folder and uses the existing engine (pull, push, conflict, history) against the relay service, under the same per-title lock as console commits, and only while Azahar is not running (section 5). The console is a second peer of the bridge: its snapshots reach the Azahar directory by the commit protocol in section 5, then go up by the normal engine path. A relay conflict with another device follows save-relay's ordinary policy; this route's rules apply only between console and host copy.

## 5. Ordering, conflicts and the commit record

### Detect change first, compare clocks only when needed

The console keeps `last_synced = {snapshot_id, content_sha256}` in `sdmc:/3ds/save-relay/state.json`, and sends it with `plan`; it is the authority on what the console last applied, validated by the bridge as described in section 3. `console_changed`: the content hash of the console files differs from `last_synced`. `host_changed`: the content hash of the live Azahar folder, re-hashed at `plan` and again at `commit`, differs from it. Only one changed: that side wins and no clock is involved (a snapshot of the winner is taken first, and the other side's state is backed up as in "Loser retention"). Both changed with equal content hashes: `noop`, record `last_synced`. Both changed and different, or the baseline unknown, or no `last_synced` with files on both sides: `needs_choice`. An empty side with no change evidence loses without a clock, still with a backup.

### Time values (recorded for the owner, not used to decide in v1)

| Side | Value | Notes |
|---|---|---|
| Host copy | newest regular-file mtime in the Azahar folder, labelled `newest_file_mtime`, else the relay head time | mtimes come from whichever machine wrote them, including other machines' clocks after a relay pull; the host clock is assumed NTP-synced and the bridge status shows whether it is. It means "when some machine's clock said the files were written", not a trusted time |
| Console | `changed_seen_rtc_ms`: the console RTC when the client first saw its archive hash differ from `last_synced`, persisted in `state.json` and not updated again until the hash changes again; `captured_rtc_ms` is the RTC at this capture and is labelled as such | the 3DS RTC is local time with no zone, and anyone holding the console can set it (**unverified**: `osGetTime` meaning and precision); the archive may expose no per-file time (**unverified**). The first-seen time is a proxy for "when the game saved", at best bounded above by the time the owner next ran the client |

Offset: the session request carries `rtc_ms`; the bridge computes `d = server_time - rtc` and sets `offset_s = round(d to the nearest 900 s)` (a quarter hour absorbs the zone) with residual `r = d - offset_s`. It is measured every session and never stored, so a daylight-saving step between sessions is absorbed. `info` in a `plan` reply reports `offset_s` and `residual_s`.

**Mtime on commit.** When a console snapshot is written into the Azahar folder, each file's mtime is set to the bridge's host time at the moment of the swap and recorded in `head.json` as `applied_at_ms`. That time means "when the bridge applied it", not when the game saved; it is shown with that label and never used to decide a winner.

### Rule (this route only)

**Default until the owner rules (question 6): when both sides changed, the answer is always `needs_choice`.** The owner resolves it on the console UI (`keep_console` or `keep_host`, flows in section 3), and the loser is kept in both cases.

**Candidate rule A, off by default** (enabled only by an explicit owner decision): newest wins with a reliable clock. 1. `|r| > 60 s`, or `|offset_s| > 26 h`: `needs_choice`, reason `clock_unreliable`. 2. With `t_c = changed_seen_rtc + offset_s` and `t_h` the host time: `|t_c - t_h| <= tolerance` (default 300 s, configurable 60 to 3600 s) is a tie: `needs_choice`, reason `tie`. 3. Else the later one wins and the loser is retained, and `plan` would then also offer `conflict_auto`. Its fragility is the point of the question: both inputs are clocks that other machines or the owner can set, and the console value is a proxy.

### Loser retention and naming

Before either side is overwritten in a both-changed resolution, the side being replaced is retained:

- **Host loser**: the previous host copy is stored as bridge snapshot `<stamp>-<seq>-loser` with the `.save-relay-conflict-loser` marker (the name the relay engine already uses, `src/sync.rs`; the bridge store mirrors it) and enters the relay history through the normal engine, so `save-relay restore` can bring it back. The relay history keeps its own count-based retention (`retention::KEEP = 10`, with the marker exempting a loser); the bridge store follows the same shape.
- **Console loser**: the archive about to be overwritten is first copied to `sdmc:/3ds/save-relay/backup/<tid>/<stamp>/` on the SD card, verified by re-hash. The bridge never deletes console backups. **The SD copy alone is not durable** (a stolen, failed or reformatted card loses it). The spec therefore also describes uploading the console loser to the bridge (and so to relay history) before overwriting; that upload stays **disabled** until the owner decides whether the SD backup is enough (section 9, question 8). With it disabled, the SD copy is the only copy of the console loser and the console says so on the confirmation screen.
- **Cap, never silent pruning.** Losers are never pruned to make room. Per title, the bridge counts retained losers and their bytes against a cap (proposed placeholders: 20 losers and 128 MiB per title, owner question 9). When a resolution would exceed it the bridge refuses the commit with `losers_full` and the owner clears old losers on the host by hand. No loser is deleted without the owner doing it.
- Snapshots that something still points at are exempt from ordinary retention: the one a pending (un-acked) pull names, and the one most recently reported as `last_synced` by each active device. They stop being exempt once that device acks or reports a newer one.

### Crash-safe commit record (console to host)

Order is a per-title integer `seq` in `head.json`, never a timestamp, so two commits cannot tie. A title is committed only while it holds the per-title lock and **Azahar is not running** (the bridge checks for the configured Azahar process; if it cannot tell, it refuses; `host_busy`; owner question 10). `commit` also re-hashes the live Azahar folder first and refuses with `changed_on_server` if it no longer equals the hash `plan` saw (a change made by playing, by a relay pull, or by hand). The journal `journal/<tid>.json` is replaced atomically (write temp, fsync, rename, fsync the directory) **before** each step it names, so a crash always leaves a record of what was about to happen:

```json
{"format":1,"title_id":"...","device_id":"...","seq":8,"op":"console_to_host",
 "state":"staged|verified|loser_saved|applying|applied|published|complete",
 "from_snapshot":"...","to_snapshot":"...","loser_snapshot":"... or null",
 "paths":{"live":"<folder>","new":"<folder>.sr-new","old":"<folder>.sr-old"},
 "decision":{"rule":"owner_choice|one_side_changed","winner":"console"}}
```

Steps: `staged` (all chunks present) -> `verified` (re-hash equals manifest) -> `loser_saved` (previous host copy stored, only if both changed) -> `applying` -> `applied` -> `published` (`head.json` written with the new `seq`, `last_synced` updated) -> `complete` (relay push attempted; a relay failure is reported later like save-relay's `deferred` and is not a failed commit).

**The swap** (state `applying`; the journal says `applying` *before* the first rename). The new tree is first written and fsynced into `<folder>.sr-new`, a sibling in the same parent directory (same volume; Azahar ignoring unknown sibling names is **unverified**). Then: (1) rename `live` to `old`; (2) rename `new` to `live`; (3) journal `applied`; (4) delete `old` only after `published`. The bridge never renames onto an existing directory, since Windows cannot replace a directory by rename and POSIX replacement is not what is wanted here. On Windows a directory with any open handle (Azahar, an indexer, a virus scanner) refuses the rename: the step is retried up to 3 times 1 s apart and then fails with `io`, leaving the state recoverable as below.

**Recovery at bridge start**, idempotent, driven by the journal state and which of `live`, `new`, `old` exist:

| State | Directories | Action |
|---|---|---|
| `staged`, `verified` | any | discard the stage |
| `loser_saved` | `live` only | live untouched; keep the stored loser; drop the journal |
| `applying` | `live` and `new`, no `old` | step 1 never ran; delete `new`, as `loser_saved` |
| `applying` | `new` and `old`, no `live` | crashed between renames; if `new` hashes to `to_snapshot`, rename `new` to `live` (roll forward), else rename `old` to `live` (roll back) |
| `applying` or `applied` | `live` and `old`, no `new` | swap done; if `live` hashes to `to_snapshot`, continue to `published`, else rename `old` back (roll back) |
| anything else | any | stop, mark the title `needs_attention`, delete nothing, report `io`; the owner decides |
| `published` and later | any | finish: write `complete`, delete `old` |

Replaying `commit` with the same `Idempotency-Key` after a crash returns the recorded result.

### Host to console

The console downloads into `sdmc:/3ds/save-relay/stage/<tid>/`, verifies hashes, saves its backup (as a console loser when both changed), writes the files into the save archive, commits the archive with `FSUSER_ControlArchive`, re-reads and hashes, writes `last_synced`, and only then calls `ack`. It keeps its own journal (`staged`, `verified`, `backed_up`, `written`, `committed`, `acked`), written before each step. The bridge counts a pull as finished only on `ack`; an un-acked pull shows as `pending_console_ack` and is offered again next session. Whether the archive commit is atomic, and whether power loss before it leaves the old archive intact, is **unverified**: the prototype cuts power at each console step. Do not assume desktop rename semantics.

## 6. Threat model

| Threat | Capability | Mitigation | Residual risk |
|---|---|---|---|
| LAN attacker, passive | sees traffic | TLS to a private CA; no plaintext route | packet sizes and timing |
| LAN attacker, fake bridge | spoofs DNS or IP, serves a certificate | the paired CA is the only trusted root (stop rule 1) or SPKI pin; session replies and every in-session reply carry a MAC it cannot form; unauthenticated errors are never acted on | relies on unverified console chain validation |
| LAN attacker, fake console | connects to the bridge | nothing without a pairing or session MAC; pairing only in a 5-minute host-opened window with a 128-bit single-use secret and 5 attempts; per-IP and per-device limits | an attacker who also captures the bundle (shoulder-surf, QR photo) in the window must still pass the owner's short-code confirmation on the host |
| Stolen console or SD card | reads `device.bin` | scoped to named titles; revoke at the host; no provider credential on the console; on suspicion revoke and re-pair (rotation would let the thief race it) | the thief can read and write those titles until revoked; every overwrite keeps a loser or earlier snapshot (a console loser on the SD card goes with the card unless upload is enabled), so data is recoverable but not private; no PIN in v1 |
| Malicious bridge peer | a rogue LAN host answering at the bridge address, or a process on the host | CA key and device keys are host-user-only; the console checks pin and MAC; the bridge binds only the configured interface | a process running as the host user can read everything: out of scope |
| Replay | resends captured requests | TLS; per-session increasing `seq`; session expiry; MAC over method, path, seq, body hash and idempotency key; idempotency keys only return the recorded result | none beyond a TLS break plus session key theft |
| Path escape, hostile manifest | `..`, absolute paths, case collisions, reserved names, huge counts | path rule and limits; mapped roots only; no symlinks | implementation bugs, covered by synthetic tests |
| Resource exhaustion | many stages, big bodies | limits, quotas, stage TTL, loser cap | LAN denial of service stays possible |
| Wrong title or mixed saves | bad mapping | reviewed mapping checked against the relay manifest; console sends only a `tid`; scope, `base_snapshot_id` and head checks | owner mapping error |
| Bad or manipulated clocks | wrong winner | change detection first; v1 asks the owner whenever both changed; times are labelled and recorded, not trusted | host mtimes are written by other machines' clocks (after a relay pull, an old or skewed machine can set them), and whoever holds the console can set its RTC to look newer or older; the owner picks wrong; the loser is kept |
| Host folder written while a commit runs | Azahar or a relay pull modifies the folder | refuse while Azahar runs; re-hash before commit; per-title lock | the Azahar process check is **unverified** and fails closed |
| Secrets in logs | | `Authorization` and MAC headers redacted as `http::Request`'s `Debug` does; messages carry no key, path or save content | |

## 7. Acceptance split

**Host-side automated tests (pure Rust bridge logic, synthetic fixtures, loopback only, no console, no real saves):**

- Manifest parse, content hash (the same files from different `device_id`, `origin` and `created` hash equal), path rules (traversal, absolute, backslash, case-fold collision, over-long, NUL, symlink, trailing dot or space, reserved device names, `:`), every limit, `len` and `offset` bounds.
- All wire byte encodings against the shared vector file (section 2), including the negative vectors; pairing derivation and short code; window expiry, attempt limit, single use, two valid proofs in one window; scopes and the check order (out-of-scope mapped and unmapped titles give the same code); revoke; rotate with a crash at each step; session `seq` replay, gaps and expiry; serialized requests; bad MAC; session reply and response MAC; unauthenticated error reply ignored; idempotency scope per (device, title, verb).
- `plan` decision table: one side changed (both directions), equal hashes, first sync, empty side, unknown `last_synced`, both changed gives `needs_choice`, 3600 s DST step and large residual in `info`, owner choice for both `keep_console` and `keep_host`; the live folder changed by hand after `plan` (`changed_on_server`); Azahar running (`host_busy`); candidate rule A tests only if enabled.
- Stage, chunk, resume at each offset, wrong offset (`stage_mismatch` with `received`), idempotent replays, verify mismatch, abort, TTL.
- Commit journal with crash injection before and after every state and every rename on a fake filesystem, each recovery row above, a Windows-style refusal to rename a held directory, losers retained and exempt from retention, the loser cap (`losers_full`), pinned snapshots, `ack` and `pending_console_ack`.
- Mapping validation (overlap, duplicates, missing directory, disagreement with the relay manifest, `gamecard` rejected) and the error-code table.
- Wire fixtures for every JSON shape, shared with the console client's parser so both sides read the same files.
- Bridge-side TLS only: a loopback server with a generated test CA accepts a client trusting that CA and refuses one that does not; the bridge's offered suites are exactly the list above.

**Only a real 3DS plus Azahar round trip proves (#300):** that httpc or the fallback stack negotiates with the bridge on the pinned firmware and one of the offered ephemeral suites; that the paired root is the only trusted root and a real public-CA certificate is refused; name or IP validation and clock behavior; client certs (optional); sustained 64 KiB chunks; QR scan or manual code entry; RTC meaning, zone and precision; whether the title's save archive can be opened, enumerated, written and committed, whether power loss before commit is safe, and how the secure value behaves; the real file set and sizes of a title; Azahar reading the written folder and the game loading it, both directions, and the Azahar process check; both-changed resolution and loser recovery on real data; host-side revocation of a real console. Record versions and outcomes without publishing save contents or credentials.

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
| Bridge Rust toolchain and server TLS crate (and its supported suites) | to pin | |
| Licenses of every vendored piece linked into the console client | to confirm | |

## 9. Open owner decisions

None of these is decided by this document. Each shows the safe default the spec holds until the owner answers.

| # | Question | Default held until answered |
|---|---|---|
| 1 | Is a 5-minute pairing window acceptable, and is typing the secret on the console acceptable if QR is not feasible? | 5-minute window; typed secret listed as the last-resort delivery (section 2) |
| 2 | Is the default total snapshot cap of 4 MiB (hard maximum 16 MiB) acceptable? | 4 MiB default, 16 MiB maximum (section 3) |
| 3 | Is re-pairing on CA rotation acceptable, with the device key on the SD card without a PIN? | re-pair on CA rotation; no PIN in v1 (sections 2 and 6) |
| 4 | Accept an application-layer HMAC in place of the mutually authenticated HTTPS proposed on #300, with mTLS only as optional hardening? | HMAC baseline; mTLS optional hardening only (section 1) |
| 5 | Cartridge saves: in scope (the card's SPI archive), or digital SD titles only for v1? | SD-installed titles only; `gamecard` rejected (section 4) |
| 6 | What does "newest" mean on the console without a save-time source: a recorded first-seen-changed time (candidate rule A), or always ask when both changed? | always `needs_choice` when both changed; the loser is kept (section 5) |
| 7 | If sslc cannot negotiate an ephemeral suite, may the bridge use a static-RSA (no forward secrecy) or non-rustls TLS profile, an exception to the no-OpenSSL rule? | no: stop rule 2 applies and the route is revised (section 1) |
| 8 | Must a console loser also go up to the bridge and relay history, or is the SD-card backup enough? | keep the loser on the SD card, upload disabled and flagged pending this decision (section 5) |
| 9 | A per-title cap on retained losers, or never prune? | never prune; placeholder cap of 20 losers and 128 MiB per title that refuses new losers instead of pruning (section 5) |
| 10 | May the bridge write to the Azahar folder while Azahar is running, or must it refuse? | refuse (`host_busy`); fails closed if the process check cannot tell (section 5) |
