'use strict';
/* SECTION 19b — TOUCH CONTROLS */
var _stickActive=false, _stickCX=0, _stickCY=0;
var _lookLast=null;
var _tbtn={};
function initTouchUI(){
  if(!IS_TOUCH) return;
  var root=el('touchui');
  var look=document.createElement('div');
  look.id='lookpad';
  root.appendChild(look);
  var base=document.createElement('div');
  base.id='stickBase';
  base.innerHTML='<div id="stickNub"></div>';
  root.appendChild(base);
  function mkBtn(id,txt){
    var b=document.createElement('div');
    b.className='tbtn';
    b.id=id;
    b.textContent=txt;
    root.appendChild(b);
    return b;
  }
  _tbtn.fire=mkBtn('tbFire','FIRE');
  _tbtn.stance=mkBtn('tbStance','STAND');
  _tbtn.ads=mkBtn('tbAds','ADS');
  _tbtn.rld=mkBtn('tbRld','RLD');
  _tbtn.jmp=mkBtn('tbJmp','JMP');
  _tbtn.frag=mkBtn('tbFrag','FRAG');
  _tbtn.use=mkBtn('tbUse','USE');
  _tbtn.pause=mkBtn('tbPause','II');
  _tbtn.orders=mkBtn('tbOrders','F1');
  _tbtn.art=mkBtn('tb155','155');
  _tbtn.nap=mkBtn('tbNap','NAP');
  var slots=document.createElement('div');
  slots.id='tslots';
  var slotNames=['MELEE','PISTOL','RIFLE','SMG','FRAG'];
  for(var i=0;i<5;i++){
    (function(idx){
      var s=document.createElement('div');
      s.textContent=(idx===4?'5':(idx+1)+'')+' '+slotNames[idx];
      s.addEventListener('touchstart',function(ev){
        ev.preventDefault();
        selectSlot(idx===4?0:(idx+1));
      });
      slots.appendChild(s);
    })(i);
  }
  root.appendChild(slots);

  /* move stick spawns where the thumb lands on left 45% */
  root.addEventListener('touchstart',function(e){
    for(var i=0;i<e.changedTouches.length;i++){
      var t=e.changedTouches[i];
      if(t.clientX<innerWidth*.45&&!_stickActive){
        _stickActive=true;
        _stickCX=t.clientX;
        _stickCY=t.clientY;
        base.style.display='block';
        base.style.left=(t.clientX-58)+'px';
        base.style.top=(t.clientY-58)+'px';
        base.dataset.tid=t.identifier;
      }
    }
  },{passive:false});
  root.addEventListener('touchmove',function(e){
    for(var i=0;i<e.changedTouches.length;i++){
      var t=e.changedTouches[i];
      if(_stickActive&&t.identifier.toString()===base.dataset.tid){
        e.preventDefault();
        var dx=t.clientX-_stickCX, dy=t.clientY-_stickCY;
        var d=Math.hypot(dx,dy);
        var max=58;
        var nd=Math.min(d,max);
        var nx=d>0?dx/d:0, ny=d>0?dy/d:0;
        var nub=el('stickNub');
        nub.style.left=(34+nx*nd*.7)+'px';
        nub.style.top=(34+ny*nd*.7)+'px';
        var mag=nd/max;
        var fw=-ny, rt=nx;
        P.touchVec={x:rt,y:fw,mag:mag};
        P.touchSprint=mag>.92&&fw>.5;
      } else if(_lookLast&&t.identifier===_lookLast.id){
        e.preventDefault();
        var sens=.0032*SET.sens*(P.scope?.35:1);
        P.yaw-=(t.clientX-_lookLast.x)*sens;
        P.pitch-=(t.clientY-_lookLast.y)*sens;
        P.pitch=clamp(P.pitch,-1.45,1.45);
        _lookLast.x=t.clientX; _lookLast.y=t.clientY;
      }
    }
  },{passive:false});
  function endTouch(e){
    for(var i=0;i<e.changedTouches.length;i++){
      var t=e.changedTouches[i];
      if(_stickActive&&t.identifier.toString()===base.dataset.tid){
        _stickActive=false;
        P.touchVec=null;
        P.touchSprint=false;
        base.style.display='none';
      }
      if(_lookLast&&t.identifier===_lookLast.id) _lookLast=null;
    }
  }
  root.addEventListener('touchend',endTouch);
  root.addEventListener('touchcancel',endTouch);
  look.addEventListener('touchstart',function(e){
    var t=e.changedTouches[0];
    _lookLast={id:t.identifier,x:t.clientX,y:t.clientY};
  },{passive:false});
  function hold(id,down,up){
    var b=el(id);
    b.addEventListener('touchstart',function(e){ e.preventDefault(); e.stopPropagation(); b.classList.add('on'); down(); },{passive:false});
    b.addEventListener('touchend',function(e){ e.preventDefault(); b.classList.remove('on'); if(up)up(); },{passive:false});
  }
  function tap(id,fn){
    var b=el(id);
    b.addEventListener('touchstart',function(e){ e.preventDefault(); e.stopPropagation(); fn(); },{passive:false});
  }
  hold('tbFire',function(){ P.trigger=true; },function(){ P.trigger=false; });
  tap('tbAds',function(){
    P.aim=!P.aim;
    el('tbAds').classList.toggle('on',P.aim);
  });
  tap('tbRld',tryReload);
  tap('tbJmp',function(){
    if(P.grounded&&P.stance===0&&P.pos.y+0.4>=WATER) P.vy=5.3;
  });
  tap('tbFrag',throwGrenadePlayer);
  tap('tbUse',function(){ doInteract(nearestInteract()); });
  tap('tbStance',function(){
    P.stance=(P.stance+1)%3;
    updateAmmoHUD();
  });
  tap('tbPause',function(){ togglePause(); });
  tap('tbOrders',function(){ toggleCmdMenu(); });
  tap('tb155',playerArtillery);
  tap('tbNap',playerNapalm);
  window.addEventListener('touchstart',function(){
    if(intro) skipIntro();
  },{passive:true});
  setInterval(refreshTouchUI,250);
  root.classList.add('show');
}
function refreshTouchUI(){
  if(!IS_TOUCH) return;
  el('tbStance').textContent=['STAND','CROUCH','PRONE'][P.stance];
  var art=Math.ceil(artyCd), nap=Math.ceil(napalmCd);
  setCd('tb155',art);
  setCd('tbNap',nap);
  el('tb155').classList.toggle('dim',art>0);
  el('tbNap').classList.toggle('dim',nap>0);
  var pr=el('prompt');
  el('tbUse').classList.toggle('dim',pr.style.opacity==='0'||!pr.style.opacity);
  var kids=el('tslots').children;
  var curSlot=WPN[P.cur]?WPN[P.cur].slot:0;
  for(var i=0;i<kids.length;i++){
    var want=i===4?0:(i+1);
    kids[i].classList.toggle('act',want===curSlot);
  }
}
function setCd(id,secs){
  var b=el(id);
  if(!b) return;
  var cd=b.querySelector('.cd');
  if(!cd){
    cd=document.createElement('span');
    cd.className='cd';
    b.appendChild(cd);
  }
  cd.textContent=secs>0?secs:'';
}
