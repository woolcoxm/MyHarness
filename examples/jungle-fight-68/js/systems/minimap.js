'use strict';
/* SECTION 18b — MINIMAP */
var mapBase=null;
var mapBakeT=0;
var lastMapBake=-99;
function bakeMinimap(){
  var c=document.createElement('canvas');
  c.width=W*2; c.height=W*2;
  var g=c.getContext(CTXID);
  var bands=[0x2a6fb0,0x5d8f3a,0x49772e,0x3a6626,0x2f5520];
  for(var z=0;z<W;z++)for(var x=0;x<W;x++){
    var h=heights[z*W+x];
    var col;
    if(h<WATER-.2) col=bands[0];
    else if(h<5) col=bands[1];
    else if(h<8) col=bands[2];
    else if(h<11) col=bands[3];
    else col=bands[4];
    g.fillStyle='#'+col.toString(16).padStart(6,'0');
    g.fillRect(x*2,z*2,2,2);
  }
  /* trees */
  g.fillStyle='#1c3d14';
  for(var t=0;t<treeTops.length;t++){
    g.fillRect((treeTops[t].gx)*2-1,(treeTops[t].gz)*2-1,3,3);
  }
  /* bridge */
  g.fillStyle='#8a8a8a';
  g.fillRect((-20+HALF)*2-3,(riverZ(-20)+HALF)*2-20,7,40);
  /* bunkers orange, gray when cleared */
  for(var b=0;b<bunkerList.length;b++){
    g.fillStyle=(bunkerList[b].cleared||bunkerList[b].dead)?'#8a8a8a':'#ff9a2a';
    g.fillRect((bunkerList[b].x+HALF)*2-4,(bunkerList[b].z+HALF)*2-4,8,8);
  }
  /* campfires pink */
  g.fillStyle='#ff6ad5';
  for(var cf=0;cf<campfires.length;cf++)
    g.fillRect((campfires[cf].x+HALF)*2-2,(campfires[cf].z+HALF)*2-2,5,5);
  /* structures */
  for(var s=0;s<structList.length;s++){
    var st=structList[s];
    if(st.dead) continue;
    var big=st.label.indexOf('COMMAND')>=0;
    g.fillStyle=st.side==='us'?'#3aa0ff':'#ff3a3a';
    var sz=big?7:5;
    g.fillRect((st.x+HALF)*2-sz/2,(st.z+HALF)*2-sz/2,sz,sz);
  }
  mapBase=c;
  lastMapBake=time;
}
var _mapT=0;
function updateMinimap(dt){
  _mapT-=dt;
  if(_mapT>0) return;
  _mapT=.12;
  if(craterDirty&&time-lastMapBake>(IS_TOUCH?25:8)){ bakeMinimap(); craterDirty=false; }
  var cv=el('minimap');
  var g=cv.getContext(CTXID);
  var S=176;
  var scale=S/W;
  var win=S/scale; /* blocks visible: whole map fits at 1px/2? use zoom */
  var zoom=3;
  var cx=(P.pos.x+HALF), cz=(P.pos.z+HALF);
  var vx=cx*zoom-S/2, vy=cz*zoom-S/2;
  var mx=clamp(vx,0,W*zoom-S), my=clamp(vy,0,W*zoom-S);
  g.clearRect(0,0,S,S);
  if(mapBase){
    g.imageSmoothingEnabled=false;
    g.drawImage(mapBase,mx/zoom*2,my/zoom*2,(S/zoom)*2,(S/zoom)*2,0,0,S,S);
  }
  function toMap(x,z){
    return {x:(x+HALF)*zoom-mx,y:(z+HALF)*zoom-my};
  }
  /* VC blips */
  g.fillStyle='#ff3a3a';
  for(var e=0;e<enemies.length;e++){
    var en=enemies[e];
    if(en.dead) continue;
    var p=toMap(en.x,en.z);
    if(p.x<0||p.x>S||p.y<0||p.y>S) continue;
    g.fillRect(p.x-1.5,p.y-1.5,3,3);
  }
  /* soldier blips */
  g.fillStyle='#4a9fff';
  for(var s=0;s<soldiers.length;s++){
    var sd=soldiers[s];
    if(sd.dead) continue;
    var pS=toMap(sd.x,sd.z);
    if(pS.x<0||pS.x>S||pS.y<0||pS.y>S) continue;
    g.fillRect(pS.x-1.5,pS.y-1.5,3,3);
  }
  /* pickups */
  g.fillStyle='#ffe93a';
  for(var k=0;k<pickups.length;k++){
    var pK=toMap(pickups[k].x,pickups[k].z);
    g.fillRect(pK.x-1,pK.y-1,2,2);
  }
  /* warn zone rings */
  for(var w=0;w<warnZones.length;w++){
    var Z=warnZones[w];
    var pZ=toMap(Z.x,Z.z);
    var pulse=2+Math.sin(time*8)*1.5;
    g.strokeStyle=Z.col||'#ff3020';
    g.lineWidth=1.5;
    g.beginPath();
    g.arc(pZ.x,pZ.y,Z.r*zoom*.6+pulse,0,TAU);
    g.stroke();
  }
  /* player arrow + view cone */
  var me=toMap(P.pos.x,P.pos.z);
  g.save();
  g.translate(me.x,me.y);
  g.rotate(-P.yaw+Math.PI);
  g.fillStyle='rgba(255,255,255,.25)';
  g.beginPath();
  g.moveTo(0,0);
  g.arc(0,0,30,-Math.PI/2-.6,-Math.PI/2+.6);
  g.closePath();
  g.fill();
  g.fillStyle='#fff';
  g.beginPath();
  g.moveTo(0,-6);
  g.lineTo(4,4);
  g.lineTo(-4,4);
  g.closePath();
  g.fill();
  g.restore();
}
