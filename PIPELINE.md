Stage: Dev Loop
Since: 2026-10-31
Kind: own
Spec: SPEC.md   Roadmap: ROADMAP.md   Code map: docs/code-map.md
Platforms: linux, windows (unsigned by owner decision for 1.0, Q6), mac (Apple Silicon CLI only, SPEC M17 as narrowed 2026-11-25), ios (M18) and android (M20, a Must by owner ruling 2026-11-25, Q2). The SPEC changes of 2026-11-25 wait for the owner's approval line.
Peers reviewed: 2026-10-31   (the list is in SPEC.md, Prior art)
Notes: Audited 2026-11-24 at main `7bfe1df`, all 11 commit checks successful. Stage remains Dev Loop: rc.2 is published, including the iOS library, but live service/device/design acceptance is incomplete. Current issue order and evidence are in ROADMAP.md, source ownership in docs/code-map.md. Owner direction 2026-11-24: shared resumable Android/iOS thin-app chunks through Dropbox, Nextcloud and Google Drive alongside save sync; not an app store; launcher changes out of scope. ACNH is the consumer reference. #305 -> #309 -> #310 track design/core/interfaces; #301/#302/#303/#306 track integration; #311 tracks audit defects. Mac signing/round trip #287/#238, iPhone #289, provider matrix #47, handheld #212/#73/#72 and final #26 remain acceptance gates. Stages 1-7, M16 and earlier hardening implementations stay completed; see roadmap for reconciliation of stale checklist issues.

Stages: Proposal -> Spec -> Roadmap -> Dev Loop -> Prototype / Beta.
What each stage means and who moves a project: `PIPELINE.md` in the pipeline repo (private).
The Project Manager updates this file when the stage changes and adds a CHANGELOG line.
