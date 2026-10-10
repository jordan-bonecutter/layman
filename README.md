# Layman

A small native display-layout editor for **Sway**, written in Rust with egui.
No GTK, Qt, Electron, webview, or external `swaymsg` process in the application.
Wayland and OpenGL connect to the normal system graphics libraries.

![Layman running in headless Sway](artifacts/headless-ui.png)

## Run

Requires Linux, Sway, Rust 1.89+, and your system's Wayland/OpenGL libraries.

```sh
cargo run --locked
# Or explore without a Sway session or changing settings:
cargo run --locked -- --demo
```

For regular use, install at a stable path **before** saving your layout:

```sh
cargo install --path . --locked
layman
```

Drag monitors to match your desk. Edges snap; Alt bypasses snapping. Select a
monitor to edit its advertised resolution/refresh and fractional scale. The scale
control snaps to 0.25× increments, with minus/plus buttons for precise changes.
Four orientation icons select 0°, 90°, 180°, or 270°; the Mirror checkbox flips
horizontally.
Arrow keys over the canvas nudge the selected monitor; Shift nudges by 10 pixels.
Scroll over the canvas to zoom around the pointer. Drag empty canvas to pan, or
hold Space and drag anywhere (middle-drag also works). Dragging screen pixels
still moves that display. Labels and borders sit outside the screen image. The
four-corner icon fits the displays to the canvas. All toolbar
and footer icons have tooltips and keyboard focus.

Each monitor shows real screen pixels, captured directly through Wayland's
wlr-screencopy protocol and shared-memory buffers. Capture runs on a background
thread at native resolution, targeting 30 frames per second, with bounded waits.
The GPU scales previews to the canvas; capture and pixel conversion use shared
memory on the CPU. Actual refresh depends on the compositor and screen sizes.
Capture handles rotation,
mirroring, and output hotplug. The app needs no screenshot utility. Previews show
the currently running desktop; draft settings take effect when you press Preview.
The workspace tab assigns current and future workspaces to displays. The refresh
icon reloads the current output settings from Sway without blocking the UI.

The **Save layout** icon applies the draft for 20 seconds. **Confirm & save** makes it
persistent; Revert, timeout, or closing/crashing the editor restores the previous
outputs and workspace placement. A separate Rust helper handles rollback even
when the UI has gone away. It runs only during a preview. Changes must leave at
least one connected display enabled. Overlaps and disconnected edges show hints.

## Survive Sway reloads

After Confirm & save, add this line **at the end** of your Sway config, after any
other output/workspace rules:

```sway
include ~/.config/layman/sway.conf
```

If you use a custom `XDG_CONFIG_HOME`, get the correct absolute include line
with `layman --print-config`, or copy it from the dialog shown after saving.
If the config already contains the include, the dialog stays hidden. Otherwise
it checks the running compositor's actual config path, previews
an include change as a colored diff, and offers **Apply diff**. It recognizes a
direct include anywhere in the config and leaves it untouched. A missing include
is appended; the diff never removes existing configuration. It preserves
permissions, keeps a `config.layman-backup-*` backup
alongside the config, and refuses to overwrite changes made after the diff was
shown. It follows a config symlink to its target. Applying the diff does not reload
Sway; reload when ready. Checking the config is read-only.

Layman writes `layout.json` and `sway.conf` to `$XDG_CONFIG_HOME/layman`, defaulting
to `~/.config/layman`. The include applies enabled display settings synchronously
on reload and uses `exec_always /absolute/path/to/layman --restore` to restore
existing workspaces. Workspace moves carry tiled and floating windows together;
the existing tiling tree stays intact. Future workspace assignments are also
in the include. Existing workspace focus is restored after moves.

**No systemd service or permanent daemon needed.** Sway owns the lifecycle.
Keep the binary at the saved absolute path; save again if you move/reinstall it
somewhere else. Connected displays with known serials are matched by make/model/
serial, so connector renumbering can be resolved. Others use connector names.
Unknown connected outputs are left alone; missing saved outputs/workspaces are
skipped by restore. Disabling runs through the restore helper, with a check for
an enabled connected saved display, so an unplugged dock cannot make the include
disable your only remaining laptop screen. Restore refuses obsolete advertised
modes and leaves the session available for editing. To restore after plugging a
monitor in, run `layman --restore` or reload Sway. Automatic hotplug monitoring
and multiple named profiles are not included.

Both saved files use staged writes and atomic replacement; a failed second
replacement restores the previous include. They are two files, so a machine
crash between replacements is not a transaction across both. A process lock
serializes previews/restores and is automatically released after process exit.
Names containing Sway command metacharacters are rejected before applying.

## Command line

```sh
layman --inspect       # Read-only JSON of current displays and workspaces
layman --restore       # Apply saved settings to connected outputs/workspaces
layman --print-config  # Print the include line
layman --demo          # Sample desktop; no IPC or saved-setting writes
layman --help
```

## Tests

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo test --locked --test headless -- --ignored --nocapture
```

The headless test needs `sway`, `grim`, and Mesa software EGL/OpenGL. It starts
its **own** two-output compositor with a private runtime directory and IPC
socket, isolates configuration writes, renders the actual native UI, captures
`artifacts/headless-ui.png` and `headless-preview.png`, tests dragging and
Arrange → Save → Confirm with a real virtual Wayland pointer, moves a workspace
with a live Wayland window, tests the save dialog's config diff and Apply button,
checks real screen capture and output re-enabling, exercises Refresh after external
layout changes, checks save/reload, and waits out the real 20-second rollback deadline. It
never sends commands to your desktop session. Ordinary tests cover IPC framing,
validation, device matching, logical dimensions/snapping, persistence, and UI
rendering without a display server. CI runs both suites.

Protocol and behavior references: [Sway IPC](https://github.com/swaywm/sway/blob/master/sway/sway-ipc.7.scd),
[output commands](https://github.com/swaywm/sway/blob/master/sway/sway-output.5.scd),
and [workspace/include/exec_always commands](https://github.com/swaywm/sway/blob/master/sway/sway.5.scd).
