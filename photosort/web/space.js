const $ = (s) => document.querySelector(s);
const $$ = (s) => [...document.querySelectorAll(s)];
const names = {overview:"Обзор",map:"Карта диска",large:"Крупные файлы",cleanup:"Очистка",optimize:"Оптимизация дисков",trash:"Корзина"};
const categories = {photos:"Изображения",video:"Видео",audio:"Аудио",archives:"Архивы и образы",documents:"Документы",apps:"Приложения",other:"Другое"};
const headings = {
  overview:["ПРОСТРАНСТВО ДЛЯ ВАЖНОГО","Каждый гигабайт<br>на своём месте<span>.</span>","Узнайте, что занимает диск. Оставьте только нужное."],
  map:["УВИДЕТЬ БОЛЬШЕ","Всё пространство.<br>В одной картине<span>.</span>","От большого к малому. Откройте папку, чтобы заглянуть глубже."],
  large:["ВЕСОМЫЕ ПРИЧИНЫ ДЛЯ ПОРЯДКА","Большие файлы.<br>Большие возможности<span>.</span>","Самое объёмное — в начале списка. Каждый файл под вашим контролем."],
  cleanup:["ОСТАВИТЬ НУЖНОЕ","Свободнее диск.<br>Легче работать<span>.</span>","Проверьте кандидатов и выберите подходящий способ очистки."],
  optimize:["БОЛЬШЕ ПОРЯДКА ВНУТРИ","Диск в хорошей<br>форме<span>.</span>","Дефрагментация HDD и подходящая оптимизация SSD средствами Windows."],
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
  const r=await fetch(path,body===undefined?{}:{method:"POST",headers:{"Content-Type":"application/json","X-PhotoSort-Token":state.token},body:JSON.stringify(body)});
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
    const b=node("button","drive-tab");b.dataset.drive=d.path;b.append(node("span","","▱"),node("span","",`Диск ${d.path.replaceAll("\\","")}`),node("small","",`${bytes(d.free)} свободно`));b.onclick=()=>{state.drive=d.path;renderDrive();};return b;
  }));
  renderDrive();progress(info.progress);
  $("#admin-button").disabled=info.administrator||!desktop||state.running;
  $("#admin-button").textContent=info.administrator?"◇ Права администратора":"◇ Администратор";
  $("#admin-button").title=info.administrator?"Приложение уже запущено с повышенными правами":!desktop?"Повышение прав доступно в настольном приложении":state.running?"Сначала остановите текущий анализ":"Перезапустить с правами администратора";
  $("#large-bytes").textContent=state.scan?bytes(info.large.bytes):"—";
  $("#large-count").textContent=state.scan?`От 100 МБ: ${num(info.large.files)} · без системных`:"Файлы от 100 МБ";
  $("#temp-bytes").textContent=state.scan?bytes(info.temporary.bytes):"—";
  $("#cleanup-temp").textContent=state.scan?bytes(info.temporary.bytes):"—";
  $("#cleanup-large").textContent=state.scan?bytes(info.large.bytes):"—";
  $("#cleanup-count").textContent=num(info.temporary.files);
  $("#trash-count").textContent=num(info.trash.files);
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
  if(!names[page])return;state.page=page;
  if(page==="large"&&!options.preserve){state.mode="large";state.folder="";state.offset=0;state.selected.clear();state.search="";$("#file-search").value="";}
  $$("[data-section]").forEach(s=>s.hidden=s.dataset.section!==page);
  $$("nav [data-page]").forEach(b=>b.classList.toggle("active",b.dataset.page===page));
  $("#crumb").textContent=names[page];
  const [eyebrow,title,description]=headings[page];$("#page-eyebrow").textContent=eyebrow;$("#page-title").innerHTML=title;$("#page-description").textContent=description;
  if(page==="large"){if(state.mode==="temporary"){$("#page-title").textContent="Временное — на проверку.";$("#page-description").textContent="Файлы TEMP старше 7 дней. Ничего не выбрано заранее.";}await loadFiles();}
  if(page==="map")await loadMap();
  if(page==="trash")await loadTrash();
  window.scrollTo({top:0,behavior:"instant"});
}
$$("[data-page]").forEach(b=>b.onclick=()=>action(()=>go(b.dataset.page)));
function openScan(folder=false) {$("#scan-path").value=folder?"":selectedDrive()?.path||"";$("#scan-dialog").showModal();$("#scan-path").focus();}
$("#scan-button").onclick=()=>openScan();$("#choose-folder").onclick=()=>openScan(true);
$("#browse-path").onclick=()=>action(async()=>{const r=await api("/api/pick",{});if(r.path)$("#scan-path").value=r.path;});
$("#scan-form").onsubmit=e=>{e.preventDefault();action(async()=>{await api("/api/space/scan",{path:$("#scan-path").value.trim()});$("#scan-dialog").close();await refresh();});};
$("#stop-scan").onclick=()=>action(async()=>{await api("/api/space/stop",{});toast("Остановка после текущей пачки файлов. Частичный результат останется доступен.");});
$$("[data-close]").forEach(b=>b.onclick=()=>b.closest("dialog").close());
async function loadFiles() {
  const request=++state.fileRequest;
  const q=new URLSearchParams({mode:state.mode,search:state.search,offset:state.offset,category:state.category,folder:state.folder});
  const data=await api(`/api/space/files?${q}`);if(request!==state.fileRequest)return;
  state.files=data.files;state.total=data.total;
  $("#all-files").textContent=state.mode==="all"?"Только от 100 МБ":"Показать все размеры";
  $("#file-list").replaceChildren(...data.files.map(f=>{
    const r=node("div","file-row");const check=node("input");check.type="checkbox";check.checked=state.selected.has(f.id);check.disabled=f.protected||state.running;check.setAttribute("aria-label",`Выбрать ${basename(f.path)}`);check.onchange=()=>{if(check.checked){if(state.selected.size>=500){check.checked=false;toast("Выберите до 500 файлов за одну операцию");return;}state.selected.set(f.id,f);}else state.selected.delete(f.id);selection();};
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
function selection() {const values=[...state.selected.values()];$("#selection-toolbar").hidden=!values.length;$("#selection-summary").textContent=`Выбрано ${num(values.length)} · ${bytes(values.reduce((n,f)=>n+f.size,0))}`;$("#quarantine-selected").disabled=state.running;}
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
  try{toast("Проверяем содержимое и переносим файлы. Это может занять время…");const r=await api("/api/space/trash",{ids:[...state.selected.keys()],scan:state.scan});$("#confirm-dialog").close();state.selected.clear();await refresh();toast(`Перемещено: ${num(r.done)}.${r.errors.length?" Ошибок: "+num(r.errors.length)+". "+r.errors[0].message:" Файлы можно восстановить в корзине PhotoSort."}`);}finally{$("#confirm-move").disabled=false;}
});
async function loadMap() {
  const request=++state.mapRequest;const d=await api(`/api/space/folders?${new URLSearchParams({path:state.mapPath,offset:state.mapOffset})}`);if(request!==state.mapRequest)return;
  state.mapPath=d.path||"";$("#map-path").textContent=d.path||"Сначала запустите анализ";$("#map-up").disabled=!d.parent;$("#map-up").onclick=()=>action(async()=>{state.mapPath=d.parent||"";state.mapOffset=0;await loadMap();});
  $("#folder-list").replaceChildren(...d.folders.map(f=>{const b=node("button","folder-row");b.append(node("span","","▱"),node("strong","",basename(f.path)),node("small","",`Файлов: ${num(f.files)}`),node("span","",bytes(f.bytes)));b.onclick=()=>action(async()=>{state.mapPath=f.path;state.mapOffset=0;await loadMap();});return b;}));
  renderTreemap(d.folders.slice(0,24));
  $("#folder-total").textContent=`Папок: ${num(d.total)} · показано ${num(d.folders.length)}`;
  $("#map-caption").textContent=`Крупнейших папок на карте: ${Math.min(24,d.folders.length)}`;
  $("#folder-previous").disabled=state.mapOffset===0;$("#folder-next").disabled=state.mapOffset+100>=d.total;
  $("#map-files").disabled=!state.scan;
}
function renderTreemap(folders) {
  const map=$("#treemap");map.replaceChildren();
  const values=folders.filter(f=>f.bytes>0);if(!values.length){map.append(empty(state.scan?"В этой папке нет вложенных папок с файлами.<br>Откройте список файлов этой папки.":"Здесь появится карта папок после анализа."));return;}
  const total=values.reduce((n,f)=>n+f.bytes,0);const width=map.clientWidth||800,height=map.clientHeight||370;
  function layout(items,x,y,w,h){if(!items.length)return;if(items.length===1){const f=items[0];const b=node("button","map-block");Object.assign(b.style,{left:`${x/width*100}%`,top:`${y/height*100}%`,width:`${w/width*100}%`,height:`${h/height*100}%`,background:["#2b2b2b","#454545","#676767","#868686","#373737","#5a5a5a"][values.indexOf(f)%6]});b.append(node("strong","",basename(f.path)),node("small","",bytes(f.bytes)));b.title=`${f.path} · ${bytes(f.bytes)}`;b.onclick=()=>action(async()=>{state.mapPath=f.path;state.mapOffset=0;await loadMap();});map.append(b);return;}
    const sum=items.reduce((n,f)=>n+f.bytes,0);let left=0,split=0;while(split<items.length-1&&left<sum/2){left+=items[split].bytes;split++;}const ratio=left/sum;
    if(w>=h){layout(items.slice(0,split),x,y,w*ratio,h);layout(items.slice(split),x+w*ratio,y,w*(1-ratio),h);}else{layout(items.slice(0,split),x,y,w,h*ratio);layout(items.slice(split),x,y+h*ratio,w,h*(1-ratio));}}
  if(total)layout(values,0,0,width,height);
}
$("#map-files").onclick=()=>action(async()=>{state.folder=state.mapPath;state.mode="all";state.offset=0;state.selected.clear();await go("large",{preserve:true});});
$("#folder-previous").onclick=()=>action(async()=>{state.mapOffset=Math.max(0,state.mapOffset-100);await loadMap();});
$("#folder-next").onclick=()=>action(async()=>{state.mapOffset+=100;await loadMap();});
window.addEventListener("resize",()=>{if(state.page==="map")action(loadMap);});
async function loadTrash() {
  const d=await api(`/api/space/trash?offset=${state.trashOffset}`);state.trashOffset=d.offset;
  $("#trash-total").textContent=`Файлов: ${num(d.total)}`;$("#trash-page").textContent=String(Math.floor(d.offset/d.page_size)+1).padStart(2,"0");
  $("#trash-previous").disabled=d.offset===0;$("#trash-next").disabled=d.offset+d.page_size>=d.total;$("#space-trash").replaceChildren(...d.files.map(f=>{const r=node("div","trash-row");const copy=node("div","file-copy");copy.append(node("strong","",basename(f.path)),node("small","",f.path),node("small","",f.state!=="stored"?"Перенос прерван — копия требует проверки":"Сохранено "+date(f.created)));const b=node("button","button secondary","Восстановить ↶");b.onclick=()=>action(async()=>{const result=await api("/api/space/restore",{ids:[f.id]});await refresh();toast(result.errors.length?result.errors[0].message:"Файл восстановлен. Обновите анализ, чтобы увидеть его в списке.");});const show=node("button","button secondary","Показать ↗");show.onclick=()=>action(()=>api("/api/space/reveal-trash",{id:f.id}));r.append(fileIcon(f.path),copy,node("span","file-size",bytes(f.size)),show,b);return r;}));if(!d.files.length)$("#space-trash").append(empty("Корзина пуста.<br>Перемещённые через анализ диска файлы будут здесь."));
}
$("#trash-previous").onclick=()=>action(async()=>{state.trashOffset=Math.max(0,state.trashOffset-60);await loadTrash();});
$("#trash-next").onclick=()=>action(async()=>{state.trashOffset+=60;await loadTrash();});
function renderOptimizer(drives) {
  $("#optimizer-drives").replaceChildren(...drives.filter(d=>d.optimizable).map(d=>{const r=node("div","optimizer-row");const copy=node("div","drive-name");copy.append(node("strong","",`Локальный диск ${d.path.replaceAll("\\","")}`),node("small","",`${bytes(d.free)} свободно из ${bytes(d.total)} · тип носителя определит Windows`));const a=node("button","button secondary","Анализировать");a.onclick=()=>openOptimize(d,true);const b=node("button","button primary","Оптимизировать ↗");b.onclick=()=>openOptimize(d,false);r.append(node("span","","▱"),copy,a,b);return r;}));if(!drives.some(d=>d.optimizable))$("#optimizer-drives").append(empty("Нет доступных локальных дисков для оптимизации."));
}
function openOptimize(drive,analyze){$("#optimize-title").textContent=`${analyze?"Анализировать":"Оптимизировать"} ${drive.path.replaceAll("\\","")}?`;$("#optimize-description").textContent=analyze?"Windows проверит состояние файловой системы и фрагментацию выбранного диска. Анализ не удаляет файлы.":"Для HDD Windows выполнит подходящую дефрагментацию, для SSD выберет оптимизацию по типу носителя. Эта операция может занять продолжительное время.";$("#confirm-optimize").onclick=()=>action(async()=>{const r=await api("/api/space/optimize",{drive:drive.path.slice(0,2),analyze});$("#optimize-dialog").close();toast(r.message);});$("#optimize-dialog").showModal();}
$$("[data-system]").forEach(b=>b.onclick=()=>action(async()=>{await api("/api/space/system",{page:b.dataset.system});toast("Открыт системный инструмент Windows");}));
const desktop=Boolean(window.__PHOTOSORT_DESKTOP__&&window.ipc?.postMessage);
$("#admin-button").onclick=()=>$("#admin-dialog").showModal();
$("#confirm-admin").onclick=()=>{if(desktop){$("#admin-dialog").close();window.ipc.postMessage("elevate");}};
$$("[data-window-command]").forEach(b=>{b.disabled=!desktop;b.onclick=()=>window.ipc?.postMessage(b.dataset.windowCommand);});
if(desktop){$$(".window-dots,.topbar").forEach(r=>r.addEventListener("mousedown",e=>{if(e.button!==0||e.target.closest("button,a,input,select"))return;e.preventDefault();window.ipc.postMessage(e.detail===2?"maximize":"drag");}));for(const d of ["n","s","e","w","ne","nw","se","sw"]){const edge=node("div",`window-resize window-resize-${d}`);edge.onpointerdown=e=>{if(e.button===0)window.ipc.postMessage(`resize:${d}`);};document.body.append(edge);}}
async function poll(){try{const wasRunning=state.running;const p=await api("/api/space/progress");progress(p);if(wasRunning&&!state.running)await refresh();}catch(e){console.warn("Progress unavailable",e);}finally{setTimeout(poll,state.running?1000:8000);}}
async function init(){theme("light",false);try{const session=await api("/api/session");state.token=session.token;await refresh();}catch(e){toast(e.message);}poll();}
init();
