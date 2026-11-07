#!/bin/sh
# Cold-cache scan measurement, one tool at a time (docs/medicion-pc-antiguo.md, step 7).
#
# Usage, from the repository root:
#   sh bench/fria.sh TREE OUT_DIR [RUNS]
#
# Drops the page cache before every run (sudo is needed only for that) and
# runs each command under GNU time. Only reads metadata of TREE; writes only
# to OUT_DIR.
set -eu

usage='uso: sh bench/fria.sh ARBOL DIRECTORIO_DE_SALIDA [CORRIDAS]'
tree=${1:?$usage}
out=${2:?$usage}
runs=${3:-1}
gx=target/release/gx

[ -x "$gx" ] || { echo "falta $gx: cargo build --release -p gx-cli --features uring" >&2; exit 1; }
[ -d "$tree" ] || { echo "no existe el árbol $tree" >&2; exit 1; }
[ -x /usr/bin/time ] || { echo "falta /usr/bin/time (paquete time)" >&2; exit 1; }
mkdir -p "$out"

sudo -v
# Keep sudo's timestamp fresh: one cold run can outlast its 5 minutes.
( while kill -0 $$ 2>/dev/null; do sudo -n -v 2>/dev/null; sleep 60; done ) &
keeper=$!
trap 'kill $keeper 2>/dev/null' EXIT INT TERM

{
    echo "fecha: $(date -Iseconds)"
    echo "commit: $(git rev-parse --short HEAD)"
    echo "kernel: $(uname -r)"
    echo "governor: $(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null || echo '?')"
    echo "corriente: $(cat /sys/class/power_supply/*/online 2>/dev/null | tr '\n' ' ')"
    echo "árbol: $tree"
} > "$out/entorno.txt"

measure() {
    name=$1
    shift
    i=1
    while [ "$i" -le "$runs" ]; do
        sync
        echo 3 | sudo tee /proc/sys/vm/drop_caches > /dev/null
        echo "$(date +%T) $name, corrida $i de $runs: $*"
        /usr/bin/time -v -o "$out/$name-$i.time" "$@" > "$out/$name-$i.out" 2>&1
        grep -E 'Elapsed|Maximum resident' "$out/$name-$i.time" | sed 's/^/    /'
        grep -E '^(Nodos|Tiempo):' "$out/$name-$i.out" | sed 's/^/    /' || true
        i=$((i + 1))
    done
}

measure gx-sync "$gx" scan "$tree"
measure du du -s -x "$tree"
measure gx-uring "$gx" scan "$tree" --statx uring
measure gx-1hilo "$gx" scan "$tree" --threads 1
echo "Listo. Resultados en $out"
