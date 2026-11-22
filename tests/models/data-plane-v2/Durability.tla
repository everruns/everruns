---------------------------- MODULE Durability ----------------------------
(***************************************************************************)
(* What a shard of three zones keeps of an answered write when two zones   *)
(* are lost (design.md, "Disaster recovery"; agentd ship.rs, restore.rs).  *)
(*                                                                          *)
(* Entries are indices 1..MaxIndex of one log; Raft's log matching is      *)
(* openraft's and assumed, so a replica's disk is the prefix it fsynced.   *)
(* Some entries must survive anything short of losing every disk (a tool   *)
(* call's start, a session's create, a move step). The leader answers one  *)
(* only once every voter holds it, or once the shipped log in the bucket   *)
(* does; it ships committed entries at any time, in order. Then up to two  *)
(* zones are lost, disks and all, and the shard is rebuilt from the bucket *)
(* plus whatever disk is left.                                              *)
(*                                                                          *)
(* Two bug configurations must keep failing:                                *)
(*  AnswerOnQuorum: answered once a quorum holds it (a plain commit).       *)
(*  BucketOnly: the rebuild ignores the surviving disk.                    *)
(***************************************************************************)
EXTENDS Integers, FiniteSets

CONSTANTS MaxIndex, AnswerOnQuorum, BucketOnly

Replicas == {1, 2, 3}
Leader == 1

VARIABLES
    held,      \* held[r]: highest index replica r fsynced (0: none)
    lost,      \* zones lost, disks and all
    shipped,   \* highest index in the bucket's shipped log
    answered,  \* must-survive entries answered to their writer
    restored   \* highest index the rebuild has (-1: not rebuilt)

vars == <<held, lost, shipped, answered, restored>>

Max(S) == CHOOSE m \in S : \A x \in S : x <= m

Committed == Max({i \in 0..MaxIndex :
                    Cardinality({r \in Replicas : held[r] >= i}) >= 2})

Live == restored = -1 /\ Leader \notin lost /\ Cardinality(lost) = 0

Init ==
    /\ held = [r \in Replicas |-> 0]
    /\ lost = {}
    /\ shipped = 0
    /\ answered = {}
    /\ restored = -1

\* The leader appends and fsyncs the next entry.
Append ==
    /\ Live
    /\ held[Leader] < MaxIndex
    /\ held' = [held EXCEPT ![Leader] = @ + 1]
    /\ UNCHANGED <<lost, shipped, answered, restored>>

\* A follower fsyncs one more entry of the leader's, then acknowledges.
Replicate(r) ==
    /\ restored = -1
    /\ r # Leader /\ r \notin lost /\ Leader \notin lost
    /\ held[r] < held[Leader]
    /\ held' = [held EXCEPT ![r] = @ + 1]
    /\ UNCHANGED <<lost, shipped, answered, restored>>

\* The leader ships committed entries (the 10 s loop, or a forced flush).
Ship ==
    /\ Live
    /\ shipped < Committed
    /\ shipped' \in (shipped + 1)..Committed
    /\ UNCHANGED <<held, lost, answered, restored>>

Safe(i) ==
    IF AnswerOnQuorum THEN i <= Committed
    ELSE (\A r \in Replicas : held[r] >= i) \/ shipped >= i

\* The writer of must-survive entry i gets its answer.
Answer(i) ==
    /\ Live
    /\ i \in 1..held[Leader]
    /\ i \notin answered
    /\ Safe(i)
    /\ answered' = answered \cup {i}
    /\ UNCHANGED <<held, lost, shipped, restored>>

\* A zone goes, disk and all; at most two.
Lose(r) ==
    /\ restored = -1
    /\ r \notin lost
    /\ Cardinality(lost) < 2
    /\ lost' = lost \cup {r}
    /\ held' = [held EXCEPT ![r] = 0]
    /\ UNCHANGED <<shipped, answered, restored>>

\* Quorum is gone: rebuild from the bucket plus the disk that is left.
Rebuild ==
    /\ restored = -1
    /\ Cardinality(lost) = 2
    /\ restored' = IF BucketOnly THEN shipped
                   ELSE Max({shipped} \cup {held[r] : r \in Replicas \ lost})
    /\ UNCHANGED <<held, lost, shipped, answered>>

Next ==
    \/ Append \/ Ship \/ Rebuild
    \/ \E r \in Replicas : Replicate(r) \/ Lose(r)
    \/ \E i \in 1..MaxIndex : Answer(i)

Spec == Init /\ [][Next]_vars

\* A rebuilt shard has every must-survive write that was answered.
NoAnsweredLoss == restored >= 0 => \A i \in answered : i <= restored
=============================================================================
