'use strict';
/* matrix element index constants (12,13,14) */
var MZ=12,MO=13,MP=14;
/* SECTION 14 — ENVIRONMENT */
var dayT=.34, dayFactor=1;
var weather={kind:'SUNNY',target:IS_TOUCH?150:210,cover:.15};
var weatherT=60;
var clouds=[], precipRain=null, precipSnow=null;
var flybyT=rand(70,150), flybyObj=null;
var _skyCol=new Col3(), _fogCol=new Col3();
var NIGHT=new Col3(0x0a0e24), DAY=new Col3(0x9fd8cf);
var DUSK=new Col3(0xff8c42);
var thunderT=rand(6,16);

function makeClouds(){
  var R=mulberry32(7);
  var mat=new THREE.MeshLambertMaterial({color:0xf0f2ee,transparent:true,opacity:.9});
  for(var c=0;c<12;c++){
    var g=new ObjT();
    var n=5+Math.floor(R()*5);
    for(var b=0;b<n;b++){
      var m=new THREE.Mesh(unitBoxGeo(),mat);
      m.scale.set(8+R()*16,3+R()*5,8+R()*12);
      m.position.set((R()-.5)*24,(R()-.5)*4,(R()-.5)*16);
      g.add(m);
    }
    g.position.set((R()-.5)*W,60,(R()-.5)*W);
    g.userData={vx:.6+R()*.8,mat:mat};
    scene.add(g);
    clouds.push(g);
  }
}
function makePrecip(){
  /* rain streaks */
  var rn=IS_TOUCH?260:600;
  var rg=new THREE.BoxGeometry(.05,.7,.05);
  var rm=new THREE.MeshBasicMaterial({color:0x9fd8e8,transparent:true,opacity:.5});
  precipRain=new THREE.InstancedMesh(rg,rm,rn);
  precipRain.count=rn;
  precipRain.frustumCulled=false;
  precipRain.userData={n:rn};
  for(var i=0;i<rn;i++){
    _tmpObj.position.set(camera.position.x+rand(-40,40),rand(0,40),camera.position.z+rand(-40,40));
    _tmpObj.rotation.set(.35,0,0);
    _tmpObj.scale.set(1,1,1);
    _tmpObj.updateMatrix();
    precipRain.setMatrixAt(i,_tmpObj.matrix);
  }
  scene.add(precipRain);
  /* snow flakes */
  var sn=IS_TOUCH?180:400;
  var sg=new THREE.BoxGeometry(.14,.14,.14);
  var sm=new THREE.MeshBasicMaterial({color:0xffffff,transparent:true,opacity:.85});
  precipSnow=new THREE.InstancedMesh(sg,sm,sn);
  precipSnow.count=sn;
  precipSnow.frustumCulled=false;
  for(var s=0;s<sn;s++){
    _tmpObj.position.set(camera.position.x+rand(-40,40),rand(0,40),camera.position.z+rand(-40,40));
    _tmpObj.rotation.set(0,0,0);
    _tmpObj.scale.set(1,1,1);
    _tmpObj.updateMatrix();
    precipSnow.setMatrixAt(s,_tmpObj.matrix);
  }
  scene.add(precipSnow);
}
function pickWeather(){
  var kinds=['SUNNY','CLOUDY','RAIN','STORM','SNOW'];
  var wts=[30,25,17,12,16];
  var pick;
  do{
    var r=Math.random()*wts.reduce(function(a,b){return a+b;},0);
    var acc=0,i=0;
    for(i=0;i<wts.length;i++){ acc+=wts[i]; if(r<acc) break; }
    pick=kinds[Math.min(i,kinds.length-1)];
  }while(pick===weather.kind);
  weather.kind=pick;
  if(pick==='SUNNY'){ weather.target=IS_TOUCH?150:210; weather.cover=.15; }
  else if(pick==='CLOUDY'){ weather.target=IS_TOUCH?125:170; weather.cover=.55; }
  else if(pick==='RAIN'){ weather.target=110; weather.cover=.85; }
  else if(pick==='STORM'){ weather.target=85; weather.cover=1; }
  else{ weather.target=IS_TOUCH?100:135; weather.cover=.7; }
  weatherT=rand(50,110);
}
function updateWeather(dt){
  weatherT-=dt;
  if(weatherT<=0) pickWeather();
  var k=weather.kind;
  setRainVol(k==='RAIN'?.16:(k==='STORM'?.3:0));
  setWindVol(k==='STORM'?.06:0);
  precipRain.visible=(k==='RAIN'||k==='STORM');
  precipSnow.visible=(k==='SNOW');
  /* lightning */
  if(k==='STORM'){
    thunderT-=dt;
    if(thunderT<=0){
      thunderT=rand(6,16);
      var op=SET.shake?.75:.18;
      var fl=el('flash');
      fl.style.transition='none';
      fl.style.opacity=op;
      setTimeout(function(){ fl.style.transition='opacity .6s'; fl.style.opacity=0; },120);
      setTimeout(sThunder,rand(300,1500));
    }
  }
  /* clouds drift */
  var sink=lerp(60,44,weather.cover);
  for(var c=0;c<clouds.length;c++){
    var cl=clouds[c];
    cl.position.x+=cl.userData.vx*dt;
    if(cl.position.x>HALF+60) cl.position.x=-HALF-60;
    cl.position.y=lerp(cl.position.y,sink,dt*.2);
    var tint=lerp(1,.42,weather.cover);
    cl.userData.mat.color.setRGB(tint*.98,tint*.99,tint);
    cl.visible=weather.cover>.1;
    cl.userData.mat.opacity=Math.min(.95,weather.cover+ .1);
  }
  /* rain fall */
  if(precipRain.visible){
    var n=precipRain.userData.n;
    for(var i=0;i<n;i++){
      precipRain.getMatrixAt(i,_m4);
      var me=_m4.elements; var x=me[MZ],y=me[MO],z=me[MP];
      y-=26*dt;
      x+=6*dt;
      if(y<camera.position.y-6){
        y=camera.position.y+rand(10,26);
        x=camera.position.x+rand(-40,40);
        z=camera.position.z+rand(-40,40);
      }
      _tmpObj.position.set(x,y,z);
      _tmpObj.rotation.set(.35,0,0);
      _tmpObj.scale.set(1,1,1);
      _tmpObj.updateMatrix();
      precipRain.setMatrixAt(i,_tmpObj.matrix);
    }
    precipRain.instanceMatrix.needsUpdate=true;
  }
  /* snow sway */
  if(precipSnow.visible){
    var ns=precipSnow.count;
    for(var s=0;s<ns;s++){
      precipSnow.getMatrixAt(s,_m4);
      var me=_m4.elements; var sx=me[MZ],sy=me[MO],sz=me[MP];
      sy-=2*dt;
      sx+=Math.sin(time*2+s)*.4*dt;
      if(sy<camera.position.y-6){
        sy=camera.position.y+rand(8,22);
        sx=camera.position.x+rand(-40,40);
        sz=camera.position.z+rand(-40,40);
      }
      _tmpObj.position.set(sx,sy,sz);
      _tmpObj.rotation.set(0,0,0);
      _tmpObj.scale.set(1,1,1);
      _tmpObj.updateMatrix();
      precipSnow.setMatrixAt(s,_tmpObj.matrix);
    }
    precipSnow.instanceMatrix.needsUpdate=true;
  }
}
var flybyR=null;
function updateFlyby(dt){
  flybyT-=dt;
  if(flybyT<=0&&!flybyObj){
    flybyT=rand(70,150);
    var g=new ObjT();
    eBox(1.4,1.5,4.6,0x3c4a26,0,0,0,g);
    eBox(1.1,.8,1.6,0x22304a,0,.9,.6,g);
    eBox(8,.16,1.5,0x354421,0,.2,0,g);
    eBox(2.6,.14,1,0x354421,0,.3,2.3,g);
    eBox(.16,1.4,1,0x354421,0,1,-2.2,g);
    var rotor=eBox(7,.08,.4,0x222222,0,1.3,0,g);
    g.userData.rotor=rotor;
    var side=Math.random()<.5?1:-1;
    var y=rand(34,46);
    g.position.set(-side*(HALF+40),y,rand(-HALF,HALF));
    scene.add(g);
    flybyObj={g:g,vx:side*rand(14,20),t:15};
    sFlyby('heli',g.position);
  }
  if(flybyObj){
    flybyObj.t-=dt;
    flybyObj.g.position.x+=flybyObj.vx*dt;
    flybyObj.g.userData.rotor.rotation.y+=dt*30;
    flybyObj.g.rotation.y=flybyObj.vx>0?Math.PI/2:-Math.PI/2;
    if(flybyObj.t<=0||Math.abs(flybyObj.g.position.x)>HALF+60){
      scene.remove(flybyObj.g);
      flybyObj=null;
    }
  }
}
function updateSky(dt){
  dayT=(dayT+dt/240)%1;
  var ang=dayT*TAU-Math.PI/2;
  var elev=Math.sin(ang);
  dayFactor=clamp(Math.sin(ang)*1.6,0,1);
  var dusk=Math.pow(clamp(1-Math.abs(elev)*2.2,0,1),1.5);
  _skyCol.copy(NIGHT).lerp(DAY,dayFactor);
  _skyCol.lerp(DUSK,dusk*.5);
  scene.background.copy(_skyCol);
  _fogCol.copy(NIGHT).lerp(DAY,dayFactor);
  _fogCol.lerp(DUSK,dusk*.3);
  scene.fog.color.copy(_fogCol);
  skyDome.material.uniforms.top.value.copy(_skyCol);
  skyDome.material.uniforms.bot.value.copy(_fogCol);
  skyDome.position.set(camera.position.x,0,camera.position.z);
  /* fog far eases toward weather target */
  var far=scene.fog.far;
  far=lerp(far,weather.target,dt*.5);
  scene.fog.far=far;
  scene.fog.near=far*.14;
  /* sun orbit */
  var sx=camera.position.x+Math.cos(ang)*250*.4;
  var sy=Math.sin(ang)*250;
  var sz=camera.position.z+90;
  sunLight.position.set(camera.position.x+Math.cos(ang)*120,sy*.4+30,camera.position.z+90);
  sunLight.target.position.set(camera.position.x,0,camera.position.z);
  sunLight.intensity=.95*dayFactor;
  sunLight.castShadow=dayFactor>.05;
  var duskMix=new Col3(0xffe084).lerp(DUSK,dusk*.8);
  sunBill.material.color.copy(duskMix);
  sunBill.position.set(camera.position.x+Math.cos(ang)*350,Math.sin(ang)*260,camera.position.z+120);
  moonBill.position.set(camera.position.x-Math.cos(ang)*350,-Math.sin(ang)*260,camera.position.z-120);
  moonLight.intensity=.13*(1-dayFactor);
  starsMat.opacity=clamp(1-dayFactor*2,0,1)*.9;
  stars.position.set(camera.position.x,0,camera.position.z);
  hemiLight.intensity=.18+.72*dayFactor;
  /* water bob */
  if(waterBobber){
    _tmpObj.position.set(0,Math.sin(time*.6)*.05,0);
    _tmpObj.updateMatrix();
    /* bob whole mesh via position */
    waterBobber.position.y=Math.sin(time*.6)*.05;
  }
  /* campfires flicker at night */
  for(var c=0;c<campfires.length;c++){
    var cf=campfires[c];
    if(cf.light) cf.light.intensity=dayFactor<.4?1.2+Math.sin(time*11+c*3)*.4:0;
    if(cf.mesh) cf.mesh.scale.y=.8+Math.abs(Math.sin(time*9+c))*.5;
  }
  /* muzzle/exp light decay */
  if(expLight.intensity>0) expLight.intensity=Math.max(0,expLight.intensity-dt*30);
}
