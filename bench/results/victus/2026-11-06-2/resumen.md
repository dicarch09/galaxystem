# Escaneo de `/home/dawuu/gx-arboles/home-dev-1m`

## Entorno

| Dato | Valor |
|---|---|
| Equipo | victus |
| Fecha | 2026-11-06 |
| Commit | e2032b5 |
| Kernel | 7.1.2-2-cachyos |
| Governor | powersave |
| Preferencia de energía (EPP) | performance |
| Corriente | conectado |
| hyperfine | hyperfine 1.20.0 |
| Corridas | 10 medidas + 3 de calentamiento |

## Herramientas

| Herramienta | Versión | Comando exacto |
|---|---|---|
| gx-sync | gx 0.0.0 | `/home/dawuu/Documentos/Dicarch_projects/galaxystem/target/release/gx scan /home/dawuu/gx-arboles/home-dev-1m` |
| gx-uring | gx 0.0.0 | `/home/dawuu/Documentos/Dicarch_projects/galaxystem/target/release/gx scan /home/dawuu/gx-arboles/home-dev-1m --statx uring` |
| dua | dua 3.0.0 | `/home/dawuu/.cargo/bin/dua aggregate /home/dawuu/gx-arboles/home-dev-1m` |
| gdu | Version:	 5.38.0 | `/usr/bin/gdu --non-interactive --no-progress --summarize /home/dawuu/gx-arboles/home-dev-1m` |
| du | du (GNU coreutils) 9.12 | `/usr/bin/du -s -x /home/dawuu/gx-arboles/home-dev-1m` |

## Escaneo con caché caliente

p95 por rango más cercano: con 10 corridas es el máximo.

| Herramienta | Mediana | p95 | Media | CPU usuario (media) | CPU sistema (media) |
|---|---|---|---|---|---|
| gx-sync | 319.5 ms | 369.2 ms | 328.0 ms | 634.5 ms | 2339.4 ms |
| gx-uring | 433.4 ms | 467.5 ms | 437.5 ms | 694.9 ms | 3754.2 ms |
| dua | 689.4 ms | 704.1 ms | 691.5 ms | 1548.1 ms | 3258.5 ms |
| gdu | 596.0 ms | 611.6 ms | 536.9 ms | 1838.6 ms | 2657.5 ms |
| du | 1393.9 ms | 1502.4 ms | 1405.6 ms | 181.1 ms | 1220.4 ms |

## gx en proceso (10 corridas)

| Variante | Nodos | Hasta 10.000 nodos (mediana / p95) | Total (mediana / p95) | B/nodo total | B/nodo columnas + nombres | Hilos con io_uring |
|---|---|---|---|---|---|---|
| gx-sync | 1140500 | 5.4 ms / 5.9 ms | 319.3 ms / 345.4 ms | 60.6 | 59.3 | 0 |
| gx-uring | 1140500 | 6.4 ms / 8.8 ms | 445.0 ms / 516.4 ms | 60.6 | 59.3 | 24 |

## Recursos (una corrida, caché caliente)

| Herramienta | RAM máxima | CPU usuario | CPU sistema | Syscalls |
|---|---|---|---|---|
| gx-sync | 188.0 MiB | 0.67 s | 2.62 s | 1664155 |
| gx-uring | 284.7 MiB | 0.72 s | 3.76 s | 623365 |
| dua | 9.0 MiB | 1.54 s | 3.21 s | 3611696 |
| gdu | 75.5 MiB | 1.77 s | 2.62 s | 1574998 |
| du | 31.1 MiB | 0.15 s | 1.22 s | 2399601 |

Con io_uring, cada `statx` del lote no es una syscall: strace solo ve los `io_uring_enter`.

## Caché fría (ejecutar a mano, requiere sudo)

```sh
/home/dawuu/.cargo/bin/hyperfine --prepare 'sync; echo 3 | sudo tee /proc/sys/vm/drop_caches' --runs 10 --export-json bench/results/victus/2026-11-06-2/scan-fria.json \
    -n gx-sync '/home/dawuu/Documentos/Dicarch_projects/galaxystem/target/release/gx scan /home/dawuu/gx-arboles/home-dev-1m' \
    -n gx-uring '/home/dawuu/Documentos/Dicarch_projects/galaxystem/target/release/gx scan /home/dawuu/gx-arboles/home-dev-1m --statx uring' \
    -n dua '/home/dawuu/.cargo/bin/dua aggregate /home/dawuu/gx-arboles/home-dev-1m' \
    -n gdu '/usr/bin/gdu --non-interactive --no-progress --summarize /home/dawuu/gx-arboles/home-dev-1m' \
    -n du '/usr/bin/du -s -x /home/dawuu/gx-arboles/home-dev-1m'
```

Resumen del JSON: `/home/dawuu/Documentos/Dicarch_projects/galaxystem/target/release/gx bench report bench/results/victus/2026-11-06-2/scan-fria.json`.

Tiempo hasta 10.000 nodos con caché fría (repetir y anotar la mediana):

```sh
sync; echo 3 | sudo tee /proc/sys/vm/drop_caches; /home/dawuu/Documentos/Dicarch_projects/galaxystem/target/release/gx scan /home/dawuu/gx-arboles/home-dev-1m
```
