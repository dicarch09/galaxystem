# Galaxystem F0 — Informe de benchmarks

Versión del núcleo: <commit>. Fecha: <fecha>.

## Equipos

Copiar las tablas de `docs/hardware.md`. Indicar explícitamente que **no hay mediciones en ARM** (si sigue siendo así).

## Herramientas comparadas

| Herramienta | Versión | Comando exacto |
|---|---|---|

## Exactitud

| Perfil | Nodos | Total disco (Galaxystem / du) | Total aparente (Galaxystem / du) | Carpetas verificadas | ¿Coincide? |
|---|---|---|---|---|---|

## Escaneo

Una tabla por equipo y por estado de caché.

| Perfil | Nodos | Galaxystem | dua | gdu | du | Hasta 10.000 nodos |
|---|---|---|---|---|---|---|

Mediana y p95 en cada celda.

## Recursos

| Perfil | Nodos | RAM máxima | B/nodo | CPU usuario | CPU sistema | Syscalls |
|---|---|---|---|---|---|---|

## A/B de `statx`

| Variante | Caché | Tiempo | Syscalls | Decisión |
|---|---|---|---|---|

## Snapshot

| Nodos | Tamaño del archivo | Escritura | Carga (O(1) + mmap) | Validación completa |
|---|---|---|---|---|

## Reconciliación

| Cambios | Modo | Tiempo | vs escaneo limpio |
|---|---|---|---|

## Búsqueda

| Nodos | Tamaño del índice | Construcción | p50 | p95 | plocate p50 | fd |
|---|---|---|---|---|---|---|

## Conclusiones

- Qué metas se cumplieron y cuáles no.
- Qué se aprendió que cambia el diseño de la F1.
- Riesgos que siguen abiertos.

## Reproducir

Comandos exactos para regenerar cada tabla, con seeds y parámetros.
