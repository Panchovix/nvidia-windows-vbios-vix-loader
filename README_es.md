# NVIDIA Windows VBIOS Vix Loader

[English](README.md) | Español

Carga una VBIOS modificada en una GPU NVIDIA en Windows **sin flashearla**. La imagen se guarda en la clave
de registro del driver de la GPU, donde el driver de NVIDIA busca una VBIOS de reemplazo, y se aplica al
reiniciar Windows. `verify` comprueba que el driver la esté usando y `unload` la quita. Nunca se escribe en el
chip flash de la tarjeta.

Funciona con los drivers NVIDIA 616.x, 617.x y 621.x (616.56 o más nuevo; probado con 621.01 en el modo
predeterminado del driver, y con versiones hasta la 0.3.2 en 617.42 y 617.70). Podría dejar de funcionar
en un driver futuro si NVIDIA decide cambiar o quitar el reemplazo por registro, así que usa **Verificar**
después de cada actualización del driver.

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
| `VixLoaderPreviousEnableGpuFirmware` | REG_SZ | El valor de `EnableGpuFirmware` de antes del primer `load`, guardado solo en el primer `load` para que `unload` pueda restaurarlo: `absent` si no existía, un DWORD como `0x00000012`, o, si no era un REG_DWORD de 4 bytes (otra herramienta o una edición a mano puede dejar ahí un REG_SZ, REG_QWORD, REG_BINARY o un DWORD de otro tamaño), una copia exacta de su tipo y sus bytes escrita como la escribe un archivo `.reg`, por ejemplo `hex(1):31,00,00,00`. |

Además borra `romOverride00` a `romOverrideFF` si existen, hace flush de la clave y vuelve a leer los valores
para comprobarlos. Deja `EnableGpuFirmware` como está, salvo que vuelve a poner el valor que guardó una versión
anterior que lo cambió a `0x11`.

Las versiones hasta la 0.3.2 también ponían `EnableGpuFirmware` en `0x11`, lo que activa el modo de firmware GSP
del driver. En Windows el driver no usa ese modo por defecto en las tarjetas GeForce y RTX de estación de
trabajo (incluidas las RTX 50), y en una RTX 4070 hacía que el driver de pantalla se reiniciara cada vez que se
abría Apple Music, incluso sin un reemplazo. En su modo predeterminado, el driver lee el reemplazo por sí mismo.

Reinicia Windows para aplicar el cambio, con **Reiniciar**, no Apagar: después de apagar, el inicio rápido de
Windows retoma el estado anterior del driver. El driver mantiene la imagen hasta que Windows se reinicia, así
que reiniciar solo la GPU no basta para aplicar ni quitar un reemplazo.

mVolt+ guarda su archivo de BIOS en el mismo valor `vbios` (y pone `EnableGpuFirmware` en `0x11`), así que una
BIOS cargada con mVolt+ en la misma GPU queda reemplazada. Si `EnableGpuFirmware` ya estaba en `0x11` antes del
primer `load`, se queda así, y el modo de firmware GSP sigue activado. Para usar el modo predeterminado del
driver, borra `EnableGpuFirmware` antes del primer `load`. Si el reemplazo ya está cargado, haz primero `unload`
(que lo vuelve a poner en `0x11`), luego borra el valor, vuelve a hacer `load` y reinicia Windows. Este proyecto
no está afiliado a mVolt+. El resto de los valores de la clave no se tocan.

`unload` borra `vbios` y los registros `romOverride` que haya, restaura `EnableGpuFirmware` tal como estaba
antes del primer `load`, con el mismo tipo (o lo borra si antes no existía), y borra
`VixLoaderPreviousEnableGpuFirmware`. No reinicia la GPU: reinicia Windows. Si el valor guardado empieza con
`hex(`, quita el reemplazo con la versión 0.3.2 o más nueva: las versiones anteriores no entienden esa forma,
así que la borran y dejan `EnableGpuFirmware` en `0x11`.

**No** flashea la tarjeta, no edita la VBIOS y no cambia por sí mismo relojes, ventiladores ni límites de
potencia. `unload` deja la clave como estaba antes del primer `load`, salvo que un reemplazo escrito por otra
herramienta (el `vbios` o los registros `romOverride` de mVolt+) no se restaura; en ese caso `load` lo avisa
y `unload` deja `EnableGpuFirmware` como lo dejó esa herramienta.

## Actualizar desde la 0.3.2 o anterior

Las versiones hasta la 0.3.2 ponían `EnableGpuFirmware` en `0x11` en cada GPU con un reemplazo, lo que deja esas
GPUs en modo de firmware GSP. Vuelve a cargar el mismo archivo con la 0.4.0 (esta versión restaura el valor
guardado), o quita el reemplazo, y reinicia Windows. `status`, y el área de **Mensajes** de la ventana, muestran
las GPUs que siguen con el modo de firmware GSP activado por una versión anterior.

## Avisos del antivirus

Algunos antivirus y analizadores en línea marcan los dos exe (por ejemplo detecciones genéricas o "ML", o
"Python DLL sideloading"). Son falsos positivos: cada exe es un programa en Python empaquetado con Nuitka
`--onefile`, que al arrancar descomprime el runtime de Python en una carpeta temporal, y los exe no están
firmados. El loader no usa la red. Los compila GitHub Actions desde este repositorio (ver [Pruebas](#pruebas));
compara la descarga con `SHA256SUMS` para confirmar que coincide con el archivo publicado.

También envié los binarios a Microsoft para que dejen de marcarlos. Si aun así no quieres ejecutar los exe de
los releases, clona el repositorio y usa el loader desde el código fuente (ver el final de [Uso](#uso)), o
compila los exe tú mismo igual que el workflow (ver [Pruebas](#pruebas)).

Por último, si no quieres usar este loader para una VBIOS editada, puedes usar el reemplazo de BIOS de mVolt+
(https://github.com/b00nz/mVolt). Los problemas de mVolt+ se reportan allá, no en este repositorio. Ten en
cuenta que su reemplazo de BIOS funciona sobre todo en Blackwell.

## Estado

El modo predeterminado del driver, el estado que deja la 0.4.0 (`vbios` guardado y `EnableGpuFirmware` sin
valor o en su valor predeterminado `0x12`), se probó con el driver 621.01 en una RTX 4090, una RTX 5070 y una
RTX A2000 (la A2000 es la que da imagen): después de reiniciar Windows, Verificar confirmó el reemplazo en las
tres, y los límites de potencia editados (máximos más altos y mínimos más bajos) se aplicaron y se mantuvieron
bajo carga. En la A2000, Apple Music se abrió y funcionó sin que el driver se reiniciara. Turing, los portátiles y los drivers distintos de 621.01 todavía no están probados en el modo predeterminado. Si
una GPU no arranca en el modo predeterminado, quita el reemplazo en Modo seguro (ver
[Si la GPU no arranca](#si-la-gpu-no-arranca)) y abre un issue; la versión 0.3.2, que activa el modo de firmware
GSP, sigue disponible.

Resultados anteriores, con versiones hasta la 0.3.2 (modo de firmware GSP):

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
- **Ada**: el driver carga como máximo 512 KiB contados desde la imagen legacy, así que hay que
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
   y acepta. Luego reinicia Windows (**Reiniciar**, no Apagar) para aplicar el reemplazo.
6. Después de reiniciar, haz clic en **Verificar** para comprobar que el driver está usando la VBIOS cargada
   (lo mismo que `verify`, ver [Verificar](#verificar)).
7. Para volver a la VBIOS propia de la tarjeta, elige la GPU, haz clic en **Quitar reemplazo** (lo mismo que
   `unload`) y reinicia Windows.

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

# Guarda la VBIOS de reemplazo; luego reinicia Windows para aplicarla.
.\vbios-loader.exe load .\payload.bin --gpu 0

# Después de reiniciar, comprueba que el driver está usando el reemplazo guardado.
.\vbios-loader.exe verify --gpu 0

# La quita; luego reinicia Windows.
.\vbios-loader.exe unload --gpu 0
```

- Elige la GPU con `--gpu ÍNDICE` (el que muestra `status`) o con `--pci BUS:DEV.FN` (por ejemplo
  `--pci 01:00.0`). Si hay una sola GPU NVIDIA, no hace falta elegirla.
- `--no-restart` no hace nada: el loader ya no reinicia la GPU, y la opción se sigue aceptando para que los
  scripts anteriores sigan funcionando.
- `--json` entrega la salida en formato para máquinas.
- Códigos de salida: `0` listo, `1` rechazado o fallido (no se escribió nada salvo que el mensaje diga lo
  contrario), `2` argumentos de línea de comandos no válidos (no se escribió nada), `4` `verify` encontró que
  el driver no está usando el reemplazo guardado (o sus dos lecturas no coincidieron), `70` error interno
  inesperado, `130` interrumpido (Ctrl+C).

El reemplazo queda en la clave de registro de la GPU hasta que lo quites, así que se vuelve a aplicar en
cada inicio de Windows. Una instalación limpia o una actualización del driver puede volver a crear la clave
del driver y perder el reemplazo. Después de actualizar, revisa `status` y vuelve a hacer `load` si hace
falta.

Desde el código fuente, con Python 3.11 o más nuevo: `pip install .` y luego `vbios-loader status` o
`vbios-loader-gui`, o `python -m nvidia_vbios.windows_loader status` /
`python -m nvidia_vbios.windows_loader.gui` con `src` en `PYTHONPATH`.

## Verificar

**Verificar** en la ventana, o `verify` en la línea de comandos, lee mediante NvAPI la imagen de VBIOS que
está usando el driver, dos veces, y la compara byte a byte con el reemplazo guardado en el registro. No
escribe nada y no necesita permisos de administrador. Después de cargar o quitar un reemplazo, reinicia
Windows antes de comprobar. El resultado (`state` en `--json`) es uno de estos:

- `override-active`: la imagen en uso es el reemplazo guardado, así que el driver lo aceptó.
- `override-not-active`: son distintas, y se muestra el primer offset que difiere. Justo después de cargarlo,
  antes de reiniciar Windows, es lo esperado; si no, lo más probable es que el driver haya rechazado el
  reemplazo y esté usando la VBIOS propia de la tarjeta.
- `no-override`: no hay nada guardado, así que el driver debería usar la VBIOS propia de la tarjeta (se
  muestran su tamaño y su sha256). Un reemplazo quitado sigue en uso hasta reiniciar Windows, y Verificar no
  puede distinguirlo, así que reinicia Windows antes de comprobar que se quitó.
- `unstable`: las dos lecturas no coinciden; inténtalo de nuevo.

NvAPI entrega como máximo 1.024.000 bytes, así que un reemplazo más grande se compara solo en esos bytes: el
resultado dice entonces que coinciden los primeros 1.024.000 bytes, no que se verificó byte a byte. Verificar
necesita un driver cuya NvAPI tenga `NvAPI_GPU_GetVbiosImage` (617.42 y 621.01 la tienen); con uno más antiguo, el
error lo indica. No funciona dentro de una máquina virtual con una GPU paravirtualizada (GPU-P).

## Si la GPU no arranca

Prueba primero en una GPU que **no** esté conectada a tu pantalla (usa los gráficos integrados u otra
tarjeta para el monitor), así siempre podrás ejecutar `unload`.

Si Windows no da imagen o la GPU aparece con error en el Administrador de dispositivos:

1. Arranca en Modo seguro: mantén Shift mientras haces clic en Reiniciar y luego Solucionar problemas →
   Opciones avanzadas → Configuración de inicio → Reiniciar → 4 (Habilitar modo seguro). Si no tienes
   escritorio, apaga el PC durante el arranque dos o tres veces (o arranca desde un medio de instalación de
   Windows → Reparar el equipo) para llegar al mismo menú Solucionar problemas. En Modo seguro el driver de
   NVIDIA no se carga, así que la pantalla funciona.
2. Abre `vbios-loader-gui.exe`, elige la GPU y haz clic en **Quitar reemplazo**; o, en un PowerShell
   elevado, ejecuta `.\vbios-loader.exe unload --gpu 0` (con el índice de tu GPU según `status`). Luego
   reinicia Windows normalmente.
3. Si eso no funciona, abre `regedit` y ve a
   `HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}`.
   Abre la subclave numerada cuyo `DriverDesc` sea tu GPU (`status` la muestra como `driver_key`). Si
   `VixLoaderPreviousEnableGpuFirmware` empieza con `hex(`, usa el paso 4. Si no, borra `vbios` y vuelve a
   poner en `EnableGpuFirmware` el valor guardado, o borra `EnableGpuFirmware` si el valor guardado es
   `absent`. Luego borra `VixLoaderPreviousEnableGpuFirmware` y reinicia Windows.
4. Si el valor guardado empieza con `hex(`, `EnableGpuFirmware` no era un REG_DWORD de 4 bytes antes del
   primer `load` y ese texto es una copia exacta en la notación de los archivos `.reg`. Lo más fácil para
   volver a ponerlo es un archivo `.reg` en lugar del paso 3: guarda estas líneas en el Bloc de notas como
   `restore.reg`, cambiando `NNNN` por el número de la subclave y el texto después de `"EnableGpuFirmware"=`
   por el valor guardado. Haz doble clic en el archivo (o ejecuta `reg import restore.reg` en un PowerShell
   elevado) y reinicia Windows. También borra `vbios` y el valor guardado.

   ```
   Windows Registry Editor Version 5.00

   [HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\NNNN]
   "vbios"=-
   "EnableGpuFirmware"=hex(1):31,00,00,00
   "VixLoaderPreviousEnableGpuFirmware"=-
   ```

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
