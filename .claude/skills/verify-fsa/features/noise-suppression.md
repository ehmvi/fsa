# Noise suppression

RNNoise neural noise suppression removes stationary background noise (fans, hum, room hiss) from the mic signal at the cost of a fixed 20 ms of latency. The user controls the dry/wet `STRENGTH` blend and an optional `VOICE GATE` threshold driven by RNNoise's speech probability estimate, toggles it from its chain row, and the settings survive a restart.

## Sub-features

- `denoise-select` shows the Noise suppression panel when row 01 is clicked (selected by default at startup).
- `denoise-switch` turns suppression on and off from the chain row (or re-enables it via the panel's `BYPASSED` chip); the status bar shows `· 20 ms latency` only while it is active.
- `denoise-suppress` attenuates stationary noise when enabled while keeping tonal speech-band energy intact.
- `denoise-knobs` adjusts `STRENGTH` (0–100 %) and `VOICE GATE` (off / 1–99 %) by drag, scroll, or focused arrow keys, and resets by double-click or `Home`.
- `denoise-persist` writes `[denoise]` (`strength`, `vad_threshold`) and the chain `enabled` flag to the config.

## How to get to it (user POV)

- Click `Noise suppression` (row 01) in the CHAIN list.
- Click the switch on row 01 to bypass or enable it.
- In the panel, drag, scroll, or arrow-key the `STRENGTH` and `VOICE GATE` knobs.

## Driving it with verify.sh

Preconditions:

- Baseline state (see README).
- Equalizer and Noise gate are turned off in the journey itself so that only RNNoise shapes the signal.

- **Signal.** Build 2 s of stationary white noise at -30 dB and 2 s of a 440 Hz tone at -12 dB: `$V signal hiss synth 2 whitenoise vol -30dB` and `$V signal tone synth 2 sine 440 vol -12dB`.
- **Isolate noise suppression.** Switch off rows 02 and 03: `$V click 76 172`, `$V click 76 218`, then `$V shot denoise-only`. Row 01 is on, the chain header reads `1/3 on`, and the status bar shows `· 20 ms latency`.
- **Suppression on.** Run `E=$($V evidence); $V play-record denoise-on-hiss $E/hiss.wav` and `$V rms $E/denoise-on-hiss.out.wav 0.8 1.0`. The noise RMS drops well below the -30 dB input.
- **Suppression off.** Bypass row 01 (`$V click 76 126`), take `$V shot denoise-bypassed` (panel shows `BYPASSED`, status bar no longer shows `· 20 ms latency`), then run `$V play-record denoise-off-hiss $E/hiss.wav` and `$V rms $E/denoise-off-hiss.out.wav 0.8 1.0`. The noise now matches the input.
- **Knobs and persistence.** Re-enable by clicking the panel's `BYPASSED` chip (or row 01 switch), adjust `STRENGTH`, wait `sleep 1.5`, and run `$V config` to verify `[denoise]` `strength` and `vad_threshold` updated.

## Gotchas

- RNNoise runs on 480-sample (10 ms) frames and adds 20 ms of fixed latency; start RMS windows at `0.8 s` or later so the startup transient and latency offset have settled.
- The `VOICE` / `NO VOICE` chip and VAD bar only animate while audio is flowing; screenshots taken after playback ends show `NO VOICE` and `0 %` (or `BYPASSED` when off).
- Pure sine tones are not always classified as human speech by RNNoise's VAD, so test `VOICE GATE` with recorded speech or a speech-like signal rather than a plain sine wave.
