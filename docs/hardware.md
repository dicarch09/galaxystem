# Equipos de medición

Completar una vez por equipo, antes de la F0.2. Repetir si cambia el kernel, el disco o el sistema de archivos.

## Comandos de inventario

```sh
uname -r                                   # kernel (mínimo 5.8)
lscpu | grep -E 'Model name|^CPU\(s\)|Thread|MHz'
free -h
lsblk -d -o NAME,ROTA,MODEL,SIZE,TRAN      # ROTA=1 es HDD
findmnt -no SOURCE,FSTYPE,OPTIONS /
findmnt -no SOURCE,FSTYPE,OPTIONS /home
cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor
```

## Equipo A — HP Victus 16

| Dato | Valor |
|---|---|
| CPU | 800Mhz a 4500Mhz |
| Núcleos / hilos | 12 |
| RAM | 15 GiB |
| Disco del sistema (modelo, NVMe/SSD) | SAMSUNG MZVL2512HCJQ-00BH1 |
| Sistema de archivos de `/` y `/home` | Btrfs |
| Distribución y kernel | CachyOS, 7.1.2-2-cachyos |
| Perfil de energía al medir | powersave |

Rol: equipo principal de rendimiento. Corre las mediciones de 1 M, 5 M y 10 M nodos.

Al medir, conectarlo a la corriente y dejarlo enfriar entre series largas. Si aparece throttling térmico (frecuencias que caen durante la corrida), los números no son comparables.

## Equipo B — Laptop antigua

| Dato | Valor |
|---|---|
| CPU | Intel Celeron CPU 3955U @ 2.00GHz 400Mhz a 2000Mhz |
| Núcleos / hilos | 2 |
| RAM | 3,7GiB |
| Disco(s) (modelo, HDD/SSD) | HGST HTS545050A7E680: SSD |
| Sistema de archivos de `/` y `/home` | ext4 |
| Distribución y kernel | EndeavourOS, 7.0.11-arch1-1 |

Rol: hardware modesto y, si tiene HDD, el camino de disco giratorio (`F0-diseno.md` §2.2).

Si su kernel es anterior a 5.8, hay que actualizarlo: es el mínimo de ADR-013.

## Lo que falta: ARM (aarch64)

No hay equipo ARM disponible. Mientras tanto:

- **Compilación:** la CI compila para `aarch64-unknown-linux-gnu` en cada commit.
- **Exactitud:** las pruebas de §V1 se ejecutan con `qemu-aarch64` (qemu-user). Esto valida que el código sea correcto en ARM, **no** su rendimiento.
- **Rendimiento en ARM:** queda pendiente hasta contar con un equipo (una Raspberry Pi 5 con NVMe sería una opción económica). El informe de la F0 debe decir explícitamente que no incluye mediciones de ARM.
