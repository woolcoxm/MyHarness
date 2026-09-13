'use strict';
/* SECTION 3 — GLOBAL STATE */
var US_POOL_MAX=5000, VC_POOL_MAX=5000;
var W=400, HALF=200, WATER=2.6;
var IS_TOUCH=!!window.IS_TOUCH;

var heights=new U8(W*W);
var treeTops=[];          /* {x,z,ids[],solid,cr} */
var warnZones=[];         /* {x,z,r,until} */
var smokingStructs=[];
var solids=[], losSolids=[], interactables=[], bunkerList=[];
var enemies=[], soldiers=[], deers=[], monkeys=[], flocks=[], chickens=[];
var projectiles=[], grenades=[], particles=[], pickups=[], barrels=[], pending=[];
var campfires=[];
var structList=[];        /* destructible command posts and bases */
var vcSpawns=[], usSpawns=[];

/* spatial grid over solids */
var GRID_CELL=10;
var GRID_DIM=Math.ceil(W/GRID_CELL)+2;
var solidGrid=[];
(function(){
  for(var i=0;i<GRID_DIM*GRID_DIM;i++) solidGrid.push([]);
})();
var gridQID=0;

function gridCellOf(x,z){
  var cx=Math.floor((x+HALF)/GRID_CELL)+1;
  var cz=Math.floor((z+HALF)/GRID_CELL)+1;
  cx=clamp(cx,0,GRID_DIM-1); cz=clamp(cz,0,GRID_DIM-1);
  return cz*GRID_DIM+cx;
}
function gridInsertSolid(s){
  var ax=Math.floor((s.x0+HALF)/GRID_CELL)+1, az=Math.floor((s.z0+HALF)/GRID_CELL)+1;
  var bx=Math.floor((s.x1+HALF)/GRID_CELL)+1, bz=Math.floor((s.z1+HALF)/GRID_CELL)+1;
  ax=clamp(ax,0,GRID_DIM-1); az=clamp(az,0,GRID_DIM-1);
  bx=clamp(bx,0,GRID_DIM-1); bz=clamp(bz,0,GRID_DIM-1);
  for(var cz=az;cz<=bz;cz++) for(var cx=ax;cx<=bx;cx++) solidGrid[cz*GRID_DIM+cx].push(s);
}
var _gath=[];
function gatherSolids(ax,az,bx,bz,useLOS,pad){
  pad=pad||0;
  gridQID++;
  var out=_gath; out.length=0;
  var cax=clamp(Math.floor((Math.min(ax,bx)-pad+HALF)/GRID_CELL)+1,0,GRID_DIM-1);
  var caz=clamp(Math.floor((Math.min(az,bz)-pad+HALF)/GRID_CELL)+1,0,GRID_DIM-1);
  var cbx=clamp(Math.floor((Math.max(ax,bx)+pad+HALF)/GRID_CELL)+1,0,GRID_DIM-1);
  var cbz=clamp(Math.floor((Math.max(az,bz)+pad+HALF)/GRID_CELL)+1,0,GRID_DIM-1);
  for(var cz=caz;cz<=cbz;cz++)for(var cx=cax;cx<=cbx;cx++){
    var cell=solidGrid[cz*GRID_DIM+cx];
    for(var i=0;i<cell.length;i++){
      var s=cell[i];
      if(s.off) continue;
      if(useLOS && !s.los) continue;
      if(s._qid===gridQID) continue;
      s._qid=gridQID;
      out.push(s);
    }
  }
  return out;
}

/* flags and counters */
var curOrder='follow', fireAtWill=true, cmdMenuOpen=false;
var time=0, started=false, paused=false, won=false;
var US=0, VC=0, kills=0, deaths=0, shotsFired=0, shotsHit=0;
var shellHitT=-99, shellHitX=0, shellHitZ=0;
var craterDirty=false;
var worldBuilt=false;

/* war-economy counters */
var usPool=US_POOL_MAX, vcPool=VC_POOL_MAX;
var usWaveDead=0, vcWaveDead=0, usKIA=0, vcKIA=0;
var vcArmyT=50, vcArmy=null;
var vcCounter=0;

/* persisted settings */
var _SET_DEF={sens:1,vol:.55,music:1,shake:1,voice:1};
var SET={};
(function(){
  var ok=false;
  try{
    var raw=localStorage.getItem('jf68_settings');
    if(raw){ var o=JSON.parse(raw); if(o&&typeof o==='object'){ for(var k in _SET_DEF) if(k in o) SET[k]=o[k]; ok=true; } }
  }catch(e){ ok=false; }
  for(var k2 in _SET_DEF) if(!(k2 in SET)) SET[k2]=_SET_DEF[k2];
})();
function saveSettings(){
  try{ localStorage.setItem('jf68_settings',JSON.stringify(SET)); }catch(e){}
}

/* career stats */
var STATS={w:0,l:0,bkd:0,k:0};
(function(){
  try{
    var raw=localStorage.getItem('jf68_stats');
    if(raw){ var o=JSON.parse(raw); if(o&&typeof o==='object'){ if('w' in o)STATS.w=o.w|0; if('l' in o)STATS.l=o.l|0; if('bkd' in o)STATS.bkd=o.bkd|0; if('k' in o)STATS.k=o.k|0; } }
  }catch(e){}
})();
function saveStats(){ try{ localStorage.setItem('jf68_stats',JSON.stringify(STATS)); }catch(e){} }
