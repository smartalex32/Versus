import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest


ROOT = Path(__file__).resolve().parents[1]
LAUNCHER = ROOT / "scripts" / "launch-linux.sh"


class LinuxLauncherTests(unittest.TestCase):
    def setUp(self):
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary_directory.name)
        self.appdir = self.root / "AppDir"
        self.bin = self.appdir / "usr" / "bin"
        self.bin.mkdir(parents=True)
        (self.appdir / "usr" / "lib" / "dri").mkdir(parents=True)
        (self.appdir / "usr" / "share" / "X11" / "xkb").mkdir(parents=True)
        (self.appdir / "usr" / "share" / "versus").mkdir(parents=True)
        shutil.copy2(ROOT / "scripts" / "compat-openbox.xml",
                     self.appdir / "usr" / "share" / "versus" / "compat-openbox.xml")
        shutil.copy2(LAUNCHER, self.appdir / "AppRun")
        os.chmod(self.appdir / "AppRun", 0o755)
        self.log = self.root / "log"
        self._write_helper("xauth", """#!/bin/sh
printf 'xauth:%s\\n' "$*" >> "$TEST_LOG"
case " $* " in
  *" nlist :0 "*) printf '01000000000000124d49542d4d414749432d434f4f4b49452d31001000112233445566778899aabbccddeeff\\n' ;;
esac
exit 0
""")
        self._write_helper("xdpyinfo", """#!/bin/sh
printf 'xdpyinfo:%s:%s\\n' "$DISPLAY" "$XAUTHORITY" >> "$TEST_LOG"
test "${XDPYINFO_FAIL:-0}" != 1
""")
        self._write_helper("Xephyr", """#!/bin/sh
fd=''
while test "$#" -gt 0; do
  if test "$1" = -displayfd; then fd="$2"; shift 2; continue; fi
  shift
done
printf 'xephyr:%s:%s\\n' "$DISPLAY" "$XAUTHORITY" >> "$TEST_LOG"
test -n "$fd" || exit 2
eval "printf '%s\\n' \"${XEPHYR_DISPLAY:-77}\" >&$fd"
trap 'printf "xephyr-term\\n" >> "$TEST_LOG"; exit 0' TERM HUP INT
if test -n "${XEPHYR_EXIT_AFTER:-}"; then sleep "$XEPHYR_EXIT_AFTER"; exit 0; fi
while :; do sleep 1; done
""")
        self._write_helper("xkbcomp", "#!/bin/sh\nexit 0\n")
        self._write_helper("xprop", """#!/bin/sh
printf 'xprop:%s:%s\\n' "$DISPLAY" "$XAUTHORITY" >> "$TEST_LOG"
test "${XPROP_FAIL:-0}" != 1 || exit 1
printf '_NET_SUPPORTING_WM_CHECK(WINDOW): window id # 0x42\\n'
""")
        self._write_helper("openbox", """#!/bin/sh
printf 'openbox:%s:%s:%s\\n' "$DISPLAY" "$XAUTHORITY" "$*" >> "$TEST_LOG"
trap 'printf "openbox-term\\n" >> "$TEST_LOG"; exit 0' TERM HUP INT
if test -n "${OPENBOX_EXIT_AFTER:-}"; then sleep "$OPENBOX_EXIT_AFTER"; exit 0; fi
while :; do sleep 1; done
""")
        self._write_helper("versus", """#!/bin/sh
printf 'versus:' >> "$TEST_LOG"
for argument in "$@"; do printf '<%s>' "$argument" >> "$TEST_LOG"; done
printf ':%s:%s:%s:%s:%s\\n' "$DISPLAY" "$XAUTHORITY" "$VERSUS_X11_COMPAT" "$LIBGL_DRIVERS_PATH" "$LD_LIBRARY_PATH" >> "$TEST_LOG"
trap 'printf "versus-term\\n" >> "$TEST_LOG"; exit 42' TERM HUP INT
if test "${VERSUS_WAIT:-0}" = 1; then while :; do sleep 1; done; fi
exit "${VERSUS_EXIT:-0}"
""")

    def tearDown(self):
        self.temporary_directory.cleanup()

    def _write_helper(self, name, content):
        path = self.bin / name
        path.write_text(content)
        path.chmod(0o755)

    def _environment(self, **extra):
        environment = os.environ.copy()
        environment.update({"TEST_LOG": str(self.log), **extra})
        return environment

    def _run(self, *arguments, **environment):
        return subprocess.run(
            [str(self.appdir / "AppRun"), *arguments],
            env=self._environment(**environment),
            text=True,
            capture_output=True,
            timeout=8,
        )

    def test_ordinary_and_headless_launches_preserve_arguments_without_display(self):
        ordinary = self._run("ordinary", "", VERSUS_EXIT="17")
        self.assertEqual(ordinary.returncode, 17)
        ordinary_log = self.log.read_text()
        self.assertIn("versus:<ordinary><>::::", ordinary_log)
        self.assertIn(str(self.appdir / "usr" / "lib"), ordinary_log)

        self.log.unlink()
        literal = self._run("--", "--compat-x11", VERSUS_EXIT="18")
        self.assertEqual(literal.returncode, 18)
        self.assertIn("versus:<--><--compat-x11>", self.log.read_text())

        self.log.unlink()
        headless = self._run("--compat-x11", "--version", "value", VERSUS_EXIT="19")
        self.assertEqual(headless.returncode, 19)
        self.assertIn("versus:<--version><value>::::", self.log.read_text())

    def test_compatibility_mode_requires_a_host_display(self):
        result = self._run("--compat-x11", "input")
        self.assertEqual(result.returncode, 1)
        self.assertIn("requires an existing X11 DISPLAY", result.stderr)
        self.assertFalse(self.log.exists())

    def test_compatibility_mode_uses_private_nested_environment_and_app_status(self):
        result = self._run(
            "first", "--compat-x11", "", "last",
            DISPLAY=":42", XAUTHORITY="/host/Xauthority", VERSUS_EXIT="23",
        )
        self.assertEqual(result.returncode, 23, result.stderr)
        entries = self.log.read_text().splitlines()
        self.assertIn("xephyr::42:/host/Xauthority", entries)
        self.assertTrue(any("nlist :0" in entry for entry in entries))
        self.assertTrue(any("nmerge -" in entry for entry in entries))
        self.assertTrue(any(entry.startswith("xauth:-f ") and "add :77 " in entry for entry in entries))
        versus = next(entry for entry in entries if entry.startswith("versus:"))
        self.assertTrue(versus.startswith("versus:<first><><last>::77:"), versus)
        self.assertIn(":1:" + str(self.appdir / "usr" / "lib" / "dri"), versus)
        self.assertNotIn("/host/Xauthority", versus)
        openbox = next(entry for entry in entries if entry.startswith("openbox:"))
        self.assertIn("--sm-disable --config-file " + str(
            self.appdir / "usr" / "share" / "versus" / "compat-openbox.xml"), openbox)
        self.assertLess(entries.index(openbox), entries.index(versus))
        self.assertIn("xephyr-term", entries)

    def test_readiness_timeout_does_not_start_versus(self):
        result = self._run(
            "--compat-x11", DISPLAY=":42", XDPYINFO_FAIL="1",
            VERSUS_X11_READINESS_TIMEOUT="1",
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("did not become ready", result.stderr)
        self.assertFalse(any(line.startswith("versus:") for line in self.log.read_text().splitlines()))

    def test_openbox_readiness_failure_does_not_start_versus(self):
        result = self._run(
            "--compat-x11", DISPLAY=":42", XPROP_FAIL="1",
            VERSUS_X11_READINESS_TIMEOUT="1",
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("Openbox did not become ready", result.stderr)
        entries = self.log.read_text().splitlines()
        self.assertTrue(any(line.startswith("openbox:") for line in entries))
        self.assertFalse(any(line.startswith("versus:") for line in entries))

    def test_openbox_exit_terminates_nested_application(self):
        result = self._run(
            "--compat-x11", DISPLAY=":42", VERSUS_WAIT="1", OPENBOX_EXIT_AFTER="1",
        )
        self.assertEqual(result.returncode, 42)
        self.assertIn("versus-term", self.log.read_text().splitlines())

    def test_signal_terminates_nested_processes(self):
        process = subprocess.Popen(
            [str(self.appdir / "AppRun"), "--compat-x11"],
            env=self._environment(DISPLAY=":42", VERSUS_WAIT="1"),
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        for _ in range(80):
            if self.log.exists() and any(
                line.startswith("versus:") for line in self.log.read_text().splitlines()
            ):
                break
            time.sleep(0.05)
        else:
            process.kill()
            self.fail("nested Versus did not start")
        process.terminate()
        process.wait(timeout=5)
        process.stdout.close()
        process.stderr.close()
        self.assertEqual(process.returncode, 143)
        entries = self.log.read_text().splitlines()
        self.assertIn("versus-term", entries)
        self.assertIn("xephyr-term", entries)

    def test_closing_xephyr_terminates_its_nested_application(self):
        result = self._run(
            "--compat-x11", DISPLAY=":42", VERSUS_WAIT="1", XEPHYR_EXIT_AFTER="1",
        )
        self.assertEqual(result.returncode, 42)
        self.assertIn("versus-term", self.log.read_text().splitlines())


if __name__ == "__main__":
    unittest.main()
