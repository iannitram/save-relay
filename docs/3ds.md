# 3DS and Azahar: proposed transport design

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
