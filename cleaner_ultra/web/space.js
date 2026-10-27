const $ = (s) => document.querySelector(s);
const $$ = (s) => [...document.querySelectorAll(s)];
const names = {startup:"Автозагрузка",registry:"Реестр",wizard:"Ускорить и очистить",performance:"Производительность",apps:"Удаление программ",history:"История и отчёты",overview:"Обзор",map:"Дерево диска",large:"Крупные файлы",cleanup:"Очистка",optimize:"Оптимизация дисков",trash:"Корзина"};
const categories = {photos:"Изображения",video:"Видео",audio:"Аудио",archives:"Архивы и образы",documents:"Документы",apps:"Приложения",other:"Другое"};
const headings = {
  startup:["БЫСТРЕЕ К РАБОТЕ","Порядок<br>в запуске<span>.</span>","Программы при входе в Windows. Уберите несуществующие пути или отключите выбранные."],
  registry:["ОБСЛУЖИВАНИЕ WINDOWS","Чистые ссылки<span>.</span>","Отдельная проверка битых путей реестра с резервными копиями."],
  wizard:["ОДНО ПРОХОЖДЕНИЕ","Ускорить<br>и очистить<span>.</span>","Выберите режимы. Проверьте общий список. Примените одним действием."],
  performance:["РЕСУРСЫ КОМПЬЮТЕРА","Найти причину<span>.</span>","Загрузка CPU и RAM, ресурсоёмкие процессы и схемы питания Windows."],
  apps:["ТОЛЬКО НУЖНЫЕ ПРОГРАММЫ","Меньше лишнего.<br>Больше места<span>.</span>","Найдите приложение и запустите его штатное удаление."],
  history:["ПОНЯТНЫЙ РЕЗУЛЬТАТ","Что изменилось<span>.</span>","История операций, подробные итоги и локальные отчёты."],
  overview:["ПРОСТРАНСТВО ДЛЯ ВАЖНОГО","Место под вашим<br>контролем<span>.</span>","Узнайте, что занимает диск. Оставьте только нужное."],
  map:["УВИДЕТЬ БОЛЬШЕ","Диск по ветвям<span>.</span>","Раскройте папки. Сравните объёмы. Оставьте важное."],
  large:["ВЕСОМЫЕ ПРИЧИНЫ ДЛЯ ПОРЯДКА","Большие файлы.<br>Большие возможности<span>.</span>","Самое объёмное — в начале списка. Каждый файл под вашим контролем."],
  cleanup:["ОСТАВИТЬ НУЖНОЕ","Свободнее диск.<br>Легче работать<span>.</span>","Проверьте кандидатов и выберите подходящий способ очистки."],
  optimize:["БОЛЬШЕ ПОРЯДКА ВНУТРИ","Диск в хорошей<br>форме<span>.</span>","Дефрагментация HDD и TRIM свободных блоков SSD внутри приложения."],
  trash:["ПРАВО ПЕРЕДУМАТЬ","Ничего важного<br>не потеряно<span>.</span>","Проверьте сохранённые файлы и восстановите нужные."],
};
const state = {token:"",page:"overview",info:null,drive:null,mode:"large",offset:0,search:"",category:"",folder:"",mapPath:"",mapOffset:0,selected:new Map(),files:[],scan:0,total:0,busy:false,running:false,fileRequest:0,mapRequest:0,trashOffset:0};
const num = (v) => Number(v||0).toLocaleString("ru-RU");
function bytes(v) { const n=Number(v||0); if(!n)return "0 Б"; const units=["Б","КБ","МБ","ГБ","ТБ"]; const i=Math.min(4,Math.floor(Math.log(n)/Math.log(1024))); return `${(n/1024**i).toLocaleString("ru-RU",{maximumFractionDigits:i>1?1:0})} ${units[i]}`; }
const basename = (p) => p.replaceAll("\\","/").split("/").filter(Boolean).pop()||p;
const date = (v) => v?new Date(v*1000).toLocaleDateString("ru-RU"):"—";
const node = (tag,cls,text) => { const n=document.createElement(tag);if(cls)n.className=cls;if(text!==undefined)n.textContent=text;return n; };
let toastTimer, searchTimer;
function toast(text) {$("#toast").textContent=text;$("#toast").hidden=false;clearTimeout(toastTimer);toastTimer=setTimeout(()=>$("#toast").hidden=true,9000);}
async function api(path,body) {
  const r=await fetch(path,body===undefined?{}:{method:"POST",headers:{"Content-Type":"application/json","X-CleanerUltra-Token":state.token},body:JSON.stringify(body)});
  const d=await r.json();if(!r.ok)throw Error(d.error||"Не удалось выполнить действие");return d;
}
async function action(fn) {if(state.busy)return;state.busy=true;document.body.classList.add("busy");try{return await fn();}catch(e){toast(e.message);}finally{state.busy=false;document.body.classList.remove("busy");}}
function theme(value,persist=true) {const resolved=value==="system"?(matchMedia("(prefers-color-scheme:dark)").matches?"dark":"light"):value;document.body.classList.toggle("theme-dark",resolved==="dark");$$("[data-theme]").forEach(b=>b.classList.toggle("active",b.dataset.theme===resolved));if(persist&&state.token)api("/api/space/theme",{theme:value}).catch(e=>toast(`Не удалось сохранить тему: ${e.message}`));}
$$("[data-theme]").forEach(b=>b.onclick=()=>theme(b.dataset.theme));
function progress(p) {
  state.running=p.state==="running";
  $("#scan-banner").hidden=!state.running;
  $("#scan-button").disabled=state.running;
  $("#choose-folder").disabled=state.running;
  exclusionControls();
  updateOptimizerAvailability(p);
  $("#scan-label").textContent="Анализируем пространство";
  $("#scan-current").textContent=p.current||p.root;
  $("#scan-counts").textContent=`Файлов: ${num(p.files)} · ${bytes(p.bytes)}`;
  $("#snapshot-line").hidden=!p.id;
  const labels={running:"Анализ идёт",complete:"Анализ завершён",cancelled:"Неполный анализ · остановлен",interrupted:"Неполный анализ · перезапуск приложения",error:"Анализ прерван ошибкой"};
  $("#snapshot-label").textContent=p.id?`${labels[p.state]||p.state} · ${p.root} · файлов: ${num(p.files)}`:"";
  $("#snapshot-label").title=p.root||"";
  $("#snapshot-errors").textContent=p.errors?`${num(p.errors)} недоступных элементов`:"";
  if(p.state==="error")$("#snapshot-errors").textContent=p.current||"Не удалось завершить анализ";
}
function selectedDrive() {return state.info?.drives.find(d=>d.path===state.drive)||state.info?.drives[0];}
function renderDrive() {
  const drive=selectedDrive();
  $$(".drive-tab").forEach(b=>b.classList.toggle("active",b.dataset.drive===drive?.path));
  if(!drive){$("#capacity-hint").textContent="Локальные диски не найдены. Можно выбрать папку.";return;}
  const percent=Math.round(drive.used/drive.total*100);
  $("#capacity-title").textContent=`Локальный диск ${drive.path.replaceAll("\\","")}`;
  $("#capacity-used").textContent=bytes(drive.used);
  $("#capacity-total").textContent=`из ${bytes(drive.total)}`;
  $("#capacity-free").textContent=bytes(drive.free);
  $("#capacity-percent").textContent=`${percent}%`;
  $("#capacity-track span").style.width=`${percent}%`;
  $("#capacity-state").textContent=percent>=90?"Почти заполнен":"Подключён";
  $("#capacity-hint").textContent=percent>=90?"Мало свободного места — начните с крупных файлов":"Фактическое использование тома по данным Windows";
}
async function refresh() {
  const info=await api("/api/space/status");const previous=state.scan;state.info=info;state.scan=info.progress.id;theme(info.theme||"light",false);
  if(previous!==state.scan){state.selected.clear();state.offset=0;state.mapPath="";state.mapOffset=0;state.folder="";}
  if(!state.drive||!info.drives.some(d=>d.path===state.drive))state.drive=info.drives[0]?.path;
  $("#drives").replaceChildren(...info.drives.map(d=>{
    const b=node("button","drive-tab");b.dataset.drive=d.path;b.append(window.CleanerUltraIcons.create("drive"),node("span","",`Диск ${d.path.replaceAll("\\","")}`),node("small","",`${bytes(d.free)} свободно`));b.onclick=()=>{state.drive=d.path;renderDrive();};return b;
  }));
  renderDrive();progress(info.progress);
  $("#exclusion-count").textContent=num(info.exclusion_count);
  $("#scan-exclusion-count").textContent=num(info.exclusion_count);
  $("#exclusions-warning").hidden=!info.exclusions_pending;
  if(info.exclusions_pending)state.selected.clear();selection();
  $("#admin-button").disabled=info.administrator||!desktop||state.running;
  $("#admin-button").textContent=info.administrator?"◇ Права администратора":"◇ Администратор";
  $("#admin-button").title=info.administrator?"Приложение уже запущено с повышенными правами":!desktop?"Повышение прав доступно в настольном приложении":state.running?"Сначала остановите текущий анализ":"Перезапустить с правами администратора";
  $("#large-bytes").textContent=state.scan?bytes(info.large.bytes):"—";
  $("#large-count").textContent=state.scan?`От 100 МБ: ${num(info.large.files)} · без системных`:"Файлы от 100 МБ";
  $("#temp-bytes").textContent="Проверить";
  $("#cleanup-temp").textContent=state.scan?bytes(info.temporary.bytes):"—";
  $("#cleanup-large").textContent=state.scan?bytes(info.large.bytes):"—";
  $("#cleanup-count").textContent=num(info.temporary.files);
  $("#trash-count").textContent=num(info.trash.files+(info.cleanup_trash_files||0));
  if(state.scan){$("#opportunity-title").textContent=info.large.files?`${bytes(info.large.bytes)} для внимательного взгляда.`:"Пространство стало понятнее.";$("#opportunity-copy").textContent=info.large.files?`Крупных файлов вне системных папок: ${num(info.large.files)}. Посмотрите, что ещё нужно, а с чем пора расстаться.`:"Откройте карту папок или системную очистку, чтобы выбрать следующий шаг.";}
  renderCategories(info.categories);
  renderOptimizer(info.drives);
  const top=await api("/api/space/files?mode=all&offset=0");
  $("#top-files").replaceChildren(...top.files.slice(0,5).map(f=>{
    const b=node("button","top-file");const copy=node("div","file-copy");copy.append(node("strong","",basename(f.path)),node("small","",f.path));b.append(fileIcon(f.path),copy,node("span","",bytes(f.size)));b.title="Показать в Проводнике";b.onclick=()=>action(()=>api("/api/space/reveal",{id:f.id}));return b;
  }));
  if(!top.files.length)$("#top-files").append(empty("Здесь появятся самые объёмные файлы.<br>Начните с анализа диска или папки."));
  if(state.page==="map")await loadMap();
  if(state.page==="large")await loadFiles();
  if(state.page==="trash")await loadTrash();
}
function empty(message) {const n=node("div","empty-note");for(const [i,line]of message.split("<br>").entries()){if(i)n.append(document.createElement("br"));n.append(document.createTextNode(line));}return n;}
function fileIcon(path) {return node("span","file-symbol",(path.split(".").pop()||"FILE").toUpperCase().slice(0,4));}
function renderCategories(values) {
  const total=values.reduce((sum,v)=>sum+v.bytes,0);
  $("#category-bars").replaceChildren(...values.map(v=>{const r=node("div","category-row");const track=node("span","category-bar");const fill=node("i");fill.style.width=`${total?v.bytes/total*100:0}%`;track.append(fill);r.append(node("span","",categories[v.category]||v.category),track,node("span","",bytes(v.bytes)));return r;}));
  if(!values.length)$("#category-bars").append(empty("Категории появятся после анализа."));
}
async function go(page,options={}) {
  if(!names[page])return;state.page=page;document.body.classList.toggle("view-map",page==="map");if(page!=="map")document.body.classList.remove("map-focus");
  if(page==="large"&&!options.preserve){state.mode="large";state.folder="";state.offset=0;state.selected.clear();state.search="";$("#file-search").value="";}
  $$("[data-section]").forEach(s=>s.hidden=s.dataset.section!==page);
  $$("nav [data-page]").forEach(b=>b.classList.toggle("active",b.dataset.page===page));
  $("#crumb").textContent=names[page];
  const [eyebrow,title,description]=headings[page];$("#page-eyebrow").textContent=eyebrow;$("#page-title").innerHTML=title;$("#page-description").textContent=description;
  if(page==="large"){if(state.mode==="temporary"){$("#page-title").textContent="Временное — на проверку.";$("#page-description").textContent="Файлы TEMP старше 7 дней. Ничего не выбрано заранее.";}await loadFiles();}
  if(page==="map")await loadMap();
  if(page==="trash"){await loadTrash();await window.CleanerUltraCleanup?.trash();}
  if(page==="wizard")await window.CleanerUltraWizard?.open();
  if(page==="performance")await window.CleanerUltraPerformance?.open();else window.CleanerUltraPerformance?.close();
  if(window.CleanerUltraMaintenance)await window.CleanerUltraMaintenance.open(page);
  $('[data-section="cleanup-recovery"]').hidden=page!=="trash";
  window.scrollTo({top:0,behavior:"instant"});
}
$$("[data-page]").forEach(b=>b.onclick=()=>action(()=>go(b.dataset.page)));
function openScan(folder=false) {$("#scan-path").value=folder?"":selectedDrive()?.path||"";$("#scan-dialog").showModal();$("#scan-path").focus();}
$("#scan-button").onclick=()=>openScan();$("#choose-folder").onclick=()=>openScan(true);
$("#browse-path").onclick=()=>action(async()=>{const r=await api("/api/pick",{});if(r.path)$("#scan-path").value=r.path;});
$("#scan-form").onsubmit=e=>{e.preventDefault();action(async()=>{await api("/api/space/scan",{path:$("#scan-path").value.trim()});$("#scan-dialog").close();await refresh();});};
let exclusionPaths=[];
function exclusionAction(fn) {return action(async()=>{try{$("#exclusion-error").hidden=true;await fn();}catch(e){$("#exclusion-error").textContent=e.message;$("#exclusion-error").hidden=false;}});}
function exclusionControls() {
  for(const id of ["exclusion-path","add-exclusion","exclude-folder","exclude-file"])$("#"+id).disabled=state.running;
  $$("#exclusion-list button").forEach(b=>b.disabled=state.running);
  $("#exclusion-status").textContent=state.running?"Анализ идёт. Остановите его, чтобы изменить исключения.":"Изменения сохраняются сразу и применяются при следующем анализе. Фото- и видеоразбор имеет отдельную библиотеку.";
}
function renderExclusions() {
  $("#exclusion-total").textContent=`${num(exclusionPaths.length)} / 256`;
  $("#exclusion-list").replaceChildren(...exclusionPaths.map((path,index)=>{
    const row=node("div","exclusion-row"),copy=node("div","exclusion-copy");copy.dataset.plainText="";
    copy.append(node("strong","",basename(path)),node("small","",path));copy.title=path;
    const remove=node("button","icon-button");remove.type="button";remove.setAttribute("aria-label",`Убрать исключение ${path}`);remove.title="Убрать исключение";remove.append(window.CleanerUltraIcons.create("close"));
    remove.onclick=()=>exclusionAction(()=>saveExclusions(exclusionPaths.filter((_,i)=>i!==index)));
    row.append(window.CleanerUltraIcons.create("shield"),copy,remove);return row;
  }));
  if(!exclusionPaths.length)$("#exclusion-list").append(empty("Исключений пока нет.<br>Добавьте папки, которые не нужно анализировать."));
  exclusionControls();
}
async function openExclusions() {
  $("#exclusion-error").hidden=true;const data=await api("/api/space/exclusions");exclusionPaths=data.paths;renderExclusions();
  $("#exclusions-dialog").showModal();if(!state.running)$("#exclusion-path").focus();
}
async function saveExclusions(paths) {
  const data=await api("/api/space/exclusions",{paths});exclusionPaths=data.paths;renderExclusions();
  $("#exclusion-path").value="";await refresh();toast("Исключения сохранены. Применятся при следующем анализе.");
}
$("#exclusions-button").onclick=()=>action(openExclusions);
$("#scan-exclusions").onclick=()=>action(openExclusions);
$("#exclusion-form").onsubmit=e=>{e.preventDefault();const path=$("#exclusion-path").value.trim();if(path)exclusionAction(()=>saveExclusions([...exclusionPaths,path]));};
for(const [id,endpoint]of [["exclude-folder","/api/pick"],["exclude-file","/api/space/pick-file"]])$("#"+id).onclick=()=>exclusionAction(async()=>{const r=await api(endpoint,{});if(r.path){$("#exclusion-path").value=r.path;$("#add-exclusion").focus();}});
$("#stop-scan").onclick=()=>action(async()=>{await api("/api/space/stop",{});toast("Остановка после текущей пачки файлов. Частичный результат останется доступен.");});
$$("[data-close]").forEach(b=>b.onclick=()=>b.closest("dialog").close());
async function loadFiles() {
  const request=++state.fileRequest;
  const q=new URLSearchParams({mode:state.mode,search:state.search,offset:state.offset,category:state.category,folder:state.folder});
  const data=await api(`/api/space/files?${q}`);if(request!==state.fileRequest)return;
  state.files=data.files;state.total=data.total;
  $("#all-files").textContent=state.mode==="all"?"Только от 100 МБ":"Показать все размеры";
  $("#file-list").replaceChildren(...data.files.map(f=>{
    const r=node("div","file-row");const check=node("input");check.type="checkbox";check.checked=state.selected.has(f.id);check.disabled=f.protected||state.running||!!state.info?.exclusions_pending;check.setAttribute("aria-label",`Выбрать ${basename(f.path)}`);check.onchange=()=>{if(check.checked){if(state.selected.size>=500){check.checked=false;toast("Выберите до 500 файлов за одну операцию");return;}state.selected.set(f.id,f);}else state.selected.delete(f.id);selection();};
    const copy=node("div","file-copy");copy.append(node("strong","",basename(f.path)),node("small","",f.path));copy.title=f.path;
    const reveal=node("button","icon-button","↗");reveal.title="Показать в Проводнике";reveal.setAttribute("aria-label",`Показать ${basename(f.path)} в Проводнике`);reveal.onclick=()=>action(()=>api("/api/space/reveal",{id:f.id}));
    const lock=node("span","protected-symbol","◇");lock.title="Системный или служебный файл. Перенос защищён.";
    r.append(f.protected?lock:check,fileIcon(f.path),copy,node("span","file-date",date(f.modified)),node("span","file-size",bytes(f.size)),reveal);return r;
  }));
  if(!data.files.length)$("#file-list").append(empty(state.scan?"Подходящих файлов нет.<br>Измените фильтр или область анализа.":"Сначала проанализируйте диск или папку."));
  $("#file-total").textContent=`Файлов: ${num(data.total)}${state.folder?" · "+basename(state.folder):""}`;
  $("#page-number").textContent=String(Math.floor(state.offset/60)+1).padStart(2,"0");
  $("#previous").disabled=state.offset===0;$("#next").disabled=state.offset+60>=data.total;
  selection();
}
function selection() {const values=[...state.selected.values()];$("#selection-toolbar").hidden=!values.length;$("#selection-summary").textContent=`Выбрано ${num(values.length)} · ${bytes(values.reduce((n,f)=>n+f.size,0))}`;$("#quarantine-selected").disabled=state.running||!!state.info?.exclusions_pending;}
$("#previous").onclick=()=>action(async()=>{state.offset=Math.max(0,state.offset-60);await loadFiles();});
$("#next").onclick=()=>action(async()=>{state.offset+=60;await loadFiles();});
$("#file-search").oninput=()=>{clearTimeout(searchTimer);searchTimer=setTimeout(()=>action(async()=>{state.search=$("#file-search").value;state.offset=0;await loadFiles();}),250);};
$("#file-category").onchange=()=>action(async()=>{state.category=$("#file-category").value;state.offset=0;await loadFiles();});
$("#all-files").onclick=()=>action(async()=>{state.mode=state.mode==="all"?"large":"all";state.offset=0;await loadFiles();});
$("#clear-selected").onclick=()=>{state.selected.clear();action(loadFiles);};
$("#review-temporary").onclick=()=>action(async()=>{state.mode="temporary";state.offset=0;state.folder="";state.category="";state.search="";$("#file-category").value="";$("#file-search").value="";state.selected.clear();await go("large",{preserve:true});});
$("#quarantine-selected").onclick=()=>{
  const files=[...state.selected.values()];if(!files.length)return;
  $("#confirm-summary").textContent=`Выбрано файлов: ${num(files.length)}, общий размер — ${bytes(files.reduce((n,f)=>n+f.size,0))}. Проверьте каждый путь.`;
  $("#confirm-files").replaceChildren(...files.map(f=>{const r=node("div","confirm-file");r.append(node("span","",f.path),node("span","",bytes(f.size)));return r;}));$("#confirm-dialog").showModal();
};
$("#confirm-move").onclick=()=>action(async()=>{
  $("#confirm-move").disabled=true;
  try{toast("Проверяем содержимое и переносим файлы. Это может занять время…");const r=await api("/api/space/trash",{ids:[...state.selected.keys()],scan:state.scan});$("#confirm-dialog").close();state.selected.clear();await refresh();window.CleanerUltraMaintenance?.result("Файлы перенесены в корзину",r);toast(`Перемещено: ${num(r.done)}.${r.errors.length?" Ошибок: "+num(r.errors.length)+". "+r.errors[0].message:" Файлы можно восстановить в корзине Cleaner Ultra."}`);}finally{$("#confirm-move").disabled=false;}
});
const mapView={data:null,selected:null,zoom:1,x:0,y:0,drag:null,moved:false};
const mapIcon=f=>f.kind==="folder"?"folder":({video:"video",photos:"image",audio:"audio",archives:"archive"}[f.category]||"file");
const svg=name=>window.CleanerUltraIcons.create(name);
async function openMap(path) {state.mapPath=path;state.mapOffset=0;mapView.selected=null;fitMap();await loadMap();}
async function loadMap() {
  const request=++state.mapRequest;
  const d=await api(`/api/space/map?${new URLSearchParams({path:state.mapPath,offset:state.mapOffset})}`);if(request!==state.mapRequest)return;
  if(mapView.data?.path!==d.path){mapView.selected=null;fitMap();}
  mapView.data=d;state.mapPath=d.path||"";state.mapOffset=d.offset;
  $("#map-crumbs").replaceChildren(...d.crumbs.flatMap((path,i)=>{const b=node("button",i===d.crumbs.length-1?"current":"",i===0?(basename(path)||path):basename(path));b.title=path;b.onclick=()=>action(()=>openMap(path));return i?[svg("chevron"),b]:[b];}));
  if(!d.crumbs.length)$("#map-crumbs").append(node("span","","Сначала проанализируйте диск или папку"));
  $("#map-size").textContent=bytes(d.bytes);$("#map-stats").textContent=`Файлов: ${num(d.files)} · элементов в папке: ${num(d.total)}`;
  $("#map-up").disabled=!d.parent;$("#map-root").disabled=!d.parent;
  $("#map-up").onclick=()=>action(()=>openMap(d.parent||""));$("#map-root").onclick=()=>action(()=>openMap(d.root));
  const list=$("#folder-list");list.replaceChildren(...d.entries.map(f=>{
    const r=node("div","map-row");r.dataset.path=f.path;
    const b=node("button","map-row-select");b.setAttribute("aria-label",`Выбрать ${basename(f.path)}`);const copy=node("span","map-row-copy");copy.append(node("strong","",basename(f.path)),node("small","",f.kind==="folder"?`Папка · файлов: ${num(f.files)}`:categories[f.category]||"Файл"));b.append(svg(mapIcon(f)),copy);b.onclick=()=>selectMap(f);b.ondblclick=()=>f.kind==="folder"&&action(()=>openMap(f.path));
    const share=node("div","map-share"),bar=node("i");bar.style.width=`${d.bytes?f.bytes/d.bytes*100:0}%`;share.append(bar,node("span","",percent(f.bytes,d.bytes)));
    const open=node("button","icon-button");open.title=f.kind==="folder"?"Открыть папку":"Показать в Проводнике";open.setAttribute("aria-label",`${open.title}: ${basename(f.path)}`);open.append(svg(f.kind==="folder"?"chevron":"arrow"));open.onclick=()=>action(()=>f.kind==="folder"?openMap(f.path):api("/api/space/reveal",{id:f.id}));
    r.append(b,share,node("strong","map-row-size",bytes(f.bytes)),open);return r;
  }));
  if(!d.entries.length)list.append(empty(state.scan?"В этой папке нет файлов в текущем снимке анализа.":"Сначала выберите папку или диск для анализа."));
  $("#folder-total").textContent=`Элементов: ${num(d.total)} · по размеру`;
  $("#map-list-range").textContent=d.total?`${num(d.offset+1)}–${num(Math.min(d.total,d.offset+60))} из ${num(d.total)}`:"Нет элементов";
  $("#map-list-page").textContent=String(Math.floor(d.offset/60)+1).padStart(2,"0");
  $("#folder-previous").disabled=d.offset===0;$("#folder-next").disabled=d.offset+60>=d.total;$("#map-files").disabled=!state.scan;
  $("#map-caption").textContent="Стрелка — раскрыть папку · размер — логический объём";
  await window.CleanerUltraTree?.load(d);renderInspector();
}
function percent(value,total){return total?`${(value/total*100).toLocaleString("ru-RU",{maximumFractionDigits:1})}%`:"0%";}
function selectMap(f){mapView.selected=f;renderInspector();highlightMap();}
function highlightMap(){for(const el of $$(".map-block,.map-row"))el.classList.toggle("selected",el.dataset.path===mapView.selected?.path);}
function renderInspector(){
  const box=$("#map-inspector"),f=mapView.selected,d=mapView.data;box.replaceChildren();
  if(!f){const mark=node("span","inspector-mark");mark.append(svg("cursor"));box.append(mark,node("p","eyebrow","ВЫБЕРИТЕ ЭЛЕМЕНТ"),node("h2","","Что занимает место?"),node("p","","Выберите файл или папку в дереве. Здесь появятся размер, расположение и действия."));const hint=node("div","inspector-hint");hint.append(svg("folder"),node("span","","Стрелка слева раскрывает ветвь. Двойной щелчок также раскрывает папку."));box.append(hint);return;}
  const mark=node("span","inspector-mark");mark.append(svg(f.remainder?"grid":mapIcon(f)));box.append(mark,node("p","eyebrow",f.remainder?"ОСТАЛЬНЫЕ ЭЛЕМЕНТЫ":f.kind==="folder"?"ПАПКА":"ФАЙЛ"),node("h2","",f.remainder?"Мелкие элементы":basename(f.path)),node("strong","inspector-size",bytes(f.bytes)));
  const details=node("dl","inspector-details");for(const [label,value] of [["Доля папки",percent(f.bytes,d.bytes)],[f.remainder?"Элементов":"Файлов",num(f.files)]])details.append(node("dt","",label),node("dd","",value));box.append(details);
  if(f.remainder){box.append(node("p","","Объединены, чтобы сохранить точные пропорции карты. Каждый элемент доступен в списке ниже."));const b=node("button","button secondary","Показать в списке");b.onclick=()=>action(async()=>{document.body.classList.remove("map-focus");$("#map-focus").setAttribute("aria-label","Развернуть карту");state.mapOffset=120;await loadMap();$("#folder-list").scrollIntoView({behavior:"smooth",block:"start"});});box.append(b);return;}
  box.append(node("p","inspector-path",f.path));if(f.protected)box.append(node("p","protected-note","Системный или служебный файл. Очистка защищена."));
  const b=node("button","button primary",f.kind==="folder"?"Открыть папку":"Показать в Проводнике");b.prepend(svg(f.kind==="folder"?"folder":"arrow"));b.onclick=()=>action(()=>f.kind==="folder"?openMap(f.path):api("/api/space/reveal",{id:f.id}));box.append(b);
}
function renderTreemap(){
  const map=$("#treemap"),d=mapView.data;map.replaceChildren();if(!d)return;
  const values=[...d.tiles];if(d.remainder.bytes>0)values.push({path:"__remainder__",bytes:d.remainder.bytes,files:d.remainder.count,remainder:true});
  const viewport=$("#map-viewport"),w=viewport.clientWidth*mapView.zoom,h=viewport.clientHeight*mapView.zoom;
  const tiles=window.CleanerUltraMap.layout(values,w,h);
  if(!tiles.length){map.append(empty(state.scan?"В этой папке нет данных для карты. Откройте другую папку или обновите анализ.":"Карта появится после анализа. Нажмите «Анализировать диск» или выберите папку."));return;}
  tiles.forEach((f,i)=>{const b=node("button",`map-block ${f.kind==="folder"?"folder-block":"file-block"}${f.remainder?" remainder-block":""}`);b.dataset.path=f.path;
    Object.assign(b.style,{left:`${f.x/w*100}%`,top:`${f.y/h*100}%`,width:`${f.w/w*100}%`,height:`${f.h/h*100}%`,"--tile-tone":String(i%4)});
    const label=f.remainder?`Остальные ${num(f.files)} элементов`:basename(f.path);b.title=`${f.remainder?label:f.path} · ${bytes(f.bytes)} · ${percent(f.bytes,d.bytes)}`;b.setAttribute("aria-label",b.title);
    if(f.w>=85&&f.h>=50){const top=node("span","map-block-top");top.append(svg(f.remainder?"grid":mapIcon(f)),node("span","",percent(f.bytes,d.bytes)));b.append(top,node("strong","",label));if(f.h>=75)b.append(node("small","",bytes(f.bytes)));}else b.classList.add("micro-block");
    b.onclick=()=>{if(!mapView.moved)selectMap(f);};b.ondblclick=()=>f.kind==="folder"&&action(()=>openMap(f.path));map.append(b);
  });highlightMap();applyMapTransform();
}
function applyMapTransform(){const v=$("#map-viewport"),z=mapView.zoom;mapView.x=Math.min(0,Math.max(v.clientWidth*(1-z),mapView.x));mapView.y=Math.min(0,Math.max(v.clientHeight*(1-z),mapView.y));Object.assign($("#treemap").style,{transform:`translate(${mapView.x}px,${mapView.y}px)`,width:`${z*100}%`,height:`${z*100}%`});$("#map-zoom-value").textContent=`${Math.round(z*100)}%`;$("#map-zoom-out").disabled=z<=1;$("#map-zoom-in").disabled=z>=4;v.classList.toggle("zoomed",z>1);}
function zoomMap(delta){const v=$("#map-viewport"),old=mapView.zoom,next=Math.min(4,Math.max(1,old+delta));mapView.x=(mapView.x-v.clientWidth/2)*next/old+v.clientWidth/2;mapView.y=(mapView.y-v.clientHeight/2)*next/old+v.clientHeight/2;mapView.zoom=next;applyMapTransform();renderTreemap();}
function fitMap(){mapView.zoom=1;mapView.x=0;mapView.y=0;applyMapTransform();renderTreemap();}
$("#map-zoom-in").onclick=()=>zoomMap(.5);$("#map-zoom-out").onclick=()=>zoomMap(-.5);$("#map-fit").onclick=fitMap;
$("#map-focus").onclick=()=>{const on=document.body.classList.toggle("map-focus");$("#map-focus").setAttribute("aria-label",on?"Свернуть карту":"Развернуть карту");$("#map-focus").title=on?"Свернуть карту":"Развернуть карту";requestAnimationFrame(()=>{fitMap();renderTreemap();});};
window.addEventListener("keydown",e=>{if(e.key==="Escape"&&document.body.classList.contains("map-focus")){$("#map-focus").click();}});
const viewport=$("#map-viewport");
viewport.addEventListener("pointerdown",e=>{mapView.moved=false;if(mapView.zoom<=1||e.button!==0||e.target.closest(".map-zoom"))return;mapView.drag={id:e.pointerId,startX:e.clientX,startY:e.clientY,x:mapView.x,y:mapView.y};});
viewport.addEventListener("pointermove",e=>{const drag=mapView.drag;if(!drag)return;const dx=e.clientX-drag.startX,dy=e.clientY-drag.startY;if(Math.abs(dx)+Math.abs(dy)>5){mapView.moved=true;viewport.setPointerCapture(e.pointerId);}if(mapView.moved){mapView.x=drag.x+dx;mapView.y=drag.y+dy;applyMapTransform();}});
const stopDrag=()=>{mapView.drag=null;setTimeout(()=>{mapView.moved=false;},0);};viewport.addEventListener("pointerup",stopDrag);viewport.addEventListener("pointercancel",stopDrag);
viewport.addEventListener("wheel",e=>{if(!e.ctrlKey)return;e.preventDefault();zoomMap(e.deltaY<0?.5:-.5);},{passive:false});
$("#map-files").onclick=()=>action(async()=>{state.folder=state.mapPath;state.mode="all";state.offset=0;state.search="";state.category="";$("#file-search").value="";$("#file-category").value="";state.selected.clear();await go("large",{preserve:true});});
$("#folder-previous").onclick=()=>action(async()=>{state.mapOffset=Math.max(0,state.mapOffset-60);await loadMap();});
$("#folder-next").onclick=()=>action(async()=>{state.mapOffset+=60;await loadMap();});
new ResizeObserver(()=>{if(state.page==="map"){applyMapTransform();renderTreemap();}}).observe(viewport);
async function loadTrash() {
  const d=await api(`/api/space/trash?offset=${state.trashOffset}`);state.trashOffset=d.offset;
  $("#trash-total").textContent=`Файлов: ${num(d.total)}`;$("#trash-page").textContent=String(Math.floor(d.offset/d.page_size)+1).padStart(2,"0");
  $("#trash-previous").disabled=d.offset===0;$("#trash-next").disabled=d.offset+d.page_size>=d.total;$("#space-trash").replaceChildren(...d.files.map(f=>{const r=node("div","trash-row");const copy=node("div","file-copy");copy.append(node("strong","",basename(f.path)),node("small","",f.path),node("small","",f.state!=="stored"?"Перенос прерван — копия требует проверки":"Сохранено "+date(f.created)));const b=node("button","button secondary","Восстановить ↶");b.onclick=()=>action(async()=>{const result=await api("/api/space/restore",{ids:[f.id]});await refresh();toast(result.errors.length?result.errors[0].message:"Файл восстановлен. Обновите анализ, чтобы увидеть его в списке.");});const show=node("button","button secondary","Показать ↗");show.onclick=()=>action(()=>api("/api/space/reveal-trash",{id:f.id}));r.append(fileIcon(f.path),copy,node("span","file-size",bytes(f.size)),show,b);return r;}));if(!d.files.length)$("#space-trash").append(empty("Корзина пуста.<br>Перемещённые через анализ диска файлы будут здесь."));
}
$("#trash-previous").onclick=()=>action(async()=>{state.trashOffset=Math.max(0,state.trashOffset-60);await loadTrash();});
$("#trash-next").onclick=()=>action(async()=>{state.trashOffset+=60;await loadTrash();});
function updateOptimizerAvailability(p){
  const note=$("#optimizer-scan-note");if(note)note.textContent=p.state==="running"?`Сканирование пространства: ${p.root} · ${num(p.files)} файлов. Проверка фрагментации станет доступна после завершения. Это отдельная операция.`:p.state==="complete"?`Снимок готов: ${p.root} · ${num(p.files)} файлов. Можно проверить фрагментацию этого диска.`:"Сначала нажмите «Сканировать диск». Неполный снимок не используется для оптимизации.";
  for(const b of $$('[data-fragment-drive]')){b.disabled=p.state!=="complete"||p.root?.slice(0,2).toUpperCase()!==b.dataset.fragmentDrive.toUpperCase();b.title=b.disabled?'Нужен завершённый снимок выбранного диска':'';}
  for(const b of $$('[data-disk-scan]'))b.disabled=p.state==='running';
}
function renderOptimizer(drives) {
  $("#optimizer-drives").replaceChildren(...drives.filter(d=>d.optimizable).map(d=>{
    const r=node("div","optimizer-row"),copy=node("div","drive-name");copy.append(node("strong","",`Локальный диск ${d.path.slice(0,2)}`),node("small","",`${bytes(d.free)} свободно из ${bytes(d.total)}`));
    const scan=node("button","button secondary","Сканировать диск");scan.dataset.diskScan=d.path;scan.onclick=()=>action(async()=>{await api('/api/space/scan',{path:d.path});await refresh();toast('Сканирование запущено. Прогресс показан над списком дисков.');});
    const analyze=node("button","button secondary","Фрагментация");analyze.dataset.fragmentDrive=d.path.slice(0,2);analyze.onclick=()=>openOptimize(d,true);
    const defrag=node("button","button primary","Дефрагментация");defrag.dataset.fragmentDrive=d.path.slice(0,2);defrag.onclick=()=>openOptimize(d,false);
    const trim=node("button","button secondary","TRIM SSD");trim.onclick=()=>openTrim(d);r.append(svg('drive'),copy,scan,analyze,defrag,trim);return r;
  }));updateOptimizerAvailability(state.info?.progress||{});
}
function openOptimize(drive,analyze){
  $("#optimize-note").textContent="Для доступа к тому нужны права администратора. Остановка завершает обработку после текущего системного вызова.";
  $("#optimize-title").textContent=`${analyze?"Проверить фрагментацию":"Дефрагментировать"} ${drive.path.slice(0,2)}?`;
  $("#optimize-description").textContent=analyze?"Cleaner Ultra прочитает карту экстентов файлов из завершённого снимка анализа выбранного диска. Содержимое файлов не изменяется.":"Встроенный движок переместит кластеры доступных файлов HDD средствами ядра Windows и сверит SHA-256. SSD, системные, занятые и неподдерживаемые файлы пропускаются.";
  $("#confirm-optimize").onclick=()=>window.CleanerUltraOptimizerStart({drive:drive.path.slice(0,2),analyze});$("#optimize-dialog").showModal();
}
function openTrim(drive){
  $("#optimize-note").textContent="Нужны права администратора. После запуска ReTrim отмена недоступна. Закрытие этого диалога до запуска ничего не изменяет.";
  $("#optimize-title").textContent=`Выполнить TRIM ${drive.path.slice(0,2)}?`;
  $("#optimize-description").textContent="Cleaner Ultra обратится напрямую к Windows Storage API. Windows обработает свободные блоки тома; содержимое существующих файлов не удаляется. После запуска ReTrim нельзя прервать. Дождитесь итогового результата в приложении.";
  $("#confirm-optimize").onclick=()=>window.CleanerUltraOptimizerStart({drive:drive.path.slice(0,2),mode:"trim"});$("#optimize-dialog").showModal();
}
$$("[data-system]").forEach(b=>b.onclick=()=>action(async()=>{await api("/api/space/system",{page:b.dataset.system});toast("Открыт системный инструмент Windows");}));
const desktop=Boolean(window.__CLEANER_ULTRA_DESKTOP__&&window.ipc?.postMessage);
$("#admin-button").onclick=()=>$("#admin-dialog").showModal();
$("#confirm-admin").onclick=()=>{if(desktop){$("#admin-dialog").close();window.ipc.postMessage("elevate");}};
$$("[data-window-command]").forEach(b=>{b.disabled=!desktop;b.onclick=()=>window.ipc?.postMessage(b.dataset.windowCommand);});
if(desktop){$$(".window-dots,.topbar").forEach(r=>r.addEventListener("mousedown",e=>{if(e.button!==0||e.target.closest("button,a,input,select"))return;e.preventDefault();window.ipc.postMessage(e.detail===2?"maximize":"drag");}));for(const d of ["n","s","e","w","ne","nw","se","sw"]){const edge=node("div",`window-resize window-resize-${d}`);edge.onpointerdown=e=>{if(e.button===0)window.ipc.postMessage(`resize:${d}`);};document.body.append(edge);}}
async function poll(){try{const wasRunning=state.running;const p=await api("/api/space/progress");const changed=p.id!==state.scan;progress(p);if(changed||(wasRunning&&!state.running))await refresh();}catch(e){console.warn("Progress unavailable",e);}finally{setTimeout(poll,state.running?1000:8000);}}
async function init(){theme("light",false);try{const session=await api("/api/session");state.token=session.token;state.version=session.version;await refresh();}catch(e){toast(e.message);}poll();}
init();
