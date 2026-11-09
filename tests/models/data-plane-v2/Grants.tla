---------------------------- MODULE Grants ----------------------------
(***************************************************************************)
(* Spend allowances: the control plane (CP) grants a cell money from an    *)
(* org's balance; the cell spends locally and settles per grant.           *)
(*                                                                          *)
(* Data plane v2 design ("strict cap on authorized spend"):                 *)
(*  - every grant has an id and is reserved in the ledger when issued;      *)
(*  - each chargeable operation reserves its maximum cost before it starts  *)
(*    and settles its actual cost (at most the maximum) after;              *)
(*  - settlements are cumulative and may be delayed or duplicated;          *)
(*  - expiry stops spending; money returns to the balance only through a    *)
(*    closed settlement, never by time;                                     *)
(*  - an operation whose process died stays reserved ("unresolved");        *)
(*  - a lost cell's open grants count as fully spent.                       *)
(*                                                                          *)
(* With ExpiryReleases = TRUE the CP treats an expired grant as closed at   *)
(* the usage it has heard of so far, as the first draft of the protocol     *)
(* did; TLC then finds real spend passing the balance.                      *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets

CONSTANTS Balance, Grants, MaxGrant, ExpiryReleases

VARIABLES
    cp,     \* cp[g]: the CP ledger's view of grant g
    cell,   \* cell[g]: the cell's view of grant g
    lost,   \* the cell was lost (quorum loss) and reports nothing more
    net     \* settlements ever sent

vars == <<cp, cell, lost, net>>

Init ==
    /\ cp = [g \in Grants |-> [issued |-> FALSE, amt |-> 0, used |-> 0, closed |-> FALSE, expired |-> FALSE]]
    /\ cell = [g \in Grants |-> [has |-> FALSE, amt |-> 0, used |-> 0, op |-> 0, expired |-> FALSE, closed |-> FALSE]]
    /\ lost = FALSE
    /\ net = {}

RECURSIVE Sum(_, _)
Sum(S, F) == IF S = {} THEN 0 ELSE LET x == CHOOSE x \in S : TRUE IN F[x] + Sum(S \ {x}, F)

\* What the CP counts against the balance: a closed grant at its settled
\* usage, any other issued grant at its full amount.
Held == [g \in Grants |-> IF ~cp[g].issued THEN 0 ELSE IF cp[g].closed THEN cp[g].used ELSE cp[g].amt]
Committed == Sum(Grants, Held)

\* What providers actually billed, settled or not.
RealSpend == Sum(Grants, [g \in Grants |-> cell[g].used])

Issue(g, amt) ==
    /\ ~cp[g].issued
    /\ amt \in 1..MaxGrant
    /\ Committed + amt <= Balance
    /\ cp' = [cp EXCEPT ![g] = [issued |-> TRUE, amt |-> amt, used |-> 0, closed |-> FALSE, expired |-> FALSE]]
    /\ cell' = IF lost THEN cell
               ELSE [cell EXCEPT ![g] = [has |-> TRUE, amt |-> amt, used |-> 0, op |-> 0, expired |-> FALSE, closed |-> FALSE]]
    /\ UNCHANGED <<lost, net>>

\* An operation reserves its maximum cost before it starts (one at a time
\* per grant keeps the model small).
Start(g, max) ==
    /\ ~lost
    /\ cell[g].has /\ ~cell[g].expired /\ ~cell[g].closed /\ cell[g].op = 0
    /\ max \in 1..MaxGrant
    /\ cell[g].used + max <= cell[g].amt
    /\ cell' = [cell EXCEPT ![g].op = max]
    /\ UNCHANGED <<cp, lost, net>>

\* The provider bills at most the reserved maximum. This can happen after the
\* cell is lost: the bill comes, but nobody reports it.
Finish(g, actual) ==
    /\ cell[g].op > 0
    /\ actual \in 0..cell[g].op
    /\ cell' = [cell EXCEPT ![g].used = @ + actual, ![g].op = 0]
    /\ UNCHANGED <<cp, lost, net>>

Expire(g) ==
    /\ cp[g].issued /\ ~cp[g].expired
    /\ cp' = [cp EXCEPT ![g].expired = TRUE,
                        ![g].closed = IF ExpiryReleases THEN TRUE ELSE @]
    /\ cell' = IF cell[g].has THEN [cell EXCEPT ![g].expired = TRUE] ELSE cell
    /\ UNCHANGED <<lost, net>>

\* The cell reports cumulative usage; it closes a grant once it is expired
\* and nothing is in flight (an unresolved operation keeps it open).
Report(g) ==
    /\ ~lost /\ cell[g].has /\ ~cell[g].closed
    /\ LET close == cell[g].expired /\ cell[g].op = 0 IN
       /\ net' = net \cup {[g |-> g, used |-> cell[g].used, closed |-> close]}
       /\ cell' = [cell EXCEPT ![g].closed = close]
    /\ UNCHANGED <<cp, lost>>

Settle(m) ==
    /\ m \in net
    /\ cp' = [cp EXCEPT ![m.g].used = IF m.used > @ THEN m.used ELSE @,
                        ![m.g].closed = @ \/ m.closed]
    /\ UNCHANGED <<cell, lost, net>>

Lose ==
    /\ ~lost
    /\ lost' = TRUE
    /\ UNCHANGED <<cp, cell, net>>

Next ==
    \/ \E g \in Grants, a \in 1..MaxGrant : Issue(g, a) \/ Start(g, a)
    \/ \E g \in Grants, a \in 0..MaxGrant : Finish(g, a)
    \/ \E g \in Grants : Expire(g) \/ Report(g)
    \/ \E m \in net : Settle(m)
    \/ Lose

Spec == Init /\ [][Next]_vars

\* The product guarantee: providers never billed more than the balance.
StrictCap == RealSpend <= Balance
\* The ledger never authorized more than the balance.
LedgerCap == Committed <= Balance
\* A closed grant's settled usage is everything the cell spent from it.
ClosedIsFinal == \A g \in Grants : (cp[g].closed /\ ~ExpiryReleases) => cp[g].used = cell[g].used
=============================================================================
