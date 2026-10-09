#!/usr/bin/env python3
"""Exercise the packaged GUI through a legacy nxagent display in CI.

Requires an isolated Linux build container with DISPLAY pointing to nxagent.
The end-user package does not need these test tools.
"""
import os
from pathlib import Path
import re
import subprocess
import struct
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
        left = root / "left input.txt"
        right = root / "right input.txt"
        left.write_text("same line\nleft change\n")
        right.write_text("same line\nright change\n")
        # A stock /usr/bin/xkbcomp would hide broken relocation. The disposable
        # container owns this file; restore it even when the smoke test fails.
        masks = [Path("/usr/bin/xkbcomp"), Path("/usr/lib64/dri"),
                 Path("/usr/share/glib-2.0/schemas"), Path("/usr/share/glvnd/egl_vendor.d")]
        for name in ["libGL.so.1", "libEGL.so.1", "libGLX.so.0", "libGLdispatch.so.0",
                     "libGLX_mesa.so.0", "libEGL_mesa.so.0", "libgbm.so.1", "libglapi.so.0",
                     "libgtk-3.so.0", "libgdk-3.so.0"]:
            library = Path("/usr/lib64") / name
            target = library.resolve(strict=True)
            if target not in masks:
                masks.append(target)
        hidden = []
        launcher = None
        try:
            for index, path in enumerate(masks):
                backup = root / "host-runtime-{}".format(index)
                path.rename(backup)
                hidden.append((path, backup))
            with (root / "launcher.log").open("w+") as log:
                launcher = subprocess.Popen(
                    [str(appdir / "AppRun"), "--compat-x11", "--diff", str(left), str(right)],
                    env=host, stdout=log, stderr=log)
                try:
                    nested = eventually(lambda: child_environment(launcher.pid),
                                        "nested Versus did not start", launcher)
                    window = eventually(lambda: find_window(nested, "^Versus"),
                                        "nested Versus window did not open", launcher)
                    window_class = command(["xprop", "-id", window, "WM_CLASS"], nested).stdout
                    if not re.search(r'"Versus"\s*$', window_class):
                        raise RuntimeError("unexpected comparison window class: " + window_class)

                    def maximized():
                        state = command(["xprop", "-id", window, "_NET_WM_STATE"], nested).stdout
                        return all(value in state for value in
                                   ["_NET_WM_STATE_MAXIMIZED_HORZ", "_NET_WM_STATE_MAXIMIZED_VERT"])

                    eventually(maximized, "comparison window was not maximized when mapped", launcher)
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
                    for width, height in [(1152, 720), (800, 600), (1400, 900), (1280, 800)]:
                        command(["xdotool", "windowsize", outer, str(width), str(height)], host)
                        try:
                            eventually(lambda: geometry(host, outer).get("WIDTH") == str(width)
                                       and geometry(host, outer).get("HEIGHT") == str(height),
                                       "host Xephyr window did not resize", launcher)
                            eventually(lambda: re.search(
                                r"dimensions:\s+{}x{}\s".format(width, height),
                                command(["xdpyinfo"], nested).stdout),
                                "private display did not follow host window resizing", launcher)
                            eventually(lambda: geometry(nested, window).get("WIDTH") == str(width)
                                       and geometry(nested, window).get("HEIGHT") == str(height),
                                       "Versus did not follow outer window resizing", launcher)
                        except Exception:
                            print("Requested resize: {}x{}; host: {}; client: {}".format(
                                width, height, geometry(host, outer), geometry(nested, window)),
                                file=sys.stderr)
                            root_info = command(["xdpyinfo"], nested).stdout
                            print(re.search(r"dimensions:.*", root_info).group(), file=sys.stderr)
                            print(command(["xprop", "-id", window, "_NET_WM_STATE",
                                           "_NET_FRAME_EXTENTS", "WM_NORMAL_HINTS", "WM_CLASS"], nested).stdout,
                                  file=sys.stderr)
                            raise
                        print("NX resize passed: {}x{}".format(width, height), flush=True)


                    # Input enters through the legacy host, rather than being
                    # injected into the modern nested display directly.
                    command(["xdotool", "windowfocus", outer], host)
                    command(["xdotool", "mousemove", "--window", outer, "400", "400"], host)
                    before = root / "before.xwd"
                    after = root / "after.xwd"

                    def capture(destination):
                        command(["xwd", "-silent", "-id", window, "-out", str(destination)], nested)
                        return destination.read_bytes()

                    previous = [None]

                    def stable_frame():
                        frame = capture(before)
                        header = struct.unpack(">25I", frame[:100])
                        pixels = frame[header[0] + header[19] * 12:]
                        painted = len(set(pixels[::max(1, header[11] // 8)])) > 16
                        stable = painted and previous[0] == frame
                        previous[0] = frame
                        return frame if stable else None

                    baseline = eventually(stable_frame, "comparison did not finish painting", launcher)
                    command(["xdotool", "keydown", "Control_L", "click", "4", "keyup", "Control_L"], host)
                    eventually(lambda: capture(after) != baseline,
                               "Ctrl+wheel through nxagent did not change the GUI", launcher)
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
                            time.sleep(0.2)
                            if find_window(nested, "File Selection"):
                                command(["xdotool", "key", "Return"], host)
                            stdout, stderr = picker.communicate(timeout=10)
                            if picker.returncode != 0 or stdout.strip() != str(choice):
                                raise RuntimeError("native picker did not return selected path: " + stderr)
                        finally:
                            stop(picker)

                    # Open and cancel both pickers through the real Browse path
                    # controls, delivering clicks and keys through nxagent.
                    for folder_mode in [False, True]:
                        command(["xdotool", "windowfocus", outer], host)
                        if folder_mode:
                            command(["xdotool", "mousemove", "--window", outer,
                                     "550", "26", "click", "1"], host)
                            time.sleep(0.2)
                        command(["xdotool", "mousemove", "--window", outer,
                                 "200", "100", "click", "1"], host)
                        dialog = eventually(lambda: find_window(nested, "File Selection"),
                                            "Browse did not open its native picker", launcher)
                        command(["xdotool", "windowactivate", "--sync", dialog], nested)
                        command(["xdotool", "key", "Escape"], host)
                        eventually(lambda: not find_window(nested, "File Selection"),
                                   "native Browse picker did not cancel", launcher)

                    authority = Path(nested["XAUTHORITY"])
                    protocols = command(["xprop", "-id", outer, "WM_PROTOCOLS"], host).stdout
                    if "WM_DELETE_WINDOW" not in protocols:
                        raise RuntimeError("outer display does not advertise normal WM close")
                    command(["xdotool", "windowactivate", "--sync", outer], host)
                    command(["xdotool", "key", "alt+F4"], host)
                    if launcher.wait(timeout=10) != 0:
                        raise RuntimeError("normal outer-window close returned a difftool error")
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
            for path, backup in reversed(hidden):
                backup.rename(path)


if __name__ == "__main__":
    main()
