# Equalizer

The equalizer shapes the mic's tone with up to 24 bands, drawn as a response curve over a spectrum analyzer. A fresh config has one band: a low cut at 80 Hz. Double-clicking the graph adds a band, dragging a node moves it, scrolling over a node changes its Q, right-clicking offers every shape, and `s` solos the selected band. Presets, Undo, Redo and Reset sit above the graph, and Analyzer, Range and Output sit below it.

## Sub-features

- `eq-lowcut` attenuates rumble under 80 Hz with the default band.
- `eq-add` adds a band where the graph is double-clicked. The band count reads `N/24 bands`.
- `eq-shape` drags a node to change frequency and gain, scrolls to change Q, and right-clicks to change the shape.
- `eq-undo` reverts with Undo and Redo, and Reset returns to the default band.
- `eq-persist` writes `[eq]` and its `[[eq.bands]]` to the config.

## How to get to it (user POV)

- Click `Equalizer` (row 02) in the CHAIN list.
- Work on the graph, the band editor under it, and the controls in the bottom bar.

## Driving it with verify.sh

Preconditions:

- Baseline state. Turn off rows 01 and 03 (`$V click 76 126`, `$V click 76 218`) so that only the EQ shapes the signal, then select the EQ with `$V click 130 172`.

- **Low cut.** Run `$V signal hum synth 2 sine 50 vol -12dB` and `$V signal mid synth 2 sine 1000 vol -12dB`, then `play-record` each one. Compare the output RMS at 0.8 to 2.0 s with the input. The 1 kHz tone is unchanged and the 50 Hz tone is clearly lower. Measure it and report the number; the curve shows about -8 dB at 50 Hz.
- **Add a band.** Run `$V dblclick 647 279` (1 kHz on the 0 dB line), then `$V shot eq-added`. Node `2` appears at 1 kHz, the band editor shows it, and the bottom right reads `2/24 bands`.
- **Boost it.** Run `$V drag 647 279 647 190` (up about 7 dB) and `$V shot eq-boosted`. The curve has a bump at 1 kHz. Play `mid` again: the 1 kHz output RMS is several dB above the input. Report the number next to the gain shown in the band editor.
- **Persistence.** Run `sleep 1.5; $V config`. There are two `[[eq.bands]]`, the second with `freq` near 1000 and a positive `gain`.
- **Undo.** Click Undo (`$V click 743 86`) twice. The band count goes back to `1/24 bands`.

## Gotchas

- Positions on the graph are logarithmic: 20 Hz at x=340, 100 Hz at 466, 1 kHz at 647, 10 kHz at 829. Convert with x = 647 + 181 * log10(f / 1000).
- The 0 dB line is y=279 at the default ±12 dB range. Changing RANGE rescales the y axis.
- Double-clicking an existing node toggles bypass on that band instead of adding one. Add new bands away from existing nodes.
- The analyzer only draws while signal flows, so take spectrum screenshots during `play-record`, not after it.
- This recipe was mapped from the UI and the code when the skill was created but has not yet been run end to end. Confirm the coordinates and the expected numbers on the first run, and fix this file.
