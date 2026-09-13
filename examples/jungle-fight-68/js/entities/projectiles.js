'use strict';
/* SECTION 12 — PROJECTILES, EXPLOSIONS, PARTICLES, PICKUPS */
var tracerMesh=null;
var TRACER_COLORS={player:0xffe9a0,squad:0x9fd8ff,enemy:0xff9a5a,hmg:0xff9a5a};
function initProjectiles(){
  var geo=new THREE.BoxGeometry(.05,.05,.9);
  var mat=new THREE.MeshBasicMaterial({vertexColors:true,blending:THREE.AdditiveBlending,transparent:true,depthWrite:false});
  tracerMesh=new THREE.InstancedMesh(geo,mat,200);
  tracerMesh.instanceColor=new THREE.InstancedBufferAttribute(new F32(200*3).fill(1),3);
  tracerMesh.count=200;
  tracerMesh.frustumCulled=false;
  scene.add(tracerMesh);
  for(var i=0;i<200;i++){
    tracerMesh.setMatrixAt(i,_hideM);
  }
  projectiles._slots=[0,200];
  projectiles._next=0;
}
function spawnProjectile(x,y,z,vx,vy,vz,owner,dmg){
  if(!tracerMesh) return;
  var slot=-1;
  for(var i=0;i<200;i++){
    var s=(projectiles._next+i)%200;
    var free=true;
    for(var j=0;j<projectiles.length;j++)
      if(projectiles[j].slot===s){ free=false; break; }
    if(free){ slot=s; projectiles._next=s+1; break; }
  }
  if(slot<0) return;
  var c=new Col3(TRACER_COLORS[owner]||0xffe9a0);
  tracerMesh.instanceColor.array[slot*3]=c.r;
  tracerMesh.instanceColor.array[slot*3+1]=c.g;
  tracerMesh.instanceColor.array[slot*3+2]=c.b;
  tracerMesh.instanceMatrix.needsUpdate=true;
  tracerMesh.instanceColor.needsUpdate=true;
  projectiles.push({x:x,y:y,z:z,vx:vx,vy:vy,vz:vz,owner:owner,dmg:dmg,slot:slot,life:2,snapped:false});
}
function updateProjectiles(dt){
  if(!tracerMesh) return;
  var i;
  for(i=projectiles.length-1;i>=0;i--){
    var p=projectiles[i];
    p.life-=dt;
    if(p.life<=0){ tracerMesh.setMatrixAt(p.slot,_hideM); projectiles.splice(i,1); continue; }
    var ox=p.x,oy=p.y,oz=p.z;
    p.x+=p.vx*dt; p.y+=p.vy*dt; p.z+=p.vz*dt;
    /* supersonic snap near camera */
    if(!p.snapped&&p.owner!=='player'&&p.owner!=='squad'){
      var dx=p.x-camera.position.x,dy=p.y-camera.position.y,dz=p.z-camera.position.z;
      if(dx*dx+dy*dy+dz*dz<2.6*2.6){ p.snapped=true; sCrack({x:p.x,z:p.z}); }
    }
    /* hit tests */
    var hit=null;
    if(p.owner==='player'||p.owner==='squad'){
      hit=testHitEnemies(p,ox,oy,oz)||testHitSoldiersByFriendly(p)||testHitPlayerSafe(p)||
           testHitDeer(p,ox,oy,oz)||testHitBarrels(p)||testHitStructs(p);
    } else {
      hit=testHitPlayer(p,ox,oy,oz)||testHitSoldiers(p,ox,oy,oz)||testHitStructs(p)||testHitBarrels(p);
    }
    var wh=worldHit(ox,oy,oz,p.x,p.y,p.z,false);
    if(wh&&(!hit||wh.t<=hit.t)) hit={t:wh.t,surf:wh.surf};
    if(hit){
      var hx=ox+(p.x-ox)*hit.t, hy=oy+(p.y-oy)*hit.t, hz=oz+(p.z-oz)*hit.t;
      impactFX(hit.surf,hx,hy,hz,p);
      if(hit.entity) hit.apply();
      tracerMesh.setMatrixAt(p.slot,_hideM);
      projectiles.splice(i,1);
      continue;
    }
    /* orient tracer */
    var v=Math.hypot(p.vx,p.vy,p.vz)||1;
    _tmpObj.position.set(p.x,p.y,p.z);
    _tmpObj.lookAt(p.x+p.vx/v,p.y+p.vy/v,p.z+p.vz/v);
    _tmpObj.scale.set(1,1,1);
    _tmpObj.updateMatrix();
    tracerMesh.setMatrixAt(p.slot,_tmpObj.matrix);
  }
  tracerMesh.instanceMatrix.needsUpdate=true;
}
function testHitEnemies(p,ox,oy,oz){
  for(var i=0;i<enemies.length;i++){
    var e=enemies[i];
    if(e.dead||e.remove) continue;
    var hh=e.stance===2?.75:(e.stance===1?1.1:1.5);
    var hw=.45;
    var t=rayBox(ox,oy,oz,p.x,p.y,p.z,e.x-hw,e.y,e.z-hw,e.x+hw,e.y+hh,e.z+hw);
    if(t<0) continue;
    /* head box */
    var ht=rayBox(ox,oy,oz,p.x,p.y,p.z,e.x-.21,e.y+hh,e.z-.21,e.x+.21,e.y+hh+.3,e.z+.21);
    var head=ht>=0;
    var tt=head?Math.min(t,ht):t;
    return {t:tt,entity:e,apply:(function(en,hd){
      return function(){
        damageEnemy(en,p.dmg,p.owner==='player',{x:ox,z:oz},hd);
        if(p.owner==='player'){ shotsHit++; hitmark(false); }
      };
    })(e,head)};
  }
  return null;
}
function testHitSoldiers(p,ox,oy,oz){
  for(var i=0;i<soldiers.length;i++){
    var s=soldiers[i];
    if(s.dead||s.wounded) continue;
    var hh=s.stance===2?.75:(s.stance===1?1.1:1.5);
    var t=rayBox(ox,oy,oz,p.x,p.y,p.z,s.x-.4,s.y,s.z-.4,s.x+.4,s.y+hh,s.z+.4);
    if(t<0) continue;
    return {t:t,entity:s,apply:(function(sd){
      return function(){ damageSoldier(sd,p.dmg,{x:ox,z:oz}); };
    })(s)};
  }
  return null;
}
function testHitSoldiersByFriendly(p){
  /* friendly fire: rare — ignore for squad shots at soldiers */
  return null;
}
function testHitPlayerSafe(p){ return null; }
function testHitPlayer(p,ox,oy,oz){
  if(P.dead||P.spawnProtT>0) return null;
  var hh=HITBOX[P.stance];
  var t=rayBox(ox,oy,oz,p.x,p.y,p.z,P.pos.x-.35,P.pos.y,P.pos.z-.35,P.pos.x+.35,P.pos.y+hh,P.pos.z+.35);
  if(t<0) return null;
  return {t:t,apply:function(){ damagePlayer(p.dmg,{x:ox,z:oz}); }};
}
function testHitDeer(p,ox,oy,oz){
  for(var i=0;i<deers.length;i++){
    var d=deers[i];
    if(d.dead) continue;
    var t=rayBox(ox,oy,oz,p.x,p.y,p.z,d.x-.6,d.y,d.z-.8,d.x+.6,d.y+1.6,d.z+.8);
    if(t<0) continue;
    return {t:t,apply:(function(dd){
      return function(){ damageDeer(dd,p.dmg); };
    })(d)};
  }
  return null;
}
function testHitBarrels(p){
  for(var i=0;i<barrels.length;i++){
    var b=barrels[i];
    if(b.dead) continue;
    var t=rayBox(p.x,p.y,p.z,p.x,p.y,p.z,b.x-.45,b.y,b.z-.45,b.x+.45,b.y+.95,b.z+.45);
    if(t<0) continue;
    return {t:0,apply:(function(bb){
      return function(){ bb.fuse=rand(.05,.18); };
    })(b)};
  }
  return null;
}
function testHitStructs(p){
  var list=(p.owner==='enemy'||p.owner==='hmg')?structList:structList;
  for(var i=0;i<list.length;i++){
    var st=list[i];
    if(st.dead) continue;
    if(p.owner!=='player'&&p.owner!=='squad'&&st.side!=='us') continue;
    if(p.owner==='enemy'&&st.side!=='us') continue;
    if((p.owner==='player'||p.owner==='squad')&&st.side!=='vc') continue;
    var rr=st.r*.7;
    var t=rayBox(p.x,p.y,p.z,p.x,p.y,p.z,st.x-rr,heightAt(st.x+HALF,st.z+HALF),st.z-rr,st.x+rr,heightAt(st.x+HALF,st.z+HALF)+3.4,st.z+rr);
    if(t<0) continue;
    return {t:0,apply:(function(sst){
      return function(){
        if(sst.label==='BUNKER') damageBunker(sst,8);
        else damageStructure(sst,8);
      };
    })(st)};
  }
  /* bunkers via projectiles */
  for(var b=0;b<bunkerList.length;b++){
    var bk=bunkerList[b];
    if(bk.dead) continue;
    if(p.owner!=='player'&&p.owner!=='squad') continue;
    var rrB=bk.r*.7;
    var tB=rayBox(p.x,p.y,p.z,p.x,p.y,p.z,bk.x-rrB,bk.fy,bk.z-rrB,bk.x+rrB,bk.fy+3.4,bk.z+rrB);
    if(tB<0) continue;
    return {t:0,apply:(function(bkk){
      return function(){ damageBunker(bkk,8); };
    })(bk)};
  }
  return null;
}
function rayBox(ax,ay,az,bx,by,bz,x0,y0,z0,x1,y1,z1){
  var s={x0:x0,y0:y0,z0:z0,x1:x1,y1:y1,z1:z1};
  var t=segAABB(ax,ay,az,bx,by,bz,s);
  if(t<0) return -1;
  return t===0?0:t;
}
function impactFX(surf,x,y,z,p){
  var col=0x795548;
  if(surf==='stone') col=0x8a8a8a;
  else if(surf==='wood') col=0x8d6e4f;
  else if(surf==='water'){ col=0x6db3e8; sSplash(); }
  else if(surf==='flesh') col=0x8a0303;
  spawnParticles(x,y,z,surf==='water'?4:3,col,1.5);
  if(surf!=='water'&&surf!=='flesh') impactDecal(x,y,z,p);
}

/* ---------- particles ---------- */
var partMesh=null;
function initParticles(){
  var geo=new THREE.BoxGeometry(.13,.13,.13);
  var mat=new THREE.MeshLambertMaterial({vertexColors:true});
  partMesh=new THREE.InstancedMesh(geo,mat,260);
  partMesh.instanceColor=new THREE.InstancedBufferAttribute(new F32(260*3).fill(1),3);
  partMesh.count=260;
  partMesh.frustumCulled=false;
  for(var i=0;i<260;i++){ partMesh.setMatrixAt(i,_hideM); particles.push(null); }
}
var partNext=0;
function spawnParticles(x,y,z,n,color,power){
  if(!partMesh) return;
  for(var i=0;i<n;i++){
    var slot=partNext=(partNext+1)%260;
    var c=new Col3(color);
    c.multiplyScalar(rand(.7,1.2));
    partMesh.instanceColor.array[slot*3]=c.r;
    partMesh.instanceColor.array[slot*3+1]=c.g;
    partMesh.instanceColor.array[slot*3+2]=c.b;
    particles[slot]={x:x,y:y,z:z,vx:rand(-1,1)*power,vy:rand(.5,1.5)*power,vz:rand(-1,1)*power,life:rand(.5,1.2)};
    partMesh.setMatrixAt(slot,_tmpObj.matrix);
  }
  partMesh.instanceColor.needsUpdate=true;
}
function updateParticles(dt){
  if(!partMesh) return;
  var any=false;
  for(var i=0;i<260;i++){
    var p=particles[i];
    if(!p) continue;
    p.life-=dt;
    if(p.life<=0){ particles[i]=null; partMesh.setMatrixAt(i,_hideM); any=true; continue; }
    p.vy-=9*dt;
    p.x+=p.vx*dt; p.y+=p.vy*dt; p.z+=p.vz*dt;
    var g=heightAt(p.x+HALF,p.z+HALF);
    if(p.y<g){ p.y=g; p.vy*=-.3; p.vx*=.7; p.vz*=.7; }
    _tmpObj.position.set(p.x,p.y,p.z);
    _tmpObj.rotation.set(p.life*5,p.life*4,0);
    _tmpObj.scale.set(1,1,1);
    _tmpObj.updateMatrix();
    partMesh.setMatrixAt(i,_tmpObj.matrix);
    any=true;
  }
  if(any) partMesh.instanceMatrix.needsUpdate=true;
}

/* ---------- AI muzzle flashes ---------- */
var muzzleFXPool=[], mfxNext=0;
function initMuzzleFX(){
  var mat=new THREE.MeshBasicMaterial({color:0xffd080,transparent:true,opacity:0,blending:THREE.AdditiveBlending,depthWrite:false});
  for(var i=0;i<26;i++){
    var q=new THREE.Mesh(new THREE.PlaneGeometry(.5,.5),mat.clone());
    q.visible=false;
    scene.add(q);
    muzzleFXPool.push({m:q,t:0});
  }
}
function muzzleFX(pt){
  var fx=muzzleFXPool[mfxNext=(mfxNext+1)%26];
  fx.m.visible=true;
  fx.m.position.set(pt.x,pt.y,pt.z);
  fx.m.rotation.z=rand(0,TAU);
  var s=rand(.7,1.4);
  fx.m.scale.set(s,s,s);
  fx.m.material.opacity=1;
  fx.t=.055;
}
function updateMuzzleFX(dt){
  for(var i=0;i<muzzleFXPool.length;i++){
    var fx=muzzleFXPool[i];
    if(fx.t<=0) continue;
    fx.t-=dt;
    fx.m.material.opacity=Math.max(0,fx.t/.055);
    fx.m.lookAt(camera.position.x,camera.position.y,camera.position.z);
    if(fx.t<=0) fx.m.visible=false;
  }
}

/* ---------- grenades ---------- */
function updateGrenades(dt){
  for(var i=grenades.length-1;i>=0;i--){
    var g=grenades[i];
    g.fuse-=dt;
    g.vy-=9.8*dt;
    var nx=g.x+g.vx*dt, ny=g.y+g.vy*dt, nz=g.z+g.vz*dt;
    var gh=heightAt(nx+HALF,nz+HALF);
    if(ny<=gh+.1){
      ny=gh+.1;
      g.vy=Math.abs(g.vy)*.35;
      g.vx*=.7; g.vz*=.7;
    }
    if(ny<WATER&&gh<WATER){
      ny=WATER;
      g.vy=Math.abs(g.vy)*.3;
      g.vx*=.9;g.vz*=.9;
    }
    g.x=nx;g.y=ny;g.z=nz;
    if(g.fuse<=0){
      explode(g.x,g.y,g.z,7,160,g.owner==='player');
      grenades.splice(i,1);
    }
  }
}

/* ---------- explosions ---------- */
function explode(x,y,z,radius,dmg,byPlayer,crR,crD){
  var i;
  /* visual plume */
  for(i=0;i<14;i++)
    spawnParticles(x,y+.5,z,2,i%3===0?0xff5a10:0x8a8a8a,4);
  for(i=0;i<8;i++)
    spawnParticles(x,y+1,z,1,0x555555,2);
  expLight.position.set(x,y+2,z);
  expLight.intensity=9;
  sExplosion({x:x,z:z});
  if(crR) crater(x,z,crR,crD||Math.ceil(crR/2));
  /* shake */
  var dP=Math.hypot(x-P.pos.x,z-P.pos.z);
  if(dP<25) P.shake=Math.max(P.shake,(1-dP/25)*.8*SET.shake);
  /* player damage */
  if(!P.dead&&dP<radius){
    var pd=dmg*(1-dP/radius)*.6;
    if(P.stance===2) pd*=.55; else if(P.stance===1) pd*=.8;
    if(P.spawnProtT<=0) damagePlayer(pd,{x:x,z:z});
  }
  /* enemies */
  for(i=0;i<enemies.length;i++){
    var e=enemies[i];
    if(e.dead) continue;
    var d=Math.hypot(x-e.x,z-e.z);
    if(d>radius) continue;
    var ed=dmg*(1-d/radius);
    /* sheltered inside standing bunker concrete */
    if(e.bref&&!e.bref.dead){
      ed=Math.min(ed,dmg*.25);
      if(d>radius*.7) continue;
    } else if(d<radius*.7){
      e.flingV={x:(e.x-x)/(d||1)*6,y:5,z:(e.z-z)/(d||1)*6};
    }
    damageEnemy(e,ed,byPlayer,{x:x,z:z});
  }
  /* soldiers */
  for(i=0;i<soldiers.length;i++){
    var s=soldiers[i];
    if(s.dead) continue;
    var dS=Math.hypot(x-s.x,z-s.z);
    if(dS>radius) continue;
    if(s.wounded){ killSoldier(s); continue; }
    damageSoldier(s,dmg*(1-dS/radius)*.85,{x:x,z:z});
  }
  /* deer */
  for(i=0;i<deers.length;i++){
    var dd=deers[i];
    if(dd.dead) continue;
    var dD=Math.hypot(x-dd.x,z-dd.z);
    if(dD<radius) damageDeer(dd,dmg*(1-dD/radius));
  }
  /* structures */
  for(i=0;i<structList.length;i++){
    var st=structList[i];
    if(st.dead) continue;
    if(Math.hypot(x-st.x,z-st.z)<radius+st.r*.5)
      damageStructure(st,dmg*2.4*(byPlayer?1:1)*(st.side==='us'&&!byPlayer?1:1));
  }
  for(i=0;i<bunkerList.length;i++){
    var bk=bunkerList[i];
    if(bk.dead) continue;
    if(Math.hypot(x-bk.x,z-bk.z)<radius+bk.r*.5) damageBunker(bk,dmg*2.4);
  }
  /* barrels chain */
  for(i=0;i<barrels.length;i++){
    var b=barrels[i];
    if(b.dead) continue;
    if(Math.hypot(x-b.x,z-b.z)<radius) b.fuse=rand(.05,.18);
  }
  gunshotEvent({x:x,z:z},140);
}
function makeBarrel(x,y,z){
  var m=eBox(.9,.95,.9,0x8a5a2a,x,y+.48,z);
  barrels.push({x:x,y:y,z:z,mesh:m,dead:false,fuse:-1});
}
function updateBarrels(dt){
  for(var i=0;i<barrels.length;i++){
    var b=barrels[i];
    if(b.dead||b.fuse<0) continue;
    b.fuse-=dt;
    if(b.fuse<=0){
      b.dead=true;
      scene.remove(b.mesh);
      explode(b.x,b.y+.5,b.z,6.5,130,true,3,1.5);
    }
  }
}

/* ---------- pickups ---------- */
function spawnPickup(x,z){
  var m=eBox(.4,.3,.4,0x4a5a30,x,heightAt(x+HALF,z+HALF)+.3,z);
  m.material=new THREE.MeshLambertMaterial({color:0xcfe36a});
  pickups.push({x:x,z:z,y:heightAt(x+HALF,z+HALF),mesh:m});
}
function updatePickups(dt){
  for(var i=pickups.length-1;i>=0;i--){
    var p=pickups[i];
    p.mesh.position.y=p.y+.35+Math.sin(time*3+i)*.08;
    if(!P.dead&&Math.hypot(p.x-P.pos.x,p.z-P.pos.z)<1.8){
      P.pools.pistol=Math.min(POOLMAX.pistol,P.pools.pistol+21);
      P.pools.smg=Math.min(POOLMAX.smg,P.pools.smg+60);
      P.pools.rifle=Math.min(POOLMAX.rifle,P.pools.rifle+30);
      if(P.nades<5) P.nades++;
      scene.remove(p.mesh);
      pickups.splice(i,1);
      sPickup();
      updateAmmoHUD();
    }
  }
}
