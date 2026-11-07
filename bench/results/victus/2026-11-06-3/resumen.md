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
| gx-sync | 318.8 ms | 353.4 ms | 322.4 ms | 674.8 ms | 2560.4 ms |
| gx-uring | 420.4 ms | 464.0 ms | 427.7 ms | 720.1 ms | 3871.0 ms |
| dua | 687.7 ms | 702.3 ms | 688.8 ms | 1554.5 ms | 3197.0 ms |
| gdu | 439.1 ms | 450.3 ms | 438.3 ms | 1814.9 ms | 2615.2 ms |
| du | 1394.2 ms | 2245.2 ms | 1557.7 ms | 198.3 ms | 1346.7 ms |

## gx en proceso (10 corridas)

| Variante | Nodos | Hasta 10.000 nodos (mediana / p95) | Total (mediana / p95) | B/nodo total | B/nodo columnas + nombres | Hilos con io_uring |
|---|---|---|---|---|---|---|
| gx-sync | 1140500 | 4.6 ms / 5.1 ms | 299.6 ms / 310.9 ms | 60.6 | 59.3 | 0 |
| gx-uring | 1140500 | 5.9 ms / 6.2 ms | 420.6 ms / 458.6 ms | 60.6 | 59.3 | 24 |

## Recursos (una corrida, caché caliente)

| Herramienta | RAM máxima | CPU usuario | CPU sistema | Syscalls |
|---|---|---|---|---|
| gx-sync | 180.1 MiB | 0.72 s | 2.86 s | 1657617 |
| gx-uring | 309.4 MiB | 0.71 s | 3.78 s | 623202 |
| dua | 8.8 MiB | 1.53 s | 3.39 s | 3611586 |
| gdu | 50.9 MiB | 1.81 s | 2.68 s | 1585121 |
| du | 31.1 MiB | 0.19 s | 1.17 s | 2399601 |

Con io_uring, cada `statx` del lote no es una syscall: strace solo ve los `io_uring_enter`.

## Caché fría (ejecutar a mano, requiere sudo)

```sh
/home/dawuu/.cargo/bin/hyperfine --prepare 'sync; echo 3 | sudo tee /proc/sys/vm/drop_caches' --runs 10 --export-json bench/results/victus/2026-11-06-3/scan-fria.json \
    -n gx-sync '/home/dawuu/Documentos/Dicarch_projects/galaxystem/target/release/gx scan /home/dawuu/gx-arboles/home-dev-1m' \
    -n gx-uring '/home/dawuu/Documentos/Dicarch_projects/galaxystem/target/release/gx scan /home/dawuu/gx-arboles/home-dev-1m --statx uring' \
    -n dua '/home/dawuu/.cargo/bin/dua aggregate /home/dawuu/gx-arboles/home-dev-1m' \
    -n gdu '/usr/bin/gdu --non-interactive --no-progress --summarize /home/dawuu/gx-arboles/home-dev-1m' \
    -n du '/usr/bin/du -s -x /home/dawuu/gx-arboles/home-dev-1m'
```

Resumen del JSON: `/home/dawuu/Documentos/Dicarch_projects/galaxystem/target/release/gx bench report bench/results/victus/2026-11-06-3/scan-fria.json`.

Tiempo hasta 10.000 nodos con caché fría (repetir y anotar la mediana):

```sh
sync; echo 3 | sudo tee /proc/sys/vm/drop_caches; /home/dawuu/Documentos/Dicarch_projects/galaxystem/target/release/gx scan /home/dawuu/gx-arboles/home-dev-1m
```
