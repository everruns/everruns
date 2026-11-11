---------------------------- MODULE Epochs ----------------------------
(***************************************************************************)
(* Execution epochs of one session, fenced by the cell's authority.        *)
(*                                                                          *)
(* An executor claims the session and gets an epoch: the holder's name and  *)
(* a fence number that only grows. Every write it makes (a journal append,  *)
(* a progress report, the release) names the epoch, and the authority      *)
(* applies it only while that epoch is the current, unexpired one           *)
(* (`Authority::owns`, `current` in crates/cell/src/authority.rs).          *)
(*                                                                          *)
(* Executors crash and come back under the same name, and every message    *)
(* stays in `net`, so a write from an earlier incarnation can arrive late   *)
(* and more than once. Expiry is judged on the proposer's clock carried in  *)
(* the command; a leader change can move that clock back, so an expired    *)
(* epoch may become current again (`Unexpire`) until someone claims.       *)
(*                                                                          *)
(* The poison guard (crates/core/src/host/session_actor/lease.rs): a claim  *)
(* counts `attempts`; a claim that brings it past K seals the turn instead  *)
(* of running it. A resumed epoch (attempts > 1) that stores two steps     *)
(* reports progress once, and the report sets attempts back to 1. The      *)
(* ghost `dry` counts holder deaths since the last progress the authority   *)
(* accepted from the epoch that made it.                                    *)
(*                                                                          *)
(* One known window: an epoch that dies after its second step but before    *)
(* its report lands counts as a death without progress. The report is sent  *)
(* right after the step is stored, so this costs at most one round trip.    *)
(*                                                                          *)
(* Bug configurations:                                                      *)
(*  - "fence_by_name": writes are checked by holder name and expiry only. A *)
(*    restarted executor's earlier incarnation then writes behind the new   *)
(*    epoch's back: the journal goes backwards.                             *)
(*  - "count_every_lease": the authority ignores progress reports, as core  *)
(*    did before the fix; a turn that advances in every epoch is sealed.    *)
(*  - "unfenced_progress": any late report resets attempts; a turn that     *)
(*    keeps dying is never sealed.                                          *)
(***************************************************************************)
EXTENDS Naturals, Sequences, FiniteSets

CONSTANTS Executors, MaxFence, K, Steps, Bug

None == [name |-> "none", fence |-> 0]

VARIABLES
    holder,   \* the authority's epoch: [name, fence], or None
    expired,  \* the proposer's clock is past the holder's expiry
    fence,    \* last fence handed out
    attempts, \* the authority's poison guard counter
    sealed,   \* the turn was sealed by the poison guard
    done,     \* the turn was released complete
    journal,  \* accepted appends, in order: <<[fence, n]>>
    acc,      \* acc[f]: appends accepted under fence f
    held,     \* held[e]: the fence executor e believes it holds (0 = none)
    resumed,  \* resumed[e]: e's epoch was claimed with attempts > 1
    sent,     \* sent[e]: appends e sent in its current epoch
    reported, \* reported[e]: e sent its progress report this epoch
    net,      \* messages ever sent
    dry       \* ghost: holder deaths since the last accepted progress

vars == <<holder, expired, fence, attempts, sealed, done, journal, acc,
          held, resumed, sent, reported, net, dry>>

Fences == 1..MaxFence

Init ==
    /\ holder = None
    /\ expired = FALSE
    /\ fence = 0
    /\ attempts = 0
    /\ sealed = FALSE
    /\ done = FALSE
    /\ journal = <<>>
    /\ acc = [f \in Fences |-> 0]
    /\ held = [e \in Executors |-> 0]
    /\ resumed = [e \in Executors |-> FALSE]
    /\ sent = [e \in Executors |-> 0]
    /\ reported = [e \in Executors |-> FALSE]
    /\ net = {}
    /\ dry = 0

\* The authority's rule, and the truth the ghost uses whatever the rule is.
Owns(m) == holder.name = m.name /\ holder.fence = m.fence /\ ~expired
Accepts(m) ==
    IF Bug = "fence_by_name" THEN holder.name = m.name /\ ~expired ELSE Owns(m)

Live == ~sealed /\ ~done

\* A claim succeeds when no unexpired epoch holds the session. Replacing an
\* expired holder is that holder's death.
Claim(e) ==
    /\ Live
    /\ held[e] = 0
    /\ holder = None \/ expired
    /\ fence < MaxFence
    /\ LET f == fence + 1
           a == attempts + 1
       IN /\ fence' = f
          /\ attempts' = a
          /\ dry' = IF holder # None THEN dry + 1 ELSE dry
          /\ expired' = FALSE
          /\ IF a > K
               THEN \* the driver seals the turn and gives the epoch back
                    /\ sealed' = TRUE
                    /\ holder' = None
                    /\ UNCHANGED <<held, resumed, sent, reported>>
               ELSE /\ sealed' = FALSE
                    /\ holder' = [name |-> e, fence |-> f]
                    /\ held' = [held EXCEPT ![e] = f]
                    /\ resumed' = [resumed EXCEPT ![e] = a > 1]
                    /\ sent' = [sent EXCEPT ![e] = 0]
                    /\ reported' = [reported EXCEPT ![e] = FALSE]
    /\ UNCHANGED <<done, journal, acc, net>>

SendAppend(e) ==
    /\ held[e] # 0
    /\ sent[e] < Steps
    /\ sent' = [sent EXCEPT ![e] = @ + 1]
    /\ net' = net \cup {[type |-> "append", name |-> e, fence |-> held[e], n |-> sent[e] + 1]}
    /\ UNCHANGED <<holder, expired, fence, attempts, sealed, done, journal, acc,
                   held, resumed, reported, dry>>

\* A resumed epoch reports once, after the authority stored its second step
\* (the executor learns that from the append's reply).
SendReport(e) ==
    /\ held[e] # 0
    /\ resumed[e] /\ ~reported[e]
    /\ acc[held[e]] >= 2
    /\ reported' = [reported EXCEPT ![e] = TRUE]
    /\ net' = net \cup {[type |-> "progress", name |-> e, fence |-> held[e], n |-> 0]}
    /\ UNCHANGED <<holder, expired, fence, attempts, sealed, done, journal, acc,
                   held, resumed, sent, dry>>

SendRelease(e) ==
    /\ held[e] # 0
    /\ sent[e] = Steps
    /\ held' = [held EXCEPT ![e] = 0]
    /\ net' = net \cup {[type |-> "release", name |-> e, fence |-> held[e], n |-> 0]}
    /\ UNCHANGED <<holder, expired, fence, attempts, sealed, done, journal, acc,
                   resumed, sent, reported, dry>>

\* The executor dies; it may claim again later under the same name.
Crash(e) ==
    /\ held[e] # 0
    /\ held' = [held EXCEPT ![e] = 0]
    /\ UNCHANGED <<holder, expired, fence, attempts, sealed, done, journal, acc,
                   resumed, sent, reported, net, dry>>

\* Appends are idempotent by id, as Engine's committed batches are.
RecvAppend(m) ==
    /\ m \in net /\ m.type = "append"
    /\ Live /\ Accepts(m)
    /\ [fence |-> m.fence, n |-> m.n] \notin {journal[i] : i \in DOMAIN journal}
    /\ journal' = Append(journal, [fence |-> m.fence, n |-> m.n])
    /\ acc' = [acc EXCEPT ![m.fence] = @ + 1]
    /\ UNCHANGED <<holder, expired, fence, attempts, sealed, done, held, resumed,
                   sent, reported, net, dry>>

RecvProgress(m) ==
    /\ m \in net /\ m.type = "progress"
    /\ Live
    /\ IF Bug = "unfenced_progress" THEN TRUE ELSE Accepts(m)
    /\ attempts' = IF Bug = "count_every_lease" THEN attempts ELSE 1
    /\ dry' = IF Owns(m) THEN 0 ELSE dry
    /\ UNCHANGED <<holder, expired, fence, sealed, done, journal, acc, held,
                   resumed, sent, reported, net>>

RecvRelease(m) ==
    /\ m \in net /\ m.type = "release"
    /\ Live /\ Accepts(m)
    /\ holder' = None
    /\ attempts' = 0
    /\ done' = TRUE
    /\ UNCHANGED <<expired, fence, sealed, journal, acc, held, resumed, sent,
                   reported, net, dry>>

Expire ==
    /\ holder # None /\ ~expired
    /\ expired' = TRUE
    /\ UNCHANGED <<holder, fence, attempts, sealed, done, journal, acc, held,
                   resumed, sent, reported, net, dry>>

\* A new leader's clock is behind the old one's.
Unexpire ==
    /\ holder # None /\ expired
    /\ expired' = FALSE
    /\ UNCHANGED <<holder, fence, attempts, sealed, done, journal, acc, held,
                   resumed, sent, reported, net, dry>>

Next ==
    \/ \E e \in Executors :
          Claim(e) \/ SendAppend(e) \/ SendReport(e) \/ SendRelease(e) \/ Crash(e)
    \/ \E m \in net : RecvAppend(m) \/ RecvProgress(m) \/ RecvRelease(m)
    \/ Expire \/ Unexpire

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ fence \in 0..MaxFence
    /\ attempts \in 0..(K + 1)
    /\ holder = None \/ (holder.name \in Executors /\ holder.fence \in Fences)
    /\ \A e \in Executors : held[e] \in 0..MaxFence

\* One writer: the journal never takes a write from an epoch older than one
\* it already took a write from, and only from epochs that were handed out.
JournalMonotonic ==
    \A i, j \in DOMAIN journal : i < j => journal[i].fence <= journal[j].fence
OnlyIssuedEpochs ==
    \A i \in DOMAIN journal : journal[i].fence <= fence

\* The guard seals only after K holder deaths without accepted progress...
NoFalsePoison == sealed => dry >= K
\* ...and never lets a turn die more than K times without progress.
GuardBounds == dry <= K
=============================================================================
