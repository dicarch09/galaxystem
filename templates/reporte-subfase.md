# Reporte de subfase F0.X — <nombre>

## Resumen

Una a tres frases: qué quedó hecho y si se cumplen los criterios.

## Criterios de aceptación

| Criterio (de `F0-validacion.md` §V3) | Meta | Resultado | ¿Cumple? |
|---|---|---|---|
| | | | Sí / No / Parcial |

Si algo no cumple, explicar por qué y qué haría falta.

## Verificación

| Comando | Resultado |
|---|---|
| `cargo fmt --all --check` | |
| `cargo clippy --workspace --all-targets -- -D warnings` | |
| `cargo test --workspace` | N pruebas, N fallidas |
| `cargo deny check` | |
| `cargo build --workspace --target aarch64-unknown-linux-gnu` | |

## Mediciones

Condiciones: equipo, sistema de archivos, perfil, nodos, caché fría o caliente.

| Métrica | Valor | Comparación |
|---|---|---|

## Desviaciones del diseño

Todo lo que se implementó distinto a `docs/F0-diseno.md`, con el motivo. Si cambia una decisión, proponer la actualización del ADR.

## Dependencias nuevas

Crate, versión, licencia y motivo. "Ninguna" si no hay.

## Agregado a pendientes

Líneas nuevas en `docs/pendientes.md`.

## Pasos que requieren a Ricardo

Comandos con root o hardware que el agente no pudo ejecutar.

## Commits

Lista de commits de la subfase.
