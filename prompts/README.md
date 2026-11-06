# Prompts de la F0 — cómo usarlos

Cada archivo es un prompt para un agente de código (por ejemplo Claude Code) que se ejecuta en la raíz del repositorio. `CLAUDE.md` se carga solo y fija las reglas comunes.

## Orden

| # | Prompt | Requiere | Dónde se valida |
|---|---|---|---|
| 1 | `F0.0-bootstrap.md` | — | Cualquier equipo |
| 2 | `F0.1-arena-generador-escaner.md` | F0.0 aprobada | Cualquier equipo |
| 3 | `F0.2-escaner-paralelo.md` | F0.1 aprobada | Victus y PC antiguo |
| 4 | `F0.3-snapshot-reconciliacion.md` | F0.2 aprobada | Victus |
| 5 | `F0.4-busqueda.md` | F0.3 aprobada | Victus |
| 6 | `F0.5-helper-tamanos.md` | F0.4 aprobada | Equipo con root y usuario de prueba |
| 7 | `F0.6-informe.md` | F0.5 aprobada | Ambos equipos |

## Cómo ejecutar cada prompt

1. Abre una **sesión nueva** del agente en la raíz del repo. Una sesión por subfase mantiene el contexto limpio.
2. Pega el contenido completo del prompt.
3. El agente responde con un **plan breve**. Revísalo y responde "aprobado" o corrige.
4. El agente implementa, valida y entrega el reporte con `templates/reporte-subfase.md`, y se detiene.
5. Ejecuta tú los **pasos que requieren root** que aparecen en el reporte.
6. Revisa el checkpoint con la lista de abajo y decide.

## Lista de revisión de cada checkpoint

- [ ] Todos los criterios de §V3 de la subfase dicen "Sí", o hay una explicación aceptable.
- [ ] Los comandos de verificación pasan; córrelos tú también.
- [ ] Las mediciones indican sus condiciones (equipo, caché, perfil, nodos).
- [ ] Las desviaciones del diseño están justificadas y, si corresponde, hay ADR nuevo.
- [ ] No entraron dependencias fuera de §9 sin justificación.
- [ ] El código no hace nada fuera del alcance del prompt.
- [ ] Hay commits firmados con DCO.

## Si un criterio falla

- **Exactitud:** no se avanza. Se abre una sesión de corrección pegando el reporte y la diferencia encontrada.
- **Rendimiento:** se decide caso a caso. Puede seguirse si la causa está entendida y anotada en `pendientes.md`, o puede ajustarse la meta por escrito en el ADR que corresponda.

## Archivos que actualizar tú al cerrar cada subfase

- `docs/hardware.md` (antes de la F0.2).
- `docs/01-decisiones.md` si cambió alguna decisión.
- El estado de la fase en `README.md`.
