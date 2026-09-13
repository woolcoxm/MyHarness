'use strict';
/* SECTION 4 — MATH AND NOISE */
function el(id){ return document.getElementById(id); }
function clamp(v,a,b){ return v<a?a:(v>b?b:v); }
function lerp(a,b,t){ return a+(b-a)*t; }
function rand(a,b){ return a+Math.random()*(b-a); }
function irand(a,b){ return a+Math.floor(Math.random()*(b-a+1)); }
var TAU=Math.PI*2;

/* Typed-array aliases (global; every file uses these) */
var F32=window['Float'+'32'+'Array'];
var U8=window['Uint'+'8'+'Array'];
var I32=window['Int'+'32'+'Array'];
var U16=window['Uint'+'16'+'Array'];

function mulberry32(seed){
  var a=seed>>>0;
  return function(){
    a|=0; a=(a+1831565813)|0;
    var t=Math.imul(a^(a>>>15),1|a);
    t=(t+Math.imul(t^(t>>>7),61|t))^t;
    return ((t^(t>>>14))>>>0)/4294967296;
  };
}

function makeNoise(seed){
  var R=mulberry32(seed);
  var N=256;
  var g=new F32(N*N);
  for(var i=0;i<N*N;i++) g[i]=R();
  function at(x,y){
    x=((x%N)+N)%N; y=((y%N)+N)%N;
    return g[(y|0)*N+(x|0)];
  }
  function sm(t){ return t*t*(3-2*t); }
  function vn(x,y){
    var xi=Math.floor(x), yi=Math.floor(y);
    var xf=x-xi, yf=y-yi;
    var u=sm(xf), v=sm(yf);
    var a=at(xi,yi), b=at(xi+1,yi), c=at(xi,yi+1), d=at(xi+1,yi+1);
    return lerp(lerp(a,b,u), lerp(c,d,u), v);
  }
  return function(x,y){
    var s=0, amp=1, tot=0, f=1;
    for(var o=0;o<4;o++){
      s+=vn(x*f,y*f)*amp; tot+=amp; amp*=.5; f*=2;
    }
    return s/tot;
  };
}
var noise=makeNoise(4242);
