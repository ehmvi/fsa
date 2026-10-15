# fsa

Small real-time microphone processor. Takes a capture source, runs it
through a processing chain and exposes the result as a virtual mic that
Discord (or anything else) can select: **Mic (processed)** via PipeWire on
Linux, **BlackHole 2ch** on macOS.

## Build / run (NixOS)

```sh
nix develop          # rust toolchain, pipewire, libclang, wayland/gl libs
cargo run --release -- --list               # show capture sources (node.name)
cargo run --release -- --input <node.name>  # start the gui (input can also be picked there)

nix build            # or: nix run
```

The flake builds for x86_64/aarch64 Linux and aarch64 macOS (see below).

To install it on NixOS, add the package; there's no module because fsa
creates its virtual mic as a normal PipeWire client, so nothing
system-level is needed:

```nix
# flake.nix of your nixos config
inputs.fsa.url = "github:ehmvi/fsa";
# ...
environment.systemPackages = [ inputs.fsa.packages.${pkgs.stdenv.hostPlatform.system}.default ];
```

It needs PipeWire with the PulseAudio shim, since Discord sees inputs
through it: `services.pipewire = { enable = true; pulse.enable = true; };`.

`--channel left|right|mix` selects which channel of the source is used
(default `left`). The source is captured as stereo, so for an interface
like the UR22C with the mic on input 1, `left` avoids the -6 dB you'd get
from a mono downmix.

## Verifying the virtual mic

```sh
pw-cli ls Node | grep -A3 fsa              # fsa.capture + fsa.processed
pw-record --target fsa.processed /tmp/t.wav  # speak, ctrl+c, play it back
pw-top                                     # check quantum / xruns
```

In Discord: Settings → Voice & Video → Input Device → **Mic (processed)**.

## macOS (nix-darwin)

Add the flake's nix-darwin module. It installs fsa (as `fsa.app`, plus
`fsa` on PATH) and the BlackHole 2ch driver:

```nix
# flake.nix of your nix-darwin config
inputs.fsa.url = "path:/path/to/fsa"; # or wherever the repo lives
# ...
darwinConfigurations.<host> = nix-darwin.lib.darwinSystem {
  modules = [
    inputs.fsa.darwinModules.default
    { programs.fsa.enable = true; }
  ];
};
```

`darwin-rebuild switch` copies the driver to `/Library/Audio/Plug-Ins/HAL`
(coreaudiod only loads drivers from there) and restarts coreaudiod when it
changes. Options: `programs.fsa.blackhole.enable` (set it to `false` if
you already installed BlackHole another way) and
`programs.fsa.blackhole.channel` (`"2ch"` by default). Disabling the
module doesn't remove an installed driver; delete
`/Library/Audio/Plug-Ins/HAL/BlackHole2ch.driver` and run
`sudo killall coreaudiod` to remove it.

Without the module: `nix build .#blackhole`, copy
`result/Library/Audio/Plug-Ins/HAL/BlackHole2ch.driver` there yourself,
then `nix run .` (or `nix develop` + `cargo run --release`).

macOS has no PipeWire, and an app can't publish a virtual mic by itself.
On macOS, fsa captures your input through CoreAudio, processes it, and
plays the result into the **BlackHole** virtual audio device. In Discord,
pick **BlackHole 2ch** as the input device.

- **Output** in the top bar picks the device to play into. By default it
  uses the first BlackHole device, and it never falls back to your
  speakers (that would feed the mic back into itself).
- Both devices run at 48 kHz float. The two device clocks drift, so fsa
  keeps a ~10 ms cushion and slips single samples (with interpolation) to
  hold it steady.
- **Test signal (no mic)** in the input list feeds a tone, which is handy
  for checking the routing without a mic.
- The first run asks for microphone access for whichever app launched
  fsa (e.g. your terminal).

## How it works

Two PipeWire streams, like `module-loopback`: a capture stream linked to the
chosen source and a mono playback stream with `media.class = Audio/Source`.
The processing chain runs in the capture callback. The capture callback
passes samples to the playback callback through a lock-free `rtrb` ring.
Both streams share `node.group` (same driver, no drift) and
`node.link-group` (the capture never links to our own source). Requested
quantum is 256 frames at 48 kHz.

## Built-in processing

- **Noise suppression**: RNNoise (via the pure-Rust `nnnoiseless`) with
  a strength (dry/wet) control and an optional voice gate that mutes the
  output while RNNoise's speech probability is below a threshold. It
  works on 10 ms frames, so it adds a fixed 10 ms of latency. Note:
  nnnoiseless sets up its FFT plans lazily, so the first denoised frame
  allocates once on the audio thread; nothing allocates after that.
- **Equalizer** (modelled on FabFilter Pro-Q), zero latency:
  - up to 24 bands: bell, low/high shelf, tilt shelf, low/high cut
    (6–96 dB/oct), notch, band pass and all pass;
  - analog-matched filters, so bells and shelves keep their shape near
    20 kHz instead of squashing like textbook (bilinear) biquads;
  - dynamic EQ per band (threshold, range, attack, release), e.g. for
    de-essing;
  - band solo, output gain, undo/redo;
  - a pre/post spectrum analyzer behind the curve.

  On the graph: double-click to add a band (cuts near the edges), drag a
  node (shift for fine control), scroll over it for Q, double-click it to
  bypass it, right-click for its menu, Delete to remove it, Cmd/Ctrl+Z to
  undo. Configs from the old fixed 5-band EQ are converted on load.
- **Noise gate** with hysteresis, hold and range.

Every chain item crossfades over 10 ms when switched on or off.

## CLAP plugins

**Add plugin…** scans `$CLAP_PATH`, `~/.clap`, `/run/current-system/sw/lib/clap`,
`lib/clap` in every `$NIX_PROFILES` entry,
`/etc/profiles/per-user/$USER/lib/clap` (home-manager) and `/usr{,/local}/lib/clap`.
So adding a plugin to `environment.systemPackages` or `home.packages` is
enough for it to show up.
On macOS it also scans `~/Library/Audio/Plug-Ins/CLAP` and
`/Library/Audio/Plug-Ins/CLAP` (and loads `.clap` bundles directly).

- Effects only: the mono mic signal goes into every channel of the plugin's
  main input, and the main output channels are averaged back to mono.
- **Edit** opens the plugin's own editor in a separate window: Cocoa on
  macOS; on Linux an X11 window (through XWayland on Hyprland) with class
  `fsa-plugin`, since CLAP on Linux only standardises X11. To float them:
  `windowrule = float, class:fsa-plugin`. Plugins that can only create
  their own floating window are supported too.
- Every plugin also gets a generic parameter list (double-click a slider
  to reset it).
- Plugin state is stored base64-encoded in the config. A plugin that fails
  to load stays in the chain as *missing* (bypassed) so its settings
  aren't lost.

To run the end-to-end hosting test against any plugin:
`FSA_TEST_CLAP=/path/to/plugin.clap cargo test hosts_plugin`.

## LV2 plugins

LV2 effects show up in the same **Add plugin…** browser (x86_64 Linux
only: `lilv` 0.2 doesn't build where `c_char` is unsigned, e.g. aarch64).
Search path: `$LV2_PATH`, `~/.lv2`, `/run/current-system/sw/lib/lv2`,
`lib/lv2` in every `$NIX_PROFILES` entry, the home-manager profile and
`/usr{,/local}/lib/lv2`.

- Instruments are hidden; anything with audio in and out is listed.
- Plugin UIs aren't hosted. Every plugin gets generic controls:
  checkboxes for toggles, dropdowns for enumerations, sliders otherwise.
- Control values are saved by port symbol. Plugins that keep internal
  state beyond their controls (LV2 state extension) start fresh on load.
- **Rescan** builds a new lilv world, so newly installed plugins appear
  without a restart.

To run the end-to-end LV2 test against any effect (find URIs with `lv2ls`):
`FSA_TEST_LV2=<uri> cargo test hosts_lv2`.

## Settings

Everything (input, channel, bypass, chain order and on/off state, EQ and
gate parameters) lives in `~/.config/fsa/config.toml` (respects
`$XDG_CONFIG_HOME`; override with `--config <path>`). It's written about
a second after the last change and again on exit. If the file can't be
parsed, it's moved to `config.toml.bad` and defaults are used.
