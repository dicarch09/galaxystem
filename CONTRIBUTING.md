# Cómo contribuir

## Licencia de las contribuciones

- Lo que aportes en `nucleo/` queda bajo **MPL-2.0**.
- Lo que aportes en `app/` queda bajo **GPL-3.0-or-later**.

No pedimos ceder derechos de autor (no hay CLA). Sí pedimos el **Developer Certificate of Origin (DCO)**: al firmar un commit declaras que tienes derecho a aportar ese código bajo la licencia del directorio donde va. El texto completo está en https://developercertificate.org/.

Para firmar, usa `git commit -s`. Eso agrega la línea:

```
Signed-off-by: Tu Nombre <tu@correo>
```

Los PR con commits sin firmar no se pueden fusionar.

## Antes de abrir un PR

1. Todos los comandos de verificación de `CLAUDE.md` pasan.
2. Si el cambio afecta el rendimiento, el PR incluye números antes y después (`docs/F0-validacion.md`).
3. Si el cambio toma una decisión de diseño nueva, se agrega a `docs/01-decisiones.md`.
