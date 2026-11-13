"""Backend contract tests: fake subprocesses, no installed CLI or Decky host needed."""

import asyncio
import importlib.util
import json
import os
from pathlib import Path
import sys
import types
import unittest
from unittest.mock import AsyncMock, Mock, patch

spec = importlib.util.spec_from_file_location("save_relay_decky", Path(__file__).with_name("main.py"))
backend = importlib.util.module_from_spec(spec)
with patch.dict(sys.modules, {"decky": types.ModuleType("decky")}):
    spec.loader.exec_module(backend)


FAKE_CLI = """#!{python}
import json, os, sys, time
mode, folder = sys.argv[2], os.environ["FAKE_DIR"]
if mode == "progress":
    # Writes a little every 50 ms for about 2 s: longer than STALL_TIMEOUT, yet making progress.
    with open(os.path.join(folder, "out"), "ab") as out:
        for _ in range(40):
            out.write(b"x" * 4096)
            out.flush()
            time.sleep(0.05)
elif mode == "stall":
    # Silent for far longer than STALL_TIMEOUT: must be stopped.
    open(os.path.join(folder, "pid"), "w").write(str(os.getpid()))
    time.sleep(30)
print(json.dumps({{"ok": True, "command": sys.argv[1], "data": {{}}}}))
"""


class TransferProgressTests(unittest.IsolatedAsyncioTestCase):
    """Real child processes: the watchdog reads the child's I/O counters on Linux."""

    def setUp(self):
        import tempfile
        self.folder = tempfile.mkdtemp()
        self.addCleanup(__import__("shutil").rmtree, self.folder, True)
        fake = Path(self.folder) / "save-relay"
        fake.write_text(FAKE_CLI.format(python=sys.executable))
        fake.chmod(0o755)
        for target, value in [
            ("shutil.which", Mock(return_value=str(fake))),
            ("os.access", Mock(return_value=False)),
        ]:
            patcher = patch.object(getattr(backend, target.split(".")[0]), target.split(".")[1], value)
            patcher.start()
            self.addCleanup(patcher.stop)
        env = patch.dict(os.environ, {"FAKE_DIR": self.folder})
        env.start()
        self.addCleanup(env.stop)
        for name, value in [("STALL_TIMEOUT", 0.5), ("TRANSFER_CEILING", 20), ("PROGRESS_POLL", 0.05)]:
            patcher = patch.object(backend, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)

    async def test_transfer_still_making_progress_is_not_timed_out(self):
        envelope = json.loads(await backend.Plugin().run("push", {"game": "progress"}))
        self.assertTrue(envelope["ok"], envelope)

    async def test_stalled_transfer_is_stopped_and_reaped(self):
        loop = asyncio.get_running_loop()
        started = loop.time()
        envelope = json.loads(await backend.Plugin().run("push", {"game": "stall"}))
        self.assertEqual(envelope["error"]["message"], "save-relay timed out")
        self.assertLess(loop.time() - started, 10)
        pid = int((Path(self.folder) / "pid").read_text())
        self.assertFalse(os.path.exists(f"/proc/{pid}"), "the stalled child was not reaped")


class BackendTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.plugin = backend.Plugin()
        self.line = '{"ok":true,"command":"where","data":{"games":[]},"future":42}\n'
        self.process = Mock(returncode=0)
        self.process.communicate = AsyncMock(return_value=(self.line.encode(), None))
        self.spawn = AsyncMock(return_value=self.process)
        for target, value in [
            ("asyncio.create_subprocess_exec", self.spawn),
            ("shutil.which", Mock(return_value="/fake/save-relay")),
            ("os.access", Mock(return_value=False)),
        ]:
            patcher = patch.object(getattr(backend, target.split(".")[0]), target.split(".")[1], value)
            patcher.start()
            self.addCleanup(patcher.stop)

    async def test_happy_path_preserves_exact_line_and_future_fields(self):
        self.assertEqual(await self.plugin.run("where", {}), self.line)
        self.spawn.assert_awaited_once_with(
            "/fake/save-relay", "where", "--json",
            stdin=asyncio.subprocess.DEVNULL, stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.DEVNULL,
        )

    async def test_arguments_for_every_command_and_json_last(self):
        cases = [
            ("where", {}, []),
            ("status", {"game": "g", "manifest": "m", "saves-dir": "s"},
             ["g", "--manifest", "m", "--saves-dir", "s"]),
            ("history", {"game": "g"}, ["g"]),
            ("restore", {"game": "g", "entry": "e", "manifest": "m"}, ["g", "e", "--manifest", "m"]),
            ("pull", {"game": "g; echo unsafe", "resolve": "keep-server", "anyway": True},
             ["g; echo unsafe", "--resolve", "keep-server", "--anyway"]),
            ("push", {"game": "g", "resolve": "keep-local"}, ["g", "--resolve", "keep-local"]),
            ("answer", {"game": "g", "answer": "play-anyway"}, ["g", "play-anyway"]),
            ("which", {"appid": 42}, ["42"]),
            ("steam-games", {"no-steam-cloud": True, "refresh": True}, ["--no-steam-cloud", "--refresh"]),
            ("install-wrap", {"app-ids": [42, 43], "dry-run": True}, ["42", "43", "--dry-run"]),
            ("launch-option", {"appid": 42, "remove": True, "current": "-novid %command%"},
             ["42", "--remove", "--current=-novid %command%"]),
            ("setup", {"service": "gdrive", "no-browser": True, "finish": False},
             ["--service", "gdrive", "--no-browser"]),
            ("setup", {"service": "dropbox", "finish": True, "device": "d", "root": "r"},
             ["--service", "dropbox", "--device", "d", "--root", "r", "--finish"]),
            ("setup", {"service": "nextcloud", "server": "https://example.invalid", "user": "u"},
             ["--service", "nextcloud", "--server", "https://example.invalid", "--user", "u"]),
            ("picture", {"game": "g", "entry": "e"}, ["g", "e", "--out", "owned-output"]),
        ]
        for command, args, expected in cases:
            with self.subTest(command=command, args=args):
                self.process.communicate.return_value = (json.dumps({"ok": True, "command": command, "data": {}}).encode(), None)
                await self.plugin._run(command, args, "owned-output")
                self.assertEqual(self.spawn.call_args.args, ("/fake/save-relay", command, *expected, "--json"))
                self.assertNotIn("shell", self.spawn.call_args.kwargs)

    async def test_bundled_binary_precedes_path(self):
        with patch.object(backend.os, "access", return_value=True):
            await self.plugin.run("where", {})
        self.assertEqual(self.spawn.call_args.args[0], str(backend.PLUGIN_DIR / "bin" / "save-relay"))
        backend.shutil.which.assert_not_called()

    async def test_missing_binary_and_start_failure_are_io(self):
        with patch.object(backend.shutil, "which", return_value=None):
            self.assert_error(await self.plugin.run("where", {}), "io")
        self.spawn.assert_not_awaited()
        self.spawn.side_effect = FileNotFoundError()
        self.assert_error(await self.plugin.run("where", {}), "io")

    async def test_non_transfer_timeout_kills_and_reaps(self):
        finished = asyncio.Event()

        async def hang():
            await finished.wait()
            return (b"", None)

        self.process.returncode = None
        self.process.communicate.side_effect = hang
        self.process.kill.side_effect = finished.set
        with patch.object(backend, "STALL_TIMEOUT", 0.05), patch.object(backend, "PROGRESS_POLL", 0.01):
            self.assert_error(await self.plugin.run("where", {}), "io")
        self.process.kill.assert_called_once()
        self.assertEqual(self.process.communicate.await_count, 1)

    async def test_bad_json_empty_multiple_objects_and_wrong_envelopes(self):
        for stdout in [b"bad", b"", b"{}\n{}", b"null", b"[]", b"{}", b'\xff',
                       b'{"ok":true,"command":"push","data":{}}',
                       b'{"ok":false,"command":"where","error":{}}']:
            with self.subTest(stdout=stdout):
                self.process.communicate.return_value = (stdout, None)
                self.assert_error(await self.plugin.run("where", {}), "io")

    async def test_cli_error_and_nonzero_success_pass_through(self):
        for envelope in [
            {"ok": False, "command": "pull", "error": {"code": "auth_failed", "message": "Sign in"}},
            {"ok": True, "command": "pull", "data": {"action": "held"}},
        ]:
            line = json.dumps(envelope)
            self.process.returncode = 6
            self.process.communicate.return_value = (line.encode(), None)
            self.assertEqual(await self.plugin.run("pull", {"game": "g"}), line)

    async def test_invalid_input_never_spawns(self):
        for command, args in [("wrap", {}), ("status", {}), ("where", {"game": "g"}),
                              ("push", {"game": "g", "anyway": True}),
                              ("pull", {"game": "g", "anyway": "yes"}),
                              ("status", {"game": "--json"}), ("setup", {}),
                              ("status", {"game": "g\0"}), ("where", [])]:
            with self.subTest(command=command, args=args):
                self.assert_error(await self.plugin.run(command, args), "usage")
        self.spawn.assert_not_awaited()

    async def test_launch_option_preserves_empty_current_and_app_ids_are_numeric(self):
        for command, args, expected in [
            ("launch-option", {"appid": 42, "current": ""}, ["42", "--current="]),
            ("install-wrap", {"app-ids": [42]}, ["42"]),
        ]:
            self.process.communicate.return_value = (json.dumps({"ok": True, "command": command, "data": {}}).encode(), None)
            await self.plugin._run(command, args, "owned-output")
            self.assertEqual(self.spawn.call_args.args[1:], (command, *expected, "--json"))
        for args in [{"appid": 42, "current": "two\nlines"}, {"app-ids": ["42"]}]:
            self.assert_error(await self.plugin.run("launch-option" if "appid" in args else "install-wrap", args), "usage")

    async def test_picture_data_url_owned_output_and_cleanup(self):
        paths = []

        async def communicate():
            argv = self.spawn.call_args.args
            path = Path(argv[argv.index("--out") + 1])
            paths.append(path)
            path.write_bytes(b"\x89PNG\r\n\x1a\nimage")
            return json.dumps({"ok": True, "command": "picture", "data": {"path": str(path), "bytes": 13}}).encode(), None

        self.process.communicate.side_effect = communicate
        self.assertEqual(await self.plugin.picture("g", "e"), "data:image/png;base64,iVBORw0KGgppbWFnZQ==")
        self.assertFalse(paths[0].exists())
        self.spawn.side_effect = OSError()
        self.assertIsNone(await self.plugin.picture("g", "e"))

    def assert_error(self, line, code):
        envelope = json.loads(line)
        self.assertFalse(envelope["ok"])
        self.assertEqual(envelope["error"]["code"], code)
        self.assertIsInstance(envelope["error"]["message"], str)


if __name__ == "__main__":
    unittest.main()
