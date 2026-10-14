// Dev-only stand-in for the nix-built highs.js: wraps the npm `highs` package's emscripten
// module and installs the same self.Highs_* globals as flake.nix's post.js.
import createModule from './highs.mjs';
export default async function init(options = {}) {
  const Module = await createModule({ locateFile: (f) => '/' + f, ...options });
  // Renamed in newer HiGHS releases.
  Module._Highs_getNumCols ??= Module._Highs_getNumCol;
  Module._Highs_getNumRows ??= Module._Highs_getNumRow;

              self.Highs_call = Module._Highs_call;
              self.Highs_changeObjectiveSense = Module._Highs_changeObjectiveSense;
              self.Highs_create = Module._Highs_create;
              self.Highs_destroy = Module._Highs_destroy;
              self.Highs_getModelStatus = Module._Highs_getModelStatus;
              self.Highs_getNumCols = Module._Highs_getNumCols;
              self.Highs_getNumRows = Module._Highs_getNumRows;
              self.Highs_getSolution = Module._Highs_getSolution;
              self.Highs_run = Module._Highs_run;
              self.Highs_setBoolOptionValue = Module.cwrap("Highs_setBoolOptionValue","number",["number", "string", "number"]);
              self.Highs_setDoubleOptionValue = Module.cwrap("Highs_setDoubleOptionValue","number",["number", "string", "number"]);
              self.Highs_setIntOptionValue = Module.cwrap("Highs_setIntOptionValue","number",["number", "string", "number"]);
              self.Highs_setStringOptionValue = Module.cwrap("Highs_setIntOptionValue","number",["number", "string", "number"]);

              self.Highs_passLp = function (h,nc,nr,nnz,a_fmt,sense,off,ccost,clow,cup,rlow,rup,astart,aidx,aval) {
                var pccost = Module._malloc(ccost.length*ccost.BYTES_PER_ELEMENT); Module.HEAPU8.set(ccost, pccost);
                var pclow = Module._malloc(clow.length*clow.BYTES_PER_ELEMENT); Module.HEAPU8.set(clow, pclow);
                var pcup = Module._malloc(cup.length*cup.BYTES_PER_ELEMENT); Module.HEAPU8.set(cup, pcup);
                var prlow = Module._malloc(rlow.length*rlow.BYTES_PER_ELEMENT); Module.HEAPU8.set(rlow, prlow);
                var prup = Module._malloc(rup.length*rup.BYTES_PER_ELEMENT); Module.HEAPU8.set(rup, prup);
                var pastart = Module._malloc(astart.length*astart.BYTES_PER_ELEMENT); Module.HEAPU8.set(astart, pastart);
                var paidx = Module._malloc(aidx.length*aidx.BYTES_PER_ELEMENT); Module.HEAPU8.set(aidx, paidx);
                var paval = Module._malloc(aval.length*aval.BYTES_PER_ELEMENT); Module.HEAPU8.set(aval, paval);
                let ret = Module._Highs_passLp(h,nc,nr,nnz,a_fmt,sense,off,pccost,pclow,pcup,prlow,prup,pastart,paidx,paval);
                Module._free(pccost); Module._free(pclow); Module._free(pcup); Module._free(prlow);
                Module._free(prup); Module._free(pastart); Module._free(paidx); Module._free(paval);
                return ret;
              }
              self.Highs_passMip = function (h,nc,nr,nnz,a_fmt,sense,off,ccost,clow,cup,rlow,rup,astart,aidx,aval,integ) {
                var pccost = Module._malloc(ccost.length*ccost.BYTES_PER_ELEMENT); Module.HEAPU8.set(ccost, pccost);
                var pclow = Module._malloc(clow.length*clow.BYTES_PER_ELEMENT); Module.HEAPU8.set(clow, pclow);
                var pcup = Module._malloc(cup.length*cup.BYTES_PER_ELEMENT); Module.HEAPU8.set(cup, pcup);
                var prlow = Module._malloc(rlow.length*rlow.BYTES_PER_ELEMENT); Module.HEAPU8.set(rlow, prlow);
                var prup = Module._malloc(rup.length*rup.BYTES_PER_ELEMENT); Module.HEAPU8.set(rup, prup);
                var pastart = Module._malloc(astart.length*astart.BYTES_PER_ELEMENT); Module.HEAPU8.set(astart, pastart);
                var paidx = Module._malloc(aidx.length*aidx.BYTES_PER_ELEMENT); Module.HEAPU8.set(aidx, paidx);
                var paval = Module._malloc(aval.length*aval.BYTES_PER_ELEMENT); Module.HEAPU8.set(aval, paval);
                var pinteg = Module._malloc(integ.length*integ.BYTES_PER_ELEMENT); Module.HEAPU8.set(integ, pinteg);
                let ret = Module._Highs_passMip(h,nc,nr,nnz,a_fmt,sense,off,pccost,pclow,pcup,prlow,prup,pastart,paidx,paval,pinteg);
                Module._free(pccost); Module._free(pclow); Module._free(pcup); Module._free(prlow);
                Module._free(prup); Module._free(pastart); Module._free(paidx); Module._free(paval); Module._free(pinteg);
                return ret;
              }

              self.Highs_getSolution = function(h,c,r) {
                let ptr0=Module._malloc(c+8);let ptr1=Module._malloc(c+8);let ptr2=Module._malloc(r+8);let ptr3=Module._malloc(r+8);
                let ret=Module._Highs_getSolution(h,ptr0+8,ptr1+8,ptr2+8,ptr3+8);
                let cv=new Uint8Array(Module.HEAPU8.buffer,ptr0+8,c);
                let cd=new Uint8Array(Module.HEAPU8.buffer,ptr1+8,c);
                let rv=new Uint8Array(Module.HEAPU8.buffer,ptr2+8,r);
                let rd=new Uint8Array(Module.HEAPU8.buffer,ptr3+8,r);
                Module._free(ptr0);Module._free(ptr1);Module._free(ptr2);Module._free(ptr3);
                return {"ret": ret, "cv": cv, "cd": cd, "rv": rv, "rd": rd};
              };
            
}
