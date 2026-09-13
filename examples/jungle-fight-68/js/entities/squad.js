'use strict';
/* SECTION 11 — DELTA COMPANY */
var ORDERS={
  '1':{key:'follow',label:'FOLLOW ME',ack:'ROGER \u2014 MOVING OUT WITH YOU'},
  '2':{key:'hold',label:'HOLD POSITION',ack:'COPY \u2014 HOLDING THIS SPOT'},
  '3':{key:'assault',label:'ASSAULT',ack:'FIX BAYONETS \u2014 GO GO GO!'},
  '4':{key:'flankL',label:'FLANK LEFT',ack:'SWINGING WIDE LEFT'},
  '5':{key:'flankR',label:'FLANK RIGHT',ack:'SWINGING WIDE RIGHT'},
  '6':{key:'search',label:'SEARCH & DESTROY',ack:'COPY \u2014 SWEEPING THE JUNGLE'},
  '7':{key:'cover',label:'TAKE COVER',ack:'GETTING DOWN AND DIRTY'},
  '8':{key:'retreat',label:'FALL BACK',ack:'BUGGING BACK TO YOU, LT!'},
  '9':{key:'fire',label:'FIRE AT WILL',ack:'WEAPONS FREE'},
  '0':{key:'cease',label:'CEASE FIRE',ack:'CEASE FIRE \u2014 CEASE FIRE'}
};
var SURNAME=['ANDERSON','BAKER','CARTER','DAVIS','EVANS','FISHER','GARCIA','HUGHES','IRWIN','JENKINS',
  'KELLY','LEWIS','MARTINEZ','NOLAN','O\'BRIEN','PARKER','QUINN','REED','SULLIVAN','TUCKER',
  'VAUGHN','WALKER','XAVIER','YOUNG','ZIMMER','BROOKS','CHAVEZ','DIXON','ELLIS','FRANKLIN',
  'GRANT','HAYES','INGRAM','JORDAN','KNIGHT','LOPEZ','MURPHY','NASH','OWENS','PRICE',
  'RIVERA','SANDERS','TORRES','URBAN','VANCE','WEBB','WYATT','YATES','ZHOU','ADAMS',
  'BARNES','COLE','DUNN','EMERSON','FORD','GIBBS','HOLT','ISAAC','JONES','KANE'];
var RANKS=['SGT','CPL','CPL','PFC','PFC','PVT'];
var PLATOON_SIZE=20;
var SQUAD_WPN=[
  {dmg:52,burstMin:3,burstMax:4,cd:.55},
  {dmg:19,burstMin:5,burstMax:8,cd:.13},
  {dmg:34,burstMin:4,burstMax:7,cd:.19}
];
var pltBrains=[null,null,null];
var usNameCount=0;

function buildOrderMenu(){
  var box=el('cmdrows');
  box.innerHTML='';
  for(var k in ORDERS){
    (function(kk){
      var row=document.createElement('div');
      row.textContent=kk+' \u00b7 '+ORDERS[kk].label;
      row.setAttribute('data-key',kk);
      row.addEventListener('click',function(){ issueOrder(kk); });
      box.appendChild(row);
    })(k);
  }
}
function refreshOrderMenu(){
  var rows=el('cmdrows').children;
  for(var i=0;i<rows.length;i++){
    var k=rows[i].getAttribute('data-key');
    rows[i].classList.toggle('act',ORDERS[k].key===curOrder);
  }
}
function issueOrder(k){
  var o=ORDERS[k];
  if(!o) return;
  curOrder=o.key;
  if(o.key==='hold'){
    for(var i=0;i<soldiers.length;i++){
      var s=soldiers[i];
      if(s.platoon===2&&!s.dead) s.holdPt={x:s.x,z:s.z};
    }
  } else if(o.key==='cover'){
    for(var j=0;j<soldiers.length;j++){
      var sB=soldiers[j];
      if(sB.platoon===2&&!sB.dead) sB.coverWish=true;
    }
  } else if(o.key==='search'){
    for(var m=0;m<soldiers.length;m++){
      var sC=soldiers[m];
      if(sC.platoon===2&&!sC.dead){ sC.searchT=0; }
    }
  } else if(o.key==='fire') fireAtWill=true;
  else if(o.key==='cease') fireAtWill=false;
  killfeed('\u00BB 2ND PLT \u2014 '+o.ack);
  radioSay(o.ack);
  banner(o.label);
  toggleCmdMenu(false);
}

function makeSoldier(x,z,platoon,idx){
  var rank=RANKS[idx%RANKS.length];
  var name=rank+' '+SURNAME[(idx+platoon*7+usNameCount)%SURNAME.length];
  var medic=idx===1;
  if(medic) name+=' DOC';
  var wpnSeed=Math.random();
  var wIdx=wpnSeed<.5?0:(wpnSeed<.82?1:2);
  var slot=charAlloc(CHAR_POOLS.us);
  var s={
    x:x,z:z,y:0,
    hp:75,maxHp:75,dead:false,deadT:0,
    platoon:platoon,idx:idx,name:name,medic:medic,
    wpn:SQUAD_WPN[wIdx],
    stance:0,faceYaw:0,walkPh:rand(0,TAU),
    target:null,scanT:rand(0,.45),
    burst:0,burstCd:rand(.5,2),fireCd:0,
    shots:0,reloadT:0,
    shade:rand(.86,1.1),
    holdPt:null,searchT:0,searchPt:null,
    wounded:false,bleedT:0,patient:null,
    hitTint:0,_tint:0,flingV:null,
    posePitch:0,poseRoll:0,poseYOff:0,
    cover:null,underFireT:-99,bias:Math.random()<.5?-1:1,stuckT:0,
    croucher:Math.random()<.5,
    _moved:false,remove:false
  };
  s._pool=CHAR_POOLS.us; s._slot=slot; s._hidden=slot<0;
  s.y=heightAt(x+HALF,z+HALF);
  soldiers.push(s);
  return s;
}
function spawnPlatoons(){
  var offs=[-13,0,13];
  for(var p=0;p<3;p++){
    for(var i=0;i<PLATOON_SIZE;i++){
      var a=i/PLATOON_SIZE*TAU;
      var x=usBase.x+offs[p]+Math.cos(a)*(3+i%5*1.4);
      var z=usBase.z-4+Math.sin(a)*3;
      var sp=findClearSpot(x,z,6,1.7,.4);
      if(!sp) sp={x:x,y:heightAt(x+HALF,z+HALF),z:z};
      makeSoldier(sp.x,sp.z,p+1,i);
    }
    pltBrains[p]=makePlatoonBrain(p+1);
  }
  pltBrains[1]=null; /* yours is commanded by you */
}
function makePlatoonBrain(n){
  return {n:n,phase:'stage',t0:time,target:null,claimed:null,
    softT:-99,callT:0,artyCd:0,mortarCd:0,napalmCd:0,
    bloodied:false,stoodDown:false,withdraws:0,withdrawT:-99,
    defendT:-99,defendTarget:null,secureT:-99,rallyT:-99};
}
function makeReplacementAt(x,z){
  var counts=[0,0,0];
  for(var i=0;i<soldiers.length;i++)
    if(!soldiers[i].dead) counts[soldiers[i].platoon-1]++;
  var thin=0;
  for(var c=1;c<3;c++) if(counts[c]<counts[thin]) thin=c;
  usNameCount++;
  var sp=findClearSpot(x,z,6,1.7,.4);
  if(!sp) return null;
  var s=makeSoldier(sp.x,sp.z,thin+1,irand(2,5));
  s.name=s.name.split(' ')[0]+' R'+usNameCount;
  return s;
}

/* ---------- per-soldier update ---------- */
function updateSoldier(s,dt){
  if(s.dead){
    s.deadT+=dt;
    if(s.flingV){
      s.x+=s.flingV.x*dt; s.z+=s.flingV.z*dt;
      s.flingV.y-=14*dt;
      s.y+=s.flingV.y*dt;
      if(s.y<heightAt(s.x+HALF,s.z+HALF)){ s.y=heightAt(s.x+HALF,s.z+HALF); s.flingV=null; s.poseRoll=Math.PI/2; }
    }
    if(!s._hidden) charPose(s,0,0);
    return;
  }
  if(s.hitTint>0){ s.hitTint-=dt; s._tint=s.hitTint>0?1:0; }
  if(s.wounded){
    s.bleedT-=dt;
    if(s.bleedT<=0){ killSoldier(s,true); return; }
    if(Math.random()<dt*3) bloodDrip(s);
    /* crawl toward DOC */
    var doc=nearestDoc(s);
    if(doc){
      var dx=doc.x-s.x,dz=doc.z-s.z;
      var d=Math.hypot(dx,dz);
      if(d>1.6) moveSoldier(s,doc.x,doc.z,dt,.8);
    }
    s.posePitch=-.72; s.poseYOff=-.5;
    if(!s._hidden) charPose(s,0,0);
    return;
  }
  /* medic brain */
  if(s.medic) medicBrain(s,dt);
  /* target scan */
  s.scanT-=dt;
  if(s.scanT<=0){
    s.scanT=rand(.3,.45);
    soldierScan(s);
  }
  /* movement target */
  var mvx=null,mvz=null,run=false;
  if(s.platoon===2){
    var t=playerOrderTarget(s);
    if(t){ mvx=t.x; mvz=t.z; run=t.run; }
  } else {
    var bt=platoonMoveTarget(s);
    if(bt){ mvx=bt.x; mvz=bt.z; run=bt.run; }
  }
  if(mvx!==null){
    var dm=Math.hypot(mvx-s.x,mvz-s.z);
    if(dm>1.5) moveSoldier(s,mvx,mvz,dt,run?4.6:2.7);
    else s._moved=false;
  } else s._moved=false;
  /* stances */
  soldierStance(s);
  /* survival: flee shells */
  for(var wz=0;wz<warnZones.length;wz++){
    var Z=warnZones[wz];
    if(Math.hypot(Z.x-s.x,Z.z-s.z)<Z.r+2){
      moveSoldier(s,s.x+(s.x-Z.x),s.z+(s.z-Z.z),dt,4.6);
      break;
    }
  }
  if(typeof burnZones!=='undefined')
  for(var bz=0;bz<burnZones.length;bz++){
    var B=burnZones[bz];
    if(Math.hypot(B.x-s.x,B.z-s.z)<B.r+1.5){
      moveSoldier(s,s.x+(s.x-B.x),s.z+(s.z-B.z),dt,4.6);
      break;
    }
  }
  /* self-evac sister platoons */
  if(s.platoon!==2&&s.hp<35&&s.target){
    var back=Math.atan2(s.x-s.target.x,s.z-s.target.z);
    moveSoldier(s,s.x+Math.sin(back)*14,s.z+Math.cos(back)*14,dt,4.6);
  }
  /* fire discipline */
  soldierFire(s,dt);
  /* facing */
  var faceTo=null;
  if(s.target&&!s.target.dead) faceTo=Math.atan2(s.target.x-s.x,s.target.z-s.z);
  else if(s._moveDir!==undefined) faceTo=s._moveDir;
  if(faceTo!==null) s.faceYaw=turnTowards(s.faceYaw,faceTo,6*dt);
  s.walkPh+=dt*6*(s._moved?1:0);
  s._moved=false;
  s.y=heightAt(s.x+HALF,s.z+HALF);
  if(!s._hidden){
    if(IS_TOUCH&&Math.hypot(s.x-camera.position.x,s.z-camera.position.z)>115) charHide(s._pool,s._slot);
    else charPose(s,Math.sin(s.walkPh)*.5,Math.sin(s.walkPh+Math.PI)*.5);
  }
}
function nearestDoc(s){
  var best=null,bd=70;
  for(var i=0;i<soldiers.length;i++){
    var d=soldiers[i];
    if(d.dead||!d.medic||d===s) continue;
    var dd=Math.hypot(d.x-s.x,d.z-s.z);
    if(dd<bd){ bd=dd; best=d; }
  }
  return best;
}
function bloodDrip(s){
  spawnParticles(s.x,s.y+.4,s.z,1,0x8a0303,1);
}
function medicBrain(s,dt){
  /* triage */
  var patient=null,pd=70;
  for(var i=0;i<soldiers.length;i++){
    var w=soldiers[i];
    if(w.dead||!w.wounded||w===s) continue;
    var d=Math.hypot(w.x-s.x,w.z-s.z);
    if(d<pd){ pd=d; patient=w; }
  }
  if(!patient){
    pd=45;
    for(var j=0;j<soldiers.length;j++){
      var h=soldiers[j];
      if(h.dead||h.wounded||h===s||h.hp>=70) continue;
      if(h.platoon!==s.platoon&&s.platoon!==1) continue;
      var dH=Math.hypot(h.x-s.x,h.z-s.z);
      if(dH<pd){ pd=dH; patient=h; }
    }
  }
  s.patient=patient;
  if(patient){
    var dx=patient.x-s.x,dz=patient.z-s.z;
    var d=Math.hypot(dx,dz);
    if(d<1.9){
      /* treatment */
      if(patient.wounded&&patient.bleedT>25) patient.bleedT=Math.max(patient.bleedT,25);
      patient.hp=Math.min(85,patient.hp+8*dt);
      if(patient.wounded&&patient.hp>=25){
        patient.wounded=false;
        patient.hp=Math.max(patient.hp,25);
        killfeed('\u271A '+pltName(patient.platoon)+' '+patient.name+' STABILIZED \u2014 BACK IN THE FIGHT');
      }
      s._moved=false;
    } else moveSoldier(s,patient.x,patient.z,dt,4.6);
  } else {
    /* trail the centroid or LT */
    var cx=0,cz=0,cn=0;
    for(var k=0;k<soldiers.length;k++){
      var m=soldiers[k];
      if(m.dead||m.platoon!==s.platoon) continue;
      cx+=m.x; cz+=m.z; cn++;
    }
    if(cn){ cx/=cn; cz/=cn; }
    var tx=cx,tz=cz;
    var dC=Math.hypot(cx-s.x,cz-s.z);
    if(s.platoon===2||dC<2){ tx=P.pos.x; tz=P.pos.z; }
    moveSoldier(s,tx,tz,dt,dC>9||s.platoon===2&&Math.hypot(P.pos.x-s.x,P.pos.z-s.z)>9?4.6:2.7);
  }
}
function pltName(n){ return ['1ST','2ND','3RD'][n-1]; }

function playerOrderTarget(s){
  var myO=curOrder;
  var fw=Math.atan2(-Math.sin(P.yaw),-Math.cos(P.yaw));
  var toLT=Math.hypot(P.pos.x-s.x,P.pos.z-s.z);
  if(myO==='follow'){
    var ringR=3+(s.idx%5)*1.4;
    var a=Math.atan2(s.x-P.pos.x,s.z-P.pos.z);
    var tx=P.pos.x+Math.sin(fw)*-1.2+Math.cos(a)*ringR;
    var tz=P.pos.z+Math.cos(fw)*-1.2+Math.sin(a)*ringR;
    return {x:tx,z:tz,run:toLT>12};
  }
  if(myO==='hold'&&s.holdPt) return {x:s.holdPt.x,z:s.holdPt.z,run:toLT>14};
  if(myO==='cover'){
    if(!s.cover){
      s.cover=findEnemyCover({x:s.x,z:s.z,faceYaw:s.faceYaw},P.pos.x,P.pos.z);
      if(!s.cover) s.cover={x:s.x,z:s.z};
    }
    return {x:s.cover.x,z:s.cover.z,run:true};
  }
  if(myO==='assault'){
    var aA=(s.idx/PLATOON_SIZE)*TAU;
    return {x:P.pos.x+Math.sin(fw)*24+Math.cos(aA)*7,z:P.pos.z+Math.cos(fw)*24+Math.sin(aA)*7,run:true};
  }
  if(myO==='flankL'||myO==='flankR'){
    var side=myO==='flankL'?-1.2:1.2;
    var fa=fw+side;
    return {x:P.pos.x+Math.sin(fa)*20,z:P.pos.z+Math.cos(fa)*20,run:true};
  }
  if(myO==='search'){
    s.searchT-=1/60;
    if(s.searchT<=0||!s.searchPt){
      s.searchT=rand(5,9);
      var off=rand(6,26);
      var sa=fw+rand(-.6,.6)+(Math.random()<.5?1:-1)*rand(0,1.2);
      s.searchPt={x:P.pos.x+Math.sin(sa)*off,z:P.pos.z+Math.cos(sa)*off};
    }
    return {x:s.searchPt.x,z:s.searchPt.z,run:false};
  }
  if(myO==='retreat'){
    var aR=Math.atan2(s.x-P.pos.x,s.z-P.pos.z);
    return {x:P.pos.x+Math.cos(aR)*3,z:P.pos.z+Math.sin(aR)*3,run:true};
  }
  return null;
}
function platoonMoveTarget(s){
  var brain=pltBrains[s.platoon-1];
  if(!brain) return null;
  var br=brain;
  var ringR=(br.phase==='secure'?4+(s.idx%6)*1.6:(br.phase==='defend'?8+(s.idx%6)*1.8:(s.idx%6)*1.8));
  var a=(s.idx/PLATOON_SIZE)*TAU;
  if(s.coverAssign&&(br.phase==='stage'||br.phase==='defend'))
    return {x:s.coverAssign.x,z:s.coverAssign.z,run:Math.hypot(s.coverAssign.x-s.x,s.coverAssign.z-s.z)>8};
  return {x:br.x+Math.cos(a)*ringR,z:br.z+Math.sin(a)*ringR,
          run:Math.hypot(br.x-s.x,br.z-s.z)>14};
}

function soldierScan(s){
  var best=null,bd=68;
  var ey=s.y+(s.stance===2?.5:s.stance===1?1.1:1.5);
  for(var i=0;i<enemies.length;i++){
    var e=enemies[i];
    if(e.dead) continue;
    var d=Math.hypot(e.x-s.x,e.z-s.z);
    if(d>bd) continue;
    if(!hasLOS(s.x,ey,s.z,e.x,e.y+1.2,e.z)) continue;
    best=e; bd=d;
  }
  s.target=best;
  if(s.underFireT>time-1&&best===null&&!s.lastShooter){
    /* next scan locates the shooter */
    s.lastShooter=null;
  }
}
function soldierFire(s,dt){
  var canFire=s.platoon===2?fireAtWill:true;
  if(!canFire){ s.burst=0; return; }
  if(s.reloadT>0){ s.reloadT-=dt; return; }
  var t=s.target;
  if(!t||t.dead) { s.burst=0; return; }
  var d=Math.hypot(t.x-s.x,t.z-s.z);
  if(d>62){ s.burst=0; return; }
  var wantYaw=Math.atan2(t.x-s.x,t.z-s.z);
  var dyaw=Math.abs(((wantYaw-s.faceYaw+Math.PI*3)%TAU)-Math.PI);
  if(dyaw>.7){ s.burst=0; return; }
  if(s.burst<=0){
    s.burstCd-=dt;
    if(s.burstCd<=0){
      s.burst=irand(s.wpn.burstMin,s.wpn.burstMax);
      s.burstCd=rand(1.1,2.4);
    }
    return;
  }
  s.fireCd-=dt;
  if(s.fireCd>0) return;
  s.fireCd=s.wpn.cd;
  s.burst--;
  s.shots++;
  if(s.shots>=irand(16,28)){ s.shots=0; s.reloadT=2.6; }
  var mp=charMuzzle(s);
  var spread=(d/240+.016)*(s.stance===2?.5:s.stance===1?.65:1);
  var ty=t.y+1.1;
  var dx=t.x-mp.x,dy=ty-mp.y,dz=t.z-mp.z;
  var dl=Math.hypot(dx,dy,dz);
  dx/=dl;dy/=dl;dz/=dl;
  dx+=rand(-spread,spread);dy+=rand(-spread,spread);dz+=rand(-spread,spread);
  spawnProjectile(mp.x,mp.y,mp.z,dx*200,dy*200,dz*200,'squad',s.wpn.dmg);
  muzzleFX(mp);
  gunSoundPos(s.wpn.cd<.2?'smg':'garand',{x:mp.x,z:mp.z});
  gunshotEvent({x:mp.x,z:mp.z},38);
}
function soldierStance(s){
  var prone=false;
  if(time-shellHitT<2.5&&Math.hypot(shellHitX-s.x,shellHitZ-s.z)<22) prone=true;
  var engaging=s.target&&!s.target.dead;
  if(engaging&&Math.hypot(s.target.x-s.x,s.target.z-s.z)>30&&s.holdLike) prone=true;
  if(s.underFireT>time-1&&s.holdLike) prone=true;
  if(s.hp<35&&engaging) prone=true;
  var crouch=false;
  if(curOrder==='cover'&&s.platoon===2) crouch=true;
  if(engaging&&Math.hypot(s.target.x-s.x,s.target.z-s.z)>25&&(s.croucher||s.hp<50)) crouch=true;
  if(prone){ s.stance=2; s.posePitch=-.72; s.poseYOff=-.5; }
  else if(crouch){ s.stance=1; s.posePitch=-.34; s.poseYOff=-.3; }
  else { s.stance=0; s.posePitch=0; s.poseYOff=0; }
}
function moveSoldier(s,x,z,dt,speed){
  var dx=x-s.x,dz=z-s.z;
  var d=Math.hypot(dx,dz);
  if(d<.4) return;
  if(s.hp<40) speed*=.75;
  if(s.y<WATER) speed*=.55;
  var mv=speed*dt;
  /* separation */
  for(var i=0;i<soldiers.length;i++){
    var o=soldiers[i];
    if(o===s||o.dead) continue;
    var ox=s.x-o.x,oz=s.z-o.z;
    var od=Math.hypot(ox,oz);
    if(od<1.7&&od>.01){ dx+=ox/od*(1.7-od); dz+=oz/od*(1.7-od); }
  }
  var ltx=s.x-P.pos.x,ltz=s.z-P.pos.z;
  var ltd=Math.hypot(ltx,ltz);
  if(ltd<1.1&&ltd>.01){ dx+=ltx/ltd*(1.1-ltd); dz+=ltz/ltd*(1.1-ltd); }
  var dl=Math.hypot(dx,dz)||1;
  dx/=dl;dz/=dl;
  /* fan of collision probes */
  var offs=[0,.6,-.6,1.2,-1.2,1.9,-1.9,2.6,-2.6];
  for(var p=0;p<offs.length;p++){
    var ang=Math.atan2(dx,dz)+offs[p]*s.bias;
    var nx=s.x+Math.sin(ang)*mv, nz=s.z+Math.cos(ang)*mv;
    if(!collideAt(nx,s.y+.1,nz,1.6,.35,true)){
      s.x=nx;s.z=nz;s._moved=true;
      s._moveDir=Math.atan2(dx,dz);
      s.stuckT=0;
      return;
    }
  }
  s.stuckT+=dt;
  if(s.stuckT>.8){ s.bias*=-1; s.stuckT=0; }
}
function killSoldier(s,bleedOut){
  if(s.dead) return;
  s.dead=true; s.deadT=0;
  s.flingV={x:rand(-2,2),y:rand(1,3),z:rand(-2,2)};
  VC+=1; usKIA++; usWaveDead++;
  killfeed('\u271D '+pltName(s.platoon)+' '+s.name+' IS KIA \u25B2 +1 VC');
  bloodPool(s.x,s.z);
}
function damageSoldier(s,dmg,src){
  if(s.dead||s.wounded) return;
  s.hp-=dmg;
  s.hitTint=.12;
  spawnParticles(s.x,s.y+1,s.z,3,0x8a0303,2);
  s.underFireT=time;
  if(src) s.lastShooter=src;
  if(s.hp<=0){
    if(s.hp<=-18||dmg>=90) killSoldier(s);
    else{
      s.wounded=true;
      s.bleedT=55;
      s.posePitch=-.72; s.poseYOff=-.5;
      killfeed('\u2193 '+pltName(s.platoon)+' '+s.name+' IS DOWN \u2014 DOC NEEDED');
      medicCall(s);
    }
  }
}
var _lastMedicCall=-9;
function medicCall(s){
  if(time-_lastMedicCall<1.6) return;
  _lastMedicCall=time;
  var kind=Math.random()<.5?'medic':'mandown';
  var line=kind==='medic'?'Medic! Medic!':'Man down! Man down!';
  if(SET.voice&&window.speechSynthesis&&VOICES.length) radioSay(line,{prio:2});
  else sMedicCall(kind);
  /* echoes */
  var echoes=0;
  for(var i=0;i<soldiers.length&&echoes<2;i++){
    var o=soldiers[i];
    if(o===s||o.dead||o.wounded) continue;
    if(Math.hypot(o.x-s.x,o.z-s.z)<15){
      echoes++;
      (function(delay){
        setTimeout(function(){ sMedicCall(kind); },delay*1000);
      })(rand(.5,1));
    }
  }
}

/* ---------- AI platoon brain ---------- */
function updatePlatoonBrain(brain,dt){
  if(!brain) return;
  var mine=soldiers.filter(function(s){return s.platoon===brain.n&&!s.dead;});
  var alive=mine.length;
  if(!alive){ return; }
  var cx=0,cz=0;
  for(var i=0;i<alive;i++){ cx+=mine[i].x; cz+=mine[i].z; }
  cx/=alive; cz/=alive;
  if(brain.x===undefined){ brain.x=cx; brain.z=cz; }
  /* bloodied fallback */
  if(alive<14&&!brain.bloodied&&brain.phase!=='regroup'){
    brain.bloodied=true;
    brain.phase='regroup';
    brain.t0=time;
    killfeed('\u00BB '+pltName(brain.n)+' PLT IS FALLING BACK TO REGROUP');
  }
  if(brain.bloodied&&alive<10&&!brain.stoodDown){
    brain.stoodDown=true;
    brain.phase='rally';
    brain.rallyT=time;
    killfeed('TOO THIN \u2014 RALLYING ON YOU');
  }
  /* defense override */
  var hurt=null;
  for(var u=0;u<structList.length;u++){
    var st=structList[u];
    if(st.side!=='us'||st.dead) continue;
    if(time-st.hurtT<20){ hurt=st; break; }
  }
  if(hurt&&!brain.stoodDown&&(hurt.hp<hurt.maxHp*.5||hurt.label.indexOf('HQ')>=0||!brain.bloodied)){
    if(brain.defendTarget!==hurt){
      brain.defendTarget=hurt;
      brain.defendT=time;
      brain.phase='defend';
      killfeed('\u00BB '+pltName(brain.n)+' PLT FALLS BACK TO DEFEND '+hurt.label);
    }
  }
  if(brain.phase==='defend'&&hurt&&time-brain.defendT>30&&time-hurt.hurtT>30){
    brain.phase='stage';
    brain.defendTarget=null;
  }
  if(brain.phase==='defend'&&brain.defendTarget){
    brain.x=brain.defendTarget.x;
    brain.z=brain.defendTarget.z;
  }
  /* rally */
  if(brain.phase==='rally'){
    brain.x=P.pos.x; brain.z=P.pos.z;
    if(time-brain.rallyT>50&&alive>=8){
      brain.phase='stage';
      brain.stoodDown=false;
      brain.bloodied=false;
      killfeed('\u00BB '+pltName(brain.n)+' PLT BACK IN THE FIGHT');
    }
    return;
  }
  if(brain.phase==='regroup'){
    if(time-brain.t0>25){
      brain.phase='stage';
      brain.bloodied=false;
      brain.withdraws=0;
    } else { brain.x=cx; brain.z=cz; return; }
  }
  /* secure phase after capturing a bunker */
  if(brain.phase==='secure'){
    if(time-brain.secureT>22){ brain.phase='stage'; brain.target=null; }
    else return;
  }
  /* target pick */
  if(!brain.target||brain.target.dead||(brain.target.cleared&&brain.target!==vcPostStruct)){
    brain.target=null;
    var sister=pltBrains[brain.n===1?2:0];
    var bd=1e9;
    for(var b=0;b<bunkerList.length;b++){
      var bk=bunkerList[b];
      if(bk.dead||bk.cleared) continue;
      if(sister&&sister.target===bk) continue;
      var d=Math.hypot(bk.x-cx,bk.z-cz);
      if(d<bd){ bd=d; brain.target=bk; }
    }
    if(!brain.target){
      if(vcPostStruct&&!vcPostStruct.dead){
        brain.target=vcPostStruct;
      } else {
        killfeed('\u00BB '+pltName(brain.n)+' PLT RALLYING ON YOU \u2014 THE WAR IS WON \u2605');
        brain.phase='rally';
        brain.rallyT=time;
        return;
      }
    } else {
      brain.phase='stage';
      brain.t0=time;
      brain.softT=-99;
      /* stage point 85 out from bunker toward base */
      var dirX=usBase.x-brain.target.x;
      var dirZ=usBase.z-brain.target.z;
      var dl=Math.hypot(dirX,dirZ)||1;
      brain.x=brain.target.x+dirX/dl*85;
      brain.z=brain.target.z+dirZ/dl*85;
      brain.called=false;
    }
  }
  var tgt=brain.target;
  if(!tgt) return;
  var dT=Math.hypot(tgt.x-cx,tgt.z-cz);
  if(brain.phase==='stage'){
    /* cover assign */
    for(var m=0;m<mine.length;m++){
      var man=mine[m];
      if(man.coverAssign) continue;
      var ringA=(man.idx/PLATOON_SIZE)*TAU;
      var rx=brain.x+Math.cos(ringA)*6, rz=brain.z+Math.sin(ringA)*6;
      man.coverAssign=findEnemyCover({x:rx,z:rz,faceYaw:0},tgt.x,tgt.z);
      if(!man.coverAssign) man.coverAssign={x:rx,z:rz};
    }
    /* support call once after 1.5s waiting */
    if(!brain.called&&time-brain.t0>1.5){
      brain.called=true;
      aiSupportCall(brain,tgt);
    }
    var gunnerUp=false;
    for(var g=0;g<tgt.garrison.length;g++)
      if(tgt.garrison[g]&&!tgt.garrison[g].dead&&tgt.garrison[g].hmg){ gunnerUp=true; break; }
    var softened=time-brain.softT<9;
    var enemiesNear=0;
    for(var eN=0;eN<enemies.length;eN++){
      var en=enemies[eN];
      if(!en.dead&&Math.hypot(en.x-brain.x,en.z-brain.z)<38) enemiesNear++;
    }
    if(gunnerUp&&time-brain.t0>30&&brain.withdraws<2){
      brain.withdraws++;
      killfeed('\u00BB '+pltName(brain.n)+' PLT WITHDRAWS \u2014 THE .50 IS STILL UP');
      brain.bloodied=true;
      brain.phase='regroup';
      brain.t0=time;
      return;
    }
    if((!gunnerUp&&time-brain.t0>2&&(softened||enemiesNear<10))||time-brain.t0>22){
      brain.phase='assault';
      brain.t0=time;
    }
  } else if(brain.phase==='assault'){
    var dirX2=tgt.x-brain.x, dirZ2=tgt.z-brain.z;
    var dl2=Math.hypot(dirX2,dirZ2)||1;
    brain.x+=dirX2/dl2*3.1*dt;
    brain.z+=dirZ2/dl2*3.1*dt;
    if(tgt.dead||tgt.cleared){
      brain.phase='secure';
      brain.secureT=time;
      killfeed('\u00BB '+pltName(brain.n)+' PLT SECURES THE POSITION');
      /* open the door */
      if(tgt.door&&!tgt.door.open){ tgt.door.open=true; tgt.door.anim=1; }
    }
    /* frag objectives during assault */
    if(Math.random()<dt/4){
      sisterFragObjective(mine,tgt);
    }
  }
}
function sisterFragObjective(men,tgt){
  if(!tgt.garrison) return;
  var thrower=men[irand(0,men.length-1)];
  if(!thrower||thrower.dead) return;
  var victims=tgt.garrison.filter(function(g){return !g.dead;});
  if(!victims.length) return;
  var v=victims[irand(0,victims.length-1)];
  if(Math.hypot(v.x-thrower.x,v.z-thrower.z)<7||Math.hypot(v.x-thrower.x,v.z-thrower.z)>22) return;
  enemyGrenade(thrower,v.x,v.z);
}
function aiSupportCall(brain,tgt){
  if(time-brain.artyCd>55){
    brain.artyCd=time;
    killfeed('\u00BB '+pltName(brain.n)+' PLT RADIO: FIRE MISSION \u2014 155MM ON THE BUNKER');
    radioSay('Fire mission, fire mission! One five five on the bunker, splash in four!');
    for(var i=0;i<7;i++){
      (function(delay,k){
        pending.push({t:time+4+k*.4,fn:function(){
          var tx=tgt.x+rand(-13,13), tz=tgt.z+rand(-13,13);
          fireShell(tx+rand(-5,5),heightAt(tx+HALF,tz+HALF)+60,tz,tx,heightAt(tx+HALF,tz+HALF),tz,1.15,175,true,false);
        }});
      })(0,i);
    }
    brain.softT=time;
  } else if(time-brain.mortarCd>35){
    brain.mortarCd=time;
    killfeed('\u00BB '+pltName(brain.n)+' PLT: 60MM MORTARS \u2014 FIRE FOR EFFECT');
    radioSay('Sixty millimeters on the position, fire for effect!');
    for(var j=0;j<6;j++){
      (function(k){
        pending.push({t:time+2.5+k*.8,fn:function(){
          var tx=tgt.x+rand(-9,9), tz=tgt.z+rand(-9,9);
          fireShell(tx,heightAt(tx+HALF,tz+HALF)+40,tz,tx,heightAt(tx+HALF,tz+HALF),tz,1.0,120,true,false);
        }});
      })(j);
    }
    brain.softT=time;
  }
}
var _aiClusterT=0;
function aiClusterScan(dt){
  _aiClusterT-=dt;
  if(_aiClusterT>0) return;
  _aiClusterT=3;
  for(var b=0;b<2;b++){
    var brain=pltBrains[b===0?0:2];
    if(!brain||brain.phase!=='stage') continue;
    /* find cluster of >5 enemies within 12 of each other */
    var found=null;
    for(var i=0;i<enemies.length;i++){
      var e=enemies[i];
      if(e.dead) continue;
      if(Math.hypot(e.x-camera.position.x,e.z-camera.position.z)<40) continue;
      var near=Math.hypot(e.x-P.pos.x,e.z-P.pos.z);
      if(near<40) continue;
      var close=0;
      for(var j=0;j<enemies.length;j++){
        var f=enemies[j];
        if(f.dead) continue;
        if(Math.hypot(f.x-e.x,f.z-e.z)<12) close++;
      }
      if(close>5){
        var farFromSoldiers=true;
        for(var s=0;s<soldiers.length;s++)
          if(!soldiers[s].dead&&Math.hypot(soldiers[s].x-e.x,soldiers[s].z-e.z)<25){ farFromSoldiers=false; break; }
        if(farFromSoldiers){ found=e; break; }
      }
    }
    if(!found) continue;
    if(time-brain.napalmCd>110){
      brain.napalmCd=time;
      killfeed('\u00BB '+pltName(brain.n)+' PLT RADIO: NAPALM DROP \u2014 HEADS DOWN');
      radioSay('Sandy one-one, drop your napalm, heads down, heads down!');
      callAIPanelNapalm(found.x,found.z);
    } else if(time-brain.mortarCd>35){
      brain.mortarCd=time;
      killfeed('\u00BB '+pltName(brain.n)+' PLT: 60MM MORTARS \u2014 FIRE FOR EFFECT');
      radioSay('Sixty millimeters on the position, fire for effect!');
      for(var k=0;k<6;k++){
        (function(kk){
          pending.push({t:time+2.5+kk*.8,fn:function(){
            fireShell(found.x+rand(-9,9),heightAt(found.x+HALF,found.z+HALF)+40,found.z+rand(-9,9),
              found.x,heightAt(found.x+HALF,found.z+HALF),found.z,1.0,120,true,false);
          }});
        })(k);
      }
    }
  }
}
