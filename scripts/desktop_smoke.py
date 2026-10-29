#!/usr/bin/env python3
"""Exercise the actual desktop compositor and native input, saving evidence on failure."""
import argparse
import ctypes
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time
import traceback

if sys.platform == "win32":
    # Match Tauri's physical screen coordinates on scaled Windows desktops.
    ctypes.windll.shcore.SetProcessDpiAwareness(2)

import numpy as np
from PIL import ImageGrab

gui = None


def desktop_size():
    if sys.platform == "darwin":
        import Quartz
        bounds = Quartz.CGDisplayBounds(Quartz.CGMainDisplayID())
        return bounds.size.width, bounds.size.height
    return tuple(gui.size())


def preflight():
    global gui
    import pyautogui as gui
    gui.PAUSE = 0.1
    size = desktop_size()
    if size[0] < 1024 or size[1] < 768:
        raise RuntimeError(f"Interactive desktop must be at least 1024x768; got {size}")
    if sys.platform == "darwin":
        import Quartz
        if not Quartz.AXIsProcessTrusted():
            raise RuntimeError("macOS Accessibility permission is missing. Use a logged-in GUI runner and grant it to the runner/Python process.")
        if hasattr(Quartz, "CGPreflightScreenCaptureAccess") and not Quartz.CGPreflightScreenCaptureAccess():
            raise RuntimeError("macOS Screen Recording permission is missing. Grant it to the runner/Python process before running CI.")
    ImageGrab.grab()  # Fail early when no usable desktop/screenshot provider exists.


class Smoke:
    def __init__(self, app, output):
        self.output = output
        output.mkdir(parents=True, exist_ok=True)
        self.process = None
        self.last_state = None
        self.sequence = 0
        self.completed = []
        for name in ("state.json", "control.json", "result.json"):
            (output / name).unlink(missing_ok=True)
        env = dict(os.environ, BTEXAMPLE_SMOKE_DIR=str(output), RUST_BACKTRACE="1")
        self.log = (output / "app.log").open("w", encoding="utf-8")
        prefix = json.loads(os.environ.get("BTEXAMPLE_LAUNCH_PREFIX", "[]"))
        self.process = subprocess.Popen(prefix + [str(app)], env=env, stdout=self.log,
                                        stderr=subprocess.STDOUT, start_new_session=os.name != "nt")

    def state(self):
        if self.process.poll() is not None:
            raise RuntimeError(f"App exited with {self.process.returncode}; see app.log")
        try:
            state = json.loads((self.output / "state.json").read_text(encoding="utf-8"))
        except (FileNotFoundError, json.JSONDecodeError, PermissionError):
            return None  # A telemetry write may be in progress.
        self.last_state = state
        if state.get("errors"):
            raise AssertionError(f"Webview JavaScript errors: {state['errors']}")
        return state

    def wait(self, predicate, description, timeout=40):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            state = self.state()
            if state and predicate(state):
                return state
            time.sleep(0.2)
        raise AssertionError(f"Timed out: {description}; last state={self.last_state}")

    def desktop_rect(self, state):
        n = state["native"]
        # macOS input APIs use points; Tauri reports physical pixels.
        scale = n["scale"] if sys.platform == "darwin" else 1
        return [n[k] / scale for k in ("x", "y", "width", "height")]

    def capture(self, name, state):
        screen = ImageGrab.grab().convert("RGB")
        desktop_width, desktop_height = desktop_size()
        sx, sy = screen.width / desktop_width, screen.height / desktop_height
        x, y, w, h = self.desktop_rect(state)
        box = (round(x*sx), round(y*sy), round((x+w)*sx), round((y+h)*sy))
        if box[0] < 0 or box[1] < 0 or box[2] > screen.width or box[3] > screen.height:
            raise AssertionError(f"Window is outside the visible desktop: {box}, screen={screen.size}")
        image = screen.crop(box)
        image.save(self.output / f"{name}.png")
        return image

    @staticmethod
    def roi(image, rect, state):
        if not rect or rect["width"] <= 0 or rect["height"] <= 0:
            raise AssertionError("Missing/empty DOM rectangle")
        sx, sy = image.width / state["width"], image.height / state["height"]
        return np.asarray(image.crop((round(rect['x']*sx), round(rect['y']*sy),
                                      round((rect['x']+rect['width'])*sx),
                                      round((rect['y']+rect['height'])*sy)))).astype(np.int16)

    def visible(self, image, state):
        logo = self.roi(image, state["logo"], state)
        r, g, b = logo[..., 0], logo[..., 1], logo[..., 2]
        cyan = (g > 100) & (b > 100) & (g > r*1.35) & (b > r*1.35)
        # Restrict the 3D check to the translucent HTML panel: this verifies
        # scene-through-overlay composition, not merely two separate regions.
        panel = self.roi(image, state["panel"], state)
        r, g, b = panel[..., 0], panel[..., 1], panel[..., 2]
        green = (g > 65) & (g > r*1.2) & (g > b*1.2)
        return int(cyan.sum()) > 100 and int(green.sum()) > 80, green

    def render_check(self, name):
        deadline = time.monotonic() + 60
        first = None
        while time.monotonic() < deadline:
            state = self.state()
            if not state:
                time.sleep(.3)
                continue
            image = self.capture(name + "-latest", state)
            visible, green = self.visible(image, state)
            if visible:
                ys, xs = np.nonzero(green)
                center = np.array([xs.mean()/green.shape[1], ys.mean()/green.shape[0]])
                if first is None:
                    first = center
                    image.save(self.output / f"{name}-first.png")
                elif np.linalg.norm(center-first) > .025:
                    image.save(self.output / f"{name}-animated.png")
                    return state
            time.sleep(.35)
        raise AssertionError(f"{name}: React logo and animated green cube were not both visible through the panel. See screenshots; verify desktop/GPU availability.")

    def click(self, state, target):
        rect = state[target]
        x, y, w, h = self.desktop_rect(state)
        gui.click(x + (rect['x'] + rect['width']/2)*w/state['width'],
                  y + (rect['y'] + rect['height']/2)*h/state['height'])

    def greet(self, value, enter=False):
        state = self.state()
        self.click(state, "input")
        gui.hotkey("command" if sys.platform == "darwin" else "ctrl", "a")
        gui.write(value, interval=.25)
        self.wait(lambda s: s.get('name') == value, "native text entry")
        if enter:
            gui.press("enter")
        else:
            self.click(self.state(), "button")
        expected = f"Hello, {value}! You've been greeted from Rust-Bevy App!"
        state = self.wait(lambda s: expected in s['text'], "greet IPC response")
        # Let the asynchronous Linux snapshot catch up with the DOM.
        time.sleep(1)
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            state = self.state()
            image = self.capture(value + "-result", state)
            greeting = self.roi(image, state["greeting"], state)
            if int(np.all(greeting > 210, axis=2).sum()) > 30:
                break
            time.sleep(.3)
        else:
            raise AssertionError("Greeting exists in DOM but is not visible in the desktop screenshot")

    def run(self):
        state = self.wait(lambda s: 'Welcome to Tauri + React + Bevy' in s['text'] and
                          re.search(r'FPS:\s*[1-9][0-9]*', s['text']), "React + frame loop startup", 180)
        start_sequence = state['sequence']
        self.render_check("initial")
        self.completed += ["native desktop composition", "3D animation"]
        self.greet("ci-click")
        self.completed += ["native typing", "mouse submit + IPC"]
        self.greet("ci-enter", enter=True)
        self.completed.append("Enter submit + IPC")
        self.sequence += 1
        (self.output / "control.json").write_text(json.dumps({"sequence": self.sequence, "width": 960, "height": 700}), encoding="utf-8")
        self.wait(lambda s: abs(s['width']-960) <= 2 and abs(s['height']-700) <= 2, "native window resize")
        self.render_check("resized")
        self.greet("ci-resized")
        self.completed.append("resize + rendering + input")
        self.wait(lambda s: s['sequence'] > start_sequence + 3, "continued telemetry")
        self.result("passed")

    def result(self, status, error=None):
        (self.output / 'result.json').write_text(json.dumps({
            'status': status, 'platform': sys.platform, 'error': error,
            'completed_checks': self.completed,
            'last_state': self.last_state,
        }, indent=2), encoding='utf-8')

    def close(self):
        if self.process and self.process.poll() is None:
            if os.name == 'nt':
                subprocess.run(['taskkill', '/PID', str(self.process.pid), '/T', '/F'], check=False)
            else:
                os.killpg(self.process.pid, signal.SIGTERM)
            try:
                self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                if os.name != 'nt':
                    os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait(timeout=5)
        self.log.close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--app', required=True, type=Path)
    parser.add_argument('--output', type=Path, default=Path('artifacts/desktop-smoke'))
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    smoke = None
    try:
        preflight()
        smoke = Smoke(args.app.resolve(), output)
        smoke.run()
        print('PASS: desktop rendering, animation, mouse/keyboard IPC, and resizing')
    except Exception as error:
        (output / 'failure.txt').write_text(traceback.format_exc(), encoding='utf-8')
        try:
            ImageGrab.grab().save(output / 'failure-desktop.png')
        except Exception:
            pass
        if smoke:
            smoke.result('failed', str(error))
        raise
    finally:
        if smoke:
            smoke.close()


if __name__ == '__main__':
    main()
