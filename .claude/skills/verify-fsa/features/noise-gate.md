# Noise gate

The noise gate mutes the mic between words. While the level stays under the threshold the output drops by the range (default threshold -50 dB, range -40 dB), and speech-level sound passes unchanged. The user turns it on or off with its switch in the chain, sets it with knobs in its panel, and the settings survive a restart.

## Sub-features

- `gate-select` shows the Noise gate panel when its chain row is clicked.
- `gate-switch` turns the gate on and off from the chain row. The panel shows `BYPASSED` when it is off.
- `gate-attenuate` drops sound under the threshold by the range, and passes sound above it.
- `gate-knobs` changes threshold, range, attack, hold and release by drag or scroll, and resets one by double-click.
- `gate-persist` writes `[gate]` values and the chain `enabled` flag to the config.

## How to get to it (user POV)

- Click `Noise gate` (row 03) in the CHAIN list.
- Click the switch on row 03 to bypass or enable it.
- In the panel, drag, scroll or double-click the THRESHOLD, RANGE, ATTACK, HOLD and RELEASE knobs.

## Driving it with verify.sh

Preconditions:

- Baseline state (see README).
- Noise suppression and the equalizer are turned off in the journey itself, so that only the gate shapes the signal. RNNoise would remove the noise segment on its own.

- **Signal.** Build 1 s of "voice" (a tone well above the threshold), then 2 s of "room" (noise well under it). Run `$V signal voice synth 1 sine 440 vol -12dB`, `$V signal room synth 2 whitenoise vol -60dB`, then `E=$($V evidence); $V concat voice-then-room $E/voice.wav $E/room.wav`. The input measures about -15 dB RMS on the tone and -65 dB on the noise.
- **Isolate the gate.** Switch off rows 01 and 02, then select the gate. Run `$V click 76 126`, `$V click 76 172`, `$V click 135 218`, then `$V shot gate-only`. The chain header reads `1/3 on`, rows 01 and 02 have grey switches, and the panel title is `Noise gate`, not dimmed, with no `BYPASSED` chip.
- **Gate on.** Run `$V play-record gate-on $E/voice-then-room.wav`. Then run `$V rms $E/gate-on.out.wav 0.7 0.6` for the tone and `$V rms $E/gate-on.out.wav 2.6 0.6` for the settled noise. The tone is within 0.5 dB of the input. The noise is about 40 dB below the input's noise. When this skill was created: tone -15.0 dB, noise -104.7 dB against -64.7 dB.
- **Gate off.** Run `$V click 76 218`, `$V shot gate-bypassed`, `$V play-record gate-off $E/voice-then-room.wav`, and the same two `rms` calls. The panel shows a `BYPASSED` chip and the title is dimmed. The noise now matches the input (about -64.7 dB). This is the control.
- **Persistence.** Run `sleep 1.5; $V config`. The `kind = "denoise"`, `"eq"` and `"gate"` chain entries each show `enabled = false`.
- **Knobs.** Run `$V scroll 363 413 up 5` and `$V shot gate-threshold`. THRESHOLD reads about -18.0 dB, in amber. Then run `$V dblclick 363 413`, `sleep 1.5`, and `$V config | grep threshold_db`. The value is back to `-50.0`.
- **Proof.** `gate-only.png`, `gate-bypassed.png`, `gate-on.stats.txt`, `gate-off.stats.txt`, and the four `rms` numbers, with the on-versus-off difference on the noise segment.

## Gotchas

- Measure the noise from 2.6 s in the recording onwards. Between 1.5 and 2.5 s the gate is still in its hold and release, and the attenuation is only partly there (about -87 dB at 2.0 s).
- Leaving noise suppression on makes the gate look like it works even when it is off, because RNNoise already kills the noise. Always compare gate on with gate off, with denoise off.
- The `CLOSED`/`OPEN` chip and the level bar in the panel only move while signal is flowing. A screenshot taken after playback ends always shows `CLOSED` and `-∞ dB`.
- Scrolling leaves the tooltip up because the pointer stays on the knob. Take the `shot` anyway, or `click` somewhere neutral first.
