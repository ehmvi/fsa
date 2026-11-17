# Virtual mic and global bypass

While fsa is open it publishes a mono PipeWire source named `fsa.processed`, described as "Mic (processed)". Other apps (Discord) record it as a mic, and what they hear is the selected input after the chain. The `ACTIVE` chip at the top right is a global bypass: when it reads `BYPASSED`, the output is the raw input. The node disappears when fsa closes.

## Sub-features

- `vmic-node` creates `fsa.processed` ("Mic (processed)") while fsa runs, and removes it on exit.
- `vmic-audio` carries the input, processed, at roughly unity gain for speech-level sound.
- `vmic-bypass` passes the raw input through while the chip reads `BYPASSED`, and persists `bypass = true`.
- `vmic-status` shows `running` in the status line, plus an underrun count when the output starved.

## How to get to it (user POV)

- Start fsa. Pick "Mic (processed)" as the input device in another app.
- Click `ACTIVE` at the top right to bypass everything, and click `BYPASSED` to resume.

## Driving it with verify.sh

Preconditions:

- Baseline state (see README).

- **Node exists.** Run `pw-dump | grep -A2 '"node.name": "fsa.processed"'` (`doctor` also checks its owner). The node is present, `node.description` is `Mic (processed)`, and the class is `Audio/Source`.
- **Audio flows.** Run `$V signal tone1k synth 2 sine 1000 vol -12dB` and `$V play-record vmic-active $($V evidence)/tone1k.wav`. In `vmic-active.stats.txt` the output `Pk lev dB` is within about 1 dB of the input's -12 dB. A 440 Hz tone measured -11.6 dB peak when this skill was created.
- **Bypass.** Run `$V shot vmic-active`, then `$V click 975 27` and `$V shot vmic-bypassed`. The chip reads `BYPASSED`, with a grey dot. Play a signal that the chain changes audibly, such as `$V signal hum synth 2 sine 50 vol -12dB`, through `play-record` both active and bypassed. The default 80 Hz low cut attenuates the 50 Hz tone while active, and the bypassed output matches the input.
- **Persistence.** Run `sleep 1.5; $V config | grep bypass`. It prints `bypass = true`.
- **Node removed.** Run `$V down`, then `pw-dump | grep -c fsa.processed`. It prints `0`.

## Gotchas

- Bypass is global and separate from the per-module switches in the chain. A module turned off in the chain stays off after bypass ends.
- The output is mono. `play-record` records one channel.
- Nobody consuming the output is normal: the ring buffer fills and samples are dropped. The meters keep moving because the engine runs in the capture callback.
