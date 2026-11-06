# Pendientes

Ideas que surgen durante el trabajo y que **no** entran en la subfase actual. Se revisan al planificar cada fase.

Formato: `- [fase sugerida] idea — origen (subfase o conversación)`

- [F2] Clasificación por magic bytes además de la extensión — diseño F0
- [F5] Tamaño exclusivo de snapshots btrfs vía qgroups — diseño F0
- [F3] Mutación en vivo del arena con tombstones y compactación — diseño F0 §4
- [F0.1] Job de CI que compile y pruebe con la MSRV (Rust 1.85), no solo con stable — F0.0
- [F0.4] El lector de `bench/queries.txt` debe ignorar las líneas que empiezan con `#` (el archivo ya trae un comentario) — F0.0
- [F0.2] Columnas segmentadas (sin reservar toda la capacidad virtual de una vez) solo si la escalera de capacidad no alcanza en algún equipo; cuesta una indirección por acceso y requiere benchmark — F0.1, punto 10 del plan
- [F0.2] `Column` concurrente (`Sync`) con tokens de rango disjuntos; en F0.1 es `Send` y no `Sync` — F0.1
- [F0.2] En `fotografo` casi todas las carpetas tienen hard links en su subárbol: a 100 000 archivos solo 22 son elegibles para la comparación por carpeta de §V1.2. Evaluar menos hard links en ese perfil o una comparación que siga el orden de recorrido de `du` — F0.1
- [F0.2] El planificador del generador usa `ln`, `exp` y `powf` de libm: el mismo seed podría dar árboles distintos entre x86_64 y aarch64. Determinismo garantizado solo dentro de una misma plataforma — F0.1
- [F1] Un bind mount del mismo sistema de archivos tiene el mismo `st_dev`: `du -x` lo recorre y nosotros lo marcamos `BIND_ALIAS`. Diferencia esperada en árboles reales con binds — F0.1
- [F0.2] `rotational` queda en 2 (desconocido) para tmpfs y otros sistemas sin dispositivo de bloques; decidir los hilos para ese caso — F0.1
