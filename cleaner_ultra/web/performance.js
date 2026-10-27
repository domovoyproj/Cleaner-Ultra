(()=>{
  const perf={timer:null,rows:[],sort:'memory',previous:null,plans:[]};
  function render(){
    const rows=[...perf.rows].sort((a,b)=>perf.sort==='cpu'?(b.cpu_percent||0)-(a.cpu_percent||0):b.memory_bytes-a.memory_bytes);
    $('#performance-processes').replaceChildren(...rows.slice(0,40).map(r=>{
      const row=node('div','ultra-row'),copy=node('div','ultra-copy');copy.append(node('strong','',r.name),node('small','',`PID ${r.pid}`));row.append(copy,node('span','ultra-metric',bytes(r.memory_bytes)),node('span','ultra-metric',r.cpu_percent===null?'CPU: замер…':`CPU: ${r.cpu_percent.toFixed(1)}%`));return row;
    }));
  }
  async function resources(){
    const d=await api('/api/performance');perf.rows=d.processes;
    $('#performance-cpu').textContent=d.cpu_percent===null?'Замер…':`${d.cpu_percent.toFixed(1)}%`;
    $('#performance-ram').textContent=`${d.memory_percent}%`;
    $('#performance-memory').textContent=`Доступно ${bytes(d.memory_available)} из ${bytes(d.memory_total)}`;
    $('#performance-note').textContent=`Доступных процессов: ${num(d.processes.length)} · защищённых/недоступных: ${num(d.inaccessible_processes)}. CPU считается между замерами, RAM — рабочий набор; общие страницы могут учитываться у нескольких процессов.`;render();
  }
  async function power(){
    const d=await api('/api/power');perf.plans=d.plans;
    $('#power-plans').replaceChildren(...d.plans.map(p=>{const option=node('option','',p.name);option.value=p.id;option.selected=p.id===d.active;return option;}));
    $('#power-active').textContent=d.plans.find(p=>p.id===d.active)?.name||d.active;
    const h=await api('/api/history'),last=h.items.find(r=>['/api/power/apply','/api/power/backup'].includes(r.kind)&&r.result.previous);perf.previous=last?.result.previous||null;
    $('#power-restore').disabled=!perf.previous||!d.plans.some(p=>p.id===perf.previous)||perf.previous===d.active;
  }
  function change(id){
    const plan=perf.plans.find(p=>p.id===id);if(!plan)return;
    window.CleanerUltraMaintenance.confirm('Изменить схему питания?', 'Будет применена существующая схема Windows. Высокая производительность может увеличить расход энергии. Предыдущая схема сохраняется в истории.',[plan.name],async()=>{const r=await api('/api/power/apply',{id,confirmed:true});await power();return r;});
  }
  async function tick(){
    clearTimeout(perf.timer);if(state.page!=='performance')return;
    try{await resources();}catch(e){$('#performance-note').textContent=e.message;}
    if(state.page==='performance')perf.timer=setTimeout(tick,2000);
  }
  $('#performance-sort').onchange=()=>{perf.sort=$('#performance-sort').value;render();};
  $('#performance-refresh').onclick=()=>action(async()=>{await resources();await power();});
  $('#power-apply').onclick=()=>change($('#power-plans').value);$('#power-restore').onclick=()=>change(perf.previous);
  window.CleanerUltraPerformance={open:async()=>{await tick();try{await power();}catch(e){$('#power-active').textContent=e.message;}},close:()=>clearTimeout(perf.timer)};
})();
