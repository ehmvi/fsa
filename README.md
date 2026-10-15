# fsa

Small real-time microphone processor for PipeWire. Takes a capture source,
runs it through a processing chain and exposes the result as a virtual mic
called **Mic (processed)** that Discord (or anything else) can select.

## Build / run (NixOS)

```sh
nix develop          # rust toolchain, pipewire, libclang, wayland/gl libs
cargo run --release -- --list               # show capture sources (node.name)
cargo run --release -- --input <node.name>  # start processing

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
