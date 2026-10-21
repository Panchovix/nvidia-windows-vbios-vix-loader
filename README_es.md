# NVIDIA Windows VBIOS Vix Loader

[English](README.md) | Español

Carga una VBIOS modificada en una GPU NVIDIA en Windows **sin flashearla**. La imagen se guarda en la clave
de registro del driver de la GPU, donde el driver de NVIDIA busca una VBIOS de reemplazo al iniciar el
dispositivo, y luego se reinicia la GPU. `verify` comprueba que el driver la esté usando y `unload` la quita.
Nunca se escribe en el chip flash de la tarjeta.

Funciona con los drivers NVIDIA 616.x, 617.x y 621.x (616.56 o más nuevo; probado con 617.42, 617.70 y 621.01).
Podría dejar de funcionar en un driver futuro si NVIDIA decide cambiar o quitar el reemplazo por registro, así
que usa **Verificar** después de cada actualización del driver. Con 621.01, los reemplazos cargados en una RTX 4090
y una RTX A2000 se mantuvieron al actualizar el driver y, después de reiniciar Windows, Verificar mostró
`override-active` en ambas. En una RTX 5070, Verificar mostró `override-active` para una VBIOS editada cargada con
621.01 al reiniciar la GPU, sin reiniciar Windows. Turing todavía no está probada con 621.01.

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
| Ampere GA102–GA107 (GeForce RTX 30, serie RTX A) | Probado en una RTX A2000 (driver 617.42) con un payload `ampere-relocated-v1`: se aplicó un mínimo de potencia menor (5 W). En una RTX 3050 6GB Laptop GPU (driver 617.42) se aplicó un techo de potencia menor (70 W); subirlo no está probado en portátiles. Las demás GPUs Ampere están sin probar. GA100 (A100, A30, A800) no está soportada. |
| Turing TU102–TU117 (GeForce RTX 20, GTX 16, TITAN RTX, Quadro RTX, serie T) | **Experimental.** Probado en una RTX 2080 Ti (BIOS ROG Matrix, driver 617.70): con la potencia de la tarjeta y de los conectores subida, llegó a unos 458 W bajo carga. Una RTX 2070 llegó a 225 W con el límite de potencia subido desde 200 W, y una RTX 2070 SUPER también usó una imagen editada. Las demás GPUs Turing están sin probar. Si el reemplazo se rechaza, la tarjeta usa su VBIOS de fábrica. Consulta la [guía para probar Turing](https://github.com/Panchovix/nvidia-vbios-vix-editor/blob/main/docs/turing_es.md). |

Si lo pruebas, abre un issue con la GPU, la versión del driver y el resultado.

## Requisitos

- Windows 10 u 11 de 64 bits.
- Una cuenta de administrador. El programa con ventana pide permisos de administrador al abrirse; la
  herramienta de línea de comandos se ejecuta desde un PowerShell elevado ("Ejecutar como administrador").
- Driver de NVIDIA **616.x, 617.x o 621.x**, 616.56 o más nuevo (`status` muestra la versión).
- Un archivo de payload para ese mismo modelo de GPU (el mismo ID de dispositivo PCI).

## Qué archivo cargar

El archivo debe tener exactamente una VBIOS para el ID PCI de esta GPU, con checksum válido. Los volcados
completos de la flash con dos bancos se rechazan: exporta un payload de una sola cadena con la herramienta
de línea de comandos de Vix Editor (`nvbios.exe`).

Desde Vix Editor 0.1.11, al guardar una copia editada de Ada o Ampere también se guarda
`<nombre>-loader.bin` al lado: carga ese archivo. Los comandos de abajo hacen lo mismo desde la línea de
comandos.

- **Blackwell**: la ROM que guarda Vix Editor después de editar, o cualquier volcado editado de un solo
  banco (por ejemplo, de GPU-Z). Si se rechaza por tener dos bancos, usa
  `.\nvbios.exe normalize editada.rom payload.bin`.
- **Ada**: el firmware GSP carga como máximo 512 KiB contados desde la imagen legacy, así que hay que
  reubicar el payload: `.\nvbios.exe normalize editada.rom payload.bin --representation ada-relocated-v2`.
  Los payloads más grandes se rechazan.
- **Ampere GA102–GA107**: el mismo límite de 512 KiB. Usa
  `.\nvbios.exe normalize editada.rom payload.bin --representation ampere-relocated-v1`.
- **Turing TU102–TU117** (experimental): el payload cabe sin reubicar:
  `.\nvbios.exe normalize editada.rom payload.bin`. Vix Editor 0.1.18 o posterior lo guarda como `<nombre>-loader.bin`, más
  `<nombre>-loader-e0end.bin` (`--representation turing-e0-end-v1`) para probar si el primero se carga pero
  el cambio no se aplica. Reporta el resultado de **Verificar** y `nvidia-smi -q -d POWER` antes y después.

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
   lo desmarcas, reinicia Windows después. Si esta GPU es la que da imagen a tu pantalla, desmarca
   **Reiniciar la GPU ahora** y reinicia Windows: reiniciar la GPU que da imagen puede dejarla sin señal
   (pasó en una RTX 2070 SUPER, donde el mismo reemplazo funcionó después de reiniciar el equipo).
6. Para comprobar que el driver está usando la VBIOS cargada, haz clic en **Verificar** (lo mismo que
   `verify`, ver [Verificar](#verificar)).
7. Para volver a la VBIOS propia de la tarjeta, elige la GPU y haz clic en **Quitar reemplazo** (lo mismo
   que `unload`).

Cada resultado y advertencia queda también en el área de **Mensajes**. La ventana sale en español si
Windows está en español; el menú **Idioma**, o `vbios-loader-gui.exe --lang es` / `--lang en`, lo cambia. Sale
oscura si Windows usa el modo oscuro para las aplicaciones; el menú **Tema**, o `--theme light` / `--theme dark`,
lo cambia.

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

# Comprueba que el driver está usando el reemplazo guardado.
.\vbios-loader.exe verify --gpu 0

# La quita y reinicia la GPU.
.\vbios-loader.exe unload --gpu 0
```

- Elige la GPU con `--gpu ÍNDICE` (el que muestra `status`) o con `--pci BUS:DEV.FN` (por ejemplo
  `--pci 01:00.0`). Si hay una sola GPU NVIDIA, no hace falta elegirla.
- `--no-restart` escribe los valores sin reiniciar la GPU; se aplican en el siguiente reinicio.
- `--json` entrega la salida en formato para máquinas.
- Códigos de salida: `0` listo, `1` rechazado o fallido (no se escribió nada salvo que el mensaje diga lo
  contrario), `2` argumentos de línea de comandos no válidos (no se escribió nada), `3` valores escritos pero
  falló el reinicio de la GPU (reinicia Windows), `4` `verify` encontró que el driver no está usando el
  reemplazo guardado (o sus dos lecturas no coincidieron), `70` error interno inesperado, `130` interrumpido
  (Ctrl+C).

El reemplazo queda en la clave de registro de la GPU hasta que lo quites, así que se vuelve a aplicar en
cada inicio de Windows. Una instalación limpia o una actualización del driver puede volver a crear la clave
del driver y perder el reemplazo. Después de actualizar, revisa `status` y vuelve a hacer `load` si hace
falta.

Desde el código fuente, con Python 3.11 o más nuevo: `pip install .` y luego `vbios-loader status` o
`vbios-loader-gui`, o `python -m nvidia_vbios.windows_loader status` /
`python -m nvidia_vbios.windows_loader.gui` con `src` en `PYTHONPATH`.

### Avisos del antivirus

Algunos antivirus y analizadores en línea marcan los dos exe (detecciones genéricas o "ML", "Python DLL
sideloading"). Son falsos positivos: cada exe es un programa en Python empaquetado con Nuitka `--onefile`, que
al arrancar descomprime el runtime de Python en una carpeta temporal, y los exe no están firmados. El loader no
usa la red. Los compila GitHub Actions desde este repositorio (ver [Pruebas](#pruebas)); compara la descarga con
`SHA256SUMS` para confirmar que coincide con el archivo publicado. Si prefieres no ejecutarlos, usa el loader
desde el código fuente como se indica arriba.

## Verificar

**Verificar** en la ventana, o `verify` en la línea de comandos, lee mediante NvAPI la imagen de VBIOS que
está ejecutando el firmware GSP del driver, dos veces, y la compara byte a byte con el reemplazo guardado en
el registro. No escribe nada y no necesita permisos de administrador. El resultado (`state` en `--json`) es
uno de estos:

- `override-active`: la imagen en uso es el reemplazo guardado, así que el driver lo aceptó.
- `override-not-active`: son distintas, y se muestra el primer offset que difiere. Justo después de cargarlo
  sin reiniciar la GPU es lo esperado; si no, lo más probable es que el driver haya rechazado el reemplazo y
  esté usando la VBIOS propia de la tarjeta.
- `no-override`: no hay nada guardado, así que el driver debería usar la VBIOS propia de la tarjeta (se
  muestran su tamaño y su sha256). Un reemplazo quitado sigue en uso hasta reiniciar la GPU o Windows, y
  Verificar no puede distinguirlo, así que reinicia antes de comprobar que se quitó.
- `unstable`: las dos lecturas no coinciden; inténtalo de nuevo.

NvAPI entrega como máximo 1.024.000 bytes, así que un reemplazo más grande se compara solo en esos bytes: el
resultado dice entonces que coinciden los primeros 1.024.000 bytes, no que se verificó byte a byte. Verificar
necesita un driver cuya NvAPI tenga `NvAPI_GPU_GetVbiosImage` (617.42 y 621.01 la tienen); con uno más antiguo, el
error lo indica. No funciona dentro de una máquina virtual con una GPU paravirtualizada (GPU-P).

## Si la GPU no arranca

Prueba primero en una GPU que **no** esté conectada a tu pantalla (usa los gráficos integrados u otra
tarjeta para el monitor), así siempre podrás ejecutar `unload`.

Si la pantalla queda en negro justo después de cargar con **Reiniciar la GPU ahora**, primero reinicia
Windows: el reemplazo queda guardado y puede funcionar después de un reinicio completo.

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

Copyright (C) 2026 panchovix. Licencia Pública General de GNU v3.0 o posterior (`GPL-3.0-or-later`), ver
[LICENSE](LICENSE): puedes usarlo, estudiarlo, modificarlo y compartirlo, y las versiones modificadas que
compartas deben seguir bajo la misma licencia. Ver también [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). La
versión 0.1.0 se publicó bajo la licencia MIT.
