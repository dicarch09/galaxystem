# Traspaso de la F0.2 (escáner paralelo) — estado al 2026-11-06, noche

La F0.2 está **implementada y medida en la Victus**. Falta medir el PC antiguo, resolver el job de qemu en la CI y tomar tres decisiones. Después se escribe el reporte final y se espera la aprobación. **No empezar la F0.3.**

Este documento reemplaza al traspaso anterior (el de mitad de implementación) y se archiva al aprobar la F0.2.

## Estado del repositorio

- `main` = `origin/main` = `e636574` (más el commit de este traspaso, solo local).
- Todos los commits de la F0.2 están en GitHub, de `7fea1dc` a `e636574`. Los últimos:
  - `23fd336 refactor(scan): use ? when queueing a task in another pool`: arreglo del lint `question_mark` de clippy 1.99.
  - `e636574 docs: add measurement steps for the old laptop`: `docs/medicion-pc-antiguo.md`.
- Verificación local en `23fd336`: fmt, clippy (Rust 1.95 y 1.99.1, con y sin `--all-features`) y las pruebas de `gx-scan`, todo OK.

### CI

| Corrida | Resultado |
|---|---|
| `e2032b5` (42294357087) | fmt, deny, test (x86, incluidos uring y el test de FUSE) y cross-aarch64 OK. **clippy falló** (ya corregido en `23fd336`). **test-aarch64-qemu cancelado** a los 30 min: `hung_network_mount_is_stale_and_the_rest_is_complete` se queda colgado |
| `e636574` (42303841367) | En curso al cerrar la sesión. Se espera clippy OK y qemu otra vez colgado |

## Pendiente de Ricardo

1. **PC antiguo:** caché caliente medida (ver abajo). Falta repetir la caché fría, que salió inválida, anotar la mediana fría hasta 10 000 nodos y repetir `gx verify-du ~/gx-arboles/home-dev-1m` (no se sabe si pasó; es la primera verificación de exactitud en ext4).
2. **Victus, repetir el tiempo frío hasta 10 000 nodos.** Los 105,2 ms (y los 2,18 s de total) se midieron **a batería**, con la preferencia de energía en `balance_power`, y no valen. Con el cargador conectado:
   ```sh
   powerprofilesctl set performance
   cat /sys/class/power_supply/ADP1/online /sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference   # 1 y performance
   sync; echo 3 | sudo tee /proc/sys/vm/drop_caches; target/release/gx scan ~/gx-arboles/home-dev-1m | grep Tiempo
   ```
   Repetir la última línea 5 veces y anotar la mediana del número entre paréntesis.
3. **Opción B elegida para el job de qemu:** reproducir el cuelgue con el mismo qemu de la CI. El clasificador de permisos bloquea al agente al ejecutar ese binario descargado, aunque Ricardo lo aprobó en el chat. Para desbloquearlo, una de dos:
   - Ejecutarlo él mismo con `!` (abajo está el comando).
   - Agregar a `.claude/settings.local.json`, en `permissions.allow`:
     ```json
     "Bash(/home/dawuu/.cache/gx-debug/qemu-aarch64-10.2.3:*)"
     ```

## Diagnóstico del job de qemu (opción B)

- La CI usa `docker/setup-qemu-action@v3` con `tonistiigi/binfmt:latest`, que trae **qemu 10.2.3**; no existe imagen con qemu 11. Rust de la CI: 1.99.1.
- Con el qemu 11.1.3 de la Victus, el test pasa siempre en ~1 s, también con `taskset` a 1, 2 y 4 núcleos.
- Copia del qemu de la CI, guardada fuera de `/tmp` (que es tmpfs y se borra al apagar): `~/.cache/gx-debug/qemu-aarch64-10.2.3`.
  - sha256 del binario: `1ad17b7bd5e15ce60075d0994d5c5e3914d16899a1e3119040b5c9e76e067f24`.
  - Viene de la capa `sha256:56685a25…` de la imagen amd64 `sha256:465d3fdd…`, con el hash verificado.
- Comando para reproducir. Recompilar el test si cambió el código: `cargo test -p gx-scan --target aarch64-unknown-linux-gnu --test hung_mount --no-run`. Usar el ejecutable que imprime.
  ```sh
  timeout 90 ~/.cache/gx-debug/qemu-aarch64-10.2.3 -L /usr/aarch64-linux-gnu target/aarch64-unknown-linux-gnu/debug/deps/hung_mount-48502ebc68708416 --nocapture; echo rc=$?
  ```
  - `rc=0` en ~1 s: no es la versión de qemu. Buscar la diferencia en la CI (kernel, binfmt con flags `F`/`C`, 4 núcleos).
  - `rc=124`: se reprodujo. Siguiente paso: ubicar dónde se traba (gdb con `qemu -g`, o marcas de etapa en el test).
  - Si se cuelga, revisar `grep sshfs /proc/self/mountinfo` y desmontar con `fusermount3 -u <ruta>`.
- Plan B si no se resuelve: saltar ese test solo en el job de qemu (`-- --skip hung_network_mount`), con un comentario y una entrada en pendientes. Era la opción A, que Ricardo no eligió.

## Decisiones abiertas de Ricardo

1. **Memoria temporal del escaneo:** ¿corregirla dentro de la F0.2 o dejarla como pendiente? Recomendación: pendiente prioritaria, junto con el reparto de carpetas gigantes (ver más abajo).
2. **uring o sync por defecto:** esperar los datos del HDD. En la Victus gana sync con caché caliente (319 frente a 420 ms) y uring con caché fría (1431 frente a 1512 ms).
3. **Desviaciones del diseño** (acumuladores en las columnas de tamaño, pools que empiezan con un hilo, tareas con metadatos, pools de red, I7): ¿documentarlas en §2.3 de `F0-diseno.md` o en un ADR?

## Mediciones en el PC antiguo (`home-dev` 1 M, ext4 en HDD, Celeron de 2 núcleos)

Llegaron en `c9baa60` (`bench/results/pc-antiguo/2026-11-06/`, commit `e636574`, kernel 7.0.11, `performance`, con cargador).

| | gx sync | gx uring | dua 3.0.0 | gdu 5.37.0 | du 9.11 |
|---|---|---|---|---|---|
| Caliente, mediana | **3081 ms** | 3901 ms | 8510 ms | 4706 ms | 6378 ms |
| Caliente, p95 | 3222 ms | 4004 ms | 8719 ms | 4908 ms | 6442 ms |
| RAM máxima | 116,8 MiB | 210,1 MiB | 16,3 MiB | 63,2 MiB | 35,1 MiB |
| Hasta 10 000 nodos (caliente, mediana) | 24,1 ms | 31,7 ms | | | |

**Caché fría: no válida.** `scan-fria.json` solo trae gx-sync, con 13, 33 y 372 ms, 0,06 ms de CPU de usuario y 14 MB de RAM, frente a 3 s y 132 MB en caliente. No puede ser un escaneo de 1,14 M nodos; lo más probable es que gx no haya recorrido el árbol en esas corridas (causa sin confirmar). dua no terminó por falta de tiempo y el resto no se corrió. Falta también el tiempo frío hasta 10 000 nodos.

Para repetirla: primero un `gx scan` a mano tras `drop_caches`, comprobando que reporta ~1 140 500 nodos, y después el hyperfine de `resumen.md`. La caché fría en el HDD es lenta: conviene dejarla corriendo con tiempo de sobra o una herramienta a la vez.

## Mediciones en la Victus (`home-dev` 1 M, btrfs en NVMe, con cargador y `performance`)

Los resultados completos están en `bench/results/victus/2026-11-06-3/`. Desde el 2026-11-06 `bench/results/` ya no está en `.gitignore` y se sube al repo, que pasa a ser privado. `2026-11-06-4` es una corrida más de la Victus, guardada antes por error con la etiqueta `pc-antiguo`.

| | gx sync | gx uring | dua 3.0.0 | gdu 5.38.0 | du 9.12 |
|---|---|---|---|---|---|
| Caliente, mediana | **319 ms** | 420 ms | 688 ms | 439 ms | 1394 ms |
| Caliente, p95 | 353 ms | 464 ms | 702 ms | 450 ms | 2245 ms |
| Fría, mediana | 1512 ms | **1431 ms** | 1524 ms | 1580 ms | 8911 ms |
| RAM máxima | 180 MiB | 309 MiB | 8,8 MiB | 50,9 MiB | 31,1 MiB |
| Syscalls (strace) | 1,66 M | 0,62 M | 3,61 M | 1,59 M | 2,40 M |

- Hasta los primeros 10 000 nodos, con caché caliente: 4,6 ms (sync) y 5,9 ms (uring).
- Bytes por nodo en el arena: 59,3 (columnas y nombres) y 60,6 en total.
- Criterios de §V3:
  - §V1 en los 4 perfiles: cumple.
  - Caché caliente ≤ el mejor entre dua y gdu: cumple (1,38× más rápido que gdu).
  - B/nodo del arena ≤ 70: cumple.
  - Montaje colgado: cumple en x86.
  - A/B de `statx`: falta el HDD.

## Hallazgo: pico de RAM durante el escaneo (sin corregir)

Medido con `/proc/PID/smaps` en el momento de máxima memoria. Sin THP significa con las páginas enormes desactivadas por `prctl` solo para ese proceso.

| Caso | Pico | Pico sin THP | Columnas sin THP |
|---|---|---|---|
| `home-dev`, un hilo | 131 MiB | 108 MiB | 65,9 MiB |
| `home-dev`, paralelo | 183 MiB | 160 MiB | 65 MiB |
| `home-dev`, uring | 302 MiB | 283 MiB | 65 MiB |
| `patologico`, un hilo | 259 MiB | 252 MiB | 57 MiB |
| `patologico`, paralelo | 329 MiB | 302 MiB | 65 MiB |
| `patologico`, uring | 628 MiB | 614 MiB | 65 MiB |

Causas:
1. **THP en `always`** redondea cada columna a páginas de 2 MiB: unos 22 MiB fijos.
2. **El `Vec<Child>` por hilo** guarda el `statx` completo, 288 B por entrada. Se queda del tamaño de la carpeta más grande que vio ese hilo y no se encoge.
   - Lo confirmó gdb sobre el `realloc` en `walk.rs:347`.
   - La carpeta más grande de `home-dev` (105 967 entradas) ocupa 36 MiB, y la de `patologico` (500 000 entradas) 144 MiB.
3. **uring suma dos buffers más por entrada:** `batch_out` (264 B) y los buffers del anillo (256 B).

Corrección propuesta, sin implementar:
- Listar la carpeta guardando solo los nombres.
- Reservar los ids, para mantener I2.
- Hacer `statx` y escribir por bloques de unas 4096 entradas, con registros compactos de unos 80 B.

Es el mismo cambio que el reparto de carpetas gigantes que ya está en `pendientes.md`. Requiere un benchmark antes y después.

Herramientas de medición guardadas en `~/.cache/gx-debug/`: `smaps_peak.py`, `rss_matrix.sh` y `nothp.c` (compilar con `gcc -O2 -o nothp nothp.c`).

## Al volver

1. `gh run view 42303841367`: confirmar que clippy pasó y ver el job de qemu.
2. Pedirle a Ricardo: los resultados del PC antiguo, la mediana fría hasta 10 000 nodos en la Victus con cargador, y el desbloqueo de la opción B (o que ejecute el comando con `!`).
3. Diagnosticar el job de qemu y corregirlo.
4. Escribir el reporte final de la F0.2 con `templates/reporte-subfase.md`: el informe ya entregado en el chat, más el PC antiguo, la memoria y la CI. Detenerse.
5. Al aprobarse la F0.2: archivar este traspaso y actualizar `pendientes.md` con lo decidido.

## Herramientas en la Victus

- hyperfine 1.20.0 y dua 3.0.0, instalados con `cargo install`.
- gdu 5.38.0, strace y GNU time, instalados con pacman.
- Toolchain 1.99.1 de rustup, igual a la de la CI, solo para clippy. El `stable` local sigue en 1.95.
- nightly + rust-src (TSan).
- gdb y qemu-user 11.1.3.
