# Input selection

The user picks which mic fsa captures from, and which channel of it. The Input dropdown offers `Auto (last plugged in)` (the default), `System default`, and every audio source by description. Choosing a device pins it. Auto follows the most recently plugged-in hardware mic and falls back when it is unplugged. The Channel dropdown picks `Left` (input 1 of an interface, the default), `Right`, or `L+R`. The same choices exist as command-line options.

## Sub-features

- `input-pick` captures from the device chosen in the dropdown, and persists `input_mode = "device"` and `input`.
- `input-default` follows the session's default source.
- `input-auto` follows the newest plugged-in hardware mic.
- `input-channel` uses the left channel, the right channel, or their mix.
- `input-cli` sets the starting input with `--input <name|auto|default>` and the channel with `--channel left|right|mix`, and lists sources with `--list`.

## How to get to it (user POV)

- Click the Input dropdown in the top bar and choose an entry.
- Click the Channel dropdown next to it and choose `Left`, `Right` or `L+R`.
- Run `fsa --list`, `fsa --input <name>`, or `fsa --channel right`.

## Driving it with verify.sh

Preconditions:

- Baseline state (see README).

- **Dropdown contents.** Run `$V click 258 27` and `$V shot input-menu`. The menu lists `Auto (last plugged in)`, `System default`, a divider, then the sources, including `fsa verify mic`, which is highlighted. Close it with `$V key Escape`.
- **Channel.** Run `$V signal left-only synth 2 sine 440 vol -12dB sine 440 vol -100dB`. That is a stereo file whose right channel is near silent; check it with `sox <file> -n remix 2 stats`. With `Left`, `play-record` it and the output carries the tone. Open the Channel dropdown (`$V click 420 27`), take a `shot` to find the `Right` item, click it, and play again. The output drops to near silence. `$V config | grep input_channel` prints `"right"`.
- **CLI list.** Run `LD_LIBRARY_PATH=$(nix develop -c sh -c 'echo $LD_LIBRARY_PATH') ./target/release/fsa --list` while a run is up. Stdout has one `node.name<TAB>description` line per source, including `fsa-verify.mic	fsa verify mic`. It exits after about 0.5 s and opens no window.
- **Pick a device.** Choosing another real device in the dropdown switches capture away from the test mic, and `fsa.log` gains `switching input to <node.name>`. Pick `fsa verify mic` again (the last item) to switch back.

## Gotchas

- `--list` starts the backend, so it also creates `fsa.processed` for half a second. Never run it while the user's own fsa is running.
- Auto (`input-auto`) cannot be proven with the test mic. Only sources with a `device.id` (real hardware) count as "plugged in", and the loopback has none. Proving it needs a real USB mic plugged in during the run, which an agent cannot do. Report it as not verified, and point at the unit tests in `src/audio/picker.rs` for the policy.
- Choosing `Auto` or `System default` in the dropdown makes fsa capture the user's real mic. Return to `fsa verify mic` before playing test signals.
- The `sox synth` channel syntax above (two generators give two channels) has not been run yet. Check the file with `sox --i` and per-channel `stats` before relying on it.
