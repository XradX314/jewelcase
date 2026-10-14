// Genera src/render/jewelcase.ts: copia secciones de la demo aprobada (demo/jewelcase-demo.html, por número de línea) y las une con
// las partes propias de esta carpeta (header, wall, footer). Los desvíos respecto de la demo están acá, cada uno comentado.
// Uso, desde la raíz del proyecto:  node tools/render/build.mjs
// IMPORTANTE: src/render/jewelcase.ts es generado; no se edita a mano (se pisa la próxima vez).
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..', '..');
const part = name => readFileSync(join(here, name), 'utf8');
const demo = readFileSync(join(root, 'demo', 'jewelcase-demo.html'), 'utf8').split(/\r?\n/);
const L = (a, b) => demo.slice(a - 1, b).join('\n'); // líneas 1-indexadas, inclusive
const check = (s, from, to) => {
  if (!s.includes(from)) throw new Error('no encontré: ' + from);
  return s.replace(from, to);
};

let env = L(198, 221); // entorno de estudio (luces como paneles)
let body = L(257, 762); // materiales, geometría, CD, tapa, papel, rayones

// Único cambio "técnico" dentro del cuerpo copiado: el estilo del disco viene del estado, no del DOM
body = check(body, "drawDisc($('discStyle').value)", 'drawDisc(S.discStyle)');

// Iluminación de la tinta: la serigrafía procedural conserva la de la demo; las fotos reales del CD ya traen su luz y, con la de la
// demo, salen ~40 % más oscuras, así que usan una iluminación casi constante con variación suave y tope.
const INKL = `vec3 inkLight(vec3 irr, float ph){ vec3 proc = irr*1.7 + 0.05; float L = 0.8 + dot(irr, vec3(0.3333))*0.72; L = L/(1.0 + max(L-1.0,0.0)*1.5); return mix(proc, vec3(L), step(0.5, ph)); }
`;

// Desvío: el papel real (fotos de portadas) se veía apagado, así que se le sube la ganancia de entorno.
body = check(body, 'envMapIntensity:0.7', 'envMapIntensity:PAPER_GAIN');
body = 'const PAPER_GAIN = 2.2;\nconst INK_GAIN_PHOTO = 1;\nconst inkGainU = {value:0};\n' + body;

// Desvío: tinta de fotos reales del CD (ver INKL)
body = check(body, 'uniform sampler2D uArt; uniform sampler2D uMarks;', 'uniform float uInkGain; uniform sampler2D uArt; uniform sampler2D uMarks;');
body = check(body, 'vec3 inkLit = ink.rgb*(irr*1.7 + 0.05) + env*F*0.4;', 'vec3 inkLit = ink.rgb*inkLight(irr, uInkGain) + env*F*0.4;');
body = check(body, 'vec3 inkLit = ink.rgb*(irr*1.7 + 0.05) + refl*F*0.4;', 'vec3 inkLit = ink.rgb*inkLight(irr, uInkGain) + refl*F*0.4;');
body = check(body, 'uniform sampler2D uInk; uniform float uInkOn;', 'uniform sampler2D uInk; uniform float uInkOn; uniform float uInkGain;');
body = check(body, 'uInk:{value:mesh.userData.ink||blankTex}, uInkOn:{value:mesh.userData.ink?1:0} }', 'uInk:{value:mesh.userData.ink||blankTex}, uInkOn:{value:mesh.userData.ink?1:0}, uInkGain:inkGainU }');
body = check(body, 'uniforms:{ uArt:{value:discTex}', 'uniforms:{ uInkGain:inkGainU, uArt:{value:discTex}');
body = check(body, "const c=discCanvas.getContext('2d'), S=1024, cx=S/2, k=S/120;\n  c.clearRect(0,0,S,S);", "const c=discCanvas.getContext('2d'), S=1024, cx=S/2, k=S/120;\n  inkGainU.value = (style==='upload' && discImg) ? INK_GAIN_PHOTO : 0;\n  c.clearRect(0,0,S,S);");

// Desvío: el reflejo del softbox principal en el vidrio (intensidad 6) se sumaba sin límite y dejaba la caja en blanco plano.
// Codo suave: por debajo de 0,8 no cambia nada; por encima se comprime (y se baja 15 %).
body = check(body, 'vec3 refl = textureLod(uEnv, Rw, lodR).rgb;', 'vec3 refl = textureLod(uEnv, Rw, lodR).rgb; refl = refl/(1.0 + max(refl-0.8,0.0)*0.8)*0.85;');
body = check(body, 'vec3 spectral(float w){', INKL + 'vec3 spectral(float w){');
body = check(body, 'vec2 scr(vec3 q){', INKL + 'vec2 scr(vec3 q){');

// Modo oscuro de la lista de canciones del librito: los colores de drawPage salen de una paleta clara u oscura
body = 'let paperDark = false;\nconst PAGE_LIGHT = {bg:\'#f4f1ea\', ink:\'#1d1a22\', mute:\'#6a6572\', hl:\'#d9eef4\', hlText:\'#0a3a48\', num:\'#8a8592\'};\nconst PAGE_DARK = {bg:\'#15151a\', ink:\'#ece8e0\', mute:\'#9b97a4\', hl:\'#22363f\', hlText:\'#a9dceb\', num:\'#6e6a78\'};\n' + body;
{
  const a = body.indexOf('function drawPage(){'), b = body.indexOf('const PAGE_Y0=', a);
  if (a < 0 || b < 0) throw new Error('no encontré drawPage');
  let fn = body.slice(a, b);
  const map = { "'#f4f1ea'": 'PG.bg', "'#1d1a22'": 'PG.ink', "'#6a6572'": 'PG.mute', "'#d9eef4'": 'PG.hl', "'#0a3a48'": 'PG.hlText', "'#8a8592'": 'PG.num' };
  for (const [k, v] of Object.entries(map)) {
    if (!fn.includes(k)) throw new Error('drawPage sin ' + k);
    fn = fn.split(k).join(v);
  }
  fn = fn.replace('const c=pageC.ctx, S=1024, PW=pageC.c.width;', 'const c=pageC.ctx, S=1024, PW=pageC.c.width; const PG = paperDark ? PAGE_DARK : PAGE_LIGHT;');
  body = body.slice(0, a) + fn + body.slice(b);
}

// Respaldo sin imagen: la portada y la serigrafía generadas usan el álbum actual (en la demo tenían texto fijo de ejemplo)
body = check(body, "c.fillText('ONDAS',70,150); c.fillText('LENTAS',70,236);",
  "const tw=ALBUM.title.toUpperCase().split(/\\s+/), half=Math.ceil(tw.length/2), l1=tw.length>1?tw.slice(0,half).join(' '):tw[0], l2=tw.length>1?tw.slice(half).join(' '):'';\n  const fs=Math.round(Math.min(86, 900/(Math.max(l1.length,l2.length,5)*0.66))); c.font='700 '+fs+'px Chivo, sans-serif';\n  c.fillText(l1,70,150); if(l2) c.fillText(l2,70,150+fs);");
body = check(body, "c.fillText('MAREA BAJA',74,290);", "c.fillText(ALBUM.artist.toUpperCase(),74,290);");
body = check(body, "`MAREA BAJA · ℗ ${ALBUM.year}`", "`${ALBUM.artist.toUpperCase()} · ℗ ${ALBUM.year||''}`");
body = check(body, "arcText(c, 'MAREA BAJA', cx,cx, 28.5*k", "arcText(c, ALBUM.artist.toUpperCase(), cx,cx, 28.5*k");
body = check(body, "'MAREA BAJA — ONDAS LENTAS'", "(ALBUM.artist+' — '+ALBUM.title).toUpperCase()");
body = check(body, "'DIGITAL AUDIO · ℗ 1998'", "'DIGITAL AUDIO · ℗ '+(ALBUM.year||'')");

// "Foto real del CD" sin foto disponible: se ve como "Parcial" (sin tocar el ajuste del usuario)
body = check(body, "if(style==='full' || (style==='upload' && !discImg)){", "if(style==='upload' && !discImg) style='partial';\n  if(style==='full'){");

if (/\$\(/.test(body) || /\$\(/.test(env)) throw new Error('quedó un $( (lectura del DOM) en el cuerpo copiado');
const out = [part('header.part.ts'), env, part('wall.part.ts'), body, part('footer.part.ts')].join('\n');
writeFileSync(join(root, 'src', 'render', 'jewelcase.ts'), out);
console.log('ok', out.split('\n').length, 'líneas');
