'use strict';
/* SECTION 6 — TERRAIN AND WORLD GENERATION */
var GREENS=[0x3a8f2e,0x2f7d26,0x46a038,0x276b1f,0x53a63c];
var LEAVES=[0x256b2f,0x1c5a26,0x2f7d38,0x2e6b27,0x38854a];
var FERNS=[0x1e6b2a,0x2f8f3a,0x17601f,0x27803a];
var DIRT=[0x6d4c33,0x5d4033,0x795548];
var LITTER=[0x5d4a2f,0x6b5836,0x4a3a24];

var cellIdx0=new I32(W*W);
var cellCnt=new U8(W*W);
var cellExtras={};
var worldStructsBuilt=false;
var bridgeSolid=null;

function heightAt(x,z){
  var xi=clamp(x|0,0,W-1), zi=clamp(z|0,0,W-1);
  return heights[zi*W+xi];
}
function riverZ(x){ return Math.sin(x*.045)*14+Math.sin(x*.013+2)*6+8; }
function mixC(a,b,t){
  var ar=(a>>16)&255, ag=(a>>8)&255, ab=a&255;
  var br=(b>>16)&255, bg=(b>>8)&255, bb=b&255;
  var r=(ar+(br-ar)*t)|0, g=(ag+(bg-ag)*t)|0, bl=(ab+(bb-ab)*t)|0;
  return (r<<16)|(g<<8)|bl;
}

/* ---------- segment vs AABB ---------- */
function segAABB(ax,ay,az,bx,by,bz,s){
  var dx=bx-ax, dy=by-ay, dz=bz-az;
  var tmin=0, tmax=1;
  var p=[ax,ay,az], d=[dx,dy,dz], mn=[s.x0,s.y0,s.z0], mx=[s.x1,s.y1,s.z1];
  for(var i=0;i<3;i++){
    if(Math.abs(d[i])<1e-9){
      if(p[i]<mn[i]||p[i]>mx[i]) return -1;
    } else {
      var t1=(mn[i]-p[i])/d[i], t2=(mx[i]-p[i])/d[i];
      if(t1>t2){ var tt=t1; t1=t2; t2=tt; }
      tmin=Math.max(tmin,t1); tmax=Math.min(tmax,t2);
      if(tmin>tmax) return -1;
    }
  }
  return tmin;
}

var _hitPt={x:0,y:0,z:0};
function worldHit(ax,ay,az,bx,by,bz,useLOS){
  var bestT=Infinity, surf='stone';
  /* solids */
  var list=gatherSolids(Math.min(ax,bx),Math.min(az,bz),Math.max(ax,bx),Math.max(az,bz),useLOS,2);
  for(var i=0;i<list.length;i++){
    var s=list[i];
    var t=segAABB(ax,ay,az,bx,by,bz,s);
    if(t>=0&&t<bestT){ bestT=t; surf=s.walk?'wood':'stone'; }
  }
  /* terrain march */
  var dx=bx-ax, dy=by-ay, dz=bz-az;
  var len=Math.sqrt(dx*dx+dy*dy+dz*dz);
  var steps=Math.min(220,Math.ceil(len/.7));
  if(steps<1) steps=1;
  for(var k=1;k<=steps;k++){
    var t=k/steps;
    var px=ax+dx*t, py=ay+dy*t, pz=az+dz*t;
    var gx=clamp(Math.round(px+HALF),0,W-1), gz=clamp(Math.round(pz+HALF),0,W-1);
    var h=heights[gz*W+gx];
    if(py<=h){ if(t<bestT){ bestT=t; surf=h<=WATER+.5?'water':'ground'; } break; }
    if(py<=WATER && h<WATER-.1){ if(t<bestT){ bestT=t; surf='water'; } break; }
  }
  if(bestT===Infinity) return null;
  _hitPt.x=ax+dx*bestT; _hitPt.y=ay+dy*bestT; _hitPt.z=az+dz*bestT;
  return {t:bestT,surf:surf,pt:_hitPt};
}

function hasLOS(ax,ay,az,bx,by,bz){
  var list=gatherSolids(Math.min(ax,bx),Math.min(az,bz),Math.max(ax,bx),Math.max(az,bz),true,2);
  for(var i=0;i<list.length;i++){
    var t=segAABB(ax,ay,az,bx,by,bz,list[i]);
    if(t>=0&&t<1) return false;
  }
  /* coarse terrain march */
  var dx=bx-ax, dy=by-ay, dz=bz-az;
  var len=Math.sqrt(dx*dx+dy*dy+dz*dz);
  var steps=Math.min(90,Math.ceil(len/1.4));
  if(steps<1) steps=1;
  for(var k=1;k<steps;k++){
    var t=k/steps;
    var py=ay+dy*t;
    if(py<heightAt(clamp(Math.round(ax+dx*t+HALF),0,W-1), clamp(Math.round(az+dz*t+HALF),0,W-1))) return false;
  }
  return true;
}

function groundAt(px,py,pz){
  var h=heightAt(px+HALF,pz+HALF);
  var list=gatherSolids(px-1,pz-1,px+1,pz+1,false,.5);
  for(var i=0;i<list.length;i++){
    var s=list[i];
    if(!s.walk) continue;
    if(px>=s.x0-.3&&px<=s.x1+.3&&pz>=s.z0-.3&&pz<=s.z1+.3){
      if(py>=s.y1-.5&&s.y1>h) h=s.y1;
    }
  }
  return h;
}

function collideAt(px,py,pz,h,r,vault){
  if(px<-HALF+2||px>HALF-2||pz<-HALF+2||pz>HALF-2) return true;
  var gy=heightAt(px+HALF,pz+HALF);
  if(gy>py+1.05) return true;
  var list=gatherSolids(px-r,pz-r,px+r,pz+r,false,r+1);
  for(var i=0;i<list.length;i++){
    var s=list[i];
    if(vault&&s.walk&&s.y1<=py+1.0) continue;
    if(px+r>s.x0&&px-r<s.x1&&pz+r>s.z0&&pz-r<s.z1){
      if(py<s.y1-.35&&py+h>s.y0+.05) return true;
    }
  }
  return false;
}

var CRATER_DEPTH_US=2.2, CRATER_DEPTH_VC=1.6;

/* ---------- site finding ---------- */
var vcCamps=[], bunkers=[], fobs=[], vcPost=null, usBase=null;
function findSites(count,zMin,zMax,minH,maxH){
  var out=[];
  var tries=0;
  while(out.length<count&&tries<4000){
    tries++;
    var x=Math.floor(rand(-HALF+40,HALF-40));
    var z=Math.floor(rand(zMin,zMax));
    var h=heights[(z+HALF)*W+(x+HALF)];
    if(h<minH||h>maxH) continue;
    if(Math.abs(z-riverZ(x))<14) continue;
    var ok=true;
    var all=vcCamps.concat(bunkers,fobs);
    if(vcPost) all=all.concat([vcPost]);
    for(var i=0;i<all.length;i++){
      if(Math.hypot(all[i].x-x,all[i].z-z)<36){ ok=false; break; }
    }
    if(ok) for(var j=0;j<out.length;j++){
      if(Math.hypot(out[j].x-x,out[j].z-z)<36){ ok=false; break; }
    }
    if(!ok) continue;
    out.push({x:x,z:z,h:h});
  }
  return out;
}
function flatten(x,z,r,h){
  for(var dz=-r;dz<=r;dz++)for(var dx=-r;dx<=r;dx++){
    var gx=x+dx, gz=z+dz;
    if(gx<-HALF||gx>=HALF||gz<-HALF||gz>=HALF) continue;
    var d=Math.sqrt(dx*dx+dz*dz);
    if(d>r) continue;
    var t=1-d/r;
    var i=(gz+HALF)*W+(gx+HALF);
    heights[i]=Math.round(lerp(heights[i],h,t*t*(3-2*t)));
  }
}

/* ---------- ground pass ---------- */
function groundPass(){
  for(var z=0;z<W;z++)for(var x=0;x<W;x++){
    var i=z*W+x;
    var h=heights[i];
    var wx=x-HALF, wz=z-HALF;
    var color=GREENS[(noise(x*.11,z*.11)*GREENS.length)|0]%GREENS.length;
    color=GREENS[Math.min(GREENS.length-1,(noise(x*.11+3,z*.11+9)*5)|0)];
    var rz=riverZ(wx);
    var dr=Math.abs(wz-rz);
    if(dr<14){
      color=mixC(color,0x2c6e31,1-dr/14);
      if(dr<4.5) color=mixC(color,0x6a5836,1-dr/4.5);
    }
    if(h<=WATER+.4) color=0x8d6e4b;
    var id=addBlock(wx,h-.5,wz,1.02,1,1.02,color);
    cellIdx0[i]=id;
    cellCnt[i]=1;
    /* cliff fill down on east/south drops */
    var hE=x+1<W?heights[i+1]:h;
    var hS=z+1<W?heights[i+W]:h;
    var low=Math.min(hE,hS);
    if(h-low>0){
      var fill=Math.min(6,Math.ceil(h-low));
      for(var f=1;f<=fill;f++)
        addBlock(wx,h-.5-f,wz,1.02,1,1.02,0x795548);
      cellCnt[i]=1+fill;
    }
  }
}

function waterPass(){
  var ids=[];
  for(var z=0;z<W;z++)for(var x=0;x<W;x++){
    var i=z*W+x;
    if(heights[i]<WATER-.2){
      ids.push([x,z]);
    }
  }
  var geo=new THREE.BoxGeometry(1.02,.75,1.02);
  var mat=new THREE.MeshPhongMaterial({color:0x3d7a64,transparent:true,opacity:.78,shininess:110,specular:0x9fd8c8});
  var m=new THREE.InstancedMesh(geo,mat,Math.max(1,ids.length));
  m.instanceColor=new THREE.InstancedBufferAttribute(new F32(Math.max(1,ids.length)*3).fill(1),3);
  for(var k=0;k<ids.length;k++){
    _tmpObj.position.set(ids[k][0]-HALF,WATER-.35,ids[k][1]-HALF);
    _tmpObj.scale.set(1,1,1); _tmpObj.rotation.set(0,0,0);
    _tmpObj.updateMatrix();
    m.setMatrixAt(k,_tmpObj.matrix);
  }
  m.count=ids.length;
  m.receiveShadow=true;
  scene.add(m);
  waterBobber=m;
}

/* ---------- trees ---------- */
function treePass(){
  var placed=0, tries=0;
  while(placed<2200&&tries<30000){
    tries++;
    var x=Math.floor(rand(-HALF+3,HALF-3));
    var z=Math.floor(rand(-HALF+3,HALF-3));
    var h=heights[(z+HALF)*W+(x+HALF)];
    if(h<4) continue;
    if(Math.abs(z-riverZ(x))<6) continue;
    if(Math.hypot(x-usBase.x,z-usBase.z)<18) continue;
    var bad=false;
    var all=vcCamps.concat(bunkers,fobs);
    if(vcPost) all=all.concat([vcPost]);
    for(var i=0;i<all.length;i++){
      if(Math.hypot(all[i].x-x,all[i].z-z)<81){ bad=true; break; }
    }
    if(bad) continue;
    for(var t=0;t<treeTops.length;t++){
      var tt=treeTops[t];
      if(Math.abs(tt.gx-(x+HALF))<4&&Math.abs(tt.gz-(z+HALF))<4){ bad=true; break; }
    }
    if(bad) continue;
    var giant=Math.random()<.07;
    var th=giant?rand(11,14):rand(5,8);
    var cr=giant?rand(4,4.6):rand(2,3);
    var tw=giant?.9:.62;
    var tc=Math.random()<.5?0x6d4c33:0x5d4033;
    var ids=[];
    var trunkH=Math.floor(th);
    for(var y=0;y<trunkH;y++)
      ids.push(addBlock(x,h-.2+y,z,tw,1,tw,tc));
    /* crown ellipsoid */
    var cy=h-.2+trunkH;
    var rad=Math.ceil(cr);
    for(var dy=-rad;dy<=rad;dy++)for(var dz=-rad;dz<=rad;dz++)for(var dx=-rad;dx<=rad;dx++){
      var d2=dx*dx+dz*dz+1.8*dy*dy;
      if(d2>cr*cr+1) continue;
      if(Math.random()>.92) continue;
      var lc=LEAVES[(Math.random()*LEAVES.length)|0];
      var edge=d2/(cr*cr+1);
      if(edge>.72) lc=mixC(lc,0x53a63c,.4);
      ids.push(addBlock(x+dx,cy+dy,z+dz,1.04,1.04,1.04,lc));
    }
    /* vines on giants */
    if(giant){
      var vines=irand(2,4);
      for(var v=0;v<vines;v++){
        var vl=irand(3,6);
        var vx=x+rand(-cr*.6,cr*.6), vz=z+rand(-cr*.6,cr*.6);
        var sway=rand(-.3,.3);
        for(var vy=0;vy<vl;vy++)
          ids.push(addBlock(vx+sway*vy*.2,cy-vy-1,vz+sway*vy*.15,.14,1,.14,0x2f7d38));
      }
    }
    var solid=addSolid(x-tw-.2,h-.7,z-tw-.2,x+tw+.2,h-.2+trunkH,z+tw+.2,false,false);
    treeTops.push({x:x,z:z,gx:x+HALF,gz:z+HALF,ids:ids,solid:solid,cr:cr});
    placed++;
  }
}

function bambooPass(){
  for(var c=0;c<190;c++){
    var cx=rand(-HALF+4,HALF-4), cz=rand(-HALF+4,HALF-4);
    var h=heightAt(cx+HALF,cz+HALF);
    if(h<3.4) continue;
    var n=irand(4,7);
    for(var i=0;i<n;i++){
      var bx=cx+rand(-1.6,1.6), bz=cz+rand(-1.6,1.6);
      var bh=rand(6,10);
      addBlock(bx,h+bh/2,bz,.22,bh,.22,0x9ccc65);
    }
  }
}

function bushPass(){
  for(var i=0;i<950;i++){
    var x=rand(-HALF+3,HALF-3), z=rand(-HALF+3,HALF-3);
    var h=heightAt(x+HALF,z+HALF);
    if(h<3.2) continue;
    if(Math.random()<.75){
      var lc=LEAVES[(Math.random()*LEAVES.length)|0];
      addBlock(x,h+.35,z,1.3,.7,1.3,lc);
      addBlock(x,h+.85,z,1,.55,1,mixC(lc,0x53a63c,.3));
    } else {
      addBlock(x,h+.4,z,rand(.9,1.4),rand(.7,1.1),rand(.9,1.4),0x8f8f8f);
    }
  }
}

function boulderPass(){
  for(var i=0;i<160;i++){
    var x=rand(-HALF+3,HALF-3), z=rand(-HALF+3,HALF-3);
    var h=heightAt(x+HALF,z+HALF);
    if(h<3) continue;
    var s=rand(1.2,2.6);
    var g=irand(0x7a7a7a,0x999999);
    var rot=rand(0,TAU);
    addBlock(x,h+s/2-.2,z,s,s,s,g,rot);
    if(Math.random()<.5) addBlock(x,h+s-.1,z,s*.7,.4,s*.7,0x3a5f2a,rot);
    addSolid(x-s/2,h-.3,z-s/2,x+s/2,h+s-.2,z+s/2,false,true);
  }
}

function undergrowthPass(){
  var target=IS_TOUCH?8000:20000;
  var placed=0, tries=0;
  while(placed<target&&tries<target*3){
    tries++;
    var x=Math.floor(rand(-HALF+2,HALF-2));
    var z=Math.floor(rand(-HALF+2,HALF-2));
    var h=heights[(z+HALF)*W+(x+HALF)];
    if(h<3.6) continue;
    if(noise(x*.05+11,z*.05+5)<.42) continue;
    var cell=(z+HALF)*W+(x+HALF);
    var r=Math.random();
    var id;
    if(r<.55){
      var fc=FERNS[(Math.random()*FERNS.length)|0];
      id=addBlock(x,h+.3,z,.5,.6,.5,fc);
      addBlock(x,h+.75,z,.36,.35,.36,mixC(fc,0x53a63c,.25));
    } else if(r<.86){
      var gb=0x3f9432;
      for(var b=0;b<3;b++)
        addBlock(x+rand(-.25,.25),h+.25+b*.35,z+rand(-.25,.25),.09,1.15-b*.28,.09,gb);
      id=null;
    } else {
      id=addBlock(x,h+.06,z,1,.12,1,LITTER[(Math.random()*LITTER.length)|0]);
    }
    if(id!==null){
      var arr=cellExtras[cell];
      if(!arr){ arr=[]; cellExtras[cell]=arr; }
      arr.push(id);
    }
    placed++;
  }
}

function palmPass(){
  var n=IS_TOUCH?45:95;
  for(var i=0;i<n;i++){
    var x=rand(-HALF+4,HALF-4);
    var z=riverZ(x)+(Math.random()<.5?rand(5,9.5):-rand(5,9.5));
    if(z<-HALF+3||z>HALF-3) continue;
    var h=heightAt(x+HALF,z+HALF);
    if(h<WATER) continue;
    var lean=Math.random()<.5?1:-1;
    var seg=irand(5,7);
    var ids=[];
    for(var s=0;s<seg;s++){
      ids.push(addBlock(x+lean*s*.16,h+1+s,z,.3,1.05,.3,0x8a6a45));
    }
    var tx=x+lean*seg*.16, ty=h+1+seg;
    for(var f=0;f<7;f++){
      var a=f/7*TAU;
      for(var st=0;st<3;st++)
        ids.push(addBlock(tx+Math.cos(a)*(.6+st*.7),ty+.3-st*.22,z+Math.sin(a)*(.6+st*.7),.9,.1,.28,0x4a8f3a,a));
    }
    ids.push(addBlock(tx,ty-.2,z,.3,.3,.3,0x6b5836));
    var solid=addSolid(tx-.5,ty-1,z-.5,tx+.5,ty+.4,z+.5,false,false);
    treeTops.push({x:tx,z:z,gx:tx+HALF,gz:z+HALF,ids:ids,solid:solid,cr:1.5});
  }
}

function logPass(){
  var n=IS_TOUCH?70:150;
  for(var i=0;i<n;i++){
    var x=rand(-HALF+3,HALF-3), z=rand(-HALF+3,HALF-3);
    var h=heightAt(x+HALF,z+HALF);
    if(h<3) continue;
    var rot=Math.random()<.5?0:Math.PI/2;
    addBlock(x,h+.25,z,rot?0.5:2.6,.5,rot?2.6:.5,0x5d4033,0);
    addBlock(x,h+.6,z,rot?0.4:2.2,.4,rot?2.2:.4,0x3a5f2a,0);
  }
}

function canopyShade(){
  for(var t=0;t<treeTops.length;t++){
    var tt=treeTops[t];
    if(!tt.cr||tt.cr<2) continue;
    var rad=Math.ceil(tt.cr+1.2);
    var gx=tt.gx, gz=tt.gz;
    for(var dz=-rad;dz<=rad;dz++)for(var dx=-rad;dx<=rad;dx++){
      var cx=gx+dx, cz=gz+dz;
      if(cx<0||cx>=W||cz<0||cz>=W) continue;
      var d2=dx*dx+dz*dz;
      var r2=tt.cr*tt.cr;
      if(d2>r2+1.44) continue;
      var id=cellIdx0[cz*W+cx];
      if(id<0) continue;
      var k=.78+.14*(d2/r2);
      var ci=(id/100000)|0, idx=id%100000;
      var arr=chunks[ci].instanceColor.array;
      var g=arr[idx*3+1]*.95*k;
      arr[idx*3]*=k;
      arr[idx*3+1]=Math.min(1,g);
      arr[idx*3+2]*=k;
      markChunkDirty(chunks[ci],idx);
    }
  }
}

function buildBridge(){
  var x=-20;
  var rz=riverZ(x);
  var z0=Math.floor(rz-10), z1=Math.ceil(rz+10);
  var y=WATER+.9;
  for(var z=z0;z<=z1;z++){
    addBlock(x,y,z,3.2,.35,1.02,0x6d4c33);
    addBlock(x,y+.45,z-.55,.24,.7,.24,0x5d4033);
    addBlock(x,y+.45,z+.55,.24,.7,.24,0x5d4033);
  }
  for(var z=z0;z<=z0+1;z++)for(var z2=z1-1;z2<=z1;z2++){}
  addBlock(x,y+.45,z0+.5,3.2,.5,.24,0x5d4033);
  addBlock(x,y+.45,z1-.5,3.2,.5,.24,0x5d4033);
  addBlock(x,WATER-1.2,(z0+z1)/2,.5,3,.5,0x4a3826);
  addBlock(x-1.4,WATER-1.2,(z0+z1)/2+2,.5,3,.5,0x4a3826);
  bridgeSolid=addSolid(x-1.6,y-.2,z0-1,x+1.6,y+.2,z1+1,true,false);
}

/* ---------- craters ---------- */
function craterBlocked(x,z){
  for(var i=0;i<structList.length;i++){
    var s=structList[i];
    if(!s.dead&&Math.hypot(s.x-x,s.z-z)<s.cratR) return true;
  }
  for(var b=0;b<bunkerList.length;b++){
    var bk=bunkerList[b];
    if(!bk.dead&&Math.hypot(bk.x-x,bk.z-z)<7.5) return true;
  }
  for(var c=0;c<campfires.length;c++){
    if(Math.hypot(campfires[c].x-x,campfires[c].z-z)<9.5) return true;
  }
  return false;
}
function rebuildCell(gx,gz){
  var cell=gz*W+gx;
  var id=cellIdx0[cell];
  var h=heights[cell];
  var wx=gx-HALF, wz=gz-HALF;
  if(id>=0){
    _tmpObj.position.set(wx,h-.5,wz);
    _tmpObj.scale.set(1.02,1,1.02);
    _tmpObj.rotation.set(0,0,0);
    _tmpObj.updateMatrix();
    var ci=(id/100000)|0, idx=id%100000;
    chunks[ci].setMatrixAt(idx,_tmpObj.matrix);
    var c=DIRT[(Math.random()*DIRT.length)|0];
    _tmpColor.setHex(c);
    chunks[ci].instanceColor.array[idx*3]=Math.min(1,_tmpColor.r*(.92+Math.random()*.16));
    chunks[ci].instanceColor.array[idx*3+1]=Math.min(1,_tmpColor.g*(.92+Math.random()*.16));
    chunks[ci].instanceColor.array[idx*3+2]=Math.min(1,_tmpColor.b*(.92+Math.random()*.16));
    markChunkDirty(chunks[ci],idx);
  }
  var extras=cellExtras[cell];
  if(extras){ for(var e=0;e<extras.length;e++) hideBlock(extras[e]); delete cellExtras[cell]; }
  var fill=irand(1,3);
  var lowH=h;
  if(gx>0) lowH=Math.min(lowH,heights[cell-1]);
  if(gx<W-1) lowH=Math.min(lowH,heights[cell+1]);
  if(gz>0) lowH=Math.min(lowH,heights[cell-W]);
  if(gz<W-1) lowH=Math.min(lowH,heights[cell+W]);
  var need=Math.max(1,Math.min(3,Math.ceil(h-lowH)));
  for(var f=1;f<=need;f++)
    addBlock(wx,h-.5-f,wz,1.02,1,1.02,DIRT[(Math.random()*DIRT.length)|0]);
}
function crater(x,z,rad,depth){
  if(blockN>MAXBLOCKS-6000) return;
  var r=Math.ceil(rad);
  var cx=x+HALF, cz=z+HALF;
  var dug=[];
  for(var dz=-r;dz<=r;dz++)for(var dx=-r;dx<=r;dx++){
    var gx=cx+dx, gz=cz+dz;
    if(gx<1||gx>=W-1||gz<1||gz>=W-1) continue;
    var d=Math.sqrt(dx*dx+dz*dz);
    if(d>rad) continue;
    var cell=gz*W+gx;
    var fall=(1-d/rad)*.95*depth;
    var rim=d<rad*.96?fall:depth*.15;
    heights[cell]=Math.max(1,heights[cell]-rim);
    dug.push(cell);
  }
  for(var i=0;i<dug.length;i++) rebuildCell(dug[i]%W,(dug[i]/W)|0);
  /* rim rebuild */
  var seen={};
  for(var j=0;j<dug.length;j++){
    var cellJ=dug[j], gxJ=cellJ%W, gzJ=(cellJ/W)|0;
    for(var nn=0;nn<4;nn++){
      var nx=gxJ+(nn===1?1:nn===3?-1:0), nz=gzJ+(nn===0?1:nn===2?-1:0);
      var nc=nz*W+nx;
      if(nx<1||nx>=W-1||nz<1||nz>=W-1) continue;
      if(seen[nc]) continue;
      var inDug=false;
      for(var q=0;q<dug.length;q++) if(dug[q]===nc){ inDug=true; break; }
      if(inDug) continue;
      seen[nc]=1;
      rebuildCell(nx,nz);
    }
  }
  /* throw down trees */
  for(var t=0;t<treeTops.length;t++){
    var tt=treeTops[t];
    if(tt.felled) continue;
    if(Math.hypot(tt.x-x,tt.z-z)<rad+1.5){
      tt.felled=true;
      for(var b=0;b<tt.ids.length;b++) hideBlock(tt.ids[b]);
      if(tt.solid) tt.solid.off=true;
      spawnParticles(tt.x,heightAt(tt.x+HALF,tt.z+HALF)+2,tt.z,6,0x8d6e4f,3);
    }
  }
  craterDirty=true;
}

/* ---------- genWorld ---------- */
function genWorld(){
  var i,x,z;
  for(z=0;z<W;z++)for(x=0;x<W;x++){
    i=z*W+x;
    var h=3+noise(x*.03+7,z*.03+3)*7;
    var north=(1-(z+HALF)/(W-1))*6*noise(x*.02+31,z*.02+17);
    h+=north;
    var rz=riverZ(x-HALF);
    var dr=Math.abs(z-rz);
    if(dr<7) h=lerp(h,1.7,1-dr/7);
    heights[i]=clamp(Math.round(h),1,15);
  }
  for(var fi=0;fi<W*W;fi++) cellIdx0[fi]=-1;
  /* sites */
  usBase={x:0,z:HALF-14};
  var bh=heights[(usBase.z+HALF)*W+(usBase.x+HALF)];
  usBase.h=Math.max(4,Math.min(12,bh));
  flatten(usBase.x,usBase.z,14,usBase.h);

  var campSites=findSites(4,-HALF+20,-100,4,12);
  for(var c=0;c<campSites.length;c++) flatten(campSites[c].x,campSites[c].z,11,campSites[c].h);
  var bunSites=findSites(8,-178,30,4,12);
  for(var b=0;b<bunSites.length;b++) flatten(bunSites[b].x,bunSites[b].z,10,bunSites[b].h);
  var fobSites=findSites(3,24,130,4,12);
  for(var f=0;f<fobSites.length;f++) flatten(fobSites[f].x,fobSites[f].z,12,fobSites[f].h);
  var postSites=findSites(1,-HALF+30,-HALF+70,4,12);
  if(!postSites.length) postSites=[{x:irand(-100,100),z:-160,h:8}];
  for(var p=0;p<postSites.length;p++) flatten(postSites[p].x,postSites[p].z,13,postSites[p].h);

  groundPass();
  waterPass();
  treePass();
  bambooPass();
  bushPass();
  boulderPass();
  undergrowthPass();
  palmPass();
  logPass();
  canopyShade();
  buildBridge();

  for(c=0;c<campSites.length;c++) buildVCCamp(campSites[c]);
  for(b=0;b<bunSites.length;b++) buildBunker(bunSites[b]);
  for(f=0;f<fobSites.length;f++) buildFOB(fobSites[f]);
  buildVCPost(postSites[0]);
  buildUSBase(usBase);

  worldBuilt=true;
  vcCamps=campSites;
  bunkers=bunSites;
  fobs=fobSites;
  vcPost=postSites[0];
}
