(() => {
  "use strict";
  function layout(entries, width, height) {
    const values=entries.filter(v=>Number.isFinite(v.bytes)&&v.bytes>0);
    const total=values.reduce((n,v)=>n+v.bytes,0), result=[];
    if(!total||!(width>0)||!(height>0))return result;
    function divide(items,x,y,w,h,sum) {
      if(items.length===1){result.push({...items[0],x,y,w,h});return;}
      let split=1, part=items[0].bytes;
      while(split<items.length-1&&Math.abs(part+items[split].bytes-sum/2)<Math.abs(part-sum/2)){part+=items[split++].bytes;}
      const ratio=part/sum;
      if(w>=h){divide(items.slice(0,split),x,y,w*ratio,h,part);divide(items.slice(split),x+w*ratio,y,w*(1-ratio),h,sum-part);}
      else {divide(items.slice(0,split),x,y,w,h*ratio,part);divide(items.slice(split),x,y+h*ratio,w,h*(1-ratio),sum-part);}
    }
    divide(values,0,0,width,height,total);return result;
  }
  const api={layout};if(typeof module!=="undefined"&&module.exports)module.exports=api;else globalThis.CleanerUltraMap=api;
})();
