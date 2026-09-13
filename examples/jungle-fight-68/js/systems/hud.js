'use strict';
/* SECTION 18 — HUD */
var dmgFlash=0, dmgDirAngle=0, dmgDirT=0;
var hitmT=0, hitmKill=false;
var bannerTxt='', bannerT=0;
var heartT=0;
var _scoreSig='', _ammoSig='', _envSig='';
var feedLines=[];

function killfeed(txt){
  feedLines.push({txt:txt,t:time});
  if(feedLines.length>6) feedLines.shift();
  var f=el('feed');
  f.innerHTML='';
  for(var i=0;i<feedLines.length;i++){
    var d=document.createElement('div');
    d.textContent=feedLines[i].txt;
    f.appendChild(d);
  }
}
function banner(txt){
  bannerTxt=txt;
  bannerT=2.6;
  var b=el('banner');
  b.textContent=txt;
  b.style.opacity=1;
}
function hitmark(kill){
  hitmT=kill?.26:.12;
  hitmKill=kill;
  var h=el('hitm');
  h.classList.toggle('big',kill);
  h.style.opacity=1;
}
function updateHUD(dt){
  /* health bar */
  var hb=el('hpbar');
  hb.style.width=Math.max(0,P.hp)+'%';
  hb.classList.toggle('low',P.hp<35);
  /* heartbeat */
  if(P.hp<30&&!P.dead){
    heartT-=dt;
    if(heartT<=0){ heartT=.72+P.hp/90; sHeartbeat(); }
  }
  /* hitmarker */
  if(hitmT>0){
    hitmT-=dt;
    if(hitmT<=0) el('hitm').style.opacity=0;
  }
  /* banner */
  if(bannerT>0){
    bannerT-=dt;
    if(bannerT<=0) el('banner').style.opacity=0;
  }
  /* damage vignette */
  if(dmgFlash>0){
    dmgFlash=Math.max(0,dmgFlash-dt*1.4);
    el('dmgvig').style.opacity=dmgFlash;
  }
  if(dmgDirT>0){
    dmgDirT=Math.max(0,dmgDirT-dt*1.2);
    var dd=el('dmgdir');
    dd.style.opacity=dmgDirT;
    dd.style.transform='rotate('+dmgDirAngle+'rad)';
  }
  /* feed fade */
  for(var i=feedLines.length-1;i>=0;i--){
    if(time-feedLines[i].t>3.6){
      feedLines.splice(i,1);
      var f=el('feed');
      f.innerHTML='';
      for(var j=0;j<feedLines.length;j++){
        var dv=document.createElement('div');
        dv.textContent=feedLines[j].txt;
        f.appendChild(dv);
      }
    }
  }
  /* score line */
  var cleared=0;
  for(var b=0;b<bunkerList.length;b++) if(bunkerList[b].cleared||bunkerList[b].dead) cleared++;
  var sig=US+'|'+VC+'|'+kills+'|'+deaths+'|'+cleared+'|'+usPool+'|'+vcPool;
  if(sig!==_scoreSig){
    _scoreSig=sig;
    var kd=deaths>0?(kills/deaths).toFixed(2):kills.toFixed(0);
    el('score').textContent='\u2605 '+US+' \u00b7 K/D '+kd+' \u2014 VC '+VC+' \u25B2 \u00b7 BUNKERS '+cleared+'/8 \u00b7 RESERVES US '+usPool+' / VC '+vcPool;
  }
  /* env line */
  var hours=Math.floor(dayT*24), mins=Math.floor((dayT*24-hours)*60);
  var wkind=weather?weather.kind:'SUNNY';
  var eSig=hours+' '+mins+' '+wkind;
  if(eSig!==_envSig){
    _envSig=eSig;
    el('envline').textContent=(hours<10?'0':'')+hours+':'+(mins<10?'0':'')+mins+' \u00b7 '+wkind;
  }
  /* crosshair */
  var cross=el('cross');
  var w=WPN[P.cur];
  if(w&&!P.dead){
    var gap;
    if(w.melee) gap=14;
    else{
      var sp=w.spread||0;
      gap=8+sp*(P.aim?900:1600)*(P.moveSpeed>.5?1.5:1);
    }
    cross.style.setProperty('--gap',gap+'px');
    cross.classList.toggle('hide',P.scope||P.aim&&w.scope);
  }
  /* prompt */
  var it=nearestInteract();
  var pr=el('prompt');
  if(it&&!P.dead){
    pr.style.opacity=1;
    pr.textContent='[E] '+it.label;
  } else pr.style.opacity=0;
  /* platoon panel every .25s */
  hudSquadT-=dt;
  if(hudSquadT<=0){ hudSquadT=.25; refreshSquadHUD(); }
}
var hudSquadT=0;
function refreshSquadHUD(){
  var counts=['','',''];
  var wia=[0,0,0];
  for(var p=0;p<3;p++){
    var alive=0,total=0;
    for(var i=0;i<soldiers.length;i++){
      var s=soldiers[i];
      if(s.platoon!==p+1) continue;
      total++;
      if(!s.dead) alive++;
      if(s.wounded&&!s.dead) wia[p]++;
    }
    counts[p]=alive+'/'+total;
  }
  var c0=parseInt(counts[0],10), c1=parseInt(counts[1],10), c2=parseInt(counts[2],10);
  var cls=c1<=15?'color:#ff5040':(c1<=36?'color:#ffc23a':'');
  var orderTxt=ORDERS['1']?'' :'';
  for(var k in ORDERS) if(ORDERS[k].key===curOrder) orderTxt=ORDERS[k].label;
  el('pltcounts').innerHTML=
    '1ST '+counts[0]+' \u00b7 <span style="'+(c0<=15?'color:#ff5040':(c0<=36?'color:#ffc23a':''))+'">2ND '+counts[1]+'</span> \u00b7 3RD '+counts[2]+
    (wia[0]+wia[1]+wia[2]>0?' \u2014 WIA '+(wia[0]+wia[1]+wia[2]):'');
  el('pltorder').textContent='ORDER: '+orderTxt+(fireAtWill?'':' \u00b7 HOLDING FIRE');
  var mix='GARAND \u00b7 SMG \u00b7 LMG';
  el('pltweapons').textContent=mix;
}
function updateAmmoHUD(){
  var w=WPN[P.cur];
  if(!w) return;
  var txt;
  if(w.melee) txt='-- MELEE --';
  else if(w.nade) txt=P.nades+' FRAG'+(P.nades===1?'':'S');
  else{
    var pool=P.pools[w.pool];
    txt=w.mag+' / '+pool;
    if(P.reloadT>0) txt+=' \u00b7 RELOADING';
  }
  var sig=P.cur+'|'+txt;
  if(sig!==_ammoSig){
    _ammoSig=sig;
    el('ammoline').textContent=txt;
    el('wline').innerHTML=w.name+(w.scope?' <span style="color:#9db07a">[OPTIC]</span>':'')+
      ' <span class="stance">['+['STAND','CROUCH','PRONE'][P.stance]+']</span>';
  }
  /* slots row */
  var slots=['','','','',''];
  var seen={};
  for(var i=0;i<WKEYS.length;i++){
    var k=WKEYS[i];
    var ww=WPN[k];
    if(ww.melee||ww.nade) continue;
    if(seen[ww.slot]===undefined){
      seen[ww.slot]=k;
      var words=ww.name.split(' ');
      slots[ww.slot]=words[words.length-1];
    }
  }
  var slotTxt='';
  var names=['SHOVEL','KNIFE',null,null,null,null,null,null,null,null];
  slots[0]=P.cur==='knife'?'KNIFE':'SHOVEL';
  slots[4]='FRAGS';
  for(var sN=0;sN<5;sN++) slotTxt+=(sN+1)+':'+(slots[sN]||'\u2014')+'  ';
  el('slots').textContent=slotTxt;
}

/* ---------- win / lose ---------- */
function checkWin(){
  if(won) return;
  if(vcPostStruct&&vcPostStruct.dead){
    won=true;
    var kd=deaths>0?kills/deaths:kills;
    var grade=kd>=3?'OVERWHELMING VICTORY':kd>=1.8?'DECISIVE VICTORY':kd>=1.1?'COSTLY VICTORY':'PYRRHIC VICTORY';
    el('winTitle').textContent='\u2605 '+grade;
    el('winSub').textContent='THE VC COMMAND POST HAS FALLEN';
    el('winStats').textContent='K/D '+(kd||0).toFixed(2)+' \u00b7 VC KIA '+vcKIA+' vs US KIA '+usKIA+' \u00b7 LT DEATHS '+deaths+' \u00b7 BUNKERS '+clearedCount()+'/8';
    var bestKd=STATS.bkd;
    var newBest=kd>bestKd;
    if(newBest) STATS.bkd=Math.round(kd*100)/100;
    STATS.w++;
    STATS.k+=kills;
    saveStats();
    el('winCareer').textContent='CAREER: '+STATS.w+' W \u00b7 '+STATS.l+' L \u00b7 BEST K/D '+STATS.bkd+(newBest?' \u2014 NEW BEST K/D!':'');
    el('ovWin').classList.add('show');
    radioSay('Command post destroyed. Outstanding work, Delta. Outstanding.');
  } else if(usBaseStruct&&usBaseStruct.dead){
    won=true;
    el('winTitle').textContent='\u25B2 OUR COMMAND POST HAS FALLEN \u25B2';
    el('winSub').textContent='THE WAR IS LOST';
    el('winStats').textContent='K/D '+(deaths>0?kills/deaths:kills).toFixed(2)+' \u00b7 VC KIA '+vcKIA+' vs US KIA '+usKIA+' \u00b7 LT DEATHS '+deaths+' \u00b7 BUNKERS '+clearedCount()+'/8';
    STATS.l++;
    STATS.k+=kills;
    saveStats();
    el('winCareer').textContent='CAREER: '+STATS.w+' W \u00b7 '+STATS.l+' L \u00b7 BEST K/D '+STATS.bkd;
    el('ovWin').classList.add('show');
    radioSay('Delta Six, we are overrun. Fall back! Fall back!');
  }
}
function clearedCount(){
  var c=0;
  for(var b=0;b<bunkerList.length;b++) if(bunkerList[b].cleared||bunkerList[b].dead) c++;
  return c;
}
function initCareerLine(){
  var c=el('career');
  c.textContent='CAREER: '+STATS.w+' W \u00b7 '+STATS.l+' L \u00b7 BEST K/D '+STATS.bkd+' \u00b7 '+STATS.k+' TOTAL KILLS';
}
function toggleCmdMenu(open){
  cmdMenuOpen=open===undefined?!cmdMenuOpen:open;
  el('cmdmenu').style.display=cmdMenuOpen?'block':'none';
  if(cmdMenuOpen) refreshOrderMenu();
}
