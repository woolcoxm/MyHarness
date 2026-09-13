'use strict';
/* SECTION 7 — STRUCTURES */
function makeStruct(side,x,z,opt){
  opt=opt||{};
  var s={
    side:side, x:x, z:z,
    r:opt.r||5.5, cratR:opt.cratR||6.5,
    hp:opt.hp||1000, maxHp:opt.hp||1000,
    dead:false, label:opt.label||'BASE',
    vis:[], hurtT:-99, announced:{}, smoking:false,
    core:null, antenna:null
  };
  structList.push(s);
  (side==='us'?usSpawns:vcSpawns).push(s);
  return s;
}

var BAG=0x9a8a5a, BAG_D=0x8a7a4c;

function damageStructure(st,dmg){
  if(st.dead) return;
  st.hp-=dmg;
  st.hurtT=time;
  var frac=st.hp/st.maxHp;
  if(st.core){
    if(frac<.5&&!st._sunk1){
      st._sunk1=true;
      st.core.position.y-=.35;
      st.core.rotation.x+=.05; st.core.rotation.z+=.04;
      if(st.antenna) st.antenna.visible=false;
    }
    if(frac<.25&&!st._sunk2){
      st._sunk2=true;
      st.core.position.y-=.55;
      st.core.rotation.x+=.09; st.core.rotation.z+=.05;
    }
  }
  if(frac<.55&&!st.smoking){
    st.smoking=true;
    smokingStructs.push(st);
  }
  spawnParticles(st.x,(st.core?st.core.position.y:heightAt(st.x+HALF,st.z+HALF)+2),st.z,6,0x8a8a8a,2);
  var who=st.side==='us'?'OUR ':'ENEMY ';
  if(frac<.75&&!st.announced[75]){ st.announced[75]=1; killfeed(who+st.label+' IS HEAVILY DAMAGED'); }
  if(frac<.5&&!st.announced[50]){ st.announced[50]=1; killfeed(who+st.label+' IS FALLING APART'); }
  if(frac<.25&&!st.announced[25]){ st.announced[25]=1; killfeed(who+st.label+' IS CRITICAL'); }
  if(st.hp<=0) destroyStructure(st);
}
function destroyStructure(st){
  st.dead=true;
  if(st.core) scene.remove(st.core);
  if(st.antenna) scene.remove(st.antenna);
  var y=heightAt(st.x+HALF,st.z+HALF);
  for(var i=0;i<14;i++){
    addBlock(st.x+rand(-3,3),y+rand(0,1.2),st.z+rand(-3,3),rand(.8,1.6),rand(.5,1),rand(.8,1.6),0x6a6a6a,rand(0,TAU));
  }
  explode(st.x,y+1,st.z,14,220,st.side==='us'?false:true,6,2.5);
  sCollapse({x:st.x,z:st.z});
  var list=st.side==='us'?usSpawns:vcSpawns;
  for(var k=list.length-1;k>=0;k--) if(list[k]===st) list.splice(k,1);
  var mark=st.side==='us'?'\u271D ':'\u2605 ';
  killfeed(mark+st.label+' IS DESTROYED');
  radioSay(st.side==='us'?'We lost the base! We lost the base!':'Good effect on target! Base is destroyed!');
  if(st.label.indexOf('COMMAND')>=0||st.label.indexOf('HQ')>=0) checkWin();
}

/* ---------- bunkers ---------- */
function buildBunker(site){
  var x=site.x, z=site.z;
  var fy=site.h;
  var S=3.5, WH=2.6, TH=.45, gap=.8;
  var i0=blockN;
  var blkSolids=[];
  function blk(bx,by,bz,sx,sy,sz,color,solidOpts){
    var id=addBlock(bx,by,bz,sx,sy,sz,color);
    if(solidOpts!==false){
      var s=addSolid(bx-sx/2,by-sy/2,bz-sz/2,bx+sx/2,by+sy/2,bz+sz/2,solidOpts==='walk',true);
      blkSolids.push(s);
    }
    return id;
  }
  /* slab */
  blk(x,fy-.25,z,S*2+1,.5,S*2+1,0x757575,'walk');
  /* north wall */
  blk(x,fy+WH/2,z-S,S*2,WH,TH,0x757575,'walk');
  /* east and west walls */
  blk(x+S,fy+WH/2,z,TH,WH,S*2,0x5f5f5f,'walk');
  blk(x-S,fy+WH/2,z,TH,WH,S*2,0x5f5f5f,'walk');
  /* south wall with door gap */
  var segW=(S*2-gap)/2;
  blk(x-gap/2-segW/2,fy+WH/2,z+S,segW,WH,TH,0x757575,'walk');
  blk(x+gap/2+segW/2,fy+WH/2,z+S,segW,WH,TH,0x757575,'walk');
  blk(x,fy+WH-.35,z+S,gap,.7,TH,0x5f5f5f,'walk');
  /* roof lip and slab */
  blk(x,fy+WH+.2,z,S*2+.4,.4,.5,BAG,'walk');
  blk(x,fy+WH+.6,z,S*2+.2,.5,S*2+.2,0x757575,'walk');
  /* parapet at z+2.2 */
  for(var b=0;b<5;b++){
    var px=x-1.6+b*.8;
    blk(px,fy+.35,z+2.2,.8,.7,.6,BAG_D,'walk');
  }
  blk(x-1.6,fy+.9,z+2.2,.8,.5,.6,BAG,'walk');
  blk(x+1.6,fy+.9,z+2.2,.8,.5,.6,BAG,'walk');
  /* wing walls */
  blk(x-S-1,fy+1,z+S+1.5,.5,2,3.4,0x5f5f5f,'walk');
  blk(x+S+1,fy+1,z+S+1.5,.5,2,3.4,0x5f5f5f,'walk');
  var i1=blockN;

  /* sliding door */
  var gapW=gap;
  var doorMesh=new THREE.Mesh(unitBoxGeo(),blockMat);
  doorMesh.scale.set(gapW*2-.06,2.4,.22);
  doorMesh.material=new THREE.MeshLambertMaterial({map:blockMat.map,color:0x8d6e4f});
  doorMesh.position.set(x,fy+1.2,z+S);
  doorMesh.castShadow=true;
  scene.add(doorMesh);
  var doorSolid=addSolid(x-gapW,fy,z+S-.15,x+gapW,fy+2.4,z+S+.15,false,true);
  var inter={
    type:'door', x:x, z:z+S, y:fy+1,
    label:'OPEN DOOR', open:false, t:0,
    mesh:doorMesh, solid:doorSolid, baseY:fy+1.2
  };
  interactables.push(inter);

  /* interior props */
  var crate={type:'ammo',x:x-S+1,z:z-1,y:fy,label:'AMMO CRATE',cd:0};
  interactables.push(crate);
  addBlock(x-S+1,fy+.4,z-1,.9,.8,.6,0x5d4033);
  var med={type:'med',x:x+S-1,z:z-1,y:fy,label:'MEDKIT',cd:0};
  interactables.push(med);
  addBlock(x+S-1,fy+.3,z-1,.6,.5,.4,0xe8e8e8);
  makeBarrel(x+1.8,fy,z-1.6);

  var bunker={
    x:x, z:z, fy:fy,
    garrison:[], cleared:false, dead:false,
    hmg:null, door:inter, gunner:null, mortar:null,
    assaultT:-99, hp:2400, maxHp:2400, r:6.5, cratR:7.5,
    i0:i0, i1:i1, blkSolids:blkSolids,
    mortarMesh:null, mortarCd:rand(6,18), mortarRounds:0,
    announced:{}, smoking:false, label:'BUNKER'
  };
  /* DShK mount */
  var hmgGroup=new ObjT();
  hmgGroup.position.set(x,fy+3.15,z);
  hmgGroup.rotation.y=Math.PI;
  eBox(.7,.12,.7,0x3a3a3a,0,0,0,hmgGroup);
  eBox(.08,.7,.08,0x2a2a2a,-.25,.4,.2,hmgGroup);
  eBox(.08,.7,.08,0x2a2a2a,.25,.4,.2,hmgGroup);
  eBox(.08,.7,.08,0x2a2a2a,-.25,.4,-.2,hmgGroup);
  eBox(.24,.24,1.1,0x4a4a3a,0,.75,-.2,hmgGroup);
  var barrelM=new ObjT(); barrelM.position.set(0,.97,-1.35);
  eBox(.09,.09,1.3,0x2a2a2a,0,0,0,barrelM);
  eBox(.16,.16,.22,0x3a3a3a,0,0,-.62,barrelM);
  hmgGroup.add(barrelM);
  eBox(.5,.4,.6,0x4a3826,.4,.55,.3,hmgGroup);
  var muzzle=new ObjT(); muzzle.position.set(0,.97,-1.8);
  hmgGroup.add(muzzle);
  scene.add(hmgGroup);
  bunker.hmg=hmgGroup;
  bunker.hmgMuzzle=muzzle;
  bunker.hmgPivot=new ObjT();
  hmgGroup.add(bunker.hmgPivot);

  /* mortar pit 2.4 behind */
  var mg=new ObjT();
  mg.position.set(x,fy,z+S+4.6);
  eBox(.8,.1,.8,0x5a5a4a,0,.05,0,mg);
  var tube=eBox(.18,.18,1.1,0x3a3a3a,0,.5,0,mg);
  tube.rotation.x=-1.05;
  eBox(.7,.4,.3,BAG_D,0,.2,.6,mg);
  eBox(.7,.4,.3,BAG_D,0,.2,-.6,mg);
  for(var r2=0;r2<3;r2++) eBox(.12,.24,.12,0x6a5836,.5+r2*.14,.12,-.5+r2*.2,mg);
  scene.add(mg);
  bunker.mortarMesh=mg;
  bunker.mortar={cd:rand(6,18),rounds:0};

  /* garrison */
  var innerBox={x0:x-2.4,x1:x+2.4,z0:z-2.4,z1:z+2.4};
  for(var g=0;g<5;g++){
    var gx=x+rand(-1.6,1.6), gz=z+rand(-1.6,1.6);
    var man=makeEnemy(gx,gz,{hp:120,garrison:innerBox,floorY:fy,bref:bunker,stanceC:1});
    bunker.garrison.push(man);
  }
  for(var pr=0;pr<2;pr++){
    var pxB=x+(pr===0?-1.2:1.2);
    var manB=makeEnemy(pxB,fy,{hp:85,post:true,floorY:fy,bref:bunker,prone:true});
    manB.x=pxB; manB.z=z+2.2;
    bunker.garrison.push(manB);
  }
  var gunner=makeEnemy(x,z,{hp:100,hmg:true,floorY:fy+3.15,bref:bunker});
  gunner.stance=2;
  bunker.garrison.push(gunner);
  bunker.gunner=gunner;

  bunkerList.push(bunker);
  vcSpawns.push(bunker);
  return bunker;
}

function damageBunker(bk,dmg){
  if(bk.dead) return;
  bk.hp-=dmg;
  var frac=bk.hp/bk.maxHp;
  spawnParticles(bk.x,bk.fy+2,bk.z,5,0x8a8a8a,2);
  if(frac<.6&&!bk.smoking){ bk.smoking=true; smokingStructs.push(bk); }
  if(frac<.66&&!bk.announced[66]){ bk.announced[66]=1; killfeed('ENEMY BUNKER HEAVILY DAMAGED'); }
  if(frac<.33&&!bk.announced[33]){ bk.announced[33]=1; killfeed('ENEMY BUNKER FALLING APART'); }
  if(bk.hp<=0) destroyBunker(bk);
}
function destroyBunker(bk){
  bk.dead=true; bk.cleared=true;
  for(var i=bk.i0;i<bk.i1;i++){
    /* hide via chunk id ranges */
  }
  hideRange(bk.i0,bk.i1);
  for(var s=0;s<bk.blkSolids.length;s++) bk.blkSolids[s].off=true;
  if(bk.door){
    scene.remove(bk.door.mesh);
    bk.door.solid.off=true;
    var ii=interactables.indexOf(bk.door);
    if(ii>=0) interactables.splice(ii,1);
  }
  if(bk.hmg) scene.remove(bk.hmg);
  if(bk.mortarMesh) scene.remove(bk.mortarMesh);
  var crew=0;
  for(var g=0;g<bk.garrison.length;g++){
    var man=bk.garrison[g];
    if(!man.dead){
      crew++;
      man.flingV={x:rand(-4,4),y:rand(3,7),z:rand(-4,4)};
      damageEnemy(man,9999,false);
    }
  }
  US+=3;
  killfeed('ENEMY BUNKER DESTROYED \u2014 CREW KIA \u2014 SPIGOT CLOSED'+(crew?' ('+crew+')':''));
  sCollapse({x:bk.x,z:bk.z});
  explode(bk.x,bk.fy+1,bk.z,12,180,false,6,2.5);
  for(var r=0;r<16;r++)
    addBlock(bk.x+rand(-3,3),bk.fy+rand(0,1),bk.z+rand(-3,3),rand(.8,1.6),rand(.5,1.1),rand(.8,1.6),[0x757575,0x5f5f5f,0x6a6a6a][r%3],rand(0,TAU));
  banner('BUNKER DESTROYED +3 \u2605');
  radioSay('Bunker destroyed! Good effect on target!');
  for(var v=vcSpawns.length-1;v>=0;v--) if(vcSpawns[v]===bk) vcSpawns.splice(v,1);
}
function hideRange(a,b){
  /* block ids a..b are global ordinals; convert per chunk */
  for(var i=a;i<b;i++) hideBlock(i);
}
function checkBunkersCleared(){
  for(var b=0;b<bunkerList.length;b++){
    var bk=bunkerList[b];
    if(bk.cleared||bk.dead) continue;
    var allDead=true;
    for(var g=0;g<bk.garrison.length;g++)
      if(!bk.garrison[g].dead){ allDead=false; break; }
    if(allDead){
      bk.cleared=true;
      US+=3;
      banner('BUNKER CLEARED +3 \u2605');
      radioSay('Bunker cleared! Bunker cleared, good job!');
      for(var v=vcSpawns.length-1;v>=0;v--) if(vcSpawns[v]===bk) vcSpawns.splice(v,1);
      vcCounterattack(bk);
    }
  }
}
function vcCounterattack(bk){
  var n=Math.min(7,vcPool);
  var spawned=0;
  for(var i=0;i<n;i++){
    var a=rand(0,TAU), d=rand(12,20);
    var x=bk.x+Math.cos(a)*d, z=bk.z+Math.sin(a)*d;
    if(x<-HALF+3||x>HALF-3||z<-HALF+3||z>HALF-3) continue;
    var e=makeEnemy(x,z,{hunt:{x:bk.x,z:bk.z}});
    if(e&&!e.remove){ spawned++; e.state='hunt'; e.alertT=time+30; e.lastSeen={x:bk.x,z:bk.z}; }
  }
  if(spawned) killfeed('\u25B2 CHARLIE COUNTERATTACKS THE BUNKER!');
}

/* ---------- VC camps ---------- */
function buildVCCamp(site){
  var x=site.x, z=site.z, fy=site.h;
  for(var h=0;h<3;h++){
    var hx=x+rand(-6,6), hz=z+rand(-6,6);
    var w=rand(2.4,3.2);
    for(var wY=0;wY<2;wY++)
      addBlock(hx,fy+.5+wY,hz,w,.6,w,0x6b5836);
    addSolid(hx-w/2,fy,hz-w/2,hx+w/2,fy+1.4,hz+w/2,false,true);
    addBlock(hx,fy+1.65,hz,w+.5,.3,w+.5,0xb8860b);
    addBlock(hx,fy+1.95,hz,w,.3,w,0xa87608);
    addBlock(hx,fy+2.2,hz,w-.5,.25,w-.5,0xb8860b);
  }
  /* campfire */
  var fx=x+rand(-3,3), fz=z+rand(-3,3);
  addBlock(fx,fy+.15,fz,.8,.3,.8,0x4a3826);
  var fl=new THREE.PointLight(0xff8830,0,9);
  if(!IS_TOUCH){ fl.position.set(fx,fy+1,fz); scene.add(fl); }
  campfires.push({x:fx,z:fz,y:fy+.4,light:fl,mesh:null});
  var fireMesh=eBox(.5,.5,.5,0xff5a10,fx,fy+.5,fz);
  fireMesh.material=new THREE.MeshBasicMaterial({color:0xff5a10});
  campfires[campfires.length-1].mesh=fireMesh;
  var st=makeStruct('vc',x,z,{label:'VC CAMP',hp:800,r:5,cratR:8});
  /* guards */
  for(var g2=0;g2<4;g2++){
    var man=makeEnemy(x+rand(-4,4),z+rand(-4,4),{home:{x:x,z:z},roam:8,garrison:{x0:x-7,x1:x+7,z0:z-7,z1:z+7}});
  }
  spawnChicken(x+2,z+2);
  spawnChicken(x-2,z+3);
}

/* ---------- US base ---------- */
function buildUSBase(site){
  var x=site.x, z=site.z, fy=site.h;
  /* sandbag ring radius 11 x 8 with south gate */
  for(var a=0;a<32;a++){
    var ang=a/32*TAU;
    var rx=Math.cos(ang)*11, rz=Math.sin(ang)*8;
    var gx=x+rx, gz=z+rz;
    if(Math.abs(gx-x)<2.2&&gz>z+6) continue; /* south gate */
    var gy=heightAt(gx+HALF,gz+HALF);
    addBlock(gx,gy+.35,gz,1.4,.7,.8,BAG);
    if(a%2===0) addBlock(gx,gy+.9,gz,1.2,.5,.7,BAG_D);
  }
  /* flag pole with canvas flag */
  var pole=new THREE.Mesh(unitBoxGeo(),matCache[0x8a8a8a]||new THREE.MeshLambertMaterial({color:0x9a9a9a}));
  pole.scale.set(.18,7,.18);
  pole.position.set(x,fy+3.5,z-4);
  scene.add(pole);
  var flagCanvas=document.createElement('canvas');
  flagCanvas.width=60; flagCanvas.height=34;
  var fc=flagCanvas.getContext(CTXID);
  for(var s2=0;s2<13;s2++){
    fc.fillStyle=s2%2?'#ffffff':'#B22234';
    fc.fillRect(0,s2*(34/13),60,34/13+1);
  }
  fc.fillStyle='#3C3B6E';
  fc.fillRect(0,0,26,15);
  fc.fillStyle='#ffffff';
  for(var dy=0;dy<5;dy++)for(var dx2=0;dx2<6;dx2++){
    fc.fillRect(2+dx2*4,1.5+dy*3,1.6,1.6);
  }
  var flagTex=new THREE.CanvasTexture(flagCanvas);
  var flag=new THREE.Mesh(new THREE.PlaneGeometry(2.6,1.5),
    new THREE.MeshBasicMaterial({map:flagTex,side:THREE.DoubleSide}));
  flag.position.set(x+1.35,fy+6.4,z-4);
  scene.add(flag);
  /* tents */
  for(var t2=0;t2<2;t2++){
    var tx=x+(t2===0?-6:6), tz=z-6;
    addBlock(tx,fy+.7,tz,3.4,1.4,2.6,0x4a5a30);
    addBlock(tx,fy+1.6,tz,2.6,.8,2,0x3c4a26);
  }
  /* watchtower */
  for(var l=0;l<4;l++){
    var lx=x+(l%2?3:-3), lz=z+(l<2?-3:3);
    addBlock(lx,fy+2.6,lz,.35,5.2,.35,0x5d4033);
  }
  addBlock(x,fy+4.6,z,4,.3,4,0x6d4c33);
  addSolid(x-2,fy+4.4,z-2,x+2,fy+4.8,z+2,true,false);
  addBlock(x,fy+5.35,z,4,.5,.3,BAG);
  addBlock(x,fy+5.35,z-2,4,.5,.3,BAG);
  /* crates */
  addBlock(x-8,fy+.5,z+2,1.4,1,1,0x5d4033);
  interactables.push({type:'ammo',x:x-8,z:z+2,y:fy,label:'AMMO CRATE',cd:0});
  addBlock(x+8,fy+.4,z+2,.8,.7,.6,0xe8e8e8);
  interactables.push({type:'med',x:x+8,z:z+2,y:fy,label:'MEDKIT',cd:0});
  makeBarrel(x-7,fy,z-2);
  makeBarrel(x+7,fy,z-2);
  /* HQ command post */
  var core=eBox(3.2,2.6,3.2,0x4b5320,x,fy+1.3,z);
  addSolid(x-1.6,fy,z-1.6,x+1.6,fy+2.6,z+1.6,false,true);
  var ant=new ObjT();
  eBox(.1,5,.1,0x8a8a8a,0,2.5,0,ant);
  var tip=eBox(.16,.16,.16,0xff3020,0,5.1,0,ant);
  tip.material=new THREE.MeshBasicMaterial({color:0xff3020});
  ant.position.set(x,fy+2.6,z);
  scene.add(ant);
  var st=makeStruct('us',x,z,{label:'HQ COMMAND POST',hp:3500,r:9,cratR:14.5});
  st.core=core; st.antenna=ant;
  usBaseStruct=st;
}
var usBaseStruct=null;

/* ---------- FOBs ---------- */
function buildFOB(site){
  var x=site.x, z=site.z, fy=site.h;
  for(var a=0;a<24;a++){
    var ang=a/24*TAU;
    var gx=x+Math.cos(ang)*8, gz=z+Math.sin(ang)*6.4;
    if(Math.abs(gx-x)<2&&gz>z+4.5) continue;
    var gy=heightAt(gx+HALF,gz+HALF);
    addBlock(gx,gy+.35,gz,1.3,.7,.7,BAG);
    if(a%2===0) addBlock(gx,gy+.9,gz,1.1,.5,.6,BAG_D);
  }
  var core=eBox(3.4,2.4,3.4,0x4b5320,x,fy+1.2,z);
  addSolid(x-1.7,fy,z-1.7,x+1.7,fy+2.4,z+1.7,false,true);
  addBlock(x,fy+2.65,z,3.8,.4,3.8,0x6d4c33);
  addSolid(x-1.9,fy+2.4,z-1.9,x+1.9,fy+2.8,z+1.9,true,false);
  var ant=new ObjT();
  eBox(.08,3.5,.08,0x8a8a8a,0,1.75,0,ant);
  ant.position.set(x,fy+2.4,z);
  scene.add(ant);
  addBlock(x-5,fy+.4,z+3,1.2,.9,.9,0x5d4033);
  interactables.push({type:'ammo',x:x-5,z:z+3,y:fy,label:'AMMO CRATE',cd:0});
  addBlock(x+5,fy+.35,z+3,.7,.6,.5,0xe8e8e8);
  interactables.push({type:'med',x:x+5,z:z+3,y:fy,label:'MEDKIT',cd:0});
  makeBarrel(x-4,fy,z-3);
  makeBarrel(x+4,fy,z-3);
  var st=makeStruct('us',x,z,{label:'FOB',hp:1500,r:7,cratR:10.8});
  st.core=core; st.antenna=ant;
  return st;
}

/* ---------- VC command post ---------- */
function buildVCPost(site){
  var x=site.x, z=site.z, fy=site.h;
  /* outer ring 11 x 9 */
  for(var a=0;a<28;a++){
    var ang=a/28*TAU;
    var gx=x+Math.cos(ang)*11, gz=z+Math.sin(ang)*9;
    var gy=heightAt(gx+HALF,gz+HALF);
    addBlock(gx,gy+.6,gz,1.6,1.4,1.2,0x4a4a3a);
    addSolid(gx-.8,gy,gz-.6,gx+.8,gy+1.3,gz+.6,false,true);
  }
  /* inner low ring */
  for(var b=0;b<16;b++){
    var angB=b/16*TAU;
    var bx=x+Math.cos(angB)*5.5, bz=z+Math.sin(angB)*4.5;
    var by=heightAt(bx+HALF,bz+HALF);
    addBlock(bx,by+.3,bz,1.4,.6,1,0x5a5a48);
  }
  /* 3 towers */
  var towers=[[-9,-7],[9,-7],[0,8]];
  for(var t=0;t<towers.length;t++){
    var tx=x+towers[t][0], tz=z+towers[t][1];
    var ty=heightAt(tx+HALF,tz+HALF);
    addBlock(tx,ty+2.5,tz,.5,5,.5,0x5d4033);
    addBlock(tx,ty+5.2,tz,3,.3,3,0x6d4c33);
    addSolid(tx-1.5,ty+5,tz-1.5,tx+1.5,ty+5.4,tz+1.5,true,false);
    eBox(.12,.12,1.4,0x2a2a2a,tx,ty+5.6,tz-.9);
    var gunner=makeEnemy(tx,tz,{hp:110,hmg:true,floorY:ty+5.5});
    gunner.x=tx; gunner.z=tz; gunner.stance=2;
  }
  /* blockhouse */
  var core=eBox(4.4,3,4.4,0x757575,x,fy+1.5,z);
  addSolid(x-2.2,fy,z-2.2,x+2.2,fy+3,z+2.2,false,true);
  addBlock(x,fy+3.25,z,5,.5,5,0x5f5f5f);
  var ant=new ObjT();
  eBox(.1,6,.1,0x8a8a8a,0,3,0,ant);
  ant.position.set(x,fy+3,z);
  scene.add(ant);
  addBlock(x-3,fy+.4,z+3,1.2,.9,.9,0x5d4033);
  interactables.push({type:'ammo',x:x-3,z:z+3,y:fy,label:'AMMO CRATE',cd:0});
  makeBarrel(x+2.5,fy,z+2.5);
  makeBarrel(x+2.5,fy,z+1);
  makeBarrel(x+1,fy,z+2.5);
  var st=makeStruct('vc',x,z,{label:'VC COMMAND POST',hp:3500,r:8,cratR:13.5});
  st.core=core; st.antenna=ant;
  vcPostStruct=st;
  spawnChicken(x+4,z-2);
  spawnChicken(x-4,z+2);
}
var vcPostStruct=null;

/* ---------- doors / interactables ---------- */
function updateDoors(dt){
  for(var i=0;i<interactables.length;i++){
    var it=interactables[i];
    if(it.cd>0) it.cd-=dt;
    if(it.type!=='door') continue;
    if(it.anim>0||it.anim<0){
      it.t=clamp(it.t+it.anim*dt*3,0,1);
      it.mesh.position.y=it.baseY-it.t*2.15;
      if(it.t>=1||it.t<=0){
        it.anim=0;
        it.solid.off=it.t>.4;
      } else it.solid.off=it.t>.4;
    }
  }
}
function nearestInteract(){
  var best=null, bestD=3.2;
  var px=P.pos.x, py=P.pos.y+1, pz=P.pos.z;
  for(var i=0;i<interactables.length;i++){
    var it=interactables[i];
    if(it.cd>0) continue;
    var dx=it.x-px, dz=it.z-pz, dy=(it.y||0)-py;
    var d=Math.sqrt(dx*dx+dz*dz+dy*dy);
    if(d>bestD) continue;
    if(d>1.2){
      var fw=camera.getWorldDirection(_fwV);
      var dot=(dx*fw.x+dz*fw.z)/(d||1);
      if(dot<.45) continue;
    }
    best=it; bestD=d;
  }
  return best;
}
var _fwV=null;
function doInteract(it){
  if(!it) return;
  if(it.type==='door'){
    it.open=!it.open;
    it.anim=it.open?1:-1;
    it.label=it.open?'CLOSE DOOR':'OPEN DOOR';
    sClick();
  } else if(it.type==='ammo'){
    it.cd=15;
    P.pools.pistol=POOLMAX.pistol;
    P.pools.smg=POOLMAX.smg;
    P.pools.rifle=POOLMAX.rifle;
    if(P.nades<4) P.nades=4;
    killfeed('AMMO RESUPPLIED');
    sPickup();
  } else if(it.type==='med'){
    if(P.hp>=100){ killfeed('ALREADY AT FULL HEALTH'); it.cd=2; }
    else{
      it.cd=25;
      P.hp=100;
      killfeed('PATCHED UP \u2014 HP 100');
      sHeal();
    }
  }
}
