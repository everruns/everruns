---------------------------- MODULE Feed ----------------------------
(***************************************************************************)
(* The data plane -> control plane feed for one source cell.               *)
(*                                                                          *)
(* One Raft entry holds many batched commands, and a command can emit       *)
(* several records, so a record's identity is (index, ordinal); the design  *)
(* adds the cell's incarnation, which is constant within this model.       *)
(* Batches are resent from the last acknowledged cursor and may be          *)
(* delayed, duplicated and reordered; the CP stores a record only if it is  *)
(* the next one after its cursor.                                           *)
(*                                                                          *)
(* With ByIndexOnly = TRUE the cursor is the Raft index alone, as the first *)
(* draft of the protocol had it; TLC then finds a record silently skipped.  *)
(***************************************************************************)
EXTENDS Naturals, Sequences

CONSTANTS MaxEntries, MaxPerEntry, ByIndexOnly

VARIABLES
    entries,  \* entries[i] = number of records entry i produced
    stored,   \* records the CP stored, in order: <<index, ordinal>>
    acked,    \* the cell's view of the CP's cursor
    net       \* batches ever sent

vars == <<entries, stored, acked, net>>

Records == { <<i, o>> : i \in 1..Len(entries), o \in 1..MaxPerEntry }

Exists(r) == r[1] <= Len(entries) /\ r[2] <= entries[r[1]]

After(r, c) == r[1] > c[1] \/ (r[1] = c[1] /\ r[2] > c[2])

\* The record that directly follows cursor c, if it exists.
Succ(c) == IF c[1] >= 1 /\ c[2] < entries[c[1]] THEN <<c[1], c[2] + 1>> ELSE <<c[1] + 1, 1>>

Cursor == IF stored = <<>> THEN <<0, 0>> ELSE stored[Len(stored)]

Init ==
    /\ entries = <<>>
    /\ stored = <<>>
    /\ acked = <<0, 0>>
    /\ net = {}

Produce(n) ==
    /\ Len(entries) < MaxEntries
    /\ n \in 1..MaxPerEntry
    /\ entries' = Append(entries, n)
    /\ UNCHANGED <<stored, acked, net>>

\* Send everything after the acknowledged cursor (records in order).
SendBatch ==
    /\ LET rs == { r \in Records : Exists(r) /\ After(r, acked) } IN
       /\ rs # {}
       /\ net' = net \cup {rs}
    /\ UNCHANGED <<entries, stored, acked>>

\* The CP walks a batch in log order and stores each record that is new.
Accepts(r, c) == IF ByIndexOnly THEN r[1] > c[1] ELSE r = Succ(c)

RECURSIVE Ingest(_, _)
Ingest(rs, s) ==
    LET c == IF s = <<>> THEN <<0, 0>> ELSE s[Len(s)]
        next == { r \in rs : Accepts(r, c) }
    IN IF next = {} THEN s
       ELSE LET r == CHOOSE r \in next : \A q \in next : ~After(r, q) IN
            Ingest(rs \ {r}, Append(s, r))

Receive(b) ==
    /\ b \in net
    /\ stored' = Ingest(b, stored)
    /\ UNCHANGED <<entries, acked, net>>

Ack ==
    /\ acked # Cursor
    /\ acked' = Cursor
    /\ UNCHANGED <<entries, stored, net>>

Next == (\E n \in 1..MaxPerEntry : Produce(n)) \/ SendBatch \/ (\E b \in net : Receive(b)) \/ Ack

Spec == Init /\ [][Next]_vars

\* The CP holds exactly the first Len(stored) records of the log, each once.
NoGapNoDuplicate ==
    \A k \in 1..Len(stored) :
        /\ Exists(stored[k])
        /\ stored[k] = IF k = 1 THEN <<1, 1>> ELSE Succ(stored[k - 1])
=============================================================================
