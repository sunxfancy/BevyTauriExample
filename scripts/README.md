# Desktop rendering smoke tests

`desktop-smoke.yml` runs on pull requests, pushes to `main`/`master`, and manual
workflow dispatch. Each OS builds the embedded frontend with
`ci-smoke,custom-protocol`, starts the real Tauri/Bevy executable, and uses OS
screenshots and native mouse/keyboard input. It does not test a standalone
browser page or treat a successful build as a rendering pass.

Checks:

- React's cyan logo and Bevy's green cube appear together inside the HTML panel.
- The cube's position changes across screenshots (a frozen scene fails).
- Native typing, mouse submission, and Enter submission reach the Rust `greet`
  command and produce a visible response.
- Resizing to 960x700 keeps the scene animated and the UI interactive.
- JavaScript errors, process exits, missing frames, and timeouts fail the test.

This is a visual smoke test, not pixel-perfect reference-image comparison.
Color/position tolerances accommodate drivers and font differences. It does
not cover every browser widget, IME, GPU, display scaling setting, or graphics
backend. The matrix tests Linux Vulkan, Windows DX12, and macOS Metal. Linux
and Windows request software/fallback adapters; macOS requires a usable Metal
device. A missing capability is a failure, never a skipped success.

## Runner requirements

An unlocked primary desktop of at least 1024x768 is required. Linux CI creates
one with Xvfb/Openbox and Mesa. Windows must run in an interactive user session,
not a service-only Session 0. macOS requires a logged-in GUI session, a usable
Metal device, and Accessibility plus Screen Recording permissions for the
runner/Python process. The test checks these permissions without prompting or
trying to bypass them. Hosted macOS images may not provide all these facilities.

The default labels are `ubuntu-24.04`, `windows-2022`, and `macos-15`. If a hosted
runner cannot provide the desktop/GPU permissions, select a prepared runner
using repository Actions variables containing JSON label arrays, for example:

- `MACOS_RENDER_RUNNER`: `["self-hosted", "macOS", "render-test"]`
- `WINDOWS_RENDER_RUNNER`: `["self-hosted", "Windows", "render-test"]`
- `LINUX_RENDER_RUNNER`: `["ubuntu-24.04"]`

Only route trusted code to persistent self-hosted runners. The workflow uses
read-only repository permissions and does not publish or deploy anything.

## Local run

Install the platform's Tauri/Bevy development libraries and a Rust toolchain
compatible with the project. In a graphical desktop session, from the repo root:

```sh
yarn install --frozen-lockfile
yarn build
python -m pip install -r scripts/requirements-render.txt
python -m unittest discover -s scripts -p 'test_*.py'
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo build --locked --manifest-path src-tauri/Cargo.toml --features ci-smoke,custom-protocol
python scripts/desktop_smoke.py --app src-tauri/target/debug/btexample
```

Use `btexample.exe` on Windows. Headless Linux can wrap the final command in
`xvfb-run -a -s '-screen 0 1280x900x24' dbus-run-session -- ...` and start Openbox
inside the same display, as the workflow does.

The `ci-smoke` feature enables a small webview telemetry probe and a constrained
window-size control. The Python driver performs typing/clicking through the OS;
the probe only reports DOM state and geometry. The feature is absent from normal
builds. Screenshots are taken from the final desktop surface, so a healthy DOM
behind an invisible webview cannot pass.

Artifacts are uploaded even on failure: screenshots, `app.log`, `state.json`,
`result.json` (completed checks), and `failure.txt`. If a GUI permission check
fails before launching the app, `failure.txt` explains the requirement. Default
output is `artifacts/desktop-smoke`; `--output` selects another directory.
