const {test}=require('node:test');const assert=require('node:assert/strict');const vm=require('node:vm');const fs=require('node:fs');const {CleanupPlan}=require('../cleaner_ultra/web/cleanup-plan.js');
function harness({partial=false}={}){
 const elements=new Map(),calls=[];let pending,dialogOpen=true;
 function node(tag,cls='',text=''){return {tag,className:cls,textContent:text,children:[],value:'',dataset:{},attrs:{},append(...items){this.children.push(...items);},replaceChildren(...items){this.children=items;},setAttribute(k,v){this.attrs[k]=v;},close(){dialogOpen=false;},click(){}};}
 const $=id=>{if(!elements.has(id))elements.set(id,node('div'));return elements.get(id);};
 const modes=['files','startup','registry'].map(mode=>({dataset:{wizardMode:mode}}));
 const stages=['modes','scan','review','apply','done'].map(name=>({dataset:{wizardStage:name}}));
 const $$=selector=>selector==='[data-wizard-mode]:checked'?modes:selector==='[data-cleanup-rule]:checked'?[{dataset:{cleanupRule:'own-rule',cleanupCategory:'user_temp'}}]:selector==='[data-wizard-stage]'?stages:[];
 $('#wizard-path').value='C:\\own-temp';
 const api=async(path,body)=>{
  calls.push({path,body});
  if(path==='/api/cleanup/scan')return {job:'own-job'};if(path==='/api/cleanup/progress')return {job:'own-job',state:'complete',files:2,rules_done:1,rules_total:1};
  if(path.startsWith('/api/cleanup/files?'))return {job:'own-job',total:2,files:[{id:1,path:'C:\\own-temp\\keep.tmp',size:10},{id:2,path:'C:\\own-temp\\remove.tmp',size:20}]};
  if(path==='/api/startup/scan')return {items:[{id:'own-startup',name:'own startup',key:'HKCU\\own',command:'own.exe',removable:true},{id:'present',name:'existing',removable:false}]};
  if(path==='/api/registry/scan')return {items:[{id:'own-registry',name:'own registry',key:'HKCU\\own',command:'own.dll',removable:true}]};
  assert.equal(dialogOpen,false,'confirmation must close before lengthy mutations');
  if(path==='/api/cleanup/apply')return partial?{done:1,moved_bytes:10,freed_bytes:0,errors:[{id:2,message:'file changed'}]}:{done:body.ids.length,moved_bytes:20,freed_bytes:0,errors:[]};
  if(path==='/api/registry/apply'&&partial)throw Error('snapshot changed');
  return {done:body.ids.length,errors:[]};
 };
 const window={CleanerUltraMaintenance:{confirm(title,copy,items,run){pending={title,copy,items,run};}}};
 vm.runInNewContext(fs.readFileSync(require.resolve('../cleaner_ultra/web/wizard.js'),'utf8'),{$,$$,node,api,window,CleanupPlan,action:async fn=>fn(),num:String,bytes:n=>`${n||0} bytes`,basename:p=>p.split('\\').pop(),toast:()=>{},refresh:async()=>{},setTimeout,Date,Blob,URL,document:{createTextNode:s=>s}});
 return {$,calls,stages,confirm:()=>pending};
}
test('wizard scans without writes, then applies only the edited selection after one confirmation',async()=>{
 const h=harness();await h.$('#wizard-scan').onclick();assert.equal(h.calls.filter(c=>c.path.includes('apply')||c.path.includes('trash')).length,0);
 h.$('#wizard-select-none').onclick();assert.equal(h.$('#wizard-apply').disabled,true);
 const row=h.$('#wizard-list').children.find(r=>r.children[0]?.attrs['aria-label']==='Включить remove.tmp');row.children[0].checked=true;row.children[0].onchange();
 await h.$('#wizard-apply').onclick();assert.equal(h.confirm().items.length,1);assert.equal(h.calls.filter(c=>c.path==='/api/cleanup/apply').length,0);
 const result=await h.confirm().run();assert.equal(result.done,1);assert.deepEqual(JSON.parse(JSON.stringify(h.calls.find(c=>c.path==='/api/cleanup/apply').body.ids)),[2]);assert.ok(!h.calls.some(c=>['/api/registry/apply','/api/startup/apply'].includes(c.path)));assert.equal(h.stages.find(s=>s.dataset.wizardStage==='done').hidden,false);
});
test('wizard filtered select-all excludes hidden rows and partial failures remain visible',async()=>{
 const h=harness({partial:true});await h.$('#wizard-scan').onclick();h.$('#wizard-select-none').onclick();h.$('#wizard-search').value='registry';h.$('#wizard-search').oninput();h.$('#wizard-select-all').onclick();await h.$('#wizard-apply').onclick();assert.equal(h.confirm().items.length,1);
 h.$('#wizard-search').value='';h.$('#wizard-search').oninput();h.$('#wizard-select-all').onclick();await h.$('#wizard-apply').onclick();const result=await h.confirm().run();assert.equal(result.done,2);assert.equal(result.errors.length,2);assert.equal(result.freed_bytes,0);
 h.$('#wizard-review-again').onclick();const rows=h.$('#wizard-list').children;const input=name=>rows.find(r=>r.children[0]?.attrs['aria-label']===`Включить ${name}`).children[0];assert.equal(input('keep.tmp').checked,false);assert.equal(input('remove.tmp').checked,true);assert.equal(input('own registry').checked,true);
});
