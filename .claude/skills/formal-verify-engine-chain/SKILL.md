---
name: formal-verify-engine-chain
description: Model-check (TLA+/TLC) the plugin-chain protocol between the gui thread and the audio thread, i.e. App::add_plugin/remove_item/move_item/collect_garbage in src/app.rs and Engine::apply_commands/process in src/engine.rs. Use when a change touches the engine Command ring, the garbage ring, MAX_SLOTS, slot removal/fading, chain reordering, or the gui's chain bookkeeping, or when asked whether the gui and the audio thread can disagree about the chain.
---

# Verify the gui/audio chain protocol

The gui (`App` in `src/app.rs`) owns the chain the user sees. It mirrors every
change to the audio thread (`Engine` in `src/engine.rs`) as a `Command`
(`Insert`, `Remove`, `Move { id, after }`) through an rtrb ring. Slots that
leave the engine come back through a second ring (the garbage ring) so they
are freed on the gui thread. `EngineChain.tla` in this folder models both
sides. TLC explores every interleaving of gui actions, audio blocks and fades
within small bounds.

## Properties

Each property says what must hold and what breaks for the user if it doesn't.

| Property | Must hold | Harm if broken |
| --- | --- | --- |
| `NoRealloc` | `Engine.slots` never grows past its capacity. | The audio thread reallocates, which can cause an xrun (an audible click or dropout). |
| `NoAudioThreadDrop` | The audio thread never drops a `Slot`. Every slot that leaves the engine goes into the garbage ring. | Plugin teardown and deallocation run on the real-time thread, which can glitch or crash inside plugin code. |
| `ChainAgrees` | Once the audio thread has drained the queue, it runs exactly the plugins the gui lists, in the same order. | The user sees, saves and tweaks a plugin that processes no audio. |
| `OrderAgrees` | The plugins both sides know about are in the same order. | Audio runs through the chain in a different order than the one shown. |
| `AwaitedComesBack` | Every id in `App.awaiting` still has a slot that can come back. | On exit, `App::teardown_plugins` waits the full `TEARDOWN_TIMEOUT` and then leaks the lilv worlds. |

## Does my change touch this?

The check covers the code below. The model was last confirmed to describe it at
**sync commit `5a48f0602d4801beb7a2d6be1cb0ed773bd7d4ca`** (`fix: stop adding plugins once the chain is full`).

| File | What the model mirrors |
| --- | --- |
| `src/engine.rs` | `MAX_SLOTS`; `Engine::new` (slot capacity, garbage ring `MAX_SLOTS * 2`); `Engine::apply_commands` (all three commands, the full-chain reject path, returning one faded slot per block when the ring has room); `Engine::process` (a removing slot fades out); `EngineHandle::send`, `EngineHandle::collect_garbage`; `Slot.removing` |
| `src/app.rs` | `App::new` (`next_id`), `App::has_room` (the add guard), `App::add_plugin`, `App::add_lv2`, `App::remove_item`, `App::move_item`, `App::collect_garbage`, `App::ui` (collects first in each frame, then at most one add and one row action), `chain_row` (built-ins have no remove button; moves go one step up or down), `Setup::from_config` (config item `k` gets id `k`), `App.awaiting` / `App::teardown_plugins` |
| `src/dsp/mod.rs` | `Fade::is_silent`, `Fade::mix`: a fade lasts some whole number of blocks (`Fade::MS`) |
| `src/audio/mod.rs` | `QUANTUM`: the block size, used only to argue that a fade spans more than one block |

To see whether the covered code changed since the sync commit:

```sh
git diff --stat 5a48f0602d4801beb7a2d6be1cb0ed773bd7d4ca -- src/engine.rs src/app.rs src/dsp/mod.rs src/audio/mod.rs
```

If none of the items in the table changed, the check still describes the code.
Run it anyway if your change is near this code. If any of them changed, update
the model first (see "When the code changes on purpose"), then run the check.

## Run it

```sh
.claude/skills/formal-verify-engine-chain/check.sh            # every property, about 15 s on a 24-core machine
.claude/skills/formal-verify-engine-chain/check.sh NoRealloc  # just one
```

Requirements: `nix` with flakes. The script fetches TLC 2.19 (nixpkgs `tlaplus` 1.7.4) from a fixed nixpkgs commit and runs one TLC job per property and bounds file. Each run prints `PASS`, `FAIL` with the path of the log that holds the counterexample trace, or `ERROR` for a parse or tool error, which is not a result. The script exits with 0 when every property passes, 1 when any property fails, and 2 on an error.

Read every TLC result, pass or fail, with the pstack `/statechart` skill. For a violation, it tells the shortest path in plain words. For a pass, it draws a chart of how the system moves between states.

### Expected result

```
PASS  EngineChain.cfg NoRealloc            82603 distinct states found
PASS  EngineChain.cfg NoAudioThreadDrop    82603 distinct states found
PASS  EngineChain.cfg OrderAgrees          82603 distinct states found
PASS  EngineChain.cfg ChainAgrees          82603 distinct states found
PASS  EngineChain.cfg AwaitedComesBack     82603 distinct states found
PASS  EngineChainStall.cfg NoAudioThreadDrop   101 distinct states found
```

Every property holds since the `App::has_room` guard fixed F1 and F2 (below).
Any `FAIL` is news: replay its trace as described in "Real bug or modeling
error?". A changed state count with every property passing means that the model
or the bounds changed. Explain that before you trust the pass.

The bounds were chosen so that they still catch the old bugs. With the guard
removed from `Add`, `OrderAgrees`, `ChainAgrees` and `AwaitedComesBack` fail
under `EngineChain.cfg`, and `NoAudioThreadDrop` fails under
`EngineChainStall.cfg`. With the weaker guard `Len(chain) < Cap`, `ChainAgrees`
still fails. If you change the bounds, repeat those three runs. A run of the
whole script on the unguarded model can take far longer than 10 minutes,
because the properties that pass enumerate a much larger state space, so check
only the properties you expect to fail. To run the stall file on its own, call
`tlc` the way `check.sh` does.

## Findings (fixed)

Both findings were fixed by `App::has_room` in `src/app.rs`. It allows an add
only while `chain.len() + awaiting.len() < MAX_SLOTS`. `add_plugin` and
`add_lv2` refuse with an error in the status line, and the `Add plugin` button
is disabled while the chain is full. The model's `Add` carries the same guard.
The notes below record what used to break, so that a regression is
recognisable.

**F1. The gui adds plugins with no capacity check, and the engine quietly rejects them.**
Neither `App::add_plugin` nor `App::add_lv2` checks `MAX_SLOTS`. When `Engine.slots` is full, `Command::Insert` hands the slot straight back through the garbage ring. A slot that is still fading out after `Remove` also counts toward that limit. `App::collect_garbage` then drops the returned unit, and for a CLAP plugin it does so without `deactivate`. The gui keeps the item in `App.chain`. Shortest traces, with the model's `Cap = 2` standing for 32:
- *ChainAgrees*: the user adds plugin 1 and then plugin 2, and one `Engine::process` runs. Plugin 2 is rejected. The gui shows `[0, 1, 2]` while the audio thread runs `[0, 1]`.
- *OrderAgrees*: after that, the user moves plugin 2 up one row, which the engine ignores because it has no slot 2, and then moves built-in 0 down below 2. The gui sends `Move { id: 0, after: Some(2) }`. Id 2 is not in the engine, so the `map_or(self.slots.len(), ..)` fallback sends 0 to the end. The gui shows `[2, 0, 1]` while audio runs `[1, 0]`, so the built-in now runs after plugin 1.
- *AwaitedComesBack*: the user then removes plugin 2. `App.awaiting` gains id 2, but no slot will ever come back for it, so `teardown_plugins` waits out `TEARDOWN_TIMEOUT` and leaks.
- Limiting the gui to `chain.len() < MAX_SLOTS` is **not** enough. A scratch model run with that guard still fails `ChainAgrees`: add plugin 1, remove it, then add plugin 2 while 1 is still fading out. The gui has 2 items, but the engine is full, so plugin 2 is rejected. A guard of `chain.len() + awaiting.len() < MAX_SLOTS` made every property pass under both bounds files. Those bounds leave little room, so raise `Cap` to 3 in `EngineChain.cfg` before you trust a fix.
- Confirmed on the real code at `MAX_SLOTS = 32` with a scratch unit test (not kept). It used 32 slots, `Remove(5)` and `Insert(99)` in one block. Slot 99 came back in the garbage ring, and a following `Move { id: 0, after: Some(99) }` put slot 0 last.

**F2. A pile of rejected inserts can make the audio thread drop a slot.** This is the stall bounds file. The command ring (256) is larger than the garbage ring (`MAX_SLOTS * 2`, 64) plus a full chain. While the audio thread is not processing, the gui can queue more `Insert`s than the garbage ring holds. When the audio thread then drains them in one `apply_commands`, it rejects each one into the garbage ring. Once that ring is full, `let _ = self.garbage.push(slot)` drops the slot on the audio thread. The trace: the chain is full, the user adds 3 plugins over two frames while no block runs, and then one block rejects all 3 into a 2-entry ring. In the real app this needs 65 or more queued adds beyond a full chain during a stall, so it is unlikely in practice, but it does break the engine's promise. The same fix as F1 removes it.

## Real bug or modeling error?

Replay the trace against the code, one step at a time. Each step's action name
names the code it stands for:

| Model action | Code |
| --- | --- |
| `Add` | `App::add_plugin` / `App::add_lv2`, which push `Command::Insert` |
| `RemoveItem` | `App::remove_item` |
| `MoveItem` | `App::move_item`, reached from `chain_row`'s up and down buttons |
| `Collect` | `App::collect_garbage`, at the top of `App::ui` |
| `Process` | one `Engine::process`: `apply_commands`, then the slot loop's fades |

- If a step cannot happen in the app, it is a modeling error, and you fix the model. Examples: the gui doing more than `FrameActs` actions between two collects, removing a built-in, or a command that the code would never send. Never fix it by changing the property.
- If every step can happen, it is a real bug. If you want proof in the real code, write a scratch test in `src/engine.rs`'s `mod tests` using `engine_with` and `run`, as F1 did, then delete it. Report the bug in the code's words. Do not change product code unless the user asks.
- An `ERROR` line is a TLA+ syntax or tool problem, not a result about the code.

## When the code changes on purpose

1. Mirror the change in `EngineChain.tla`, and keep each action next to the function it models. Each action's comment names its function.
2. If a new invariant describes a promise the code makes, add it to `RUNS` in `check.sh`.
3. Run `check.sh`. The result should match the expected table, or differ only in ways you can explain from your change. A fix for F1 or F2 should turn its failures into passes. Then update the expected table and the findings.
4. Move the sync commit (above, and in the diff command) forward only after you have confirmed that the model describes the covered code at the new commit.

## Bounds and assumptions

- `EngineChain.cfg` has `Cap = 3` slots (the code has 32), 1 built-in, up to 6 plugin adds, up to 3 queued commands, and 2 gui actions per frame. That is 82,603 distinct states, a couple of seconds per property.
- `EngineChainStall.cfg` has `Cap = 1`, no built-ins, up to 5 plugin adds and up to 4 queued commands. It keeps the code's ratio of command ring > garbage ring + chain, which the main bounds file does not reach. It is small (101 states) because the guard leaves the gui little to do with one slot, but it is the shape that exposed F2.
- The model treats one `Engine::process` as a single step. That is sound because the rings are FIFO single-producer, single-consumer queues. A gui push that races the drain loop is seen in this block or the next.
- How long a fade lasts is nondeterministic: a removing slot may finish fading in any block after its `Remove`.

## Not covered

- Command-ring overflow, where `EngineHandle::send` drops a command once 256 are queued. Gui actions stop at `CmdBound` instead.
- Slots that were already disabled when removed. In the code they come back one block sooner. Toggling `SlotState.enabled` is not modeled.
- Startup plugins from the config, which are modeled as built-ins that cannot be removed. The ids of oversized configs are covered (`EngCap`).
- Audio-level behaviour: the fade gain ramp, the reset-on-reenable in `Engine::process`, global bypass, and peak meters.
- Plugin-side lifecycles (`ClapPlugin::deactivate`, lv2 worlds) beyond whether a slot comes back.
- Anything outside the bounds, such as longer chains or more queued commands. A pass covers the model within its bounds, not the whole app.
