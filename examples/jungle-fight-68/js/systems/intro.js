'use strict';
/* SECTION 17 — INTRO CINEMATIC */
var intro=null;
var INTRO_CAPTIONS=[
  {t:1.5,txt:'\u00AB DELTA SIX \u2014 ANVIL FLIGHT OF THREE INBOUND, LZ RIDGE \u00BB'},
  {t:6.5,txt:'\u00AB BE ADVISED: FRIENDLY PLATOONS ALREADY IN THE TREELINES NORTH \u00BB'},
  {t:12,txt:'\u00AB WE GOT CONTACT SMOKE OFF THE HAMLET \u2014 CHARLIE\u2019S STILL HOME \u00BB'},
  {t:17,txt:'\u00AB THIRTY SECONDS OUT. LOCK AND LOAD, GENTLEMEN \u00BB'},
  {t:22.5,txt:'\u00AB ANVIL 2-1 ON THE GROUND \u2014 GO! GO! GO! \u00BB'}
];
var GUILLEMS='\u00AB\u00BB';
function stripGuillemets(s){
  var out='';
  for(var i=0;i<s.length;i++)
    if(s[i]!==GUILLEMS[CHAR_ZERO]&&s[i]!==GUILLEMS[CHAR_ONE]) out+=s[i];
  return out;
}
var CHAR_ZERO=0, CHAR_ONE=1;
var introCapA={zero:0};
var introCapIdx=0;
function makeHuey(){
  var g=new ObjT();
  eBox(1.5,1.6,5,0x3c4a26,0,0,0,g);
  eBox(1.2,.9,1.8,0x22304a,0,.9,.5,g);
  eBox(8.4,.16,1.6,0x354421,0,.3,0,g);
  eBox(2.8,.14,1.1,0x354421,0,.4,2.4,g);
  eBox(.16,1.5,1.1,0x354421,0,1,-2.3,g);
  eBox(.5,.4,2.6,0x2c3820,1,.1,-1,g);
  eBox(.5,.4,2.6,0x2c3820,-1,.1,-1,g);
  eBox(.2,.2,1.4,0x222222,.95,0,1.4,g);
  eBox(.2,.2,1.4,0x222222,-.95,0,1.4,g);
  var rotor=eBox(7,.1,.42,0x1c1c1c,0,1.5,0,g);
  var tailRotor=eBox(.1,1.6,.3,0x1c1c1c,.3,.8,-2.5,g);
  var skidL=eBox(.14,.14,4,0x222222,.8,-1,0,g);
  var skidR=eBox(.14,.14,4,0x222222,-.8,-1,0,g);
  g.userData.rotor=rotor;
  g.userData.tailRotor=tailRotor;
  return g;
}
function startIntro(){
  var lz={x:usBase.x+8,z:usBase.z-11};
  var ships=[];
  for(var i=0;i<3;i++){
    var h=makeHuey();
    h.position.set(-HALF-60+i*14,44+i,HALF+30+i*10);
    scene.add(h);
    ships.push({g:h,i:i});
  }
  intro={
    t:0,ships:ships,lz:lz,phase:'fly',
    warT:0,capIdx:0,
    riders:[],dustT:0
  };
  /* hide the soldiers during flight */
  for(var s=0;s<soldiers.length;s++){
    var man=soldiers[s];
    if(man.platoon===2){ intro.riders.push(man); charHide(man._pool,man._slot); man._introHidden=true; }
  }
  el('lbTop').style.opacity=1;
  el('lbBot').style.opacity=1;
  startWash();
  startIntroMusic();
  radioSay(stripGuillemets(INTRO_CAPTIONS[introCapA.zero].txt),{prio:2});
  el('capbar').textContent=INTRO_CAPTIONS[0].txt;
}
function introBezier(ship,t){
  var start=ship.g.position0||{x:-HALF-60+ship.i*14,y:44+ship.i,z:HALF+30+ship.i*10};
  var end={x:intro.lz.x+ship.i*6,y:0,z:intro.lz.z};
  var mid={x:(start.x+end.x)/2-45,y:(start.y+end.y)/2,z:(start.z+end.z)/2-30};
  var u=t, iu=1-u;
  var x=iu*iu*start.x+2*iu*u*mid.x+u*u*end.x;
  var y=iu*iu*start.y+2*iu*u*mid.y+u*u*end.y;
  var z=iu*iu*start.z+2*iu*u*mid.z+u*u*end.z;
  return {x:x,y:y,z:z};
}
function smoothstep(t){ return t*t*(3-2*t); }
function updateIntro(dt){
  if(!intro) return;
  intro.t+=dt;
  var t=intro.t;
  var lead=intro.ships[0];
  if(intro.phase==='fly'){
    var ft=smoothstep(clamp(t/22,0,1));
    for(var i=0;i<intro.ships.length;i++){
      var sh=intro.ships[i];
      var p=introBezier(sh,ft);
      var yBase=heightAt(p.x+HALF,p.z+HALF);
      sh.g.position.set(p.x,Math.max(p.y,yBase+3)+Math.sin(t*2+i)*.4,p.z);
      sh.g.rotation.x=-.08;
      sh.g.rotation.y=Math.atan2(p.x-(intro.lz.x),p.z-(intro.lz.z))+Math.PI;
      sh.g.userData.rotor.rotation.y+=dt*28;
      sh.g.userData.tailRotor.rotation.x+=dt*34;
    }
    var lp=lead.g.position;
    var back={x:lp.x+Math.sin(lead.g.rotation.y)* -16,z:lp.z+Math.cos(lead.g.rotation.y)*-16};
    camera.position.set(back.x+5,lp.y+4,back.z);
    camera.lookAt(lp.x+lead.g.rotation.x*0,lp.y,lp.z+14);
    camera.rotation.z=0;
    /* staged war below */
    intro.warT-=dt;
    if(intro.warT<=0){
      intro.warT=rand(.45,1.1);
      var n=irand(3,6);
      var wx=lp.x+rand(-150,150), wz=lp.z+rand(-150,150);
      for(var k=0;k<n;k++)
        (function(delay){setTimeout(function(){gunSoundPos('ak',{x:wx,z:wz});},delay);})(k*100);
      if(Math.random()<.5){
        var ex=wx+rand(-30,30),ez=wz+rand(-30,30);
        sExplosion({x:ex,z:ez});
        for(var sm=0;sm<6;sm++)
          spawnParticles(ex,heightAt(ex+HALF,ez+HALF)+2+sm,ez,2,0x444444,2);
      }
    }
    /* captions */
    while(intro.capIdx<INTRO_CAPTIONS.length&&t>=INTRO_CAPTIONS[intro.capIdx].t){
      var cap=INTRO_CAPTIONS[intro.capIdx];
      el('capbar').textContent=cap.txt;
      if(introCapIdx>0) radioSay(stripGuillemets(cap.txt),{prio:2});
      intro.capIdx++;
    }
    if(t>=22){ intro.phase='land'; intro.t2=0; }
  } else if(intro.phase==='land'){
    intro.t2=(intro.t2||0)+dt;
    var lt=clamp(intro.t2/4.5,0,1);
    var groundY=heightAt(intro.lz.x+HALF,intro.lz.z+HALF);
    for(var l=0;l<intro.ships.length;l++){
      var shB=intro.ships[l];
      var pB=introBezier(shB,1);
      var startX=shB.g.position.x,startY=shB.g.position.y,startZ=shB.g.position.z;
      var targetY=groundY+1.35;
      shB.g.position.y=lerp(startY,targetY,smoothstep(lt));
      shB.g.rotation.x=lerp(-.08,.12,smoothstep(lt));
      shB.g.userData.rotor.rotation.y+=dt*28*(1-lt*.3);
    }
    /* dust bursts under 9 */
    intro.dustT-=dt;
    if(intro.dustT<=0&&lt>.3){
      intro.dustT=.2;
      for(var d=0;d<3;d++){
        var dg=intro.ships[d].g;
        spawnParticles(dg.position.x+rand(-3,3),groundY+.4,dg.position.z+rand(-3,3),3,0x9a8a6a,2);
      }
    }
    var gp=lead.g.position;
    var gx=intro.lz.x+18,gz=intro.lz.z+14;
    camera.position.set(gx,groundY+2.5,gz);
    camera.lookAt(gp.x,gp.y+1,gp.z);
    if(lt>=1){ intro.phase='unload'; intro.uT=0; intro.riderIdx=0; intro.orbitA=0; }
  } else if(intro.phase==='unload'){
    intro.uT+=dt;
    /* hop out every .17s alternating doors */
    if(intro.riderIdx<intro.riders.length&&intro.uT>intro.riderIdx*.17){
      var man=intro.riders[intro.riderIdx];
      var ship=intro.ships[intro.riderIdx%3];
      var door=(intro.riderIdx%2===0)?1:-1;
      man.x=ship.g.position.x+door*1.6;
      man.z=ship.g.position.z;
      man.y=ship.g.position.y-1;
      man._introHidden=false;
      var ang=(intro.riderIdx/intro.riders.length)*TAU;
      man._ringX=intro.lz.x+Math.cos(ang)*7;
      man._ringZ=intro.lz.z+Math.sin(ang)*7;
      intro.riderIdx++;
    }
    /* walk to ring */
    for(var r=0;r<intro.riders.length;r++){
      var mn=intro.riders[r];
      if(mn._introHidden) continue;
      if(mn._ringX===undefined) continue;
      var dx=mn._ringX-mn.x, dz=mn._ringZ-mn.z;
      var d=Math.hypot(dx,dz);
      if(d>.3){
        var mv=Math.min(d,3.4*1/60);
        mn.x+=dx/d*mv; mn.z+=dz/d*mv;
        mn.walkPh+=.3;
        mn.faceYaw=Math.atan2(dx,dz);
        mn.y=heightAt(mn.x+HALF,mn.z+HALF);
        charPose(mn,Math.sin(mn.walkPh)*.5,Math.sin(mn.walkPh+Math.PI)*.5);
      }
    }
    intro.orbitA+=dt*.15;
    var oR=16;
    camera.position.set(intro.lz.x+Math.cos(intro.orbitA)*oR,
      heightAt(intro.lz.x+HALF,intro.lz.z+HALF)+5,
      intro.lz.z+Math.sin(intro.orbitA)*oR);
    camera.lookAt(intro.lz.x,heightAt(intro.lz.x+HALF,intro.lz.z+HALF)+2,intro.lz.z);
    if(intro.uT>intro.riders.length*.17+2.4) endIntro();
  }
}
function endIntro(){
  if(!intro) return;
  for(var i=0;i<intro.ships.length;i++) scene.remove(intro.ships[i].g);
  stopWash();
  stopIntroMusic();
  clearLatchedKeys();
  /* place any skipped riders on the ring */
  for(var r=0;r<intro.riders.length;r++){
    var mn=intro.riders[r];
    mn._introHidden=false;
    if(mn._ringX===undefined){
      var a=r/intro.riders.length*TAU;
      mn.x=intro.lz.x+Math.cos(a)*7;
      mn.z=intro.lz.z+Math.sin(a)*7;
      mn.y=heightAt(mn.x+HALF,mn.z+HALF);
    }
    /* safety net: anyone outside map bounds */
    mn.x=clamp(mn.x,-HALF+4,HALF-4);
    mn.z=clamp(mn.z,-HALF+4,HALF-4);
  }
  /* LT at the LZ facing north */
  var spot=findClearSpot(intro.lz.x,intro.lz.z,4,1.75,.4);
  if(spot){ P.pos.x=spot.x;P.pos.y=spot.y;P.pos.z=spot.z; }
  P.yaw=Math.PI;
  P.spawnProtT=5;
  el('lbTop').style.opacity=0;
  el('lbBot').style.opacity=0;
  el('capbar').textContent='';
  intro=null;
  started=true;
  startMusic();
  banner('DEPLOYED \u2014 CHOKE THE SPIGOTS \u00b7 DESTROY THEIR COMMAND POST');
}
function skipIntro(){
  if(!intro) return;
  if(intro.phase!=='unload'){
    for(var i=0;i<intro.riders.length;i++) intro.riders[i]._ringX=undefined;
    endIntro();
  } else endIntro();
}
