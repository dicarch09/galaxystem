# Galaxystem

Explorador de archivos espacial para Linux, de Dicarch. Cada carpeta es un espacio, cada archivo muestra su contenido y los tamaños siguen una escala logarítmica. A futuro, Galaxystem también será un compositor Wayland: el universo de archivos es el entorno y las aplicaciones se fijan sobre él.

**Estado actual: F0 — indexador.** Todavía no hay interfaz. La F0 construye el núcleo que escanea `/` completo con millones de archivos, lo guarda en un índice instantáneo de cargar y permite buscar por nombre.

## Licencias

| Ruta | Licencia | Qué contiene |
|---|---|---|
| `nucleo/` | MPL-2.0 | Indexador, arena, snapshot, búsqueda, generador sintético |
| `app/` | GPL-3.0-or-later | Interfaz y compositor (desde F1) |

Las contribuciones se firman con DCO (`Signed-off-by`). Ver `CONTRIBUTING.md`.

## Estructura

```
CLAUDE.md                 instrucciones para agentes de código
docs/
  00-vision.md            visión completa del producto
  01-decisiones.md        registro de decisiones (ADR)
  F0-diseno.md            diseño técnico detallado de la F0
  F0-validacion.md        metodología de pruebas y benchmarks
  hardware.md             equipos de medición
  pendientes.md           ideas fuera de alcance, para después
prompts/
  README.md               cómo usar los prompts, en orden
  F0.0 … F0.6             un prompt por subfase
templates/
  reporte-subfase.md      formato del reporte de cada checkpoint
  informe-benchmark.md    formato del informe final de la F0
nucleo/                   crates MPL-2.0 (se crean en F0.0)
```

## Roadmap

| Fase | Contenido |
|---|---|
| **F0** | Indexador: arena SoA, escáner paralelo, snapshot, reconciliación, búsqueda, helper de tamaños, benchmarks |
| **F0-R** | Prueba de render: wgpu vs Vulkan directo, pensando en la integración con Smithay |
| **F1** | App Wayland: constelación con layout estable, floating origin, LOD con nebulosas, brillo por magnitud, búsqueda con vuelo, barra de recientes |
| **F2** | Previews por niveles, caché de texturas comprimidas, workers aislados |
| **F3** | Cambios en vivo, operaciones completas con deshacer, lente de lista, selección con lazo |
| **F3.5** | Portal de selector de archivos del sistema |
| **F4** | Compositor: escritorios en capa elevada, apps fijadas, tiling híbrido, cámara por monitor |
| **F5** | Galaxias de montaje con actividad, SFTP y SMB, `/proc` como procesos, ecos de snapshots |
