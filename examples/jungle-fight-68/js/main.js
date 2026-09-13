'use strict';
/* SECTION 20 — GAME FLOW AND MAIN LOOP */
var lastT=0;
var dbgT=0, dbgFrames=0, dbgFPS=0, dbgElapsed=0;

function battleLogic(){
  /* US replacement wave */
  if(usWaveDead>=6&&usPool>0&&usSpawns.length){
    var alive=0;
    for(var i=0;i<soldiers.length;i++) if(!soldiers[i].dead) alive++;
    if(alive<110){
      var src=usSpawns[irand(0,usSpawns.length-1)];
      var spawned=0;
      for(var r=0;r<6;r++){
        var spot=findClearSpot(src.x,src.z,8,1.7,.4);
        if(!spot) spot={x:src.x+rand(-4,4),y:heightAt(src.x+HALF,src.z+HALF),z:src.z+rand(-4,4)};
        var man=makeReplacementAt(spot.x,spot.z);
        if(man){ man.x=spot.x; man.z=spot.z; man.y=spot.y; spawned++; }
      }
      if(spawned){
        usPool=Math.max(0,usPool-6);
        usWaveDead=0;
        killfeed('\u2605 6 REINFORCEMENTS FROM '+src.label+' \u2014 RESERVES '+usPool);
      } else usWaveDead=3;
    }
  }
  /* VC replacement wave */
  if(vcWaveDead>=6&&vcPool>0&&vcSpawns.length){
    var freeVC=0;
    for(var v=0;v<enemies.length;v++)
      if(!enemies[v].dead&&!enemies[v].garrison) freeVC++;
    if(freeVC<150){
      var vsrc=vcSpawns[irand(0,vcSpawns.length-1)];
      var n=Math.min(6,vcPool);
      for(var k=0;k<n;k++){
        var spotV=findClearSpot(vsrc.x,vsrc.z,8,1.6,.35);
        if(spotV) makeEnemy(spotV.x,spotV.z,{home:{x:vsrc.x,z:vsrc.z},roam:20});
      }
      vcPool=Math.max(0,vcPool-6);
      vcWaveDead=0;
    }
  }
  updateVCArmy(1/60);
  aiClusterScan(1/60);
  checkWin();
}

function init(){
  makeRenderer();
  makeLights();
  makeSky();
  makeChunks();
  buildCharPools();
  initDecals();
  initProjectiles();
  initParticles();
  initMuzzleFX();
  initPlayer();
  genWorld();
  buildViewmodels();
  attachViewmodel(P.cur);
  for(var i=0;i<WKEYS.length;i++){
    var w=WPN[WKEYS[i]];
    if(w&&w.magMax) w.mag=w.magMax;
  }
  bakeMinimap();
  makeClouds();
  makePrecip();
  buildWildlife();
  spawnPlatoons();
  buildOrderMenu();
  refreshSquadHUD();
  updateAmmoHUD();
  initCareerLine();
  /* wildlife */
  for(var d=0;d<24;d++){
    var dx=rand(-HALF+10,HALF-10), dz=rand(-HALF+10,HALF-10);
    if(heightAt(dx+HALF,dz+HALF)>WATER+.5) spawnDeer(dx,dz);
    else spawnDeer(usBase.x+rand(-20,20),usBase.z+rand(-20,20));
  }
  for(var m=0;m<16;m++) spawnMonkey();
  for(var f=0;f<11;f++) spawnFlock();
  spawnFireflies();
  if(IS_TOUCH){
    scene.traverse(function(o){
      if(o.isMesh&&!o.isInstancedMesh) o.castShadow=false;
    });
  }
  /* VC jungle patrols */
  var seeded=0,tries=0;
  while(seeded<42&&tries<4000){
    tries++;
    var px=rand(-HALF+10,HALF-10), pz=rand(-HALF+10,80);
    if(heightAt(px+HALF,pz+HALF)<3.5) continue;
    if(Math.abs(pz-riverZ(px))<6) continue;
    makeEnemy(px,pz,{home:{x:px,z:pz},roam:26});
    seeded++;
  }
  /* LT at firebase */
  var lt=findClearSpot(usBase.x,usBase.z-6,5,1.75,.4);
  if(lt){ P.pos.x=lt.x;P.pos.y=lt.y;P.pos.z=lt.z; }
  P.yaw=Math.PI;
  initInput();
  initTouchUI();
  initSettingsUI();
  /* DEPLOY button */
  el('btnDeploy').addEventListener('click',function(){
    initAudio();
    if(AC&&AC.state==='suspended') AC.resume();
    el('ovStart').classList.remove('show');
    if(!IS_TOUCH){
      try{ renderer.domElement.requestPointerLock(); }catch(e){}
    }
    startIntro();
  });
  el('btnRestart').addEventListener('click',function(){ location.reload(); });
  el('ovStart').classList.add('show');
  pickVoices();
  updateSky(0);
  requestAnimationFrame(loop);
}

function loop(t){
  requestAnimationFrame(loop);
  if(!lastT) lastT=t;
  var dt=Math.min(.05,(t-lastT)/1000);
  lastT=t;
  time+=dt;
  _distShots=0;
  if(intro){
    updateIntro(dt);
    updateParticles(dt);
    updateSky(dt);
    updateWeather(dt);
    updateFlyby(dt);
    flushCharPools();
    flushInstances();
    renderer.render(scene,camera);
    updateDebug(dt);
    return;
  }
  if(paused||!started){
    renderer.render(scene,camera);
    updateDebug(dt);
    return;
  }
  updatePlayer(dt);
  updateWeapon(dt);
  updateDoors(dt);
  var i;
  for(i=0;i<enemies.length;i++) updateEnemy(enemies[i],dt);
  for(i=enemies.length-1;i>=0;i--) if(enemies[i].remove) enemies.splice(i,1);
  /* squad brains */
  for(i=0;i<2;i++){
    var brain=pltBrains[i===0?0:2];
    if(brain) updatePlatoonBrain(brain,dt);
  }
  for(i=0;i<soldiers.length;i++) updateSoldier(soldiers[i],dt);
  updateDeer(dt);
  updateMonkeys(dt);
  updateFlocks(dt);
  updateFireflies(dt);
  updateChickens(dt);
  updateProjectiles(dt);
  updateGrenades(dt);
  updateParticles(dt);
  updateMuzzleFX(dt);
  updatePickups(dt);
  updateBarrels(dt);
  /* pending timers */
  for(i=pending.length-1;i>=0;i--){
    if(time>=pending[i].t){
      var fn=pending[i].fn;
      pending.splice(i,1);
      fn();
    }
  }
  updateSupport(dt);
  updateSky(dt);
  updateWeather(dt);
  updateFlyby(dt);
  battleLogic();
  updateAmbience(dt);
  updateHUD(dt);
  updateMinimap(dt);
  flushInstances();
  flushCharPools();
  updateChunkVisibility();
  renderer.render(scene,camera);
  updateDebug(dt);
}

/* ---------- debug overlay ---------- */
function updateDebug(dt){
  var d=el('dbg');
  if(!location.search.match(/debug=1/)) return;
  d.classList.add('show');
  dbgFrames++;
  dbgElapsed+=dt;
  dbgT+=dt;
  if(dbgT>=1){
    dbgT=0;
    dbgFPS=dbgFrames;
    dbgFrames=0;
    var vc=0;
    for(var i=0;i<enemies.length;i++) if(!enemies[i].dead) vc++;
    d.textContent='FPS '+dbgFPS+' | BLOCKS '+blockN+' | VC '+vc+' | T '+Math.floor(dbgElapsed)+'s';
    if(window.__loadErr){ d.classList.add('err'); d.textContent+=' | ERR '+window.__loadErr; }
  }
}

/* ---------- settings UI ---------- */
function initSettingsUI(){
  function build(boxId){
    var box=el(boxId);
    box.innerHTML='';
    function row(label,ctrl,valId){
      var r=document.createElement('div');
      r.className='srow';
      var l=document.createElement('label');
      l.textContent=label;
      r.appendChild(l);
      r.appendChild(ctrl);
      box.appendChild(r);
      return r;
    }
    /* sensitivity */
    var sA=document.createElement('input');
    sA.type='range';sA.min=30;sA.max=250;sA.step=5;sA.value=SET.sens*100;
    var sAv=document.createElement('span');sAv.className='sval';sAv.textContent=(SET.sens).toFixed(1);
    sA.addEventListener('input',function(){
      SET.sens=sA.value/100;
      sAv.textContent=SET.sens.toFixed(1);
      saveSettings();
    });
    var rA=document.createElement('div');rA.className='srow';
    var lA=document.createElement('label');lA.textContent='SENSITIVITY';
    rA.appendChild(lA);rA.appendChild(sA);rA.appendChild(sAv);
    box.appendChild(rA);
    /* volume */
    var sB=document.createElement('input');
    sB.type='range';sB.min=0;sB.max=100;sB.step=5;sB.value=SET.vol*100;
    var svB=document.createElement('span');svB.className='sval';svB.textContent=Math.round(SET.vol*100)+'%';
    sB.addEventListener('input',function(){
      SET.vol=sB.value/100;
      setVol(SET.vol);
      svB.textContent=Math.round(SET.vol*100)+'%';
      saveSettings();
    });
    var rB=document.createElement('div');rB.className='srow';
    var lB=document.createElement('label');lB.textContent='VOLUME';
    rB.appendChild(lB);rB.appendChild(sB);rB.appendChild(svB);
    box.appendChild(rB);
    /* music */
    var cM=document.createElement('input');
    cM.type='checkbox';cM.checked=!!SET.music;
    var lM=document.createElement('label');lM.textContent='RADIO MUSIC';
    cM.addEventListener('change',function(){
      SET.music=cM.checked?1:0;
      if(musicBus) musicBus.gain.value=SET.music?.30:0;
      saveSettings();
    });
    var rM=document.createElement('div');rM.className='srow';
    rM.appendChild(lM);rM.appendChild(cM);
    box.appendChild(rM);
    /* voices */
    var cV=document.createElement('input');
    cV.type='checkbox';cV.checked=!!SET.voice;
    var lV=document.createElement('label');lV.textContent='RADIO VOICES';
    cV.addEventListener('change',function(){
      SET.voice=cV.checked?1:0;
      saveSettings();
    });
    var rV=document.createElement('div');rV.className='srow';
    rV.appendChild(lV);rV.appendChild(cV);
    box.appendChild(rV);
    /* shake */
    var cS=document.createElement('input');
    cS.type='checkbox';cS.checked=!SET.shake;
    var lS=document.createElement('label');lS.textContent='REDUCE SHAKE & FLASH';
    cS.addEventListener('change',function(){
      SET.shake=cS.checked?0:1;
      saveSettings();
    });
    var rS=document.createElement('div');rS.className='srow';
    rS.appendChild(lS);rS.appendChild(cS);
    box.appendChild(rS);
  }
  build('set1');
  build('set2');
}

/* boot */
init();
