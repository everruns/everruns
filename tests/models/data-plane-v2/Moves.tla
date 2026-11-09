---------------------------- MODULE Moves ----------------------------
(***************************************************************************)
(* Moving one placeable unit (a session) between cells.                     *)
(*                                                                          *)
(* Data plane v2 design: every placement has a generation that only grows. *)
(* Import, directory update, activation, forward and cleanup all name the   *)
(* generation they belong to; a cell keeps a tombstone with the last        *)
(* generation of every unit that left it. The destination holds the unit    *)
(* prepared (not runnable) until the directory commits the new generation, *)
(* and only a prepared copy can be activated: a late duplicate "activate"  *)
(* must not wake a copy that has since started leaving (TLC found that in *)
(* the first draft, where both states were one "frozen").                 *)
(*                                                                          *)
(* Messages are never removed from `net`, so any message can be delivered   *)
(* late and any number of times: delay and duplication come for free.       *)
(*                                                                          *)
(* With Fenced = FALSE the model behaves like funout at 8ce744b (Import     *)
(* accepts any unit the cell doesn't hold and makes it live, Place          *)
(* overwrites, Forward is unconditional); TLC then finds a delayed import   *)
(* reviving a stale copy.                                                   *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets

CONSTANTS Cells, MaxGen, Fenced

VARIABLES
    st,     \* st[c]: the unit's copy in cell c: "none", "prepared" (imported,
            \* not yet allowed to run), "live", or "leaving" (frozen source)
    gen,    \* gen[c]: generation of the copy cell c holds (0 = none)
    tomb,   \* tomb[c]: highest generation that left cell c
    dir,    \* directory: [gen |-> g, cell |-> c]
    net     \* messages ever sent

vars == <<st, gen, tomb, dir, net>>

First == CHOOSE c \in Cells : TRUE

Init ==
    /\ st = [c \in Cells |-> IF c = First THEN "live" ELSE "none"]
    /\ gen = [c \in Cells |-> IF c = First THEN 1 ELSE 0]
    /\ tomb = [c \in Cells |-> 0]
    /\ dir = [gen |-> 1, cell |-> First]
    /\ net = {}

Send(m) == net' = net \cup {m}

\* The source freezes its live copy (Engine hands the epoch back at the next
\* step boundary) and offers the image to the destination at generation g+1.
StartMove(src, dst) ==
    /\ src # dst
    /\ st[src] = "live"
    /\ gen[src] < MaxGen
    /\ st' = [st EXCEPT ![src] = "leaving"]
    /\ Send([type |-> "import", g |-> gen[src] + 1, to |-> dst, from |-> src])
    /\ UNCHANGED <<gen, tomb, dir>>

\* The destination installs the image. Fenced: only a newer generation than
\* anything it holds or has seen leave, and the copy stays frozen.
RecvImport(m) ==
    /\ m \in net /\ m.type = "import"
    /\ LET c == m.to IN
       IF Fenced
       THEN /\ m.g > gen[c] /\ m.g > tomb[c]
            /\ st' = [st EXCEPT ![c] = "prepared"]
            /\ gen' = [gen EXCEPT ![c] = m.g]
            /\ Send([type |-> "place", g |-> m.g, to |-> c, from |-> m.from])
            /\ UNCHANGED <<tomb, dir>>
       ELSE \* funout 8ce744b: accept when the cell doesn't hold the unit,
            \* drop any forward and make the image live.
            /\ st[c] = "none"
            /\ st' = [st EXCEPT ![c] = "live"]
            /\ gen' = [gen EXCEPT ![c] = m.g]
            /\ Send([type |-> "place", g |-> m.g, to |-> c, from |-> m.from])
            /\ UNCHANGED <<tomb, dir>>

\* The directory repoints the unit. Fenced: compare-and-swap on the previous
\* generation; this commit is the ownership transfer.
RecvPlace(m) ==
    /\ m \in net /\ m.type = "place"
    /\ IF Fenced THEN dir.gen = m.g - 1 ELSE TRUE
    /\ dir' = [gen |-> m.g, cell |-> m.to]
    /\ net' = net \cup {[type |-> "forward", g |-> m.g, at |-> m.from, to |-> m.to],
                        [type |-> "activate", g |-> m.g, to |-> m.to]}
    /\ UNCHANGED <<st, gen, tomb>>

\* The destination may run the unit only once the directory committed g.
RecvActivate(m) ==
    /\ Fenced
    /\ m \in net /\ m.type = "activate"
    /\ st[m.to] = "prepared" /\ gen[m.to] = m.g
    /\ st' = [st EXCEPT ![m.to] = "live"]
    /\ UNCHANGED <<gen, tomb, dir, net>>

\* The source drops its copy and leaves a tombstone. Fenced: only the copy
\* this move took the image from (generation g-1).
RecvForward(m) ==
    /\ m \in net /\ m.type = "forward"
    /\ LET c == m.at IN
       /\ IF Fenced THEN st[c] = "leaving" /\ gen[c] = m.g - 1 ELSE st[c] # "none"
       /\ st' = [st EXCEPT ![c] = "none"]
       /\ tomb' = [tomb EXCEPT ![c] = IF gen[c] > tomb[c] THEN gen[c] ELSE tomb[c]]
       /\ gen' = [gen EXCEPT ![c] = 0]
       /\ UNCHANGED <<dir, net>>

\* A move whose import or place got lost is abandoned by the source. It takes
\* the unit back through the directory, at a generation above the abandoned
\* move's, so a late place for that move fails its compare-and-swap and the
\* prepared copy at the destination can never be activated.
AbortMove(src) ==
    /\ Fenced
    /\ st[src] = "leaving"
    /\ dir.cell = src /\ dir.gen = gen[src]
    /\ gen[src] + 2 <= MaxGen
    /\ dir' = [gen |-> gen[src] + 2, cell |-> src]
    /\ gen' = [gen EXCEPT ![src] = gen[src] + 2]
    /\ st' = [st EXCEPT ![src] = "live"]
    /\ UNCHANGED <<tomb, net>>

\* A prepared copy the directory never committed is dropped by generation.
DropStale(c) ==
    /\ Fenced
    /\ st[c] = "prepared"
    /\ dir.gen >= gen[c] /\ dir.cell # c
    /\ st' = [st EXCEPT ![c] = "none"]
    /\ tomb' = [tomb EXCEPT ![c] = IF gen[c] > tomb[c] THEN gen[c] ELSE tomb[c]]
    /\ gen' = [gen EXCEPT ![c] = 0]
    /\ UNCHANGED <<dir, net>>

Next ==
    \/ \E s, d \in Cells : StartMove(s, d)
    \/ \E c \in Cells : AbortMove(c) \/ DropStale(c)
    \/ \E m \in net : RecvImport(m) \/ RecvPlace(m) \/ RecvActivate(m) \/ RecvForward(m)

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ st \in [Cells -> {"none", "prepared", "live", "leaving"}]
    /\ dir.cell \in Cells

\* At most one cell may run the unit; several inactive copies are allowed.
OneExecutableOwner == Cardinality({c \in Cells : st[c] = "live"}) <= 1

\* A live copy is the one the directory points at, at the directory's generation.
LiveIsPlaced == \A c \in Cells : st[c] = "live" => (dir.cell = c /\ dir.gen = gen[c])

\* The directory's generation never goes backwards.
DirMonotonic == [][dir'.gen >= dir.gen]_vars
=============================================================================
