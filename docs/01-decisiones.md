# Registro de decisiones (ADR)

Cada decisión tiene un número fijo. Si se reemplaza, se marca "Reemplazada por ADR-xxx" y se agrega la nueva; nunca se borra.

| ADR | Decisión | Motivo | Estado |
|---|---|---|---|
| 001 | Rust nativo, sin tecnologías web | Un explorador de archivos no gana nada con Tauri o Electron y paga su costo | Aceptada |
| 002 | Núcleo MPL-2.0, app GPL-3.0-or-later, contribuciones con DCO y sin CLA | MPL permite reutilizar el núcleo en productos cerrados de Dicarch; GPL mantiene abiertos los forks de la app | Aceptada |
| 003 | Bytes por defecto: uso en disco (`st_blocks × 512`), hard links deduplicados como `du`. Tamaño aparente como alternativa | Responde "dónde se va mi espacio"; el aparente engaña con archivos dispersos y compresión | Aceptada |
| 004 | Layout estable: ángulo por hash de `(dev, ino)`; el tamaño solo afecta el radio, con histéresis | Protege la memoria espacial; se pierde "lo grande al centro" | Aceptada |
| 005 | La jerarquía no se dibuja con líneas, solo con contención | Decisión visual de Ricardo | Aceptada |
| 006 | Sin historial de "usados juntos"; recientes desde `recently-used.xbel` más las aperturas propias | Privacidad: no crear un historial de uso nuevo | Aceptada |
| 007 | Documentación en español; identificadores y comentarios de código en inglés | Comunidad open source internacional | Aceptada |
| 008 | Solo Wayland por ahora | Alcance | Aceptada |
| 009 | Procesos separados: indexador, compositor/render, workers de preview | Un fallo de decodificación no debe tumbar la sesión | Aceptada |
| 010 | El escaneo corre como el usuario. Un helper privilegiado responde **solo el tamaño total** de carpetas que el usuario no puede leer, y solo si el kernel confirma ese límite con las credenciales del usuario | El kernel aplica DAC, ACL y MAC por nosotros: la vista nunca muestra más que `ls`, sin simular permisos ni duplicar memoria. Reemplaza la "vista filtrada" | Aceptada |
| 011 | Búsqueda por trigramas sobre **bloques de 64 nombres**, no sobre nodos individuales | Un índice por nodo cuesta cientos de MB con 10 millones de archivos; técnica de plocate | Aceptada |
| 012 | Snapshot columnar, little-endian, alineado a 4 KB, cargado con `mmap` sin copias | El arranque cuesta lo mismo con 1 o 10 millones de nodos | Aceptada |
| 013 | Kernel mínimo 5.8 (`openat2`, `STATX_MNT_ID`) | Necesario para abrir sin seguir symlinks y detectar montajes de forma fiable | Aceptada |
| 014 | Escritorios: capa virtual del compositor con apariencia de carpeta, flotando en una capa elevada 2,5D, movibles; el escritorio viene al usuario | Decisión de producto | Aceptada |
| 015 | Varios monitores: una cámara por monitor, con opción de unir | Monitores con distinto DPI u orientación hacen incómoda una cámara única | Aceptada |
| 016 | Portal de selector de archivos en F3.5 | Mayor vía de adopción | Aceptada |
| 017 | Lente de lista por carpeta en F3 | Lo espacial es malo para comparar metadatos | Aceptada |
| 018 | Stack de render (wgpu vs Vulkan directo) | Depende de la importación de dmabuf para Smithay | Abierta: se decide en F0-R |
| 019 | Reconciliación de la F0 reconstruye un arena nuevo en lugar de mutar el existente | Simple y correcto; la mutación en vivo llega en F3 | Aceptada |
| 020 | Toda frontera de `st_dev` (montaje o subvolumen/snapshot btrfs anidado) se escanea y guarda su total, pero aporta 0 a su padre | GNU `du -x` excluye cada entrada con `st_dev` distinto al de la raíz, y en btrfs cada subvolumen tiene su propio `st_dev` aunque no esté montado aparte. Así se mantiene la exactitud contra `du -x`. Corrige la frase de `F0-diseno.md` §2.4 que decía que un `SNAPSHOT_ECHO` aporta al total | Aceptada (F0.1) |
| 021 | El tamaño aparente sigue a GNU `du --apparent-size`: solo archivos regulares y symlinks aportan su `st_size`; carpetas y archivos especiales aportan 0 propio | Es la única semántica verificable byte a byte contra una referencia externa (`du -b`). No cambia el rendimiento: el dato sale del mismo `statx` | Aceptada (F0.2) |
