"""The Save Relay Decky plugin's backend (SPEC.md M15).

Decky Loader runs this file in its Python host. ``Plugin.run`` is the one method the front end
calls (``callable("run")`` in src/index.tsx): it runs ``save-relay <command> ... --json`` with
arguments built from the same JSON object the desktop panel posts (src/panel.rs ``api_args``)
and returns the one envelope line it prints. The binary is the one ``install-wrap`` put in
place, found on PATH or beside this file. Skeleton: the methods are built by their issue.
"""

import decky  # noqa: F401  (supplied by Decky Loader at run time)


class Plugin:
    async def run(self, command: str, args: dict) -> str:
        """Run one save-relay command with ``--json`` and return its envelope line.

        ``command`` must be one of src/panel.rs ``API_COMMANDS``; anything else returns a
        ``usage`` envelope without running anything.
        """
        raise NotImplementedError("not built yet: decky main.Plugin.run")

    async def _main(self) -> None:
        """Called once when the plugin loads."""

    async def _unload(self) -> None:
        """Called once when the plugin unloads."""
