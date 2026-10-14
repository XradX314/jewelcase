// @ts-nocheck
/* Render del widget JewelCase.
   Porte de demo/jewelcase-demo.html (etapa 1, look aprobado). Las secciones de entorno, materiales, geometría, CD, tapa,
   papel y rayones están copiadas tal cual de la demo. Lo que cambia respecto de ella:
   - canvas transparente (alpha 0);
   - el fondo se dibuja SOLO en los render targets de la refracción (capa WALL_LAYER), nunca en el pase final;
   - el control es por API (setConfig, hitTest...) en vez de leer el DOM. */
import * as THREE from 'three';

const WALL_LAYER = 12;

export function createJewelCase(canvas) {
"use strict";
const reduceMotion = matchMedia('(prefers-reduced-motion: reduce)').matches;
const noop = () => {};
const hooks = { onTracks: noop, onSpinStop: noop };

/* ---------------- Datos del álbum (de prueba hasta que el resolvedor/Spotify traiga el real) ---------------- */
let ALBUM = {
  artist:'Marea Baja', title:'Ondas Lentas', year:1998,
  tracks:[['Muelle Norte','3:42'],['Agua de Fondo','4:15'],['Hora Azul','3:58'],['Corriente','5:02'],['Faro Apagado','4:31'],
          ['Sal y Cobre','3:27'],['Bajante','6:10'],['Isla Chica','2:54'],['Remanso','4:48'],['Última Luz','5:36']]
};
const fmt = s => Math.floor(s/60)+':'+String(s%60).padStart(2,'0');
let totalSec = 0;
const calcTotal = () => { totalSec = ALBUM.tracks.reduce((s,[,d])=>{const [m,x]=String(d).split(':').map(Number);return s+(m||0)*60+(x||0)},0); };
calcTotal();
let currentTrack = 0;

/* ---------------- Estado ---------------- */
const S = { yaw:-22, pitch:14, roll:0, size:1, sway:false, rpm:24, playing:true, paper:false, open:false, discStyle:'partial' };
let spin=0, spinSpeed=0, openT=0, swayT=0;

/* ---------------- Renderer, cámara ---------------- */
const renderer = new THREE.WebGLRenderer({canvas, antialias:true, alpha:true, premultipliedAlpha:true});
renderer.setClearColor(0x000000, 0);
let quality = 1.5;   // supersampling: renderiza más píxeles que la pantalla y los promedia
// el buffer de dibujo no pasa de MAX_BUFFER px de lado: a tamaños grandes (hasta 400 %) 5 render targets MSAA de 3000x3000 usarían >1 GB de GPU
const MAX_BUFFER = 2304;
const applyQuality = ()=> renderer.setPixelRatio(Math.max(0.5, Math.min(Math.max(devicePixelRatio,1)*quality, 3, MAX_BUFFER/Math.max(1, canvas.clientWidth||innerWidth, canvas.clientHeight||innerHeight))));
applyQuality();
renderer.outputColorSpace = THREE.SRGBColorSpace;
renderer.toneMapping = THREE.NoToneMapping;   // todo lo de la caja se ve a través del plástico: sin curva global para que el fondo no cambie
renderer.toneMappingExposure = 1.0;
const scene = new THREE.Scene();
const camera = new THREE.PerspectiveCamera(20, 1, 1, 6000);
camera.position.set(0,0,660);

