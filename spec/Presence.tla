---------------------------- MODULE Presence ---------------------------
\* Whether a pane is still there (#186, #192).
\*
\* A session whose tab was closed goes on running, and the board went on
\* drawing a tile for it under a tab nobody could open. Only the terminal
\* can say the seat is gone — and only when it is sure. So a pane has to be
\* missing from a *good* answer for its whole wait before the board lets go,
\* and an answer that is not an answer (a database missing, locked or
\* refused, #67) says nothing at all.
\*
\* Rust: `host::warp::Presence::next`, which `Warp::gone_at` asks once per
\* seat per reading.
\*
\* Time is counted as "how long it has been missing", capped at the wait, so
\* the model is finite without an arbitrary end of time.

EXTENDS Naturals

CONSTANT Wait

VARIABLES pres, age, seen

vars == <<pres, age, seen>>

TypeOK ==
    /\ pres \in {"Listed", "Missing", "Gone"}
    /\ age \in 0..Wait
    /\ seen \in {"None", "There", "NotThere", "Unknown"}

Init ==
    /\ pres = "Listed"
    /\ age = 0
    /\ seen = "None"

\* Time passing between readings. It only matters while something is
\* missing, and it stops at the wait: longer than enough is enough.
Tick ==
    /\ age < Wait
    /\ age' = age + 1
    /\ UNCHANGED <<pres, seen>>

\* One reading of the terminal's own list, about this one pane.
See(s) ==
    /\ seen' = s
    /\ IF s \in {"There", "Unknown"}
         THEN /\ pres' = "Listed"
              /\ age' = 0
         ELSE /\ pres' = CASE pres = "Listed"  -> "Missing"
                           [] pres = "Missing" -> IF age >= Wait THEN "Gone" ELSE "Missing"
                           [] OTHER            -> "Gone"
              /\ age' = IF pres = "Listed" THEN 0 ELSE age

Next == Tick \/ (\E s \in {"There", "NotThere", "Unknown"}: See(s))

Spec == Init /\ [][Next]_vars

-----------------------------------------------------------------------
\* A pane is never let go before its whole wait has passed.
GoneOnlyAfterTheWait == (pres = "Gone") => (age >= Wait)

\* One unlucky reading is never enough: a pane that the board had listed
\* cannot be gone in the same step. The wait has to span more than one
\* fetch, so the list must say the same thing at least twice.
NeverGoneInOneStep == [][ (pres = "Listed") => (pres' # "Gone") ]_vars

\* The two readings that never condemn anything. A board that drops
\* sessions on a maybe is worse than one that keeps a stale tile.
ListingItKeepsIt == [][ (seen' = "There") => (pres' = "Listed") ]_vars
NotKnowingKeepsIt == [][ (seen' = "Unknown") => (pres' = "Listed") ]_vars

\* And the other side of it: once the wait has passed, a good answer that
\* still does not list the pane does let it go. This is stated as a step
\* rather than as "eventually gone" on purpose — if the terminal never
\* gives a good answer again, the board is right to hold the tile for ever.
WaitedAndStillMissingGoes ==
    [][ (pres = "Missing" /\ age >= Wait /\ seen' = "NotThere") => (pres' = "Gone") ]_vars
=======================================================================
