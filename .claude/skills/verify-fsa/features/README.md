# fsa verification map

This directory is the maintained source for verifying fsa's user-facing behaviour. Read this index before driving the app, then use the matching feature file as the recipe. Every command is `$V <subcommand>`, where `V=.claude/skills/verify-fsa/scripts/verify.sh`, run from the repo root.

## Baseline preconditions

- `$V build` has run since the last change to `src/`.
- No other fsa runs on the PipeWire session (`up` refuses otherwise; never kill the user's fsa).
- `$V up` succeeded and `$V doctor` printed only `ok` lines.
- Baseline state: default config (chain 01 Noise suppression, 02 Equalizer, 03 Noise gate, all on), global `ACTIVE`, Input `fsa verify mic`, Channel `Left`, Noise suppression panel selected.

## Driving conventions

- Start each recipe from the baseline unless its preconditions say otherwise. A fresh `down` and `up` restores it.
- Coordinates are window-relative at 1040x700 (see the table in `../SKILL.md`). After every click that matters, take a `shot` and check the image before going on.
- Change state through the UI. Use a prepared `--config` only for preconditions that the journey is not about, and say so in the report.
- Put the feature ID in every artifact name (`$V shot gate-on-closed`, `$V play-record gate-on …`).

## Proof and skip reporting

- UI proof: a `shot` before and after the action, showing the changed control.
- Audio proof: `play-record` with the feature on and with it off (or against the input file), and the RMS of the same segment in each. Report the numbers.
- Persistence proof: `$V config` about 1.5 s after the change shows the new value.
- Report an unreachable path with the attempted command and the unmet precondition. Do not report it as verified through a different path.

## Feature entry contract

Each feature file starts with an H1 title and one paragraph on the user-visible behaviour, then exactly four H2 sections in this order: `Sub-features`, `How to get to it (user POV)`, `Driving it with verify.sh`, `Gotchas`.

## Features

- [Virtual mic and global bypass](./virtual-mic.md): "Mic (processed)" carries the processed signal, and `ACTIVE`/`BYPASSED` toggles all processing.
- [Noise gate](./noise-gate.md): mutes the mic between words, with a chain on/off switch, knobs and persistence. Proven end to end when this skill was created.
- [Equalizer](./equalizer.md): the default low cut, and adding or shaping bands on the graph.
- [Input selection](./input-selection.md): the Input and Channel dropdowns, and the `--input`, `--channel` and `--list` options.
- [Plugins](./plugins.md): Add plugin browser, CLAP/LV2 effects in the chain.

Not mapped yet: noise suppression on its own (strength, voice gate), chain reordering (⏶ ⏷), EQ dynamics, analyzer modes, and macOS output selection.
