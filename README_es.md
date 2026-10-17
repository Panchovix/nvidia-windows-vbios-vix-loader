# NVIDIA Windows VBIOS Vix Loader

[English](README.md) | Español

Carga una VBIOS modificada en una GPU NVIDIA en Windows **sin flashearla**. La imagen se guarda en la clave
de registro del driver de la GPU, donde el driver de NVIDIA busca una VBIOS de reemplazo al iniciar el
dispositivo, y luego se reinicia la GPU. `unload` la quita. Nunca se escribe en el chip flash de la tarjeta.

Gracias a **@pantsoftime** ([overclock.net](https://www.overclock.net)), cuyo trabajo original con
`vbios_override` es la base de estos proyectos. Sin él, nada de esto habría sido posible.

Para modificar la VBIOS, usa [NVIDIA VBIOS Vix Editor](https://github.com/Panchovix/nvidia-vbios-vix-editor).
Para Linux, está [NVIDIA Linux VBIOS Vix Loader](https://github.com/Panchovix/nvidia-linux-vbios-vix-loader).
Este repositorio solo carga la VBIOS en Windows.

Este proyecto no está afiliado a NVIDIA ni cuenta con su respaldo. NVIDIA y GeForce son marcas registradas
de NVIDIA Corporation.

> **Advertencia.** Una VBIOS modificada puede subir la potencia, los relojes y los voltajes por encima de lo
> que la tarjeta soporta por diseño. Puedes dañar el hardware. Guarda una copia de tu ROM original y úsalo
> bajo tu propio riesgo.

## Qué hace

`load` escribe estos valores en la clave del driver de la GPU,
`HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\NNNN`:

| Valor | Tipo | Contenido |
| --- | --- | --- |
| `vbios` | REG_BINARY | La cadena de imágenes de la VBIOS de tu archivo: desde la imagen legacy hasta el final de la última imagen. Lo que haya antes o después en el archivo no se guarda. |
| `EnableGpuFirmware` | REG_DWORD | `0x11` (firmware GSP activado, con respaldo permitido). |
| `VixLoaderPreviousEnableGpuFirmware` | REG_SZ | El valor anterior de `EnableGpuFirmware`, o `absent` si no existía. Se guarda solo en el primer `load`, para que `unload` pueda restaurarlo. |

Además borra `romOverride00` a `romOverrideFF` si existen, hace flush de la clave, vuelve a leer los valores
para comprobarlos y reinicia la GPU con `pnputil /restart-device`. Si el reinicio falla, reinicia Windows.

Son los mismos valores `vbios` y `EnableGpuFirmware` que escribe mVolt+ cuando carga un archivo de BIOS, así
que una BIOS cargada con mVolt+ en la misma GPU queda reemplazada. Este proyecto no está afiliado a mVolt+.
El resto de los valores de la clave no se tocan.

`unload` borra `vbios` y los registros `romOverride` que haya, restaura `EnableGpuFirmware` (o lo borra si
antes no existía), borra `VixLoaderPreviousEnableGpuFirmware` y reinicia la GPU.

**No** flashea la tarjeta, no edita la VBIOS y no cambia por sí mismo relojes, ventiladores ni límites de
potencia. `unload` deja la clave como estaba antes del primer `load`, salvo que un reemplazo escrito por otra
herramienta (el `vbios` o los registros `romOverride` de mVolt+) no se restaura; en ese caso `load` lo avisa
y `unload` deja `EnableGpuFirmware` como lo dejó esa herramienta.

## Estado

| Familia | Estado |
| --- | --- |
| Blackwell GB20x (GeForce RTX 50, RTX PRO Blackwell) | Usa los mismos valores de registro con los que mVolt+ carga archivos de BIOS en Blackwell. Probado en una RTX 5070 (driver 617.42): una VBIOS editada se cargó y funcionó al reiniciar la GPU, sin reiniciar Windows. |
| Ada AD10x (GeForce RTX 40, RTX Ada) | Probado en una RTX 4090 (VBIOS 95.02.18, driver 617.42) con un payload `ada-relocated-v2`: se aplicaron un límite de potencia mayor (671 W) y un rango de offset de reloj del núcleo mayor (+1200 MHz). Las demás GPUs Ada están sin probar. |
| Ampere GA102–GA107 (GeForce RTX 30, serie RTX A) | Probado en una RTX A2000 (driver 617.42) con un payload `ampere-relocated-v1`: se aplicó un mínimo de potencia menor (5 W). Las demás GPUs Ampere están sin probar. GA100 (A100, A30, A800) no está soportada. |

Si lo pruebas, abre un issue con la GPU, la versión del driver y el resultado.

## Requisitos

- Windows 10 u 11 de 64 bits.
- Una cuenta de administrador. El programa con ventana pide permisos de administrador al abrirse; la
  herramienta de línea de comandos se ejecuta desde un PowerShell elevado ("Ejecutar como administrador").
- Driver de NVIDIA **616.56 o más nuevo** (`status` muestra la versión).
- Un archivo de payload para ese mismo modelo de GPU (el mismo ID de dispositivo PCI).

## Qué archivo cargar

El archivo debe tener exactamente una VBIOS para el ID PCI de esta GPU, con checksum válido. Los volcados
completos de la flash con dos bancos se rechazan: exporta un payload de una sola cadena con la herramienta
de línea de comandos de Vix Editor (`nvbios`).

Desde Vix Editor 0.1.11, al guardar una copia editada de Ada o Ampere también se guarda
`<nombre>-loader.bin` al lado: carga ese archivo. Los comandos de abajo hacen lo mismo desde la línea de
comandos.

- **Blackwell**: la ROM que guarda Vix Editor después de editar, o cualquier volcado editado de un solo
  banco (por ejemplo, de GPU-Z). Si se rechaza por tener dos bancos, usa
  `nvbios normalize editada.rom payload.bin`.
- **Ada**: el firmware GSP carga como máximo 512 KiB contados desde la imagen legacy, así que hay que
  reubicar el payload: `nvbios normalize editada.rom payload.bin --representation ada-relocated-v2` (en la
  RTX 2000 Ada también sirve `ada-relocated-v1`). Los payloads más grandes se rechazan.
- **Ampere GA102–GA107**: el mismo límite de 512 KiB. Usa
  `nvbios normalize editada.rom payload.bin --representation ampere-relocated-v1`.

## Uso

### Ventana

1. Descarga `vbios-loader-gui.exe` desde los releases y compáralo con `SHA256SUMS`.
2. Ábrelo. Windows te pide permisos de administrador (escribe en la clave de registro de la GPU).
3. Elige la GPU en la lista. La lista muestra cada GPU NVIDIA con su ubicación PCI, la versión del driver y
   el estado del reemplazo.
4. Haz clic en **Archivo VBIOS...** y elige el archivo. La ventana lo revisa de inmediato para la GPU elegida y
   muestra si es válido (familia, bytes a guardar, sha256) o por qué se rechaza.
5. Haz clic en **Cargar** (lo mismo que `load`), lee la confirmación (GPU, archivo, cambios en el registro)
   y acepta. Con **Reiniciar la GPU ahora** marcado, la GPU se reinicia (puede tardar hasta 45 segundos); si
   lo desmarcas, reinicia Windows después.
6. Para volver a la VBIOS propia de la tarjeta, elige la GPU y haz clic en **Quitar reemplazo** (lo mismo
   que `unload`).

Cada resultado y advertencia queda también en el área de **Mensajes**. La ventana sale en español si
Windows está en español; el menú **Idioma**, o `vbios-loader-gui.exe --lang es` / `--lang en`, lo cambia.

### Línea de comandos

Descarga `vbios-loader.exe` desde los releases, compáralo con `SHA256SUMS` y abre un PowerShell elevado en
esa carpeta.

```powershell
# Lista las GPUs NVIDIA: índice, ubicación PCI, versión del driver y estado del reemplazo.
.\vbios-loader.exe status

# Muestra lo que se escribiría, sin escribir nada.
.\vbios-loader.exe load .\payload.bin --gpu 0 --dry-run

# Guarda la VBIOS de reemplazo y reinicia la GPU.
.\vbios-loader.exe load .\payload.bin --gpu 0

# La quita y reinicia la GPU.
.\vbios-loader.exe unload --gpu 0
```

- Elige la GPU con `--gpu ÍNDICE` (el que muestra `status`) o con `--pci BUS:DEV.FN` (por ejemplo
  `--pci 01:00.0`). Si hay una sola GPU NVIDIA, no hace falta elegirla.
- `--no-restart` escribe los valores sin reiniciar la GPU; se aplican en el siguiente reinicio.
- `--json` entrega la salida en formato para máquinas.
- Códigos de salida: `0` listo, `1` rechazado o fallido (no se escribió nada salvo que el mensaje diga lo
  contrario), `3` valores escritos pero falló el reinicio de la GPU (reinicia Windows).

El reemplazo queda en la clave de registro de la GPU hasta que lo quites, así que se vuelve a aplicar en
cada inicio de Windows. Una instalación limpia o una actualización del driver puede volver a crear la clave
del driver y perder el reemplazo. Después de actualizar, revisa `status` y vuelve a hacer `load` si hace
falta.

Desde el código fuente, con Python 3.11 o más nuevo: `pip install .` y luego `vbios-loader status` o
`vbios-loader-gui`, o `python -m nvidia_vbios.windows_loader status` /
`python -m nvidia_vbios.windows_loader.gui` con `src` en `PYTHONPATH`.

## Si la GPU no arranca

Prueba primero en una GPU que **no** esté conectada a tu pantalla (usa los gráficos integrados u otra
tarjeta para el monitor), así siempre podrás ejecutar `unload`.

Si Windows no da imagen o la GPU aparece con error en el Administrador de dispositivos:

1. Arranca en Modo seguro: mantén Shift mientras haces clic en Reiniciar y luego Solucionar problemas →
   Opciones avanzadas → Configuración de inicio → Reiniciar → 4 (Habilitar modo seguro). Si no tienes
   escritorio, apaga el PC durante el arranque dos o tres veces (o arranca desde un medio de instalación de
   Windows → Reparar el equipo) para llegar al mismo menú Solucionar problemas. En Modo seguro el driver de
   NVIDIA no se carga, así que la pantalla funciona.
2. Abre `vbios-loader-gui.exe`, elige la GPU, desmarca **Reiniciar la GPU ahora** y haz clic en **Quitar
   reemplazo**; o, en un PowerShell elevado, ejecuta `.\vbios-loader.exe unload --gpu 0 --no-restart` (con
   el índice de tu GPU según `status`). Luego reinicia Windows normalmente.
3. Si eso no funciona, abre `regedit` y ve a
   `HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}`.
   Abre la subclave numerada cuyo `DriverDesc` sea tu GPU (`status` la muestra como `driver_key`). Borra
   `vbios`. Vuelve a poner en `EnableGpuFirmware` el valor guardado en `VixLoaderPreviousEnableGpuFirmware`,
   o borra `EnableGpuFirmware` si ahí dice `absent`. Luego borra `VixLoaderPreviousEnableGpuFirmware` y
   reinicia.

## Pruebas

```sh
PYTHONPATH=src python -m unittest discover -s tests
```

Las pruebas usan archivos sintéticos y un registro simulado; funcionan en Linux y en Windows y nunca tocan
una GPU ni el registro real. El workflow `.github/workflows/build.yml` las ejecuta en Windows, compila
`vbios-loader.exe` y `vbios-loader-gui.exe` con Nuitka, prueba que la ventana abra (con una captura de
pantalla) y publica las versiones con tag como releases, junto con `SHA256SUMS` y los textos de licencia de
terceros.

## Licencia

MIT. Ver [LICENSE](LICENSE) y [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
