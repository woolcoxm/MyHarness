'use strict';
/* SECTION 10 — VC ENEMY AI */
function makeEnemy(x,z,opts){
  opts=opts||{};
  var e={
    x:x,z:z,y:0,
    hp:opts.hp||85,maxHp:opts.hp||85,
    dead:false,deadT:0,
    state:'patrol',
    wx:x,wz:z,waitT:rand(0,3),
    faceYaw:opts.faceYaw!==undefined?opts.faceYaw:rand(0,TAU),
    losT:0,canSee:false,lastSeen:null,alertT:-99,
    fireCd:rand(1,2.5),burst:0,burstCd:rand(.5,2),
    walkPh:rand(0,TAU),flashT:0,
    flankOf:rand(-1,1),
    coverPt:null,coverSeekT:rand(0,4),
    nadeT:rand(6,16),
    garrison:opts.garrison||null,
    home:opts.home||{x:x,z:z},
    roam:opts.roam||26,
    remove:false,
    hmgRef:opts.hmg||null,
    hmg:!!opts.hmg,
    floorY:opts.floorY!==undefined?opts.floorY:null,
    post:!!opts.post,
    prone:!!opts.prone,
    stance:opts.prone?2:(opts.hmg?2:0),
    slitCd:rand(.5,2),
    bref:opts.bref||null,
    rpgCd:rand(4,9),
    rpg:Math.random()<.055&&!opts.garrison&&!opts.hmg&&!opts.post,
    shade:rand(.86,1.10),
    target:null,targetSoldier:null,
    suppress:0,hitTint:0,
    flingV:null,posePitch:0,poseRoll:0,poseYOff:0,
    _tint:0,
    assaultOrder:null,
    formation:null,
    wounded:false
  };
  var slot=charAlloc(CHAR_POOLS.vc);
  e._pool=CHAR_POOLS.vc; e._slot=slot;
  e._hidden=slot<0;
  if(slot<0){ e.remove=true; e._hidden=true; }
  if(vcPool<=0){ e.remove=true; }
  else vcPool--;
  e.vcSide=true;
  e.y=e.floorY!==null?e.floorY:heightAt(x+HALF,z+HALF);
  enemies.push(e);
  return e;
}
function charMuzzle(e){
  var mx=.12,my=1.2,mz=-1.05;
  var cos=Math.cos(e.faceYaw),sin=Math.sin(e.faceYaw);
  return {x:e.x+mx*cos+mz*sin,y:e.y+my,z:e.z-mx*sin+mz*cos};
}
function enemyEyeY(e){
  return e.y+(e.post?1.05:1.5);
}
function setStance(e,s){
  e.stance=s;
  e.posePitch=s===2?-.72:(s===1?-.34:0);
  if(s===2) e.poseYOff=-.5;
  else if(s===1) e.poseYOff=-.3;
  else e.poseYOff=0;
}
function updateEnemy(e,dt){
  if(e.dead){
    e.deadT+=dt;
    if(e.deadT<.8&&e.flingV){
      e.x+=e.flingV.x*dt;
      e.z+=e.flingV.z*dt;
      e.flingV.y-=14*dt;
      e.y+=e.flingV.y*dt;
      e.posePitch+=dt*6;
      if(e.y<heightAt(e.x+HALF,e.z+HALF)){ e.y=heightAt(e.x+HALF,e.z+HALF); e.flingV=null; }
    } else if(e.deadT>=.8){
      e.poseRoll=Math.PI/2;
      e.posePitch=0;
    }
    if(e.deadT>5) e.y-=dt*.5;
    if(e.deadT>8){
      if(!e._hidden) charFree(e._pool,e._slot);
      e.remove=true;
    } else if(!e._hidden){
      charPose(e,0,0);
    }
    return;
  }
  if(e.suppress>0) e.suppress-=dt;
  if(e.hitTint>0){ e.hitTint-=dt; e._tint=e.hitTint>0?1:0; }
  /* vision throttle */
  e.losT-=dt;
  if(e.losT<=0){
    e.losT=.22;
    enemyScan(e);
  }
  /* state */
  var seen=e.canSee;
  if(seen) e.state='attack';
  else if(time-e.alertT<10) e.state='hunt';
  else e.state='patrol';

  var spd=0;
  var mvx=0,mvz=0;

  /* assault orders for army men */
  if(e.assaultOrder){
    var tgt=e.assaultOrder;
    if(tgt.dead||usSpawns.indexOf(tgt)<0&&tgt!==usBaseStruct){
      e.assaultOrder=null;
    } else {
      var dA=Math.hypot(tgt.x-e.x,tgt.z-e.z);
      if(dA<9){
        e.sapperT=(e.sapperT||0)-dt;
        if(e.sapperT<=0){
          e.sapperT=rand(2.2,3.4);
          if(tgt===usBaseStruct||structList.indexOf(tgt)>=0) damageStructure(tgt,26);
          else damageBunker(tgt,26);
          spawnParticles(e.x,e.y+1,e.z,5,0xffe080,1);
          sThud({x:e.x,z:e.z});
        }
        setStance(e,1);
      } else {
        huntMove(e,tgt.x,tgt.z,dt,2.6);
      }
      finishEnemy(e,dt);
      return;
    }
  }
  /* formation slots for army men */
  if(e.formation){
    var f=e.formation;
    if(f.state==='muster'&&Math.hypot(f.x-e.x,f.z-e.z)>2.5)
      moveTo(e,f.x+e.formX,f.z+e.formZ,dt,2.4);
    else if(f.state==='march')
      moveTo(e,f.x+e.formX,f.z+e.formZ,dt,3.1);
    else if(f.state==='assault'&&(!seen))
      huntMove(e,f.x+e.formX,f.z+e.formZ,dt,2.6);
  }

  if(e.state==='patrol'){
    e.waitT-=dt;
    if(e.waitT<=0){
      e.waitT=rand(1,4);
      if(e.garrison){
        e.wx=rand(e.garrison.x0,e.garrison.x1);
        e.wz=rand(e.garrison.z0,e.garrison.z1);
      } else if(Math.random()<.4){
        var rr=e.roam*rand(.2,1);
        e.wx=e.home.x+rand(-rr,rr);
        e.wz=e.home.z+rand(-rr,rr);
      } else {
        e.wx=e.x+rand(-14,14);
        e.wz=e.z+rand(-14,14);
      }
    }
    if(Math.abs(e.wz-riverZ(e.wx))<4){ e.wx=e.x; e.wz=e.z; }
    moveTo(e,e.wx,e.wz,dt,1.5);
  } else if(e.state==='hunt'){
    var lx=e.lastSeen?e.lastSeen.x:e.home.x;
    var lz=e.lastSeen?e.lastSeen.z:e.home.z;
    var fx=lx+Math.cos(e.flankOf*1.9)*8;
    var fz=lz+Math.sin(e.flankOf*1.9)*8;
    var dh=Math.hypot(lx-e.x,lz-e.z);
    if(dh<4){ e.alertT=-99; }
    huntMove(e,fx,fz,dt,2.6);
    if(dh<48) setStance(e,2); else if(!e.post&&!e.hmg) setStance(e,0);
  } else if(e.state==='attack'){
    var t=e.target;
    var tx=t?t.x:(e.lastSeen?e.lastSeen.x:e.x);
    var tz=t?t.z:(e.lastSeen?e.lastSeen.z:e.z);
    var tAlive=t&&(t===P? !P.dead&&!P.spawnProtT:!t.dead);
    var dT=Math.hypot(tx-e.x,tz-e.z);
    if(dT<6&&!e.hmg) mvx=-1;
    else if(dT<14) mvx=(e.flankOf>0?1:-1);
    else if(dT<72){
      e.coverSeekT-=dt;
      if(e.coverSeekT<=0){
        e.coverSeekT=rand(4,8);
        e.coverPt=findEnemyCover(e,tx,tz);
      }
      if(e.coverPt) moveTo(e,e.coverPt.x,e.coverPt.z,dt,e.suppress>0?2.2:3.4);
      else moveTo(e,tx,tz,dt,e.suppress>0?2.2:3.4);
      if(e.coverPt&&Math.hypot(e.coverPt.x-e.x,e.coverPt.z-e.z)<1.5) setStance(e,1);
    } else moveTo(e,tx,tz,dt,2.2);
    if(mvx!==0){
      var perp=Math.atan2(tx-e.x,tz-e.z)+Math.PI/2*mvx;
      moveTo2(e,perp,dt,2.2);
    }
    if(!e.post&&!e.hmg&&dT<20) setStance(e,1);
  }

  /* flee warn zones and burn zones */
  for(var wz=0;wz<warnZones.length;wz++){
    var Z=warnZones[wz];
    if(Math.hypot(Z.x-e.x,Z.z-e.z)<Z.r){
      var ang=Math.atan2(e.x-Z.x,e.z-Z.z)+e.flankOf*.5;
      moveTo2(e,ang,dt,3.2);
      break;
    }
  }
  if(typeof burnZones!=='undefined')
  for(var bz=0;bz<burnZones.length;bz++){
    var B=burnZones[bz];
    if(Math.hypot(B.x-e.x,B.z-e.z)<B.r+2){
      moveTo2(e,Math.atan2(e.x-B.x,e.z-B.z),dt,3.2);
      break;
    }
  }

  /* dive prone after hostile shell */
  if(time-shellHitT<2.2&&Math.hypot(shellHitX-e.x,shellHitZ-e.z)<24&&!e.post&&!e.hmg)
    setStance(e,2);

  finishEnemy(e,dt);
  /* firing */
  enemyFire(e,dt);
  /* touch culling */
  if(!e._hidden){
    if(IS_TOUCH&&Math.hypot(e.x-camera.position.x,e.z-camera.position.z)>115){
      for(var pk in e._pool.parts){}
      charHide(e._pool,e._slot);
    } else charPose(e,Math.sin(e.walkPh)*.5,Math.sin(e.walkPh+Math.PI)*.5);
  }
  /* hmg pivot */
  if(e.hmgRef&&e.hmgRef.rotation) e.hmgRef.rotation.y=e.faceYaw+Math.PI;
}
function finishEnemy(e,dt){
  e.walkPh+=dt*6*(e._moved?1:0);
  if(e._moved) e._moved=false;
  /* turning */
  var turnTo=e._faceTarget!==undefined?e._faceTarget:e.faceYaw;
  var rate=e.hmg?2.4:4;
  if(e._moveDir!==undefined){
    e.faceYaw=turnTowards(e.faceYaw,e._moveDir,6*dt);
  } else if(e._faceTarget!==undefined){
    e.faceYaw=turnTowards(e.faceYaw,e._faceTarget,rate*dt);
  }
  e._faceTarget=undefined; e._moveDir=undefined;
  /* y position */
  if(e.floorY!==null&&e.floorY!==undefined) e.y=e.floorY;
  else e.y=heightAt(e.x+HALF,e.z+HALF);
  /* garrison clamp */
  if(e.garrison){
    e.x=clamp(e.x,e.garrison.x0,e.garrison.x1);
    e.z=clamp(e.z,e.garrison.z0,e.garrison.z1);
  }
}
function turnTowards(a,b,maxStep){
  var d=b-a;
  while(d>Math.PI)d-=TAU;
  while(d<-Math.PI)d+=TAU;
  if(Math.abs(d)<=maxStep) return b;
  return a+Math.sign(d)*maxStep;
}
function moveTo(e,x,z,dt,speed){
  var dx=x-e.x, dz=z-e.z;
  var d=Math.hypot(dx,dz);
  if(d<.4) return;
  var crawl=e.stance===2?speed*.5:speed;
  var mv=crawl*dt;
  var nx=e.x+dx/d*mv, nz=e.z+dz/d*mv;
  var step=e.stance===2?.3:1;
  if(!collideAt(nx,e.y+.1,nz,e.stance===2?.7:1.6,.35,true)){
    e.x=nx; e.z=nz; e._moved=true;
    e._moveDir=Math.atan2(dx,dz);
  } else {
    var alt=Math.atan2(dx,dz)+ (e.flankOf>0?1:-1)*1.2;
    var ax=e.x+Math.sin(alt)*mv, az=e.z+Math.cos(alt)*mv;
    if(!collideAt(ax,e.y+.1,az,e.stance===2?.7:1.6,.35,true)){
      e.x=ax; e.z=az; e._moved=true;
      e._moveDir=alt;
    }
  }
}
function moveTo2(e,ang,dt,speed){
  moveTo(e,e.x+Math.sin(ang)*3,e.z+Math.cos(ang)*3,dt,speed);
}
function huntMove(e,x,z,dt,speed){
  moveTo(e,x,z,dt,speed);
}
function findEnemyCover(e,tx,tz){
  var list=gatherSolids(Math.min(e.x,tx)-20,Math.min(e.z,tz)-20,Math.max(e.x,tx)+20,Math.max(e.z,tz)+20,true,0);
  var best=null,bestD=1e9;
  for(var i=0;i<list.length&&i<40;i++){
    var s=list[i];
    if(!s.los) continue;
    var cx=(s.x0+s.x1)/2, cz=(s.z0+s.z1)/2;
    var d=Math.hypot(cx-e.x,cz-e.z);
    if(d>25||d<1.5) continue;
    var dThreat=Math.hypot(tx-cx,tz-cz);
    if(dThreat<d) continue; /* must hold the far side */
    if(d<bestD){ bestD=d; best={x:cx,z:cz}; }
  }
  return best;
}
function enemyScan(e){
  var day=dayFactor!==undefined?dayFactor:1;
  var base=64*day+26*(1-day);
  var wet=weather&&(weather.kind==='RAIN'||weather.kind==='STORM');
  if(wet) base*=.75;
  if(e.hmg) base*=1.4;
  if(e.suppress>0) base*=.7;
  var range=base;
  /* player */
  var best=null,bestD=1e9;
  if(!P.dead){
    var pv=P.stance===2?.6:(P.stance===1?.8:1);
    var px=P.pos.x,py=P.pos.y+1.6,pz=P.pos.z;
    var d=Math.hypot(px-e.x,pz-e.z);
    var r2=range*pv;
    if(time-P.lastShotT<2&&d<90&&P.lastShotT>0) r2=Math.max(r2,60);
    if(d<r2){
      var fw={x:Math.sin(e.faceYaw),z:Math.cos(e.faceYaw)};
      var dot=((px-e.x)*fw.x+(pz-e.z)*fw.z)/(d||1);
      if(dot>.15||d<7){
        if(hasLOS(e.x,e.y+enemyEyeY(e),e.z,px,py,pz)) { best=P; bestD=d; }
      }
    }
  }
  /* soldiers */
  for(var i=0;i<soldiers.length;i++){
    var s=soldiers[i];
    if(s.dead||s.wounded) continue;
    var sv=s.stance===2?.5:(s.stance===1?.75:1);
    var dS=Math.hypot(s.x-e.x,s.z-e.z);
    if(dS>range*sv||dS>=bestD) continue;
    var fwS={x:Math.sin(e.faceYaw),z:Math.cos(e.faceYaw)};
    var dotS=((s.x-e.x)*fwS.x+(s.z-e.z)*fwS.z)/(dS||1);
    if(dotS<.05&&dS>7) continue;
    if(hasLOS(e.x,e.y+enemyEyeY(e),e.z,s.x,s.y+1.5,s.z)){ best=s; bestD=dS; }
  }
  if(best){
    e.target=best;
    e.canSee=true;
    e.lastSeen={x:best.x,z:best.z};
    e.alertT=time;
  } else {
    e.canSee=false;
    if(best===null&&e.target) e.target=null;
  }
}
function enemyFire(e,dt){
  if(e.state!=='attack'||!e.canSee||!e.target) { e.burst=0; return; }
  var t=e.target;
  var alive=t===P?!P.dead:!t.dead;
  if(!alive){ e.canSee=false; return; }
  var isP=t===P;
  if(isP&&P.spawnProtT>0) return;
  var d=Math.hypot(t.x-e.x,t.z-e.z);
  var maxR=e.hmg?115:82;
  if(d>maxR) return;
  /* aim window */
  var wantYaw=Math.atan2(t.x-e.x,t.z-e.z);
  var aimWin=e.hmg?.35:.55;
  var dyaw=Math.abs(((wantYaw-e.faceYaw+Math.PI*3)%TAU)-Math.PI);
  if(dyaw>aimWin) return;
  /* bursts */
  if(e.burst<=0){
    e.burstCd-=dt;
    if(e.burstCd<=0){
      e.burst=irand(e.hmg?8:4,e.hmg?14:8);
      e.burstCd=rand(e.hmg?1.3:.85,e.hmg?2.3:1.8);
    }
    return;
  }
  e.fireCd-=dt;
  if(e.fireCd>0) return;
  e.fireCd=e.hmg?.085:.115;
  e.burst--;
  var mp;
  if(e.post) mp={x:e.x,y:e.y+.5,z:e.z+1.6};
  else if(e.hmg&&e.bref&&e.bref.hmgMuzzle){
    var wp=e.bref.hmgMuzzle.getWorldPosition(_ewp);
    mp={x:wp.x,y:wp.y,z:wp.z};
  } else mp=charMuzzle(e);
  var ty=t.y+(isP?(P.stance===2?.4:1.3):1.2)-(isP?.2:0);
  var spread=(d/(e.hmg?130:80))+.02;
  var tgtMoving=isP?P.moveSpeed>1:(t._moved);
  if(tgtMoving) spread+=.03;
  if(e.stance===2) spread*=.6; else if(e.stance===1) spread*=.8;
  if(e.garrison) spread*=.75;
  if(e.suppress>0) spread*=1.5;
  var dx=t.x-mp.x,dy2=ty-mp.y,dz=t.z-mp.z;
  var dl=Math.hypot(dx,dy2,dz);
  dx/=dl;dy2/=dl;dz/=dl;
  dx+=rand(-spread,spread);dy2+=rand(-spread,spread);dz+=rand(-spread,spread);
  var sp=e.hmg?125:95;
  var dmg=e.hmg?irand(40,60):irand(26,40);
  var owner=e.hmg?'hmg':'enemy';
  spawnProjectile(mp.x,mp.y,mp.z,dx*sp,dy2*sp,dz*sp,owner,dmg);
  muzzleFX(mp);
  e.flashT=.05;
  gunSoundPos(e.hmg?'lmg':'ak',{x:mp.x,z:mp.z});
}
var _ewp=new THREE.Vector3();
/* firing ports */
function updateFiringPorts(e,dt){
  if(!e.garrison||e.dead||!e.bref||e.bref.dead) return;
  if(time-e.alertT>12) return;
  e.slitCd-=dt;
  if(e.slitCd>0) return;
  e.slitCd=rand(1.4,2.6);
  var bk=e.bref;
  var px=bk.x, py=bk.fy+1.2, pz=bk.z+bk.z1!==undefined?bk.z+4.6:bk.z+4.6;
  /* pick a forward target within 30 */
  var best=null,bestD=30;
  var fwd={x:Math.sin(e.faceYaw),z:Math.cos(e.faceYaw)};
  function consider(tx,tz,tobj){
    var dx=tx-px,dz=tz-pz;
    var d=Math.hypot(dx,dz);
    if(d>bestD) return;
    var dot=(dx*fwd.x+dz*fwd.z)/(d||1);
    if(dot<.5) return;
    best=tobj;bestD=d;
  }
  if(!P.dead) consider(P.pos.x,P.pos.z,P);
  for(var i=0;i<soldiers.length;i++)
    if(!soldiers[i].dead&&!soldiers[i].wounded) consider(soldiers[i].x,soldiers[i].z,soldiers[i]);
  if(!best) return;
  var ty2=best===P?P.pos.y+1.2:best.y+1.2;
  if(!hasLOS(px,py,pz,best.x,ty2,best.z)) return;
  var sp=95;
  var spread=.03*1.35;
  var dx=best.x-px,dy=ty2-py,dz=best.z-pz;
  var dl=Math.hypot(dx,dy,dz);
  dx/=dl;dy/=dl;dz/=dl;
  dx+=rand(-spread,spread);dy+=rand(-spread,spread);dz+=rand(-spread,spread);
  spawnProjectile(px,py,pz,dx*sp,dy*sp,dz*sp,'enemy',irand(26,40));
  muzzleFX({x:px,y:py,z:pz});
  gunSoundPos('ak',{x:px,z:pz});
}
/* grenades and RPG */
function updateEnemyNades(e,dt){
  if(e.garrison||e.post||e.hmg) return;
  e.nadeT-=dt;
  if(e.nadeT>0) return;
  e.nadeT=rand(9,16);
  if(!e.canSee||!e.target) return;
  var t=e.target;
  var d=Math.hypot(t.x-e.x,t.z-e.z);
  if(d<7||d>24) return;
  if(t===P&&P.moveSpeed>1) return;
  var tx=t.x+rand(-2.5,2.5), tz=t.z+rand(-2.5,2.5);
  enemyGrenade(e,tx,tz);
  e.alertT=time;
}
function enemyGrenade(e,tx,tz){
  var sx=e.x, sy=e.y+1.2, sz=e.z;
  var T=1.2;
  var dx=tx-sx, dz=tz-sz;
  var vx=dx/T, vz=dz/T;
  var vy=(0-sy+heightAt(tx+HALF,tz+HALF))/T+ .5*9.8*T;
  grenades.push({x:sx,y:sy,z:sz,vx:vx,vy:vy,vz:vz,fuse:2.4,owner:'enemy',snapped:false});
}
function updateEnemyRPG(e,dt){
  if(!e.rpg) return;
  e.rpgCd-=dt;
  if(e.rpgCd>0) return;
  e.rpgCd=rand(13,20);
  if(e.state!=='attack'||!e.canSee||!e.target) return;
  var t=e.target;
  var d=Math.hypot(t.x-e.x,t.z-e.z);
  if(d<16||d>58) return;
  var T=clamp(d/32,.55,2.1);
  var sy=e.y+1.3;
  fireShell(e.x,sy,e.z,t.x,(t===P?P.pos.y:t.y)+1,t.z,T,irand(85,125),false,true);
  spawnParticles(e.x,sy,e.z,6,0xbbaa88,2);
  killfeed('\u25B2 RPG TEAM \u2014 BACK BLAST, ROUND OUT!');
}
function damageEnemy(e,dmg,byPlayer,src,headshot){
  if(e.dead) return;
  e.hp-=dmg*(headshot?2.2:1);
  e.hitTint=.12;
  spawnParticles(e.x,e.y+1,e.z,4,0x8a0303,2);
  e.alertT=time;
  if(src) e.lastSeen={x:src.x,z:src.z};
  e.suppress=2.5;
  if(e.bref&&!e.bref.dead) e.bref.assaultT=time;
  if(e.garrison||e.bref) bunkerAlarm(e,src);
  if(e.hp<=0) killEnemy(e,byPlayer?'player':'other',headshot);
}
function bunkerAlarm(e,src){
  if(!src) return;
  for(var i=0;i<enemies.length;i++){
    var o=enemies[i];
    if(o===e||o.dead) continue;
    if(Math.hypot(o.x-e.x,o.z-e.z)<75){
      o.alertT=Math.max(o.alertT,time+15);
      o.lastSeen={x:src.x,z:src.z};
    }
  }
}
function killEnemy(e,who,headshot){
  if(e.dead) return;
  e.dead=true; e.deadT=0;
  e.flingV={x:rand(-3,3),y:rand(2,5),z:rand(-3,3)};
  vcWaveDead++;
  if(who==='player'){
    kills++;
    US+=1;
    vcKIA++;
    killfeed(headshot?'HEADSHOT \u2014 VC KIA':'VC KIA +1 \u2605');
    hitmark(true);
    sKillConfirm();
    if(Math.random()<.25) spawnPickup(e.x,e.z);
  } else if(who==='squad'){
    US+=1;
    vcKIA++;
    killfeed('SQUAD FIRE \u2014 VC KIA \u2605 +1');
  }
  bloodPool(e.x,e.z);
  checkBunkersCleared();
}
function gunshotEvent(pos,loud){
  for(var i=0;i<enemies.length;i++){
    var e=enemies[i];
    if(e.dead) continue;
    if(Math.hypot(e.x-pos.x,e.z-pos.z)<loud){
      if(!e.canSee){ e.alertT=Math.max(e.alertT,time); if(Math.random()<.4) e.lastSeen={x:pos.x,z:pos.z}; }
    }
  }
  wildlifeScare(pos,loud);
}

/* ---------- VC companies ---------- */
function updateVCArmy(dt){
  vcArmyT-=dt;
  if(vcArmy&&!vcArmy.done){
    var company=vcArmy;
    var alive=0;
    for(var i=0;i<company.men.length;i++) if(!company.men[i].dead&&!company.men[i].assaultOrder) alive++;
    if(company.phase==='muster'){
      var gathered=0;
      for(var m=0;m<company.men.length;m++){
        var man=company.men[m];
        if(man.dead) continue;
        if(Math.hypot(man.x-company.x,man.z-company.z)<16) gathered++;
      }
      var total=0;
      for(var mB=0;mB<company.men.length;mB++) if(!company.men[mB].dead) total++;
      if(gathered>=total*.7||time-company.t0>40){
        company.phase='march';
        killfeed('VC COMPANY ON THE MOVE \u2014 THEY ARE COMING FOR OUR '+company.target.label);
      }
    } else if(company.phase==='march'){
      var dirX=company.target.x-company.x;
      var dirZ=company.target.z-company.z;
      var dLen=Math.hypot(dirX,dirZ);
      if(dLen>1){
        company.x+=dirX/dLen*3.1*dt;
        company.z+=dirZ/dLen*3.1*dt;
      }
      var near=1e9;
      for(var mC=0;mC<company.men.length;mC++){
        var manC=company.men[mC];
        if(manC.dead) continue;
        near=Math.min(near,Math.hypot(manC.x-company.target.x,manC.z-company.target.z));
      }
      if(near<55||time-company.t0>240){
        company.phase='assault';
        banner('\u25B2 VC ASSAULT ON OUR '+company.target.label);
        for(var mA=0;mA<company.men.length;mA++){
          if(!company.men[mA].dead) company.men[mA].assaultOrder=company.target;
        }
      }
    } else if(company.phase==='assault'){
      if(company.target.dead){
        killfeed('THE VC COMPANY TOOK OUR '+company.target.label);
        company.done=true;
        vcArmyT=rand(40,70);
        for(var mD=0;mD<company.men.length;mD++)
          if(!company.men[mD].dead) company.men[mD].assaultOrder=null;
        return;
      }
    }
    /* break under 40% */
    var surv=0;
    for(var mE=0;mE<company.men.length;mE++) if(!company.men[mE].dead) surv++;
    if(surv<company.men.length*.4&&company.phase!=='done'){
      killfeed('VC COMPANY BROKEN \u2014 '+surv+' SURVIVORS');
      for(var mF=0;mF<company.men.length;mF++){
        var manF=company.men[mF];
        if(manF.dead) continue;
        manF.assaultOrder=null;
        manF.home={x:company.x,z:company.z};
        manF.state='patrol';
      }
      company.done=true;
      vcArmyT=rand(80,120);
    }
    return;
  }
  if(vcArmyT>0) return;
  /* start a new company */
  var free=0;
  for(var f=0;f<enemies.length;f++)
    if(!enemies[f].dead&&!enemies[f].garrison&&!enemies[f].formation) free++;
  if(free<26) { vcArmyT=20; return; }
  if(!vcSpawns.length||!usSpawns.length){ vcArmyT=30; return; }
  /* mass center of free VC */
  var mx=0,mz=0,mc=0;
  for(var g=0;g<enemies.length;g++){
    var eg=enemies[g];
    if(eg.dead||eg.garrison) continue;
    mx+=eg.x; mz+=eg.z; mc++;
  }
  if(mc<10){ vcArmyT=30; return; }
  mx/=mc; mz/=mc;
  /* nearest US base to the mass center */
  var target=usBaseStruct, td=1e9;
  for(var u=0;u<usSpawns.length;u++){
    var d2=Math.hypot(usSpawns[u].x-mx,usSpawns[u].z-mz);
    if(d2<td){ td=d2; target=usSpawns[u]; }
  }
  /* muster point: VC spigot nearest target */
  var muster=vcSpawns[0], md=1e9;
  for(var v=0;v<vcSpawns.length;v++){
    var d3=Math.hypot(vcSpawns[v].x-target.x,vcSpawns[v].z-target.z);
    if(d3<md){ md=d3; muster=vcSpawns[v]; }
  }
  /* conscript 24 nearest men to muster point */
  var byD=enemies.filter(function(en){return !en.dead&&!en.garrison&&!en.formation;});
  byD.sort(function(a,b){
    return Math.hypot(a.x-muster.x,a.z-muster.z)-Math.hypot(b.x-muster.x,b.z-muster.z);
  });
  var men=byD.slice(0,24);
  if(men.length<24){ vcArmyT=30; return; }
  for(var q=0;q<men.length;q++){
    var fr=rand(2,9), fa=rand(0,TAU);
    men[q].formation={x:muster.x,z:muster.z,state:'muster'};
    men[q].formX=Math.cos(fa)*fr;
    men[q].formZ=Math.sin(fa)*fr;
  }
  vcArmy={men:men,phase:'muster',x:muster.x,z:muster.z,target:target,t0:time,done:false};
  vcArmyT=1e9;
  killfeed('VC COMPANY FORMING \u2014 '+men.length+' MEN');
}
