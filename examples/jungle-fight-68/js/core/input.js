'use strict';
/* SECTION 19 — DESKTOP INPUT */
var _keys={};
function key(code){ return !!_keys[code]; }
function clearLatchedKeys(){ _keys={}; }
var KEYMAP={
  KeyW:'fwd',KeyS:'back',KeyA:'left',KeyD:'right',
  ArrowUp:'fwd',ArrowDown:'back',ArrowLeft:'leftA',ArrowRight:'rightA',
  Space:'jump',ShiftLeft:'sprint',ShiftRight:'sprint'
};
function initInput(){
  document.addEventListener('keydown',function(e){
    if(e.code==='F1'){ e.preventDefault(); toggleCmdMenu(); return; }
    if(intro){ skipIntro(); return; }
    if(!started) return;
    if(cmdMenuOpen&&/^Digit[0-9]$/.test(e.code)){
      var d=e.code.slice(-1);
      issueOrder(d);
      e.preventDefault();
      return;
    }
    if(KEYMAP[e.code]) _keys[KEYMAP[e.code]]=true;
    switch(e.code){
      case 'KeyR': tryReload(); break;
      case 'KeyG': throwGrenadePlayer(); break;
      case 'KeyT': playerArtillery(); break;
      case 'KeyY': playerNapalm(); break;
      case 'KeyE': doInteract(nearestInteract()); break;
      case 'KeyC': P.stance=P.stance===1?0:1; updateAmmoHUD(); break;
      case 'KeyX': P.stance=P.stance===2?0:2; updateAmmoHUD(); break;
      case 'KeyF': flashlight.intensity=flashlight.intensity>0?0:2.2; break;
      case 'KeyM': toggleMusic(); break;
      case 'KeyP': togglePause(); break;
      case 'Digit1': selectSlot(1); break;
      case 'Digit2': selectSlot(2); break;
      case 'Digit3': selectSlot(3); break;
      case 'Digit4': selectSlot(4); break;
      case 'Digit5': selectSlot(0); break;
      case 'Space': e.preventDefault(); break;
    }
  });
  document.addEventListener('keyup',function(e){
    if(KEYMAP[e.code]) _keys[KEYMAP[e.code]]=false;
  });
  window.addEventListener('blur',clearLatchedKeys);
  document.addEventListener('pointerlockchange',function(){
    var locked=document.pointerLockElement===renderer.domElement;
    if(!locked&&started&&!P.dead&&!won) togglePause(true);
  });
  document.addEventListener('visibilitychange',function(){
    if(document.hidden&&started) togglePause(true);
  });
  renderer.domElement.addEventListener('click',function(){
    if(started&&!IS_TOUCH&&!paused){
      try{ renderer.domElement.requestPointerLock(); }catch(e){}
    }
  });
  document.addEventListener('mousemove',function(e){
    if(document.pointerLockElement!==renderer.domElement) return;
    var sens=.0022*SET.sens*(P.scope?.3:1);
    P.yaw-=e.movementX*sens;
    P.pitch-=e.movementY*sens;
    P.pitch=clamp(P.pitch,-1.45,1.45);
  });
  document.addEventListener('mousedown',function(e){
    if(!started||paused) return;
    if(document.pointerLockElement!==renderer.domElement&&!IS_TOUCH) return;
    if(e.button===0) P.trigger=true;
    if(e.button===2) P.aim=true;
  });
  document.addEventListener('mouseup',function(e){
    if(e.button===0) P.trigger=false;
    if(e.button===2) P.aim=false;
  });
  document.addEventListener('contextmenu',function(e){ e.preventDefault(); });
  document.addEventListener('wheel',function(e){
    if(!started||paused||cmdMenuOpen) return;
    cycleWeapon(e.deltaY>0?1:-1);
  },{passive:true});
}
function togglePause(force){
  if(!started||won) return;
  paused=force===undefined?!paused:force;
  el('ovPause').classList.toggle('show',paused);
  if(paused){
    if(document.pointerLockElement) document.exitPointerLock();
  } else if(!IS_TOUCH){
    try{ renderer.domElement.requestPointerLock(); }catch(e){}
  }
}
