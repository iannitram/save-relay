# save-relay

Share game saves between my machines through the service I pick once at setup (my own Nextcloud, Google Drive or Dropbox): the launcher pulls the newest saves before a game starts and pushes them after it quits. Built once, reused by every game port I maintain.

For Steam games without Steam Cloud, `save-relay steam-games` shows the store state and whether save-relay knows a saves folder. `save-relay install-wrap --no-steam-cloud` adds only games with a definite no and a known saves folder; unknown stays unknown. The panel also has an Add games view. See [`docs/wrap.md`](docs/wrap.md) and [`docs/panel.md`](docs/panel.md) for the browser and Game Mode flows. The Game Mode Steam-client callback is covered by automated tests and still needs a handheld check.

**Stage:** see `PIPELINE.md`.

## Install

Download the binary for your system and `SHA256SUMS` from the [releases](https://github.com/iannitram/save-relay/releases) page, and check the file before use. On Linux, run `sha256sum -c --ignore-missing SHA256SUMS` in the folder that holds the download and `SHA256SUMS`. Then run `chmod +x save-relay-<version>-x86_64-unknown-linux-musl` before you run it. On Windows, run `Get-FileHash <file> -Algorithm SHA256` and compare the hash with the matching line in `SHA256SUMS`. The Linux binary is x86_64 and static, so it needs no system libraries; the Windows binary is x86_64. macOS is a Should for 1.0 (my ruling of 2026-11-12): the release workflow builds a Mac binary and signs and notarizes it when the Apple secrets are set, but the Mac build is not yet supported. Progress is in #238.

## Start here

- `IDEA.md`: the idea, in my words.
- `SPEC.md`: what it must do, what it won't do, and what 1.0 means.
- `ROADMAP.md` and `docs/code-map.md`: the plan and how the code is laid out; `docs/adr/`: the decisions behind it.
- `AGENTS.md`: rules for anyone, person or agent, changing the code.
- `CHANGELOG.md`: what changed and when.

## License

MIT.
