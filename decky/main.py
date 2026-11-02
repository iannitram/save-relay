"""Decky host adapter for the CLI's JSON contract; no shell or service credentials here."""

from __future__ import annotations

import asyncio
import base64
import json
import os
from pathlib import Path
import shutil
import tempfile

import decky  # supplied by Decky Loader

TIMEOUT = 60
PLUGIN_DIR = Path(__file__).resolve().parent
API_COMMANDS = {
    "status", "history", "restore", "pull", "push", "answer", "where", "which",
    "picture", "setup",
}


def failure(command, code, message):
    return json.dumps({"ok": False, "command": command,
                       "error": {"code": code, "message": message}})


def api_args(command, args, picture_out):
    """Mirror panel::api_args: required positionals, allowed flags, --json last."""
    if command not in API_COMMANDS or not isinstance(args, dict):
        raise ValueError("Unknown command or invalid arguments")
    positionals = {
        "where": (), "setup": (), "which": ("appid",),
        "restore": ("game", "entry"), "picture": ("game", "entry"),
        "answer": ("game", "answer"),
    }.get(command, ("game",))
    flags = {
        "status": ("manifest", "saves-dir"),
        "restore": ("manifest", "saves-dir"),
        "pull": ("manifest", "saves-dir", "resolve", "anyway"),
        "push": ("manifest", "saves-dir", "resolve"),
        "setup": ("service", "server", "user", "device", "root", "no-browser", "finish"),
    }.get(command, ())
    if set(args) - set(positionals) - set(flags):
        raise ValueError("Unexpected command argument")
    result = [command]
    for key in positionals:
        value = args.get(key)
        if key == "appid" and type(value) is int and 0 <= value <= 4294967295:
            value = str(value)
        if not isinstance(value, str) or not value or value.startswith("-") or "\0" in value:
            raise ValueError(f"Missing or invalid {key}")
        result.append(value)
    for key in flags:
        if key not in args:
            continue
        value = args[key]
        if key in {"anyway", "no-browser", "finish"}:
            if type(value) is not bool:
                raise ValueError(f"{key} must be a boolean")
            if value:
                result.append(f"--{key}")
        else:
            if not isinstance(value, str) or not value or value.startswith("-") or "\0" in value:
                raise ValueError(f"Invalid {key}")
            result.extend([f"--{key}", value])
    if command == "setup" and "service" not in args:
        raise ValueError("setup requires service")
    if command == "picture":
        result.extend(["--out", str(picture_out)])
    return result + ["--json"]


class Plugin:
    async def _run(self, command, args, picture_out):
        try:
            argv = api_args(command, args, picture_out)
        except (ValueError, TypeError):
            return failure(command, "usage", "Invalid command arguments")
        bundled = PLUGIN_DIR / "bin" / "save-relay"
        binary = str(bundled) if os.access(bundled, os.X_OK) else shutil.which("save-relay")
        if binary is None:
            return failure(command, "io", "save-relay binary not found")
        process = None
        try:
            process = await asyncio.create_subprocess_exec(
                binary, *argv, stdin=asyncio.subprocess.DEVNULL,
                stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.DEVNULL,
            )
            try:
                stdout, _ = await asyncio.wait_for(process.communicate(), TIMEOUT)
            except (asyncio.TimeoutError, asyncio.CancelledError):
                if process.returncode is None:
                    try:
                        process.kill()
                    except ProcessLookupError:
                        pass
                await process.communicate()
                raise
            line = stdout.decode("utf-8")
            envelope = json.loads(line)
            if (not isinstance(envelope, dict)
                    or type(envelope.get("ok")) is not bool
                    or envelope.get("command") != command
                    or (envelope["ok"] and "data" not in envelope)
                    or (not envelope["ok"] and (
                        not isinstance(envelope.get("error"), dict)
                        or not isinstance(envelope["error"].get("code"), str)
                        or not isinstance(envelope["error"].get("message"), str)))):
                raise ValueError("Invalid envelope")
            # Preserve CLI errors and conflict/held successes, regardless of exit status.
            return line
        except asyncio.TimeoutError:
            return failure(command, "io", "save-relay timed out")
        except (OSError, ValueError, UnicodeError):
            return failure(command, "io", "save-relay could not run or returned invalid JSON")

    async def run(self, command: str, args: dict) -> str:
        """Return the CLI envelope unchanged; picture output is always plugin-owned."""
        try:
            with tempfile.TemporaryDirectory(dir=getattr(decky, "DECKY_PLUGIN_RUNTIME_DIR", None)) as tmp:
                return await self._run(command, args, Path(tmp) / "picture")
        except OSError:
            return failure(command, "io", "Cannot create plugin temporary directory")

    async def picture(self, game: str, entry: str) -> str | None:
        """Read a picture as a data URL without changing run's envelope contract."""
        try:
            with tempfile.TemporaryDirectory(dir=getattr(decky, "DECKY_PLUGIN_RUNTIME_DIR", None)) as tmp:
                path = Path(tmp) / "picture"
                envelope = json.loads(await self._run("picture", {"game": game, "entry": entry}, path))
                if not envelope["ok"]:
                    return None
                data = path.read_bytes()
                if len(data) > 512 * 1024:
                    return None
                if data.startswith(b"\x89PNG\r\n\x1a\n"):
                    mime = "image/png"
                elif data.startswith(b"\xff\xd8\xff"):
                    mime = "image/jpeg"
                else:
                    return None
                return f"data:{mime};base64,{base64.b64encode(data).decode('ascii')}"
        except (OSError, ValueError):
            return None

    async def _main(self) -> None:
        """Decky load hook; commands run only when the panel asks."""

    async def _unload(self) -> None:
        """No persistent subprocess or temporary picture to release."""
