'use strict';
/* SECTION 13 — INDIRECT FIRE, ARTILLERY, NAPALM */
var shells=[];
var artyCd=0, napalmCd=0;
var warnMarkers=[];
var burnZones=[];
var scorchDiscs=[];
var fireBoxes=[];
var jetPlane=null;

function fireShell(x0,y0,z0,tx,ty,tz,T,dmg,us,whistle){
  var mesh=new THREE.Mesh(unitBoxGeo(),matCache[0x2a2a2a]||new THREE.MeshLambertMaterial({color:0x2a2a2a}));
  mesh.scale.set(.22,.22,.5);
  mesh.position.set(x0,y0,z0);
  scene.add(mesh);
  var dx=tx-x0,dy=ty-y0,dz=tz-z0;
  var vx=dx/T, vz=dz/T;
  var vy=dy/T+.5*9.8*T;
  var sh={x:x0,y:y0,z:z0,vx:vx,vy:vy,vz:vz,T:T,fuse:T,dmg:dmg,us:us,mesh:mesh,
          tx:tx,ty:ty,tz:tz,whistle:!!whistle,whistled:false};
  shells.push(sh);
  if(whistle){
    warnZones.push({x:tx,z:tz,r:9,until:time+T+.4,col:us?'#ffc23a':'#ff3020'});
    var mk=makeWarnMarker(tx,tz,us?0xffc23a:0xff3020);
    warnMarkers.push({m:mk,until:time+T+.4});
  }
  return sh;
}
function makeWarnMarker(x,z,color){
  var m=new THREE.Mesh(new THREE.RingGeometry(6.5,7.5,24),
    new THREE.MeshBasicMaterial({color:color,transparent:true,opacity:.7,side:THREE.DoubleSide,depthWrite:false}));
  m.rotation.x=-Math.PI/2;
  m.position.set(x,heightAt(x+HALF,z+HALF)+.15,z);
  scene.add(m);
  return m;
}
function updateShells(dt){
  for(var i=shells.length-1;i>=0;i--){
    var s=shells[i];
    s.fuse-=dt;
    if(s.whistle&&!s.whistled&&s.fuse<=1.2){ s.whistled=true; sIncoming(); }
    s.vy-=9.8*dt;
    s.x+=s.vx*dt; s.y+=s.vy*dt; s.z+=s.vz*dt;
    s.mesh.position.set(s.x,s.y,s.z);
    s.mesh.lookAt(s.x+s.vx,s.y+s.vy,s.z+s.vz);
    /* direct hit test: .8 box over soldier/enemy/player */
    var det=false;
    if(s.vy<0){
      if(!P.dead&&Math.abs(s.x-P.pos.x)<.8&&Math.abs(s.z-P.pos.z)<.8&&Math.abs(s.y-(P.pos.y+1))<1.2) det=true;
      for(var e=0;!det&&e<enemies.length;e++){
        var en=enemies[e];
        if(en.dead) continue;
        if(Math.abs(s.x-en.x)<.8&&Math.abs(s.z-en.z)<.8&&Math.abs(s.y-(en.y+1))<1.2) det=true;
      }
      for(var so=0;!det&&so<soldiers.length;so++){
        var sd=soldiers[so];
        if(sd.dead) continue;
        if(Math.abs(s.x-sd.x)<.8&&Math.abs(s.z-sd.z)<.8&&Math.abs(s.y-(sd.y+1))<1.2) det=true;
      }
    }
    if(s.fuse<=0||det){
      scene.remove(s.mesh);
      shells.splice(i,1);
      var crR=s.us?4.5:3.8, crD=s.us?2.2:1.6;
      explode(s.x,s.y,s.z,7.5,s.dmg,false,crR,crD);
      if(!s.us){
        shellHitT=time; shellHitX=s.x; shellHitZ=s.z;
        /* staging sister platoon flips to assault */
        for(var b=0;b<2;b++){
          var brain=pltBrains[b===0?0:2];
          if(brain&&brain.phase==='stage'){
            for(var m=0;m<soldiers.length;m++){
              var man=soldiers[m];
              if(man.platoon===brain.n&&!man.dead&&Math.hypot(man.x-s.x,man.z-s.z)<14){
                brain.phase='assault';
                brain.t0=time;
                killfeed('\u00BB '+pltName(brain.n)+' PLT TAKING SHELLS \u2014 GOING IN!');
                break;
              }
            }
          }
        }
      }
    }
  }
  /* markers */
  for(var w=warnMarkers.length-1;w>=0;w--){
    var wm=warnMarkers[w];
    if(time>wm.until){
      scene.remove(wm.m);
      warnMarkers.splice(w,1);
    } else {
      var pulse=1+Math.sin(time*10)*.12;
      wm.m.scale.set(pulse,pulse,pulse);
      wm.m.material.opacity=.4+Math.abs(Math.sin(time*6))*.4;
    }
  }
  for(var z=warnZones.length-1;z>=0;z--)
    if(time>warnZones[z].until) warnZones.splice(z,1);
}

/* ---------- VC bunker mortars ---------- */
function updateMortars(dt){
  for(var b=0;b<bunkerList.length;b++){
    var bk=bunkerList[b];
    if(bk.dead||bk.cleared) continue;
    if(!bk.mortar) continue;
    bk.mortar.cd-=dt;
    if(bk.mortar.cd>0) continue;
    /* spotter? */
    var spotter=false;
    for(var g=0;g<bk.garrison.length;g++){
      var man=bk.garrison[g];
      if(man&&!man.dead&&time-man.alertT<12){ spotter=true; break; }
    }
    if(!spotter){
      for(var e=0;e<enemies.length;e++){
        var en=enemies[e];
        if(en.dead||en.garrison) continue;
        if(time-en.alertT<12&&Math.hypot(en.x-bk.x,en.z-bk.z)<60){ spotter=true; break; }
      }
    }
    var dLT=Math.hypot(P.pos.x-bk.x,P.pos.z-bk.z);
    if(!spotter&&dLT<75&&time-P.lastShotT<3) spotter=true;
    if(!spotter){ bk.mortar.cd=2; continue; }
    /* final protective fire after assault */
    var fpf=time-bk.assaultT<14;
    var tx=null,tz=null;
    if(fpf){
      tx=bk.x+rand(-12,12);
      tz=bk.z+rand(-12,12)+8;
    } else {
      /* densest friendly cluster within 10 counting >=3, 35-95 band */
      var best=null,bestN=2;
      var pool=[{x:P.pos.x,z:P.pos.z,wt:P.dead?0:2}];
      for(var s=0;s<soldiers.length;s++)
        if(!soldiers[s].dead&&!soldiers[s].wounded) pool.push({x:soldiers[s].x,z:soldiers[s].z,wt:1});
      for(var p=0;p<pool.length;p++){
        var c=pool[p];
        var dC=Math.hypot(c.x-bk.x,c.z-bk.z);
        if(cCheat(c)) continue;
        if(dC<35||dC>95) continue;
        var n=c.wt;
        for(var q=0;q<pool.length;q++)
          if(q!==p&&Math.hypot(pool[q].x-c.x,pool[q].z-c.z)<10) n+=pool[q].wt;
        if(n>bestN){ bestN=n; best=c; }
      }
      function cCheat(){ return false; }
      if(!best){
        /* hunt you directly half the time at 35-55 */
        if(dLT>=35&&dLT<=55&&Math.random()<.5){ best={x:P.pos.x,z:P.pos.z}; }
      }
      if(!best){ bk.mortar.cd=rand(3.5,6); continue; }
      tx=best.x+rand(-5,5);
      tz=best.z+rand(-5,5);
    }
    var T=2.4;
    fireShell(bk.x,bk.fy+1,bk.z+4.6,tx,heightAt(tx+HALF,tz+HALF),tz,T,190,false,true);
    sMortarFire({x:bk.x,z:bk.z});
    bk.mortar.rounds++;
    if(bk.mortar.rounds>=5){ bk.mortar.rounds=0; bk.mortar.cd=rand(14,24); }
    else bk.mortar.cd=rand(3.5,6);
  }
}

/* ---------- player radio support ---------- */
function playerArtillery(){
  if(artyCd>0) return;
  var dir=camera.getWorldDirection(_artDir);
  var ox=camera.position.x, oy=camera.position.y, oz=camera.position.z;
  var best=null;
  for(var d=25;d<=130;d+=2){
    var x=ox+dir.x*d, y=oy+dir.y*d, z=oz+dir.z*d;
    var g=heightAt(x+HALF,z+HALF);
    if(y<=g){ best={x:x,z:z}; break; }
  }
  if(!best) best={x:ox+dir.x*80,z:oz+dir.z*80};
  var dP=Math.hypot(best.x-P.pos.x,best.z-P.pos.z);
  if(dP<15){
    killfeed('DANGER CLOSE \u2014 REQUEST REFUSED');
    return;
  }
  artyCd=40;
  killfeed('\u00BB FIRE MISSION! 155MM GRID '+(best.x|0)+'-'+(best.z|0)+' \u2014 SPLASH IN 4');
  radioSay('Fire mission! Guns, fire mission \u2014 splash in four seconds!');
  for(var i=0;i<9;i++){
    (function(k){
      pending.push({t:time+4+k*.38,fn:function(){
        var tx=best.x+rand(-9,9), tz=best.z+rand(-9,9);
        var ty=heightAt(tx+HALF,tz+HALF);
        fireShell(tx-18,ty+72,tz-12,tx,ty,tz,1.15,175,true,false);
      }});
    })(i);
  }
}
var _artDir=new THREE.Vector3();
function playerNapalm(){
  if(napalmCd>0) return;
  var dir=camera.getWorldDirection(_artDir);
  var ox=camera.position.x, oy=camera.position.y, oz=camera.position.z;
  var best=null;
  for(var d=25;d<=130;d+=2){
    var x=ox+dir.x*d, y=oy+dir.y*d, z=oz+dir.z*d;
    var g=heightAt(x+HALF,z+HALF);
    if(y<=g){ best={x:x,z:z}; break; }
  }
  if(!best) best={x:ox+dir.x*80,z:oz+dir.z*80};
  var dP=Math.hypot(best.x-P.pos.x,best.z-P.pos.z);
  if(dP<18){
    killfeed('DANGER CLOSE \u2014 REQUEST REFUSED');
    return;
  }
  napalmCd=60;
  killfeed('\u00BB NAPALM, DANGER CLOSE \u2014 BIRDS INBOUND, HEADS DOWN');
  radioSay('Air support, napalm on my mark \u2014 everybody heads down!');
  flyJet(best.x,best.z,true);
}
function callAIPanelNapalm(x,z){
  flyJet(x,z,false);
}

/* ---------- A-1 Skyraider ---------- */
function flyJet(tx,tz,byPlayer){
  var g=new ObjT();
  eBox(1.3,1.2,5.4,0x4a5a40,0,0,0,g);
  eBox(1,.7,1.4,0x22304a,0,.6,.4,g);
  eBox(9,.2,2.2,0x3e4c36,0,0,-.4,g);
  eBox(3.4,.16,1.2,0x3e4c36,0,.2,2.4,g);
  eBox(.18,1.8,1.4,0x3e4c36,0,1,-2.6,g);
  eBox(.5,.5,2.2,0x555555,-2.6,-.3,-.4,g);
  eBox(.5,.5,2.2,0x555555,2.6,-.3,-.4,g);
  eBox(.6,.6,2.6,0x8a8a8a,0,-.7,.4,g);
  var prop=eBox(3,.1,.3,0x222222,0,0,2.9,g);
  var side=Math.random()<.5?1:-1;
  var startY=heightAt(tx+HALF,tz+HALF)+14+6;
  g.position.set(tx-side*160,startY,tz);
  g.rotation.y=side>0?-Math.PI/2:Math.PI/2;
  scene.add(g);
  jetPlane={g:g,tx:tx,tz:tz,t:0,dur:4.8,side:side,prop:prop,byPlayer:byPlayer,dropped:false};
  sFlyby('jet',g.position);
}
function updateSupport(dt){
  if(artyCd>0) artyCd-=dt;
  if(napalmCd>0) napalmCd-=dt;
  updateShells(dt);
  updateMortars(dt);
  /* jet */
  if(jetPlane){
    var j=jetPlane;
    j.t+=dt;
    var prog=j.t/j.dur;
    var px=j.tx-j.side*160+j.side*320*prog;
    j.g.position.x=px;
    j.g.position.y=heightAt(px+HALF,j.tz+HALF)+14+6+Math.sin(j.t*2)*2;
    j.prop.rotation.z+=dt*40;
    if(!j.dropped&&j.t>4){
      j.dropped=true;
      tone({type:'sine',f:1800,f2:600,dur:1.1,vol:.1,exp:true});
    }
    if(j.t>=5.15){
      /* impact */
      var y=heightAt(j.tx+HALF,j.tz+HALF);
      explode(j.tx,y+1,j.tz,11,90,j.byPlayer,4,1);
      makeBurnZone(j.tx,j.tz,9,14,true);
      var nSpots=irand(1,3);
      for(var s=0;s<nSpots;s++){
        var bx=j.tx+rand(-14,14), bz=j.tz+rand(-14,14);
        makeBurnZone(bx,bz,2.2,10,false);
      }
      scene.remove(j.g);
      jetPlane=null;
    }
  }
  updateBurnZones(dt);
  updateStructureSmoke(dt);
  /* HUD support line */
  var txt='[T] 155MM '+(artyCd>0?Math.ceil(artyCd)+'s':'READY')+
          ' \u00b7 [Y] NAPALM '+(napalmCd>0?Math.ceil(napalmCd)+'s':'READY');
  el('supline').textContent=txt;
}

/* ---------- fire zones ---------- */
var _fireLightA=null,_fireLightB=null;
function makeBurnZone(x,z,r,dur,storm){
  var n=Math.min(IS_TOUCH?8:14,Math.max(4,Math.floor(r*r*.6)));
  var boxes=[];
  var mats=[new THREE.MeshBasicMaterial({color:0xff5a10,transparent:true,opacity:.85,blending:THREE.AdditiveBlending,depthWrite:false}),
            new THREE.MeshBasicMaterial({color:0xffc23a,transparent:true,opacity:.85,blending:THREE.AdditiveBlending,depthWrite:false})];
  for(var i=0;i<n;i++){
    var m=new THREE.Mesh(unitBoxGeo(),mats[i%2]);
    var fx=x+rand(-r,r)*.8, fz=z+rand(-r,r)*.8;
    m.position.set(fx,heightAt(fx+HALF,fz+HALF)+1,fz);
    m.scale.set(rand(.8,1.6),rand(1.5,3),rand(.8,1.6));
    scene.add(m);
    boxes.push(m);
  }
  if(!IS_TOUCH){
    if(!_fireLightA){
      _fireLightA=new THREE.PointLight(0xff6a20,0,20); scene.add(_fireLightA);
      _fireLightB=new THREE.PointLight(0xff6a20,0,20); scene.add(_fireLightB);
    }
  }
  /* scorch disc */
  var disc=new THREE.Mesh(new THREE.CircleGeometry(r*.85,14),
    new THREE.MeshBasicMaterial({color:0x171008,transparent:true,opacity:.8,depthWrite:false}));
  disc.rotation.x=-Math.PI/2;
  disc.position.set(x,heightAt(x+HALF,z+HALF)+.06,z);
  scene.add(disc);
  if(scorchDiscs.length>=48){
    var old=scorchDiscs.shift();
    scene.remove(old);
  }
  scorchDiscs.push(disc);
  burnZones.push({x:x,z:z,r:r,until:time+dur,boxes:boxes,storm:storm,tick:0});
}
function updateBurnZones(dt){
  var li=0;
  for(var i=burnZones.length-1;i>=0;i--){
    var B=burnZones[i];
    if(time>B.until){
      for(var b=0;b<B.boxes.length;b++) scene.remove(B.boxes[b]);
      burnZones.splice(i,1);
      continue;
    }
    /* flicker */
    for(var f=0;f<B.boxes.length;f++){
      B.boxes[f].scale.y=1.5+Math.abs(Math.sin(time*9+f*2))*1.4;
      if(Math.random()<dt*4)
        spawnParticles(B.boxes[f].position.x,B.boxes[f].position.y+2,B.boxes[f].position.z,1,0x555555,1.5);
    }
    if(_fireLightA){
      _fireLightA.position.set(B.x,heightAt(B.x+HALF,B.z+HALF)+2,B.z);
      _fireLightA.intensity=1.5+Math.sin(time*11)*.6;
    }
    /* damage ticks every .4s */
    B.tick-=dt;
    if(B.tick<=0){
      B.tick=.4;
      var dmg=B.storm?18:10;
      for(var e=0;e<enemies.length;e++){
        var en=enemies[e];
        if(en.dead) continue;
        if(en.bref&&!en.bref.dead) continue;
        if(Math.hypot(en.x-B.x,en.z-B.z)<B.r)
          damageEnemy(en,dmg,false,{x:B.x,z:B.z});
      }
      for(var s=0;s<soldiers.length;s++){
        var sd=soldiers[s];
        if(sd.dead) continue;
        if(Math.hypot(sd.x-B.x,sd.z-B.z)<B.r) damageSoldier(sd,dmg*.8,{x:B.x,z:B.z});
      }
      if(!P.dead&&Math.hypot(P.pos.x-B.x,P.pos.z-B.z)<B.r) damagePlayer(dmg,{x:B.x,z:B.z});
      for(var d=0;d<deers.length;d++){
        var dd=deers[d];
        if(!dd.dead&&Math.hypot(dd.x-B.x,dd.z-B.z)<B.r) damageDeer(dd,dmg);
      }
    }
  }
}
function updateStructureSmoke(dt){
  for(var i=0;i<smokingStructs.length;i++){
    var st=smokingStructs[i];
    if(st.dead){ smokingStructs.splice(i,1); i--; continue; }
    if(Math.random()<dt*4){
      var y=st.fy!==undefined?st.fy+2:(st.core?st.core.position.y:heightAt(st.x+HALF,st.z+HALF)+2);
      spawnParticles(st.x+rand(-2,2),y,st.z+rand(-2,2),1,Math.random()<.3?0xff6a20:0x444444,1.2);
    }
  }
}
