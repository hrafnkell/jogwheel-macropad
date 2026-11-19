#!/usr/bin/env python3
"""
Jogwheel auto layer switcher (Linux / X11).

Watches which window has focus and tells the macropad over USB serial, which
switches to the layer whose "apps" list matches the window class (or title,
with a "title:" prefix). All matching rules live in layers.json on the pad -
this script just reports the focused window.

Standard library only. Needs `xprop` (x11-utils) and read/write access to the
pad's serial port (be in the `dialout` group).

    tools/jogwheel-autoswitch.py              run (Ctrl+C to stop)
    tools/jogwheel-autoswitch.py --verbose    print every focused window's class/title
    tools/jogwheel-autoswitch.py --install    start automatically at login
"""
import argparse
import glob
import os
import re
import shutil
import subprocess
import sys
import termios
import threading
import time

PORT_GLOB = "/dev/serial/by-id/usb-Raspberry_Pi_Pico_*-if00"


def find_port(explicit=None):
    if explicit:
        return explicit if os.path.exists(explicit) else None
    ports = sorted(glob.glob(PORT_GLOB))
    return ports[0] if ports else None


def send(port, line):
    """Open the port, write one command line, close. Never reads (so it can't
    steal replies from the web configurator), never sits on the port."""
    try:
        fd = os.open(port, os.O_WRONLY | os.O_NOCTTY | os.O_NONBLOCK)
    except OSError:
        return False  # unplugged, or held exclusively (e.g. configurator open)
    try:
        # Make sure the line isn't at 1200 baud: closing at 1200 reboots the
        # Pico into its bootloader (that's how uploads work).
        attrs = termios.tcgetattr(fd)
        attrs[4] = attrs[5] = termios.B115200
        termios.tcsetattr(fd, termios.TCSANOW, attrs)
        os.write(fd, (line + "\n").encode("utf-8", "replace"))
        termios.tcdrain(fd)
        return True
    except OSError:
        return False
    finally:
        os.close(fd)


def window_info(win_id):
    """Return (wm_class, title) for an X window id."""
    try:
        out = subprocess.run(
            ["xprop", "-id", win_id, "WM_CLASS", "_NET_WM_NAME"],
            capture_output=True, text=True, timeout=2,
        ).stdout
    except (subprocess.SubprocessError, OSError):
        return "", ""
    cls = title = ""
    for line in out.splitlines():
        if line.startswith("WM_CLASS"):
            cls = " ".join(re.findall(r'"([^"]*)"', line))  # "instance", "Class"
        elif line.startswith("_NET_WM_NAME"):
            m = re.search(r'=\s*"(.*)"\s*$', line)
            title = m.group(1) if m else ""
    return cls, title


def watch_focus():
    """Yield the active window id every time focus changes (no polling)."""
    proc = subprocess.Popen(
        ["xprop", "-root", "-spy", "_NET_ACTIVE_WINDOW"],
        stdout=subprocess.PIPE, text=True,
    )
    for line in proc.stdout:
        m = re.search(r"window id # (0x[0-9a-fA-F]+)", line)
        if m and int(m.group(1), 16) != 0:
            yield m.group(1)


def install():
    src = os.path.abspath(__file__)
    autostart = os.path.expanduser("~/.config/autostart")
    os.makedirs(autostart, exist_ok=True)
    path = os.path.join(autostart, "jogwheel-autoswitch.desktop")
    with open(path, "w") as f:
        f.write(
            "[Desktop Entry]\n"
            "Type=Application\n"
            "Name=Jogwheel auto layer switch\n"
            "Comment=Switch macropad layers based on the focused app\n"
            f'Exec=/usr/bin/env python3 "{src}"\n'
            "X-GNOME-Autostart-enabled=true\n"
            "NoDisplay=false\n"
        )
    print(f"Installed {path}\nIt starts at your next login. To start it now:\n  {src} &")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--port", help=f"serial port (default: {PORT_GLOB})")
    ap.add_argument("-v", "--verbose", action="store_true", help="print focused windows")
    ap.add_argument("--install", action="store_true", help="add to desktop autostart and exit")
    args = ap.parse_args()

    if args.install:
        install()
        return
    if not os.environ.get("DISPLAY"):
        sys.exit("No X11 display found. This script supports X11 sessions only.")
    if not shutil.which("xprop"):
        sys.exit("xprop not found - install it with: sudo apt install x11-utils")

    state = {"last": None, "port": None}
    lock = threading.Lock()

    def report(cls, title):
        with lock:
            state["last"] = (cls, title)
            port = find_port(args.port)
            ok = bool(port) and send(port, f"APP {cls}\t{title}".replace("\n", " "))
            if args.verbose:
                print(f"{'->' if ok else '  '} class={cls!r} title={title!r}", flush=True)

    # Re-send the current window whenever the pad (re)appears, so it picks the
    # right layer after being plugged in or after a firmware upload.
    def watch_plug():
        while True:
            port = find_port(args.port)
            if port and not state["port"] and state["last"]:
                time.sleep(1.5)  # let it finish booting
                report(*state["last"])
            state["port"] = port
            time.sleep(2)

    threading.Thread(target=watch_plug, daemon=True).start()
    if args.verbose:
        print("Watching focus changes (Ctrl+C to stop)...", flush=True)
    try:
        for win in watch_focus():
            report(*window_info(win))
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
