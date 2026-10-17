const assert=require('node:assert/strict');
const test=require('node:test');
const {layout}=require('../photosort/web/map-layout.js');
for(const [w,h] of [[1000,450],[340,600],[1,1]])test(`map proportions and bounds ${w}x${h}`,()=>{
 const values=[1000000,3000,1500,100,20,1].map((bytes,i)=>({bytes,path:String(i)}));
 const tiles=layout(values,w,h),sum=values.reduce((n,v)=>n+v.bytes,0);
 assert.equal(tiles.length,values.length);
 for(const t of tiles){assert.ok(t.w>0&&t.h>0);assert.ok(t.x>=0&&t.y>=0);assert.ok(t.x+t.w<=w+1e-8&&t.y+t.h<=h+1e-8);assert.ok(Math.abs(t.w*t.h/(w*h)-t.bytes/sum)<1e-10);}
 for(let i=0;i<tiles.length;i++)for(let j=i+1;j<tiles.length;j++){const a=tiles[i],b=tiles[j];const overlap=Math.max(0,Math.min(a.x+a.w,b.x+b.w)-Math.max(a.x,b.x))*Math.max(0,Math.min(a.y+a.h,b.y+b.h)-Math.max(a.y,b.y));assert.ok(overlap<1e-6);}
});
test('empty, zero and invalid sizes do not produce broken blocks',()=>{assert.deepEqual(layout([],100,100),[]);assert.deepEqual(layout([{bytes:0},{bytes:NaN},{bytes:-1}],100,100),[]);assert.deepEqual(layout([{bytes:1}],0,100),[]);});
test('hundreds of equal entries preserve all identities',()=>{const values=Array.from({length:601},(_,i)=>({bytes:1,path:String(i)}));const tiles=layout(values,1200,600);assert.equal(new Set(tiles.map(v=>v.path)).size,601);assert.ok(Math.abs(tiles.reduce((n,v)=>n+v.w*v.h,0)-720000)<1e-6);});
