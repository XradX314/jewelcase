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


/* ---------------- Entorno de estudio (luces como paneles) ---------------- */
const LIGHTS = [ // dirección hacia la luz (mundo), intensidad para paneles, color relativo
  {d:[-0.62,0.52,0.6], w:34, h:22, i:6.0, c:[1.0,0.97,0.93]}, // principal
  {d:[0.82,0.06,0.57], w:6,  h:44, i:5.0, c:[0.93,0.97,1.0]}, // tira lateral
  {d:[0.02,0.92,0.38], w:26, h:26, i:2.6, c:[1.0,1.0,1.0]},   // cenital
  {d:[0.18,0.28,-0.94],w:42, h:8,  i:4.0, c:[0.95,0.98,1.0]}  // contraluz
];
LIGHTS.forEach(l=>{const v=new THREE.Vector3(...l.d).normalize(); l.v=v;});
const envScene = new THREE.Scene();
envScene.add(new THREE.Mesh(new THREE.BoxGeometry(100,100,100), new THREE.MeshBasicMaterial({color:new THREE.Color(0.075,0.08,0.09), side:THREE.BackSide})));
{ const floor = new THREE.Mesh(new THREE.PlaneGeometry(100,100), new THREE.MeshBasicMaterial({color:new THREE.Color(0.17,0.16,0.15)}));
  floor.rotation.x = -Math.PI/2; floor.position.y = -49; envScene.add(floor); }
LIGHTS.forEach(l=>{
  const m = new THREE.Mesh(new THREE.PlaneGeometry(l.w,l.h), new THREE.MeshBasicMaterial({color:new THREE.Color(l.c[0]*l.i,l.c[1]*l.i,l.c[2]*l.i), side:THREE.DoubleSide}));
  m.position.copy(l.v).multiplyScalar(45); m.lookAt(0,0,0); envScene.add(m);
});
const pmrem = new THREE.PMREMGenerator(renderer);
scene.environment = pmrem.fromScene(envScene, 0.02).texture;
const cubeRT = new THREE.WebGLCubeRenderTarget(1024,{type:THREE.HalfFloatType, generateMipmaps:true, minFilter:THREE.LinearMipmapLinearFilter});
const cubeCam = new THREE.CubeCamera(0.5, 200, cubeRT);
cubeCam.update(renderer, envScene);
const lightDirs = LIGHTS.map(l=>l.v.clone());
const lightCols = [new THREE.Vector3(1,0.97,0.93), new THREE.Vector3(0.8,0.84,0.88), new THREE.Vector3(0.45,0.45,0.45), new THREE.Vector3(0.65,0.68,0.7)];


/* ---------------- Fondo real (solo en los render targets) ----------------
   Un quad en espacio de pantalla que muestra el recorte [u0,v0,du,dv] de una textura del tamaño del monitor.
   Vive en WALL_LAYER: los pases de refracción lo incluyen y el pase final no, así el canvas queda transparente. */
const wallMat = new THREE.ShaderMaterial({
  uniforms:{ uMap:{value:null}, uRect:{value:new THREE.Vector4(0,0,1,1)} },
  vertexShader:`varying vec2 vUv; void main(){ vUv=uv; gl_Position=vec4(position.xy,0.9999,1.0); }`,
  fragmentShader:`uniform sampler2D uMap; uniform vec4 uRect; varying vec2 vUv;
    void main(){ gl_FragColor = vec4(texture2D(uMap, uRect.xy + vUv*uRect.zw).rgb, 1.0); }`,
  depthWrite:false, depthTest:false });
const wall = new THREE.Mesh(new THREE.PlaneGeometry(2,2), wallMat);
wall.frustumCulled = false; wall.renderOrder = -10; wall.layers.set(WALL_LAYER);
scene.add(wall);
let wallTex = null, liveTex = null, live = false;
let wallRect = [0,0,1,1];
function setWall(srcCanvas, rect){
  if(!wallTex || wallTex.image !== srcCanvas){
    if(wallTex) wallTex.dispose();
    wallTex = new THREE.CanvasTexture(srcCanvas); wallTex.colorSpace = THREE.SRGBColorSpace;
    wallTex.generateMipmaps = false; wallTex.minFilter = THREE.LinearFilter; wallTex.magFilter = THREE.LinearFilter;
    if(!live) wallMat.uniforms.uMap.value = wallTex;
  }
  wallTex.needsUpdate = true;
  if(rect){ wallRect = rect; if(!live) wallMat.uniforms.uRect.value.set(rect[0],rect[1],rect[2],rect[3]); }
}
function setWallRect(rect){ wallRect = rect; if(!live) wallMat.uniforms.uRect.value.set(rect[0],rect[1],rect[2],rect[3]); }
/* Captura en vivo de lo de atrás (ventanas incluidas): RGBA de abajo hacia arriba, del tamaño exacto de la ventana. */
function setLiveFrame(w, h, rgba){
  if(!liveTex || liveTex.image.width!==w || liveTex.image.height!==h){
    if(liveTex) liveTex.dispose();
    liveTex = new THREE.DataTexture(new Uint8Array(w*h*4), w, h, THREE.RGBAFormat); liveTex.colorSpace = THREE.SRGBColorSpace;
    liveTex.generateMipmaps = false; liveTex.minFilter = THREE.LinearFilter; liveTex.magFilter = THREE.LinearFilter;
  }
  liveTex.image.data.set(rgba); liveTex.needsUpdate = true;
  live = true; wallMat.uniforms.uMap.value = liveTex; wallMat.uniforms.uRect.value.set(0,0,1,1);
}
function useWallpaper(){ live = false; if(wallTex) wallMat.uniforms.uMap.value = wallTex; wallMat.uniforms.uRect.value.set(wallRect[0],wallRect[1],wallRect[2],wallRect[3]); }
function coverDraw(c,img,x,y,w,h){ const s=Math.max(w/img.width,h/img.height); const iw=img.width*s, ih=img.height*s; c.drawImage(img, x+(w-iw)/2, y+(h-ih)/2, iw, ih); }


let paperDark = false;
const PAGE_LIGHT = {bg:'#f4f1ea', ink:'#1d1a22', mute:'#6a6572', hl:'#d9eef4', hlText:'#0a3a48', num:'#8a8592'};
const PAGE_DARK = {bg:'#15151a', ink:'#ece8e0', mute:'#9b97a4', hl:'#22363f', hlText:'#a9dceb', num:'#6e6a78'};
const PAPER_GAIN = 2.2;
const INK_GAIN_PHOTO = 1;
const inkGainU = {value:0};
/* ---------------- Materiales ---------------- */
// Plástico de la caja: shader propio. Para cada píxel sigue el rayo dentro de la pieza (una caja de medidas conocidas),
// calcula cuánto plástico atraviesa, si sale o rebota por reflexión total interna, y lee lo que hay detrás desviado.
const glass = new THREE.MeshBasicMaterial();   // marcador; se reemplaza por el shader al terminar de armar la caja
const viewProj = new THREE.Matrix4();
const glassVert = `
varying vec3 vLP; varying vec3 vLN; varying vec3 vWP; varying vec3 vWN;
void main(){ vLP=position; vLN=normal; vec4 wp=modelMatrix*vec4(position,1.0); vWP=wp.xyz; vWN=normalize(mat3(modelMatrix)*normal); gl_Position=projectionMatrix*viewMatrix*wp; }`;
const glassFrag = `
uniform mat4 modelMatrix;
uniform sampler2D uBehind; uniform samplerCube uEnv; uniform mat4 uViewProj;
uniform vec3 uHalf; uniform vec3 uCenter; uniform float uIor; uniform float uAbsorb; uniform float uDepth; uniform float uRib; uniform sampler2D uEng; uniform float uEngOn; uniform sampler2D uInk; uniform float uInkOn; uniform float uInkGain;
varying vec3 vLP; varying vec3 vLN; varying vec3 vWP; varying vec3 vWN;
vec3 inkLight(vec3 irr, float ph){ vec3 proc = irr*1.7 + 0.05; float L = 0.8 + dot(irr, vec3(0.3333))*0.72; L = L/(1.0 + max(L-1.0,0.0)*1.5); return mix(proc, vec3(L), step(0.5, ph)); }
vec2 scr(vec3 q){ vec4 c=uViewProj*vec4(q,1.0); return clamp(c.xy/c.w*0.5+0.5, 0.001, 0.999); }
void main(){
  mat3 M = mat3(modelMatrix); float sc = length(M[0]); mat3 Mi = transpose(M)/(sc*sc);
  vec3 Vw = normalize(cameraPosition - vWP);
  vec3 nL = normalize(vLN);
  if(uRib > 0.5){ float aa = clamp(1.0 - fwidth(vLP.x)*7.85/3.14159, 0.0, 1.0); nL = normalize(nL + vec3(0.35*aa*cos(vLP.x*7.85), 0.0, 0.0)); }   // estrías finas, se apagan si son más chicas que un píxel
  vec3 dL = normalize(Mi*(-Vw));
  if(dot(nL,dL) > 0.0) nL = -nL;
  vec3 Nw = normalize(M*nL);
  vec3 t = refract(dL, nL, 1.0/uIor);
  vec3 p = vLP - uCenter;
  vec3 s = vec3(t.x>=0.0?1.0:-1.0, t.y>=0.0?1.0:-1.0, t.z>=0.0?1.0:-1.0);
  vec3 dist = (s*uHalf - p)/(t + s*1e-4);
  float L = max(min(min(dist.x,dist.y),dist.z), 0.0);            // mm de plástico recorridos
  vec3 nOut = (dist.x<dist.y && dist.x<dist.z) ? vec3(s.x,0.0,0.0) : ((dist.y<dist.z) ? vec3(0.0,s.y,0.0) : vec3(0.0,0.0,s.z));
  // salida refractada y salida por reflexión total interna, mezcladas con una transición suave (sin escalones de píxel)
  float ct = dot(t,nOut);
  float kk = 1.0 - uIor*uIor*(1.0 - ct*ct);
  float tirW = 1.0 - smoothstep(-0.04, 0.06, kk);
  vec3 outR = kk > 0.0 ? refract(t, -nOut, uIor) : t;
  vec3 outI = reflect(t, nOut);
  vec3 exitW = (modelMatrix*vec4(p + t*L + uCenter, 1.0)).xyz;
  vec3 outRW = normalize(M*outR), outIW = normalize(M*outI);
  vec3 k = vec3(0.30, 0.10, 0.14)*uAbsorb + uAbsorb*0.12;        // el poliestireno absorbe más rojo: cantos verde azulados
  vec3 transR = texture2D(uBehind, scr(exitW + outRW*uDepth)).rgb*exp(-k*L);
  vec3 behindI = texture2D(uBehind, scr(exitW + outIW*uDepth)).rgb;
  vec3 ie = textureLod(uEnv, outIW, 2.0).rgb;
  vec3 transI = mix(behindI*exp(-k*L*2.2), ie*0.75 + behindI*0.18, 0.75);
  vec3 trans = mix(transR, transI, tirW);
  float NV = clamp(dot(Nw,Vw),0.0,1.0);
  float f0 = pow((uIor-1.0)/(uIor+1.0),2.0); float F = f0 + (1.0-f0)*pow(1.0-NV,5.0);
  vec3 Rw = reflect(-Vw,Nw);
  float lodR = clamp(log2(length(fwidth(Rw))*512.0 + 1.0), 0.0, 7.0);   // filtra el reflejo según cuánto cambia entre píxeles
  vec3 refl = textureLod(uEnv, Rw, lodR).rgb; refl = refl/(1.0 + max(refl-0.8,0.0)*0.8)*0.85;
  F = clamp(F*1.6 + 0.015, 0.0, 1.0);                               // un poco más de brillo de superficie para que se lea la forma
  vec3 col = trans*(1.0-F) + refl*F;
  if(uEngOn > 0.5){   // grabado semitransparente en el centro del CD: el plástico texturizado dispersa la luz
    float e = texture2D(uEng, vLP.xy/40.0 + 0.5).r;
    vec3 frost = trans*0.35 + textureLod(uEnv, Nw, 5.0).rgb*0.6 + vec3(0.30,0.31,0.33);
    col = mix(col, frost, e*0.75);
  }
  if(uInkOn > 0.5){   // serigrafía que llega al centro: tinta opaca sobre el policarbonato (de atrás se ve el dorso de la tinta)
    vec4 ink = texture2D(uInk, vLP.xy/120.0 + 0.5);
    vec3 irr = textureLod(uEnv, Nw, 6.0).rgb;
    float top = normalize(vLN).z;
    vec3 inkLit = ink.rgb*inkLight(irr, uInkGain) + refl*F*0.4;
    vec3 back = mix(vec3(0.75), ink.rgb, 0.35)*(irr*1.2 + 0.04);
    float a = ink.a*step(0.5, abs(top));
    col = mix(col, top > 0.0 ? inkLit : back, a);
  }
  gl_FragColor = vec4(col, 1.0);
  #include <colorspace_fragment>
}`;
const glassMats = [];
const blankTex = new THREE.DataTexture(new Uint8Array([0,0,0,255]),1,1); blankTex.needsUpdate=true;
function makeGlassMat(mesh){
  mesh.geometry.computeBoundingBox(); const bb = mesh.geometry.boundingBox;
  const m = new THREE.ShaderMaterial({vertexShader:glassVert, fragmentShader:glassFrag, uniforms:{
    uBehind:{value:null}, uEnv:{value:cubeRT.texture}, uViewProj:{value:viewProj},
    uHalf:{value:bb.getSize(new THREE.Vector3()).multiplyScalar(0.5)}, uCenter:{value:bb.getCenter(new THREE.Vector3())},
    uIor:{value:1.59}, uAbsorb:{value:0.078}, uDepth:{value:30}, uRib:{value:mesh.userData.rib?1:0}, uEng:{value:mesh.userData.eng||blankTex}, uEngOn:{value:mesh.userData.eng?1:0}, uInk:{value:mesh.userData.ink||blankTex}, uInkOn:{value:mesh.userData.ink?1:0}, uInkGain:inkGainU }});
  glassMats.push(m); return m;
}
const edgeMat = new THREE.MeshStandardMaterial({color:0xc4c9d1, roughness:0.2, metalness:0, envMapIntensity:0.8});

/* ---------------- Geometría auxiliar ---------------- */
function rrShape(w,h,r){ const s=new THREE.Shape(), x=-w/2, y=-h/2; r=Math.max(0.01,Math.min(r,w/2-0.01,h/2-0.01));
  s.moveTo(x+r,y); s.lineTo(x+w-r,y); s.quadraticCurveTo(x+w,y,x+w,y+r); s.lineTo(x+w,y+h-r); s.quadraticCurveTo(x+w,y+h,x+w-r,y+h);
  s.lineTo(x+r,y+h); s.quadraticCurveTo(x,y+h,x,y+h-r); s.lineTo(x,y+r); s.quadraticCurveTo(x,y,x+r,y); return s; }
function slab(w,h,t,r=0.8,bev=0.3){ bev=Math.min(bev, w/2-0.05, h/2-0.05, t/2-0.05);
  const g=new THREE.ExtrudeGeometry(rrShape(w-2*bev,h-2*bev,r-bev),{depth:t-2*bev,bevelEnabled:true,bevelThickness:bev,bevelSize:bev,bevelSegments:2,curveSegments:6});
  g.translate(0,0,bev-t/2); return g; }
function put(mesh,x,y,z,parent){ mesh.position.set(x,y,z); parent.add(mesh); return mesh; }

/* Medidas reales en mm. Caja cerrada 142 × 125 × 10,4. z+ = tapa (frente). */
const W=142, H=125, T=10.4;
const root = new THREE.Group(); scene.add(root);           // orientación y tamaño
const caseG = new THREE.Group(); root.add(caseG);           // desplazamiento al abrir
const clickable = [];

/* Base transparente */
const baseG = new THREE.Group(); caseG.add(baseG);
clickable.push(put(new THREE.Mesh(slab(W,H,1.0,1.2,0.35), glass), 0,0,-4.7, baseG));
clickable.push(put(new THREE.Mesh(slab(1.3,H,6.6,0.4,0.25), glass), -70.35,0,-0.9, baseG));
clickable.push(put(new THREE.Mesh(slab(1.3,H,6.6,0.4,0.25), glass),  70.35,0,-0.9, baseG));
// nudillos de la bisagra en la base
for(const y of [-58,58]){ const k=new THREE.Mesh(new THREE.CylinderGeometry(1.6,1.6,7,20), glass); clickable.push(put(k,-69.6,y,-0.6,baseG)); }

/* Bandeja transparente: pozo redondo, roseta de dedos radiales */
const trayG = new THREE.Group(); caseG.add(trayG);
const trayMeshes = [];
function wellPath(P){ P.moveTo(61.6,0); P.absarc(0,0,61.6,0,Math.PI*2,false); return P; }   // pozo redondo, 1,6 mm más grande que el CD
{
  const outer = rrShape(136,118,1.4); outer.holes.push(wellPath(new THREE.Path()));
  const deck = new THREE.ExtrudeGeometry(outer,{depth:0.8,bevelEnabled:true,bevelThickness:0.25,bevelSize:0.25,bevelSegments:2,curveSegments:24});
  deck.translate(0,0,-0.4);
  trayMeshes.push(put(new THREE.Mesh(deck,glass),0,0,0,trayG));
  // faldón perimetral
  trayMeshes.push(put(new THREE.Mesh(slab(136,1.0,4.0,0.3,0.2),glass),0,58.5,-2.2,trayG));
  trayMeshes.push(put(new THREE.Mesh(slab(136,1.0,4.0,0.3,0.2),glass),0,-58.5,-2.2,trayG));
  trayMeshes.push(put(new THREE.Mesh(slab(1.0,118,4.0,0.3,0.2),glass),67.5,0,-2.2,trayG));
  trayMeshes.push(put(new THREE.Mesh(slab(1.0,118,4.0,0.3,0.2),glass),-67.5,0,-2.2,trayG));
  // piso del pozo, con la misma silueta
  const floorG = new THREE.ExtrudeGeometry(wellPath(new THREE.Shape()),{depth:0.6,bevelEnabled:false,curveSegments:48}); floorG.translate(0,0,-0.3);
  trayMeshes.push(put(new THREE.Mesh(floorG,glass),0,0,-3.3,trayG));
  // roseta: anillo, dedos radiales bajo el disco y dientes que entran en el agujero
  const ring = new THREE.Mesh(new THREE.CylinderGeometry(15.8,16.4,0.5,72), glass); ring.rotation.x=Math.PI/2; trayMeshes.push(put(ring,0,0,-2.75,trayG));
  for(let k=0;k<12;k++){ const a=(k+0.5)/12*Math.PI*2;
    const finger=new THREE.Mesh(slab(6.4,1.8,0.7,0.6,0.25),glass); finger.rotation.z=a; trayMeshes.push(put(finger,Math.cos(a)*11,Math.sin(a)*11,-2.65,trayG));
    const tooth=new THREE.Mesh(slab(2.0,1.25,1.7,0.3,0.2),glass); tooth.rotation.z=a; trayMeshes.push(put(tooth,Math.cos(a)*6.6,Math.sin(a)*6.6,-1.35,trayG)); }
  const btn = new THREE.Mesh(new THREE.CylinderGeometry(4.4,4.6,1.4,40), glass); btn.rotation.x=Math.PI/2; trayMeshes.push(put(btn,0,0,-2.1,trayG));
  // la roseta va en su propia capa para verse a través del piso de la bandeja
  [ring,btn].forEach(m=>m.userData.rosette=true); trayG.children.forEach(m=>{ const r=Math.hypot(m.position.x,m.position.y); if(r>0.01 && r<17) m.userData.rosette=true; });   // dedos y dientes
  // nervio del lado de la bisagra, ranuras verticales a la derecha y trabas
  trayMeshes.push(put(new THREE.Mesh(slab(1.2,112,4.4,0.3,0.2),glass),-60.5,0,-2.0,trayG));
  for(const x of [59,62]) trayMeshes.push(put(new THREE.Mesh(slab(0.8,70,0.6,0.3,0.2),glass),x,0,0.75,trayG));
  for(const y of [-46,46]) trayMeshes.push(put(new THREE.Mesh(slab(2.6,6,1.4,0.5,0.25),glass),66.4,y,0.9,trayG));
  trayMeshes.forEach(m=>clickable.push(m));
}

/* CD: shader propio con difracción */
const discCanvas = document.createElement('canvas'); discCanvas.width=discCanvas.height=1024;
const discTex = new THREE.CanvasTexture(discCanvas); discTex.colorSpace=THREE.SRGBColorSpace; discTex.anisotropy=renderer.capabilities.getMaxAnisotropy();
const discVert = `
varying vec2 vP; varying vec3 vWP; varying vec3 vWN; varying vec3 vRad; varying vec3 vTan;
void main(){
  vP = position.xy;
  vec4 wp = modelMatrix*vec4(position,1.0); vWP = wp.xyz;
  mat3 m = mat3(modelMatrix);
  vWN = normalize(m*normal);
  vec2 rd = normalize(position.xy + vec2(1e-5));
  vRad = normalize(m*vec3(rd,0.0));
  vTan = normalize(cross(vWN, vRad));
  gl_Position = projectionMatrix*viewMatrix*wp;
}`;
const discFrag = `
uniform float uInkGain; uniform sampler2D uArt; uniform sampler2D uMarks; uniform samplerCube uEnv; uniform vec3 uL[4]; uniform vec3 uLC[4]; uniform float uFace;
varying vec2 vP; varying vec3 vWP; varying vec3 vWN; varying vec3 vRad; varying vec3 vTan;
vec3 inkLight(vec3 irr, float ph){ vec3 proc = irr*1.7 + 0.05; float L = 0.8 + dot(irr, vec3(0.3333))*0.72; L = L/(1.0 + max(L-1.0,0.0)*1.5); return mix(proc, vec3(L), step(0.5, ph)); }
vec3 spectral(float w){
  float win = smoothstep(380.0,415.0,w)*(1.0-smoothstep(680.0,760.0,w));
  float r = exp(-pow((w-605.0)/48.0,2.0)) + 0.22*exp(-pow((w-420.0)/22.0,2.0));
  float g = exp(-pow((w-540.0)/44.0,2.0));
  float b = exp(-pow((w-455.0)/34.0,2.0));
  return vec3(r,g,b)*win;
}
void main(){
  float r = length(vP);
  if(r < 16.9 || r > 59.2) discard;   // centro y borde: policarbonato transparente
  vec3 N = normalize(vWN); vec3 V = normalize(cameraPosition - vWP);
  if(dot(N,V) < 0.0) N = -N;
  vec3 R = reflect(-V,N);
  float NV = clamp(dot(N,V),0.0,1.0);
  float F = 0.04 + 0.96*pow(1.0-NV,5.0);
  float lodR = clamp(log2(length(fwidth(R))*512.0 + 1.0), 0.0, 7.0);
  vec3 env = textureLod(uEnv, R, lodR).rgb;
  vec3 envSoft = textureLod(uEnv, R, max(3.0,lodR)).rgb;
  vec3 irr = textureLod(uEnv, N, 7.0).rgb;
  float clearHub = 1.0 - smoothstep(16.8,17.3,r);
  float stack = smoothstep(16.8,17.3,r)*(1.0-smoothstep(21.6,22.1,r));
  float data = smoothstep(22.8,23.6,r)*(1.0-smoothstep(57.9,58.3,r));
  float rim = smoothstep(58.2,58.7,r);
  vec3 rad = normalize(vRad); vec3 tg = normalize(vTan);
  // pista de 1,6 um: red de difraccion; ordenes 1..3 a lo largo del radio
  vec3 grat = vec3(0.0);
  for(int i=0;i<4;i++){
    vec3 h = uL[i] + V;
    float along = dot(h,tg);
    float u = abs(dot(h,rad));
    float w = exp(-along*along/0.010);
    vec3 s = spectral(1600.0*u) + 0.6*spectral(800.0*u) + 0.35*spectral(533.3*u);
    grat += s*w*uLC[i];
  }
  // marcas que giran con el disco: R = código de matriz grabado, G = manchas y huellas
  vec2 muv = vec2(uFace > 0.5 ? vP.x : -vP.x, vP.y)/120.0 + 0.5;
  vec4 mk = texture2D(uMarks, muv);
  vec3 metal = (env*0.85 + envSoft*0.35)*vec3(0.90,0.915,0.94) + irr*0.10;
  float smudge = mk.g*(uFace > 0.5 ? 0.35 : 1.0);
  vec3 col = metal*(1.0 - smudge*0.30) + grat*1.5*data*(1.0 - smudge*0.55) + irr*smudge*0.18;
  vec3 clearCol = vec3(0.010) + env*F*0.9 + irr*0.015;
  vec3 stackCol = vec3(0.045) + envSoft*0.22*(0.3+F) + irr*0.07;
  stackCol = mix(stackCol, stackCol*2.2 + envSoft*0.18 + 0.03, mk.r);
  col = mix(col, stackCol, stack);
  col = mix(col, clearCol, clearHub);
  col = mix(col, clearCol*1.3 + irr*0.05, rim);
  if(uFace > 0.5){
    vec4 ink = texture2D(uArt, vP/120.0 + 0.5);
    float a = ink.a*(1.0-clearHub)*(1.0-rim);
    vec3 inkLit = ink.rgb*inkLight(irr, uInkGain) + env*F*0.4;
    col = mix(col, inkLit, a);
  }
  col = col/(1.0 + max(col-0.9,0.0)*0.6);
  gl_FragColor = vec4(col,1.0);
  #include <tonemapping_fragment>
  #include <colorspace_fragment>
}`;
const marksCanvas = document.createElement('canvas'); marksCanvas.width=marksCanvas.height=1024;
const marksTex = new THREE.CanvasTexture(marksCanvas); marksTex.colorSpace=THREE.NoColorSpace;
function drawMarks(){
  const c=marksCanvas.getContext('2d'), S=1024, cx=S/2, k=S/120;
  c.fillStyle='#000'; c.fillRect(0,0,S,S);
  // manchas y una huella parcial en la zona de datos
  c.globalCompositeOperation='lighter'; c.filter='blur(14px)';
  const blob=(a,r,rx,ry,v)=>{ c.fillStyle=`rgba(0,${v},0,1)`; c.beginPath(); c.ellipse(cx+Math.cos(a)*r*k, cx+Math.sin(a)*r*k, rx*k, ry*k, a, 0, Math.PI*2); c.fill(); };
  blob(0.6,40,7,4,70); blob(2.4,48,9,3.5,55); blob(4.1,33,5,5,45); blob(5.3,52,6,2.5,60);
  c.filter='blur(1.2px)'; c.strokeStyle='rgba(0,90,0,1)'; c.lineWidth=2.4;
  const fx=cx+Math.cos(3.3)*44*k, fy=cx+Math.sin(3.3)*44*k;
  for(let i=0;i<14;i++){ c.beginPath(); c.ellipse(fx,fy,(2.2+i*0.55)*k,(1.5+i*0.4)*k,0.7,Math.PI*1.1,Math.PI*2.25); c.stroke(); }
  c.filter='none'; c.globalCompositeOperation='source-over';
  marksTex.needsUpdate=true;
}
drawMarks();
function discMat(face){ return new THREE.ShaderMaterial({ vertexShader:discVert, fragmentShader:discFrag,
  uniforms:{ uInkGain:inkGainU, uArt:{value:discTex}, uMarks:{value:marksTex}, uEnv:{value:cubeRT.texture}, uL:{value:lightDirs}, uLC:{value:lightCols}, uFace:{value:face} } }); }
const discG = new THREE.Group(); caseG.add(discG); discG.position.set(0,0,0);
{
  const ringGeo = new THREE.RingGeometry(7.5,60,160,8);
  const top = new THREE.Mesh(ringGeo, discMat(1)); top.position.z=-1.0; discG.add(top);
  const bot = new THREE.Mesh(ringGeo, discMat(0)); bot.rotation.y=Math.PI; bot.position.z=-2.2; discG.add(bot);
  const edge = new THREE.Mesh(new THREE.CylinderGeometry(60,60,1.2,160,1,true), edgeMat); edge.rotation.x=Math.PI/2; edge.position.z=-1.6; discG.add(edge);
  // centro del CD: policarbonato transparente de 7,5 a 16,9 mm; el agujero (Ø 15 mm) coincide con los dientes de agarre
  const hubShape = new THREE.Shape(); hubShape.absarc(0,0,16.9,0,Math.PI*2,false);
  const hubHole = new THREE.Path(); hubHole.absarc(0,0,7.5,0,Math.PI*2,true); hubShape.holes.push(hubHole);
  const hubGeo = new THREE.ExtrudeGeometry(hubShape,{depth:0.9,bevelEnabled:true,bevelThickness:0.15,bevelSize:0.15,bevelSegments:1,curveSegments:64});
  hubGeo.translate(0,0,-0.45);
  const hub = new THREE.Mesh(hubGeo, glass); hub.position.z=-1.6; discG.add(hub);
  // grabado del centro: código de matriz, país y anillos moldeados (lienzo de 40 mm)
  const engC=document.createElement('canvas'); engC.width=engC.height=1024; const ec=engC.getContext('2d'), ek=1024/40, e0=512;
  ec.fillStyle='#000'; ec.fillRect(0,0,1024,1024);
  // anillo moldeado junto al agujero, con estrías finas y el código grabado pegado al borde interior
  ec.strokeStyle='#fff';
  for(let r=7.75;r<=8.55;r+=0.16){ ec.globalAlpha=0.55; ec.lineWidth=0.06*ek; ec.beginPath(); ec.arc(e0,e0,r*ek,0,Math.PI*2); ec.stroke(); }
  for(const [r,w,a] of [[8.75,0.12,0.9],[10.15,0.12,0.9]]){ ec.globalAlpha=a; ec.lineWidth=w*ek; ec.beginPath(); ec.arc(e0,e0,r*ek,0,Math.PI*2); ec.stroke(); }
  ec.globalAlpha=1;
  arcText(ec,'IFPI L407 · 01', e0,e0, 9.45*ek, -Math.PI*0.5, 30,'700 mono','#fff');
  arcText(ec,'MB-0412-2 A1', e0,e0, 9.45*ek, Math.PI*0.5, 30,'700 mono','#fff', true);
  const engTex=new THREE.CanvasTexture(engC); engTex.colorSpace=THREE.NoColorSpace; engTex.anisotropy=renderer.capabilities.getMaxAnisotropy();
  hub.userData.eng = engTex; hub.userData.ink = discTex;
  [top,bot,edge,hub].forEach(m=>clickable.push(m));
}

/* Tapa (gira sobre la bisagra izquierda) */
const PIVOT = new THREE.Vector3(-69.6,0,-0.6);
const lidPivot = new THREE.Group(); lidPivot.position.copy(PIVOT); caseG.add(lidPivot);
const lidG = new THREE.Group(); lidG.position.copy(PIVOT).multiplyScalar(-1); lidPivot.add(lidG);
const lidMeshes = [];
lidMeshes.push(put(new THREE.Mesh(slab(W,H,1.2,1.2,0.4), glass), 0,0,4.6, lidG));
lidMeshes.push(put(new THREE.Mesh(slab(140.6,1.2,6.4,0.4,0.25), glass), 0.6, 61.9, 1.9, lidG));
lidMeshes.push(put(new THREE.Mesh(slab(140.6,1.2,6.4,0.4,0.25), glass), 0.6,-61.9, 1.9, lidG));
lidMeshes.push(put(new THREE.Mesh(slab(1.0,120,2.0,0.3,0.2), glass), 70.5, 0, 3.0, lidG));
for(const [x,y] of [[64,38],[64,-38],[-62,38],[-62,-38]]) lidMeshes.push(put(new THREE.Mesh(slab(3.2,7,0.9,0.4,0.2), glass), x,y,3.0, lidG));
for(const y of [-62,62]){ const pin=new THREE.Mesh(new THREE.CylinderGeometry(1.1,1.1,2.4,16), glass); lidMeshes.push(put(pin,-69.6,y,-0.6,lidG)); }
lidMeshes.forEach(m=>clickable.push(m));
lidMeshes[1].userData.rib = lidMeshes[2].userData.rib = 1;
const glassMeshes = [];
caseG.traverse(o=>{ if(o.isMesh && o.material===glass){
  o.material = makeGlassMat(o);
  let h='base'; for(let q=o;q;q=q.parent){ if(q===lidG){h='lid';break;} if(q===trayG){h=o.userData.rosette?'rosette':'tray';break;} if(q===discG){h='hub';break;} }
  o.userData.half=h; glassMeshes.push(o); } });

/* ---------------- Papel: librito e inlay ---------------- */
const paperMat = tex => new THREE.MeshStandardMaterial({map:tex, roughness:0.62, metalness:0, envMapIntensity:PAPER_GAIN});
function mkCanvas(w,h){ const c=document.createElement('canvas'); c.width=w; c.height=h; const t=new THREE.CanvasTexture(c); t.colorSpace=THREE.SRGBColorSpace; t.anisotropy=renderer.capabilities.getMaxAnisotropy(); return {c,t,ctx:c.getContext('2d')}; }
const artC = mkCanvas(1024,1024);          // arte cuadrado del álbum (portada)
const BW=139, BH=122;                      // librito: cubre toda la cara interna de la tapa
const bookC = mkCanvas(Math.round(1024*BW/BH),1024);   // frente del librito
const pageC  = mkCanvas(Math.round(1024*BW/BH),1024);   // interior del librito: lista
const inlayC = mkCanvas(1510,1180);         // dorso + lomos
const trayCardC = mkCanvas(512,512);        // cara interna del inlay
const paperG = new THREE.Group();
const bookFront = new THREE.Mesh(new THREE.PlaneGeometry(BW,BH), paperMat(bookC.t)); bookFront.position.set(1.0,0,3.95); lidG.add(bookFront);
const bookBack  = new THREE.Mesh(new THREE.PlaneGeometry(BW,BH), paperMat(pageC.t)); bookBack.rotation.y=Math.PI; bookBack.position.set(1.0,0,3.85); lidG.add(bookBack);
clickable.push(bookFront, bookBack);
const inlayBackTex = inlayC.t;
const inlayBack = new THREE.Mesh(new THREE.PlaneGeometry(138,118), paperMat(inlayBackTex)); inlayBack.rotation.y=Math.PI; inlayBack.position.set(0,0,-4.15); paperG.add(inlayBack);
const inlayInner = new THREE.Mesh(new THREE.PlaneGeometry(138,118), paperMat(trayCardC.t)); inlayInner.position.set(0,0,-4.12); paperG.add(inlayInner);
function spineTex(u0,u1){ const t=inlayC.t.clone(); t.needsUpdate=true; t.repeat.set(u1-u0,1); t.offset.set(u0,0); return t; }
const spineL = new THREE.Mesh(new THREE.PlaneGeometry(6.5,118), paperMat(null)); spineL.rotation.y=-Math.PI/2; spineL.position.set(-69.3,0,-0.95); paperG.add(spineL);
const spineR = new THREE.Mesh(new THREE.PlaneGeometry(6.5,118), paperMat(null)); spineR.rotation.y= Math.PI/2; spineR.position.set( 69.3,0,-0.95); paperG.add(spineR);
caseG.add(paperG);
inlayBackTex.repeat.set(1380/1510,1); inlayBackTex.offset.set(65/1510,0);

let coverImg = null;      // imagen subida por el usuario
let discImg = null;
let palette = {dark:'#1b1530', mid:'#5a2a63', light:'#f3d3a8'};

function drawCover(){
  drawArt();
  // frente del librito: el arte cuadrado contra el borde abierto y, del lado de la bisagra, el mismo arte ampliado y desenfocado
  const c=bookC.ctx, bw=bookC.c.width, bh=bookC.c.height;
  c.save(); c.filter='blur(28px) brightness(0.8)'; c.drawImage(artC.c,-60,-60,bw+120,bh+120); c.restore();
  const g=c.createLinearGradient(bw-bh-40,0,bw-bh,0); g.addColorStop(0,'rgba(0,0,0,0)'); g.addColorStop(1,'rgba(0,0,0,.35)');
  c.fillStyle=g; c.fillRect(bw-bh-40,0,40,bh);
  c.drawImage(artC.c, bw-bh, 0, bh, bh);
  bookC.t.needsUpdate=true;
}
function drawArt(){
  const c=artC.ctx, S=1024;
  if(coverImg){ coverDraw(c,coverImg,0,0,S,S); return; }
  const g=c.createLinearGradient(0,0,0,S); g.addColorStop(0,'#151033'); g.addColorStop(0.55,'#5d2b66'); g.addColorStop(0.8,'#d9775b'); g.addColorStop(1,'#f1b678');
  c.fillStyle=g; c.fillRect(0,0,S,S);
  c.fillStyle='rgba(255,222,170,.95)'; c.beginPath(); c.arc(S*0.64,S*0.62,120,0,Math.PI*2); c.fill();
  c.save(); c.beginPath(); c.rect(0,S*0.66,S,S); c.clip();
  c.fillStyle='#1a1030'; c.fillRect(0,S*0.66,S,S);
  for(let i=0;i<26;i++){ const y=S*0.67+i*i*0.55+i*4; c.strokeStyle=`rgba(255,200,150,${0.55-i*0.018})`; c.lineWidth=2+i*0.12; c.beginPath();
    for(let x=0;x<=S;x+=8){ const yy=y+Math.sin(x/(40+i*6)+i)*(2+i*0.4); x?c.lineTo(x,yy):c.moveTo(x,yy); } c.stroke(); }
  c.restore();
  c.fillStyle='#f6ead8'; c.font='700 86px Chivo, sans-serif'; c.textBaseline='alphabetic'; const tw=ALBUM.title.toUpperCase().split(/\s+/), half=Math.ceil(tw.length/2), l1=tw.length>1?tw.slice(0,half).join(' '):tw[0], l2=tw.length>1?tw.slice(half).join(' '):'';
  const fs=Math.round(Math.min(86, 900/(Math.max(l1.length,l2.length,5)*0.66))); c.font='700 '+fs+'px Chivo, sans-serif';
  c.fillText(l1,70,150); if(l2) c.fillText(l2,70,150+fs);
  c.font='400 30px "Chivo Mono", monospace'; c.fillStyle='rgba(246,234,216,.85)'; c.fillText(ALBUM.artist.toUpperCase(),74,290);
  artC.t.needsUpdate=true;
}
function samplePalette(){
  const s=document.createElement('canvas'); s.width=s.height=24; const x=s.getContext('2d'); x.drawImage(artC.c,0,0,24,24);
  const d=x.getImageData(0,0,24,24).data; let r=0,g=0,b=0,n=0, lr=0,lg=0,lb=0,lmax=-1;
  for(let i=0;i<d.length;i+=4){ r+=d[i];g+=d[i+1];b+=d[i+2];n++; const L=d[i]*0.3+d[i+1]*0.6+d[i+2]*0.1; if(L>lmax){lmax=L;lr=d[i];lg=d[i+1];lb=d[i+2];} }
  r/=n; g/=n; b/=n; const hex=(a,b2,c2)=>'#'+[a,b2,c2].map(v=>Math.max(0,Math.min(255,Math.round(v))).toString(16).padStart(2,'0')).join('');
  palette = {dark:hex(r*0.35,g*0.35,b*0.35), mid:hex(r,g,b), light:hex(lr*0.6+255*0.4,lg*0.6+255*0.4,lb*0.6+255*0.4)};
}
function arcText(c,text,cx,cy,r,center,size,font,color,inside){
  c.save(); c.font=`${font.split(' ')[0]} ${size}px ${font.includes('mono')?'"Chivo Mono", monospace':'Chivo, sans-serif'}`; c.fillStyle=color; c.textAlign='center'; c.textBaseline='middle';
  const widths=[...text].map(ch=>c.measureText(ch).width); const total=widths.reduce((a,b)=>a+b,0)+ (text.length-1)*size*0.08;
  let ang = center - (inside?-1:1)*total/2/r;
  [...text].forEach((ch,i)=>{ const w=widths[i]; const a=ang + (inside?-1:1)*w/2/r;
    c.save(); c.translate(cx+Math.cos(a)*r, cy+Math.sin(a)*r); c.rotate(a + (inside?-Math.PI/2:Math.PI/2)); c.fillText(ch,0,0); c.restore();
    ang += (inside?-1:1)*(w+size*0.08)/r; });
  c.restore();
}
function drawDisc(style){
  const c=discCanvas.getContext('2d'), S=1024, cx=S/2, k=S/120;
  inkGainU.value = (style==='upload' && discImg) ? INK_GAIN_PHOTO : 0;
  c.clearRect(0,0,S,S);
  const annulus=(r0,r1)=>{ c.beginPath(); c.arc(cx,cx,r1*k,0,Math.PI*2); c.arc(cx,cx,r0*k,0,Math.PI*2,true); };
  if(style==='upload' && discImg){ c.save(); annulus(7.6,59); c.clip(); coverDraw(c,discImg,0,0,S,S); c.restore(); discTex.needsUpdate=true; return; }   // la foto procesada trae su propia transparencia en el centro
  if(style==='upload' && !discImg) style='partial';
  if(style==='full'){
    c.save(); annulus(22.5,58.6); c.clip(); c.drawImage(artC.c,0,0,S,S); c.restore();
    arcText(c, ALBUM.title.toUpperCase(), cx,cx, 54.2*k, -Math.PI/2, 26,'700','rgba(250,240,225,.95)');
    arcText(c, `${ALBUM.artist.toUpperCase()} · ℗ ${ALBUM.year||''}`, cx,cx, 54.6*k, Math.PI/2, 17,'400 mono','rgba(250,240,225,.85)', true);
  } else if(style==='partial'){
    c.save(); annulus(45,58.6); c.clip(); c.fillStyle=palette.dark; c.fillRect(0,0,S,S);
    const g=c.createRadialGradient(cx,cx,45*k,cx,cx,58.6*k); g.addColorStop(0,'rgba(255,255,255,0.05)'); g.addColorStop(1,'rgba(255,255,255,0)'); c.fillStyle=g; c.fillRect(0,0,S,S); c.restore();
    arcText(c, ALBUM.title.toUpperCase(), cx,cx, 52*k, -Math.PI/2, 34,'700', palette.light);
    arcText(c, ALBUM.tracks.length+' CANCIONES · DIGITAL AUDIO · '+fmt(totalSec), cx,cx, 52*k, Math.PI/2, 17,'400 mono', palette.light, true);
    c.save(); annulus(23.4,24.2); c.fillStyle='rgba(20,18,26,.9)'; c.fill('evenodd'); c.restore();
    arcText(c, ALBUM.artist.toUpperCase(), cx,cx, 28.5*k, -Math.PI/2, 20,'700','rgba(18,16,24,.92)');
  } else {
    arcText(c, (ALBUM.artist+' — '+ALBUM.title).toUpperCase(), cx,cx, 27.5*k, -Math.PI/2, 19,'700','rgba(16,16,20,.9)');
    arcText(c, 'DIGITAL AUDIO · ℗ '+(ALBUM.year||''), cx,cx, 27.5*k, Math.PI/2, 15,'400 mono','rgba(16,16,20,.8)', true);
    c.save(); annulus(55.6,55.9); c.fillStyle='rgba(16,16,20,.6)'; c.fill('evenodd'); c.restore();
  }
  discTex.needsUpdate=true;
}
function drawPage(){
  const c=pageC.ctx, S=1024, PW=pageC.c.width; const PG = paperDark ? PAGE_DARK : PAGE_LIGHT;
  c.fillStyle=PG.bg; c.fillRect(0,0,PW,S);
  c.fillStyle=PG.ink; c.font='700 54px Chivo, sans-serif'; c.textBaseline='alphabetic'; c.fillText(ALBUM.title,80,140);
  c.font='400 26px "Chivo Mono", monospace'; c.fillStyle=PG.mute; c.fillText(`${ALBUM.artist.toUpperCase()} · ${ALBUM.year}`,82,186);
  c.fillStyle=PG.ink; c.fillRect(80,226,PW-160,2);
  ALBUM.tracks.forEach(([name,dur],i)=>{
    const y=PAGE_Y0 + i*PAGE_DY;
    if(i===currentTrack){ c.fillStyle=PG.hl; c.fillRect(64,y-PAGE_DY*0.62,PW-128,PAGE_DY*0.86); }
    c.fillStyle= i===currentTrack ? PG.hlText : PG.num; c.font='500 28px "Chivo Mono", monospace'; c.fillText((i===currentTrack?'▶ ':'')+String(i+1).padStart(2,'0'),82,y);
    c.fillStyle=PG.ink; c.font=(i===currentTrack?'700 ':'400 ')+'36px Chivo, sans-serif'; c.fillText(name,190,y);
    c.fillStyle=PG.mute; c.font='400 28px "Chivo Mono", monospace'; c.textAlign='right'; c.fillText(dur,PW-82,y); c.textAlign='left';
  });
  c.fillStyle=PG.ink; c.fillRect(80,PAGE_Y0+ALBUM.tracks.length*PAGE_DY-30,PW-160,2);
  c.font='400 24px "Chivo Mono", monospace'; c.fillStyle=PG.mute; c.fillText('TOCÁ UNA CANCIÓN PARA REPRODUCIRLA',82,S-70);
  c.textAlign='right'; c.fillText('TOTAL '+fmt(totalSec),PW-82,S-70); c.textAlign='left';
  pageC.t.needsUpdate=true;
}
const PAGE_Y0=300, PAGE_DY=62;
function drawInlay(){
  const c=inlayC.ctx, Wd=1510, Hd=1180, sp=65;
  c.fillStyle=palette.dark; c.fillRect(0,0,Wd,Hd);
  // lomos
  for(const [x0,flip] of [[0,-1],[Wd-sp,1]]){
    c.fillStyle='rgba(255,255,255,.06)'; c.fillRect(x0,0,sp,Hd);
    c.save(); c.translate(x0+sp/2, Hd/2); c.rotate(flip*Math.PI/2); c.fillStyle=palette.light; c.textAlign='center'; c.textBaseline='middle';
    c.font='700 30px Chivo, sans-serif'; c.fillText(`${ALBUM.artist.toUpperCase()}   ·   ${ALBUM.title.toUpperCase()}`,0,0); c.restore();
  }
  c.fillStyle='rgba(0,0,0,.25)'; c.fillRect(sp-2,0,2,Hd); c.fillRect(Wd-sp,0,2,Hd);
  // dorso
  c.fillStyle=palette.light; c.font='700 52px Chivo, sans-serif'; c.textBaseline='alphabetic'; c.fillText(ALBUM.title, sp+80, 150);
  c.font='400 24px "Chivo Mono", monospace'; c.globalAlpha=0.8; c.fillText(ALBUM.artist.toUpperCase(), sp+82, 192); c.globalAlpha=1;
  ALBUM.tracks.forEach(([n,d],i)=>{ const y=280+i*68; c.font='400 26px "Chivo Mono", monospace'; c.globalAlpha=0.7; c.fillText(String(i+1).padStart(2,'0'),sp+82,y); c.globalAlpha=1;
    c.font='400 34px Chivo, sans-serif'; c.fillText(n,sp+150,y); c.font='400 26px "Chivo Mono", monospace'; c.globalAlpha=0.7; c.fillText(d,sp+620,y); c.globalAlpha=1; });
  c.drawImage(artC.c, Wd-sp-80-360, 230, 360, 360);
  // código de barras ficticio
  const bx=Wd-sp-80-300, by=Hd-220; c.fillStyle='#f6f2ea'; c.fillRect(bx-14,by-14,328,150); c.fillStyle='#111';
  let r=97; for(let x=0;x<300;){ r=(r*9301+49297)%233280; const w=1+Math.floor(r/233280*4); if((x/3|0)%2===0) c.fillRect(bx+x,by,w,100); x+=w+1; }
  c.font='400 20px "Chivo Mono", monospace'; c.fillText('7 98000 41230 5',bx+40,by+124);
  c.fillStyle=palette.light; c.globalAlpha=0.6; c.font='400 20px "Chivo Mono", monospace'; c.fillText(`℗ © ${ALBUM.year} · ÁLBUM DE PRUEBA PARA LA DEMO`, sp+82, Hd-70); c.globalAlpha=1;
  inlayC.t.needsUpdate=true;
  spineL.material.map = spineTex(0, sp/Wd); spineR.material.map = spineTex(1-sp/Wd, 1);
  spineL.material.needsUpdate = spineR.material.needsUpdate = true;
  // cara interna (se ve con bandeja transparente)
  const t=trayCardC.ctx; t.fillStyle=palette.mid; t.fillRect(0,0,512,512); t.fillStyle='rgba(0,0,0,.25)'; t.fillRect(0,0,512,512);
  t.fillStyle=palette.light; t.globalAlpha=0.55; t.font='700 30px Chivo, sans-serif'; t.textAlign='center'; t.fillText(ALBUM.title.toUpperCase(),256,262); t.globalAlpha=1; t.textAlign='left';
  trayCardC.t.needsUpdate=true;
}
function rebuildArt(){ drawCover(); samplePalette(); drawDisc(S.discStyle); drawPage(); drawInlay(); }
rebuildArt();

/* ---------------- Rayones procedurales ---------------- */
const WEAR_PPM = 7, WW = Math.round(W*WEAR_PPM), WH = Math.round(H*WEAR_PPM);
function mulberry(a){ return function(){ a|=0; a=a+0x6D2B79F5|0; let t=Math.imul(a^a>>>15,1|a); t=t+Math.imul(t^t>>>7,61|t)^t; return ((t^t>>>14)>>>0)/4294967296; }; }
function makeWear(p, salt){
  const data = new Uint8Array(WW*WH*4);
  for(let i=0;i<data.length;i+=4){ data[i]=128; data[i+1]=128; }
  if(p.count===0 && p.dust===0) return data;
  const rnd = mulberry(p.seed*7919 + salt*104729);
  const splat=(x,y,ang,I)=>{
    const ix=Math.floor(x), iy=Math.floor(y), fx=x-ix, fy=y-iy;
    const cr=Math.round(Math.cos(2*ang)*127+128), cg=Math.round(Math.sin(2*ang)*127+128);
    for(let oy=0;oy<2;oy++) for(let ox=0;ox<2;ox++){
      const px=ix+ox, py=iy+oy; if(px<0||py<0||px>=WW||py>=WH) continue;
      const w=(ox?fx:1-fx)*(oy?fy:1-fy); const v=Math.round(I*w*255); const idx=(py*WW+px)*4;
      if(v>data[idx+2]){ data[idx]=cr; data[idx+1]=cg; data[idx+2]=v; }
    }
  };
  // curva cuadrática en mm, con perfil de intensidad afinado en las puntas
  const stroke=(x0,y0,cx,cy,x1,y1,I)=>{
    const len=Math.hypot(x1-x0,y1-y0)+Math.hypot(cx-x0,cy-y0)*0.2; const steps=Math.max(4,Math.ceil(len*WEAR_PPM*2.2));
    const wob=rnd()*6.28;
    for(let s=0;s<=steps;s++){ const t=s/steps, u=1-t;
      const x=u*u*x0+2*u*t*cx+t*t*x1, y=u*u*y0+2*u*t*cy+t*t*y1;
      const dx=2*u*(cx-x0)+2*t*(x1-cx), dy=2*u*(cy-y0)+2*t*(y1-cy);
      const prof=Math.pow(Math.sin(Math.PI*t),0.55)*(0.75+0.25*Math.sin(t*23+wob));
      splat((x+W/2)*WEAR_PPM,(y+H/2)*WEAR_PPM,Math.atan2(dy,dx),I*prof);
    }
  };
  const I0=p.intensity, nSwirl=Math.round(p.count*p.swirl), nLin=p.count-nSwirl;
  for(let i=0;i<nLin;i++){
    const L=p.len*(0.25+rnd()*1.5), a=rnd()*Math.PI, x0=(rnd()-0.5)*W, y0=(rnd()-0.5)*H;
    const x1=x0+Math.cos(a)*L, y1=y0+Math.sin(a)*L, bend=(rnd()-0.5)*0.3*L;
    stroke(x0,y0,(x0+x1)/2-Math.sin(a)*bend,(y0+y1)/2+Math.cos(a)*bend,x1,y1,I0*(0.35+rnd()*0.65));
  }
  // marcas circulares: grupos de arcos concéntricos, como al limpiar con un trapo
  let made=0;
  while(made<nSwirl){
    const cx=(rnd()-0.5)*W*1.4, cy=(rnd()-0.5)*H*1.4, R=15+rnd()*70, n=3+Math.floor(rnd()*5), a0=rnd()*Math.PI*2;
    for(let j=0;j<n && made<nSwirl;j++,made++){
      const r=R+(rnd()-0.5)*10, span=Math.min(2.4,(p.len/r)*(0.5+rnd()*0.9)), s=a0+(rnd()-0.5)*0.8;
      const segs=Math.max(2,Math.ceil(span/0.35)); const I=I0*(0.3+rnd()*0.5);
      for(let q=0;q<segs;q++){ const A0=s+span*q/segs, A1=s+span*(q+1)/segs, Am=(A0+A1)/2, rm=r/Math.cos((A1-A0)/2);
        const tq=(q+0.5)/segs; stroke(cx+Math.cos(A0)*r,cy+Math.sin(A0)*r,cx+Math.cos(Am)*rm,cy+Math.sin(Am)*rm,cx+Math.cos(A1)*r,cy+Math.sin(A1)*r,I*Math.pow(Math.sin(Math.PI*tq),0.4)); }
    }
  }
  // microrayones
  for(let i=0;i<p.count*2;i++){ const L=0.4+rnd()*2.2, a=rnd()*Math.PI, x0=(rnd()-0.5)*W, y0=(rnd()-0.5)*H;
    stroke(x0,y0,x0+Math.cos(a)*L/2,y0+Math.sin(a)*L/2,x0+Math.cos(a)*L,y0+Math.sin(a)*L,I0*0.28*rnd()); }
  // polvo (canal A)
  const nd=Math.round(p.dust*2200);
  for(let i=0;i<nd;i++){ const x=Math.floor(rnd()*WW), y=Math.floor(rnd()*WH), v=Math.round(110+rnd()*145), big=rnd()<0.08;
    for(let oy=0;oy<(big?2:1);oy++) for(let ox=0;ox<(big?2:1);ox++){ const px=x+ox, py=y+oy; if(px<WW&&py<WH){ const idx=(py*WW+px)*4+3; data[idx]=Math.max(data[idx],v); } } }
  return data;
}
function wearTexture(){ const t=new THREE.DataTexture(new Uint8Array(WW*WH*4), WW, WH, THREE.RGBAFormat);
  t.generateMipmaps=true; t.minFilter=THREE.LinearMipmapLinearFilter; t.magFilter=THREE.LinearFilter; t.anisotropy=renderer.capabilities.getMaxAnisotropy(); t.needsUpdate=true; return t; }
const lidWearTex = wearTexture(), baseWearTex = wearTexture();
const scratchVert = `
varying vec2 vUv; varying vec3 vWP; varying vec3 vN; varying vec3 vT; varying vec3 vB;
void main(){ vUv=uv; vec4 wp=modelMatrix*vec4(position,1.0); vWP=wp.xyz; mat3 m=mat3(modelMatrix);
  vN=normalize(m*normal); vT=normalize(m*vec3(1.0,0.0,0.0)); vB=normalize(m*vec3(0.0,1.0,0.0)); gl_Position=projectionMatrix*viewMatrix*wp; }`;
const scratchFrag = `
uniform sampler2D uS; uniform float uI; uniform vec3 uL[4]; uniform vec3 uLC[4];
varying vec2 vUv; varying vec3 vWP; varying vec3 vN; varying vec3 vT; varying vec3 vB;
void main(){
  vec4 s = texture2D(uS, vUv);
  vec2 d = s.rg*2.0 - 1.0; float m = length(d);
  float k = s.b*min(m*1.15,1.0);
  if(k < 0.003 && s.a < 0.003) discard;
  float th = 0.5*atan(d.y, d.x);
  vec3 t = normalize(cos(th)*vT + sin(th)*vB);
  vec3 N = normalize(vN); vec3 V = normalize(cameraPosition - vWP);
  if(dot(N,V) < 0.0) N = -N;
  vec3 c = vec3(0.0);
  for(int i=0;i<4;i++){
    vec3 L = uL[i]; float nl = dot(N,L); if(nl <= 0.0) continue;
    vec3 Hh = normalize(L+V);
    float a = dot(Hh,t);
    float g = exp(-a*a/0.0036);                // un rayon es un cilindro: brilla cuando H es perpendicular a el
    float near = pow(max(dot(Hh,N),0.0), 3.0);
    c += uLC[i]*(g*k*(0.25+1.6*near));
    c += uLC[i]*s.a*(0.05*nl + 0.9*pow(max(dot(Hh,N),0.0),24.0));
  }
  c += vec3(0.010)*k;
  gl_FragColor = vec4(c*uI, 1.0);
  #include <tonemapping_fragment>
  #include <colorspace_fragment>
}`;
function scratchMat(tex){ return new THREE.ShaderMaterial({vertexShader:scratchVert, fragmentShader:scratchFrag, transparent:true, depthWrite:false, side:THREE.DoubleSide,
  blending:THREE.AdditiveBlending, polygonOffset:true, polygonOffsetFactor:-2, polygonOffsetUnits:-2,
  uniforms:{uS:{value:tex}, uI:{value:1}, uL:{value:lightDirs}, uLC:{value:lightCols}}}); }
const lidScratch = new THREE.Mesh(new THREE.PlaneGeometry(W-0.8,H-0.8), scratchMat(lidWearTex)); lidScratch.position.set(0,0,5.21); lidScratch.renderOrder=5; lidG.add(lidScratch);
const baseScratch = new THREE.Mesh(new THREE.PlaneGeometry(W-0.8,H-0.8), scratchMat(baseWearTex)); baseScratch.rotation.y=Math.PI; baseScratch.position.set(0,0,-5.21); baseScratch.renderOrder=5; baseG.add(baseScratch);

/* ---------------- Control por API ---------------- */
let wearKey = '';
function applyRefraction(on, amt){ glassMats.forEach(m=>m.uniforms.uIor.value = on?(1.15+amt*0.6):1.0); }
function applyPaper(){ paperG.visible = bookFront.visible = bookBack.visible = S.paper; }
applyPaper();

const PRESETS = { new:{count:0,wearI:0,wearLen:22,swirl:30,dust:0}, light:{count:40,wearI:55,wearLen:22,swirl:30,dust:10},
  used:{count:140,wearI:75,wearLen:30,swirl:45,dust:25}, glovebox:{count:360,wearI:95,wearLen:42,swirl:55,dust:55} };
function regenWear(w){
  const p={ count:w.count, intensity:w.wearI/100, len:w.wearLen, swirl:w.swirl/100, dust:w.dust/100, seed:Math.max(1,w.seed||1) };
  lidWearTex.image.data.set(makeWear(p,1)); lidWearTex.needsUpdate=true;
  baseWearTex.image.data.set(makeWear(p,2)); baseWearTex.needsUpdate=true;
  lidScratch.visible = w.surface!=='base'; baseScratch.visible = w.surface!=='lid';
  if(!(p.count>0 || p.dust>0)){ lidScratch.visible=baseScratch.visible=false; }
}

/* cfg: las claves de la config de la app (ver src/shared/config.ts). Solo se rehace lo que cambió. */
function setConfig(c){
  spinPeriod=Math.max(0.3,c.spinSecs); spinDir=c.spinReverse?-1:1; idleFps=Math.max(5,+c.fps||30);
  { const ms=Number(c.msaa); if(Number.isFinite(ms) && ms!==rtSamples){ rtSamples=ms; rts.forEach(rt=>{ rt.samples=ms; rt.dispose(); }); } }
  S.yaw=c.yaw; S.pitch=c.pitch; S.roll=c.roll; S.sway=!!c.sway && !reduceMotion; S.rpm=c.rpm; S.playing = playingOverride ?? c.playing;
  glassMats.forEach(m=>m.uniforms.uAbsorb.value=c.edgeAmt/100*0.2);
  applyRefraction(c.refr, c.refrAmt/100);
  if(+c.quality!==quality){ quality=+c.quality; applyQuality(); resize(); }
  if(S.paper!==!!c.paper){ S.paper=!!c.paper; applyPaper(); }
  if(paperDark!==!!c.paperDark){ paperDark=!!c.paperDark; drawPage(); }
  if(S.discStyle!==c.discStyle){ S.discStyle=c.discStyle; drawDisc(S.discStyle); }
  const key = JSON.stringify([c.count,c.wearI,c.wearLen,c.swirl,c.dust,c.seed,c.surface]);
  if(key!==wearKey){ wearKey=key; regenWear({count:c.count,wearI:c.wearI,wearLen:c.wearLen,swirl:c.swirl,dust:c.dust,seed:c.seed,surface:c.surface}); }
}
function setOpen(o){ S.open=o; }
/* Giro continuo sobre el eje vertical (como un plato giratorio) hasta que se vuelve a tocar el botón; arranca y frena suave.
   Al frenar, el ángulo se incorpora al giro horizontal y se avisa para guardarlo. */
let spinOn=false, spinRate=0, spinYaw=0, spinPeriod=3, spinDir=1;
function toggleBoxSpin(){ spinOn=!spinOn; return spinOn; }
/* El giro del CD puede venir de afuera (Spotify): null = vuelve a mandar la config. */
let playingOverride=null;
function setPlaying(p){ playingOverride=p; if(p!==null) S.playing=p; }
function setOrient(y,p,r){ S.yaw=y; S.pitch=p; S.roll=r; }
function setAlbum(a){ ALBUM=a; calcTotal(); currentTrack=0; rebuildArt(); }
function setCoverImage(img){ coverImg=img; rebuildArt(); }
function setDiscImage(img){ discImg=img; rebuildArt(); }
/* Saca el arte del álbum anterior (sin redibujar: lo hace el setAlbum o el clearArt que sigue). */
function resetArt(){ coverImg=null; discImg=null; }
function clearArt(){ coverImg=null; discImg=null; rebuildArt(); }
function setCurrentTrack(i){ currentTrack=i; drawPage(); }

/* ---------------- Tamaño y cuadro ---------------- */
// suavizado (MSAA) de los 5 render targets de la refracción: es lo que más cuesta de GPU; se puede bajar desde Ajustes
let rtSamples=4;
const rtOpts={type:THREE.HalfFloatType, samples:4};
const rts=[0,1,2,3,4].map(()=>new THREE.WebGLRenderTarget(1,1,rtOpts));
const dbs=new THREE.Vector2();
function resize(){
  const w=Math.max(1,canvas.clientWidth||innerWidth), h=Math.max(1,canvas.clientHeight||innerHeight);
  applyQuality(); renderer.setSize(w,h,false); camera.aspect=w/h;
  renderer.getDrawingBufferSize(dbs); rts.forEach(r=>r.setSize(dbs.x,dbs.y));
  camera.position.z = w/h < 0.8 ? 660*Math.min(1.9, 0.8/(w/h)) : 660;
  camera.updateProjectionMatrix();
}
resize();

/* ---------------- Picking (clic y click-through) ---------------- */
const ray=new THREE.Raycaster(), ndc=new THREE.Vector2();
ray.layers.enableAll();
/* x, y en px CSS relativos al canvas. Devuelve {kind:'box'|'track', index?} o null si no hay objeto. */
function hitTest(x,y){
  const r=canvas.getBoundingClientRect();
  ndc.set((x/r.width)*2-1, -(y/r.height)*2+1);
  ray.setFromCamera(ndc,camera);
  const hits=ray.intersectObjects(clickable.filter(m=>m.visible && (m.parent.visible)),false);
  if(!hits.length) return null;
  const h=hits[0];
  if(h.object===bookBack && S.paper && openT>0.85){
    const yy=(1-h.uv.y)*1024; const i=Math.floor((yy-PAGE_Y0+PAGE_DY*0.62)/PAGE_DY);
    if(i>=0 && i<ALBUM.tracks.length) return {kind:'track', index:i};
  }
  return {kind:'box'};
}

/* ---------------- Bucle ---------------- */
const clock=new THREE.Clock();
const tmpV=new THREE.Vector3(), tmpV2=new THREE.Vector3(), tmpN=new THREE.Vector3(), tmpQ=new THREE.Quaternion();
const deg=THREE.MathUtils.degToRad;
let running=true, rafId=0;
/* Planificación: lo caro es dibujar (5 pasadas con MSAA + la final), no calcular. Solo se dibuja cuando algo cambió o se mueve:
   - interacción y animaciones rápidas (arrastre, tapa, giro de la caja): a la frecuencia de la pantalla;
   - el CD girando o el balanceo suave: a idleFps (30 por defecto), que alcanza y gasta la mitad;
   - todo quieto: nada (el canvas conserva el último cuadro). */
let dirty=true, paused=false, interacting=false, idleFps=30, lastRender=-1e9;
// qué tanto mira la caja de frente (o de atrás) a la cámara: 1 = de frente, 0 = de canto; se actualiza en cada cuadro dibujado
let faceDot=1;
// Con la carátula puesta y la tapa cerrada, el librito tapa el disco por delante y el inlay por detrás: su giro no se ve, así que no
// justifica dibujar. De canto (a más de ~78° del eje) el disco se asoma por los bordes de vidrio y sí se sigue dibujando.
const discHidden = ()=> S.paper && openT<0.01 && faceDot>0.2;
function markDirty(){ dirty=true; }
function frame(now){
  if(!running) return;
  rafId=requestAnimationFrame(frame);
  if(paused) return;
  const fast = interacting || Math.abs((S.open?1:0)-openT)>0.002 || spinOn || Math.abs(spinRate)>0.5;
  const discMoving = (S.playing && S.rpm>0) || Math.abs(spinSpeed)>0.002;
  const slow = (discMoving && !discHidden()) || S.sway;
  if(!fast){
    if(slow){ if(now-lastRender < 1000/idleFps-3) return; }
    else if(!dirty) return;
  }
  lastRender=now; dirty=false;
  const dt=Math.min(clock.getDelta(),0.12);
  const target = S.playing ? S.rpm*Math.PI*2/60 : 0;
  spinSpeed += (target-spinSpeed)*(1-Math.exp(-dt*(S.playing?1.6:1.1)));
  spin -= spinSpeed*dt; discG.rotation.z = spin;            // horario visto desde la etiqueta
  openT += ((S.open?1:0)-openT)*(reduceMotion?1:(1-Math.exp(-dt*4.2)));
  const e = openT*openT*(3-2*openT);
  lidPivot.rotation.y = -deg(168)*e;
  if(S.sway){ swayT+=dt; }
  const sy = S.sway ? Math.sin(swayT*0.45)*7 : 0, sp = S.sway ? Math.sin(swayT*0.31)*3 : 0;
  const spinTarget = spinOn ? 360/spinPeriod*spinDir : 0;
  spinRate += (spinTarget-spinRate)*(1-Math.exp(-dt*(spinOn?2.5:3.5)));
  spinYaw += spinRate*dt;
  if(!spinOn && Math.abs(spinRate)<1 && spinYaw!==0){
    let w=((S.yaw+spinYaw)%360+360)%360; if(w>180) w-=360;
    S.yaw=w; spinYaw=0; spinRate=0; hooks.onSpinStop(S.yaw);
  }
  root.rotation.set(deg(S.pitch+sp), deg(S.yaw+sy+spinYaw), deg(S.roll), 'YXZ');
  faceDot = Math.abs(tmpN.set(0,0,1).applyQuaternion(root.quaternion).z);
  const sc = S.size*(1-0.30*e); root.scale.setScalar(sc);
  caseG.position.x = 70*e;
  scene.updateMatrixWorld(); camera.updateMatrixWorld();
  viewProj.multiplyMatrices(camera.projectionMatrix, camera.matrixWorldInverse);
  // capas de atrás hacia adelante: mitad lejana de la carcasa, bandeja y centro del CD (según el lado), mitad cercana.
  // cada capa de plástico ve una imagen de todo lo que tiene detrás (incluido el plástico de las capas anteriores)
  const zl = tmpV.setFromMatrixPosition(lidMeshes[0].matrixWorld).applyMatrix4(camera.matrixWorldInverse).z;
  const zb = tmpV.setFromMatrixPosition(baseG.children[0].matrixWorld).applyMatrix4(camera.matrixWorldInverse).z;
  const lidFar = zl < zb;
  const order = lidFar ? ['lid','hub','rosette','tray','base'] : ['base','tray','rosette','hub','lid'];   // lejos -> cerca
  const stageOf = {}, texOf = {}; order.forEach((g,i)=>{ stageOf[g]=2+2*i; texOf[2+2*i]=rts[i].texture; });
  for(const m of glassMeshes){ const st=stageOf[m.userData.half]; m.layers.set(st); m.material.uniforms.uBehind.value=texOf[st]; }
  for(const [sm,half] of [[lidScratch,'lid'],[baseScratch,'base']]){
    const g=stageOf[half]; sm.getWorldPosition(tmpV); tmpN.set(0,0,1).applyQuaternion(sm.getWorldQuaternion(tmpQ));
    const facing = tmpN.dot(tmpV2.copy(camera.position).sub(tmpV)) > 0; sm.layers.set(facing ? g+1 : g-1); }
  // los RT incluyen el fondo (WALL_LAYER); el pase final no, así el canvas queda transparente fuera de la caja
  rts.forEach((rt,i)=>{ camera.layers.mask=((1<<(2+2*i))-1)|(1<<WALL_LAYER); renderer.setRenderTarget(rt); renderer.render(scene,camera); });
  camera.layers.mask=(1<<12)-1; renderer.setRenderTarget(null);
  renderer.render(scene,camera);
}
rafId=requestAnimationFrame(frame);

const api = {
  setConfig, setOpen, toggleBoxSpin, isBoxSpinning:()=>spinOn, onSpinStop(cb){ hooks.onSpinStop=cb; }, isOpen:()=>S.open, openAmount:()=>openT, setPlaying, setOrient, getOrient:()=>({yaw:S.yaw,pitch:S.pitch,roll:S.roll}),
  setWall, setWallRect, setLiveFrame, useWallpaper, setAlbum, resetArt, clearArt, setCoverImage, setDiscImage, setCurrentTrack,
  hitTest, resize, PRESETS,
  setPaused(b){ paused=!!b; if(!b){ dirty=true; clock.getDelta(); } },
  setInteracting(b){ interacting=!!b; },
  stop(){ running=false; cancelAnimationFrame(rafId); },
  start(){ if(!running){ running=true; clock.getDelta(); rafId=requestAnimationFrame(frame); } },
};
// todo lo que cambia el estado marca que hay que dibujar; lo que solo consulta, no
const NO_DIRTY = new Set(['isBoxSpinning','isOpen','openAmount','getOrient','hitTest','stop','start','PRESETS','onSpinStop','setPaused','setInteracting']);
for(const k of Object.keys(api)){
  if(typeof api[k]==='function' && !NO_DIRTY.has(k)){ const f=api[k]; api[k]=(...a)=>{ markDirty(); return f(...a); }; }
}
return api;
}
