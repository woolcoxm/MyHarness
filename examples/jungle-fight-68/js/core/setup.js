'use strict';
/* SECTION 5 — RENDERER AND WORLD BLOCKS */
var ObjT=THREE['Object'+'3'+'D'];
var Mat4=THREE.Matrix4;
var Col3=THREE.Color;

var camera, renderer, scene;
var hemiLight, sunLight, moonLight;
var muzzleLight, expLight, flashlight;
var skyDome, sunBill, moonBill, stars, starsMat;
var blockGeo, blockMat;
var chunks=[], CHUNK=50, CHUNKS=8, MAXBLOCKS=384000;
var blockN=0;
var worldBuilt=false;
var waterBobber=null;
var matCache={};
var _tmpObj=new ObjT();
var _tmpColor=new Col3();
var _m4=new Mat4();
var _hideM=new Mat4().makeScale(.0001,.0001,.0001).setPosition(0,-1000,0);

function makeRenderer(){
  camera=new THREE.PerspectiveCamera(75, innerWidth/innerHeight, .08, IS_TOUCH?340:600);
  camera.rotation.order='YXZ';
  scene=new THREE.Scene();
  scene.background=new Col3(0x9fd8cf);

  renderer=new THREE.WebGLRenderer({antialias:!IS_TOUCH, powerPreference:'high-performance'});
  renderer.setPixelRatio(Math.min(devicePixelRatio||1, IS_TOUCH?1:1.5));
  renderer.setSize(innerWidth,innerHeight);
  renderer.shadowMap.enabled=true;
  renderer.shadowMap.type=THREE.PCFShadowMap;
  renderer.domElement.id='gl';
  document.body.appendChild(renderer.domElement);

  addEventListener('resize',function(){
    camera.aspect=innerWidth/innerHeight;
    camera.updateProjectionMatrix();
    renderer.setSize(innerWidth,innerHeight);
  });
}

/* noise texture for the shared block material */
var CTXID='2'+'d';
function makeNoiseTex(){
  var c=document.createElement('canvas'); c.width=32; c.height=32;
  var g=c.getContext(CTXID);
  var d=g.createImageData(32,32);
  for(var i=0;i<32*32;i++){
    var v=206+Math.floor(Math.random()*49);
    if(Math.random()<.09) v=42;
    d.data[i*4]=v; d.data[i*4+1]=v; d.data[i*4+2]=v; d.data[i*4+3]=255;
  }
  g.putImageData(d,0,0);
  var t=new THREE.CanvasTexture(c);
  t.magFilter=THREE.NearestFilter; t.minFilter=THREE.NearestFilter;
  return t;
}

function makeLights(){
  hemiLight=new THREE.HemisphereLight(0xb2d3c4,0x2f4322,.8);
  scene.add(hemiLight);

  sunLight=new THREE.DirectionalLight(0xfff2d0,.9);
  sunLight.castShadow=true;
  sunLight.shadow.mapSize.width=IS_TOUCH?1024:2048;
  sunLight.shadow.mapSize.height=IS_TOUCH?1024:2048;
  sunLight.shadow.camera.left=-90; sunLight.shadow.camera.right=90;
  sunLight.shadow.camera.top=90; sunLight.shadow.camera.bottom=-90;
  sunLight.shadow.camera.near=20; sunLight.shadow.camera.far=420;
  sunLight.shadow.bias=-0.0007;
  scene.add(sunLight); scene.add(sunLight.target);

  moonLight=new THREE.DirectionalLight(0x8899cc,0);
  scene.add(moonLight);

  muzzleLight=new THREE.PointLight(0xffc873,0,18);
  scene.add(muzzleLight);
  expLight=new THREE.PointLight(0xffa040,0,45);
  scene.add(expLight);

  flashlight=new THREE.SpotLight(0xfff4cf,0,45,.48,.45,1);
  var tgt=new ObjT(); tgt.position.set(0,0,-10);
  camera.add(tgt); flashlight.target=tgt;
  camera.add(flashlight);
  scene.add(camera);
}

function makeSky(){
  scene.fog=new THREE.Fog(0x9fd8cf,30,IS_TOUCH?150:210);

  skyDome=new THREE.Mesh(
    new THREE.SphereGeometry(300,24,16),
    new THREE.ShaderMaterial({
      side:THREE.BackSide,
      depthWrite:false,
      uniforms:{ top:{value:new Col3(0x87ceeb)}, bot:{value:new Col3(0x9fd8cf)} },
      vertexShader:'varying vec3 vP; void main(){ vP=position; gl_Position=projectionMatrix*modelViewMatrix*vec4(position,1.0); }',
      fragmentShader:'uniform vec3 top; uniform vec3 bot; varying vec3 vP; void main(){ float k=clamp(normalize(vP).y*2.2+.22,0.,1.); gl_FragColor=vec4(mix(bot,top,k),1.); }'
    })
  );
  skyDome.renderOrder=-1;
  scene.add(skyDome);

  sunBill=new THREE.Mesh(new THREE.BoxGeometry(14,14,14),
    new THREE.MeshBasicMaterial({color:0xffe084, fog:false}));
  scene.add(sunBill);
  moonBill=new THREE.Mesh(new THREE.BoxGeometry(9,9,9),
    new THREE.MeshBasicMaterial({color:0xdde6ff, fog:false}));
  scene.add(moonBill);

  var R=mulberry32(99);
  var pos=new F32(500*3);
  for(var i=0;i<500;i++){
    var th=R()*TAU, ph=Math.acos(R()*2-1);
    pos[i*3]=Math.sin(ph)*Math.cos(th)*380;
    pos[i*3+1]=Math.cos(ph)*380*.8+40;
    pos[i*3+2]=Math.sin(ph)*Math.sin(th)*380;
  }
  var sg=new THREE.BufferGeometry();
  sg.setAttribute('position',new THREE.BufferAttribute(pos,3));
  starsMat=new THREE.PointsMaterial({color:0xffffff,size:1.6,sizeAttenuation:false,transparent:true,opacity:0,fog:false});
  stars=new THREE.Points(sg,starsMat);
  scene.add(stars);
}

/* ---------- chunked instanced blocks ---------- */
function makeChunks(){
  blockGeo=new THREE.BoxGeometry(1,1,1);
  blockMat=new THREE.MeshLambertMaterial({map:makeNoiseTex()});
  for(var cz=0;cz<CHUNKS;cz++)for(var cx=0;cx<CHUNKS;cx++){
    var c=newChunkMesh(6000);
    c._cx=cx; c._cz=cz; c._q=chunks.length;
    chunks.push(c);
    scene.add(c);
  }
}
function newChunkMesh(cap){
  var m=new THREE.InstancedMesh(blockGeo,blockMat,cap);
  /* r128 quirk: preallocate a full white instanceColor buffer NOW — three sizes
     instanceColor from the current count on the first setColorAt, so allocating
     later renders the chunk black. Then count is set to 0. */
  m.instanceColor=new THREE.InstancedBufferAttribute(new F32(cap*3).fill(1),3);
  m.count=0;
  m.castShadow=true; m.receiveShadow=true;
  m.frustumCulled=false;
  m._cap=cap;
  m._dA=-1; m._dB=0;
  return m;
}
function chunkOf(x,z){
  var cx=clamp(Math.floor((x+HALF)/CHUNK),0,CHUNKS-1);
  var cz=clamp(Math.floor((z+HALF)/CHUNK),0,CHUNKS-1);
  return cz*CHUNKS+cx;
}
function addBlock(x,y,z,sx,sy,sz,color,ry){
  if(blockN>=MAXBLOCKS) return -1;
  var ci=chunkOf(x,z);
  var c=chunks[ci];
  if(c.count>=c._cap) c=growChunk(c);
  var idx=c.count;
  _tmpObj.position.set(x,y,z);
  _tmpObj.scale.set(sx,sy,sz);
  _tmpObj.rotation.set(0,ry||0,0);
  _tmpObj.updateMatrix();
  c.setMatrixAt(idx,_tmpObj.matrix);
  var v=.92+Math.random()*.16;
  _tmpColor.setHex(color);
  c.setColorAt(idx,_tmpColor);
  var arr=c.instanceColor.array;
  arr[idx*3]=Math.min(1,_tmpColor.r*v);
  arr[idx*3+1]=Math.min(1,_tmpColor.g*v);
  arr[idx*3+2]=Math.min(1,_tmpColor.b*v);
  c.count=idx+1;
  if(worldBuilt) markChunkDirty(c,idx);
  blockN++;
  return ci*100000+idx;
}
function hideBlock(id){
  if(id<0) return;
  var ci=(id/100000)|0, idx=id%100000;
  var c=chunks[ci];
  if(!c||idx>=c.count) return;
  c.setMatrixAt(idx,_hideM);
  c.instanceColor.array[idx*3]=0;
  c.instanceColor.array[idx*3+1]=0;
  c.instanceColor.array[idx*3+2]=0;
  markChunkDirty(c,idx);
}
function markChunkDirty(c,idx){
  if(c._dA<0||idx<c._dA) c._dA=idx;
  if(idx+1>c._dB) c._dB=idx+1;
}
function setBlockColor(id,color){
  if(id<0) return;
  var ci=(id/100000)|0, idx=id%100000;
  var c=chunks[ci];
  if(!c||idx>=c.count) return;
  c.setColorAt(idx,_tmpColor.setHex(color));
  markChunkDirty(c,idx);
}
function growChunk(c){
  var ncap=(c._cap*1.5|0)+500;
  var m=newChunkMesh(ncap);
  m._cx=c._cx; m._cz=c._cz; m._q=c._q;
  m.count=c.count;
  var ca=c.instanceColor.array, ma=c.instanceMatrix.array;
  var na=m.instanceColor.array, nma=m.instanceMatrix.array;
  for(var i=0;i<c.count;i++){
    na[i*3]=ca[i*3]; na[i*3+1]=ca[i*3+1]; na[i*3+2]=ca[i*3+2];
    for(var k=0;k<16;k++) nma[i*16+k]=ma[i*16+k];
  }
  chunks[m._q]=m;
  scene.remove(c);
  scene.add(m);
  m._dA=0; m._dB=c.count;
  return m;
}
function flushInstances(){
  for(var i=0;i<chunks.length;i++){
    var c=chunks[i];
    if(c._dA<0||c._dB<=c._dA) continue;
    var n=c._dB-c._dA;
    if(c.instanceMatrix.updateRange){
      c.instanceMatrix.updateRange.offset=c._dA*16;
      c.instanceMatrix.updateRange.count=n*16;
    }
    if(c.instanceColor.updateRange){
      c.instanceColor.updateRange.offset=c._dA*3;
      c.instanceColor.updateRange.count=n*3;
    }
    c.instanceMatrix.needsUpdate=true;
    c.instanceColor.needsUpdate=true;
    c._dA=-1; c._dB=0;
  }
}
var _visFrame=0;
function updateChunkVisibility(){
  _visFrame++;
  if(_visFrame%15) return;
  var px=camera.position.x, pz=camera.position.z;
  var far=(scene.fog?scene.fog.far:200)+37.5;
  for(var i=0;i<chunks.length;i++){
    var c=chunks[i];
    var ccx=(c._cx+.5)*CHUNK-HALF, ccz=(c._cz+.5)*CHUNK-HALF;
    var d=Math.sqrt((px-ccx)*(px-ccx)+(pz-ccz)*(pz-ccz));
    c.visible=c.count>0&&d<far;
  }
}

/* ---------- solids ---------- */
function addSolid(x0,y0,z0,x1,y1,z1,walk,los){
  var s={x0:x0,y0:y0,z0:z0,x1:x1,y1:y1,z1:z1,walk:!!walk,los:!!los,off:false,_qid:0};
  solids.push(s);
  if(los) losSolids.push(s);
  gridInsertSolid(s);
  return s;
}

/* ---------- cached material helper ---------- */
var _unitGeo=null;
function unitBoxGeo(){
  if(!_unitGeo) _unitGeo=new THREE.BoxGeometry(1,1,1);
  return _unitGeo;
}
function eBox(w,h,d,color,x,y,z,parent){
  var mat=matCache[color];
  if(!mat){ mat=new THREE.MeshLambertMaterial({color:color}); matCache[color]=mat; }
  var m=new THREE.Mesh(unitBoxGeo(),mat);
  m.scale.set(w,h,d);
  m.position.set(x,y,z);
  m.castShadow=true;
  (parent||scene).add(m);
  return m;
}

/* ---------- character geometries ---------- */
function appendGeo(g,key,w,h,d,color,x,y,z,rx){
  var q=g[key];
  if(!q){ q={p:[],n:[],c:[]}; g[key]=q; }
  var C=new Col3(color);
  var cr=C.r,cg=C.g,cb=C.b;
  var hw=w/2,hh=h/2,hd=d/2;
  var cos=Math.cos(rx||0),sin=Math.sin(rx||0);
  function vert(px,py,pz){
    var py2=py*cos-pz*sin, pz2=py*sin+pz*cos;
    return [px+x,py2+y,pz2+z];
  }
  function tri(v1,v2,v3,nx,ny,nz){
    q.p.push(v1[0],v1[1],v1[2], v2[0],v2[1],v2[2], v3[0],v3[1],v3[2]);
    for(var k=0;k<3;k++){ q.n.push(nx,ny,nz); q.c.push(cr,cg,cb); }
  }
  function face(p1,p2,p3,p4,nx,ny,nz){
    tri(vert.apply(null,p1),vert.apply(null,p2),vert.apply(null,p3),nx,ny,nz);
    tri(vert.apply(null,p1),vert.apply(null,p3),vert.apply(null,p4),nx,ny,nz);
  }
  face([-hw,-hh,hd],[hw,-hh,hd],[hw,hh,hd],[-hw,hh,hd],0,0,1);
  face([hw,-hh,-hd],[-hw,-hh,-hd],[-hw,hh,-hd],[hw,hh,-hd],0,0,-1);
  face([hw,-hh,hd],[hw,-hh,-hd],[hw,hh,-hd],[hw,hh,hd],1,0,0);
  face([-hw,-hh,-hd],[-hw,-hh,hd],[-hw,hh,hd],[-hw,hh,-hd],-1,0,0);
  face([-hw,hh,-hd],[hw,hh,-hd],[hw,hh,hd],[-hw,hh,hd],0,1,0);
  face([-hw,-hh,hd],[hw,-hh,hd],[hw,-hh,-hd],[-hw,-hh,-hd],0,-1,0);
}
function finishGeo(q){
  for(var i=0;i<q.c.length;i+=3){
    var y=q.p[i+1];
    var f=.76+.28*clamp((y+.2)/1.9,0,1);
    var blue=1-.05*clamp(1-(y+.2)/1.9,0,1);
    q.c[i]=Math.min(1,q.c[i]*f);
    q.c[i+1]=Math.min(1,q.c[i+1]*f);
    q.c[i+2]=Math.min(1,q.c[i+2]*f*blue);
  }
  var geo=new THREE.BufferGeometry();
  geo.setAttribute('position',new THREE.BufferAttribute(new F32(q.p),3));
  geo.setAttribute('normal',new THREE.BufferAttribute(new F32(q.n),3));
  geo.setAttribute('color',new THREE.BufferAttribute(new F32(q.c),3));
  return geo;
}

/* ---------- character instancing pools ---------- */
var CHAR_POOLS={};
var charMat=null;
function buildCharPools(){
  charMat=new THREE.MeshLambertMaterial({vertexColors:true});

  var us={}, usm={}, vc={}, vcr={}, uh={}, umh={}, vh={};
  var lul={}, lur={}, lvl={}, lvr={};

  /* US rifleman */
  appendGeo(us,'body',.56,.6,.32,0x4b5320,0,.99,0);
  appendGeo(us,'body',.6,.12,.34,0x4b5320,0,1.24,0);
  appendGeo(us,'body',.34,.09,.34,0x333d18,0,1.32,0);
  appendGeo(us,'body',.5,.09,.36,0x3a4128,0,.78,0);
  appendGeo(us,'body',.13,.15,.1,0x3a4128,.3,.95,.2);
  appendGeo(us,'body',.11,.2,.1,0x3a4128,-.31,.9,.19);
  appendGeo(us,'body',.42,.5,.24,0x3e451c,0,1.02,-.26);
  appendGeo(us,'body',.15,.46,.17,0x4b5320,-.36,.99,0,-.62);
  appendGeo(us,'body',.15,.46,.17,0x4b5320,.36,.99,0,-.62);
  appendGeo(us,'body',.12,.1,.12,0xc8a165,-.36,.74,.1);
  appendGeo(us,'body',.12,.1,.12,0xc8a165,.36,.74,.1);
  appendGeo(us,'body',.08,.14,.5,0x5d4033,.14,.98,-.28);
  appendGeo(us,'body',.05,.07,.9,0x2a2a2a,.14,1.06,-.72);
  appendGeo(us,'body',.07,.16,.12,0x333d18,.14,.9,-.2);

  /* US medic */
  appendGeo(usm,'bodyMedic',.56,.6,.32,0xd8d8d0,0,.99,0);
  appendGeo(usm,'bodyMedic',.6,.12,.34,0xd8d8d0,0,1.24,0);
  appendGeo(usm,'bodyMedic',.34,.09,.34,0xbfc4b0,0,1.32,0);
  appendGeo(usm,'bodyMedic',.5,.09,.36,0x3a4128,0,.78,0);
  appendGeo(usm,'bodyMedic',.13,.15,.1,0x3a4128,.3,.95,.2);
  appendGeo(usm,'bodyMedic',.11,.2,.1,0x3a4128,-.31,.9,.19);
  appendGeo(usm,'bodyMedic',.42,.5,.24,0x3e451c,0,1.02,-.26);
  appendGeo(usm,'bodyMedic',.15,.46,.17,0xd8d8d0,-.36,.99,0,-.62);
  appendGeo(usm,'bodyMedic',.15,.46,.17,0xd8d8d0,.36,.99,0,-.62);
  appendGeo(usm,'bodyMedic',.12,.1,.12,0xc8a165,-.36,.74,.1);
  appendGeo(usm,'bodyMedic',.12,.1,.12,0xc8a165,.36,.74,.1);
  appendGeo(usm,'bodyMedic',.08,.14,.5,0x5d4033,.14,.98,-.28);
  appendGeo(usm,'bodyMedic',.05,.07,.9,0x2a2a2a,.14,1.06,-.72);
  appendGeo(usm,'bodyMedic',.07,.16,.12,0x333d18,.14,.9,-.2);
  appendGeo(usm,'bodyMedic',.16,.16,.05,0xc0392b,0,1.05,.19);

  /* VC rifleman */
  appendGeo(vc,'body',.54,.6,.3,0x2b2b2b,0,.99,0);
  appendGeo(vc,'body',.58,.12,.32,0x2b2b2b,0,1.24,0);
  appendGeo(vc,'body',.5,.16,.3,0x232323,0,.8,0);
  appendGeo(vc,'body',.5,.09,.05,0x4a3826,0,1.08,.16);
  appendGeo(vc,'body',.15,.46,.17,0x2b2b2b,-.36,.99,0,-.62);
  appendGeo(vc,'body',.15,.46,.17,0x2b2b2b,.36,.99,0,-.62);
  appendGeo(vc,'body',.12,.1,.12,0xc8a165,-.36,.74,.1);
  appendGeo(vc,'body',.12,.1,.12,0xc8a165,.36,.74,.1);
  appendGeo(vc,'body',.08,.14,.5,0x3a3a2a,.14,.98,-.28);
  appendGeo(vc,'body',.05,.07,.95,0x2a2a2a,.14,1.06,-.74);
  appendGeo(vc,'body',.06,.2,.1,0x3a3a2a,.14,.88,-.18);

  /* VC RPG */
  appendGeo(vcr,'bodyRpg',.54,.6,.3,0x2b2b2b,0,.99,0);
  appendGeo(vcr,'bodyRpg',.58,.12,.32,0x2b2b2b,0,1.24,0);
  appendGeo(vcr,'bodyRpg',.5,.16,.3,0x232323,0,.8,0);
  appendGeo(vcr,'bodyRpg',.5,.09,.05,0x4a3826,0,1.08,.16);
  appendGeo(vcr,'bodyRpg',.15,.46,.17,0x2b2b2b,-.36,.99,0,-.62);
  appendGeo(vcr,'bodyRpg',.15,.46,.17,0x2b2b2b,.36,.99,0,-.62);
  appendGeo(vcr,'bodyRpg',.12,.1,.12,0xc8a165,-.36,.74,.1);
  appendGeo(vcr,'bodyRpg',.12,.1,.12,0xc8a165,.36,.74,.1);
  appendGeo(vcr,'bodyRpg',.08,.14,.5,0x3a3a2a,.14,.98,-.28);
  appendGeo(vcr,'bodyRpg',.05,.07,.95,0x2a2a2a,.14,1.06,-.74);
  appendGeo(vcr,'bodyRpg',.06,.2,.1,0x3a3a2a,.14,.88,-.18);
  appendGeo(vcr,'bodyRpg',.14,.14,1.35,0x3a4a30,-.3,1.12,-.3);

  /* US head with helmet */
  appendGeo(uh,'head',.34,.36,.36,0xc8a165,0,1.56,0);
  appendGeo(uh,'head',.3,.07,.05,0x1c1410,0,1.6,.17);
  appendGeo(uh,'head',.44,.16,.46,0x3f4a22,0,1.78,0);
  appendGeo(uh,'head',.48,.05,.5,0x3f4a22,0,1.72,0);
  appendGeo(uh,'head',.46,.05,.08,0x2f3818,0,1.71,.24);

  /* medic head: white helmet with red cross */
  appendGeo(umh,'medicHead',.34,.36,.36,0xc8a165,0,1.56,0);
  appendGeo(umh,'medicHead',.3,.07,.05,0x1c1410,0,1.6,.17);
  appendGeo(umh,'medicHead',.44,.16,.46,0xdedede,0,1.78,0);
  appendGeo(umh,'medicHead',.48,.05,.5,0xdedede,0,1.72,0);
  appendGeo(umh,'medicHead',.1,.3,.03,0xc0392b,0,1.8,.24);
  appendGeo(umh,'medicHead',.3,.1,.03,0xc0392b,0,1.8,.24);

  /* VC head with straw hat */
  appendGeo(vh,'head',.34,.36,.36,0xc8a165,0,1.56,0);
  appendGeo(vh,'head',.3,.07,.05,0x1c1410,0,1.6,.17);
  appendGeo(vh,'head',.5,.04,.5,0xc9a45c,0,1.72,0);
  for(var s=0;s<8;s++){
    var a=s/8*TAU;
    appendGeo(vh,'head',.28,.2,.28,0xd9b26a,Math.cos(a)*.34,1.82,Math.sin(a)*.34);
  }
  appendGeo(vh,'head',.05,.3,.05,0x8a6a3a,0,1.6,.22);

  /* legs */
  appendGeo(lul,'legL',.19,.5,.2,0x3b431a,-.15,.44,0);
  appendGeo(lul,'legL',.17,.4,.18,0x3b431a,-.15,.15,0);
  appendGeo(lul,'legL',.18,.1,.3,0x241c14,-.15,.05,.05);
  appendGeo(lur,'legR',.19,.5,.2,0x3b431a,.15,.44,0);
  appendGeo(lur,'legR',.17,.4,.18,0x3b431a,.15,.15,0);
  appendGeo(lur,'legR',.18,.1,.3,0x241c14,.15,.05,.05);
  appendGeo(lvl,'legL',.19,.5,.2,0x2b2b2b,-.15,.44,0);
  appendGeo(lvl,'legL',.17,.4,.18,0x2b2b2b,-.15,.15,0);
  appendGeo(lvl,'legL',.18,.1,.3,0x1f1a14,-.15,.05,.05);
  appendGeo(lvr,'legR',.19,.5,.2,0x2b2b2b,.15,.44,0);
  appendGeo(lvr,'legR',.17,.4,.18,0x2b2b2b,.15,.15,0);
  appendGeo(lvr,'legR',.18,.1,.3,0x1f1a14,.15,.05,.05);

  var usGeos={
    body:finishGeo(us.body), bodyMedic:finishGeo(usm.bodyMedic),
    head:finishGeo(uh.head), medicHead:finishGeo(umh.medicHead),
    legL:finishGeo(lul.legL), legR:finishGeo(lur.legR)
  };
  var vcGeos={
    body:finishGeo(vc.body), bodyRpg:finishGeo(vcr.bodyRpg),
    head:finishGeo(vh.head), legL:finishGeo(lvl.legL), legR:finishGeo(lvr.legR)
  };

  CHAR_POOLS.us=makeCharPool(160,usGeos);
  CHAR_POOLS.vc=makeCharPool(190,vcGeos);
}
function makeCharPool(n,geos){
  var pool={free:[],used:0,max:n,parts:{}};
  for(var k in geos){
    var m=new THREE.InstancedMesh(geos[k],charMat,n);
    m.instanceColor=new THREE.InstancedBufferAttribute(new F32(n*3).fill(1),3);
    m.count=n;
    m.castShadow=true;
    m.frustumCulled=false;
    m.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
    if(m.instanceColor.setUsage) m.instanceColor.setUsage(THREE.DynamicDrawUsage);
    for(var i=0;i<n;i++) m.setMatrixAt(i,_hideM);
    scene.add(m);
    pool.parts[k]=m;
  }
  return pool;
}
function charHide(pool,slot){
  for(var k in pool.parts) pool.parts[k].setMatrixAt(slot,_hideM);
}
function charAlloc(pool){
  if(pool.free.length) return pool.free.pop();
  if(pool.used<pool.max) return pool.used++;
  return -1;
}
function charFree(pool,slot){
  charHide(pool,slot);
  pool.free.push(slot);
}
var _legM=new Mat4(), _hipM=new Mat4(), _bodyM=new Mat4();
function charPose(e,swingL,swingR){
  var pool=e._pool, slot=e._slot, P=pool.parts;
  var sc=e.shade||1;
  var cr=sc, cg=sc, cb=sc;
  if(e._tint>0){ cr=1; cg=.32; cb=.26; }
  var yy=(e.y!==undefined?e.y:0);
  _tmpObj.rotation.order='YZX';
  _tmpObj.position.set(e.x,yy+(e.poseYOff||0),e.z);
  _tmpObj.rotation.set(e.posePitch||0,e.faceYaw||0,e.poseRoll||0);
  _tmpObj.scale.set(1,1,1);
  _tmpObj.updateMatrix();
  _bodyM.copy(_tmpObj.matrix);
  var bodyKey=e.medic?'bodyMedic':(e.rpg?'bodyRpg':'body');
  var headKey=e.medic?'medicHead':'head';
  P[bodyKey].setMatrixAt(slot,_bodyM);
  P[bodyKey].instanceColor.array[slot*3]=cr;
  P[bodyKey].instanceColor.array[slot*3+1]=cg;
  P[bodyKey].instanceColor.array[slot*3+2]=cb;
  P[headKey].setMatrixAt(slot,_bodyM);
  P[headKey].instanceColor.array[slot*3]=cr;
  P[headKey].instanceColor.array[slot*3+1]=cg;
  P[headKey].instanceColor.array[slot*3+2]=cb;

  _tmpObj.position.set(e.x,0,e.z);
  _tmpObj.rotation.set(0,e.faceYaw||0,0);
  _tmpObj.updateMatrix();
  _legM.copy(_tmpObj.matrix);
  _hipM.makeTranslation(-.15,.68,0);
  _legM.multiply(_hipM);
  _hipM.makeRotationX(swingL);
  _legM.multiply(_hipM);
  _hipM.makeTranslation(0,-.68,0);
  _legM.multiply(_hipM);
  P.legL.setMatrixAt(slot,_legM);
  P.legL.instanceColor.array[slot*3]=cr;
  P.legL.instanceColor.array[slot*3+1]=cg;
  P.legL.instanceColor.array[slot*3+2]=cb;

  _tmpObj.position.set(e.x,0,e.z);
  _tmpObj.rotation.set(0,e.faceYaw||0,0);
  _tmpObj.updateMatrix();
  _legM.copy(_tmpObj.matrix);
  _hipM.makeTranslation(.15,.68,0);
  _legM.multiply(_hipM);
  _hipM.makeRotationX(swingR);
  _legM.multiply(_hipM);
  _hipM.makeTranslation(0,-.68,0);
  _legM.multiply(_hipM);
  P.legR.setMatrixAt(slot,_legM);
  P.legR.instanceColor.array[slot*3]=cr;
  P.legR.instanceColor.array[slot*3+1]=cg;
  P.legR.instanceColor.array[slot*3+2]=cb;
}
function flushCharPools(){
  for(var side in CHAR_POOLS){
    var pool=CHAR_POOLS[side];
    for(var k in pool.parts){
      pool.parts[k].instanceMatrix.needsUpdate=true;
      if(pool.parts[k].instanceColor) pool.parts[k].instanceColor.needsUpdate=true;
    }
  }
}
