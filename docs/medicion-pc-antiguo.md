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

**Por qué cambió este paso.** Las dos mediciones frías con hyperfine (2026-11-06 y 2026-11-07) salieron inválidas por cómo llegó el comando, no por gx. El comando de varias líneas se pegó unido en una sola, con las `\` adentro. Cada `\ ` llegó a hyperfine como un comando que es solo un espacio: terminaba al instante y se llevaba el nombre `-n` de la herramienta siguiente. Lo que parecía "dua" o "du" sin terminar era gx-sync en frío.

En su lugar, `bench/fria.sh` corre una herramienta a la vez:
- limpia la caché antes de cada corrida;
- mide con `/usr/bin/time -v`;
- guarda la salida completa de cada corrida;
- mantiene viva la contraseña de sudo mientras dura.

Todos los comandos de este paso son de **una sola línea**.

1. Traer los cambios y recompilar (el escáner cambió en `da99cfb`):
   ```sh
   git pull
   ```
   ```sh
   cargo build --release -p gx-cli --features uring
   ```
2. Comprobar la energía (paso 5.4). El primer valor debe ser `1` y el segundo `performance`:
   ```sh
   cat /sys/class/power_supply/*/online /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor
   ```
3. Correr la medición. Pide la contraseña de sudo una vez:
   ```sh
   sh bench/fria.sh ~/gx-arboles/home-dev-1m bench/results/pc-antiguo/2026-11-07
   ```
   - Mide, en este orden y una vez cada uno: gx-sync, du, gx-uring y gx con 1 hilo. Con unos 21 min por corrida, son **más o menos 1 h 30 min**. No hace falta mirarlo: cada corrida imprime su tiempo y su RAM al terminar.
   - En cada corrida de gx, la línea `Nodos:` debe decir **1140500**. Si dice otra cosa, guardar la salida y avisar.
   - Si hay que cortarlo, Ctrl+C. Lo ya medido queda guardado.
4. Opcional, solo si sobra tiempo: dua y gdu en frío no bloquean ninguna decisión. Si se quieren, una corrida de cada uno a mano, después de limpiar la caché:
   ```sh
   sync; echo 3 | sudo tee /proc/sys/vm/drop_caches; /usr/bin/time -v dua aggregate ~/gx-arboles/home-dev-1m
   ```

## 8. Entregar los resultados

Desde el 2026-11-06, `bench/results/` ya no está en `.gitignore`. `bench/fria.sh` deja todo en archivos. En `bench/results/pc-antiguo/2026-11-07/notas.md` anotar solo lo que no quede ahí, por ejemplo si se cortó o se usó la máquina durante la medición. Después:

```sh
git add bench/results/pc-antiguo
git commit -s -m "bench: add old laptop cold cache results"
git push
```

El `-s` agrega el `Signed-off-by` que exige el proyecto (DCO).
