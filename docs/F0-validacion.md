# F0 — Validación y benchmarks

## Reglas generales

- **La exactitud se valida en cada subfase; el rendimiento, desde la F0.2.**
- **Las metas son iniciales.** Se ajustan después de la primera medición real, siempre por escrito en el reporte y con el motivo.
- **Ningún número sin condiciones:** cada medición indica máquina, sistema de archivos, perfil, cantidad de nodos y estado de la caché.

## §V1. Exactitud contra `du`

**Comandos de referencia** (GNU coreutils):

```sh
du -x -s -B1 <ruta>     # uso en disco total, sin cruzar montajes
du -x -s -b  <ruta>     # tamaño aparente total
```

**Pruebas:**

1. **Totales de la raíz.** Para cada perfil sintético y para un árbol real (`~` o `/usr`), los totales de Galaxystem deben ser **idénticos byte a byte** a los de `du`.
2. **Totales por carpeta.** Se eligen 200 carpetas al azar (seed fijo) y se compara cada una con `du -x -s` sobre esa carpeta. Se **excluyen** las carpetas cuyo subárbol contiene hard links, porque la atribución depende del orden de recorrido (`F0-diseno.md` §2.5).
3. **Conteo de entradas.** La cantidad de nodos debe coincidir con `find <ruta> -xdev | wc -l`, descontando los descendientes de carpetas `PSEUDO_FS`, `BIND_ALIAS` y `LOCKED`.

**Árboles reales que cambian durante la prueba** (por ejemplo `~` con un navegador abierto) pueden dar diferencias legítimas. Para los criterios de aceptación valen **solo los árboles sintéticos**, que no cambian; el árbol real se reporta como informativo.

**Herramienta:** el subcomando `gx verify-du <ruta>` ejecuta las tres pruebas y produce un resumen con los totales, las diferencias y las carpetas que no coinciden.

## §V2. Benchmarks

**Herramientas a comparar** (anotar la versión exacta de cada una):

| Herramienta | Para qué |
|---|---|
| `du` | Referencia de exactitud y de línea base |
| `dua` (dua-cli) | Rival de velocidad, en modo no interactivo |
| `gdu` | Rival de velocidad, en modo no interactivo |
| `plocate` | Rival de búsqueda |
| `fd` | Búsqueda sin índice, como referencia |

Las opciones de modo no interactivo cambian entre versiones: verificar con `--help` en la versión instalada y anotar el comando exacto en el informe.

**Condiciones de medición:**

- **Caché fría** (requiere root; lo ejecuta Ricardo):
  ```sh
  hyperfine --prepare 'sync; echo 3 | sudo tee /proc/sys/vm/drop_caches' --runs 10 '<comando>'
  ```
- **Caché caliente:**
  ```sh
  hyperfine --warmup 3 --runs 10 '<comando>'
  ```
- **Antes de medir:**
  - governor de CPU en `performance` (o `powerprofilesctl set performance`);
  - laptop conectada a la corriente;
  - sin otras cargas pesadas.
- **Reporte:** mediana y p95, exportados con `--export-json`.

**Métricas:**

| Métrica | Cómo se mide |
|---|---|
| Tiempo hasta 10.000 nodos | Callback de progreso con reloj monotónico |
| Tiempo total de escaneo | hyperfine |
| RAM máxima | `/usr/bin/time -v` → "Maximum resident set size" |
| Bytes por nodo | Suma de las columnas del arena / nodos |
| CPU de usuario y sistema | `/usr/bin/time -v` |
| Syscalls | `strace -c -f`, en una corrida aparte (strace distorsiona los tiempos) |
| Escritura y carga del snapshot | Cronómetro interno, más hyperfine sobre `gx snap load` |
| Búsqueda p50 y p95 | Set fijo de 200 consultas (`bench/queries.txt`), repetido 10 veces |
| Reconciliación | Tras modificar el 1 % y el 10 % del árbol con `gx synth mutate` |

**Tamaños de prueba:**

| Equipo | Nodos |
|---|---|
| Victus HP16 | 1 M, 5 M, 10 M |
| PC antiguo | 1 M; 5 M si el disco lo permite |

Para 10 millones de archivos sobre una imagen ext4 hacen falta suficientes inodes (`mkfs.ext4 -N 12000000`). Crear ese árbol en un HDD puede tardar horas.

## §V3. Criterios de aceptación por subfase

| Subfase | Criterio | Meta inicial |
|---|---|---|
| F0.0 | Todos los comandos de verificación de `CLAUDE.md` pasan, incluida la compilación cruzada a aarch64 | Obligatorio |
| F0.1 | §V1 completo en los 4 perfiles a 100 000 archivos | Exacto |
| F0.1 | El generador es determinista: dos ejecuciones con el mismo seed dan árboles idénticos (comparados por lista de rutas y tamaños) | Exacto |
| F0.2 | §V1 completo en los 4 perfiles a 1 M | Exacto |
| F0.2 | Caché caliente: tiempo total ≤ el mejor entre dua y gdu | Victus, `home-dev` 1 M |
| F0.2 | Caché fría: se reporta contra dua y gdu | Informativo |
| F0.2 | Bytes por nodo (columnas + nombres) | ≤ 70 |
| F0.2 | El A/B de `statx` está medido y decidido | Obligatorio |
| F0.2 | Un montaje de red colgado (simulado) no bloquea el resto del escaneo | Obligatorio |
| F0.3 | Carga del snapshot (validación O(1) + mmap) | < 20 ms con 10 M |
| F0.3 | Arena reconciliado en modo `full` idéntico a un escaneo limpio | Exacto |
| F0.3 | Modo `structure` con 1 % de cambios | ≤ 25 % del tiempo de un escaneo limpio con caché caliente |
| F0.3 | Un snapshot corrupto (fuzzing de bytes) se rechaza sin pánico ni UB | Obligatorio |
| F0.4 | Mismos resultados que una búsqueda lineal en las 200 consultas | Exacto |
| F0.4 | Búsqueda p95 con 10 M | < 20 ms en Victus |
| F0.4 | Tamaño del índice reportado | Informativo |
| F0.5 | El helper rechaza toda ruta que no sea un límite real de `EACCES` para el solicitante | Obligatorio |
| F0.5 | Totales de carpetas bloqueadas idénticos a `sudo du -x -s` | Exacto |
| F0.6 | Informe completo en ambos equipos con `templates/informe-benchmark.md` | Obligatorio |

## §V4. Pruebas de seguridad

- **Snapshot:** fuzzing con `cargo fuzz` sobre el cargador. Cada cambio aleatorio de bytes debe terminar en un error controlado.
- **Symlinks:** el perfil `patologico` incluye ciclos y symlinks a `/`. El escáner nunca debe salir del árbol de prueba (se verifica que todas las rutas indexadas empiezan con la raíz del escaneo).
- **Generador:** rechaza destinos existentes no vacíos y rutas fuera de su prefijo temporal sin `--allow-path`.
- **Helper:** pruebas con un usuario sin privilegios `gxtest`, creado por Ricardo, intentando consultar:
  - carpetas legibles (debe rechazar);
  - rutas dentro de carpetas bloqueadas (debe rechazar);
  - el límite exacto (debe responder).
