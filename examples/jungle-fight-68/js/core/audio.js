'use strict';
/* SECTION 16 — AUDIO */
var AC=null, masterGain, musicBus, sfxBus, noiseBuf;
var rainNode=null, windNode=null, radioNode=null;
var _distShots=0;
var _lastCrack=0;

function initAudio(){
  if(AC) return;
  var Ctx=window.AudioContext||window.webkitAudioContext;
  if(!Ctx) return;
  AC=new Ctx();
  var comp=AC.createDynamicsCompressor();
  comp.threshold.value=-18; comp.ratio.value=6;
  comp.connect(AC.destination);
  masterGain=AC.createGain();
  masterGain.gain.value=.55*SET.vol;
  masterGain.connect(comp);
  musicBus=AC.createGain(); musicBus.gain.value=.30;
  musicBus.connect(masterGain);
  sfxBus=AC.createGain(); sfxBus.gain.value=1;
  sfxBus.connect(masterGain);
  var len=AC.sampleRate*2;
  noiseBuf=AC.createBuffer(1,len,AC.sampleRate);
  var d=noiseBuf.getChannelData(0);
  for(var i=0;i<len;i++) d[i]=Math.random()*2-1;
  makeAmbienceLoops();
}
function setVol(v){ if(masterGain) masterGain.gain.value=.55*v; }

function makeAmbienceLoops(){
  if(!AC) return;
  /* rain: noise loop lowpassed 1200 */
  var rs=AC.createBufferSource(); rs.buffer=noiseBuf; rs.loop=true;
  var rf=AC.createBiquadFilter(); rf.type='lowpass'; rf.frequency.value=1200;
  var rg=AC.createGain(); rg.gain.value=0;
  rs.connect(rf); rf.connect(rg); rg.connect(sfxBus); rs.start();
  rainNode={src:rs,gain:rg};
  /* wind: noise loop bandpassed 400 Q .4 at .012 */
  var ws=AC.createBufferSource(); ws.buffer=noiseBuf; ws.loop=true;
  var wf=AC.createBiquadFilter(); wf.type='bandpass'; wf.frequency.value=400; wf.Q.value=.4;
  var wg=AC.createGain(); wg.gain.value=.012;
  ws.connect(wf); wf.connect(wg); wg.connect(sfxBus); ws.start();
  windNode={src:ws,gain:wg};
  /* radio static: bandpassed 2600 */
  var ss=AC.createBufferSource(); ss.buffer=noiseBuf; ss.loop=true;
  var sf=AC.createBiquadFilter(); sf.type='bandpass'; sf.frequency.value=2600;
  var sg=AC.createGain(); sg.gain.value=0;
  ss.connect(sf); sf.connect(sg); sg.connect(sfxBus); ss.start();
  radioNode={src:ss,gain:sg};
}
function setRainVol(v){ if(rainNode) rainNode.gain.gain.value=v; }
function setWindVol(v){ if(windNode) windNode.gain.value=v; }
function setRadioStatic(v){ if(radioNode) radioNode.gain.gain.value=v; }

/* ---------- helpers ---------- */
function tone(opt){
  if(!AC) return;
  var t0=AC.currentTime+(opt.at||0);
  var dur=opt.dur||.2;
  var osc=AC.createOscillator();
  osc.type=opt.type||'sine';
  osc.frequency.setValueAtTime(opt.f,t0);
  if(opt.fB){
    if(opt.exp) osc.frequency.exponentialRampToValueAtTime(Math.max(1,opt.fB),t0+dur);
    else osc.frequency.linearRampToValueAtTime(Math.max(1,opt.fB),t0+dur);
  }
  var g=AC.createGain();
  g.gain.setValueAtTime(0,t0);
  g.gain.linearRampToValueAtTime(opt.vol||.3,t0+(opt.a||.005));
  g.gain.exponentialRampToValueAtTime(.0001,t0+dur);
  var dst=opt.dest||sfxBus;
  osc.connect(g); g.connect(dst);
  osc.start(t0); osc.stop(t0+dur+.05);
}
function noiseHit(opt){
  if(!AC) return;
  var t0=AC.currentTime+(opt.at||0);
  var dur=opt.dur||.2;
  var src=AC.createBufferSource(); src.buffer=noiseBuf; src.loop=true;
  var node=src;
  if(opt.lp){
    var f=AC.createBiquadFilter(); f.type='lowpass'; f.frequency.value=opt.lp;
    if(opt.lp2) f.frequency.exponentialRampToValueAtTime(Math.max(20,opt.lp2),t0+dur);
    node.connect(f); node=f;
  } else if(opt.bp){
    var fB=AC.createBiquadFilter(); fB.type='bandpass'; fB.frequency.value=opt.bp;
    if(opt.bp2) fB.frequency.exponentialRampToValueAtTime(Math.max(20,opt.bp2),t0+dur);
    if(opt.q) fB.Q.value=opt.q;
    node.connect(fB); node=fB;
  } else if(opt.hp){
    var fH=AC.createBiquadFilter(); fH.type='highpass'; fH.frequency.value=opt.hp;
    node.connect(fH); node=fH;
  }
  var g=AC.createGain();
  g.gain.setValueAtTime(0,t0);
  g.gain.linearRampToValueAtTime(opt.vol||.3,t0+(opt.a||.005));
  g.gain.exponentialRampToValueAtTime(.0001,t0+dur);
  node.connect(g); g.connect(opt.dest||sfxBus);
  src.start(t0); src.stop(t0+dur+.05);
}
function spat(pos,maxDist){
  if(!AC||!pos) return {vol:0,pan:0};
  var dx=pos.x-camera.position.x, dz=pos.z-camera.position.z;
  var d=Math.sqrt(dx*dx+dz*dz);
  if(d>maxDist) return {vol:0,pan:0};
  var vol=1-d/maxDist;
  vol=vol*vol;
  var yaw=camera.rotation.y;
  var rx=dx*Math.cos(-yaw)-dz*Math.sin(-yaw);
  var pan=clamp(rx/(d+1),-1,1)*.8;
  return {vol:vol,pan:pan};
}
function panDest(vol,pan){
  var g=AC.createGain(); g.gain.value=vol;
  if(AC.createStereoPanner){
    var p=AC.createStereoPanner(); p.pan.value=clamp(pan,-1,1);
    g.connect(p); p.connect(sfxBus);
  } else g.connect(sfxBus);
  return g;
}

/* ---------- weapon sounds ---------- */
var GUN_SND={
  revolver:{dur:.34,lp:1600,lp2:180,vol:.9,boom:.9,crack:1.2},
  pistol:{dur:.16,lp:2600,lp2:300,vol:.7,boom:.35,crack:1},
  garand:{dur:.22,lp:3000,lp2:250,vol:.85,boom:.5,crack:1.1},
  bolt:{dur:.30,lp:2200,lp2:200,vol:.95,boom:.7,crack:1.15},
  sniper:{dur:.28,lp:2400,lp2:220,vol:.95,boom:.65,crack:1.15},
  smg:{dur:.11,lp:2800,lp2:400,vol:.6,boom:.22,crack:.9},
  lmg:{dur:.15,lp:1900,lp2:260,vol:.8,boom:.5,crack:1},
  ak:{dur:.13,lp:2200,lp2:300,vol:.65,boom:.3,crack:1}
};
function gunSound(kind){
  if(!AC) return;
  var s=GUN_SND[kind]||GUN_SND.garand;
  noiseHit({dur:s.dur,lp:s.lp,lp2:s.lp2,vol:s.vol});
  tone({type:'square',f:1600*s.crack,fB:150,dur:.03,vol:.28});
  tone({type:'sine',f:85,fB:40,dur:.12,vol:s.boom*.5,exp:true});
  noiseHit({at:.03,dur:s.dur*.7,lp:800,lp2:120,vol:s.vol*.35});
}
function gunSoundPos(kind,pos){
  if(!AC) return;
  var sv=spat(pos,260);
  if(sv.vol<=0) return;
  if(_distShots>=10) return;
  _distShots++;
  var s=GUN_SND[kind]||GUN_SND.ak;
  var dest=panDest(sv.vol*s.vol,sv.pan);
  var lp=clamp(500*sv.vol+120,150,700);
  noiseHit({dur:s.dur*2.2,lp:lp,vol:1,dest:dest});
  tone({type:'sine',f:60,fB:30,dur:.12,vol:.5,exp:true,dest:dest});
}

/* ---------- named effects ---------- */
function sCrack(pos){
  if(!AC) return;
  var now=performance.now();
  if(now-_lastCrack<55) return;
  _lastCrack=now;
  var sv=spat(pos,40);
  if(sv.vol<=0) return;
  var d=panDest(sv.vol,sv.pan);
  noiseHit({dur:.05,hp:2200,vol:.5,dest:d});
  noiseHit({dur:.1,bp:rand(3400,5200),bp2:800,vol:.3,dest:d});
  tone({type:'sine',f:170,fB:60,dur:.1,vol:.35,exp:true,dest:d});
}
function sKillConfirm(){
  tone({type:'square',f:340,fB:200,dur:.09,vol:.2});
  tone({type:'square',f:1300,dur:.05,vol:.12,at:.07});
}
function sExplosion(pos){
  if(!AC) return;
  var sv=pos?spat(pos,260*1.4):{vol:1,pan:0};
  if(sv.vol<=0) return;
  var d=pos?panDest(sv.vol,sv.pan):sfxBus;
  tone({type:'sine',f:36,fB:24,dur:1.1,vol:.9,exp:true,dest:d});
  tone({type:'sine',f:72,fB:17,dur:1.5,vol:.6,exp:true,dest:d});
  noiseHit({dur:1.2,lp:400,lp2:50,vol:.8,dest:d});
  noiseHit({at:.05,dur:.12,bp:900,vol:.5,dest:d});
  for(var i=0;i<8;i++){
    tone({type:'square',f:rand(120,600),dur:.03,vol:.1,at:rand(0,1.2),dest:d});
  }
}
function sCollapse(pos){
  if(!AC) return;
  var sv=pos?spat(pos,300):{vol:1,pan:0};
  if(sv.vol<=0) return;
  var d=pos?panDest(sv.vol,sv.pan):sfxBus;
  tone({type:'sine',f:34,fB:15,dur:2.4,vol:.8,exp:true,dest:d});
  noiseHit({dur:2.2,lp:520,lp2:55,vol:.7,dest:d});
  for(var i=0;i<9;i++) tone({type:'square',f:rand(80,500),dur:.04,vol:.14,at:rand(0,2),dest:d});
}
function sHeartbeat(){
  tone({type:'sine',f:54,dur:.12,vol:.5});
  tone({type:'sine',f:46,dur:.14,vol:.45,at:.16});
}
function sMedicCall(kind){
  if(!AC) return;
  var base=rand(165,235);
  var syl=kind==='mandown'?3:2;
  for(var i=0;i<syl;i++){
    var f=base*(i===1?(kind==='mandown'?.82:1.38):1)*(i===2?.7:1);
    var dur=i===2?.5:.16;
    var t0=AC.currentTime+i*.22;
    var osc=AC.createOscillator();
    osc.type='sawtooth'; osc.frequency.setValueAtTime(f,t0);
    osc.frequency.linearRampToValueAtTime(f*(i===2?.85:1.02),t0+dur);
    var fa=AC.createBiquadFilter(); fa.type='bandpass'; fa.frequency.value=750; fa.Q.value=2;
    var fb=AC.createBiquadFilter(); fb.type='bandpass'; fb.frequency.value=1700; fb.Q.value=3;
    var g=AC.createGain();
    g.gain.setValueAtTime(0,t0);
    g.gain.linearRampToValueAtTime(.16,t0+.03);
    g.gain.exponentialRampToValueAtTime(.001,t0+dur);
    osc.connect(fa); fa.connect(fb); fb.connect(g); g.connect(sfxBus);
    osc.start(t0); osc.stop(t0+dur+.05);
  }
}
function sPing(){
  tone({type:'triangle',f:2450,dur:.18,vol:.22});
  tone({type:'sine',f:3620,dur:.14,vol:.12});
}
function sWhoosh(){ noiseHit({dur:.18,bp:900,bp2:300,vol:.2}); }
function sThud(pos){
  var sv=pos?spat(pos,60):{vol:1,pan:0};
  var d=pos?panDest(sv.vol*.7,sv.pan):sfxBus;
  tone({type:'sine',f:110,fB:50,dur:.12,vol:.5,exp:true,dest:d});
  noiseHit({dur:.08,lp:400,vol:.3,dest:d});
}
function sMortarFire(pos){
  var sv=pos?spat(pos,220):{vol:1,pan:0};
  if(sv.vol<=0) return;
  var d=panDest(sv.vol,sv.pan);
  tone({type:'square',f:900,dur:.03,vol:.25,dest:d});
  tone({type:'square',f:700,dur:.03,vol:.22,at:.07,dest:d});
  tone({type:'sine',f:120,fB:60,dur:.3,vol:.6,exp:true,at:.12,dest:d});
  noiseHit({at:.12,dur:.2,lp:600,vol:.4,dest:d});
}
function sIncoming(){
  tone({type:'sawtooth',f:1900,fB:320,dur:1.15,vol:.14,exp:true});
}
function sClick(){ tone({type:'square',f:1200,dur:.03,vol:.12}); }
function sReload(dur){
  for(var i=0;i<3;i++) tone({type:'square',f:rand(500,800),dur:.03,vol:.12,at:dur*(.25+i*.25)});
}
function sStep(splash){
  if(splash) noiseHit({dur:.12,bp:1200,bp2:400,vol:.14});
  else noiseHit({dur:.05,lp:500,vol:.09});
}
function sSplash(){ noiseHit({dur:.25,bp:900,bp2:300,vol:.3}); }
function sGrunt(){
  var f=rand(120,180);
  tone({type:'sawtooth',f:f,fB:f*.7,dur:.15,vol:.16});
}
function sThunder(){
  noiseHit({dur:2.2,lp:300,lp2:40,vol:.6});
  tone({type:'sine',f:40,fB:22,dur:2.2,vol:.4,exp:true});
}
function sChirp(){
  var f=rand(2200,4200);
  for(var i=0;i<irand(2,4);i++) tone({type:'sine',f:f+i*200,fB:f*.8,dur:.07,vol:.05,at:i*.09});
}
function sCricket(){
  for(var i=0;i<3;i++) tone({type:'square',f:4200,dur:.02,vol:.035,at:i*.06});
}
function sPickup(){ tone({type:'sine',f:600,fB:900,dur:.12,vol:.18}); }
function sHeal(){ tone({type:'sine',f:500,fB:800,dur:.3,vol:.15}); }
function sRadio(){
  tone({type:'square',f:1500,dur:.02,vol:.1});
  tone({type:'square',f:900,dur:.02,vol:.1,at:.05});
  tone({type:'sine',f:950,dur:.06,vol:.1,at:.1});
}
function sFlyby(kind,pos){
  if(!AC||!pos) return;
  var sv=spat(pos,400);
  if(sv.vol<=0) return;
  if(kind==='heli'){
    var d=panDest(sv.vol*.5,sv.pan);
    var t0=AC.currentTime, dur=15;
    var osc=AC.createOscillator(); osc.type='square'; osc.frequency.value=52;
    var lfo=AC.createOscillator(); lfo.frequency.value=10.5;
    var lg=AC.createGain(); lg.gain.value=.5;
    var g=AC.createGain(); g.gain.value=.0001;
    lfo.connect(lg); lg.connect(g.gain);
    var f=AC.createBiquadFilter(); f.type='lowpass'; f.frequency.value=420;
    osc.connect(f); f.connect(g); g.connect(d);
    osc.start(t0); lfo.start(t0);
    g.gain.setValueAtTime(.0001,t0);
    g.gain.linearRampToValueAtTime(.4,t0+2);
    g.gain.setValueAtTime(.4,t0+dur-3);
    g.gain.linearRampToValueAtTime(.0001,t0+dur);
    osc.stop(t0+dur); lfo.stop(t0+dur);
    noiseHit({dur:dur,lp:200,vol:.12,dest:d});
  } else {
    var d2=panDest(sv.vol*.45,sv.pan);
    tone({type:'sawtooth',f:70,fB:200,dur:4.5,vol:.3,dest:d2});
    tone({type:'sawtooth',f:71.5,fB:202,dur:4.5,vol:.3,dest:d2});
  }
}
function sTankDistant(pos){
  var sv=pos?spat(pos,500):{vol:.3,pan:0};
  if(sv.vol<=0) return;
  var d=panDest(sv.vol,sv.pan);
  tone({type:'sine',f:44,fB:30,dur:2.5,vol:.4,exp:true,dest:d});
  noiseHit({dur:2.5,lp:150,vol:.25,dest:d});
}

/* ---------- radio voices ---------- */
var RADIO_BUSY=false, RADIO_PRIO=0, RADIO_TIME=0;
var VOICES=[];
function pickVoices(){
  VOICES=[];
  if(!window.speechSynthesis) return;
  var want=['Google US English','Google UK English Male','Online Natural','Natural','Aria','Jenny','Zira','Guy'];
  var all=speechSynthesis.getVoices()||[];
  for(var w=0;w<want.length;w++){
    for(var i=0;i<all.length;i++){
      if((all[i].name||'').indexOf(want[w])>=0 && /en/i.test(all[i].lang||''))
        { VOICES.push(all[i]); break; }
    }
    if(VOICES.length) break;
  }
  if(!VOICES.length) for(var j=0;j<all.length;j++) if(/^en/i.test(all[j].lang||'')){ VOICES.push(all[j]); break; }
}
if(window.speechSynthesis) speechSynthesis.onvoiceschanged=pickVoices;

function radioSay(text,opts){
  opts=opts||{};
  var prio=opts.prio||1;
  if(!SET.voice) { synthShout(text); return; }
  if(RADIO_BUSY && prio<RADIO_PRIO) return;
  RADIO_BUSY=true; RADIO_PRIO=prio; RADIO_TIME=time;
  sRadio();
  setRadioStatic(.015);
  if(musicBus) musicBus.gain.value=.07;
  var done=function(){
    RADIO_BUSY=false; RADIO_PRIO=0;
    setRadioStatic(0);
    if(musicBus) musicBus.gain.value=SET.music?.30:0;
  };
  if(window.speechSynthesis && VOICES.length){
    pickVoices();
    var u=new SpeechSynthesisUtterance(text);
    if(VOICES.length) u.voice=VOICES[0];
    u.rate=.95; u.pitch=.9; u.volume=clamp(SET.vol,0,1);
    u.onend=done; u.onerror=done;
    try{ speechSynthesis.cancel(); speechSynthesis.speak(u); }catch(e){ done(); }
    setTimeout(done,9000);
  } else {
    synthShout(text);
    setTimeout(done,Math.min(2600,text.length*55));
  }
}
function synthShout(text){
  if(!AC) return;
  var base=rand(150,210);
  var n=clamp(Math.floor(text.length/6),2,6);
  for(var i=0;i<n;i++){
    var f=base*rand(.9,1.15);
    tone({type:'sawtooth',f:f,fB:f*.85,dur:.14,vol:.08,at:i*.16});
  }
}

/* ---------- soundtrack ---------- */
var musicOn=true, musicTimer=null, musicStep=0, musicBar=0;
var NOTE={E2:82.41,G2:98,A2:110,B2:123.47,C3:130.81,E3:164.81,G3:196,A3:220,B3:246.94,
  C4:261.63,D4:293.66,E4:329.63,G4:392,A4:440,B4:493.88,E5:659.26,D5:587.33,
  F3:174.61,F4:349.23,Bb3:233.08,Bb4:466.16,C5:523.25,D3:146.83};
function mNote(freq,step,dur,type,vol,bus){
  var spb=60/106, st=spb/4;
  tone({type:type,f:freq,dur:dur*st,vol:vol,at:step*st,dest:bus||musicBus,exp:false});
}
function musicTick(){
  if(!AC||!musicOn||!SET.music||!started||paused) return;
  var spb=60/106, st=spb/4;
  var bar=musicBar%8, step=musicStep%16;
  /* kick */
  if(step===0||step===8) tone({type:'sine',f:120,fB:38,dur:.2,vol:.7,at:0,dest:musicBus,exp:true});
  if(bar%2===1&&step===10) tone({type:'sine',f:120,fB:38,dur:.18,vol:.6,at:0,dest:musicBus,exp:true});
  /* snare */
  if(step===4||step===12){
    noiseHit({dur:.1,bp:1800,vol:.35,dest:musicBus});
    tone({type:'triangle',f:190,dur:.05,vol:.2,dest:musicBus});
  }
  /* hats */
  if(step%2===0) noiseHit({dur:.03,hp:6000,vol:.06,dest:musicBus});
  /* shaker from bar 2 */
  if(musicBar>=2&&step%2===1) noiseHit({dur:.04,bp:5200,vol:.05,dest:musicBus});
  /* fill on bar 7 */
  if(bar===7&&step>=12) noiseHit({dur:.06,bp:2000+step*300,vol:.12,dest:musicBus});
  /* bass: E2 E2 G2 G2 A2 A2 C3 B2 */
  var roots=['E2','E2','G2','G2','A2','A2','C3','B2'];
  var root=NOTE[roots[bar]];
  if(step%8===0){
    tone({type:'sawtooth',f:root,dur:st*7,vol:.22,at:0,dest:musicBus});
    tone({type:'sine',f:root/2,dur:st*7,vol:.3,at:0,dest:musicBus});
  }
  if(step===14) tone({type:'sawtooth',f:root*2,dur:st*2,vol:.18,at:0,dest:musicBus});
  /* chord stabs on steps 0 and 8 */
  var chords=[['E3','B3','E4'],['E3','B3','E4'],['G3','D4','G4'],['G3','D4','G4'],
              ['A3','E4','A4'],['A3','E4','A4'],['C4','E4','G4'],['B3','D4','B4']];
  if(step===0||step===8){
    var ch=chords[bar];
    for(var i=0;i<ch.length;i++){
      tone({type:'square',f:NOTE[ch[i]],dur:st*3,vol:.07,at:0,dest:musicBus});
      tone({type:'square',f:NOTE[ch[i]]*2,dur:st*3,vol:.03,at:0,dest:musicBus});
    }
  }
  /* pentatonic lead bars 4-7 */
  var leads=[[null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null],
             [null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null],
             [null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null],
             [null,null,null,null,null,null,null,null,null,null,null,null,null,null,null,null],
             ['E4',null,'G4',null,'A4',null,null,'G4',null,null,'E4',null,null,null,null,null],
             ['D4',null,'E4',null,'G4',null,null,'E4',null,null,null,null,null,null,null,null],
             ['A4',null,'G4',null,'E4',null,'D4',null,'E4',null,null,null,null,null,null,null],
             ['E5',null,'D5',null,'B4',null,'G4',null,'A4',null,'B4',null,null,null,null,null]];
  var L=leads[bar][step];
  if(L){
    tone({type:'square',f:NOTE[L],dur:st*1.6,vol:.09,at:0,dest:musicBus});
    tone({type:'square',f:NOTE[L],dur:st*1.6,vol:.05,at:st*3,dest:musicBus});
  }
  musicStep++;
  if(musicStep%16===0) musicBar++;
}
function startMusic(){
  if(musicTimer||!AC) return;
  var spb=60/106;
  musicTimer=setInterval(musicTick,(spb/4)*1000);
}
function stopMusic(){
  if(musicTimer){ clearInterval(musicTimer); musicTimer=null; }
}
function toggleMusic(){
  musicOn=!musicOn;
  if(musicBus) musicBus.gain.value=musicOn&&SET.music?.30:0;
}

/* ---------- intro ride music (118 BPM in G) ---------- */
var introMusicTimer=null, introBar=0, introStep=0;
function introRiffStep(bar,step){
  /* phrases alternate every 2 bars */
  var phraseA=['G3',null,'G3','Bb3',null,'C4','Bb3','G3'];
  var phraseB=['F3',null,'F3','G3',null,'Bb3','G3','F3'];
  var phrase=(Math.floor(bar/2)%2===0)?phraseA:phraseB;
  var n=phrase[step%8];
  if(n){
    tone({type:'sawtooth',f:NOTE[n]*1.004,dur:.16,vol:.13,at:0,dest:musicBus});
    tone({type:'square',f:NOTE[n]*.997,dur:.16,vol:.08,at:0,dest:musicBus});
  }
  if(step===0||step===6||step===8||step===12) tone({type:'sine',f:120,fB:38,dur:.16,vol:.5,at:0,dest:musicBus,exp:true});
  if(step===4||step===12){ noiseHit({dur:.09,bp:1800,vol:.28,dest:musicBus}); tone({type:'triangle',f:190,dur:.04,vol:.15,dest:musicBus}); }
  if(step%2===0) noiseHit({dur:.025,hp:6000,vol:.05,dest:musicBus});
  if(step===0) tone({type:'sine',f:NOTE.G2,dur:.4,vol:.25,at:0,dest:musicBus});
  if(step===14&&(Math.floor(bar/2)%2===1)) tone({type:'sine',f:NOTE.G2*2,dur:.2,vol:.2,at:0,dest:musicBus});
  if(bar>=4){
    var organs=[['G','D','G'],['C','E','G'],['G','Bb','D']];
    var oc=organs[(bar-4)%3];
    if(step%4===0) for(var i=0;i<oc.length;i++)
      tone({type:'triangle',f:NOTE[oc[i]+(oc[i].length===1?'3':'3')],dur:.3,vol:.05,at:0,dest:musicBus});
  }
}
function introTomPickup(){
  for(var i=0;i<4;i++) tone({type:'sine',f:180-i*20,fB:70,dur:.14,vol:.3,at:i*.19,dest:musicBus,exp:true});
}
function startIntroMusic(){
  if(!AC) return;
  introTomPickup();
  var spb=60/118;
  var startAt=spb*1;
  introBar=0; introStep=0;
  introMusicTimer=setInterval(function(){
    if(!AC) return;
    introRiffStep(introBar,introStep);
    introStep++;
    if(introStep%8===0) introBar++;
  },(spb/2)*1000);
}
function stopIntroMusic(){
  if(introMusicTimer){ clearInterval(introMusicTimer); introMusicTimer=null; }
  if(musicBus){
    var t=AC.currentTime;
    musicBus.gain.setValueAtTime(musicBus.gain.value,t);
    musicBus.gain.linearRampToValueAtTime(.0001,t+.7);
    setTimeout(function(){ if(musicBus) musicBus.gain.value=SET.music?.30:0; },750);
  }
}

/* ---------- rotor wash ---------- */
var washNodes=null;
function startWash(){
  if(!AC||washNodes) return;
  washNodes=[];
  var defs=[{f:12.6,type:'lowpass',cf:240},{f:9.4,type:'lowpass',cf:90},{f:12.6,type:'bandpass',cf:300}];
  for(var i=0;i<3;i++){
    var src=AC.createBufferSource(); src.buffer=noiseBuf; src.loop=true;
    var flt=AC.createBiquadFilter(); flt.type=defs[i].type; flt.frequency.value=defs[i].cf;
    var lfo=AC.createOscillator(); lfo.frequency.value=defs[i].f;
    var lg=AC.createGain(); lg.gain.value=.5;
    var g=AC.createGain(); g.gain.value=.0001;
    lfo.connect(lg); lg.connect(g.gain);
    src.connect(flt); flt.connect(g); g.connect(sfxBus);
    src.start(); lfo.start();
    g.gain.linearRampToValueAtTime(.18/3+i*.04,AC.currentTime+1);
    washNodes.push({src:src,lfo:lfo,g:g});
  }
}
function stopWash(){
  if(!washNodes||!AC) return;
  for(var i=0;i<washNodes.length;i++){
    var w=washNodes[i];
    try{
      w.g.gain.setTargetAtTime(.0001,AC.currentTime,.35);
      w.src.stop(AC.currentTime+2); w.lfo.stop(AC.currentTime+2);
    }catch(e){}
  }
  washNodes=null;
}

/* ---------- ambience scheduler ---------- */
var ambT=0;
function updateAmbience(dt){
  if(!AC||!started||paused) return;
  ambT-=dt;
  if(ambT>0) return;
  ambT=rand(2.5,8);
  var r=Math.random();
  var day=dayFactor!==undefined?dayFactor:1;
  var wet=weather&&(weather.kind==='RAIN'||weather.kind==='STORM');
  if(r<.26){ if(day>.35&&!wet) sChirp(); }
  else if(r<.36){ if(day<.25) sCricket(); }
  else if(r<.5) gunSoundPos('ak',{x:camera.position.x+rand(-200,200),y:20,z:camera.position.z+rand(-200,200)});
  else if(r<.6){
    var n=irand(4,8);
    var px=camera.position.x+rand(-250,250), pz=camera.position.z+rand(-250,250);
    for(var i=0;i<n;i++) setTimeout(function(){ gunSoundPos('lmg',{x:px,y:20,z:pz}); },i*90);
  }
  else if(r<.68) sExplosion({x:camera.position.x+rand(-300,300),y:10,z:camera.position.z+rand(-300,300)});
  else if(r<.78) sFlyby('jet',{x:camera.position.x+rand(-300,300),y:60,z:camera.position.z+rand(-300,300)});
  else if(r<.88) sFlyby('heli',{x:camera.position.x+rand(-200,200),y:50,z:camera.position.z+rand(-200,200)});
  else if(r<.94) sTankDistant({x:camera.position.x+rand(-400,400),y:10,z:camera.position.z+rand(-400,400)});
  else sCricket();
}
