# Visión de Galaxystem

## En una frase

Ves tu disco entero al instante y nunca más abres un archivo para saber qué es.

## El universo de archivos

- **Cada carpeta es un espacio**; navegar es hacer zoom infinito y continuo. Para evitar la pérdida de precisión de f32, cada nodo vive en coordenadas locales de su padre y la cámara se re-ancla al ancestro visible más profundo (*floating origin*).
- **Layout orgánico tipo constelación.** Los hijos de cada carpeta se ubican en una espiral filotáctica (ángulo áureo). El cálculo es O(n), determinista y se puede hacer en GPU.
- **Layout estable (ADR-004).** El ángulo de cada nodo depende de un hash estable de `(dispositivo, inode)`. El tamaño solo cambia el radio, con histéresis. Las cosas nunca se mueven solas: la memoria de lugar es la ventaja central del producto.
- **La jerarquía no lleva líneas (ADR-005).** Se entiende por contención: anillos tenues que se desvanecen con la profundidad.
- **Líneas que sí existen:**
  - capa opcional "mismo tipo",
  - archivo en uso → su ventana,
  - enlaces de montaje con partículas proporcionales al I/O.

## Escala logarítmica

- **Tamaño del nodo:** relativo entre hermanos. Dentro de cada carpeta, los hijos se reparten el espacio según log₂(1 + bytes).
- **Brillo del nodo:** absoluto y comparable en todo el árbol, por orden de magnitud (B, KB, MB, GB con halo, TB). Es la metáfora de la magnitud estelar.
- **Bytes por defecto:** uso real en disco (`st_blocks × 512`), con deduplicación de hard links igual que `du`. El tamaño aparente es una vista alternativa (ADR-003).
- **Carpetas gigantes:** de lejos se ven como nebulosas de densidad y se resuelven en puntos solo al acercarse.

## Contenido visible

- Previews por niveles: color por tipo → miniatura embebida o cacheada → decodificación a la resolución necesaria.
- Caché de previews en texturas comprimidas para GPU (BC7 en x86, ASTC en ARM).
- Decodificación en workers aislados con seccomp y landlock.
- Es solo visual: no se edita dentro del espacio.

## Permisos

- **Archivo visible sin permiso de lectura:** nodo oscuro con su tamaño.
- **Carpeta sin permiso de lectura:** un solo nodo oscuro con su tamaño total, **sin revelar nombres de su contenido**. Galaxystem nunca muestra más de lo que mostraría `ls` con tu usuario (ADR-010).

## Sistema completo

- `/` completo, con millones de archivos.
- **Cada sistema de archivos es una galaxia**; los puntos de montaje son portales con su actividad de I/O.
- **Red:** NFS, SMB y SFTP sin bloquear nunca el render; se marcan como desactualizados si no responden.
- **`/proc`** se muestra como procesos vivos: tamaño = log(RSS), brillo = CPU.
- **Snapshots de btrfs:** se muestran como "ecos" de la galaxia original.

## Galaxystem como compositor (F4)

- **Escritorios:** capas virtuales del compositor con apariencia de carpeta. Flotan en una capa elevada (2,5D), se mueven a mano, y es en esa capa donde se fijan.
- **Al cambiar de escritorio, el escritorio viene hacia ti.** No interrumpe a quien está explorando carpetas.
- **Las apps pertenecen a un escritorio.**
- **Tiling híbrido:** mosaico por defecto, flotante con un gesto o una tecla modificadora.
- **Abrir un archivo:** la ventana sale del marco del archivo. El marco queda en su lugar en estado "en uso", como un contorno punteado sin preview, conectado a la ventana por una línea.
- **Varios monitores:** una cámara independiente por monitor sobre el mismo universo. Arrastrar entre monitores es copiar o mover. Opción "unir cámaras" para monitores iguales.
- **Procesos separados:** indexador, compositor/render y workers de preview. Si un decodificador falla, la sesión sigue en pie.
- **Base:** Smithay. La integración del render se decide en la F0-R.

## Navegación

- Búsqueda con vuelo hasta el resultado.
- Barra de recientes en una esquina, alimentada por `~/.local/share/recently-used.xbel` y por las aperturas hechas desde Galaxystem.
- **No hay historial de "usados juntos"** (ADR-006).
- Barra de ruta editable, operación completa por teclado y modo tipo vim opcional.
- Lente de lista por carpeta, para comparar metadatos (ADR-017).
- Portal de selector de archivos del sistema: el universo aparece cuando cualquier app pide abrir o guardar (ADR-016).

## Objetivos técnicos

- Wayland primero.
- GPU integrada como hardware mínimo; x86_64 y aarch64.
- 0 % de GPU en reposo: se renderiza solo cuando algo cambia.
- **Más rápido que Nautilus en todo**, incluso al abrir una carpeta simple. El rendimiento es la propuesta de valor.

## Público inicial

1. Usuarios de Hyprland, Sway y niri.
2. Quienes manejan grandes volúmenes de datos: fotógrafos, datasets de ML, sysadmins.
3. Pensadores visuales.

## Proyecto

- Proyecto paralelo de Dicarch para probar límites de rendimiento.
- Open source: núcleo MPL-2.0 y app GPL-3.0-or-later, con DCO.
- Lo aprendido se reutilizará en el lienzo infinito de Dicarch.
