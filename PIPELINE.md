Stage: Dev Loop
Since: 2026-10-31
Kind: own
Spec: SPEC.md   Roadmap: ROADMAP.md   Code map: docs/code-map.md
Platforms: linux, windows (1.0); mac, android, ios with the game ports' 2.0
Peers reviewed: 2026-10-31   (the list is in SPEC.md, Prior art)
Notes: 2026-11-11: every build issue of stages 1-7 is closed (F01-F25, F27-F45, F48-F66, F69). Spec changed 2026-11-10: M16, Steam games without Steam Cloud (ADR 0004), in 1.0. What is left, in order, is ROADMAP.md "Run order to 1.0". The baseline audit used main at `a8e07ff`; the current implementation tip is M16 through #213 (F78), with Game Mode handheld acceptance pending. The #73 handheld re-check follows #212; #36 can proceed in parallel. #205 (F71, the first release candidate) waits on #36 and #213. Then the owner's #47, #72 and #26. #49 (F70) is after-1.0. Design pass: findings #164-#179, #197, #198, #201, #202 are fixed; the pass is recorded here when #73 closes. History of how the issues were filed: CHANGELOG.md.

Stages: Proposal -> Spec -> Roadmap -> Dev Loop -> Prototype / Beta.
What each stage means and who moves a project: `PIPELINE.md` in the pipeline repo (private).
The Project Manager updates this file when the stage changes and adds a CHANGELOG line.
