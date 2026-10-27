const{test}=require('node:test');const assert=require('node:assert/strict');const vm=require('node:vm');const fs=require('node:fs');
function fixture(api){
 const ids=new Map();
 function node(tag,cls='',text=''){const n={tag,className:cls,textContent:text,children:[],attrs:{},dataset:{},style:{setProperty(){}},classList:{toggle(){}},append(...children){for(const c of children){c.parent=this;this.children.push(c);}},replaceChildren(...children){this.children=[];this.append(...children);},setAttribute(k,v){this.attrs[k]=v;},remove(){if(this.parent)this.parent.children=this.parent.children.filter(c=>c!==this);},querySelector(selector){const name=selector.slice(1);return this.children.find(c=>c.className?.split(' ').includes(name))||null;},focus(){this.focused=true;}};return n;}
 const $=id=>{if(!ids.has(id))ids.set(id,node('div'));return ids.get(id);};const window={CleanerUltraIcons:{create:()=>node('svg')}};
 vm.runInNewContext(fs.readFileSync(require.resolve('../cleaner_ultra/web/disk-tree.js'),'utf8'),{window,node,$,api,bytes:n=>`${n} B`,num:String,basename:p=>p.split('/').pop(),selectMap(){},action:async fn=>fn(),URLSearchParams});
 return {window,$,node};
}
test('folder tree loads branches lazily, collapses without losing siblings, and never treats files as folders',async()=>{
 let calls=0;const f=fixture(async url=>{calls++;assert.ok(url.includes('path=%2Froot%2Fa'));return {scan:7,entries:[{kind:'file',path:'/root/a/file.txt',bytes:5}],offset:0,total:1};});
 await f.window.CleanerUltraTree.load({scan:7,path:'/root',bytes:10,files:2,total:2,entries:[{kind:'folder',path:'/root/a',bytes:5},{kind:'file',path:'/root/direct.txt',bytes:5}]});
 assert.equal(calls,0);const root=f.$('#disk-tree').children[0],group=root.children[1],folder=group.children[0],file=group.children[1];assert.equal(file.children[0].children[0].disabled,true);
 await folder.children[0].children[0].onclick();assert.equal(calls,1);assert.equal(folder.attrs['aria-expanded'],'true');assert.equal(folder.children[1].children.length,1);assert.equal(group.children.length,2);
 await folder.children[0].children[0].onclick();assert.equal(folder.children[1].hidden,true);await folder.children[0].children[0].onclick();assert.equal(calls,1);assert.equal(folder.children[1].hidden,false);
});
test('large folder pagination preserves all items rather than grouping small files into tiles',async()=>{
 const all=Array.from({length:130},(_,i)=>({kind:'file',path:`/root/${i}.bin`,bytes:1}));const f=fixture(async url=>{const q=new URL(url,'http://localhost').searchParams,offset=Number(q.get('offset'));return {scan:8,entries:all.slice(offset,offset+60),offset,total:130};});
 await f.window.CleanerUltraTree.load({scan:8,path:'/root',bytes:130,total:130,entries:all.slice(0,60)});const group=f.$('#disk-tree').children[0].children[1];assert.equal(group.children.length,61);
 await group.children.at(-1).onclick();assert.equal(group.children.length,121);await group.children.at(-1).onclick();assert.equal(group.children.length,130);assert.equal(new Set(group.children.map(c=>c.dataset.path)).size,130);
});
test('folder expansion rejects results from another disk generation',async()=>{
 const f=fixture(async()=>({scan:99,entries:[],offset:0,total:0}));await f.window.CleanerUltraTree.load({scan:1,path:'/root',bytes:1,total:1,entries:[{kind:'folder',path:'/root/a',bytes:1}]});const folder=f.$('#disk-tree').children[0].children[1].children[0];await assert.rejects(folder.children[0].children[0].onclick(),/Снимок диска изменился/);assert.equal(folder.children[1].children.length,0);
});
