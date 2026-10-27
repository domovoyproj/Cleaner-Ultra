/* Categories feed the common wizard; cache quarantine remains independently restorable. */
(()=>{
  const categoryNames={user_temp:'TEMP пользователя',browsers:'Кеш браузеров',windows:'Временные файлы Windows',programs:'Кеш программ'};
  let excluded=new Set();try{excluded=new Set(JSON.parse(localStorage.getItem('cleaner-ultra.keep-cache-rules')||'[]'));}catch{}
  async function catalog(){
    const d=await api('/api/cleanup/catalog');$('#cleanup-catalog-note').textContent=d.note;
    $('#cleanup-catalog').replaceChildren(...Object.entries(categoryNames).map(([category,label])=>{
      const group=node('details','cleanup-category');group.open=category==='user_temp';const summary=node('summary','',label);group.append(summary);
      const rows=d.items.filter(i=>i.rule.category===category);
      for(const item of rows){const r=item.rule,row=node('label','wizard-mode'),check=node('input');check.type='checkbox';check.dataset.cleanupRule=r.id;check.dataset.cleanupCategory=r.category;check.checked=!item.blocked&&!excluded.has(r.id);check.onchange=()=>{if(check.checked)excluded.delete(r.id);else excluded.add(r.id);try{localStorage.setItem('cleaner-ultra.keep-cache-rules',JSON.stringify([...excluded]));}catch{}};check.disabled=item.blocked;
        const copy=node('span');copy.append(node('strong','',r.label),node('small','ultra-path',r.root));if(item.reason)copy.append(node('small','ultra-errors',item.reason));row.append(check,copy);group.append(row);
      }
      if(!rows.length)group.append(node('p','fineprint','Поддерживаемые каталоги не найдены.'));
      return group;
    }));
  }
  async function trash(){
    const data=await api('/api/cleanup/trash'),container=$('#cleanup-trash');container.replaceChildren();
    async function render(group,target){
      target.replaceChildren(node('h3','',group.label));
      if(group.error){target.append(node('p','ultra-errors',group.error));return;}
      for(const f of group.files){const row=node('div','ultra-row'),copy=node('div','ultra-copy');copy.append(node('strong','',basename(f.path)),node('small','ultra-path',f.path),node('small','',bytes(f.size)));const restore=node('button','button secondary','Восстановить');restore.onclick=()=>window.CleanerUltraMaintenance.confirm('Восстановить файл кеша / TEMP?','Закройте соответствующую программу. Существующий файл не будет перезаписан.',[f.path],async()=>{const result=await api('/api/cleanup/restore',{rule_id:group.rule_id,ids:[f.id],confirmed:true});await trash();return result;});row.append(copy,restore);target.append(row);}
      const controls=node('div','ultra-toolbar');for(const[label,offset,disabled]of [['Назад',Math.max(0,group.offset-60),group.offset===0],['Далее',group.offset+60,group.offset+60>=group.total]]){const b=node('button','button secondary',label);b.disabled=disabled;b.onclick=()=>action(async()=>{const d=await api(`/api/cleanup/trash?rule_id=${group.rule_id}&offset=${offset}`);if(d.groups[0])await render(d.groups[0],target);else await trash();});controls.append(b);}controls.append(node('span','',`${group.offset+1}–${Math.min(group.offset+60,group.total)} из ${group.total}`));target.append(controls);
    }
    for(const group of data.groups){const panel=node('article','panel wizard-panel');container.append(panel);await render(group,panel);}
    if(!data.groups.length)container.append(node('p','empty-note','В корзине кешей и TEMP пока нет файлов.'));
  }
  $('#cleanup-catalog-refresh').onclick=()=>action(catalog);$('#cleanup-trash-refresh').onclick=()=>action(trash);
  window.CleanerUltraCleanup={catalog,trash};
})();
