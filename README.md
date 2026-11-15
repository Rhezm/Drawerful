# Drawerly

Organizador de escritorio para Windows, hecho en Rust. Agrupa programas, carpetas y archivos en **cajones** compactos: cada cajón es un pequeño icono en el escritorio que se despliega al pasar el mouse o al hacer clic, y vuelve a guardarse cuando terminas.

> **Estado:** versión temprana (0.1). Funciona, pero puede tener errores y cambiar bastante entre versiones.

## Características

- **Cajones desplegables:** se abren con hover o con clic (lo eliges en la configuración).
- **Arrastrar y soltar:** agrega archivos, carpetas o accesos directos soltándolos sobre el cajón, esté abierto o cerrado. Si mantienes el arrastre sobre un cajón cerrado, se abre solo.
- **Dos modos de guardado:**
  - *Referencia:* el archivo original queda donde está.
  - *Mover:* el archivo sale del escritorio y se guarda dentro del cajón. Al quitarlo, vuelve a su ubicación original.
- **Apariencia personalizable:**
  - Color de fondo, opacidad y desenfoque del fondo de pantalla.
  - Borde, redondeo de esquinas y color del texto.
  - Por defecto: oscurecido al 20 % con desenfoque leve.
- **Iconos para cada cajón:** 20 prediseñados (juegos, documentos, fotos, videos, música…) o una imagen propia (PNG, JPG, ICO, WEBP, BMP). La imagen se reduce automáticamente para ahorrar recursos.
- **Tamaños configurables:** ancho, alto, columnas, tamaño de celda e iconos. Por defecto usa los mismos tamaños que el escritorio de Windows.
- **Orden:** manual (arrastrando dentro del cajón), por nombre, por tipo o por fecha de agregado.
- **Vive en la bandeja del sistema:**
  - Doble clic en el icono: abre la configuración.
  - Clic derecho: activar o desactivar, iniciar con Windows, bloquear posición, crear cajón y salir.
- **Liviano:** Win32 nativo, sin motor web ni runtime. El desenfoque se calcula una sola vez y los iconos se cargan en segundo plano.

## Requisitos

- Windows 10 u 11 (64 bits)

## Instalación

Descarga `drawerly.exe` desde [Releases](../../releases) y ejecútalo. No necesita instalador: aparecerá en la bandeja del sistema (en los iconos ocultos, junto al reloj).

Para que arranque con Windows: clic derecho en el icono de la bandeja → **Iniciar con Windows**.

## Uso rápido

1. Al primer inicio se crea un cajón en la esquina superior izquierda del escritorio.
2. Arrastra archivos o accesos directos sobre él.
3. Pasa el mouse (o haz clic) para abrirlo y haz clic en un elemento para ejecutarlo.
4. Arrastra el cajón con el clic izquierdo para moverlo.
5. Clic derecho sobre un cajón o elemento para ver más opciones.

## Compilar desde el código

Necesitas [Rust](https://rustup.rs) con el toolchain MSVC y las Build Tools de Visual Studio (carga de trabajo *Desarrollo de escritorio con C++*).

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install --id Rustlang.Rustup -e
# Reabre la terminal
git clone https://github.com/Rhezm/drawerly.git
cd drawerly
cargo build --release
```

El ejecutable queda en `target\release\drawerly.exe`.

## Datos y configuración

Todo se guarda en `%APPDATA%\Drawerly`:

| Archivo / carpeta | Contenido |
|---|---|
| `config.json` | Configuración y contenido de los cajones |
| `store\` | Archivos movidos dentro de los cajones (modo *Mover*) |
| `icons\` | Iconos personalizados ya reducidos |
| `drawerly.log` | Registro de errores |

> Si usas el modo *Mover*, no borres la carpeta `store` a mano: ahí están tus archivos. Para devolverlos, quítalos del cajón o elimina el cajón.

## Estructura del proyecto

| Módulo | Función |
|---|---|
| `host.rs` | Proceso principal: bandeja, ventanas de los cajones, eventos y arrastrar y soltar |
| `settings.rs` | Ventana de configuración (egui), se ejecuta como proceso aparte |
| `canvas.rs` | Dibujo propio con antialiasing: formas, imágenes, texto y desenfoque |
| `surface.rs` | Ventanas con transparencia por píxel (`UpdateLayeredWindow`) |
| `wallpaper.rs` | Fondo de pantalla desenfocado y cacheado |
| `icons.rs` | Iconos y miniaturas del sistema, cargados en un hilo aparte |
| `storage.rs` | Mover y restaurar archivos, importar iconos |
| `config.rs` | Modelo de datos y guardado atómico |
