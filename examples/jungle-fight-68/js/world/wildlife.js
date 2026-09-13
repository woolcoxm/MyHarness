'use strict';
/* SECTION 15 — WILDLIFE */
var deerMat=null, deerGeos=null;
var monkMat=null;
var birdMat=null;
var ffMat=null, ffPts=null;
var chickMat=null;

function buildWildlife(){
  deerMat=new THREE.MeshLambertMaterial({vertexColors:true});
  monkMat=new THREE.MeshLambertMaterial({vertexColors:true});
  birdMat=new THREE.MeshLambertMaterial({vertexColors:true});
  chickMat=new THREE.MeshLambertMaterial({vertexColors:true});
  ffMat=new THREE.MeshBasicMaterial({color:0xc8ff6a,transparent:true,opacity:.9,blending:THREE.AdditiveBlending,depthWrite:false});
}

function spawnDeer(x,z){
  var g=new ObjT();
  var body=eBox(.9,.7,1.5,0x8a6a45,0,1,0,g); body.material=deerMat;
  var headM=eBox(.4,.45,.55,0x8a6a45,0,1.5,.85,g);
  var legMats=[eBox(.16,.9,.16,0x6b5836,-.3,-.1,.5,g),eBox(.16,.9,.16,0x6b5836,.3,-.1,.5,g),
               eBox(.16,.9,.16,0x6b5836,-.3,-.1,-.5,g),eBox(.16,.9,.16,0x6b5836,.3,-.1,-.5,g)];
  var y=heightAt(x+HALF,z+HALF);
  g.position.set(x,y,z);
  var d={x:x,z:z,y:y,g:g,legs:legMats,hp:30,dead:false,
         wanderT:0,wx:x,wz:z,fleeT:0,fleeDir:0,walkPh:0,speed:1.2};
  deers.push(d);
  return d;
}
function updateDeer(dt){
  for(var i=0;i<deers.length;i++){
    var d=deers[i];
    if(d.dead) continue;
    d.wanderT-=dt;
    if(d.fleeT>0){
      d.fleeT-=dt;
      var sp=5.5;
      var nx=d.x+Math.cos(d.fleeDir)*sp*dt;
      var nz=d.z+Math.sin(d.fleeDir)*sp*dt;
      if(!collideAt(nx,d.y+1,nz,1,.5,true)){
        d.x=nx; d.z=nz;
        d.g.rotation.y=d.fleeDir;
      } else d.fleeDir+=rand(1,2.5);
    } else if(d.wanderT<=0){
      d.wanderT=rand(3,8);
      d.wx=d.x+rand(-12,12); d.wz=d.z+rand(-12,12);
    }
    if(d.fleeT<=0){
      var dx=d.wx-d.x, dz=d.wz-d.z;
      var dist=Math.hypot(dx,dz);
      if(dist>.5){
        var mv=Math.min(dist,d.speed*dt);
        d.x+=dx/dist*mv; d.z+=dz/dist*mv;
        d.g.rotation.y=Math.atan2(dx,dz);
        d.walkPh+=dt*6;
      }
    } else d.walkPh+=dt*14;
    d.y=heightAt(d.x+HALF,d.z+HALF);
    d.g.position.set(d.x,d.y,d.z);
    var sw=Math.sin(d.walkPh)*.5;
    d.legs[0].rotation.x=sw; d.legs[3].rotation.x=sw;
    d.legs[1].rotation.x=-sw; d.legs[2].rotation.x=-sw;
    if(IS_TOUCH&&Math.hypot(d.x-camera.position.x,d.z-camera.position.z)>90) d.g.visible=false;
    else d.g.visible=true;
  }
}
function damageDeer(d,dmg){
  d.hp-=dmg;
  if(d.hp<=0&&!d.dead){
    d.dead=true;
    d.g.rotation.z=Math.PI/2;
    killfeed("DEER DOWN \u2026 IT'S WAR");
  }
}

function spawnMonkey(){
  var t=treeTops[(Math.random()*treeTops.length)|0];
  if(!t) return;
  var m={x:t.x,y:heightAt(t.x+HALF,t.z+HALF)+rand(6,9),z:t.z,
         tx:t.x,tz:t.z,waitT:rand(0,3),ph:rand(0,TAU),g:null};
  var g=new ObjT();
  eBox(.34,.4,.3,0x7a5a3a,0,0,0,g);
  eBox(.26,.26,.26,0x8a6a4a,0,.32,.14,g);
  eBox(.4,.1,.1,0x7a5a3a,0,.02,-.28,g);
  eBox(.09,.3,.09,0x6b4a2f,-.12,-.3,0,g);
  eBox(.09,.3,.09,0x6b4a2f,.12,-.3,0,g);
  scene.add(g);
  m.g=g;
  monkeys.push(m);
}
function updateMonkeys(dt){
  for(var i=0;i<monkeys.length;i++){
    var m=monkeys[i];
    m.waitT-=dt;
    if(m.waitT<=0){
      m.waitT=rand(1,4);
      var t=treeTops[(Math.random()*treeTops.length)|0];
      if(t&&Math.hypot(t.x-m.x,t.z-m.z)<40){ m.tx=t.x; m.tz=t.z; }
    }
    var dx=m.tx-m.x, dz=m.tz-m.z;
    var d=Math.hypot(dx,dz);
    if(d>.3){
      var mv=Math.min(d,2.4*dt);
      m.x+=dx/d*mv; m.z+=dz/d*mv;
    }
    var ground=heightAt(m.x+HALF,m.z+HALF);
    m.y=ground+rand(6,9)*.5+3.5+Math.sin(m.ph+=dt*3)*.2;
    m.g.position.set(m.x,m.y+Math.sin(time*6+i)*.1,m.z);
    m.g.rotation.y=Math.atan2(dx,dz);
    if(IS_TOUCH&&Math.hypot(m.x-camera.position.x,m.z-camera.position.z)>90) m.g.visible=false;
    else m.g.visible=true;
  }
}

function spawnFlock(){
  var cx=rand(-HALF+20,HALF-20), cz=rand(-HALF+20,HALF-20);
  var n=irand(3,5);
  var birds=[];
  var g=new ObjT();
  for(var i=0;i<n;i++){
    var b=eBox(.22,.14,.5,0x3a3a3a,0,0,0,g);
    var wL=eBox(.5,.05,.2,0x4a4a4a,-.3,0,0,g);
    var wR=eBox(.5,.05,.2,0x4a4a4a,.3,0,0,g);
    birds.push({m:b,wL:wL,wR:wR,ph:rand(0,TAU),r:rand(3,7),a:rand(0,TAU),sp:rand(.5,1.1)});
  }
  scene.add(g);
  flocks.push({g:g,cx:cx,cz:cz,h:rand(26,36),birds:birds,scatterT:0});
}
function updateFlocks(dt){
  for(var i=0;i<flocks.length;i++){
    var f=flocks[i];
    if(f.scatterT>0) f.scatterT-=dt;
    for(var b=0;b<f.birds.length;b++){
      var bd=f.birds[b];
      bd.a+=bd.sp*dt*(f.scatterT>0?3:1);
      var rr=bd.r+(f.scatterT>0?bd.r*2:0);
      var x=f.cx+Math.cos(bd.a)*rr;
      var z=f.cz+Math.sin(bd.a)*rr;
      bd.m.position.set(x,f.h+Math.sin(bd.ph+time*3)*1.5,z);
      bd.m.rotation.y=-bd.a;
      var fl=Math.sin(time*10+bd.ph)*.7;
      bd.wL.rotation.z=fl; bd.wR.rotation.z=-fl;
    }
    if(IS_TOUCH&&Math.hypot(f.cx-camera.position.x,f.cz-camera.position.z)>120) f.g.visible=false;
    else f.g.visible=true;
  }
}

/* fireflies */
var fireflies=[];
function spawnFireflies(){
  var n=IS_TOUCH?22:36;
  var pos=new F32(n*3);
  for(var i=0;i<n;i++){
    pos[i*3]=camera.position.x+rand(-26,26);
    pos[i*3+1]=heightAt(camera.position.x+HALF,camera.position.z+HALF)+rand(.5,3);
    pos[i*3+2]=camera.position.z+rand(-26,26);
  }
  var g=new THREE.BufferGeometry();
  g.setAttribute('position',new THREE.BufferAttribute(pos,3));
  ffPts=new THREE.Points(g,ffMat);
  scene.add(ffPts);
  for(var k=0;k<n;k++)
    fireflies.push({ph:rand(0,TAU),vx:rand(-.4,.4),vz:rand(-.4,.4)});
}
function updateFireflies(dt){
  if(!ffPts) return;
  var on=dayFactor<.22;
  ffPts.visible=on;
  if(!on) return;
  var pos=ffPts.geometry.attributes.position;
  for(var i=0;i<fireflies.length;i++){
    var f=fireflies[i];
    f.ph+=dt*rand(1,3);
    var x=pos.getX(i)+f.vx*dt;
    var z=pos.getZ(i)+f.vz*dt;
    if(Math.hypot(x-camera.position.x,z-camera.position.z)>26){
      x=camera.position.x+rand(-24,24);
      z=camera.position.z+rand(-24,24);
    }
    var y=heightAt(x+HALF,z+HALF)+.6+Math.abs(Math.sin(f.ph))*1.6;
    pos.setXYZ(i,x,y,z);
  }
  pos.needsUpdate=true;
  ffMat.opacity=.4+Math.abs(Math.sin(time*2))*.5;
}

function spawnChicken(x,z){
  var g=new ObjT();
  eBox(.3,.35,.4,0xe8e8e8,0,.3,0,g);
  eBox(.18,.2,.2,0xd8d8d8,0,.6,.2,g);
  eBox(.08,.1,.12,0xc0392b,0,.72,.28,g);
  scene.add(g);
  chickens.push({x:x,z:z,g:g,peckT:rand(0,3),fleeT:0,dir:rand(0,TAU),wx:x,wz:z,ph:0});
}
function updateChickens(dt){
  for(var i=0;i<chickens.length;i++){
    var c=chickens[i];
    if(c.fleeT>0){
      c.fleeT-=dt;
      var nx=c.x+Math.cos(c.dir)*3.5*dt, nz=c.z+Math.sin(c.dir)*3.5*dt;
      if(!collideAt(nx,heightAt(c.x+HALF,c.z+HALF)+.5,nz,.6,.25,true)){ c.x=nx; c.z=nz; }
      else c.dir+=rand(1,2);
    } else {
      c.peckT-=dt;
      if(c.peckT<=0){
        c.peckT=rand(.5,2.5);
        c.wx=c.x+rand(-3,3); c.wz=c.z+rand(-3,3);
      }
      var dx=c.wx-c.x, dz=c.wz-c.z;
      var d=Math.hypot(dx,dz);
      if(d>.2){ c.x+=dx/d*.8*dt; c.z+=dz/d*.8*dt; c.ph+=dt*8; }
    }
    var y=heightAt(c.x+HALF,c.z+HALF);
    c.g.position.set(c.x,y+Math.abs(Math.sin(c.ph))*.06,c.z);
    c.g.rotation.y=Math.atan2(c.wx-c.x,c.wz-c.z);
    if(c.fleeT>0) c.g.rotation.x=.3; else c.g.rotation.x=c.peckT<.4?.5:0;
    if(IS_TOUCH&&Math.hypot(c.x-camera.position.x,c.z-camera.position.z)>80) c.g.visible=false;
    else c.g.visible=true;
  }
}
function wildlifeScare(pos,r){
  for(var i=0;i<deers.length;i++){
    var d=deers[i];
    if(!d.dead&&Math.hypot(d.x-pos.x,d.z-pos.z)<35){
      d.fleeT=5;
      d.fleeDir=Math.atan2(d.x-pos.x,d.z-pos.z)+rand(-.5,.5);
    }
  }
  for(var c=0;c<chickens.length;c++){
    var ch=chickens[c];
    if(Math.hypot(ch.x-pos.x,ch.z-pos.z)<30){
      ch.fleeT=3;
      ch.dir=Math.atan2(ch.x-pos.x,ch.z-pos.z);
    }
  }
  for(var f=0;f<flocks.length;f++){
    var fl=flocks[f];
    if(Math.hypot(fl.cx-pos.x,fl.cz-pos.z)<45) fl.scatterT=4;
  }
}
