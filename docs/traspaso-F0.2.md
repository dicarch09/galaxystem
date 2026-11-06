# Traspaso de la F0.2 (escáner paralelo) — estado al 2026-11-06

La F0.2 está **en curso**. El plan está aprobado, igual que el ADR-021 y `fuser` como dev-dependency. Este documento sirve para retomar en una sesión nueva sin perder contexto. Al terminar la F0.2 se borra o se archiva.

## Hecho

| Commit | Qué |
|---|---|
| `6312dca` | `docs/hardware.md` actualizado: el disco del PC antiguo es un HDD |
| `6baa0f4` | ADR-021: el tamaño aparente sigue a `du --apparent-size` |
| `674c79b` | `Column` y `ArenaBuilder` compartibles entre hilos: celdas atómicas `Relaxed`, `freeze()` reinterpreta la memoria, tablas detrás de `Mutex` y caché local por hilo (`LocalCache`). El walker de un hilo no cambia de comportamiento |

En `674c79b` pasan `fmt`, clippy, `cargo test` (55 pruebas, 4 ignoradas), las pruebas `--ignored` de 100.000 (4), `deny` y el build para aarch64. La CI del último push (`e31d829`) pasó completa, incluido el job de qemu.

## Herramientas en la Victus

| Herramienta | Estado |
|---|---|
| hyperfine, dua | Instaladas con `cargo install` en `~/.cargo/bin` (anotar `--version` en el informe) |
| nightly + rust-src | Instalado con rustup (para TSan) |
| fusermount3, `/dev/fuse` | Disponibles |
| `io_uring` | Habilitado (`kernel.io_uring_disabled = 0`) |
| **gdu, strace, GNU time** | **Faltan.** Ricardo: `sudo pacman -S gdu strace time` |
| Governor de CPU | `powersave` en la última revisión; `hardware.md` deja vacío el perfil al medir |

## Lo que falta, en orden (plan aprobado)

1. **Escáner paralelo** en `gx-scan`: un módulo nuevo `par`; el actual queda como `ScanMode::Sequential`, la referencia para la equivalencia.
   - Pools por sistema de archivos, con clave `major:minor` de mountinfo: los subvolúmenes btrfs comparten pool.
   - `crossbeam-deque` (un `Injector` y un `Worker`/`Stealer` por hilo) dentro de `std::thread::scope`. Un pool nuevo se crea de forma perezosa cuando aparece un `MOUNT_POINT`.
   - Hilos: SSD → `min(núcleos×2, 32)`, HDD → 2, desconocido → núcleos; todo configurable en `ScanOptions`.
   - El escaneo termina cuando un contador global de tareas llega a 0. El hilo principal reporta el progreso cada 50 ms.
   - Hard links en 64 shards `Mutex<HashSet>`. Estadísticas y errores por hilo, combinados al final.
2. **Agregación concurrente (§2.3):** columnas transitorias por fila de carpeta: `pending: Column<u32>` y acumuladores `Column<u64>` (ya existen `fetch_sub` y `fetch_add`).
   - `pending[D] = 1 + subcarpetas que aportan`, fijado antes de encolarlas.
   - Al llegar a 0, D escribe su total y lo propaga al padre; las fronteras de dispositivo (ADR-020) no propagan.
3. **Red:** hilos auxiliares desechables (`std::thread::spawn`, fuera del scope) con `recv_timeout(5 s)`; si vence, la carpeta queda `STALE`.
   - Máximo 4 hilos abandonados por montaje.
   - El `statx` de un punto de montaje de red, desde un padre local, también va con timeout.
4. **Prueba de montaje colgado** con `fuser` (dev-dependency, MIT): `lookup`/`getattr` responden, `readdir` se bloquea, y el subtipo es `sshfs` para que mountinfo lo vea como red. Si no hay FUSE, la prueba se omite con aviso. Justificar `fuser` en el reporte y pasar `cargo deny`.
5. **Feature `uring`** (crate `io-uring`): lotes de `IORING_OP_STATX` por directorio, un ring por hilo, `unsafe` en `gx-scan::sys`, backend elegido en ejecución (`--statx sync|uring`) y caída a `sync` si no hay `io_uring`.
6. **`gx bench scan`** en `gx-bench`:
   - hyperfine `--export-json` sobre gx (sync y uring), dua, gdu y `du`;
   - `/usr/bin/time -v` y `strace -c -f` en una corrida aparte;
   - JSON en `bench/results/<equipo>/<fecha>/`;
   - imprime los comandos de caché fría con sudo; mediana y p95 extraídas del JSON con un lector mínimo, sin `serde_json`;
   - verificar con `--help` las opciones no interactivas de dua y gdu.
7. **Pruebas:**
   - equivalencia paralelo vs. un hilo **por ruta** en los 4 perfiles;
   - estrés: 50 corridas de `patologico` a 100.000;
   - TSan: `RUSTFLAGS=-Zsanitizer=thread cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu`;
   - montaje colgado;
   - `verify-du` a 1 M en los 4 perfiles y en `/usr` (informativo).
8. **Mediciones:** las tablas de escaneo y recursos de `templates/informe-benchmark.md` para `home-dev` a 1 M con caché caliente, más el A/B de `statx`. Dejar el comando exacto para repetirlo en ambos equipos con caché fría.
9. Pendientes nuevos: repartir carpetas gigantes en subtareas (una carpeta de 500.000 entradas hoy la procesa un solo hilo), con una estimación del beneficio.
10. Reporte con `templates/reporte-subfase.md`, y detenerse.

## Preguntas para Ricardo (responder al abrir la próxima sesión)

1. **¿Dónde se generan los árboles de 1 M para medir?** `/tmp` es tmpfs (RAM, 7,6 GiB): sirve para la exactitud, pero no representa un disco. Propuesta: medir sobre el NVMe btrfs, por ejemplo con `gx synth --allow-path ~/gx-arboles/home-dev-1m`, y dejar tmpfs solo para las pruebas.
2. **Perfil de energía para medir en la Victus:** ¿`performance` (lo que pide §V2) conectado a la corriente? Si sí, completa la fila de `hardware.md`.
3. **¿Instalaste gdu, strace y GNU time?** Sin ellos, la comparación contra gdu y las tablas de recursos y syscalls quedan incompletas.
4. **CI y FUSE:** ¿se agrega `fuse3` a los jobs de prueba para que la prueba de montaje colgado corra en GitHub? Si no, se omite con aviso.
5. **Si `io_uring` pierde el A/B:** ¿se conserva el código detrás de la feature `uring` (sin activarla por defecto) o se elimina?
6. **¿Un job de CI con TSan (nightly)?** No es obligatorio; agrega unos minutos por corrida.

## Para la sesión nueva

Abrir el agente en la raíz del repo, pegar `prompts/F0.2-escaner-paralelo.md` e indicar: "El plan ya está aprobado; lee `docs/traspaso-F0.2.md` y continúa desde el paso 1, con estas respuestas: …".
