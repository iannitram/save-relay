# save-relay

Share game saves between my machines through the service I pick once at setup (my own Nextcloud, Google Drive or Dropbox): the launcher pulls the newest saves before a game starts and pushes them after it quits. Built once, reused by every game port I maintain.

**Stage:** see `PIPELINE.md`.

## Install

Download the binary for your system and `SHA256SUMS` from the [releases](https://github.com/iannitram/save-relay/releases) page, and check the file with `sha256sum -c SHA256SUMS` (Linux) or `Get-FileHash` (Windows) before use. The Linux binary is x86_64 and static, so it needs no system libraries; the Windows binary is x86_64. The macOS build is unsigned and not supported for 1.0.

## Start here

- `IDEA.md`: the idea, in my words.
- `SPEC.md`: what it must do, what it won't do, and what 1.0 means.
- `ROADMAP.md` and `docs/code-map.md`: the plan and how the code is laid out; `docs/adr/`: the decisions behind it.
- `AGENTS.md`: rules for anyone, person or agent, changing the code.
- `CHANGELOG.md`: what changed and when.

## License

MIT.
