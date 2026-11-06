# F0 — Diseño del indexador

Este documento es la referencia técnica de la F0. Los prompts de `prompts/` remiten a sus secciones por número (§).

## §0. Alcance

**La F0 entrega:**
- un escáner que indexa `/` completo con exactitud igual a `du`;
- un snapshot que se carga al instante;
- reconciliación al arrancar;
- búsqueda por nombre;
- un helper de tamaños para carpetas bloqueadas;
- benchmarks reproducibles.

**La F0 no incluye:**
- interfaz ni render;
- cambios en vivo (fanotify, F3);
- previews;
- clasificación por magic bytes;
- tamaño exclusivo de snapshots btrfs.

## §1. Modelo de datos: el arena

### §1.1 Invariantes

| ID | Invariante |
|---|---|
| I1 | La raíz es el nodo 0. `parent[0] = NONE` (`u32::MAX`) |
| I2 | Los hijos de cada carpeta ocupan un rango contiguo `[first_child, first_child + child_count)` |
| I3 | Para todo nodo `n ≠ 0`, `parent[n]` es un `DIR` y `n` está dentro de su rango de hijos |
| I4 | `disk_bytes[dir] = bloques propios del dir + Σ disk_bytes de hijos que aportan`. No aportan: `HARDLINK_DUP`, `MOUNT_POINT`, `BIND_ALIAS`. Igual para `apparent_bytes` |
| I5 | Un `HARDLINK_DUP` guarda sus tamaños reales (para mostrarlo), pero no aporta al padre |
| I6 | `1 ≤ name_len ≤ 255`. Los nombres no contienen `/` ni `\0`. Excepción: la raíz guarda la ruta absoluta escaneada |
| I7 | En el snapshot, `name_off` es monótono creciente con el id |

**Nota sobre montajes.** Un `MOUNT_POINT` guarda en sus campos el total de la galaxia montada, pero aporta 0 a su padre. Así los totales de cada sistema de archivos coinciden con `du -x`.

### §1.2 Columnas calientes (todos los nodos, 24 B)

| Columna | Tipo | Contenido |
|---|---|---|
| `parent` | u32 | Id del padre |
| `name_off` | u32 | Offset en el buffer de nombres |
| `name_len` | u8 | Largo del nombre en bytes |
| `class` | u8 | Ver §1.5 |
| `flags` | u16 | Ver §1.4 |
| `aux` | u32 | Carpeta: índice en la tabla de carpetas. Archivo: id de extensión |
| `disk_bytes` | u64 | Archivo: `st_blocks × 512`. Carpeta: total según I4 |

### §1.3 Columnas frías (todos los nodos, 26 B)

| Columna | Tipo | Contenido |
|---|---|---|
| `ino` | u64 | `stx_ino` |
| `apparent_bytes` | u64 | Archivo: `stx_size`. Carpeta: total según I4 |
| `mtime` | u32 | Segundos Unix, saturado a `[0, u32::MAX]` |
| `dev` | u16 | Índice en la tabla de dispositivos (§1.6) |
| `mode` | u16 | `stx_mode` |
| `owner` | u16 | Índice en la tabla de dueños. `0xFFFF` significa desborde y se resuelve en un mapa lateral |

### §1.4 Flags (`u16`)

| Bit | Flag | Significado |
|---|---|---|
| 0 | `DIR` | Directorio |
| 1 | `SYMLINK` | Enlace simbólico (nunca se sigue) |
| 2 | `SPECIAL` | Dispositivo, socket o fifo |
| 3 | `MOUNT_POINT` | Raíz de otro sistema de archivos |
| 4 | `PSEUDO_FS` | proc, sysfs y similares; no se escanea su contenido |
| 5 | `NET_FS` | NFS, CIFS, sshfs u otro FUSE de red |
| 6 | `LOCKED` | El usuario no puede leerlo (`EACCES` al abrir) |
| 7 | `HARDLINK_DUP` | Aparición repetida de un `(dev, ino)` con `nlink > 1` |
| 8 | `SPARSE` | `st_blocks × 512 < st_size` |
| 9 | `SUBVOL` | Raíz de subvolumen btrfs (inode 256) |
| 10 | `SNAPSHOT_ECHO` | Subvolumen con `parent_uuid` (snapshot) |
| 11 | `NON_UTF8` | El nombre no es UTF-8 válido |
| 12 | `STALE` | Dato posiblemente desactualizado |
| 13 | `BIND_ALIAS` | Bind mount de algo ya indexado; no se desciende |
| 14 | `ERROR` | Falló el `statx` o la apertura por un motivo distinto de `EACCES` |
| 15 | — | Reservado |

### §1.5 Clases (`u8`)

La clasificación se hace por extensión, sin distinguir mayúsculas. Las extensiones se internan en una tabla y su id va en `aux`.

| Valor | Clase | Extensiones (no exhaustivo) |
|---|---|---|
| 0 | Otro | — |
| 1 | Código | rs c h cpp hpp py js ts tsx jsx go java kt rb php sh lua zig toml yaml yml json html css scss sql |
| 2 | Imagen | jpg jpeg png gif webp avif heic bmp tiff svg ico dng cr2 nef arw psd xcf kra |
| 3 | Video | mp4 mkv webm mov avi m4v m2ts |
| 4 | Audio | mp3 flac ogg opus wav m4a aac |
| 5 | Documento | pdf odt ods odp doc docx xls xlsx ppt pptx txt md epub rtf csv tex |
| 6 | Comprimido | zip tar gz tgz xz zst bz2 7z rar |
| 7 | Binario | so a o exe dll bin deb rpm appimage iso img qcow2 vdi vmdk; también archivos sin extensión con bit de ejecución |

**`.ts` es TypeScript**, no MPEG-TS: el video usa `.m2ts`.

### §1.6 Tablas auxiliares

**Carpetas** (solo directorios, 12 B): `node: u32`, `first_child: u32`, `child_count: u32`.

**Resúmenes** (solo carpetas con 1.024 hijos o más, 96 B): `count: u32` y `bytes: u64` para cada una de las 8 clases. Cuentan hijos directos y sirven para las nebulosas de la F1.

**Dispositivos** (una fila por sistema de archivos):

| Campo | Tipo | Contenido |
|---|---|---|
| `dev` | u64 | `makedev(stx_dev_major, stx_dev_minor)` |
| `mnt_id` | u64 | `stx_mnt_id` |
| `fs_magic` | u64 | `statfs.f_type` |
| `rotational` | u8 | Leído de `/sys/block/*/queue/rotational`; 2 = desconocido |
| `mount_node` | u32 | Nodo raíz de la galaxia |
| `generation` | u64 | Generación btrfs (0 si no aplica) |
| `mount_path` | offset + largo | En un buffer de strings aparte |

**Dueños:** pares `(uid, gid)` únicos.

### §1.7 Presupuesto de memoria (metas iniciales)

| Concepto | B/nodo |
|---|---|
| Columnas calientes | 24 |
| Columnas frías | 26 |
| Nombres (promedio esperado) | ~18 |
| Tabla de carpetas (~10 % de nodos) | ~1,2 |
| **Total en disco** | **~69** |
| **Residente típico** (calientes + nombres) | **~42** |

Las columnas frías se cargan con `mmap` desde el snapshot y el kernel las trae a RAM solo al consultarlas.

### §1.8 Almacenamiento y asignación

- **`Column<T>`**: memoria virtual reservada con `mmap(MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE)` para una capacidad máxima configurable (por defecto 2³² elementos), con un largo atómico.
- **Reserva de bloques:** `reserve(n) -> Range<u32>` usa `fetch_add` sobre el largo. Sin locks ni realocaciones: las direcciones nunca cambian.
- **Buffer de nombres:** misma técnica. Al reservar ids para los hijos de una carpeta, se reservan también sus bytes de nombre en un solo bloque.
- **Uso desde la F0.1:** aunque la F0.1 es de un solo hilo, ya usa `Column<T>`. Así la F0.2 no tiene que reescribir el almacenamiento.

## §2. Escáner

### §2.1 Recorrido básico (F0.1, un hilo)

Por cada carpeta:

1. **Abrir.** Se abre con `openat2(AT_FDCWD, ruta, O_RDONLY | O_DIRECTORY | O_CLOEXEC, RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS)`. La ruta se reconstruye desde el arena siguiendo `parent`, porque mantener abiertos los descriptores de todo el árbol agotaría `RLIMIT_NOFILE`. Si falla con `EACCES`, la carpeta se marca `LOCKED`, aporta solo sus bloques propios y no se desciende.
2. **Leer entradas.** `getdents64` con un buffer de 256 KB (`rustix::fs::RawDir`), ignorando `.` y `..`.
3. **Reservar** `n` ids y `Σ len` bytes de nombre.
4. **Obtener metadatos** de cada entrada con `statx(dirfd, nombre, AT_SYMLINK_NOFOLLOW, STATX_TYPE | STATX_MODE | STATX_NLINK | STATX_UID | STATX_GID | STATX_MTIME | STATX_INO | STATX_SIZE | STATX_BLOCKS | STATX_MNT_ID)`. En `NET_FS` se agrega `AT_STATX_DONT_SYNC`.
5. **Clasificar** y escribir las columnas.
6. **Encolar** cada subcarpeta.

**Requisitos del sistema:**
- Kernel 5.8 o superior (ADR-013). Si `STATX_MNT_ID` no viene en `stx_mask`, se compara `stx_dev`.
- Al iniciar, se sube `RLIMIT_NOFILE` al máximo permitido.

**Agregación en la F0.1:** un pase post-orden al final del escaneo.

### §2.2 Escáner paralelo (F0.2)

- **Pools por dispositivo.** Cada sistema de archivos tiene su propio pool de hilos con work-stealing (`crossbeam-deque`). Una tarea es un `NodeId`.
- **Hilos según el disco:**

  | Tipo de disco | Hilos |
  |---|---|
  | NVMe o SSD (`rotational = 0`) | `min(núcleos × 2, 32)` |
  | HDD (`rotational = 1`) | 2 |
  | Desconocido | `núcleos` |

  Todos se pueden configurar.
- **Red.** Hilos dedicados, con timeout por directorio (por defecto 5 s). Si vence, la carpeta se marca `STALE` y el escaneo sigue sin esperar. Un hilo bloqueado en el kernel (estado D) se abandona y no se reutiliza.
- **A/B de `statx`** detrás de la feature `uring`, con el crate `io-uring`:
  - **A:** `statx` síncrono en cada hilo;
  - **B:** `statx` en lotes por io_uring (`IORING_OP_STATX`).
  
  `getdents64` sigue siendo síncrono en ambos casos, porque no está disponible en io_uring del kernel principal. Gana la variante con mejores números en el benchmark de §F0.2.

### §2.3 Agregación concurrente (F0.2)

- Columnas transitorias (no van al snapshot): `pending: AtomicU32` por carpeta (hijos directorio sin terminar + 1 por sus propias entradas) y acumuladores `AtomicU64` de disco y aparente.
- **Al terminar las entradas de una carpeta:** se suman los archivos que aportan al acumulador y se decrementa `pending`.
- **Cuando `pending` llega a 0:** la carpeta escribe su total, lo suma al acumulador del padre y decrementa el `pending` del padre. La propagación sigue hacia arriba.
- **Los totales parciales son visibles** mientras tanto. Eso es lo que permite el escaneo progresivo.

### §2.4 Fronteras

| Detección | Acción |
|---|---|
| `stx_mnt_id` distinto al del padre | `MOUNT_POINT`. Se registra en la tabla de dispositivos y se escanea como galaxia en su propio pool |
| `statfs.f_type` de proc, sysfs, cgroup, cgroup2, debugfs, tracefs, securityfs, bpf, pstore, configfs, devpts, mqueue, hugetlbfs, fusectl | `PSEUDO_FS`: no se escanea su contenido |
| tmpfs, devtmpfs | Se escanean normalmente |
| NFS, CIFS/SMB2, FUSE de red (sshfs y similares, según `/proc/self/mountinfo`) | `NET_FS`, con las reglas de §2.2 |
| Montaje cuya raíz ya está indexada (bind mount, por `/proc/self/mountinfo`) | `BIND_ALIAS`: no se desciende |
| btrfs con `stx_ino == 256` | `SUBVOL`. `BTRFS_IOC_GET_SUBVOL_INFO` (sin privilegios desde el kernel 4.18): si `parent_uuid` no es cero, `SNAPSHOT_ECHO` |

En la F0 un `SNAPSHOT_ECHO` **sí** se escanea y aporta al total, igual que `du -x`. Su tratamiento como eco es visual (F1) y su tamaño exclusivo queda para la F5.

### §2.5 Hard links

- Solo para archivos no directorio con `nlink > 1`.
- Se inserta `(dev, ino)` en un conjunto concurrente particionado (64 shards con `Mutex<HashSet>`).
- La primera inserción aporta normalmente; las siguientes se marcan `HARDLINK_DUP`.
- **Qué aparición es la "primera" depende del orden de recorrido**, igual que en `du`. El total del sistema de archivos coincide siempre, pero la atribución por carpeta puede diferir de la de `du` (ver `F0-validacion.md` §V1).

### §2.6 Errores

| Caso | Tratamiento |
|---|---|
| `EACCES` al abrir carpeta | `LOCKED` |
| `ENOENT` (borrado durante el escaneo) | La entrada se omite y se cuenta en las estadísticas |
| `ELOOP` (symlink en la ruta) | `ERROR`, sin descender |
| Cualquier otro error | `ERROR`, y se registra en el log de escaneo con su `errno` |

El escaneo nunca aborta por errores de entradas individuales.

### §2.7 Progreso

- El escáner recibe un `&dyn Progress` con `on_batch(nodes_total, bytes_total)`, llamado como máximo cada 50 ms, y `on_done(stats)`.
- **Métrica clave:** el tiempo hasta los primeros 10.000 nodos, que es lo que verá la interfaz.

## §3. Snapshot

### §3.1 Formato `index-v1.gxs`

**Cabecera de 4096 bytes**, `#[repr(C)]`, little-endian:

| Campo | Tipo |
|---|---|
| `magic` | `[u8; 8] = b"GXSNAP\0\x01"` |
| `version` | u32 = 1 |
| `header_flags` | u32 |
| `node_count`, `dir_count`, `summary_count` | u64 |
| `name_bytes`, `device_count`, `owner_count` | u64 |
| `created_unix_ns` | u64 |
| `scan_started_unix_ns` | u64 |
| `columns` | `[ColumnDesc; 24]` |
| relleno hasta 4096 | — |

`ColumnDesc = { id: u32, elem_size: u32, offset: u64, len_bytes: u64, xxh3: u64 }`

**Después de la cabecera**, cada columna va cruda y alineada a 4096 bytes, en el orden de §1.2, §1.3 y §1.6. El buffer de nombres se reescribe en orden de id (I7).

### §3.2 Escritura

1. Archivo temporal en el mismo directorio.
2. Escritura de las columnas.
3. `fsync` del archivo.
4. `rename` atómico.
5. `fsync` del directorio.

El snapshot se escribe con permisos 0600 en `$XDG_CACHE_HOME/galaxystem/`, o en `~/.cache/galaxystem/` si la variable no existe.

### §3.3 Carga y validación

1. **Validación síncrona en O(1):**
   - `magic` y `version` correctos;
   - endianness del host little-endian (si no, se rechaza);
   - para cada columna, `offset + len_bytes ≤ tamaño del archivo`, `len_bytes = count × elem_size` y alineación a 4096.
2. **`mmap(PROT_READ, MAP_PRIVATE)`.** El arena queda disponible en **modo verificado**: todos los accesores usan acceso con chequeo de rango.
3. **Validación estructural en O(n), en segundo plano:**
   - checksums xxh3;
   - invariantes I1, I2, I3 e I7;
   - rangos de `name_off + name_len`, `aux` y `dev`.
4. **Al terminar sin errores**, el arena pasa a **modo confiado**, con accesores sin chequeo. Si falla, el snapshot se descarta y se escanea de nuevo.

**Meta:** el paso 1 más el paso 2 en menos de 20 ms, sin importar el tamaño.

## §4. Reconciliación al arrancar (F0.3)

**Limitación de fondo:** el `mtime` de un directorio cambia cuando se agregan, borran o renombran entradas, **no** cuando un archivo de adentro cambia de tamaño.

La reconciliación **construye un arena nuevo** (ADR-019) recorriendo el viejo en orden. Hay dos modos:

| Modo | Qué hace | Exactitud |
|---|---|---|
| `structure` | Hace `statx` de cada carpeta. Si su `mtime` coincide con el guardado y **no es "dudoso"**, copia sus entradas del arena viejo sin `getdents`. Los archivos copiados quedan `STALE` | Estructura exacta; tamaños pueden estar desactualizados |
| `full` | Igual que `structure`, pero hace `statx` de todos los archivos | Igual a un escaneo limpio |

**Regla del mtime dudoso** (equivalente al "racy git"): si el `mtime` guardado es mayor o igual a `scan_started − 1 s`, la carpeta se trata como cambiada. Así no se pierden cambios hechos en el mismo segundo del escaneo anterior.

**Flujo previsto para la F1:**
1. cargar el snapshot y mostrarlo como `STALE`;
2. ejecutar `structure`;
3. ejecutar `full` en segundo plano con prioridad de I/O idle (`ioprio_set` con `IOPRIO_CLASS_IDLE`).

## §5. Búsqueda (F0.4)

- **Bloques:** el bloque `b` contiene los nodos `[64b, 64b + 64)`. Como los hijos son contiguos, los bloques caen mayormente dentro de una misma carpeta.
- **Normalización:** si el nombre es UTF-8 válido, minúsculas Unicode simples (`char::to_lowercase`). Si no, minúsculas solo en ASCII. La consulta se normaliza igual.
- **Índice:** por cada trigrama de bytes normalizados, un bitmap `roaring` de bloques. Se guarda en `search-v1.gxi`:
  - cabecera;
  - tabla ordenada de `(trigrama: u32, offset: u64, len: u32)`;
  - bitmaps serializados.
  
  Se carga con `mmap`.
- **Consulta de 3 bytes o más:**
  1. intersectar los bitmaps de sus trigramas, de menor a mayor;
  2. obtener los bloques candidatos;
  3. verificar cada nombre con `memchr::memmem`;
  4. ordenar los resultados.
- **Consulta de 1 o 2 bytes:** recorrido lineal de todos los nombres con `memchr::memmem`.
- **Orden de resultados en la F0:**
  1. coincidencia exacta del nombre;
  2. prefijo;
  3. subcadena.
  
  Dentro de cada grupo, por `mtime` descendente y luego por menor profundidad. Los recientes de `recently-used.xbel` se suman en la F1.
- **API:** `search(query: &[u8], limit: usize) -> Vec<NodeId>`. La ruta completa se reconstruye con `Arena::path`.

## §6. Generador sintético (F0.1)

- **Crate** `gx-synth` con su binario de CLI. Es determinista: mismo seed y mismos parámetros dan el mismo árbol.
- **Destino:** solo un directorio vacío creado por el propio generador, o uno indicado con `--allow-path`.
- **Tamaño sin escribir datos:**
  - por defecto `set_len` (archivo disperso: sirve para tamaño aparente);
  - con `--allocate`, `fallocate` (sirve para uso en disco).

**Parámetros:**

| Parámetro | Por defecto |
|---|---|
| `--files` | 100 000 |
| `--seed` | 1 |
| Profundidad máxima | 12 |
| Ramificación | Pareto con α = 1,2 |
| Carpetas gigantes | Cantidad y tamaño configurables |
| Tamaños | Log-normal, μ = 9, σ = 3 (en bytes, con base e) |
| Hard links | 0,5 % |
| Archivos dispersos | 1 % |
| Nombres no UTF-8 | 0,1 % |
| Symlinks | 1 %, incluyendo ciclos y symlinks a carpetas |
| Archivos especiales | fifos 0,05 % |

**Perfiles:**

| Perfil | Rasgos |
|---|---|
| `home-dev` | Muchos `node_modules` profundos, archivos de código pequeños |
| `fotografo` | Carpetas planas con miles de imágenes de 5 a 50 MB |
| `servidor` | Logs grandes, cachés binarias, hard links frecuentes |
| `patologico` | Una carpeta de 500 000 archivos, profundidad 200, nombres no UTF-8 y ciclos de symlinks |

**Opcional, requiere root** (Ricardo lo ejecuta, el agente lo documenta): crear la imagen dentro de un loop device ext4 o btrfs, para medir sobre un sistema de archivos dedicado y con snapshots btrfs.

## §7. Helper de tamaños (F0.5)

Implementa ADR-010.

- **Daemon** `galaxystem-sized`, que corre como root bajo systemd. Escucha en `/run/galaxystem/sized.sock`.
- **Una sola petición:** `size_of(path) -> { disk_bytes, apparent_bytes, file_count_approx }`.
- **Antes de responder, el helper verifica el límite con las credenciales reales del solicitante:**
  1. obtiene uid, gid y pid del cliente con `SO_PEERCRED`, y sus grupos suplementarios desde `/proc/<pid>/status`;
  2. en un hilo dedicado, cambia a esas credenciales con `setfsuid`, `setfsgid` y `setgroups` (por hilo);
  3. comprueba que el padre de `path` **se puede abrir y recorrer** y que `path` **devuelve `EACCES`** al abrirse;
  4. si no se cumplen ambas condiciones, rechaza la petición.

  Con esto, el helper **no puede usarse como oráculo** para descubrir nombres dentro de carpetas bloqueadas: solo responde por el límite exacto que el usuario ya ve.
- **Trampa de capacidades.** El kernel retira `CAP_DAC_OVERRIDE` y `CAP_DAC_READ_SEARCH` del conjunto efectivo **solo cuando el fsuid pasa de 0 a otro valor**. Si el helper corriera con un uid distinto de 0 y esas capacidades, `setfsuid` no las retiraría y la verificación quedaría anulada en silencio. Por eso el hilo de verificación, además de `setfsuid`, **retira explícitamente** esas capacidades de su conjunto efectivo, y una prueba confirma que la verificación falla si no se retiran.
- **Cálculo:** el helper escanea el subárbol con el mismo escáner de §2, sin guardar nombres, y cachea los totales por `(dev, ino)`.
- **Limitación conocida:** el contexto de seguridad MAC (SELinux, AppArmor) es del proceso y no cambia con `setfsuid`. El helper puede ver como abiertos límites que MAC le bloquearía al usuario, pero en el peor caso responde un tamaño, nunca nombres.
- **Opcional, si queda tiempo:** diferencias de generación btrfs para la reconciliación, solo con inodes y sin nombres.

## §8. API pública del núcleo (borrador)

```rust
pub struct NodeId(pub u32);

pub struct Arena { /* columnas */ }
impl Arena {
    pub fn len(&self) -> usize;
    pub fn root(&self) -> NodeId;
    pub fn parent(&self, n: NodeId) -> Option<NodeId>;
    pub fn children(&self, n: NodeId) -> core::ops::Range<u32>;
    pub fn name(&self, n: NodeId) -> &[u8];
    pub fn flags(&self, n: NodeId) -> Flags;
    pub fn class(&self, n: NodeId) -> Class;
    pub fn disk_bytes(&self, n: NodeId) -> u64;
    pub fn apparent_bytes(&self, n: NodeId) -> u64;
    pub fn path(&self, n: NodeId, out: &mut Vec<u8>);
}

pub struct ScanOptions { /* hilos, timeouts, feature uring, … */ }
pub trait Progress: Sync {
    fn on_batch(&self, nodes: u64, disk_bytes: u64);
    fn on_done(&self, stats: &ScanStats);
}
pub fn scan(root: &std::path::Path, opts: &ScanOptions, progress: &dyn Progress)
    -> Result<Arena, ScanError>;
```

**Crates:**

| Crate | Contenido |
|---|---|
| `gx-arena` | Tipos y `Column<T>` |
| `gx-scan` | Escáner |
| `gx-snap` | Snapshot y reconciliación |
| `gx-search` | Búsqueda |
| `gx-synth` | Generador sintético |
| `gx-bench` | Harness de benchmarks |
| `gx-cli` | Binario `gx` con subcomandos `scan`, `verify-du`, `snap`, `search`, `synth` y `bench` |
| `gx-sized` | Helper de la F0.5 |

## §9. Dependencias permitidas

Todas con licencias compatibles con MPL-2.0 (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode-3.0):

| Uso | Crate |
|---|---|
| Syscalls | `rustix` (preferido), `libc` (solo para ioctls que `rustix` no cubra) |
| io_uring | `io-uring` (feature `uring`) |
| Concurrencia | `crossbeam-deque`, `crossbeam-utils` |
| Checksums | `xxhash-rust` (feature `xxh3`) |
| Búsqueda | `roaring`, `memchr` |
| CLI | `clap` |
| Errores | `thiserror` (bibliotecas), `anyhow` (binarios) |
| Aleatoriedad del generador | `rand`, `rand_xoshiro` |
| Solo para pruebas | `proptest`, `tempfile`, `criterion` |

## §10. Riesgos abiertos

| Riesgo | Cuándo se resuelve |
|---|---|
| Que io_uring no gane frente a `statx` síncrono con caché caliente | A/B en F0.2 |
| El costo real de reconstruir rutas en árboles muy profundos | Perfil `patologico` en F0.2 |
| La memoria real por nodo frente a la meta de §1.7 | F0.2 |
| Reconciliación en modo `full`: sin atajo en ext4 | Se mide en F0.3; el atajo de btrfs depende del helper |
| No hay equipo ARM para medir rendimiento | Solo compilación cruzada y pruebas con qemu-user por ahora (ver `hardware.md`) |
