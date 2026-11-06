# Medición de la F0.2 en el PC antiguo

Pasos para medir el escáner en la laptop antigua (Equipo B de `hardware.md`: EndeavourOS, Celeron de 2 núcleos, 3,7 GiB de RAM, HDD con ext4). El objetivo es el camino de disco giratorio de `F0-diseno.md` §2.2, que la Victus no puede medir.

Todos los comandos se ejecutan desde la raíz del repo: `gx bench` guarda los resultados en `bench/results/`.

## 1. Inventario y espacio

```sh
lsblk -d -o NAME,ROTA,MODEL      # el HGST debe salir con ROTA = 1
df -h ~ && df -i ~               # unos 3 GB libres y 1,2 M de inodos libres
```

## 2. Paquetes

```sh
sudo pacman -Syu rustup git base-devel gdu strace time dua-cli
rustup default stable
cargo install --locked hyperfine@1.20.0
```

- hyperfine se instala con cargo porque pacman trae la 2.0, y `gx bench` solo está probado con la 1.20 (la misma de la Victus). En el Celeron tarda unos minutos en compilar.
- `dua-cli` de pacman es la 3.0.0, la misma versión que en la Victus.

## 3. Código y compilación

```sh
git clone https://github.com/dicarch09/galaxystem.git
cd galaxystem
cargo build --release -p gx-cli --features uring
```

## 4. Árbol de prueba y exactitud en ext4

```sh
target/release/gx synth --profile home-dev --files 1000000 --seed 1 --allow-path ~/gx-arboles/home-dev-1m
target/release/gx verify-du ~/gx-arboles/home-dev-1m
```

- Generar el árbol en el HDD puede tardar entre 10 y 30 minutos (estimado, no medido).
- `verify-du` debe terminar con todo coincidiendo. Hasta ahora la exactitud solo se había verificado en btrfs.

## 5. Preparar la medición

1. Conectar el cargador.
2. Cerrar el navegador y todo lo posible: con 3,7 GiB de RAM, la falta de memoria vacía la caché y la medición "caliente" deja de serlo.
3. Poner el CPU en modo rendimiento: `powerprofilesctl set performance`, o, si no existe, `sudo cpupower frequency-set -g performance` (paquete `cpupower`).
4. Comprobar la energía. El primer valor debe ser `1` (cargador conectado) y el segundo `performance`:
   ```sh
   cat /sys/class/power_supply/*/online /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor
   ```
   Si existe `energy_performance_preference` en la misma carpeta, también debe decir `performance`.
5. Comprobar los hilos. La línea `Tiempo:` debe decir **2 hilos**; si dice otro número, detenerse y avisar:
   ```sh
   target/release/gx scan ~/gx-arboles/home-dev-1m
   ```

## 6. Medición con caché caliente

```sh
target/release/gx bench scan ~/gx-arboles/home-dev-1m --machine pc-antiguo
```

Los resultados quedan en `bench/results/pc-antiguo/<fecha>/`, con un `resumen.md`.

## 7. Medición con caché fría

1. Una corrida de prueba para saber cuánto tarda:
   ```sh
   sync; echo 3 | sudo tee /proc/sys/vm/drop_caches; time target/release/gx scan ~/gx-arboles/home-dev-1m
   ```
2. Ejecutar el comando de hyperfine de la sección "Caché fría" de `resumen.md`. Son 5 herramientas × 10 corridas: si la corrida de prueba pasó de un minuto, cambiar `--runs 10` por `--runs 3`.
3. Resumir el JSON con el comando `gx bench report` que aparece en el mismo `resumen.md`.
4. Tiempo hasta los primeros 10 000 nodos: repetir 5 veces y anotar la mediana del número entre paréntesis (`primeros 10.000 nodos: X ms`):
   ```sh
   sync; echo 3 | sudo tee /proc/sys/vm/drop_caches; target/release/gx scan ~/gx-arboles/home-dev-1m | grep Tiempo
   ```

Estos comandos manuales no revisan la energía; solo `gx bench scan` lo hace. Repetir la comprobación del paso 5.4 antes de empezar.

## 8. Entregar los resultados

`bench/results/` está en `.gitignore`. Copiar la carpeta `bench/results/pc-antiguo/` a la misma ruta del repo en la Victus (memoria USB o `scp`) y anotar las medianas tomadas a mano.
