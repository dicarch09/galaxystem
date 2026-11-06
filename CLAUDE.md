# Instrucciones para agentes de código — Galaxystem

Este archivo se carga automáticamente en cada sesión. Léelo completo antes de actuar.

## Qué es este proyecto

Galaxystem es un explorador de archivos espacial para Linux (y después un compositor Wayland) de Dicarch. Estamos en la **F0: el indexador**, sin interfaz. La visión completa está en `docs/00-vision.md`; el diseño de la fase actual en `docs/F0-diseno.md`.

## Principios no negociables

1. **El rendimiento es un requisito y se mide, no se supone.** Ningún cambio que diga mejorar el rendimiento se acepta sin un benchmark antes y después, en las mismas condiciones.
2. **La exactitud va antes que la velocidad.** Los totales del indexador deben coincidir con `du`. Un escáner rápido pero incorrecto no sirve.
3. **Plan antes de código.** Cada prompt de subfase empieza con un plan breve y **esperas aprobación explícita** antes de implementar.
4. **Checkpoints.** Al terminar una subfase entregas el reporte de `templates/reporte-subfase.md` y **te detienes**. Nunca empiezas la subfase siguiente por iniciativa propia.
5. **Alcance estricto.** Implementa solo lo que pide el prompt. Las ideas que surjan van a `docs/pendientes.md`, no al código.
6. **Honestidad en los reportes.** Si algo no cumple el criterio, se reporta como no cumplido, con números. No se maquillan resultados ni se ajustan metas sin avisar.

## Seguridad

- **Nunca sigas symlinks** al escanear: `O_NOFOLLOW`, `AT_SYMLINK_NOFOLLOW` y, al abrir directorios por ruta, `openat2` con `RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS`.
- **El núcleo solo lee metadatos.** Ningún código de la F0 escribe, mueve ni borra archivos del usuario. Las únicas escrituras permitidas son el snapshot, el índice de búsqueda y los árboles del generador sintético.
- **El generador sintético y los tests destructivos solo operan dentro de directorios temporales creados por ellos mismos.** Rechaza cualquier ruta destino que ya exista y no esté vacía, y cualquier ruta fuera de un prefijo temporal salvo que se pase `--allow-path` explícito.
- **Los snapshots se tratan como entrada no confiable.** Ningún acceso por índice sin validación previa de rangos.
- **Nada se ejecuta con `sudo` desde el agente.** Los pasos que requieren root (limpiar caché, montar imágenes, el helper) se documentan como comandos para que Ricardo los ejecute.

## Código

- Rust edición 2024, MSRV 1.85.
- `unsafe` solo dentro de los módulos `sys` y `column` de cada crate. Cada bloque lleva un comentario `// SAFETY:` que explica por qué es correcto.
- `#![deny(unsafe_op_in_unsafe_fn)]` y `#![warn(missing_docs)]` en todos los crates del núcleo.
- Identificadores y comentarios de código en **inglés**. Documentación, reportes y mensajes al usuario en **español** (ADR-007).
- Cada archivo de `nucleo/` empieza con el aviso de MPL-2.0:
  ```rust
  // This Source Code Form is subject to the terms of the Mozilla Public
  // License, v. 2.0. If a copy of the MPL was not distributed with this
  // file, You can obtain one at https://mozilla.org/MPL/2.0/.
  ```
- Dependencias: solo las de `docs/F0-diseno.md` §9. Agregar otra exige justificarla en el reporte y pasar `cargo deny check`.

## Comandos de verificación

Antes de cada reporte, todos deben pasar:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
cargo build --workspace --target aarch64-unknown-linux-gnu
```

## Commits

- Conventional Commits: `feat(scan): …`, `fix(arena): …`, `bench: …`, `docs: …`.
- Todos firmados con `Signed-off-by` (DCO).
- Un commit por unidad lógica; nunca mezclar formato con cambios de comportamiento.
