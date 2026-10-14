
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
