---
name: verify-fsa
description: Drive the real fsa desktop app (egui GUI plus PipeWire virtual mic "Mic (processed)") the way a user does, and capture proof. Use it to show that a change to fsa's UI, audio chain (noise suppression, EQ, gate, CLAP/LV2 plugins), input selection or config persistence actually works. It launches fsa on a private X display against a virtual test mic, clicks and types in the window, plays test signals in, and records what comes out. Linux/PipeWire only.
---

# Verify fsa

fsa is a Rust/egui app. It captures a mic through PipeWire, runs it through a chain (noise suppression, EQ, gate, plugins), and publishes the result as a virtual source, `fsa.processed` ("Mic (processed)"). The user sees a window and picks that virtual mic in Discord. So a proof has two halves: what the **window** shows after a real click, and what **comes out of `fsa.processed`** when a known signal goes in.

Everything goes through one helper, run from the repo root:

```sh
V=.claude/skills/verify-fsa/scripts/verify.sh
$V            # lists subcommands
```

It re-execs itself inside `nix shell` with xdotool, imagemagick, sox and jq, pinned by this repo's `flake.lock`. The first call may download them. The repo's `nix develop` shell provides cargo and the runtime libraries.

## Isolation: read before launching

- **Display.** fsa runs on its own `Xvfb` display (`:90` and up), with `WAYLAND_DISPLAY` unset, so nothing appears on the user's desktop. Window size is fixed at 1040x700, scale 1, at 0,0. There is no window manager.
- **Audio.** It uses the **user's own PipeWire session**. No private PipeWire exists. fsa's node names (`fsa.processed`, `fsa.capture`) are global, so **only one fsa can run per session**. `up` refuses when those nodes already exist. If the user's own fsa is running, **do not kill it**: ask them to close it, or report the verification as blocked.
- **Test mic.** `up` creates a `pw-loopback` pair. You play into `fsa-verify.speaker` (Audio/Sink, "fsa verify speaker"), and fsa captures `fsa-verify.mic` (Audio/Source, "fsa verify mic"). Loopback nodes are filters, so they never become the default device, and the user's audio routing stays as it was. They do show up in other apps' device lists while a run is up.
- **Config.** fsa gets `--config <run>/scratch/config.toml` and its own `XDG_CONFIG_HOME`. The user's `~/.config/fsa/config.toml` is never read or written.
- **Input.** `up` passes `--input fsa-verify.mic` unless you pass your own `--input`. Without it, Auto mode would capture the user's real mic.

## Launch

```sh
$V build                 # nix develop -c cargo build --release; run it after any change to src/
$V up                    # prints the run dir; evidence goes to <run>/evidence
$V up --config preset.toml           # start from a prepared config (it is copied, never modified)
$V up --plugins nixpkgs#lsp-plugins  # also expose that package's CLAP and LV2 plugins
$V up --channel right                # extra arguments go to fsa as-is
```

`up` refuses a stale binary (one older than anything in `src/`, `Cargo.toml` or `Cargo.lock`). It returns only once all of these hold: fsa's log says `fsa::audio::pipewire] running` (the capture stream is streaming), the `fsa.processed` node exists, and the window is mapped. A warm `up` takes about 3 s. Runs live under `${FSA_VERIFY_ROOT:-/tmp/fsa-verify}/<run-id>/`, and `/tmp/fsa-verify/current` points at the live one.

## Doctor

```sh
$V doctor
```

Read-only. Run it right after `up`, and whenever anything looks off. Each line is `ok` or `FAIL`. It checks that Xvfb, the loopback and fsa are alive and are the processes this run started; that `fsa.processed` belongs to our fsa's pid (so you are not measuring someone else's fsa); that the capture stream's last state is `running`; that the binary is newer than `src/`; and that the window is mapped at 1040x700. It also prints any error lines from fsa's log. Any `FAIL` means do not drive this instance. Run `down`, fix the cause, and `up` again.

## Drive

The UI has **no accessibility tree** (eframe's `accesskit` feature is off in `Cargo.toml`). So the GUI is driven by window-relative coordinates, which are stable only because the window is always 1040x700 at scale 1. After every action that matters, take a `shot` and confirm the state from the image. Never assume a click landed.

```sh
$V click X Y [button]     # hover, click, then park the pointer off-window (hides hover-only tools)
$V dblclick X Y           # knob reset, EQ: add band / reset band
$V hover X Y              # hover-only tools (chain row ⏶ ⏷ 🗙) only render while hovered
$V drag X1 Y1 X2 Y2       # knobs (drag up = more), EQ nodes
$V scroll X Y up|down N   # knobs: one notch moves a lot (5 notches: gate threshold -50 -> -18 dB)
$V key Escape             # keys (xdotool names); focuses the window first
$V type "text"            # into the focused text field
$V shot NAME              # window screenshot -> evidence/NAME.png
$V config                 # the config fsa autosaved (about 1 s after the last change)
$V log                    # fsa's stderr log
```

Coordinates for the default layout (left chain panel 256 px wide):

| Target | X, Y |
| --- | --- |
| Global bypass chip (`ACTIVE` / `BYPASSED`) | 975, 27 |
| Input dropdown / Channel dropdown | 258, 27 / 420, 27 |
| Input menu items: Auto (last plugged in), System default, then devices sorted by description, 34 px apart from y=142 (the test mic is usually last) | 222, 60 / 202, 94 / 243, 142 … |
| Chain row 1, 2, 3 name (selects the panel) | 160, 126 / 130, 172 / 135, 218 |
| Chain row 1, 2, 3 on/off switch | 76, 126 / 76, 172 / 76, 218 |
| Add plugin button (moves down 46 px per extra chain row) | 90, 272 |
| Plugin browser, empty (no plugins found): search field / Rescan / close ×. With plugins the dialog is centred and sized by its content, and moves when the filter changes, so `shot` first. With lsp-plugins the search field was at 445, 315 and the × moved between x=681 and 717 at y=259 | 480, 299 / 694, 300 / 717, 243 |
| Gate knobs: threshold, range, attack, hold, release | 363, 413 / 476, 413 / 588, 413 / 700, 413 / 812, 413 |
| Denoise knobs: strength, voice gate | 363, 413 / 476, 413 |
| EQ graph: 0 dB line at y=279, about 181 px per decade, 1 kHz at x=647 | 287..889, 128..430 |
| EQ Undo / Redo / Reset | 743, 86 / 803, 86 / 862, 86 |
| Status line (`running`, underrun count, errors) | 14..400, 680 |

The default chain is 01 Noise suppression, 02 Equalizer, 03 Noise gate, with Noise suppression selected. If a layout change moves things, take a `shot`, find the new positions in the image, and update this table and the feature files.

Audio:

```sh
$V signal NAME synth 1 sine 440 vol -12dB   # any sox synth arguments -> evidence/NAME.wav (48 kHz stereo float)
$V concat NAME A.wav B.wav                  # segments back to back
$V play-record NAME IN.wav                  # plays IN into the test mic and records fsa.processed
                                            # -> evidence/NAME.out.wav and NAME.stats.txt
$V rms FILE START DURATION                  # RMS dBFS of a window
```

Recording starts 0.5 s before playback, and fsa adds tens of milliseconds (20 ms of it is noise suppression), so a segment that starts at `t` in the input is at about `t + 0.5` in the `.out.wav`. Measure inside segments, away from edges and gate or EQ release tails. Feed signals on the **left** channel (`signal` writes both) because the default Channel is Left.

The features to drive, with their exact steps, are in [`features/README.md`](features/README.md). Read it first.

## Evidence

Everything a run captures goes to `<run>/evidence/`: screenshots, input and output wavs, `*.stats.txt` (sox stats of input and output), and, after `down`, `fsa.log`, `fsa.cmd` (the exact command line) and the final `config.toml`. Proof standards:

- Drive the real user path: click the switch, knob or menu. Do not hand-edit the config to get the state you need. A prepared `--config` is only for preconditions that the journey is not about, and you say so.
- Capture the action and the resulting state: a `shot` before and after, plus the measured audio.
- Verify side effects: the audio that comes out of `fsa.processed`, and the autosaved config (`$V config`, wait about 1.5 s after the change). A screenshot of a moved knob does not prove that the audio changed.
- Compare against a control: the same signal with the feature off, or the input file itself. A single level without a baseline proves nothing.
- Report the numbers: segment, RMS in, RMS out, difference.

## Cleanup

```sh
$V down
```

It stops fsa first (SIGTERM, so fsa saves its config and removes its nodes), then the loopback, then Xvfb, each only if its pid is still the process this run started. It never kills by name. It copies the log, the command line and the final config into `evidence/`, deletes `scratch/`, and checks that no `fsa*` nodes remain. Evidence stays at `/tmp/fsa-verify/<run-id>/evidence` until you delete it. Run `down` after every run, including failed or abandoned ones. `up` will not start while `current` exists.

## Gotchas

- The status bar's underrun count rises each time a recorder attaches to or detaches from `fsa.processed` (`play-record` causes some). It is not a regression unless it climbs during steady playback.
- Keyboard input works only after the window has focus. `key` and `type` focus it first. Raw `xdotool key --window` events are ignored.
- Hover-only controls (chain row ⏶ ⏷ 🗙) need a `hover` on the row first, then a `click` on the control. `click` parks the pointer afterwards, so use one `click` for the control itself.
- Auto input mode only follows sources that have a `device.id` (real hardware). The loopback test mic has none, so the hotplug-follow behaviour cannot be proven with it. See [`features/input-selection.md`](features/input-selection.md).
- `up` with no `--input` still pins the test mic. To test Auto or System default, pass `--input auto` or `--input default` and accept that the user's real mic (or silence) is then the input. Do not play test signals expecting to hear them back.
- Plugins: nothing is installed system-wide on this machine. Use `--plugins nixpkgs#lsp-plugins` (pinned via the flake) to get CLAP and LV2 effects.
- macOS (CoreAudio into BlackHole) is not covered by this harness.
