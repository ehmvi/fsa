# Plugins

`Add plugin` under the chain opens a browser of the CLAP and LV2 effects installed in the standard folders and in `$CLAP_PATH` and `$LV2_PATH`. Picking one appends it to the chain, where it gets a switch, ⏶ ⏷ 🗙 on hover and, for CLAP plugins with a GUI, an `Edit` button that opens the plugin's own editor window. LV2 plugins get generic controls in the panel. A plugin that can't be loaded later stays in the chain in red and is bypassed.

## Sub-features

- `plug-browse` lists the plugins found. The search field filters by name, vendor or format, and Rescan scans again.
- `plug-add` appends the chosen plugin to the chain and processes audio with it.
- `plug-controls` shows LV2 generic controls, and opens or hides a CLAP editor with `Edit`/`Hide`.
- `plug-remove` removes the plugin with 🗙 on the hovered row.
- `plug-persist` writes a `kind = "clap"` or `kind = "lv2"` chain entry, with state or controls, to the config.
- `plug-missing` shows a plugin whose file is gone in red, says it could not be loaded, and keeps its settings.

## How to get to it (user POV)

- Click `+ Add plugin` under the chain, search, and click a plugin.
- Hover its chain row for ⏶ ⏷ 🗙, and click `Edit` for a CLAP editor.

## Driving it with verify.sh

Preconditions:

- No plugins are installed on this machine. Start with `$V up --plugins nixpkgs#lsp-plugins`, which builds lsp-plugins from the flake-pinned nixpkgs and puts its `lib/clap` on `CLAP_PATH` and its `lib/lv2` first on `LV2_PATH`.

- **Browse.** Run `$V click 90 272`, `sleep 2`, and `$V shot plug-browser`. The `Add plugin` dialog lists LSP plugins (for example `CLAP  A/B Tester x2 Mono · LSP CLAP`) instead of `No plugins found`.
- **Search.** Click the search field where the `shot` shows it (it was at `$V click 445 315` with lsp-plugins), then run `$V type "gate"` and `$V shot plug-search`. Only matching entries remain (`Gate LeftRight`, `Gate MidSide`, …).
- **Add.** Take a `shot` to find the entry's position, then click it. The dialog closes, a row `04` appears in the chain, and `Add plugin` moves down by 46 px. `$V config` shows a new `[[chain]]` entry with `kind = "clap"` or `"lv2"`.
- **Audio.** Pick a plugin with an obvious effect (a gain or a gate). Set it in its panel, turn off rows 01 to 03, and compare `play-record` output with the plugin's switch on and off.
- **Remove.** Run `$V hover <row 04 name>`, then take a `shot` to find the 🗙 at the row's right end, and `$V click` it. The row is gone.

## Gotchas

- CLAP editors open as separate X11 windows with class `fsa-plugin`, on the same Xvfb display. `shot` only captures the main window. To capture the editor, run `DISPLAY=$(cat /tmp/fsa-verify/current/scratch/display) import -window root …` inside `nix shell --inputs-from . nixpkgs#imagemagick`.
- The first `--plugins` run downloads lsp-plugins (about 100 MB).
- LV2 hosting exists only on x86_64 Linux. `plugins/lv2/stub.rs` covers the other platforms.
- The dialog has a fixed width (460 px) and a fixed scrollable list height (320 px, ~10 rows visible) so it stays centered and does not resize as the search filter changes. The search box auto-focuses on open, and multi-token search filters by name, vendor, or format (`"lsp gate"`).
- Only `Browse` and `Search` had been run when this skill was created. Fix the remaining steps on the first real run.
- To test a full chain (32 items, the engine's `MAX_SLOTS`), start with a `--config` holding the three built-ins plus 29 `kind = "lv2"` entries with made-up URIs (`uri = "urn:fsa-verify:missing-01"` and so on). Each loads as a red missing item that still takes a slot. Repeating a built-in does not work, because `Config::normalize` keeps each one only once. With 32 items, `Add plugin` is greyed out and its tooltip reads `the chain is full (32 items); remove one to add another`. Removing one row with its 🗙 enables the button again. Rows past 11 need scrolling in the chain list.
