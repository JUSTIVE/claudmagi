--------------------------- MODULE PrWorkers ---------------------------
\* The pull-request worker pool (#64, #247).
\*
\* Three worker threads share one list of what the board wants and one map
\* of what `gh` has said about each of them. A worker takes the next pull
\* request nobody is holding — one never asked about first, then one whose
\* answer has aged out — asks `gh`, and puts the answer down.
\*
\* Rust: `pr::Shared::claim` and `pr::Shared::store`, with `pr::ask` naming
\* the four states and `pr::again_after` deciding when Fresh turns Stale.
\*
\* What a unit test cannot do and this can: the three workers interleave
\* freely, and the answer of one may age while another is still deciding
\* what to take.

EXTENDS Naturals, FiniteSets

CONSTANTS Count, Workers, NoRef

\* The board's list is a list: `claim` walks it with `find` and takes the
\* **first** one it may have. Modelling it as a set instead lets a worker
\* pick any of them, which is a different program — and a worse one. TLC
\* says so in nine steps: with a free choice, two pull requests can be
\* handed round for ever while a third is never taken.
Refs == 1..Count

VARIABLES ask, held

vars == <<ask, held>>

Standing == {"Unasked", "Asking", "Fresh", "Stale"}

TypeOK ==
    /\ ask \in [Refs -> Standing]
    /\ held \in [Workers -> Refs \cup {NoRef}]

Init ==
    /\ ask = [r \in Refs |-> "Unasked"]
    /\ held = [w \in Workers |-> NoRef]

Takeable(r) == ask[r] \in {"Unasked", "Stale"}
NothingUnasked == \A q \in Refs: ask[q] # "Unasked"

\* A worker takes one: never-asked first, and only then one gone stale —
\* and in both cases the first in the board's own order.
First(kind) == CHOOSE r \in Refs: ask[r] = kind /\ \A q \in Refs: (ask[q] = kind) => (r =< q)

Claim(w, r) ==
    /\ held[w] = NoRef
    /\ Takeable(r)
    /\ IF \E q \in Refs: ask[q] = "Unasked"
         THEN r = First("Unasked")
         ELSE r = First("Stale")
    /\ ask' = [ask EXCEPT ![r] = "Asking"]
    /\ held' = [held EXCEPT ![w] = r]

\* `gh` answered: the worker puts the answer down and lets go.
Store(w) ==
    /\ held[w] # NoRef
    /\ ask' = [ask EXCEPT ![held[w]] = "Fresh"]
    /\ held' = [held EXCEPT ![w] = NoRef]

\* Time passes and an answer ages out of its own freshness. How long that
\* takes is what the pull request's own state decides (#247); here it may
\* happen at any moment, which covers every pace at once.
Age(r) ==
    /\ ask[r] = "Fresh"
    /\ ask' = [ask EXCEPT ![r] = "Stale"]
    /\ UNCHANGED held

Next ==
    \/ \E w \in Workers: (\E r \in Refs: Claim(w, r)) \/ Store(w)
    \/ \E r \in Refs: Age(r)

Fair ==
    \A w \in Workers:
        /\ WF_vars(Store(w))
        /\ WF_vars(\E r \in Refs: Claim(w, r))

Spec == Init /\ [][Next]_vars /\ Fair

-----------------------------------------------------------------------
\* Two workers never hold the same pull request: `gh` is never asked twice
\* over for one answer, and one `store` never lands on another's work.
OnePerRef ==
    \A w1, w2 \in Workers:
        (w1 # w2 /\ held[w1] # NoRef) => held[w1] # held[w2]

\* "Asking" means exactly "a worker is holding it", from both sides. This
\* is the `inflight` set and the `seen` map agreeing with each other.
AskingIsHeld ==
    \A r \in Refs: (ask[r] = "Asking") <=> (\E w \in Workers: held[w] = r)

\* Nothing the board is showing stays unasked for ever: a tile that says
\* nothing is the one thing the board cannot explain (#112).
EverythingGetsAsked == \A r \in Refs: <>(ask[r] # "Unasked")

\* What is **not** asserted here, and why.
\*
\* `NothingStarves == \A r \in Refs: []<>(ask[r] = "Fresh")` does not hold,
\* and TLC is right to say so: nothing in the pool shares its attention out.
\* It always takes the earliest, so if a pull request could go stale the
\* instant its answer was put down, the workers would hand the first few
\* round for ever and never reach the rest.
\*
\* What keeps that from happening is **arithmetic, not the pool**: the
\* shortest freshness is twelve seconds (#247) and a whole sweep of the
\* board is about four (thirteen pull requests, a second a call, three
\* workers). A pull request cannot come back round before the others have
\* had their turn. That is a rule about numbers, so it is kept where the
\* numbers are — `pr::MOVING` and its neighbours — and not pretended to be
\* a property of the algorithm.
=======================================================================
