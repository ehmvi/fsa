---------------------------- MODULE EngineChain ----------------------------
(***************************************************************************)
(* The chain protocol between the gui thread (src/app.rs, App) and the     *)
(* audio thread (src/engine.rs, Engine).                                   *)
(*                                                                         *)
(* The gui owns the authoritative chain (App.chain) and mirrors every      *)
(* change to the audio thread as a Command through an rtrb ring            *)
(* (EngineHandle::send -> Engine::apply_commands). Slots that leave the    *)
(* engine come back through a second rtrb ring (Engine.garbage ->          *)
(* EngineHandle::collect_garbage) so they are dropped on the gui thread.   *)
(*                                                                         *)
(* Atomic steps and why they are atomic in the code:                       *)
(*   Process   = one Engine::process call. Only the audio thread touches   *)
(*               Engine.slots. A gui push racing the drain loop is seen    *)
(*               either in this drain or the next (rtrb is a FIFO SPSC),   *)
(*               which is the same as the push landing before or after.    *)
(*   Collect   = App::collect_garbage at the top of every App::ui frame.   *)
(*   Add, RemoveItem, MoveItem = App::add_plugin / add_lv2, remove_item,   *)
(*               move_item. They touch gui-only state plus one push.       *)
(***************************************************************************)
EXTENDS Integers, Sequences, FiniteSets

CONSTANTS
    Cap,        \* engine.rs MAX_SLOTS (32 in the code)
    Init,       \* items built by Setup::from_config before the engine starts
    MaxId,      \* search bound: App.next_id never reaches MaxId
    CmdBound,   \* search bound: gui acts only while fewer commands are queued
    FrameActs   \* gui actions per App::ui frame (one browser add + one row action)

None == -1  \* Command::Move { after: None }

\* Engine::new: Vec::with_capacity(MAX_SLOTS) then extend(initial), so an
\* oversized config grows the capacity on the non-rt thread before start.
EngCap == IF Init > Cap THEN Init ELSE Cap
\* Engine::new: rtrb::RingBuffer::new(MAX_SLOTS * 2) for the garbage ring
GcCap == 2 * Cap
\* Setup::from_config gives config item k the id k; these are the built-in
\* Denoise/Eq/Gate rows, which chain_row offers no remove button for.
Builtin == 0..(Init - 1)

VARIABLES
    chain,      \* App.chain, as ids, in gui order
    nextId,     \* App.next_id
    awaiting,   \* App.awaiting (ids are unique, so a set)
    acts,       \* gui actions taken since the last collect_garbage
    cmds,       \* the Command ring, oldest first
    slots,      \* Engine.slots: [id, removing, silent]
    gc,         \* the garbage ring of returned slot ids, oldest first
    audioDrop   \* a Slot was dropped (deallocated) on the audio thread

vars == <<chain, nextId, awaiting, acts, cmds, slots, gc, audioDrop>>
guiVars == <<chain, nextId, awaiting, acts>>

Range(s) == {s[i] : i \in DOMAIN s}
RemoveAt(s, i) == SubSeq(s, 1, i - 1) \o SubSeq(s, i + 1, Len(s))
InsertAt(s, i, e) == SubSeq(s, 1, i - 1) \o <<e>> \o SubSeq(s, i, Len(s))
Min(S) == CHOOSE x \in S : \A y \in S : x <= y
\* `iter().position(|s| s.id == id)`, 1-based, 0 = not found
PosOf(ss, id) ==
    LET I == {i \in DOMAIN ss : ss[i].id = id} IN IF I = {} THEN 0 ELSE Min(I)

NewSlot(id) == [id |-> id, removing |-> FALSE, silent |-> FALSE]

---------------------------------------------------------------------------
(* audio thread: Engine::apply_commands, one command                       *)

ApplyCmd(e, c) ==
    CASE c.op = "insert" ->
            \* Command::Insert
            IF Len(e.slots) < EngCap
            THEN [e EXCEPT !.slots = Append(@, NewSlot(c.id))]
            \* full: slot.unit.stop(); let _ = self.garbage.push(slot);
            ELSE IF Len(e.gc) < GcCap
                 THEN [e EXCEPT !.gc = Append(@, c.id)]
                 ELSE [e EXCEPT !.drop = TRUE]
      [] c.op = "remove" ->
            \* Command::Remove: mark the slot, it fades out first
            LET i == PosOf(e.slots, c.id) IN
            IF i = 0 THEN e ELSE [e EXCEPT !.slots[i].removing = TRUE]
      [] c.op = "move" ->
            \* Command::Move: remove, then insert right after `after`
            \* (front for None, end when `after` is not in the engine)
            LET from == PosOf(e.slots, c.id)
                rest == RemoveAt(e.slots, from)
                p    == PosOf(rest, c.after)
                to   == IF c.after = None THEN 1
                        ELSE IF p = 0 THEN Len(rest) + 1 ELSE p + 1
            IN IF from = 0 THEN e
               ELSE [e EXCEPT !.slots = InsertAt(rest, to, e.slots[from])]

RECURSIVE Drain(_, _)
Drain(e, cs) == IF cs = <<>> THEN e ELSE Drain(ApplyCmd(e, Head(cs)), Tail(cs))

\* end of apply_commands: hand back the first fully faded removed slot,
\* only if the garbage ring has room (`self.garbage.slots() > 0`)
ReturnOne(e) ==
    LET I == {i \in DOMAIN e.slots : e.slots[i].removing /\ e.slots[i].silent}
    IN IF I /= {} /\ Len(e.gc) < GcCap
       THEN LET i == Min(I)
            IN [e EXCEPT !.slots = RemoveAt(e.slots, i),
                         !.gc = Append(e.gc, e.slots[i].id)]
       ELSE e

\* Engine::process: apply_commands, then the slot loop, where a removing
\* slot's Fade ramps toward 0. A fade is 480 samples (Fade::MS at 48 kHz)
\* and blocks are QUANTUM = 256, so a fade ends after a nondeterministic
\* number of blocks; any subset of fading slots may finish in this block.
Process ==
    LET e == ReturnOne(Drain([slots |-> slots, gc |-> gc, drop |-> audioDrop], cmds))
        fading == {i \in DOMAIN e.slots : e.slots[i].removing /\ ~e.slots[i].silent}
    IN \E done \in SUBSET fading :
        /\ slots' = [i \in DOMAIN e.slots |->
                        IF i \in done THEN [e.slots[i] EXCEPT !.silent = TRUE]
                        ELSE e.slots[i]]
        /\ gc' = e.gc
        /\ audioDrop' = e.drop
        /\ cmds' = <<>>
        /\ UNCHANGED guiVars

---------------------------------------------------------------------------
(* gui thread: App                                                         *)

GuiCanAct == acts < FrameActs /\ Len(cmds) < CmdBound

\* App::add_plugin / App::add_lv2, guarded by App::has_room: the chain plus the
\* slots still fading out (awaiting) must leave room under MAX_SLOTS
Add ==
    /\ GuiCanAct
    /\ Len(chain) + Cardinality(awaiting) < Cap
    /\ nextId < MaxId
    /\ chain' = Append(chain, nextId)
    /\ cmds' = Append(cmds, [op |-> "insert", id |-> nextId])
    /\ nextId' = nextId + 1
    /\ acts' = acts + 1
    /\ UNCHANGED <<awaiting, slots, gc, audioDrop>>

\* App::remove_item
RemoveItem ==
    /\ GuiCanAct
    /\ \E i \in 1..Len(chain) :
        /\ chain[i] \notin Builtin
        /\ chain' = RemoveAt(chain, i)
        /\ cmds' = Append(cmds, [op |-> "remove", id |-> chain[i]])
        /\ awaiting' = awaiting \cup {chain[i]}
    /\ acts' = acts + 1
    /\ UNCHANGED <<nextId, slots, gc, audioDrop>>

\* App::move_item(i, i + dir), dir = +-1 from chain_row
MoveItem ==
    /\ GuiCanAct
    /\ \E i \in 1..Len(chain), d \in {-1, 1} :
        LET to == i + d
            c2 == InsertAt(RemoveAt(chain, i), to, chain[i])
        IN /\ to \in 1..Len(chain)
           /\ chain' = c2
           /\ cmds' = Append(cmds, [op |-> "move", id |-> chain[i],
                                    after |-> IF to = 1 THEN None ELSE c2[to - 1]])
    /\ acts' = acts + 1
    /\ UNCHANGED <<nextId, awaiting, slots, gc, audioDrop>>

\* App::collect_garbage, first thing in every App::ui frame
Collect ==
    /\ awaiting' = awaiting \ Range(gc)
    /\ gc' = <<>>
    /\ acts' = 0
    /\ UNCHANGED <<chain, nextId, cmds, slots, audioDrop>>

---------------------------------------------------------------------------

Init0 ==
    /\ chain = [k \in 1..Init |-> k - 1]
    /\ nextId = Init            \* App::new: max id + 1
    /\ awaiting = {}
    /\ acts = 0
    /\ cmds = <<>>
    /\ slots = [k \in 1..Init |-> NewSlot(k - 1)]
    /\ gc = <<>>
    /\ audioDrop = FALSE

Next == Process \/ Add \/ RemoveItem \/ MoveItem \/ Collect

Spec == Init0 /\ [][Next]_vars

---------------------------------------------------------------------------
(* properties                                                              *)

\* Engine::process never deallocates a Slot (engine.rs module doc).
NoAudioThreadDrop == ~audioDrop

\* Engine.slots never outgrows its capacity, so push/insert never reallocate
\* on the audio thread ("remove + insert within capacity never reallocates").
NoRealloc == Len(slots) <= EngCap

Live == LET L == {i \in DOMAIN slots : ~slots[i].removing}
            idx == [k \in 1..Cardinality(L) |->
                        CHOOSE i \in L : Cardinality({j \in L : j <= i}) = k]
        IN [k \in DOMAIN idx |-> slots[idx[k]].id]

\* Once the audio thread has caught up, it runs exactly the chain the gui
\* shows, in the same order.
ChainAgrees == cmds = <<>> => Live = chain

\* Weaker: the items both sides have are in the same order (what the
\* relative Move { after } is for).
Restrict(s, S) == SelectSeq(s, LAMBDA x : x \in S)
OrderAgrees == cmds = <<>> => Restrict(Live, Range(chain)) = Restrict(chain, Range(Live))

\* Every id App::teardown_plugins waits for can still come back: its slot is
\* in the engine, in the garbage ring, or its Insert is still queued.
AwaitedComesBack ==
    \A id \in awaiting :
        \/ \E i \in DOMAIN slots : slots[i].id = id
        \/ id \in Range(gc)
        \/ \E k \in DOMAIN cmds : cmds[k].op = "insert" /\ cmds[k].id = id
=============================================================================
