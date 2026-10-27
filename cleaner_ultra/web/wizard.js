/* Scan -> editable server-issued findings -> explicit batch confirmation -> actual results. */
(()=>{
  const w={plan:new CleanupPlan(),offset:0,cancel:false,running:false,scan:0,report:null,visible:[],loaded:false};
  const kindNames={files:'Временный файл',startup:'Автозагрузка',registry:'Реестр',cache:'Кеш Cleaner Ultra',trim:'TRIM',power:'Схема питания'};
  const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
  function stage(name){for(const el of $$('[data-wizard-stage]'))el.hidden=el.dataset.wizardStage!==name;$('#wizard-stop').hidden=!['scan','apply'].includes(name);}
  function progress(text){$('#wizard-progress').textContent=text;}
  function review(){
    const query=$('#wizard-search').value.trim().toLocaleLowerCase('ru-RU');const rows=w.plan.items.filter(r=>`${r.name} ${r.detail||''} ${kindNames[r.kind]}`.toLocaleLowerCase('ru-RU').includes(query)),summary=w.plan.summary();w.offset=Math.min(w.offset,Math.max(0,Math.floor((rows.length-1)/60)*60));
    w.visible=rows;
    $('#wizard-total').textContent=`Выбрано: ${num(summary.count)} из ${num(w.plan.items.length)} · в фильтре: ${num(rows.length)} · файлов: ${num(summary.file_count)} (${bytes(summary.file_bytes)}) · записей: ${num(summary.registry_count)}`;
    $('#wizard-apply').disabled=!summary.count;
    $('#wizard-list').replaceChildren(...rows.slice(w.offset,w.offset+60).map(r=>{
      const row=node('label','ultra-row'),check=node('input');check.type='checkbox';check.checked=r.selected;check.setAttribute('aria-label',`Включить ${r.name}`);check.onchange=()=>{w.plan.select(r.key,check.checked);review();};
      const copy=node('div','ultra-copy');copy.append(node('strong','',r.name),node('small','',kindNames[r.kind]),node('small','ultra-path',r.detail||''));if(r.size)copy.append(node('small','',bytes(r.size)));row.append(check,copy);return row;
    }));if(!rows.length)$('#wizard-list').append(node('p','empty-note','По выбранным режимам изменений не найдено.'));
    $('#wizard-page').textContent=`${num(rows.length?w.offset+1:0)}–${num(Math.min(w.offset+60,rows.length))} из ${num(rows.length)}`;
    $('#wizard-prev').disabled=w.offset===0;$('#wizard-next').disabled=w.offset+60>=rows.length;
  }
  async function scanFiles(){
    const started=await api('/api/space/scan',{path:$('#wizard-path').value.trim()});w.scan=started.id;
    for(;;){
      const p=await api('/api/space/progress');if(p.id!==w.scan)throw Error('Другой анализ заменил снимок. Повторите мастер.');
      progress(`Временные файлы: проверено ${num(p.files)} · ${p.current||p.root}`);
      if(w.cancel){await api('/api/space/stop',{});throw Error('Анализ файлов остановлен.');}
      if(p.state!=='running'){if(p.state!=='complete')throw Error(p.current||'Анализ файлов не завершён');if(p.errors)warn(`Файлы: ${num(p.errors)} ошибок доступа; недоступные файлы не включены.`);break;}await sleep(250);
    }
    for(let offset=0;;){
      const d=await api(`/api/space/files?mode=temporary&offset=${offset}`);if(d.scan!==w.scan)throw Error('Снимок файлов изменился');
      for(const f of d.files)w.plan.add({kind:'files',id:f.id,name:basename(f.path),detail:f.path,size:f.size,scan:w.scan});
      offset+=d.files.length;if(offset>=d.total||!d.files.length)break;if(w.cancel)break;
    }
  }
  function warn(text){$('#wizard-warnings').append(node('p','ultra-errors',text));}
  async function scan(){
    const modes=$$('[data-wizard-mode]:checked').map(c=>c.dataset.wizardMode);if(!modes.length)throw Error('Выберите хотя бы один режим.');
    if(modes.includes('trim')&&!$$('[data-wizard-drive]:checked').length)throw Error('Выберите том для TRIM.');
    w.plan=new CleanupPlan();w.offset=0;w.cancel=false;w.report=null;w.running=true;$('#wizard-warnings').replaceChildren();stage('scan');
    try{
      for(const mode of modes){
        if(w.cancel)break;progress(`Проверка: ${kindNames[mode]||mode}`);
        try{
          if(mode==='files')await scanFiles();
          if(mode==='startup'||mode==='registry'){
            const d=await api(`/api/${mode}/scan`,{});for(const r of d.items.filter(r=>r.removable))w.plan.add({kind:mode,id:r.id,name:r.name||'(По умолчанию)',detail:`${r.key}\n${r.command}`});
          }
          if(mode==='cache'){const d=await api('/api/cache/status');if(d.files)w.plan.add({kind:'cache',id:'previews',name:`Кеш превью Cleaner Ultra: ${num(d.files)} файлов`,detail:'Только созданные приложением превью. Исходные фото и видео останутся. Превью будут созданы заново.',size:d.bytes});}
          if(mode==='trim'){const job=await api('/api/optimizer/status');if(job.state==='running')throw Error('Оптимизация уже выполняется');for(const c of $$('[data-wizard-drive]:checked'))w.plan.add({kind:'trim',id:c.dataset.wizardDrive,name:`TRIM ${c.dataset.wizardDrive}`,detail:'Windows обработает свободные блоки. Нужны права администратора. После запуска ReTrim нельзя отменить.'});}
          if(mode==='power'){const d=await api('/api/power'),id=$('#wizard-power').value,plan=d.plans.find(p=>p.id===id);if(!plan)throw Error('Выберите доступную схему питания');if(d.active!==id)w.plan.add({kind:'power',id,name:`Питание: ${plan.name}`,detail:'Изменится схема питания Windows. Производительная схема может повысить расход энергии. Предыдущая схема останется в истории.'});}
        }catch(e){warn(`${mode}: ${e.message}`);}
      }
    }finally{w.running=false;if(w.cancel)warn('Прохождение остановлено. Ниже только результаты завершённых проверок.');progress(w.cancel?'Проверка остановлена.':'Проверка завершена.');stage('review');review();}
  }
  async function runOperation(op){
    const ids=op.items.map(r=>r.id);
    if(op.kind==='files')return api('/api/space/trash',{ids,scan:op.items[0].scan});
    if(['startup','registry'].includes(op.kind))return api(`/api/${op.kind}/apply`,{ids,action:'clean',confirmed:true});
    if(op.kind==='cache'){const d=await api('/api/cache',{all:true,confirmed:true});return {...d,done:d.removed};}
    if(op.kind==='power')return api('/api/power/apply',{id:ids[0],confirmed:true});
    if(op.kind==='trim'){
      const started=await api('/api/space/optimize',{drive:ids[0],mode:'trim',confirmed:true});
      for(;;){const d=await api('/api/optimizer/status');if(d.id&&started.id&&d.id!==started.id)throw Error('Другая задача заменила TRIM');if(d.state!=='running')return {done:d.state==='complete'?1:0,errors:(d.errors||[]).map(message=>({message})),state:d.state};await sleep(700);}
    }
  }
  async function apply(){
    const operations=w.plan.operations();if(!operations.length)return;
    // One explicit confirmation covers exactly the edited selection. Never rescan or add rows here.
    window.CleanerUltraMaintenance.confirm('Применить выбранный список?',`Элементов в выборе: ${num(w.plan.summary().count)}. Файлы перейдут в восстанавливаемую корзину; это пока не освободит место. Изменения реестра получат резервные копии.`,w.plan.selected().map(r=>`${kindNames[r.kind]}: ${r.name} — ${r.detail||''}`),async()=>{
      $('#ultra-confirm').close();stage('apply');w.cancel=false;w.running=true;const started=Date.now(),steps=[];let done=0,moved_bytes=0,freed_bytes=0,errors=[];
      try{
        for(const [index,op]of operations.entries()){
          if(w.cancel)break;progress(`Шаг ${index+1} из ${operations.length}: ${kindNames[op.kind]}`);
          try{const r=await runOperation(op);steps.push({kind:op.kind,...r});done+=r.done||0;moved_bytes+=r.moved_bytes||0;freed_bytes+=r.freed_bytes||0;errors.push(...(r.errors||[]).map(e=>({message:`${kindNames[op.kind]}: ${typeof e==='string'?e:e.message}`})));for(const item of op.items)if(!r.errors?.length||(r.errors.every(e=>e.id!==undefined)&&!r.errors.some(e=>e.id===item.id)))w.plan.select(item.key,false);}
          catch(e){steps.push({kind:op.kind,error:e.message});errors.push({message:`${kindNames[op.kind]}: ${e.message}`});}
        }
      }finally{w.running=false;}
      const result={done,moved_bytes,freed_bytes,errors,elapsed_ms:Date.now()-started,note:w.cancel?'Выполнение остановлено после текущего шага.':'Выполнен только подтверждённый список. Пропущенные и ошибочные изменения не считаются успешными.'};
      w.report={application:'Cleaner Ultra',created_at:new Date().toISOString(),result,steps};$('#wizard-result').replaceChildren(...[`Обработано: ${num(done)}`,`В корзину: ${bytes(moved_bytes)}`,`Освобождено: ${bytes(freed_bytes)}`,`Ошибок: ${num(errors.length)}`].map(t=>node('p','ultra-result-line',t)),node('p','',result.note),...errors.map(e=>node('p','ultra-errors',e.message)));stage('done');await refresh().catch(e=>toast(e.message));return result;
    });
  }
  async function open(){
    if(w.loaded)return;w.loaded=true;
    try{const d=await api('/api/cleanup/defaults');$('#wizard-path').value=d.temporary_path;}catch(e){toast(e.message);}
    try{const d=await api('/api/power');$('#wizard-power').replaceChildren(...d.plans.map(p=>{const o=node('option','',p.name);o.value=p.id;o.selected=p.id===d.active;return o;}));}catch(e){$('#wizard-power').disabled=true;}
    const info=await api('/api/space/status');$('#wizard-drives').replaceChildren(...info.drives.map(d=>{const label=node('label','wizard-drive'),c=node('input');c.type='checkbox';c.dataset.wizardDrive=d.path.slice(0,2);label.append(c,document.createTextNode(d.path));return label;}));
  }
  $('#wizard-scan').onclick=()=>action(scan);$('#wizard-apply').onclick=()=>action(apply);
  $('#wizard-select-all').onclick=()=>{for(const item of w.visible)w.plan.select(item.key,true);review();};$('#wizard-select-none').onclick=()=>{w.plan.selectAll(false);review();};
  $('#wizard-search').oninput=()=>{w.offset=0;review();};
  $('#wizard-prev').onclick=()=>{w.offset=Math.max(0,w.offset-60);review();};$('#wizard-next').onclick=()=>{w.offset+=60;review();};
  $('#wizard-edit-modes').onclick=()=>stage('modes');$('#wizard-new').onclick=()=>stage('modes');$('#wizard-review-again').onclick=()=>{stage('review');review();};
  $('#wizard-stop').onclick=()=>{w.cancel=true;$('#wizard-progress').textContent='Остановка после текущей проверки / системного вызова…';};
  $('#wizard-browse').onclick=()=>action(async()=>{const d=await api('/api/pick',{});if(d.path)$('#wizard-path').value=d.path;});
  $('#wizard-report').onclick=()=>{if(!w.report)return;const url=URL.createObjectURL(new Blob([JSON.stringify(w.report,null,2)],{type:'application/json'}));const a=node('a');a.href=url;a.download=`CleanerUltra-session-${Date.now()}.json`;a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);};
  window.CleanerUltraWizard={open};
})();
