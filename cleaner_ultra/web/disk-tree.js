/* Lazy, paginated folder tree. Expanding never allocates the complete disk in the DOM. */
(()=>{
  const tree={scan:0,generation:0,nodes:new Map(),root:null};
  function row(item,level,parent){
    const wrapper=node('div','disk-tree-item');wrapper.setAttribute('role','treeitem');wrapper.setAttribute('aria-level',String(level));wrapper.dataset.path=item.path;
    const line=node('div','disk-tree-line'),expand=node('button','tree-expander',item.kind==='folder'?'›':'');expand.type='button';expand.disabled=item.kind!=='folder';expand.setAttribute('aria-label',`Раскрыть ${basename(item.path)||item.path}`);
    const name=node('button','tree-name');name.type='button';name.append(window.CleanerUltraIcons.create(item.kind==='folder'?'folder':({photos:'image',video:'video',archives:'archive'}[item.category]||'file')),node('span','',basename(item.path)||item.path));name.title=item.path;
    const size=node('span','tree-size',bytes(item.bytes)),bar=node('span','tree-share');bar.style.setProperty('--share',`${Math.min(100,tree.root?.bytes?100*item.bytes/tree.root.bytes:0)}%`);bar.setAttribute('aria-hidden','true');
    line.append(expand,name,bar,size);const children=node('div','tree-children');children.setAttribute('role','group');children.hidden=true;wrapper.append(line,children);
    const entry={item,wrapper,expand,name,children,level,parent,opened:false,loaded:false,offset:0,generation:tree.generation};tree.nodes.set(item.path,entry);
    name.onclick=()=>{selectMap(item);for(const el of tree.nodes.values())el.wrapper.classList.toggle('selected',el===entry);};name.ondblclick=()=>item.kind==='folder'&&action(()=>toggle(entry));expand.onclick=()=>action(()=>toggle(entry));
    name.onkeydown=e=>{if(e.key==='ArrowRight'&&item.kind==='folder'){e.preventDefault();action(()=>toggle(entry,true));}else if(e.key==='ArrowLeft'){e.preventDefault();if(entry.opened)action(()=>toggle(entry,false));else if(parent)parent.name.focus();}else if(e.key==='ArrowDown'||e.key==='ArrowUp'){e.preventDefault();const visible=[...$('#disk-tree').querySelectorAll('.tree-name')].filter(b=>b.getClientRects().length);const i=visible.indexOf(name);visible[i+(e.key==='ArrowDown'?1:-1)]?.focus();}};
    return entry;
  }
  async function fetchChildren(entry,offset=0){
    const generation=tree.generation;entry.expand.disabled=true;
    try{const d=await api(`/api/space/map?${new URLSearchParams({path:entry.item.path,offset})}`);if(tree.generation!==generation)return;if(d.scan!==tree.scan)throw Error('Снимок диска изменился. Обновите дерево.');
      if(!offset)entry.children.replaceChildren();else entry.children.querySelector('.tree-more')?.remove();
      for(const item of d.entries)entry.children.append(row(item,entry.level+1,entry).wrapper);
      entry.loaded=true;entry.offset=d.offset+d.entries.length;
      if(entry.offset<d.total){const more=node('button','button secondary tree-more',`Показать ещё · ${entry.offset} из ${num(d.total)}`);more.onclick=()=>action(()=>fetchChildren(entry,entry.offset));entry.children.append(more);}
      if(!d.total)entry.children.append(node('p','tree-empty','Папка пуста в текущем снимке.'));
    }finally{entry.expand.disabled=false;}
  }
  async function toggle(entry,force){
    if(entry.generation!==tree.generation)return;const open=force??!entry.opened;
    if(open&&!entry.loaded)await fetchChildren(entry);
    entry.opened=open;entry.children.hidden=!open;entry.wrapper.setAttribute('aria-expanded',String(open));entry.expand.textContent=open?'⌄':'›';entry.expand.setAttribute('aria-label',`${open?'Свернуть':'Раскрыть'} ${basename(entry.item.path)||entry.item.path}`);
  }
  async function load(data){
    tree.generation++;tree.nodes.clear();tree.scan=data.scan||0;tree.root=data;const host=$('#disk-tree');host.replaceChildren();
    if(!data.path){host.append(node('p','empty-note','Выберите диск или папку и запустите анализ.'));return;}
    const root=row({path:data.path,kind:'folder',bytes:data.bytes,files:data.files,protected:true},1,null);root.loaded=true;root.opened=true;root.children.hidden=false;root.expand.textContent='⌄';root.expand.setAttribute('aria-label',`Свернуть ${basename(data.path)||data.path}`);root.wrapper.setAttribute('aria-expanded','true');host.append(root.wrapper);
    for(const item of data.entries)root.children.append(row(item,2,root).wrapper);
    if(data.entries.length<data.total){const more=node('button','button secondary tree-more',`Показать ещё · ${data.entries.length} из ${num(data.total)}`);more.onclick=()=>action(()=>fetchChildren(root,data.entries.length));root.children.append(more);}
  }
  window.CleanerUltraTree={load};
})();
