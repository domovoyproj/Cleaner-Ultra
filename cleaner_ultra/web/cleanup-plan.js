/* Shared review model: excluded items never become API operations. */
(function(root){
  'use strict';
  class CleanupPlan {
    constructor(){this.items=[];this.ids=new Set();}
    add(item){const key=`${item.kind}:${item.id}`;if(this.ids.has(key))return;this.ids.add(key);this.items.push({...item,key,selected:item.selected!==false});}
    select(key,value){const item=this.items.find(r=>r.key===key);if(item)item.selected=!!value;}
    selectAll(value,kind){for(const item of this.items)if(!kind||item.kind===kind)item.selected=!!value;}
    selected(){return this.items.filter(r=>r.selected);}
    summary(){const rows=this.selected();return {count:rows.length,file_count:rows.filter(r=>r.kind==='files').length,file_bytes:rows.filter(r=>r.kind==='files').reduce((n,r)=>n+(r.size||0),0),registry_count:rows.filter(r=>['startup','registry'].includes(r.kind)).length};}
    operations(){
      const rows=this.selected(),operations=[];
      for(const kind of ['files','startup','registry','cache','power','trim']){
        const items=rows.filter(r=>r.kind===kind),size=kind==='files'?500:kind==='startup'||kind==='registry'?50000:1;
        for(let i=0;i<items.length;i+=size)operations.push({kind,items:items.slice(i,i+size)});
      }
      return operations;
    }
  }
  if(typeof module==='object'&&module.exports)module.exports={CleanupPlan};else root.CleanupPlan=CleanupPlan;
})(typeof globalThis==='object'?globalThis:this);
