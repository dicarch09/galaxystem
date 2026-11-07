# Escaneo de `/home/dawuu/gx-arboles/home-dev-1m`

## Entorno

| Dato | Valor |
|---|---|
| Equipo | victus |
| Fecha | 2026-11-06 |
| Commit | e8909b3 |
| Kernel | 7.1.2-2-cachyos |
| Governor | powersave |
| Preferencia de energía (EPP) | performance |
| Corriente | conectado |
| hyperfine | hyperfine 1.20.0 |
| Corridas | 10 medidas + 3 de calentamiento |

**Avisos:**

- gdu no está instalado; se omite (`sudo pacman -S gdu`)
- GNU time (/usr/bin/time) no está instalado: no hay RAM ni CPU (`sudo pacman -S time`)
- strace no está instalado: no hay conteo de syscalls (`sudo pacman -S strace`)

## Herramientas

| Herramienta | Versión | Comando exacto |
|---|---|---|
| gx-sync | gx 0.0.0 | `/home/dawuu/Documentos/Dicarch_projects/galaxystem/target/release/gx scan /home/dawuu/gx-arboles/home-dev-1m` |
| gx-uring | gx 0.0.0 | `/home/dawuu/Documentos/Dicarch_projects/galaxystem/target/release/gx scan /home/dawuu/gx-arboles/home-dev-1m --statx uring` |
| dua | dua 3.0.0 | `/home/dawuu/.cargo/bin/dua aggregate /home/dawuu/gx-arboles/home-dev-1m` |
| du | du (GNU coreutils) 9.11 | `/usr/bin/du -s -x /home/dawuu/gx-arboles/home-dev-1m` |

## Escaneo con caché caliente

p95 por rango más cercano: con 10 corridas es el máximo.

| Herramienta | Mediana | p95 | Media | CPU usuario (media) | CPU sistema (media) |
|---|---|---|---|---|---|
| gx-sync | 339.3 ms | 372.6 ms | 342.4 ms | 660.0 ms | 2887.6 ms |
| gx-uring | 447.3 ms | 486.6 ms | 450.5 ms | 715.5 ms | 4028.5 ms |
| dua | 654.0 ms | 663.2 ms | 654.7 ms | 1541.7 ms | 3368.2 ms |
| du | 1460.0 ms | 1555.1 ms | 1477.7 ms | 182.0 ms | 1291.1 ms |

## gx en proceso (10 corridas)

| Variante | Nodos | Hasta 10.000 nodos (mediana / p95) | Total (mediana / p95) | B/nodo total | B/nodo columnas + nombres | Hilos con io_uring |
|---|---|---|---|---|---|---|
| gx-sync | 1140500 | 5.0 ms / 7.2 ms | 322.8 ms / 346.4 ms | 60.6 | 59.3 | 0 |
| gx-uring | 1140500 | 7.2 ms / 8.8 ms | 480.7 ms / 515.8 ms | 60.6 | 59.3 | 24 |

## Recursos (una corrida, caché caliente)

| Herramienta | RAM máxima | CPU usuario | CPU sistema | Syscalls |
|---|---|---|---|---|
| gx-sync | — | — | — | — |
| gx-uring | — | — | — | — |
| dua | — | — | — | — |
| du | — | — | — | — |

Con io_uring, cada `statx` del lote no es una syscall: strace solo ve los `io_uring_enter`.

## Caché fría (ejecutar a mano, requiere sudo)

```sh
/home/dawuu/.cargo/bin/hyperfine --prepare 'sync; echo 3 | sudo tee /proc/sys/vm/drop_caches' --runs 10 --export-json bench/results/victus/2026-11-06/scan-fria.json \
    -n gx-sync '/home/dawuu/Documentos/Dicarch_projects/galaxystem/target/release/gx scan /home/dawuu/gx-arboles/home-dev-1m' \
    -n gx-uring '/home/dawuu/Documentos/Dicarch_projects/galaxystem/target/release/gx scan /home/dawuu/gx-arboles/home-dev-1m --statx uring' \
    -n dua '/home/dawuu/.cargo/bin/dua aggregate /home/dawuu/gx-arboles/home-dev-1m' \
    -n du '/usr/bin/du -s -x /home/dawuu/gx-arboles/home-dev-1m'
```

Resumen del JSON: `/home/dawuu/Documentos/Dicarch_projects/galaxystem/target/release/gx bench report bench/results/victus/2026-11-06/scan-fria.json`.

Tiempo hasta 10.000 nodos con caché fría (repetir y anotar la mediana):

```sh
sync; echo 3 | sudo tee /proc/sys/vm/drop_caches; /home/dawuu/Documentos/Dicarch_projects/galaxystem/target/release/gx scan /home/dawuu/gx-arboles/home-dev-1m
```
