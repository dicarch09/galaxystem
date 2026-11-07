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

La primera medición (2026-11-06) salió inválida: gx-sync marcó 13, 33 y 372 ms con 14 MB de RAM, cuando en caliente tarda 3 s con 132 MB, y dua no terminó por falta de tiempo. Por eso ahora se comprueba cada paso antes de seguir y se miden solo tres herramientas.

1. Traer los cambios y comprobar la energía (paso 5.4). El primer valor debe ser `1` y el segundo `performance`:
   ```sh
   git pull
   cat /sys/class/power_supply/*/online /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor
   ```
   El código del escáner no cambió desde `e636574`, así que no hace falta recompilar.
2. Exactitud en ext4. Debe terminar con todo coincidiendo; si muestra diferencias, guardar la salida completa:
   ```sh
   target/release/gx verify-du ~/gx-arboles/home-dev-1m
   ```
3. Dejar la contraseña de sudo en caché, para que hyperfine no se trabe pidiéndola:
   ```sh
   sudo -v
   ```
4. Escaneo de prueba en frío. Anotar el tiempo total y comprobar que `Nodos:` dice **1140500**:
   ```sh
   sync; echo 3 | sudo tee /proc/sys/vm/drop_caches; target/release/gx scan ~/gx-arboles/home-dev-1m
   ```
   **Si dice muchos menos nodos, detenerse** y guardar la salida completa, sobre todo la línea `Incidencias:`. Eso explicaría la medición inválida anterior.
5. Hyperfine con gx-sync, gx-uring y du, 3 corridas cada uno. Cada corrida en frío dura más o menos lo que el escaneo de prueba (du, probablemente más), así que conviene reservar al menos 10 veces ese tiempo:
   ```sh
   hyperfine --prepare 'sync; echo 3 | sudo tee /proc/sys/vm/drop_caches' --runs 3 \
       --export-json bench/results/pc-antiguo/2026-11-06/scan-fria-2.json \
       -n gx-sync  'target/release/gx scan ~/gx-arboles/home-dev-1m' \
       -n gx-uring 'target/release/gx scan ~/gx-arboles/home-dev-1m --statx uring' \
       -n du       'du -s -x ~/gx-arboles/home-dev-1m'
   ```
   Si sudo vuelve a pedir contraseña a mitad de camino (su caché dura unos 5 minutos por defecto), repetir `sudo -v` en otra terminal.
6. Resumir el JSON:
   ```sh
   target/release/gx bench report bench/results/pc-antiguo/2026-11-06/scan-fria-2.json
   ```
7. Tiempo hasta los primeros 10 000 nodos. Repetir 5 veces y anotar los 5 números entre paréntesis (`primeros 10.000 nodos: X ms`):
   ```sh
   sync; echo 3 | sudo tee /proc/sys/vm/drop_caches; target/release/gx scan ~/gx-arboles/home-dev-1m | grep Tiempo
   ```
8. Opcional, solo si sobra tiempo: dua y gdu en frío, en una corrida aparte con `--runs 1`. Son los lentos y no bloquean ninguna decisión.

## 8. Entregar los resultados

Desde el 2026-11-06, `bench/results/` ya no está en `.gitignore`. Anotar en `bench/results/pc-antiguo/2026-11-06/notas.md` lo que no queda en archivos: el resultado de `verify-du`, la salida del escaneo de prueba y los 5 tiempos hasta 10 000 nodos. Después:

```sh
git add bench/results/pc-antiguo
git commit -s -m "bench: add old laptop cold cache results"
git push
```

El `-s` agrega el `Signed-off-by` que exige el proyecto (DCO).
