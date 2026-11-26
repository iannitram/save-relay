Stage: Dev Loop
Since: 2026-10-31
Kind: own
Spec: SPEC.md   Roadmap: ROADMAP.md   Code map: docs/code-map.md
Platforms: linux, windows (unsigned by owner decision for 1.0, Q6), mac (Apple Silicon CLI only, SPEC M17 as narrowed 2026-11-25), ios (M18) and android (M20, a Must by owner ruling 2026-11-25, Q2). The SPEC changes of 2026-11-25 wait for the owner's approval line.
Peers reviewed: 2026-10-31   (the list is in SPEC.md, Prior art)
Notes: Audited 2026-11-25 at main `bc3a7b1` (#331 merged; its 11 checks were green on the PR head). Stage remains Dev Loop: rc.2 is published, including the iOS library; the game-file installer (SPEC M19) is implemented and fake-tested on Dropbox, Nextcloud and Google Drive, not released and not device-verified. SPEC was amended 2026-11-25 (M17 narrowed, M19, M20) and waits for the owner's approval line. Order and evidence: ROADMAP.md; source ownership: docs/code-map.md. Save Relay syncs saves and supplies resumable private game-file chunks for Android and iOS thin apps; not an app store; launcher changes out of scope; ACNH is the consumer reference. Open agent work: P2 and P2a (installer contract, iOS verify-and-publish entry point), #328 publisher (P13), P5 release gates including the Android asset, then rc.3. Acceptance gates: Mac #287/#238, iPhone #289, providers #47, handheld #212/#73/#72, installer device proof #309/#310 (G6), final #26. Stages 1-7, M16 and earlier hardening stay completed.

Stages: Proposal -> Spec -> Roadmap -> Dev Loop -> Prototype / Beta.
What each stage means and who moves a project: `PIPELINE.md` in the pipeline repo (private).
The Project Manager updates this file when the stage changes and adds a CHANGELOG line.
