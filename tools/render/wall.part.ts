
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

