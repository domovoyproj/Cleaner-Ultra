/* Maintenance uses the same authenticated local API and persistent theme as disk analysis. */
(() => {
  const model={registry:[],startup:[],selections:{startup:new Set(),registry:new Set()},offsets:{startup:0,registry:0},apps:[],appOffset:0,history:[],loadedApps:false,loadedRegistry:false,backupOffset:0,optimizerTimer:null};
  const button=(text,fn,cls="button secondary")=>{const b=node("button",cls,text);b.type="button";b.onclick=()=>action(fn);return b;};
  const labels={missing:"EXE не найден",present:"EXE существует",review:"Ручная проверка"};
  const kinds={"/api/startup/apply":"Автозагрузка","/api/registry/apply":"Исправление реестра","/api/power/apply":"Смена схемы питания","/api/optimizer/completed":"Встроенная оптимизация","/api/space/trash":"Файлы в корзину","/api/space/restore":"Восстановление файлов","/api/maintenance/apply":"Изменение реестра","/api/maintenance/restore":"Восстановление реестра","/api/apps/uninstall":"Запуск удаления программы","/api/cache":"Обслуживание кеша"};
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
  const categoryNames={app_paths:"Путь приложения",icons:"Значок удалённой программы",com_dll:"COM-библиотека",com_exe:"COM-сервер",startup:"Автозагрузка"};
  function selection(scope){
    const values=model[scope].filter(r=>model.selections[scope].has(r.id));
    $(`#${scope}-selection`).textContent=`Выбрано: ${num(values.length)} · отсутствующих: ${num(model[scope].filter(r=>r.removable).length)}`;
    $(`#${scope}-clean`).disabled=!values.length||values.some(r=>!r.removable);
    $(`#${scope}-missing`).disabled=!model[scope].some(r=>r.removable);
    if(scope==="startup")$("#startup-disable").disabled=!values.length;
  }
  function renderRegistry(scope){
    const filter=$(`#${scope}-filter`).value;
    const values=model[scope].filter(r=>filter==="all"||r.state===filter);
    const offset=model.offsets[scope]=Math.min(model.offsets[scope],Math.max(0,Math.floor((values.length-1)/60)*60));
    $(`#${scope}-list`).replaceChildren(...values.slice(offset,offset+60).map(r=>{
      const row=node("div","ultra-row"),check=node("input");check.type="checkbox";
      check.setAttribute("aria-label",`Выбрать запись ${r.name||"По умолчанию"}`);check.checked=model.selections[scope].has(r.id);check.disabled=scope==="registry"&&!r.removable;
      check.onchange=()=>{if(check.checked)model.selections[scope].add(r.id);else model.selections[scope].delete(r.id);selection(scope);};
      const copy=node("div","ultra-copy");copy.append(node("strong","",r.name||"(По умолчанию)"),node("small","",categoryNames[r.category]||""),node("small","ultra-path",r.command),node("small","ultra-path",r.key));
      row.append(check,copy,node("span",`ultra-tag ${r.state}`,labels[r.state]));return row;
    }));
    if(!values.length)$(`#${scope}-list`).append(empty("В этом фильтре записей нет."));
    $(`#${scope}-page`).textContent=`${num(values.length?offset+1:0)}–${num(Math.min(offset+60,values.length))} из ${num(values.length)}`;
    $(`#${scope}-prev`).disabled=offset===0;$(`#${scope}-next`).disabled=offset+60>=values.length;selection(scope);
  }
  async function scanRegistry(scope){
    $(`#${scope}-meta`).textContent="Проверяем записи…";
    try{const data=await api(`/api/${scope}/scan`,{});model[scope]=data.items;model.selections[scope].clear();model.offsets[scope]=0;renderRegistry(scope);$(`#${scope}-meta`).textContent=`Записей: ${num(data.items.length)} · ${data.scope}`;}
    catch(e){$(`#${scope}-meta`).textContent="Проверка не завершена";throw e;}
  }
  function changeRegistry(scope,operation,allMissing=false){
    const values=model[scope].filter(r=>allMissing?r.removable:model.selections[scope].has(r.id));if(!values.length)return;
    confirm(scope==="startup"?(operation==="clean"?"Удалить несуществующие пути автозагрузки?":"Отключить автозагрузку?"):"Исправить выбранные ошибки реестра?","До изменения каждого значения сохраняется точная резервная копия. Проверьте список. Восстановление доступно в обоих разделах.",values.map(r=>`${r.name||"По умолчанию"} — ${r.command}`),async()=>{
      const r=await api(`/api/${scope}/apply`,{ids:values.map(r=>r.id),action:operation,confirmed:true});
      try{await scanRegistry(scope);await backups();}catch(e){toast(`Изменения выполнены. Обновление: ${e.message}`);}return r;
    });
  }
  async function backups(offset=model.backupOffset){
    const d=await api(`/api/maintenance/backups?offset=${offset}`);model.backupOffset=d.offset;
    for(const scope of ["startup","registry"]){
      $(`#${scope}-backups-page`).textContent=`${num(d.total?d.offset+1:0)}–${num(Math.min(d.offset+d.page_size,d.total))} из ${num(d.total)}`;
      $(`#${scope}-backups-prev`).disabled=d.offset===0;$(`#${scope}-backups-next`).disabled=d.offset+d.page_size>=d.total;
      const list=$(`#${scope}-backups`);list.replaceChildren(...d.items.map(r=>{
        const row=node("div","ultra-row"),copy=node("div","ultra-copy");copy.append(node("strong","",r.name||"По умолчанию"),node("small","ultra-path",r.key),node("small","",`${r.created} UTC · ${r.state==="restored"?"Восстановлено":r.state==="pending"?"Незавершённая операция; копия сохранена":"Резервная копия сохранена"}`));row.append(copy);
        if(r.state!=="restored")row.append(button("Восстановить",async()=>confirm("Восстановить запись?","Существующая другая запись не будет заменена.",[r.name||"По умолчанию",r.key],async()=>{const answer=await api("/api/maintenance/restore",{id:r.id});await backups();return answer;})));return row;
      }));if(!d.items.length)list.append(empty("Резервные копии появятся после первого изменения."));
    }
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
      for(const e of h.result.errors||[])row.append(node("p","ultra-errors",typeof e==="string"?e:e.message));return row;
    }));if(!d.items.length)$("#history-list").append(empty("История пока пуста. Выполненные операции будут сохранены здесь."));
  }
  async function optimizer(){
    const d=await api("/api/optimizer/status");
    const labels={running:"Операция выполняется",complete:"Операция завершена",cancelled:"Операция остановлена",error:"Операция прервана"};
    $("#optimizer-state").textContent=labels[d.state]||"Готов к работе";$("#optimizer-stop").disabled=d.state!=="running"||d.mode==="trim";
    $("#optimizer-progress").max=d.total||1;if(d.mode==="trim"&&d.state==="running")$("#optimizer-progress").removeAttribute("value");else $("#optimizer-progress").value=d.checked||0;$("#optimizer-current").textContent=(d.mode==="trim"&&d.state==="running")?"Windows ReTrim выполняется. Точный процент недоступен; дождитесь результата.":d.current||"";
    $("#optimizer-summary").replaceChildren(...[`Проверено: ${num(d.checked)} / ${num(d.total)}`,`Фрагментированных: ${num(d.fragmented)}`,`Дефрагментировано: ${num(d.moved)}`,`Проверено SHA-256: ${bytes(d.verified_bytes)}`,`Пропущено: ${num(d.skipped)}`,`Время: ${Math.round(d.elapsed_ms/1000)} с`].map(t=>node("span","ultra-metric",t)));
    $("#optimizer-errors").replaceChildren(...(d.errors||[]).map(t=>node("p","",t)));
    clearTimeout(model.optimizerTimer);if(d.state==="running")model.optimizerTimer=setTimeout(()=>optimizer().catch(e=>toast(e.message)),1000);
  }
  for(const scope of ["startup","registry"]){
    $(`#${scope}-scan`).onclick=()=>action(()=>scanRegistry(scope));
    $(`#${scope}-filter`).onchange=()=>{model.offsets[scope]=0;renderRegistry(scope);};
    $(`#${scope}-clean`).onclick=()=>changeRegistry(scope,"clean");
    $(`#${scope}-missing`).onclick=()=>changeRegistry(scope,"clean",true);
    $(`#${scope}-select-missing`).onclick=()=>{model.selections[scope]=new Set(model[scope].filter(r=>r.removable).map(r=>r.id));renderRegistry(scope);};
    $(`#${scope}-clear`).onclick=()=>{model.selections[scope].clear();renderRegistry(scope);};
    $(`#${scope}-prev`).onclick=()=>{model.offsets[scope]=Math.max(0,model.offsets[scope]-60);renderRegistry(scope);};
    $(`#${scope}-next`).onclick=()=>{model.offsets[scope]+=60;renderRegistry(scope);};
    $(`#${scope}-backups-refresh`).onclick=()=>action(()=>backups());
    $(`#${scope}-backups-prev`).onclick=()=>action(()=>backups(Math.max(0,model.backupOffset-60)));
    $(`#${scope}-backups-next`).onclick=()=>action(()=>backups(model.backupOffset+60));
  }
  $("#startup-disable").onclick=()=>changeRegistry("startup","disable");
  $("#apps-refresh").onclick=()=>action(loadApps);$("#apps-search").oninput=()=>{model.appOffset=0;renderApps();};
  $("#apps-prev").onclick=()=>{model.appOffset=Math.max(0,model.appOffset-50);renderApps();};$("#apps-next").onclick=()=>{model.appOffset+=50;renderApps();};
  $("#history-refresh").onclick=()=>action(history);$("#optimizer-stop").onclick=()=>action(async()=>{await api("/api/optimizer/stop",{});toast("Остановка после текущего системного вызова");});
  $("#report-export").onclick=()=>action(async()=>{
    await history();const info=await api("/api/space/status");const job=await api("/api/optimizer/status");
    const report={application:"Cleaner Ultra",version:state.version||"0.8.0",created_at:new Date().toISOString(),disk_snapshot:info,operations:model.history,optimizer:job};
    const url=URL.createObjectURL(new Blob([JSON.stringify(report,null,2)],{type:"application/json"}));const a=node("a");a.href=url;a.download=`CleanerUltra-report-${new Date().toISOString().slice(0,10)}.json`;a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);toast("Локальный отчёт подготовлен. Он содержит пути файлов из вашего анализа.");
  });
  window.CleanerUltraMaintenance={result,confirm,optimizer,open:async(page)=>{if(page==="registry"||page==="startup")await backups();if(page==="apps"&&!model.loadedApps)await loadApps();if(page==="history")await history();if(page==="optimize")await optimizer();}};
})();
