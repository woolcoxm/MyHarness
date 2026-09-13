'use strict';
/* SECTION 8 — WEAPONS */
var WPN={
  shovel:{slot:0,name:'ENTRENCHING SHOVEL',melee:true,dmg:65,rate:.95,range:2.7},
  knife:{slot:0,name:'KA-BAR KNIFE',melee:true,dmg:45,rate:2.2,range:2.3},
  revolver:{slot:1,name:'M1917 REVOLVER',dmg:70,rate:1.6,mag:6,spread:.014,semi:true,reload:3.4,pool:'pistol',snd:'revolver',mob:1},
  mNineteenEleven:{slot:1,name:'M1911 SEMI-AUTO',dmg:42,rate:4.2,mag:7,spread:.02,semi:true,reload:2.2,pool:'pistol',snd:'pistol',mob:1},
  bolt:{slot:2,name:'M1903 SPRINGFIELD',dmg:120,rate:1,mag:5,spread:.004,semi:true,reload:3,pool:'rifle',snd:'bolt',mob:.92,boltT:.95,scope:true},
  sniper:{slot:2,name:'M21 SNIPER SYSTEM',dmg:130,rate:1.8,mag:20,spread:.0025,semi:true,reload:2.8,pool:'rifle',snd:'sniper',mob:.95,scope:true},
  garand:{slot:2,name:'M1 GARAND',dmg:65,rate:2.3,mag:8,spread:.009,semi:true,reload:2.4,pool:'rifle',snd:'garand',mob:.95,ping:true},
  thompson:{slot:3,name:'M1A1 THOMPSON',dmg:28,rate:11,mag:30,spread:.03,reload:2.6,pool:'smg',snd:'smg',mob:1},
  browning:{slot:3,name:'M1918 BAR',dmg:45,rate:6.5,mag:20,spread:.024,reload:3.2,pool:'rifle',snd:'lmg',mob:.85},
  nades:{slot:4,name:'FRAG GRENADES',nade:true}
};
var WKEYS=['shovel','knife','revolver','mNineteenEleven','bolt','sniper','garand','thompson','browning','nades'];
var POOLMAX={pistol:72,smg:240,rifle:150};

var viewmodels={}, vmGroup=null, vmMuzzle=null, vmFlash=null;

function buildViewmodels(){
  vmGroup=new ObjT();
  vmGroup.position.set(.34,-.3,-.5);
  camera.add(vmGroup);
  var flashMat=new THREE.MeshBasicMaterial({color:0xffd080,transparent:true,opacity:0,blending:THREE.AdditiveBlending,depthWrite:false});
  var armMat=new THREE.MeshLambertMaterial({color:0x4b5320});
  var handMat=new THREE.MeshLambertMaterial({color:0xc8a165});
  function mk(w,h,d,color,x,y,z,parent){
    var m=new THREE.Mesh(unitBoxGeo(),new THREE.MeshLambertMaterial({color:color}));
    m.scale.set(w,h,d); m.position.set(x,y,z);
    m.castShadow=false;
    (parent||vmGroup).add(m);
    return m;
  }
  function baseGroup(){
    var g=new ObjT();
    /* arm base: olive sleeve + skin hand */
    mk(.09,.09,.5,0x4b5320,0,-.05,.16,g);
    mk(.08,.08,.12,0xc8a165,.06,-.12,.28,g);
    return g;
  }
  function addMuzzle(g){
    var mz=new ObjT();
    mz.position.set(0,.03,-.75);
    g.add(mz);
    var fq=new THREE.Mesh(new THREE.PlaneGeometry(.34,.34),flashMat.clone());
    fq.position.set(0,0,0);
    mz.add(fq);
    g.userData.muzzle=mz;
    g.userData.flash=fq;
    return g;
  }
  /* shovel */
  var g=baseGroup();
  mk(.05,.05,.4,0x6b5836,0,0,-.15,g);
  mk(.16,.2,.05,0x8a8a8a,0,.02,-.4,g);
  viewmodels.shovel=addMuzzle(g);
  /* knife */
  g=baseGroup();
  mk(.04,.04,.3,0x2a2a2a,0,0,-.2,g);
  mk(.03,.1,.06,0x241c14,0,-.02,-.05,g);
  viewmodels.knife=addMuzzle(g);
  /* revolver */
  g=baseGroup();
  mk(.05,.07,.28,0x3a3a3a,0,.01,-.25,g);
  mk(.05,.12,.07,0x5d4033,0,-.06,-.1,g);
  mk(.035,.035,.14,0x2a2a2a,0,.035,-.42,g);
  viewmodels.revolver=addMuzzle(g);
  /* m1911 */
  g=baseGroup();
  mk(.05,.08,.26,0x333333,0,.01,-.24,g);
  mk(.05,.13,.06,0x2a2a2a,0,-.07,-.12,g);
  viewmodels.mNineteenEleven=addMuzzle(g);
  /* springfield bolt */
  g=baseGroup();
  mk(.05,.07,.7,0x5d4033,0,0,-.3,g);
  mk(.035,.035,.4,0x2a2a2a,0,.02,-.7,g);
  mk(.05,.05,.14,0x8a2a2a,0,.06,-.35,g);
  mk(.04,.12,.2,0x5d4033,0,-.06,-.02,g);
  viewmodels.bolt=addMuzzle(g);
  /* sniper */
  g=baseGroup();
  mk(.05,.07,.75,0x3a4128,0,0,-.3,g);
  mk(.035,.035,.45,0x2a2a2a,0,.02,-.72,g);
  mk(.05,.06,.22,0x222222,0,.075,-.3,g);
  var lens=mk(.03,.03,.02,0x4a9fff,0,.075,-.41,g);
  lens.material=new THREE.MeshBasicMaterial({color:0x4a9fff});
  mk(.05,.14,.2,0x3a4128,0,-.06,0,g);
  viewmodels.sniper=addMuzzle(g);
  /* garand */
  g=baseGroup();
  mk(.05,.07,.6,0x5d4033,0,0,-.25,g);
  mk(.035,.035,.3,0x2a2a2a,0,.02,-.6,g);
  mk(.04,.1,.18,0x5d4033,0,-.05,-.05,g);
  viewmodels.garand=addMuzzle(g);
  /* thompson */
  g=baseGroup();
  mk(.05,.08,.4,0x3a3a3a,0,0,-.2,g);
  mk(.03,.03,.2,0x2a2a2a,0,.02,-.48,g);
  mk(.045,.18,.06,0x333333,0,-.1,-.16,g);
  mk(.04,.1,.16,0x5d4033,0,-.05,0,g);
  viewmodels.thompson=addMuzzle(g);
  /* BAR */
  g=baseGroup();
  mk(.06,.09,.55,0x3a3a3a,0,0,-.25,g);
  mk(.04,.04,.35,0x2a2a2a,0,.03,-.62,g);
  mk(.05,.16,.07,0x333333,0,-.09,-.14,g);
  mk(.05,.12,.2,0x5d4033,0,-.05,.02,g);
  viewmodels.browning=addMuzzle(g);
  /* grenades */
  g=baseGroup();
  mk(.09,.13,.09,0x3c4a26,.02,-.02,-.2,g);
  mk(.05,.04,.05,0x8a8a8a,.02,.06,-.2,g);
  viewmodels.nades=addMuzzle(g);

  for(var k in viewmodels){
    viewmodels[k].visible=false;
    vmGroup.add(viewmodels[k]);
  }
  vmGroup.visible=false;
}
function attachViewmodel(key){
  for(var k in viewmodels) viewmodels[k].visible=(k===key);
}

/* ---------- firing ---------- */
var _shootDir=new THREE.Vector3();
function fireWeapon(){
  var w=WPN[P.cur];
  if(!w||w.melee||w.nade) return;
  if(w.mag<=0) return;
  w.mag--;
  P.fireCd=1/w.rate;
  if(w.boltT) P.boltT=w.boltT;
  P.recoil=1;
  camera.pitchK=(camera.pitchK||0)+(w.dmg>90?.032:.012);
  if(!w.auto) camera.pitchK=(camera.pitchK||0)+0;
  if(w.dmg>90) camera.pitchK=(camera.pitchK||0)+.02;
  P.shake=Math.max(P.shake,.15);
  P.flashT=.05;
  gunSound(w.snd);
  if(w.ping&&w.mag===0) setTimeout(sPing,250);
  /* spread */
  var stanceMul=P.stance===2?.55:(P.stance===1?.75:1);
  var moving=Math.hypot(P.velX||0,P.velZ||0)>.5;
  var spread=w.spread*stanceMul*(moving?1.6:1)*(P.aim?.4:1);
  if(time-P.lastHurtT<1.6) spread*=1.6;
  camera.getWorldDirection(_shootDir);
  _shootDir.x+=rand(-spread,spread);
  _shootDir.y+=rand(-spread,spread);
  _shootDir.z+=rand(-spread,spread);
  _shootDir.normalize();
  var ex=camera.position.x, ey=camera.position.y, ez=camera.position.z;
  spawnProjectile(ex+_shootDir.x*.45,ey+_shootDir.y*.45,ez+_shootDir.z*.45,
    _shootDir.x*260,_shootDir.y*260,_shootDir.z*260,'player',w.dmg);
  P.lastShotT=time;
  shotsFired++;
  gunshotEvent(camera.position,w.dmg>90?95:70);
  updateAmmoHUD();
}
function meleeSwing(){
  var w=WPN[P.cur];
  if(!w||!w.melee) return;
  if(P.meleeT>0) return;
  P.meleeT=.42;
  P.swingT=.42;
  sWhoosh();
  pending.push({t:time+.16,fn:function(){
    var fw=camera.getWorldDirection(_shootDir);
    var ex=camera.position.x, ey=camera.position.y, ez=camera.position.z;
    var hit=false;
    for(var i=0;i<enemies.length;i++){
      var e=enemies[i];
      if(e.dead) continue;
      var dx=e.x-ex, dz=e.z-ez, dy=(e.y+1)-ey;
      var d=Math.sqrt(dx*dx+dy*dy+dz*dz);
      if(d>w.range) continue;
      var dot=(dx*fw.x+dz*fw.z)/(Math.hypot(dx,dz)||1);
      if(dot<.4&&d>1) continue;
      damageEnemy(e,w.dmg,true,{x:ex,z:ez});
      hit=true;
    }
    for(var b=0;b<barrels.length;b++){
      var br=barrels[b];
      if(br.dead) continue;
      var dxB=br.x-ex, dzB=br.z-ez;
      if(Math.hypot(dxB,dzB)<w.range+1){ br.fuse=Math.min(br.fuse||9,rand(.05,.18)); hit=true; }
    }
    if(hit){ sThud(null); P.shake=Math.max(P.shake,.2); }
  }});
}
function throwGrenadePlayer(){
  if(P.nades<=0) return;
  if(P.nadeCd>0) return;
  P.nades--;
  P.nadeCd=.6;
  camera.getWorldDirection(_shootDir);
  var vx=_shootDir.x*13, vy=_shootDir.y*13+3.5, vz=_shootDir.z*13;
  vx+=(P.velX||0); vz+=(P.velZ||0);
  grenades.push({x:camera.position.x,y:camera.position.y,z:camera.position.z,
    vx:vx,vy:vy,vz:vz,fuse:2.4,owner:'player',bounced:false,snapped:false});
  sWhoosh();
  updateAmmoHUD();
}

function tryReload(){
  var w=WPN[P.cur];
  if(!w||w.melee||w.nade) return;
  if(P.reloadT>0) return;
  if(w.mag>=w.magMax) return;
  if(P.pools[w.pool]<=0) return;
  P.reloadT=w.reload;
  sReload(w.reload);
}
function finishReload(){
  var w=WPN[P.cur];
  if(!w||w.melee||w.nade) return;
  var need=w.magMax-w.mag;
  var have=P.pools[w.pool];
  var take=Math.min(need,have);
  w.mag+=take;
  P.pools[w.pool]-=take;
  updateAmmoHUD();
}
function selectSlot(n){
  var cands=[];
  for(var i=0;i<WKEYS.length;i++){
    var k=WKEYS[i];
    if(WPN[k].slot===n) cands.push(k);
  }
  if(!cands.length) return;
  var idx=0;
  if(P.cur&&WPN[P.cur]&&WPN[P.cur].slot===n){
    idx=(cands.indexOf(P.cur)+1)%cands.length;
  }
  switchWeapon(cands[idx]);
}
function cycleWeapon(dir){
  var idx=WKEYS.indexOf(P.cur);
  idx=(idx+dir+WKEYS.length)%WKEYS.length;
  switchWeapon(WKEYS[idx]);
}
function switchWeapon(key){
  if(P.cur===key) return;
  P.cur=key;
  P.fireCd=.25; P.reloadT=0; P.boltT=0; P.meleeT=0;
  sClick();
  attachViewmodel(key);
  updateAmmoHUD();
}

function updateWeapon(dt){
  var w=WPN[P.cur];
  if(!w) return;
  if(P.fireCd>0) P.fireCd-=dt;
  if(P.boltT>0) P.boltT-=dt;
  if(P.nadeCd>0) P.nadeCd-=dt;
  if(P.meleeT>0) P.meleeT-=dt;
  if(P.swingT>0) P.swingT-=dt;
  if(P.recoil>0) P.recoil=Math.max(0,P.recoil-dt*6);
  if(P.flashT>0) P.flashT-=dt;
  if(P.reloadT>0){
    P.reloadT-=dt;
    if(P.reloadT<=0) finishReload();
  }
  /* viewmodel anims */
  var vm=viewmodels[P.cur];
  if(vm){
    var bob=Math.sin(P.bobT*2)*.012*(moving()?1:0);
    var aimPose=P.aim&&!WPN[P.cur].scope;
    var tx=.34, ty=-.3+bob, tz=-.5;
    if(aimPose){ tx=.001; ty=-.222+bob*.5; tz=-.4; }
    if(P.reloadT>0){
      var rp=1-P.reloadT/(WPN[P.cur].reload||2.4);
      ty-=Math.sin(Math.PI*rp)*.16;
    }
    if(P.swingT>0){
      var sp2=P.swingT/.42;
      tx+=Math.sin(sp2*Math.PI)*.2;
      ty+=Math.sin(sp2*Math.PI)*.1;
      vm.rotation.z=Math.sin(sp2*Math.PI)*.9;
    } else vm.rotation.z=0;
    if(P.recoil>0){
      tz+=P.recoil*.09;
      vm.rotation.x=P.recoil*.16;
    } else vm.rotation.x=0;
    if(P.boltT>0){
      vm.rotation.x+=Math.sin((1-P.boltT/(WPN[P.cur].boltT||1))*Math.PI)*.3;
    }
    vmGroup.position.x=lerp(vmGroup.position.x,tx,dt*10);
    vmGroup.position.y=lerp(vmGroup.position.y,ty,dt*10);
    vmGroup.position.z=lerp(vmGroup.position.z,tz,dt*10);
    var fl=vm.userData.flash;
    if(fl) fl.material.opacity=P.flashT>0?.9:0;
    var mz=vm.userData.muzzle;
    if(mz&&P.flashT>0){
      muzzleLight.position.copy(mz.getWorldPosition(_wp));
      muzzleLight.intensity=2.2;
    }
    if(P.flashT<=0&&muzzleLight.intensity>0&&time-P.lastShotT>.08) muzzleLight.intensity*=Math.max(0,1-dt*22);
  }
  /* trigger logic */
  if(P.dead||paused||!started) return;
  var wantFire=P.trigger&&!cmdMenuOpen;
  if(w.melee){
    if(wantFire) meleeSwing();
  } else if(w.nade){
    if(wantFire&&!P.firedPress){ throwGrenadePlayer(); P.firedPress=true; }
  } else {
    if(w.semi){
      if(wantFire&&!P.firedPress&&P.fireCd<=0&&P.boltT<=0&&P.reloadT<=0){
        if(w.mag>0){ fireWeapon(); P.firedPress=true; }
        else if(!P.clickedEmpty){ sClick(); P.clickedEmpty=true; tryReload(); }
      }
    } else {
      if(wantFire&&P.fireCd<=0&&P.reloadT<=0){
        if(w.mag>0) fireWeapon();
        else if(!P.clickedEmpty){ sClick(); P.clickedEmpty=true; tryReload(); }
      }
    }
  }
  if(!P.trigger){ P.firedPress=false; P.clickedEmpty=false; }
}
var _wp=new THREE.Vector3();
function moving(){
  return Math.hypot(P.velX||0,P.velZ||0)>.5;
}
