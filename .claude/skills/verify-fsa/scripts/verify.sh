#!/usr/bin/env bash
# drives a real fsa instance on a private X display against a virtual test mic
# on the user's pipewire session. see ../SKILL.md for the workflow.
set -euo pipefail

SCRIPT="$(readlink -f "${BASH_SOURCE[0]}")"
REPO="$(cd "$(dirname "$SCRIPT")/../../../.." && pwd)"
ROOT="${FSA_VERIFY_ROOT:-/tmp/fsa-verify}"
CURRENT="$ROOT/current"
BIN="$REPO/target/release/fsa"
TEST_MIC=fsa-verify.mic
TEST_SPEAKER=fsa-verify.speaker

# xdotool, imagemagick, sox and jq, pinned by the repo's flake.lock
if [ -z "${FSA_VERIFY_TOOLS:-}" ]; then
    exec env FSA_VERIFY_TOOLS=1 nix shell --inputs-from "$REPO" \
        nixpkgs#xdotool nixpkgs#imagemagick nixpkgs#sox nixpkgs#jq \
        -c "$SCRIPT" "$@"
fi

die() { echo "verify: $*" >&2; exit 1; }
say() { echo "verify: $*" >&2; }

run_dir() {
    [ -L "$CURRENT" ] || die "no running instance, start one with: $SCRIPT up"
    readlink -f "$CURRENT"
}
ev() { echo "$(run_dir)/evidence"; }
sc() { echo "$(run_dir)/scratch"; }

# pid file -> pid, only if that pid is still the process we started
our_pid() {
    local file="$1" want="$2" pid
    [ -f "$file" ] || return 1
    pid="$(cat "$file")"
    [ -r "/proc/$pid/cmdline" ] || return 1
    tr '\0' ' ' < "/proc/$pid/cmdline" | grep -q -- "$want" || return 1
    echo "$pid"
}

fsa_nodes() {
    pw-dump 2>/dev/null | grep -oE '"node.name": "(fsa\.processed|fsa\.capture|fsa-verify\.[a-z]+)"' |
        sed 's/.*: "//; s/"$//' | sort -u
    true
}

# pid of the process that owns a pipewire node
node_pid() {
    pw-dump | jq -r --arg n "$1" '. as $all
        | [.[] | select(.info.props["node.name"]? == $n)][0].info.props["client.id"] as $c
        | $all[] | select(.id == $c) | .info.props["application.process.id"]'
}

win() {
    local d; d="$(sc)"
    DISPLAY="$(cat "$d/display")" xdotool search --class '^fsa$' 2>/dev/null | head -1
}

xdo() {
    local d; d="$(sc)"
    DISPLAY="$(cat "$d/display")" xdotool "$@"
}

wait_for() { # seconds, description, command...
    local secs="$1" what="$2"; shift 2
    for _ in $(seq $((secs * 4))); do
        if "$@" >/dev/null 2>&1; then return 0; fi
        sleep 0.25
    done
    die "timed out after ${secs}s waiting for $what"
}

cmd_build() {
    (cd "$REPO" && nix develop -c cargo build --release)
}

cmd_up() {
    local config="" plugins="" args=()
    while [ $# -gt 0 ]; do
        case "$1" in
            --config) config="$(readlink -f "$2")"; shift 2 ;;
            --plugins) plugins="$2"; shift 2 ;;
            *) args+=("$1"); shift ;;
        esac
    done
    if [ -L "$CURRENT" ]; then
        die "an instance is already up ($(readlink -f "$CURRENT")); run '$SCRIPT down' first"
    fi
    local existing; existing="$(fsa_nodes)"
    if [ -n "$existing" ]; then
        die "pipewire already has fsa nodes:
$existing
another fsa (maybe the user's own) is running, or a run was not cleaned up.
node names are global, so a second instance would collide. do not kill the
user's fsa; ask them to close it. for a stale verify run use '$SCRIPT down'."
    fi
    [ -x "$BIN" ] || die "no binary at $BIN, run '$SCRIPT build'"
    local stale; stale="$(find "$REPO/src" "$REPO/Cargo.toml" "$REPO/Cargo.lock" -newer "$BIN" -print -quit)"
    [ -z "$stale" ] || die "$BIN is older than $stale, run '$SCRIPT build'"

    local id dir d n
    id="$(date +%Y%m%d-%H%M%S)-$$"
    dir="$ROOT/$id"
    mkdir -p "$dir/evidence" "$dir/scratch"
    d="$dir/scratch"
    ln -sfn "$dir" "$CURRENT"
    say "run $id"

    # a free display number
    for n in $(seq 90 140); do
        [ -e "/tmp/.X11-unix/X$n" ] || [ -e "/tmp/.X$n-lock" ] || break
    done
    echo ":$n" > "$d/display"
    setsid Xvfb ":$n" -screen 0 1280x800x24 -nolisten tcp > "$d/xvfb.log" 2>&1 < /dev/null &
    echo $! > "$d/xvfb.pid"
    wait_for 10 "Xvfb on :$n" test -e "/tmp/.X11-unix/X$n"

    # the test mic: play into fsa-verify.speaker, fsa captures fsa-verify.mic.
    # pw-loopback nodes are filters, so they never become the default device.
    setsid pw-loopback -n fsa-verify \
        --capture-props="media.class=Audio/Sink node.name=$TEST_SPEAKER node.description=\"fsa verify speaker\" audio.position=[FL FR]" \
        --playback-props="media.class=Audio/Source node.name=$TEST_MIC node.description=\"fsa verify mic\" audio.position=[FL FR]" \
        > "$d/loopback.log" 2>&1 < /dev/null &
    echo $! > "$d/loopback.pid"
    wait_for 10 "the test mic" sh -c "pw-dump | grep -q '\"node.name\": \"$TEST_MIC\"'"

    # runtime libraries egui/winit dlopen, from the dev shell
    local env_file="$d/devshell.env"
    (cd "$REPO" && nix develop -c sh -c 'printf "%s\n%s\n" "$LD_LIBRARY_PATH" "$LV2_PATH"') > "$env_file"
    local ld lv2 clap=""
    ld="$(sed -n 1p "$env_file")"
    lv2="$(sed -n 2p "$env_file")"
    if [ -n "$plugins" ]; then
        local p
        p="$(cd "$REPO" && nix build --no-link --print-out-paths --inputs-from . "$plugins" | head -1)"
        lv2="$p/lib/lv2:$lv2"
        clap="$p/lib/clap"
        say "plugins from $p"
    fi

    local cfg="$d/config.toml"
    if [ -n "$config" ]; then cp "$config" "$cfg"; fi
    # pin the test mic unless the caller picked an input
    local has_input=""
    for a in "${args[@]+"${args[@]}"}"; do
        case "$a" in --input | -i) has_input=1 ;; esac
    done
    [ -n "$has_input" ] || args+=(--input "$TEST_MIC")

    printf '%q ' "$BIN" --config "$cfg" "${args[@]}" > "$d/fsa.cmd"
    setsid env -u WAYLAND_DISPLAY DISPLAY=":$n" LD_LIBRARY_PATH="$ld" LV2_PATH="$lv2" \
        CLAP_PATH="$clap" XDG_CONFIG_HOME="$d/xdg" RUST_LOG="${RUST_LOG:-info}" \
        "$BIN" --config "$cfg" "${args[@]}" > "$d/fsa.log" 2>&1 < /dev/null &
    echo $! > "$d/fsa.pid"

    wait_for 20 "fsa to report 'running'" grep -q 'fsa::audio::pipewire\] running' "$d/fsa.log"
    wait_for 10 "the fsa.processed node" sh -c "pw-dump | grep -q '\"node.name\": \"fsa.processed\"'"
    wait_for 20 "the fsa window" sh -c "DISPLAY=:$n xdotool search --class '^fsa\$'"
    sleep 1 # first frames
    say "up: display :$n, evidence in $dir/evidence"
    echo "$dir"
}

cmd_doctor() {
    local d ok=1 pid w geo
    d="$(sc)"
    check() { if "$@" >/dev/null 2>&1; then echo "ok    $label"; else echo "FAIL  $label"; ok=0; fi; }
    label="Xvfb ($(cat "$d/display")) is ours and alive"; check our_pid "$d/xvfb.pid" Xvfb
    label="test mic loopback is ours and alive"; check our_pid "$d/loopback.pid" pw-loopback
    label="fsa is ours and alive"; check our_pid "$d/fsa.pid" "$BIN"
    pid="$(our_pid "$d/fsa.pid" "$BIN" || true)"
    label="fsa.processed belongs to our fsa (pid ${pid:-?})"
    check test -n "$pid" -a "$(node_pid fsa.processed)" = "$pid"
    label="capture is streaming (last state 'running')"
    check sh -c "grep 'fsa::audio::pipewire\]' '$d/fsa.log' | grep -E 'running|paused|unconnected|connecting|error' | tail -1 | grep -q running"
    label="binary is newer than src/"
    check test -z "$(find "$REPO/src" "$REPO/Cargo.toml" -newer "$BIN" -print -quit)"
    w="$(win || true)"
    label="window mapped"; check test -n "$w"
    if [ -n "$w" ]; then
        geo="$(xdo getwindowgeometry --shell "$w" | grep -E 'WIDTH|HEIGHT' | tr '\n' ' ')"
        label="window is 1040x700 (the size the coordinates in SKILL.md assume): $geo"
        check sh -c "echo '$geo' | grep -q 'WIDTH=1040 HEIGHT=700'"
    fi
    if grep -iE 'panic|error' "$d/fsa.log" | grep -qv XSETTINGS; then
        echo "note  fsa.log mentions errors:"; grep -iE 'panic|error' "$d/fsa.log" | grep -v XSETTINGS | tail -5 | sed 's/^/      /'
    fi
    [ "$ok" = 1 ] || exit 1
}

cmd_shot() {
    local name="${1:?shot NAME}" out
    out="$(ev)/$name.png"
    # just the window, at 1040x700
    DISPLAY="$(cat "$(sc)/display")" import -window "$(win)" "$out"
    echo "$out"
}

# window-relative pointer actions. egui wants a hover frame before a click.
cmd_click() {
    local x="${1:?click X Y}" y="${2:?}" button="${3:-1}"
    xdo mousemove --window "$(win)" "$x" "$y" sleep 0.15 click "$button" sleep 0.15
    # park the pointer off the window so hover-only tools and tooltips go away
    xdo mousemove 1200 780 sleep 0.3
}
cmd_dblclick() {
    local x="${1:?dblclick X Y}" y="${2:?}"
    xdo mousemove --window "$(win)" "$x" "$y" sleep 0.15 click --repeat 2 --delay 80 1 sleep 0.15
    xdo mousemove 1200 780 sleep 0.3
}
cmd_hover() {
    local x="${1:?hover X Y}" y="${2:?}"
    xdo mousemove --window "$(win)" "$x" "$y" sleep 0.4
}
cmd_drag() {
    local x1="${1:?drag X1 Y1 X2 Y2}" y1="${2:?}" x2="${3:?}" y2="${4:?}" w i steps=10
    w="$(win)"
    xdo mousemove --window "$w" "$x1" "$y1" sleep 0.15 mousedown 1 sleep 0.1
    for i in $(seq 1 $steps); do
        xdo mousemove --window "$w" $((x1 + (x2 - x1) * i / steps)) $((y1 + (y2 - y1) * i / steps)) sleep 0.03
    done
    xdo mouseup 1 sleep 0.15 mousemove 1200 780 sleep 0.3
}
cmd_scroll() { # X Y up|down [clicks]
    local x="${1:?scroll X Y up|down [n]}" y="${2:?}" dir="${3:?}" n="${4:-1}" b
    case "$dir" in up) b=4 ;; down) b=5 ;; *) die "scroll direction is up or down" ;; esac
    xdo mousemove --window "$(win)" "$x" "$y" sleep 0.15 click --repeat "$n" --delay 60 "$b" sleep 0.4
}
# there is no window manager on the private display, so focus the window
# explicitly; events sent with --window are ignored by winit
focus() { xdo windowfocus --sync "$(win)" 2>/dev/null || xdo windowfocus "$(win)"; }
cmd_key() { focus; xdo key --delay 60 "$@"; sleep 0.3; }
cmd_type() { focus; xdo type --delay 40 "$*"; sleep 0.3; }

# signal NAME SOX_SYNTH_ARGS... -> stereo 48 kHz float wav in evidence
cmd_signal() {
    local name="${1:?signal NAME SYNTH-ARGS...}" out; shift
    out="$(ev)/$name.wav"
    sox -n -r 48000 -c 2 -b 32 -e floating-point "$out" "$@"
    echo "$out"
}

# concat NAME A.wav B.wav ... -> one wav in evidence, segments back to back
cmd_concat() {
    local name="${1:?concat NAME A.wav B.wav ...}" out; shift
    out="$(ev)/$name.wav"
    sox "$@" "$out"
    echo "$out"
}

# play WAV into the test mic while recording "Mic (processed)"
# usage: play-record NAME IN.wav  -> evidence/NAME.out.wav + NAME.stats.txt
cmd_play_record() {
    local name="${1:?play-record NAME IN.wav}" in="${2:?}" e out rec
    e="$(ev)"
    out="$e/$name.out.wav"
    pw-record --target fsa.processed --rate 48000 --channels 1 --format f32 "$out" &
    rec=$!
    sleep 0.5
    pw-play --target "$TEST_SPEAKER" "$in"
    sleep 0.5
    kill -INT "$rec"; wait "$rec" || true
    {
        echo "# input $in"; sox "$in" -n stats 2>&1
        echo "# output $out"; sox "$out" -n stats 2>&1
    } > "$e/$name.stats.txt"
    echo "$out"
}

# rms FILE START DURATION -> RMS level in dBFS of that window
cmd_rms() {
    local f="${1:?rms FILE START DUR}" s="${2:?}" n="${3:?}"
    sox "$f" -n trim "$s" "$n" stats 2>&1 | awk '/^RMS lev dB/ {print $4}'
}

cmd_log() { cat "$(sc)/fsa.log"; }
cmd_config() { cat "$(sc)/config.toml"; }

cmd_down() {
    local dir d p
    [ -L "$CURRENT" ] || { say "nothing to tear down"; return 0; }
    dir="$(readlink -f "$CURRENT")"
    d="$dir/scratch"
    # fsa first, so it saves its config and drops its nodes
    if p="$(our_pid "$d/fsa.pid" "$BIN")"; then
        kill "$p"
        for _ in $(seq 40); do [ -d "/proc/$p" ] || break; sleep 0.25; done
        [ ! -d "/proc/$p" ] || { say "fsa ignored SIGTERM, killing"; kill -9 "$p"; }
    fi
    if p="$(our_pid "$d/loopback.pid" pw-loopback)"; then kill "$p"; fi
    if p="$(our_pid "$d/xvfb.pid" Xvfb)"; then kill "$p"; fi
    sleep 0.5
    # keep what proves the run, drop the rest
    for f in fsa.log fsa.cmd config.toml; do
        [ -f "$d/$f" ] && cp "$d/$f" "$dir/evidence/"
    done
    rm -rf "$d"
    rm -f "$CURRENT"
    local left; left="$(fsa_nodes)"
    [ -z "$left" ] || say "warning, fsa nodes still present: $left"
    say "down; evidence kept in $dir/evidence"
    echo "$dir/evidence"
}

case "${1:-}" in
    build) shift; cmd_build "$@" ;;
    up) shift; cmd_up "$@" ;;
    doctor) shift; cmd_doctor ;;
    shot) shift; cmd_shot "$@" ;;
    click) shift; cmd_click "$@" ;;
    dblclick) shift; cmd_dblclick "$@" ;;
    hover) shift; cmd_hover "$@" ;;
    drag) shift; cmd_drag "$@" ;;
    scroll) shift; cmd_scroll "$@" ;;
    key) shift; cmd_key "$@" ;;
    type) shift; cmd_type "$@" ;;
    signal) shift; cmd_signal "$@" ;;
    concat) shift; cmd_concat "$@" ;;
    play-record) shift; cmd_play_record "$@" ;;
    rms) shift; cmd_rms "$@" ;;
    log) cmd_log ;;
    config) cmd_config ;;
    evidence) ev ;;
    down) cmd_down ;;
    *) sed -n '/^case/,/^esac/p' "$SCRIPT" | grep -oE '^    [a-z-]+\)' | tr -d ' )' | sed 's/^/usage: verify.sh /'; exit 2 ;;
esac
