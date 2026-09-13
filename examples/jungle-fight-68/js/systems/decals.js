'use strict';
/* SECTION 12b — DECALS */
var decalChips=null, bloodPools=null;
var chipNext=0, bloodNext=0;
function initDecals(){
  /* impact chips: 72 dark planes */
  var geo=new THREE.PlaneGeometry(.34,.34);
  var mat=new THREE.MeshBasicMaterial({color:0x241c12,transparent:true,opacity:.85,depthWrite:false});
  decalChips=new THREE.InstancedMesh(geo,mat,72);
  decalChips.count=72;
  decalChips.frustumCulled=false;
  for(var i=0;i<72;i++) decalChips.setMatrixAt(i,_hideM);
  scene.add(decalChips);
  /* blood pools: 36 dark red circles */
  var cg=new THREE.CircleGeometry(.42,10);
  var cm=new THREE.MeshBasicMaterial({color:0x4a0808,transparent:true,opacity:.9,depthWrite:false});
  bloodPools=new THREE.InstancedMesh(cg,cm,36);
  bloodPools.count=36;
  bloodPools.frustumCulled=false;
  for(var b=0;b<36;b++) bloodPools.setMatrixAt(b,_hideM);
  scene.add(bloodPools);
}
function impactDecal(x,y,z,p){
  var slot=chipNext=(chipNext+1)%72;
  var dy=p?(p.vy||0):0;
  var steep=p?Math.abs(dy/ (Math.hypot(p.vx,p.vz,p.vy)||1))>.72:false;
  _tmpObj.position.set(x,y+.03,z);
  if(steep){
    _tmpObj.rotation.set(-Math.PI/2,0,rand(0,TAU));
  } else {
    _tmpObj.lookAt(camera.position.x,y,camera.position.z);
    _tmpObj.rotateX(Math.PI/2);
    /* pull slightly back along travel */
    _tmpObj.position.x-= (p?p.vx:0)*.002;
    _tmpObj.position.z-= (p?p.vz:0)*.002;
  }
  var s=rand(.7,1.3);
  _tmpObj.scale.set(s,s,s);
  _tmpObj.updateMatrix();
  decalChips.setMatrixAt(slot,_tmpObj.matrix);
  decalChips.instanceMatrix.needsUpdate=true;
}
function bloodPool(x,z){
  var slot=bloodNext=(bloodNext+1)%36;
  var y=heightAt(x+HALF,z+HALF)+.05+ (slot%36)*.0012;
  _tmpObj.position.set(x,y,z);
  _tmpObj.rotation.set(-Math.PI/2,0,0);
  var s=rand(.9,1.6);
  _tmpObj.scale.set(s,s,s);
  _tmpObj.updateMatrix();
  bloodPools.setMatrixAt(slot,_tmpObj.matrix);
  bloodPools.instanceMatrix.needsUpdate=true;
}
