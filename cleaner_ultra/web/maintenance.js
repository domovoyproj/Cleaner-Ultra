/* Maintenance uses the same authenticated local API and persistent theme as disk analysis. */
(() => {
  const model={registry:[],selected:new Set(),apps:[],appOffset:0,history:[],loadedApps:false,loadedRegistry:false,optimizerTimer:null};
  const button=(text,fn,cls="button secondary")=>{const b=node("button",cls,text);b.type="button";b.onclick=()=>action(fn);return b;};
  const labels={missing:"EXE не найден",present:"EXE существует",review:"Ручная проверка"};
  const kinds={"/api/optimizer/completed":"Встроенная оптимизация","/api/space/trash":"Файлы в корзину","/api/space/restore":"Восстановление файлов","/api/maintenance/apply":"Изменение реестра","/api/maintenance/restore":"Восстановление реестра","/api/apps/uninstall":"Запуск удаления программы","/api/cache":"Обслуживание кеша"};
  function metrics(r){
    const parts=[];
    if(r.checked!==undefined)parts.push(`Проверено файлов: ${num(r.checked)}`,`Дефрагментировано: ${num(r.moved)}`,`Проверено SHA-256: ${bytes(r.verified_bytes)}`,`Состояние: ${r.state}`);
    if(r.done!==undefined)parts.push(`Обработано: ${num(r.done)}`);
    if(r.moved_bytes!==undefined)parts.push(`В корзину: ${bytes(r.moved_bytes)}`);
    if(r.freed_bytes!==undefined)parts.push(`Освобождено: ${bytes(r.freed_bytes)}`);
    if(r.restored)parts.push("Запись восстановлена");
    if(r.launched)parts.push("Деинсталлятор запущен; завершение не подтверждено");
    if(r.elapsed_ms!==undefined)parts.push(`Время: ${(r.elapsed_ms/1000).toLocaleString("ru-RU",{maximumFractionDigits:1})} с`);
    parts.push(`Ошибок: ${num(r.errors?.length||0)}`);
    return parts;
  }
  function result(title,r){
    $("#operation-result-title").textContent=title;
    const body=$("#operation-result-body");body.replaceChildren(...metrics(r).map(t=>node("p","ultra-result-line",t)));
    if(r.note)body.append(node("p","fineprint",r.note));
    if(r.history_saved===false)body.append(node("p","ultra-errors","Историю не удалось записать. Сохраните этот результат."));
    for(const e of r.errors||[])body.append(node("p","ultra-errors",`${e.name||e.id||""}: ${e.message}`));
    $("#operation-result").showModal();
  }
  function confirm(title,copy,items,run){
    $("#ultra-confirm-title").textContent=title;$("#ultra-confirm-copy").textContent=copy;
    $("#ultra-confirm-items").replaceChildren(...items.map(text=>node("p","ultra-path",text)));
    $("#ultra-confirm-run").onclick=()=>action(async()=>{
      const b=$("#ultra-confirm-run");b.disabled=true;
      try{const r=await run();$("#ultra-confirm").close();result(title,r);}finally{b.disabled=false;}
    });$("#ultra-confirm").showModal();
  }
  function registrySelection(){
    const values=model.registry.filter(r=>model.selected.has(r.id));
    $("#registry-selection").textContent=values.length?`Выбрано: ${num(values.length)} / 100`:"Ничего не выбрано";
    $("#registry-clean").disabled=!values.length||values.some(r=>!r.removable);
    $("#startup-disable").disabled=!values.length;
  }
  function renderRegistry(){
    const filter=$("#registry-filter").value;
    const values=model.registry.filter(r=>filter==="all"||r.state===filter);
    $("#registry-list").replaceChildren(...values.map(r=>{
      const row=node("div","ultra-row");const check=node("input");check.type="checkbox";
      check.setAttribute("aria-label",`Выбрать запись ${r.name}`);check.checked=model.selected.has(r.id);
      check.onchange=()=>{if(check.checked){if(model.selected.size>=100){check.checked=false;toast("За раз можно изменить до 100 записей");return;}model.selected.add(r.id);}else model.selected.delete(r.id);registrySelection();};
      const copy=node("div","ultra-copy");copy.append(node("strong","",r.name),node("small","ultra-path",r.command),node("small","ultra-path",r.key));
      row.append(check,copy,node("span",`ultra-tag ${r.state}`,labels[r.state]));return row;
    }));if(!values.length)$("#registry-list").append(empty("В этом фильтре записей нет."));registrySelection();
  }
  async function scanRegistry(){
    $("#registry-meta").textContent="Проверяем записи…";
    try{const data=await api("/api/maintenance/scan",{});model.registry=data.items;model.selected.clear();model.loadedRegistry=true;renderRegistry();$("#registry-meta").textContent=`Записей: ${num(data.items.length)} · ${data.scope} · ${new Date().toLocaleTimeString("ru-RU")}`;}
    catch(e){$("#registry-meta").textContent="Проверка не завершена";throw e;}
  }
  function changeRegistry(operation){
    const values=model.registry.filter(r=>model.selected.has(r.id));if(!values.length)return;
    const ids=values.map(r=>r.id);
    confirm(operation==="clean"?"Убрать отсутствующие записи?":"Отключить автозагрузку?","Полное значение каждой записи сохранится в резервной копии. Отключение может убрать нужный вам компонент из автозагрузки. Восстановление доступно ниже.",values.map(r=>`${r.name} — ${r.command}`),async()=>{
      const r=await api("/api/maintenance/apply",{ids,action:operation,confirmed:true});
      // Show the mutation result even if a subsequent refresh fails.
      try{await scanRegistry();await backups();}catch(e){toast(`Изменения выполнены. Обновление: ${e.message}`);}return r;
    });
  }
  async function backups(){
    const d=await api("/api/maintenance/backups");
    $("#registry-backups").replaceChildren(...d.items.map(r=>{
      const row=node("div","ultra-row");const copy=node("div","ultra-copy");copy.append(node("strong","",r.name),node("small","ultra-path",r.command||r.key),node("small","",`${r.created} UTC · ${r.state==="restored"?"Восстановлено":r.state==="pending"?"Незавершённая операция; копия сохранена":"Резервная копия сохранена"}`));row.append(copy);
      if(r.state!=="restored")row.append(button("Восстановить",async()=>{confirm("Восстановить запись?","Существующая другая запись не будет заменена.",[r.name,r.key],async()=>{const answer=await api("/api/maintenance/restore",{id:r.id});try{await backups();if(model.loadedRegistry)await scanRegistry();}catch(e){toast(e.message);}return answer;});}));return row;
    }));if(!d.items.length)$("#registry-backups").append(empty("Резервные копии появятся после первого изменения."));
  }
  async function loadApps(){
    $("#apps-count").textContent="Читаем список…";
    try{const d=await api("/api/apps");model.apps=d.items;model.appOffset=0;model.loadedApps=true;$("#apps-warnings").textContent=(d.warnings||[]).join(" · ");renderApps();}
    catch(e){$("#apps-count").textContent="Не удалось прочитать список";throw e;}
  }
  function renderApps(){
    const query=$("#apps-search").value.trim().toLocaleLowerCase("ru-RU");
    const apps=model.apps.filter(a=>`${a.name} ${a.publisher}`.toLocaleLowerCase("ru-RU").includes(query));
    model.appOffset=Math.min(model.appOffset,Math.max(0,Math.floor((apps.length-1)/50)*50));
    $("#apps-count").textContent=`Найдено: ${num(apps.length)}`;
    $("#apps-list").replaceChildren(...apps.slice(model.appOffset,model.appOffset+50).map(a=>{
      const row=node("div","ultra-row");const copy=node("div","ultra-copy");copy.append(node("strong","",a.name),node("small","",[a.publisher,a.version,a.size_kb?`Оценка установщика: ${bytes(a.size_kb*1024)}`:"Размер неизвестен"].filter(Boolean).join(" · ")));
      const remove=button("Удалить",async()=>confirm(`Удалить ${a.name}?`,"Откроется штатный деинсталлятор приложения. Завершите его мастер и обновите список. Оценка размера не означает объём, который будет освобождён.",[a.name,a.publisher],()=>api("/api/apps/uninstall",{id:a.id,confirmed:true})));remove.disabled=!a.removable;if(!a.removable)remove.title="Безопасная команда удаления не найдена";
      row.append(copy,remove);return row;
    }));if(!apps.length)$("#apps-list").append(empty("Подходящие приложения не найдены."));
    $("#apps-page").textContent=`${num(apps.length?model.appOffset+1:0)}–${num(Math.min(model.appOffset+50,apps.length))} из ${num(apps.length)}`;
    $("#apps-prev").disabled=model.appOffset===0;$("#apps-next").disabled=model.appOffset+50>=apps.length;
  }
  async function history(){
    const d=await api("/api/history");model.history=d.items;
    $("#history-list").replaceChildren(...d.items.map(h=>{
      const row=node("article","ultra-history");row.append(node("strong","",kinds[h.kind]||h.kind),node("small","",`${h.created} UTC`),node("p","",metrics(h.result).join(" · ")));
      for(const e of h.result.errors||[])row.append(node("p","ultra-errors",e.message));return row;
    }));if(!d.items.length)$("#history-list").append(empty("История пока пуста. Выполненные операции будут сохранены здесь."));
  }
  async function optimizer(){
    const d=await api("/api/optimizer/status");
    const labels={running:"Операция выполняется",complete:"Операция завершена",cancelled:"Операция остановлена",error:"Операция прервана"};
    $("#optimizer-state").textContent=labels[d.state]||"Готов к работе";$("#optimizer-stop").disabled=d.state!=="running";
    $("#optimizer-progress").max=d.total||1;$("#optimizer-progress").value=d.checked||0;$("#optimizer-current").textContent=d.current||"";
    $("#optimizer-summary").replaceChildren(...[`Проверено: ${num(d.checked)} / ${num(d.total)}`,`Фрагментированных: ${num(d.fragmented)}`,`Дефрагментировано: ${num(d.moved)}`,`Проверено SHA-256: ${bytes(d.verified_bytes)}`,`Пропущено: ${num(d.skipped)}`,`Время: ${Math.round(d.elapsed_ms/1000)} с`].map(t=>node("span","ultra-metric",t)));
    $("#optimizer-errors").replaceChildren(...(d.errors||[]).map(t=>node("p","",t)));
    clearTimeout(model.optimizerTimer);if(d.state==="running")model.optimizerTimer=setTimeout(()=>optimizer().catch(e=>toast(e.message)),1000);
  }
  $("#registry-scan").onclick=()=>action(scanRegistry);$("#registry-filter").onchange=renderRegistry;
  $("#registry-clean").onclick=()=>changeRegistry("clean");$("#startup-disable").onclick=()=>changeRegistry("disable");$("#registry-backups-refresh").onclick=()=>action(backups);
  $("#apps-refresh").onclick=()=>action(loadApps);$("#apps-search").oninput=()=>{model.appOffset=0;renderApps();};
  $("#apps-prev").onclick=()=>{model.appOffset=Math.max(0,model.appOffset-50);renderApps();};$("#apps-next").onclick=()=>{model.appOffset+=50;renderApps();};
  $("#history-refresh").onclick=()=>action(history);$("#optimizer-stop").onclick=()=>action(async()=>{await api("/api/optimizer/stop",{});toast("Остановка после текущего системного вызова");});
  $("#report-export").onclick=()=>action(async()=>{
    await history();const info=await api("/api/space/status");const job=await api("/api/optimizer/status");
    const report={application:"Cleaner Ultra",version:"0.7.0",created_at:new Date().toISOString(),disk_snapshot:info,operations:model.history,optimizer:job};
    const url=URL.createObjectURL(new Blob([JSON.stringify(report,null,2)],{type:"application/json"}));const a=node("a");a.href=url;a.download=`CleanerUltra-report-${new Date().toISOString().slice(0,10)}.json`;a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);toast("Локальный отчёт подготовлен. Он содержит пути файлов из вашего анализа.");
  });
  window.CleanerUltraMaintenance={result,optimizer,open:async(page)=>{if(page==="maintenance")await backups();if(page==="apps"&&!model.loadedApps)await loadApps();if(page==="history")await history();if(page==="optimize")await optimizer();}};
})();
