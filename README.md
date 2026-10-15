# fsa

Small real-time microphone processor for PipeWire. Takes a capture source,
runs it through a processing chain and exposes the result as a virtual mic
called **Mic (processed)** that Discord (or anything else) can select.

## Build / run (NixOS)

```sh
nix develop          # rust toolchain, pipewire, libclang, wayland/gl libs
cargo run --release -- --list               # show capture sources (node.name)
cargo run --release -- --input <node.name>  # start the gui (input can also be picked there)

nix build            # or: nix run
```

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

## How it works

Two PipeWire streams, like `module-loopback`: a capture stream linked to the
chosen source and a mono playback stream with `media.class = Audio/Source`.
The processing chain runs in the capture callback. The capture callback
passes samples to the playback callback through a lock-free `rtrb` ring.
Both streams share `node.group` (same driver, no drift) and
`node.link-group` (the capture never links to our own source). Requested
quantum is 256 frames at 48 kHz.

## CLAP plugins

**Add plugin…** scans `$CLAP_PATH`, `~/.clap`, `/run/current-system/sw/lib/clap`,
`lib/clap` in every `$NIX_PROFILES` entry,
`/etc/profiles/per-user/$USER/lib/clap` (home-manager) and `/usr{,/local}/lib/clap`.
So adding a plugin to `environment.systemPackages` or `home.packages` is
enough for it to show up.

- Effects only: the mono mic signal goes into every channel of the plugin's
  main input, and the main output channels are averaged back to mono.
- **Edit** opens the plugin's own editor. CLAP on Linux only standardises
  X11, so editors live in a separate X11 window (through XWayland on
  Hyprland) with class `fsa-plugin`. To float them:
  `windowrule = float, class:fsa-plugin`. Plugins that can only create
  their own floating window are supported too.
- Every plugin also gets a generic parameter list (double-click a slider
  to reset it).
- Plugin state is stored base64-encoded in the config. A plugin that fails
  to load stays in the chain as *missing* (bypassed) so its settings
  aren't lost.

To run the end-to-end hosting test against any plugin:
`FSA_TEST_CLAP=/path/to/plugin.clap cargo test hosts_plugin`.

## Settings

Everything (input, channel, bypass, chain order and on/off state, EQ and
gate parameters) lives in `~/.config/fsa/config.toml` (respects
`$XDG_CONFIG_HOME`; override with `--config <path>`). It's written about
a second after the last change and again on exit. If the file can't be
parsed, it's moved to `config.toml.bad` and defaults are used.
