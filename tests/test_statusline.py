#!/usr/bin/env python3
"""
Tests for the status line bridge and the cache file contract.

Run with:  python3 -m unittest discover -s tests -v

The contract tests matter most. The bridge is written in Python and the menu
bar app that reads its output is written in Swift, so nothing but these tests
checks that the two agree on field names and types.
"""

import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import threading
import unittest

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BRIDGE = os.path.join(REPO, "statusline", "claude-quota-statusline.py")
INSTALLER = os.path.join(REPO, "statusline", "install.py")
CACHE_READER = os.path.join(
    REPO, "menubar", "Sources", "ClaudeQuotaBar", "StatusLineCache.swift"
)


def swift_cache_keys():
    """Every JSON key StatusLineCache.swift actually looks up.

    Two shapes appear in that file: direct subscripts, `root["model"]` and
    `object["used_percentage"]`, and the window table, `(.fiveHour, "five_hour")`.
    """
    with open(CACHE_READER) as file:
        source = file.read()
    subscripts = set(re.findall(r'(?:root|object)\["([a-z_]+)"\]', source))
    window_keys = set(re.findall(r'\(\.\w+,\s*"([a-z_]+)"\)', source))
    return subscripts | window_keys


def run_bridge(payload, cache_path, env=None, raw=None):
    """Run the bridge with a payload, return its stdout."""
    environment = dict(os.environ)
    environment["CLAUDE_QUOTA_CACHE"] = cache_path
    if env:
        environment.update(env)

    stdin = raw if raw is not None else json.dumps(payload)
    result = subprocess.run(
        [sys.executable, BRIDGE],
        input=stdin,
        capture_output=True,
        text=True,
        env=environment,
        timeout=15,
    )
    assert result.returncode == 0, "bridge exited %d: %s" % (result.returncode, result.stderr)
    return result.stdout.strip()


def full_payload(five=23.5, seven=41.2):
    """A status line payload shaped like the documented Claude Code JSON."""
    return {
        "model": {"display_name": "Opus 5"},
        "session_id": "sess-abc123",
        "context_window": {"used_percentage": 34, "context_window_size": 200000},
        "cost": {"total_cost_usd": 1.23},
        "rate_limits": {
            "five_hour": {"used_percentage": five, "resets_at": 1738425600},
            "seven_day": {"used_percentage": seven, "resets_at": 1738857600},
        },
    }


class CacheTempDir(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.mkdtemp()
        self.cache = os.path.join(self.directory, "quota-bar-cache.json")

    def tearDown(self):
        shutil.rmtree(self.directory, ignore_errors=True)

    def read_cache(self):
        with open(self.cache) as file:
            return json.load(file)


class TestCacheWriting(CacheTempDir):
    def test_writes_both_windows(self):
        run_bridge(full_payload(), self.cache)
        cache = self.read_cache()
        self.assertEqual(cache["five_hour"]["used_percentage"], 23.5)
        self.assertEqual(cache["seven_day"]["used_percentage"], 41.2)
        self.assertEqual(cache["five_hour"]["resets_at"], 1738425600)

    def test_records_context(self):
        cache = (run_bridge(full_payload(), self.cache), self.read_cache())[1]
        self.assertEqual(cache["model"], "Opus 5")
        self.assertEqual(cache["session_id"], "sess-abc123")
        self.assertIsInstance(cache["captured_at"], float)

    def test_missing_rate_limits_does_not_clobber(self):
        """Pro/Max data is absent early in a session and on free accounts.

        Overwriting a good cache with nothing would blank the menu bar every
        time a session started.
        """
        run_bridge(full_payload(five=77.0), self.cache)
        run_bridge({"model": {"display_name": "Opus 5"}}, self.cache)
        self.assertEqual(self.read_cache()["five_hour"]["used_percentage"], 77.0)

    def test_no_cache_written_when_never_present(self):
        run_bridge({"model": {"display_name": "Opus 5"}}, self.cache)
        self.assertFalse(os.path.exists(self.cache))

    def test_partial_windows(self):
        """seven_day can be absent independently of five_hour."""
        payload = full_payload()
        del payload["rate_limits"]["seven_day"]
        run_bridge(payload, self.cache)
        cache = self.read_cache()
        self.assertIn("five_hour", cache)
        self.assertNotIn("seven_day", cache)

    def test_non_numeric_percentage_ignored(self):
        payload = full_payload()
        payload["rate_limits"]["five_hour"]["used_percentage"] = None
        run_bridge(payload, self.cache)
        cache = self.read_cache()
        self.assertNotIn("five_hour", cache)
        self.assertIn("seven_day", cache)

    def test_null_resets_at_preserved_as_null(self):
        payload = full_payload()
        payload["rate_limits"]["five_hour"]["resets_at"] = None
        run_bridge(payload, self.cache)
        self.assertIsNone(self.read_cache()["five_hour"]["resets_at"])

    def test_integer_percentage_becomes_float(self):
        payload = full_payload()
        payload["rate_limits"]["five_hour"]["used_percentage"] = 50
        run_bridge(payload, self.cache)
        self.assertIsInstance(self.read_cache()["five_hour"]["used_percentage"], float)

    def test_creates_missing_parent_directory(self):
        nested = os.path.join(self.directory, "deep", "nested", "cache.json")
        run_bridge(full_payload(), nested)
        self.assertTrue(os.path.exists(nested))

    def test_no_temp_files_left_behind(self):
        run_bridge(full_payload(), self.cache)
        leftovers = [name for name in os.listdir(self.directory) if name.startswith(".quota-bar-")]
        self.assertEqual(leftovers, [])


class TestNeverBreaksTheStatusLine(CacheTempDir):
    """A status line that exits non-zero or throws shows an error in Claude
    Code on every single render. The bridge must survive anything."""

    def test_malformed_json(self):
        self.assertEqual(run_bridge(None, self.cache, raw="not json at all"), "Claude")

    def test_empty_stdin(self):
        self.assertEqual(run_bridge(None, self.cache, raw=""), "Claude")

    def test_json_that_is_not_an_object(self):
        run_bridge(None, self.cache, raw="[1, 2, 3]")

    def test_unwritable_cache_still_prints(self):
        unwritable = os.path.join(self.directory, "readonly")
        os.makedirs(unwritable)
        os.chmod(unwritable, 0o500)
        try:
            output = run_bridge(full_payload(), os.path.join(unwritable, "cache.json"))
            self.assertIn("Opus 5", output)
        finally:
            os.chmod(unwritable, 0o700)

    def test_rate_limits_wrong_type(self):
        payload = full_payload()
        payload["rate_limits"] = "surprise"
        self.assertIn("Opus 5", run_bridge(payload, self.cache))

    def test_window_wrong_type(self):
        payload = full_payload()
        payload["rate_limits"]["five_hour"] = 42
        run_bridge(payload, self.cache)
        self.assertNotIn("five_hour", self.read_cache())


class TestDefaultStatusLine(CacheTempDir):
    def test_renders_model_context_and_quota(self):
        output = run_bridge(full_payload(), self.cache)
        self.assertIn("Opus 5", output)
        self.assertIn("34% ctx", output)
        self.assertIn("5h 23%", output)
        self.assertIn("7d 41%", output)

    def test_survives_missing_context_window(self):
        payload = full_payload()
        del payload["context_window"]
        output = run_bridge(payload, self.cache)
        self.assertIn("Opus 5", output)
        self.assertNotIn("ctx", output)

    def test_bar_is_proportional(self):
        payload = full_payload()
        payload["context_window"]["used_percentage"] = 100
        self.assertIn("▓" * 10, run_bridge(payload, self.cache))


class TestChaining(CacheTempDir):
    def test_chained_output_replaces_ours(self):
        chain = "%s -c \"import sys; sys.stdin.read(); print('CHAINED')\"" % sys.executable
        output = run_bridge(full_payload(), self.cache, env={"CLAUDE_QUOTA_CHAIN": chain})
        self.assertEqual(output, "CHAINED")

    def test_chained_command_receives_original_stdin(self):
        chain = (
            "%s -c \"import json,sys; print(json.load(sys.stdin)['model']['display_name'])\""
            % sys.executable
        )
        output = run_bridge(full_payload(), self.cache, env={"CLAUDE_QUOTA_CHAIN": chain})
        self.assertEqual(output, "Opus 5")

    def test_cache_still_written_when_chaining(self):
        chain = "%s -c \"import sys; sys.stdin.read(); print('X')\"" % sys.executable
        run_bridge(full_payload(), self.cache, env={"CLAUDE_QUOTA_CHAIN": chain})
        self.assertEqual(self.read_cache()["five_hour"]["used_percentage"], 23.5)

    def test_broken_chain_falls_back_to_our_line(self):
        env = {"CLAUDE_QUOTA_CHAIN": "/nonexistent/command/that/does/not/exist"}
        output = run_bridge(full_payload(), self.cache, env=env)
        self.assertIn("Opus 5", output)


class TestAtomicity(CacheTempDir):
    def test_concurrent_writes_never_expose_a_partial_file(self):
        """The app watches this file. A reader must never see a half-written
        JSON document, which is why the bridge writes to a temp file and
        renames."""
        stop = threading.Event()
        failures = []

        def reader():
            while not stop.is_set():
                try:
                    with open(self.cache) as file:
                        json.load(file)
                except FileNotFoundError:
                    pass
                except Exception as error:
                    failures.append(repr(error))

        thread = threading.Thread(target=reader)
        thread.start()
        try:
            for index in range(40):
                run_bridge(full_payload(five=float(index)), self.cache)
        finally:
            stop.set()
            thread.join()

        self.assertEqual(failures, [], "reader saw a torn file: %s" % failures[:3])
        self.assertEqual(self.read_cache()["five_hour"]["used_percentage"], 39.0)


class TestSwiftContract(CacheTempDir):
    """Locks the cache format to what StatusLineCache.swift actually parses.

    Swift reads these with casts that silently yield nil on a mismatch, so a
    rename or type drift would show up as an empty menu bar rather than an
    error. The Swift half of this contract lives in
    `menubar/Tests/ClaudeQuotaBarTests/StatusLineCacheTests.swift`, which pins
    what Swift reads; this class pins what Python writes, and
    `test_key_names_match_the_swift_source` compares the two directly.
    """

    def test_swift_source_is_where_we_think_it_is(self):
        """Guards the test below: a moved file would otherwise make it pass
        vacuously by comparing against an empty key set."""
        self.assertTrue(os.path.exists(CACHE_READER), CACHE_READER)
        self.assertTrue(swift_cache_keys(), "parsed no keys out of the Swift source")

    def test_key_names_match_the_swift_source(self):
        """The real cross-language check: every key the bridge writes is one
        Swift looks up, and vice versa. Catches a rename on either side."""
        run_bridge(full_payload(), self.cache)
        cache = self.read_cache()
        written = set(cache)
        for window in ("five_hour", "seven_day"):
            written |= set(cache[window])
        self.assertEqual(written, swift_cache_keys())

    def test_top_level_keys_are_exactly_what_swift_reads(self):
        run_bridge(full_payload(), self.cache)
        cache = self.read_cache()
        self.assertEqual(
            set(cache),
            {"five_hour", "seven_day", "captured_at", "model", "session_id"},
        )

    def test_window_keys_match_swift(self):
        run_bridge(full_payload(), self.cache)
        for key in ("five_hour", "seven_day"):
            self.assertEqual(set(self.read_cache()[key]), {"used_percentage", "resets_at"})

    def test_types_match_swift_casts(self):
        run_bridge(full_payload(), self.cache)
        cache = self.read_cache()
        # `as? Double` on a JSON number succeeds for both int and real via
        # NSNumber bridging, but percentages must never arrive as strings.
        self.assertIsInstance(cache["five_hour"]["used_percentage"], float)
        self.assertIsInstance(cache["five_hour"]["resets_at"], (int, float))
        self.assertIsInstance(cache["captured_at"], float)
        self.assertIsInstance(cache["model"], str)
        self.assertIsInstance(cache["session_id"], str)

    def test_percentages_are_0_to_100_not_0_to_1(self):
        """The Swift colour ramp and ring both assume 0-100. The legacy CLI
        used 0-1 fractions, so this is a real trap."""
        run_bridge(full_payload(five=93.0), self.cache)
        self.assertEqual(self.read_cache()["five_hour"]["used_percentage"], 93.0)

    def test_captured_at_is_epoch_seconds_not_milliseconds(self):
        """Swift does Date(timeIntervalSince1970:), so milliseconds would
        land in the year 57000 and every snapshot would look fresh forever."""
        import time

        run_bridge(full_payload(), self.cache)
        self.assertLess(abs(self.read_cache()["captured_at"] - time.time()), 60)


class TestInstaller(unittest.TestCase):
    def setUp(self):
        self.home = tempfile.mkdtemp()
        os.makedirs(os.path.join(self.home, ".claude"))

    def tearDown(self):
        shutil.rmtree(self.home, ignore_errors=True)

    @property
    def settings_path(self):
        return os.path.join(self.home, ".claude", "settings.json")

    def run_installer(self, *args):
        environment = dict(os.environ)
        environment["HOME"] = self.home
        result = subprocess.run(
            [sys.executable, INSTALLER, *args],
            capture_output=True,
            text=True,
            env=environment,
            timeout=15,
        )
        return result

    def settings(self):
        with open(self.settings_path) as file:
            return json.load(file)

    def write_settings(self, data):
        with open(self.settings_path, "w") as file:
            json.dump(data, file)

    def test_dry_run_changes_nothing(self):
        self.run_installer()
        self.assertFalse(os.path.exists(self.settings_path))

    def test_apply_sets_status_line(self):
        self.run_installer("--apply")
        self.assertEqual(self.settings()["statusLine"]["command"], BRIDGE)
        self.assertEqual(self.settings()["statusLine"]["type"], "command")

    def test_apply_is_idempotent(self):
        self.run_installer("--apply")
        result = self.run_installer("--apply")
        self.assertIn("Already installed", result.stdout)
        self.assertEqual(self.settings()["statusLine"]["command"], BRIDGE)

    def test_preserves_existing_status_line_by_chaining(self):
        self.write_settings({"statusLine": {"type": "command", "command": "my-line.sh --x"}})
        self.run_installer("--apply")
        command = self.settings()["statusLine"]["command"]
        self.assertIn("CLAUDE_QUOTA_CHAIN=", command)
        self.assertIn("my-line.sh --x", command)
        self.assertTrue(command.endswith(BRIDGE))

    def test_preserves_unrelated_settings(self):
        self.write_settings({"model": "opus", "env": {"FOO": "bar"}})
        self.run_installer("--apply")
        self.assertEqual(self.settings()["model"], "opus")
        self.assertEqual(self.settings()["env"], {"FOO": "bar"})

    def test_backs_up_before_writing(self):
        self.write_settings({"model": "opus"})
        self.run_installer("--apply")
        with open(self.settings_path + ".quota-bar-backup") as file:
            self.assertEqual(json.load(file), {"model": "opus"})

    def test_remove_restores_chained_command(self):
        self.write_settings({"statusLine": {"type": "command", "command": "my-line.sh --x"}})
        self.run_installer("--apply")
        self.run_installer("--apply", "--remove")
        self.assertEqual(self.settings()["statusLine"]["command"], "my-line.sh --x")

    def test_remove_drops_setting_when_there_was_none(self):
        self.run_installer("--apply")
        self.run_installer("--apply", "--remove")
        self.assertNotIn("statusLine", self.settings())

    def test_remove_refuses_foreign_status_line(self):
        self.write_settings({"statusLine": {"type": "command", "command": "someone-elses.sh"}})
        result = self.run_installer("--apply", "--remove")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.settings()["statusLine"]["command"], "someone-elses.sh")

    def test_double_install_preserves_the_chained_command(self):
        """A second --apply must not discard the user's original status line,
        nor nest a second wrapper around it."""
        self.write_settings({"statusLine": {"type": "command", "command": "my-line.sh"}})
        self.run_installer("--apply")
        first = self.settings()["statusLine"]["command"]
        self.run_installer("--apply")
        second = self.settings()["statusLine"]["command"]

        self.assertEqual(first, second)
        self.assertEqual(second.count("CLAUDE_QUOTA_CHAIN="), 1)
        self.assertIn("my-line.sh", second)

    def test_remove_after_double_install_still_restores(self):
        self.write_settings({"statusLine": {"type": "command", "command": "my-line.sh"}})
        self.run_installer("--apply")
        self.run_installer("--apply")
        self.run_installer("--apply", "--remove")
        self.assertEqual(self.settings()["statusLine"]["command"], "my-line.sh")

    def test_rejects_unparseable_settings(self):
        with open(self.settings_path, "w") as file:
            file.write("{ this is not json")
        result = self.run_installer("--apply")
        self.assertNotEqual(result.returncode, 0)


class TestInstallerQuoting(unittest.TestCase):
    """Claude Code runs `statusLine.command` through a shell, so both halves of
    the generated command have to survive that shell intact."""

    def setUp(self):
        self.home = tempfile.mkdtemp()
        os.makedirs(os.path.join(self.home, ".claude"))
        self.cache = os.path.join(self.home, "cache.json")

    def tearDown(self):
        shutil.rmtree(self.home, ignore_errors=True)

    @property
    def settings_path(self):
        return os.path.join(self.home, ".claude", "settings.json")

    def write_settings(self, data):
        with open(self.settings_path, "w") as file:
            json.dump(data, file)

    def installed_command(self):
        with open(self.settings_path) as file:
            return json.load(file)["statusLine"]["command"]

    def install(self, installer=INSTALLER):
        environment = dict(os.environ)
        environment["HOME"] = self.home
        result = subprocess.run(
            [sys.executable, installer, "--apply"],
            capture_output=True, text=True, env=environment, timeout=15,
        )
        assert result.returncode == 0, result.stderr
        return result

    def test_script_path_with_spaces_still_runs(self):
        """A repo checked out under `~/My Projects` produced a command the
        shell split in half."""
        spaced = os.path.join(self.home, "my status line")
        shutil.copytree(os.path.join(REPO, "statusline"), spaced)
        os.chmod(os.path.join(spaced, "claude-quota-statusline.py"), 0o755)

        self.install(os.path.join(spaced, "install.py"))

        environment = dict(os.environ)
        environment["HOME"] = self.home
        environment["CLAUDE_QUOTA_CACHE"] = self.cache
        result = subprocess.run(
            self.installed_command(),
            shell=True, input=json.dumps(full_payload()),
            capture_output=True, text=True, env=environment, timeout=15,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Opus 5", result.stdout)
        self.assertTrue(os.path.exists(self.cache))

    def test_previous_command_reaches_the_bridge_unexpanded(self):
        """The chained command must arrive as literal text. Double-quoting it
        let the outer shell expand `$VARS` and backticks first, so the bridge
        ran something the user never wrote."""
        original = 'my-line.sh "$SENTINEL" `hostname`'
        self.write_settings({"statusLine": {"type": "command", "command": original}})
        self.install()

        # Swap our script for a probe that just reports what the shell set, so
        # this measures the outer shell only.
        probe = "%s -c 'import os,sys; sys.stdout.write(os.environ[\"CLAUDE_QUOTA_CHAIN\"])'" % (
            shlex.quote(sys.executable)
        )
        command = self.installed_command().replace(shlex.quote(BRIDGE), probe)

        result = subprocess.run(
            command, shell=True, capture_output=True, text=True, timeout=15,
            env={"SENTINEL": "EXPANDED", "PATH": os.environ["PATH"]},
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, original)


if __name__ == "__main__":
    unittest.main()
