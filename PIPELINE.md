Stage: Dev Loop
Since: 2026-10-31
Kind: own
Spec: SPEC.md   Roadmap: ROADMAP.md   Code map: docs/code-map.md
Platforms: linux, windows (1.0); the mac build and the android and ios bindings are a Should for 1.0 (rulings 2026-11-12)
Peers reviewed: 2026-10-31   (the list is in SPEC.md, Prior art)
Notes: 2026-11-12: every build issue of stages 1-7 is closed (F01-F25, F27-F45, F48-F66, F69), M16 (F72-F78) is merged, and the six audit blockers B1-B6 (#223-#228) are merged and closed; main is at `79ff440`. #236 (rc.1 packaging) is merged (version `1.0.0-rc.1`; no tag yet). #212 (F77) is merged and waits on the SteamOS handheld check, then #73 (the Game Mode re-check). The audit's should-fix items and Minor list are issues F79-F87 (#242-#250, milestone `1.0`). #205 (F71, the first release candidate) waits on #36; then the owner's #47, #72 and #26. Spec changed 2026-11-10: M16, Steam games without Steam Cloud (ADR 0004), is in 1.0. #49 (F70) is in 1.0 (ruling 2026-11-12), with the bindings (#254) and the signed Mac build (#238) as Shoulds for 1.0. Design pass: findings #164-#179, #197, #198, #201, #202 are fixed; the pass is recorded here when #73 closes. History of how the issues were filed: CHANGELOG.md.

Stages: Proposal -> Spec -> Roadmap -> Dev Loop -> Prototype / Beta.
What each stage means and who moves a project: `PIPELINE.md` in the pipeline repo (private).
The Project Manager updates this file when the stage changes and adds a CHANGELOG line.
