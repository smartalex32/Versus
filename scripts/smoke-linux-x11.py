#!/usr/bin/env python3
"""Exercise the packaged GUI through a legacy nxagent display in CI.

Requires an isolated Linux build container with DISPLAY pointing to nxagent.
The end-user package does not need these test tools.
"""
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time


def command(arguments, environment=None, check=True):
    return subprocess.run(arguments, env=environment, check=check,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                          universal_newlines=True, timeout=15)


def eventually(action, message, process=None):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        if process is not None and process.poll() is not None:
            raise RuntimeError("launcher exited with {}: {}".format(process.returncode, message))
        result = action()
        if result:
            return result
        time.sleep(0.1)
    raise RuntimeError(message)


def child_environment(parent):
    # Observe only this launcher's direct child, without printing credentials.
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            status = (entry / "status").read_text()
            if not re.search(r"^PPid:\s*{}$".format(parent), status, re.MULTILINE):
                continue
            pairs = (entry / "environ").read_bytes().split(b"\0")
            environment = dict(pair.decode().split("=", 1) for pair in pairs if b"=" in pair)
            if environment.get("VERSUS_X11_COMPAT") == "1":
                return environment
        except (OSError, UnicodeError):
            continue
    return None


def find_window(environment, name):
    result = command(["xdotool", "search", "--onlyvisible", "--name", name], environment, False)
    return result.stdout.splitlines()[0] if result.returncode == 0 and result.stdout else None


def geometry(environment, window):
    output = command(["xdotool", "getwindowgeometry", "--shell", window], environment).stdout
    return dict(line.split("=", 1) for line in output.splitlines() if "=" in line)


def stop(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


def main():
    image = Path(sys.argv[1]).resolve()
    host = os.environ.copy()
    host["DBUS_SESSION_BUS_ADDRESS"] = "unix:path=/nonexistent-versus-test-bus"
    with tempfile.TemporaryDirectory(prefix="versus-nx-test-") as temporary:
        root = Path(temporary)
        subprocess.run([str(image), "--appimage-extract"], cwd=str(root),
                       stdout=subprocess.DEVNULL, check=True, timeout=60)
        appdir = root / "squashfs-root"
        left = root / "left.txt"
        right = root / "right.txt"
        left.write_text("same line\nleft change\n")
        right.write_text("same line\nright change\n")
        # A stock /usr/bin/xkbcomp would hide broken relocation. The disposable
        # container owns this file; restore it even when the smoke test fails.
        compiler = Path("/usr/bin/xkbcomp")
        hidden_compiler = root / "host-xkbcomp"
        compiler.rename(hidden_compiler)
        drivers = Path("/usr/lib64/dri")
        hidden_drivers = root / "host-dri"
        drivers.rename(hidden_drivers)
        launcher = None
        try:
            with (root / "launcher.log").open("w+") as log:
                launcher = subprocess.Popen(
                    [str(appdir / "AppRun"), "--compat-x11", "--diff", str(left), str(right)],
                    env=host, stdout=log, stderr=log)
                try:
                    nested = eventually(lambda: child_environment(launcher.pid),
                                        "nested Versus did not start", launcher)
                    window = eventually(lambda: find_window(nested, "^Versus"),
                                        "nested Versus window did not open", launcher)
                    info = command(["xdpyinfo", "-ext", "XInputExtension"], nested).stdout
                    if "XInputExtension" not in info:
                        raise RuntimeError("nested XInput extension unavailable")
                    # A missing/wrong cookie must not access the private server.
                    unauthorized = nested.copy()
                    unauthorized["XAUTHORITY"] = str(root / "absent-authority")
                    if command(["xdpyinfo"], unauthorized, False).returncode == 0:
                        raise RuntimeError("private nested display accepted an unauthorized client")

                    outer = eventually(lambda: find_window(host, "Xephyr"),
                                       "Xephyr host window did not open", launcher)
                    for width, height in [(1152, 720), (1400, 900), (1280, 800)]:
                        command(["xdotool", "windowsize", outer, str(width), str(height)], host)
                        eventually(lambda: geometry(nested, window).get("WIDTH") == str(width)
                                   and geometry(nested, window).get("HEIGHT") == str(height),
                                   "Versus did not follow outer window resizing", launcher)

                    # Input enters through the legacy host, rather than being
                    # injected into the modern nested display directly.
                    command(["xdotool", "windowfocus", outer], host)
                    command(["xdotool", "mousemove", "--window", outer, "400", "400"], host)
                    time.sleep(0.3)
                    before = root / "before.xwd"
                    after = root / "after.xwd"
                    command(["xwd", "-silent", "-id", window, "-out", str(before)], nested)
                    command(["xdotool", "keydown", "Control_L", "click", "4", "keyup", "Control_L"], host)
                    time.sleep(0.4)
                    command(["xwd", "-silent", "-id", window, "-out", str(after)], nested)
                    if before.read_bytes() == after.read_bytes():
                        raise RuntimeError("Ctrl+wheel through nxagent did not change the GUI")
                    command(["xdotool", "key", "ctrl+0"], host)

                    # Exercise both bundled picker modes without a desktop
                    # portal; a successful selection also tests keyboard focus.
                    for directory in [False, True]:
                        arguments = [str(appdir / "usr/bin/zenity"), "--file-selection"]
                        if directory:
                            arguments.append("--directory")
                        picker = subprocess.Popen(arguments, env=nested, stdout=subprocess.PIPE,
                                                  stderr=subprocess.PIPE, universal_newlines=True)
                        try:
                            dialog = eventually(lambda: find_window(nested, "File Selection"),
                                                "bundled native picker did not open", launcher)
                            command(["xdotool", "windowactivate", "--sync", dialog], nested)
                            command(["xdotool", "key", "ctrl+l"], host)
                            choice = root if directory else left
                            command(["xdotool", "type", "--clearmodifiers", str(choice)], host)
                            command(["xdotool", "key", "Return"], host)
                            stdout, stderr = picker.communicate(timeout=10)
                            if picker.returncode != 0 or stdout.strip() != str(choice):
                                raise RuntimeError("native picker did not return selected path: " + stderr)
                        finally:
                            stop(picker)

                    authority = Path(nested["XAUTHORITY"])
                    command(["xdotool", "windowclose", outer], host)
                    launcher.wait(timeout=10)
                    if authority.parent.exists():
                        raise RuntimeError("launcher left private state after outer-window close")
                    print("NX compatibility passed: authenticated GUI, resize, Ctrl+wheel, file/folder pickers, cleanup.")
                except Exception:
                    log.flush()
                    log.seek(0)
                    print(log.read(), file=sys.stderr)
                    raise
        finally:
            if launcher is not None:
                stop(launcher)
            hidden_compiler.rename(compiler)
            hidden_drivers.rename(drivers)


if __name__ == "__main__":
    main()
