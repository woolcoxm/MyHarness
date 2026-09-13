'use strict';
/* SECTION 9 — PLAYER */
var P={};
function initPlayer(){
  P={
    pos:{x:0,y:10,z:HALF-20},
    velX:0, velZ:0, vy:0,
    yaw:0, pitch:0,
    stance:0, hp:100, dead:false, deathT:0,
    trigger:false, firedPress:false, clickedEmpty:false,
    cur:'garand', fireCd:0, boltT:0, reloadT:0, meleeT:0, swingT:0, nadeCd:0,
    nades:4, aim:false, lastShotT:-9, lastHurtT:-9, spawnProtT:0,
    moveSpeed:0, stepT:0, bobT:0, shake:0, flashT:0, recoil:0, scope:false,
    touchVec:null, touchSprint:false,
    pools:{pistol:48,smg:180,rifle:120}
  };
}
var EYE=[1.62,1.05,.45];
var HITBOX=[1.75,1.2,.6];

function damagePlayer(dmg,src){
  if(P.dead||P.spawnProtT>0) return;
  P.hp-=dmg;
  P.lastHurtT=time;
  dmgFlash=Math.min(1,(dmgFlash||0)+.45);
  P.shake=Math.max(P.shake,.25);
  sGrunt();
  if(src){
    var dx=src.x-P.pos.x, dz=src.z-P.pos.z;
    var ang=Math.atan2(dx,-dz)-P.yaw;
    dmgDirAngle=ang;
    dmgDirT=1;
  }
  if(P.hp<=0) diePlayer();
}
function diePlayer(){
  P.dead=true;
  P.deathT=time;
  deaths++;
  VC+=3;
  el('ovDeath').classList.add('show');
}
function findClearSpot(cx,cz,r,h,rad){
  for(var ring=0;ring<28;ring++){
    var a=rand(0,TAU);
    var d=r*(ring+1)/28*2;
    var x=cx+Math.cos(a)*Math.min(d,r*2);
    var z=cz+Math.sin(a)*Math.min(d,r*2);
    if(x<-HALF+4||x>HALF-4||z<-HALF+4||z>HALF-4) continue;
    var y=heightAt(x+HALF,z+HALF);
    if(y<=WATER+.2) continue;
    if(collideAt(x,y+.1,z,h||1.7,rad||.4,false)) continue;
    return {x:x,y:y,z:z};
  }
  return null;
}
function respawnPlayer(){
  var spots=[{x:usBase.x,z:usBase.z,label:'FIREBASE'}];
  for(var i=0;i<usSpawns.length;i++)
    spots.push({x:usSpawns[i].x,z:usSpawns[i].z,label:usSpawns[i].label});
  spots.sort(function(a,b){
    return Math.hypot(a.x-P.pos.x,a.z-P.pos.z)-Math.hypot(b.x-P.pos.x,b.z-P.pos.z);
  });
  var spot=null,label='FIREBASE';
  for(var s=0;s<spots.length;s++){
    var c=findClearSpot(spots[s].x,spots[s].z,8,1.75,.4);
    if(c){ spot=c; label=spots[s].label; break; }
  }
  if(!spot) spot={x:usBase.x,y:heightAt(usBase.x+HALF,usBase.z+HALF),z:usBase.z};
  P.pos.x=spot.x; P.pos.y=spot.y; P.pos.z=spot.z;
  P.hp=100; P.dead=false; P.vy=0;
  for(var k=0;k<WKEYS.length;k++){
    var w=WPN[WKEYS[k]];
    if(w&&w.magMax) w.mag=w.magMax;
  }
  if(P.nades<2) P.nades=2;
  P.spawnProtT=3;
  el('ovDeath').classList.remove('show');
  killfeed('BACK IN THE FIGHT \u2014 RESPAWNED AT '+label);
  if(!IS_TOUCH){
    try{ renderer.domElement.requestPointerLock(); }catch(e){}
  }
}

function updatePlayer(dt){
  if(P.spawnProtT>0) P.spawnProtT-=dt;
  if(P.dead){
    if(time-P.deathT>3.2) respawnPlayer();
    camera.position.y=Math.max(camera.position.y-dt*1.4,P.pos.y+.4);
    return;
  }
  /* regen */
  if(time-P.lastHurtT>6&&P.hp<100) P.hp=Math.min(100,P.hp+5*dt);
  var inWater=P.pos.y+0.4<WATER;
  var w=WPN[P.cur];
  var speed=4.3*(P.stance===1?.5:P.stance===2?.25:1)*((w&&w.mob)||1);
  var fwd=key('KeyW')||key('ArrowUp');
  var sprinting=false;
  if((key('ShiftLeft')||key('ShiftRight')||(P.touchSprint&&fwdLike()))&&P.stance===0&&fwd&&!inWater){
    speed*=1.5; sprinting=true;
  }
  if(inWater) speed*=.5;
  if(P.aim) speed*=.55;
  /* input vector camera-relative */
  var mx=0,mz=0;
  if(!IS_TOUCH){
    if(key('KeyW'))mz-=1; if(key('KeyS'))mz+=1;
    if(key('KeyA'))mx-=1; if(key('KeyD'))mx+=1;
  } else if(P.touchVec){
    mx=P.touchVec.x; mz=P.touchVec.y;
  }
  var len=Math.hypot(mx,mz);
  if(len>1){ mx/=len; mz/=len; }
  var cos=Math.cos(P.yaw), sin=Math.sin(P.yaw);
  var wx=(mx*cos-mz*sin)*speed;
  var wz=(mx*sin+mz*cos)*speed;
  P.velX=wx; P.velZ=wz;
  P.moveSpeed=Math.hypot(wx,wz);
  var stepAssist=1.25;
  /* axis-separated movement */
  var nx=P.pos.x+wx*dt;
  if(!collideAt(nx,P.pos.y+.1,P.pos.z,HITBOX[P.stance],.35,false)) P.pos.x=nx;
  else{
    var upY=P.pos.y+stepAssist;
    if(!collideAt(nx,upY,P.pos.z,HITBOX[P.stance],.35,false)) P.pos.x=nx;
  }
  var nz=P.pos.z+wz*dt;
  if(!collideAt(P.pos.x,P.pos.y+.1,P.nzSafe!==undefined?P.pos.z:P.pos.z,HITBOX[P.stance],.35,false)&&
     !collideAt(P.pos.x,P.pos.y+.1,nz,HITBOX[P.stance],.35,false)) P.pos.z=nz;
  else{
    var upZ=P.pos.y+stepAssist;
    if(!collideAt(P.pos.x,upZ,nz,HITBOX[P.stance],.35,false)) P.pos.z=nz;
  }
  /* gravity and ground */
  P.vy-=14*dt;
  var ny=P.pos.y+P.vy*dt;
  var g=groundAt(P.pos.x,ny,P.pos.z);
  if(ny<=g){
    ny=g;
    if(P.vy<-11) damagePlayer(Math.min(60,(-P.vy-11)*4),null);
    P.vy=0;
    P.grounded=true;
  } else P.grounded=false;
  P.pos.y=ny;
  if(key('Space')&&P.grounded&&P.stance===0&&!inWater){
    P.vy=5.3; P.grounded=false;
  }
  /* footsteps */
  var moveAmt=Math.hypot(wx,wz)*dt;
  P.stepT+=moveAmt;
  if(P.stepT>2.2&&P.grounded){
    P.stepT=0;
    sStep(inWater);
    if(!sprinting&&moveAmt>0) P.bobT+=0;
  }
  if(P.moveSpeed>0.5) P.bobT+=dt*(sprinting?11:8);
  /* camera */
  var eyeH=EYE[P.stance];
  P.eyeCur=lerp(P.eyeCur===undefined?eyeH:P.eyeCur,eyeH,dt*8);
  var bob=Math.sin(P.bobT*2)*.035*(P.moveSpeed>0.5?1:0);
  P.shake=Math.max(0,P.shake-dt*1.5);
  var shx=(Math.random()-.5)*P.shake*.12*SET.shake;
  var shy=(Math.random()-.5)*P.shake*.12*SET.shake;
  camera.position.set(P.pos.x+shx,P.pos.y+P.eyeCur+bob+shy,P.pos.z);
  camera.rotation.y=P.yaw;
  camera.rotation.x=clamp(P.pitch+(camera.pitchK||0),-1.45,1.45);
  camera.pitchK=(camera.pitchK||0)*Math.max(0,1-dt*8);
  camera.rotation.z=0;
  /* FOV / scope */
  var wantFov=P.scope?16:(P.aim?55:75);
  if(Math.abs(camera.fov-wantFov)>.1){
    camera.fov=lerp(camera.fov,wantFov,dt*10);
    camera.updateProjectionMatrix();
  }
  P.scope=w.scope&&P.aim;
  el('scope').style.display=P.scope?'block':'none';
  el('cross').classList.toggle('hide',P.scope);
  vmGroup.visible=!P.scope;
}
function fwdLike(){
  return P.touchVec&&P.touchVec.y<-.5;
}
