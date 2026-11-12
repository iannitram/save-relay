Stage: Dev Loop
Since: 2026-10-31
Kind: own
Spec: SPEC.md   Roadmap: ROADMAP.md   Code map: docs/code-map.md
Platforms: linux, windows (1.0); the mac build and the android and ios bindings are a Should for 1.0 (rulings 2026-11-12)
Peers reviewed: 2026-10-31   (the list is in SPEC.md, Prior art)
Notes: 2026-11-11: every build issue of stages 1-7 is closed (F01-F25, F27-F45, F48-F66, F69). M16 implementation through F78 (#213) is assembled in the stacked pull requests; Game Mode handheld acceptance remains pending. An audit of main at `a8e07ff` found six additional 1.0 blockers, B1-B6, now listed ahead of `rc.1` in ROADMAP.md (`docs/audit-2026-11-11.md`). Spec changed 2026-11-10: M16, Steam games without Steam Cloud (ADR 0004), is in 1.0. The owner's #36 can proceed in parallel; #73 (the Game Mode re-check) follows handheld acceptance for #212; #205 (F71, the first release candidate) waits on #36, #213 and B1-B6. Then the owner's #47, #72 and #26. #49 (F70) is after-1.0. Design pass: findings #164-#179, #197, #198, #201, #202 are fixed; the pass is recorded here when #73 closes. History of how the issues were filed: CHANGELOG.md.

Stages: Proposal -> Spec -> Roadmap -> Dev Loop -> Prototype / Beta.
What each stage means and who moves a project: `PIPELINE.md` in the pipeline repo (private).
The Project Manager updates this file when the stage changes and adds a CHANGELOG line.
