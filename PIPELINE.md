Stage: Dev Loop
Since: 2026-10-31
Kind: own
Spec: SPEC.md   Roadmap: ROADMAP.md   Code map: docs/code-map.md
Platforms: linux, windows (1.0); mac, android, ios with the game ports' 2.0
Peers reviewed: 2026-10-31   (the list is in SPEC.md, Prior art)
Notes: Issues #1-#26 cover the code map (F01-F26 in ROADMAP.md). Free to take now: Easy #1, #2, #4, #5, #7, #9, #13, #14, #15; Medium #10, #12, #16. The rest wait on their "Blocked by". #26 is the 1.0 check on the three PCs (needs-owner-device), not a build job. Design pass: skipped, no UI (ROADMAP.md stage 8). 2026-10-31 spec re-approved (save service chosen at setup: Nextcloud, Google Drive or Dropbox; M6, M9, M10): ADR 0002, skeleton modules http, service, oauth, clients, dropbox, gdrive (+ guard), fake; issues #27-#47 (F27-F47, ROADMAP stage 6). Free to take now: Easy #27-#34 (pure-fn ones are Strix Halo jobs: pure-fn is unfit on the RTX 3090 Ti desktop), Medium #37, #39, #40. #35 waits on #10, #46 on #35. #36 (register the app) is the owner's clicks; #47 the owner's real round trips. #17 now also waits on #37 (the transport). Design pass: still skipped, stage 8. 2026-11-01 spec re-approved (M11-M15: the open-elsewhere mark, `wrap` for any Steam game with the ludusavi data, save cards, the panel in Game Mode and the browser): ADR 0003, skeleton modules presence, steam (+ vdf, launch), ludusavi, pending, wrap, card, panel, scaffolds `panel/` and `decky/` with a CI front-end job; issues #53-#73 (F48-F68, ROADMAP stage 7; #48 and #49 are TotK's F69 and F70, #50-#52 satellite PRs). Free to take now: Medium #58 (VDF); Hard #67 (the front end, on fixtures). Easy #53 and #65 wait on #2, Easy #54 on #3, #7 and #9. The rest wait on their "Blocked by". #72 is the owner's device checks. F69 (#48) is in the roadmap; F70 (#49) is after-1.0. Design pass: no longer skipped, stage 8 (#73, the panel).

Stages: Proposal -> Spec -> Roadmap -> Dev Loop -> Prototype / Beta.
What each stage means and who moves a project: `PIPELINE.md` in the pipeline repo (private).
The Project Manager updates this file when the stage changes and adds a CHANGELOG line.
