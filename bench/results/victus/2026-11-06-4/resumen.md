# Escaneo de `/home/dawuu/gx-arboles/home-dev-1m`

## Entorno

| Dato | Valor |
|---|---|
| Equipo | pc-antiguo |
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
| gx-sync | 315.7 ms | 380.3 ms | 321.8 ms | 673.6 ms | 2338.3 ms |
| gx-uring | 422.5 ms | 474.3 ms | 435.1 ms | 720.1 ms | 3739.0 ms |
| dua | 692.3 ms | 704.1 ms | 691.2 ms | 1567.9 ms | 3203.4 ms |
| gdu | 426.2 ms | 585.0 ms | 439.9 ms | 1781.5 ms | 2556.1 ms |
| du | 1311.2 ms | 1334.3 ms | 1313.4 ms | 172.2 ms | 1137.8 ms |

## gx en proceso (10 corridas)

| Variante | Nodos | Hasta 10.000 nodos (mediana / p95) | Total (mediana / p95) | B/nodo total | B/nodo columnas + nombres | Hilos con io_uring |
|---|---|---|---|---|---|---|
| gx-sync | 1140500 | 4.7 ms / 4.9 ms | 305.1 ms / 351.1 ms | 60.6 | 59.3 | 0 |
| gx-uring | 1140500 | 6.1 ms / 6.5 ms | 409.9 ms / 450.3 ms | 60.6 | 59.3 | 24 |

## Recursos (una corrida, caché caliente)

| Herramienta | RAM máxima | CPU usuario | CPU sistema | Syscalls |
|---|---|---|---|---|
| gx-sync | 183.1 MiB | 0.74 s | 2.60 s | 1653217 |
| gx-uring | 304.9 MiB | 0.74 s | 3.61 s | 623061 |
| dua | 8.6 MiB | 1.54 s | 3.17 s | 3608898 |
| gdu | 77.7 MiB | 1.78 s | 2.54 s | 1581032 |
| du | 32.8 MiB | 0.17 s | 1.13 s | 2399601 |

Con io_uring, cada `statx` del lote no es una syscall: strace solo ve los `io_uring_enter`.

## Caché fría (ejecutar a mano, requiere sudo)

```sh
/home/dawuu/.cargo/bin/hyperfine --prepare 'sync; echo 3 | sudo tee /proc/sys/vm/drop_caches' --runs 10 --export-json bench/results/pc-antiguo/2026-11-06/scan-fria.json \
    -n gx-sync '/home/dawuu/Documentos/Dicarch_projects/galaxystem/target/release/gx scan /home/dawuu/gx-arboles/home-dev-1m' \
    -n gx-uring '/home/dawuu/Documentos/Dicarch_projects/galaxystem/target/release/gx scan /home/dawuu/gx-arboles/home-dev-1m --statx uring' \
    -n dua '/home/dawuu/.cargo/bin/dua aggregate /home/dawuu/gx-arboles/home-dev-1m' \
    -n gdu '/usr/bin/gdu --non-interactive --no-progress --summarize /home/dawuu/gx-arboles/home-dev-1m' \
    -n du '/usr/bin/du -s -x /home/dawuu/gx-arboles/home-dev-1m'
```

Resumen del JSON: `/home/dawuu/Documentos/Dicarch_projects/galaxystem/target/release/gx bench report bench/results/pc-antiguo/2026-11-06/scan-fria.json`.

Tiempo hasta 10.000 nodos con caché fría (repetir y anotar la mediana):

```sh
sync; echo 3 | sudo tee /proc/sys/vm/drop_caches; /home/dawuu/Documentos/Dicarch_projects/galaxystem/target/release/gx scan /home/dawuu/gx-arboles/home-dev-1m
```
