(() => {
  "use strict";
  const paths = {
    bolt:'<path d="m13 2-9 12h7l-1 8 10-13h-7Z"/>',
    activity:'<path d="M2 12h4l3-8 6 16 3-8h4"/>',
    window:'<rect x="5" y="5" width="14" height="14" rx="1"/>',
    'arrow-right':'<path d="M4 12h16m-6-6 6 6-6 6"/>',
    home:'<path d="m3 10 9-7 9 7v10H3Z"/><path d="M9 20v-7h6v7"/>',
    grid:'<rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/>',
    files:'<path d="M8 3h8l4 4v14H8Z"/><path d="M16 3v5h4M4 7v14"/>',
    file:'<path d="M5 3h9l5 5v13H5Z"/><path d="M14 3v5h5"/>',
    folder:'<path d="M3 7V5a1 1 0 0 1 1-1h6l2 3h8a1 1 0 0 1 1 1v11a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V7Z"/>',
    drive:'<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M3 14h18M16 17h2M6 17h.01"/>',
    image:'<rect x="3" y="3" width="18" height="18" rx="3"/><circle cx="8" cy="8" r="1.5"/><path d="m3 17 5-5 4 4 4-6 5 7"/>',
    video:'<rect x="3" y="5" width="18" height="14" rx="3"/><path d="m10 9 5 3-5 3Z"/>',
    archive:'<rect x="4" y="4" width="16" height="17" rx="2"/><path d="M3 4h18V1H3ZM12 7v2m0 2v2m0 2v2"/>',
    audio:'<path d="M9 17V5l11-2v12M9 8l11-2"/><ellipse cx="6" cy="18" rx="3" ry="2"/><ellipse cx="17" cy="16" rx="3" ry="2"/>',
    sparkles:'<path d="m12 3 2.5 6.5L21 12l-6.5 2.5L12 21l-2.5-6.5L3 12l6.5-2.5ZM20 2v4m-2-2h4"/>',
    disc:'<circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="3"/><path d="M12 6a6 6 0 0 1 6 6"/>',
    trash:'<path d="M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7m4-7v7"/>',
    shield:'<path d="m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6Z"/><path d="m8 12 3 3 5-6"/>',
    sun:'<circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1.5 1.5m11 11L19 19M5 19l1.5-1.5m11-11L19 5"/>',
    moon:'<path d="M20 14A9 9 0 0 1 10 3a9 9 0 1 0 10 11Z"/>',
    search:'<circle cx="10.5" cy="10.5" r="7"/><path d="m16 16 5 5"/>',
    plus:'<path d="M12 5v14M5 12h14"/>',
    minus:'<path d="M5 12h14"/>',
    close:'<path d="m6 6 12 12M6 18 18 6"/>',
    expand:'<path d="M8 3H3v5m13-5h5v5M3 16v5h5m13-5v5h-5"/>',
    arrow:'<path d="M5 19 19 5M6 5h13v13"/>',
    up:'<path d="m5 12 7-7 7 7M12 5v15"/>',
    back:'<path d="m12 5-7 7 7 7M5 12h15"/>',
    chevron:'<path d="m9 5 7 7-7 7"/>',
    restore:'<path d="M3 10a9 9 0 1 1 1 8M3 3v7h7"/>',
    clock:'<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
    heart:'<path d="M12 21 3.5 12.5C-2 6 6 0 12 7c6-7 14-1 8.5 5.5Z"/>',
    settings:'<path d="m10 3-1 3-3 1-3-1-1 4 3 2v3l-2 2 3 3 3-1 3 1 1 2 4-1v-3l2-2 3-1-1-4-3-1-1-3Z"/><circle cx="12" cy="12" r="3"/>',
    logo:'<path d="m12 2 10 10-10 10L2 12Z"/><path d="m12 7 5 5-5 5-5-5Z"/>',
    info:'<circle cx="12" cy="12" r="9"/><path d="M12 11v6m0-10v.01"/>',
    cursor:'<path d="m4 3 16 9-7 1-3 7Z"/>',
  };
  const glyphs = {'⌂':'home','▦':'grid','▤':'files','▱':'folder','▣':'drive','▧':'image','✧':'sparkles','◎':'disc','♧':'trash','◉':'shield','☼':'sun','◐':'moon','◇':'shield','⌕':'search','＋':'plus','↗':'arrow','↶':'restore','↑':'up','×':'close','−':'minus','◷':'clock','♡':'heart','⚙':'settings','◈':'logo','▥':'grid','▷':'video'};
  function create(name) {
    const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');
    svg.setAttribute('viewBox','0 0 24 24');svg.setAttribute('fill','none');svg.setAttribute('stroke','currentColor');svg.setAttribute('stroke-width','1.6');svg.setAttribute('stroke-linecap','round');svg.setAttribute('stroke-linejoin','round');svg.setAttribute('aria-hidden','true');svg.setAttribute('focusable','false');svg.classList.add('ps-icon');svg.dataset.icon=name;svg.innerHTML=paths[name]||paths.file;return svg;
  }
  function hydrate(root) {
    if(root.nodeType===1&&root.closest('script,style,svg,textarea,option'))return;
    const texts=[];
    if(root.nodeType===3)texts.push(root);else {const w=document.createTreeWalker(root,NodeFilter.SHOW_TEXT);while(w.nextNode())texts.push(w.currentNode);}
    for(const text of texts){if(!text.parentElement||text.parentElement.closest('script,style,svg,textarea,option,.file-copy,.inspector-path,.map-row-copy,.map-crumbs,.map-block strong,.confirm-file,.photo-caption,.photo-detail,.plan-file-body,[data-plain-text]'))continue;
      const chars=[...text.data],isIcon=(c,i)=>glyphs[c]&&(i===0||/\s/.test(chars[i-1]))&&(i===chars.length-1||/\s/.test(chars[i+1]));if(!chars.some(isIcon)||text.data.includes('\\'))continue;
      const f=document.createDocumentFragment();let buffer='';
      for(const [i,c] of chars.entries()){if(isIcon(c,i)){if(buffer){f.append(buffer);buffer='';}f.append(create(glyphs[c]));}else buffer+=c;}
      if(buffer)f.append(buffer);text.replaceWith(f);
    }
    root.querySelectorAll?.('[data-icon]:not(svg)').forEach(el=>{el.replaceChildren(create(el.dataset.icon));el.removeAttribute('data-icon');});
  }
  window.CleanerUltraIcons={create,hydrate};
  const style=document.createElement('style');style.textContent='.ps-icon{display:inline-block;width:20px;height:20px;flex-shrink:0;vertical-align:middle;pointer-events:none}.window-dots .ps-icon{width:10px;height:10px;opacity:0}.window-dots:hover .ps-icon{opacity:1}';document.head.append(style);
  const observer=new MutationObserver(records=>{observer.disconnect();for(const r of records){if(r.type==='characterData')hydrate(r.target);else for(const n of r.addedNodes)hydrate(n);}observe();});
  function observe(){observer.observe(document.body,{childList:true,subtree:true,characterData:true});}
  hydrate(document.body);observe();
})();
