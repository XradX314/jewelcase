# JewelCase

**Un CD girando en tu escritorio.** Widget 3D de código abierto: una jewel case transparente con el CD del álbum que suena en Spotify, con refracción real de tu fondo de pantalla.

🌐 **Web y demo en vivo:** https://xradx314.github.io/jewelcase/
⬇️ **Descargas:** [Releases](https://github.com/XradX314/jewelcase/releases/latest)

> *English:* JewelCase is an open-source desktop widget (Tauri 2 + Three.js) that renders a physically-inspired transparent CD jewel case with the disc of the album currently playing on Spotify. Windows is fully supported; Linux and macOS builds are experimental. See the [website](https://xradx314.github.io/jewelcase/) for a live demo.

## Características

- Caja con medidas reales (142 × 125 × 10,4 mm) y plástico transparente: absorción de Beer-Lambert, reflexión interna total, Fresnel, rayones procedurales.
- CD con difracción física, marcas que giran y centro de policarbonato con grabado.
- Busca la foto real del disco (MusicBrainz + Cover Art Archive, Discogs, TheAudioDB, Fanart.tv); si no hay, genera la serigrafía desde la portada.
- Ventana transparente con click-through, refracción en vivo del escritorio (Windows), bandeja del sistema y ventana de ajustes.
- Spotify (PKCE, tu propio Client ID): álbum actual, lista de canciones en el librito y reproducir al tocar un renglón (Premium).

## Instalación

| Sistema | Archivo | Estado |
|---|---|---|
| Windows 10/11 x64 | `.msi` / `-setup.exe` | Completo |
| Linux x64 | `.AppImage` / `.deb` / `.rpm` | Experimental |
| macOS (ARM / Intel) | `.dmg` | Experimental |

Los instaladores no están firmados: en Windows, SmartScreen → «Más información → Ejecutar de todas formas»; en macOS, clic derecho → Abrir.

En Linux y macOS no hay refracción en vivo del escritorio ni detección instantánea de Spotify (SMTC); el resto funciona. En Linux con Wayland el click-through dinámico puede no funcionar (usá X11).

## Uso

- **Arrastrar:** rota la caja · **Clic:** abre/cierra la tapa · **Alt + arrastrar:** mueve la ventana.
- **Bandeja:** Ajustes, Reposicionar, Recargar arte, Salir.
- **Spotify:** creá una app en <https://developer.spotify.com/dashboard>, agregá el Redirect URI `http://127.0.0.1:47862/callback`, y pegá el Client ID en Ajustes → Spotify.
- **Claves opcionales** de Fanart.tv y Discogs en Ajustes para mejores resultados de discos.

## Compilar

Requisitos: Node LTS, Rust estable; Windows: Visual Studio Build Tools (C++); Linux: `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev libdbus-1-dev`.

```sh
npm install
npm run tauri dev
npm run tauri build
```

`src/render/jewelcase.ts` es **generado** por `npm run render` a partir de `demo/jewelcase-demo.html` y `tools/render/*.part.ts`. No lo edites a mano ni cambies los números de línea de la demo sin actualizar `tools/render/build.mjs`.

## Estructura

- `src/` frontend (Vite + TS + Three.js 0.158) · `src-tauri/` backend Rust (resolvedor de discos, Spotify, captura, bandeja)
- `demo/` referencia visual · `prototype/` prototipo Python del resolvedor · `docs/` sitio web (GitHub Pages)

## Aviso

No está afiliado a Spotify. Portadas y discos pertenecen a sus dueños y se consultan en servicios públicos. Licencia [MIT](LICENSE).
