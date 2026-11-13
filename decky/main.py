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

# A command that prints nothing and moves no bytes for this long is stopped (seconds).
STALL_TIMEOUT = 60
# Transfers (pull, push, restore) may run longer while they still move bytes, up to this (seconds).
TRANSFER_CEILING = 4 * 3600
TRANSFER_COMMANDS = {"pull", "push", "restore"}
PROGRESS_POLL = 1.0
PLUGIN_DIR = Path(__file__).resolve().parent
API_COMMANDS = {
    "status", "history", "restore", "pull", "push", "answer", "where", "which",
    "picture", "setup", "steam-games", "install-wrap", "launch-option",
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
        "steam-games": (), "install-wrap": ("app-ids",),
        "launch-option": ("appid",),
    }.get(command, ("game",))
    flags = {
        "status": ("manifest", "saves-dir"),
        "restore": ("manifest", "saves-dir"),
        "pull": ("manifest", "saves-dir", "resolve", "anyway"),
        "push": ("manifest", "saves-dir", "resolve"),
        "setup": ("service", "server", "user", "device", "root", "no-browser", "finish"),
        "steam-games": ("no-steam-cloud", "refresh"),
        "install-wrap": ("no-steam-cloud", "remove", "dry-run"),
        "launch-option": ("remove", "current"),
    }.get(command, ())
    if set(args) - set(positionals) - set(flags):
        raise ValueError("Unexpected command argument")
    result = [command]
    for key in positionals:
        value = args.get(key)
        if command == "launch-option" and key == "appid" and type(value) is not int:
            raise ValueError("appid must be a whole number")
        if key == "app-ids":
            if value is None:
                continue
            if not isinstance(value, list) or any(type(app_id) is not int or not 1 <= app_id <= 4294967295 for app_id in value):
                raise ValueError("app-ids must be positive whole numbers")
            result.extend(str(app_id) for app_id in value)
            continue
        if key == "appid" and type(value) is int and 0 <= value <= 4294967295:
            value = str(value)
        if not isinstance(value, str) or not value or value.startswith("-") or "\0" in value:
            raise ValueError(f"Missing or invalid {key}")
        result.append(value)
    for key in flags:
        if key not in args:
            continue
        value = args[key]
        if key in {"anyway", "no-browser", "finish", "no-steam-cloud", "refresh", "remove", "dry-run"}:
            if type(value) is not bool:
                raise ValueError(f"{key} must be a boolean")
            if value:
                result.append(f"--{key}")
        else:
            if command == "launch-option" and key == "current":
                if not isinstance(value, str) or "\n" in value or "\r" in value or "\0" in value:
                    raise ValueError("current must be single-line text without NUL")
                result.append(f"--current={value}")
                continue
            if not isinstance(value, str) or not value or value.startswith("-") or "\0" in value:
                raise ValueError(f"Invalid {key}")
            result.extend([f"--{key}", value])
    if command == "setup" and "service" not in args:
        raise ValueError("setup requires service")
    if command == "picture":
        result.extend(["--out", str(picture_out)])
    return result + ["--json"]


def _progress(pid):
    """Bytes the child has read and written so far (Linux /proc), or None when unknown."""
    try:
        counters = Path(f"/proc/{pid}/io").read_text()
    except (OSError, ValueError):
        return None
    total = 0
    for line in counters.splitlines():
        if line.startswith(("rchar:", "wchar:")):
            total += int(line.split()[1])
    return total


async def _communicate(process, command):
    """Wait for the child's output. A transfer is stopped only when its I/O has stopped moving
    for STALL_TIMEOUT, or when it runs past TRANSFER_CEILING; any other command past
    STALL_TIMEOUT. Raises asyncio.TimeoutError after killing and reaping the child."""
    loop = asyncio.get_running_loop()
    task = asyncio.ensure_future(process.communicate())
    started = moved = loop.time()
    last = _progress(process.pid)
    try:
        while True:
            done, _ = await asyncio.wait({task}, timeout=PROGRESS_POLL)
            if done:
                return task.result()
            now = loop.time()
            current = _progress(process.pid)
            if current is not None and current != last:
                last, moved = current, now
            if command in TRANSFER_COMMANDS:
                expired = now - moved >= STALL_TIMEOUT or now - started >= TRANSFER_CEILING
            else:
                expired = now - started >= STALL_TIMEOUT
            if expired:
                raise asyncio.TimeoutError()
    except BaseException:
        if process.returncode is None:
            try:
                process.kill()
            except ProcessLookupError:
                pass
        await asyncio.wait({task})
        raise


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
            stdout, _ = await _communicate(process, command)
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
