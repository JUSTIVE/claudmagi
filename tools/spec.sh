#!/usr/bin/env bash
# Model-checks every spec in spec/ with TLC (#250).
#
# The specs are not a second copy of the code: they are the handful of rules
# whose correctness is about *ordering* — several polls in flight, three
# workers sharing one map, a clock that has to pass twice. `cargo test` can
# only ever try one interleaving of those; TLC tries all of them.
#
# The toolchain is one jar. It is fetched once into a cache rather than kept
# in the repository, because it is 2MB of somebody else's build.
set -euo pipefail

JAR="${TLA_TOOLS:-$HOME/.cache/tlaplus/tla2tools.jar}"
HERE="$(cd "$(dirname "$0")/.." && pwd)"

if [ ! -f "$JAR" ]; then
    echo "  fetching tla2tools.jar into $(dirname "$JAR")"
    mkdir -p "$(dirname "$JAR")"
    curl -sSL --max-time 180 -o "$JAR" \
        https://github.com/tlaplus/tlaplus/releases/latest/download/tla2tools.jar
fi

cd "$HERE/spec"
failed=0
for cfg in *.cfg; do
    name="${cfg%.cfg}"
    # -deadlock: these models stop when there is nothing left to do, which
    # is an end, not a fault.
    out="$(java -XX:+UseParallelGC -cp "$JAR" tlc2.TLC -deadlock -cleanup \
        -config "$name.cfg" "$name.tla" 2>&1 || true)"
    if printf '%s' "$out" | grep -q "Model checking completed. No error has been found"; then
        states="$(printf '%s' "$out" | grep -oE "[0-9]+ states generated" | head -1)"
        printf "  ok    %-12s %s\n" "$name" "$states"
    else
        failed=1
        printf "  FAIL  %s\n" "$name"
        printf '%s\n' "$out" | tail -n 30 | sed 's/^/        /'
    fi
done

if [ "$failed" -ne 0 ]; then
    echo
    echo "  a spec did not hold. Either the code is wrong or the spec is out of"
    echo "  date with it — and which one it is, is the question worth asking."
    exit 1
fi
