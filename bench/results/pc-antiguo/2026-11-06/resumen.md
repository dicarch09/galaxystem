# Escaneo de `/home/kixxy/gx-arboles/home-dev-1m`

## Entorno

| Dato | Valor |
|---|---|
| Equipo | pc-antiguo |
| Fecha | 2026-11-06 |
| Commit | e636574 |
| Kernel | 7.0.11-arch1-1 |
| Governor | performance |
| Preferencia de energía (EPP) | performance |
| Corriente | conectado |
| hyperfine | hyperfine 1.20.0 |
| Corridas | 10 medidas + 3 de calentamiento |

## Herramientas

| Herramienta | Versión | Comando exacto |
|---|---|---|
| gx-sync | gx 0.0.0 | `/home/kixxy/Documentos/dicarch-projects/galaxystem/target/release/gx scan /home/kixxy/gx-arboles/home-dev-1m` |
| gx-uring | gx 0.0.0 | `/home/kixxy/Documentos/dicarch-projects/galaxystem/target/release/gx scan /home/kixxy/gx-arboles/home-dev-1m --statx uring` |
| dua | dua 3.0.0 | `/home/kixxy/.cargo/bin/dua aggregate /home/kixxy/gx-arboles/home-dev-1m` |
| gdu | Version:	 5.37.0 | `/usr/bin/gdu --non-interactive --no-progress --summarize /home/kixxy/gx-arboles/home-dev-1m` |
| du | du (GNU coreutils) 9.11 | `/usr/bin/du -s -x /home/kixxy/gx-arboles/home-dev-1m` |

## Escaneo con caché caliente

p95 por rango más cercano: con 10 corridas es el máximo.

| Herramienta | Mediana | p95 | Media | CPU usuario (media) | CPU sistema (media) |
|---|---|---|---|---|---|
| gx-sync | 3081.4 ms | 3222.1 ms | 3082.9 ms | 2136.8 ms | 3518.9 ms |
| gx-uring | 3900.7 ms | 4003.9 ms | 3900.0 ms | 1502.6 ms | 5589.1 ms |
| dua | 8510.3 ms | 8718.5 ms | 8527.2 ms | 6205.6 ms | 8274.2 ms |
| gdu | 4705.6 ms | 4907.5 ms | 4718.5 ms | 3741.3 ms | 4479.2 ms |
| du | 6377.8 ms | 6441.6 ms | 6387.8 ms | 2494.4 ms | 3730.3 ms |

## gx en proceso (10 corridas)

| Variante | Nodos | Hasta 10.000 nodos (mediana / p95) | Total (mediana / p95) | B/nodo total | B/nodo columnas + nombres | Hilos con io_uring |
|---|---|---|---|---|---|---|
| gx-sync | 1140500 | 24.1 ms / 52.9 ms | 3014.0 ms / 3081.3 ms | 60.6 | 59.3 | 0 |
| gx-uring | 1140500 | 31.7 ms / 40.9 ms | 3829.6 ms / 3956.1 ms | 60.6 | 59.3 | 2 |

## Recursos (una corrida, caché caliente)

| Herramienta | RAM máxima | CPU usuario | CPU sistema | Syscalls |
|---|---|---|---|---|
| gx-sync | 116.8 MiB | 2.14 s | 3.42 s | 1641822 |
| gx-uring | 210.1 MiB | 1.48 s | 5.51 s | 620064 |
| dua | 16.3 MiB | 6.10 s | 8.25 s | 4283093 |
| gdu | 63.2 MiB | 3.69 s | 4.49 s | 1650389 |
| du | 35.1 MiB | 2.45 s | 3.71 s | 2399730 |

Con io_uring, cada `statx` del lote no es una syscall: strace solo ve los `io_uring_enter`.

## Caché fría (ejecutar a mano, requiere sudo)

```sh
/home/kixxy/.cargo/bin/hyperfine --prepare 'sync; echo 3 | sudo tee /proc/sys/vm/drop_caches' --runs 3 --export-json bench/results/pc-antiguo/2026-11-06/scan-fria.json \
    -n gx-sync '/home/kixxy/Documentos/dicarch-projects/galaxystem/target/release/gx scan /home/kixxy/gx-arboles/home-dev-1m' \
    -n dua '/home/kixxy/.cargo/bin/dua aggregate /home/kixxy/gx-arboles/home-dev-1m' \
    -n gx-uring '/home/kixxy/Documentos/dicarch-projects/galaxystem/target/release/gx scan /home/kixxy/gx-arboles/home-dev-1m --statx uring' \
    -n gdu '/usr/bin/gdu --non-interactive --no-progress --summarize /home/kixxy/gx-arboles/home-dev-1m' \
    -n du '/usr/bin/du -s -x /home/kixxy/gx-arboles/home-dev-1m'
```

Resumen del JSON: `/home/kixxy/Documentos/dicarch-projects/galaxystem/target/release/gx bench report bench/results/pc-antiguo/2026-11-06/scan-fria.json`.

Tiempo hasta 10.000 nodos con caché fría (repetir y anotar la mediana):

```sh
sync; echo 3 | sudo tee /proc/sys/vm/drop_caches; /home/kixxy/Documentos/dicarch-projects/galaxystem/target/release/gx scan /home/kixxy/gx-arboles/home-dev-1m
```
