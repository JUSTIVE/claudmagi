---------------------------- MODULE Polling ----------------------------
\* The generation guard (#197).
\*
\* A poll reads the session registry on a background thread and comes back
\* some time later. In between, a test-tools button may have changed the
\* board *in hand* — and the snapshot in flight knows nothing about that, so
\* laying it down would undo what was just done. The board counts its
\* hand-edits; a poll carries the count it set out with.
\*
\* Rust: `ui::state::landing`, which `Board::take_polled` asks before it
\* lays anything down. `shown` here is "the board is showing the world as of
\* generation n" — a poll that set out at n carries a picture of that world.
\*
\* What a unit test cannot do and this can: there may be several polls in
\* flight at once, they may come back in any order, and edits may land
\* between any two of them.

EXTENDS Naturals

CONSTANT MaxEdits

VARIABLES gen, shown, flight

vars == <<gen, shown, flight>>

\* `flight` is the set of generations the polls now in flight set out with.
TypeOK ==
    /\ gen \in 0..MaxEdits
    /\ shown \in 0..MaxEdits
    /\ flight \subseteq (0..MaxEdits)

Init ==
    /\ gen = 0
    /\ shown = 0
    /\ flight = {}

\* A poll sets out, carrying the count the board has now.
SetOut ==
    /\ flight' = flight \cup {gen}
    /\ UNCHANGED <<gen, shown>>

\* A test-tools button changes the board in hand and bumps the count.
HandEdit ==
    /\ gen < MaxEdits
    /\ gen' = gen + 1
    /\ shown' = gen + 1
    /\ UNCHANGED flight

\* A poll comes back. It lands only if nothing happened in hand while it
\* was away; otherwise it is dropped without a word.
Land(p) ==
    /\ p \in flight
    /\ flight' = flight \ {p}
    /\ IF p = gen THEN shown' = p ELSE UNCHANGED shown
    /\ UNCHANGED gen

Next ==
    \/ SetOut
    \/ HandEdit
    \/ \E p \in flight: Land(p)

Spec == Init /\ [][Next]_vars

-----------------------------------------------------------------------
\* What the board shows is never older than the last edit made by hand.
\* Drop the guard from `take_polled` and TLC finds the three-step counter-
\* example at once: a poll sets out, an edit lands, the poll comes back.
NeverUndone == shown = gen

\* And nothing but a landing or an edit moves what is shown.
OnlyLandingsAndEditsShow == [][ shown' # shown => shown' = gen' ]_vars
=======================================================================
