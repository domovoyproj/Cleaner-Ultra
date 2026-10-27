const $ = (selector) => document.querySelector(selector);
const desktopWindow = Boolean(
  window.__CLEANER_ULTRA_DESKTOP__ && window.ipc?.postMessage,
);
function windowCommand(command) {
  if (desktopWindow) window.ipc.postMessage(command);
}
document.body.classList.toggle("desktop-window", desktopWindow);
document.querySelectorAll("[data-window-command]").forEach((button) => {
  button.disabled = !desktopWindow;
  button.onclick = () => windowCommand(button.dataset.windowCommand);
});
if (desktopWindow) {
  document.querySelectorAll(".desktop-titlebar, .topbar").forEach((region) => {
    region.addEventListener("mousedown", (event) => {
      if (
        event.button !== 0 ||
        event.target.closest("button, a, input, select")
      )
        return;
      event.preventDefault();
      windowCommand(event.detail === 2 ? "maximize" : "drag");
    });
  });
  for (const direction of ["n", "s", "e", "w", "ne", "nw", "se", "sw"]) {
    const edge = document.createElement("div");
    edge.className = `window-resize window-resize-${direction}`;
    edge.setAttribute("aria-hidden", "true");
    edge.onpointerdown = (event) => {
      if (event.button !== 0) return;
      event.preventDefault();
      windowCommand(`resize:${direction}`);
    };
    document.body.append(edge);
  }
}
const titles = {
  all: "Все файлы",
  photos: "Фотографии",
  videos: "Видео",
  pending: "Не разобраны",
  favorites: "Избранное",
  similar: "Похожие кадры",
  bursts: "Серии",
  duplicates: "Точные дубли",
  trash: "Корзина",
};
const state = {
  view: "all",
  offset: 0,
  photos: [],
  selected: new Set(),
  token: "",
  running: false,
  request: 0,
  busy: false,
  culling: false,
  duplicatePlan: null,
  duplicateSelected: new Set(),
  duplicateVisible: 200,
  scanErrorsRetryAllowed: false,
  reviewRequest: 0,
  reviewUpdatedAt: 0,
  viewerPhotos: [],
  viewerIndex: 0,
  settings: {
    sensitivity: 6,
    cache_mb: 1024,
    preferred_folder: "",
    theme: "light",
    density: "normal",
    ui_state: {},
  },
};
const number = (value) => Number(value || 0).toLocaleString("ru-RU");
const fileWord = (value) => {
  const count = Math.abs(Number(value || 0));
  const lastTwo = count % 100;
  if (lastTwo >= 11 && lastTwo <= 14) return "файлов";
  switch (count % 10) {
    case 1: return "файл";
    case 2:
    case 3:
    case 4: return "файла";
    default: return "файлов";
  }
};
const duration = (seconds) => {
  const total = Math.max(0, Math.round(seconds || 0));
  const hours = Math.floor(total / 3600);
  return `${hours ? hours + ":" : ""}${String(Math.floor(total / 60) % 60).padStart(hours ? 2 : 1, "0")}:${String(total % 60).padStart(2, "0")}`;
};
const size = (value) =>
  value >= 1e9
    ? `${(value / 1e9).toFixed(1)} ГБ`
    : value >= 1e6
      ? `${(value / 1e6).toFixed(1)} МБ`
      : `${Math.max(1, Math.round(value / 1024))} КБ`;
const planSize = (value) => Number(value) > 0 ? size(value) : "0 Б";
let toastTimer,
  searchTimer,
  saveTimer,
  eyeWorker,
  eyeQueue = [],
  zoom = 1,
  pan = { x: 0, y: 0 },
  drag = null,
  pixel = false;
function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}
function toast(message) {
  $("#toast").textContent = message;
  $("#toast").hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => ($("#toast").hidden = true), 6500);
}
async function api(path, body) {
  const response = await fetch(
    path,
    body === undefined
      ? {}
      : {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            "X-CleanerUltra-Token": state.token,
          },
          body: JSON.stringify(body),
        },
  );
  const data = await response.json();
  if (!response.ok) throw Error(data.error || "Ошибка запроса");
  return data;
}
async function action(work) {
  if (state.busy) return;
  state.busy = true;
  document.body.classList.add("busy");
  try {
    return await work();
  } catch (error) {
    toast(error.message);
  } finally {
    state.busy = false;
    document.body.classList.remove("busy");
  }
}
function outcome(result, label) {
  toast(
    `${label}: ${Array.isArray(result.done) ? result.done.length : result.done}${result.errors?.length ? `. Ошибок: ${result.errors.length}. ${result.errors[0].message}` : ""}`,
  );
}
function applyTheme() {
  const theme = state.settings.theme;
  document.body.classList.toggle(
    "theme-dark",
    theme === "dark" ||
      (theme === "system" && matchMedia("(prefers-color-scheme:dark)").matches),
  );
  $("#gallery").classList.toggle("large", state.settings.density === "large");
}
async function status() {
  const info = await api("/api/status");
  state.settings = info.settings;
  const rootFilter = $("#root-filter");
  const previousRoot = rootFilter.value || info.settings.ui_state?.root || "";
  const roots = info.roots || [];
  rootFilter.replaceChildren(new Option("Все папки", ""));
  for (const root of roots) {
    const parts = root.replaceAll("\\", "/").split("/").filter(Boolean);
    const option = new Option(parts.slice(-2).join("/"), root);
    option.title = root;
    rootFilter.add(option);
  }
  rootFilter.value = roots.includes(previousRoot) ? previousRoot : "";
  rootFilter.hidden = roots.length < 2 && !rootFilter.value;
  rootFilter.title = rootFilter.value || "Показывать файлы из папки";
  updateReviewScope();
  applyTheme();
  $("#undo").disabled = !info.undo;
  $("#undo").title = info.undo
    ? `Отменить: ${info.undo} · Ctrl+Z`
    : "Нет действий для отмены";
  $("#codec-note").textContent = info.codecs
    ? "JPEG · PNG · TIFF · WEBP · HEIC · RAW. RAW+JPEG выбираются и экспортируются вместе."
    : "Для HEIC и RAW требуется комплект локальных декодеров из полной сборки.";
  $("#pending-operations").hidden = !info.pending_operations;
  $("#pending-operations").textContent = info.pending_operations
    ? `${number(info.pending_operations)} незавершённых операций переноса требуют проверки. Исходные файлы и копии сохранены.`
    : "";
  progress(info.progress);
  return info;
}
function persist() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(async () => {
    try {
      state.settings.ui_state = {
        view: state.view,
        offset: state.offset,
        sort: $("#sort").value,
        search: $("#search").value,
        root: $("#root-filter").value,
        cullKind: state.cullKind || "similar",
      };
      await api("/api/settings", state.settings);
    } catch (error) {
      toast(`Не удалось сохранить место разбора: ${error.message}`);
    }
  }, 450);
}
function updateSelection() {
  const n = state.selected.size;
  $("#selection-bar").hidden = !n;
  $("#selection-count").textContent = `Выбрано: ${n}`;
  $("#trash-selected").textContent =
    state.view === "trash" ? "Восстановить" : "В корзину";
  $("#compare-selected").disabled = n !== 2;
  $("#favorite-selected").textContent =
    state.view === "favorites" ? "♡ Убрать из избранного" : "♡ В избранное";
  document.querySelectorAll(".photo-card").forEach((card) => {
    const selected = state.selected.has(Number(card.dataset.id));
    card.classList.toggle("selected", selected);
    card.querySelector("input").checked = selected;
  });
}
function visibleAssets(photos) {
  const groups = new Map();
  for (const p of photos) {
    const key = p.asset_group || p.path;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(p);
  }
  const result = [];
  for (const group of groups.values()) {
    const raw = group.some((p) =>
      /\.(cr2|cr3|nef|nrw|arw|dng|raf|rw2|orf|pef|srw|raw)$/i.test(p.path),
    );
    if (raw && group.length > 1) {
      const p = group.find((p) => /\.jpe?g$/i.test(p.path)) || group[0];
      result.push({ ...p, pairCount: group.length });
    } else result.push(...group);
  }
  return result;
}
function card(photo, index) {
  const item = element("article", "photo-card");
  item.dataset.id = photo.id;
  const frame = element("div", "photo-image"),
    open = element("button", "photo-open");
  open.title = `Открыть ${photo.name}`;
  open.setAttribute("aria-label", open.title);
  const image = element("img");
  image.src = `/media/${photo.id}`;
  image.alt = photo.name;
  image.loading = "lazy";
  open.append(image);
  if (photo.media_kind === "video") {
    frame.append(element("span", "video-mark", "▶"));
    frame.append(element("span", "video-duration", duration(photo.duration)));
  }
  open.onclick = () => {
    state.culling = false;
    state.viewerPhotos = state.displayPhotos;
    showPhotos([photo], index);
  };
  frame.append(open);
  const check = element("input", "photo-select");
  check.type = "checkbox";
  check.setAttribute("aria-label", `Выбрать ${photo.name}`);
  check.onchange = () => {
    check.checked
      ? state.selected.add(photo.id)
      : state.selected.delete(photo.id);
    updateSelection();
  };
  frame.append(check);
  const heart = element("button", "photo-heart", photo.favorite ? "♥" : "♡");
  heart.title = photo.favorite ? "Убрать из избранного" : "В избранное";
  heart.setAttribute("aria-label", heart.title);
  heart.onclick = () =>
    action(async () => {
      await api("/api/favorite", { ids: [photo.id], value: !photo.favorite });
      await load();
      await status();
    });
  frame.append(heart);
  const badge = photo.pairCount
    ? `RAW + JPEG · ${photo.pairCount}`
    : {
        duplicates: `Дубль · ${photo.hash.slice(0, 6)}`,
        similar: `Группа ${photo.similar}`,
        bursts: `Серия ${photo.burst}`,
        trash: "В корзине",
      }[state.view];
  if (badge) frame.append(element("span", "photo-badge", badge));
  const caption = element("div", "photo-caption");
  caption.append(
    element("strong", "", photo.name),
    element("span", "", size(photo.size)),
  );
  const date = photo.captured
    ? new Date(photo.captured * 1000).toLocaleDateString("ru-RU")
    : "Без даты съёмки";
  item.append(
    frame,
    caption,
    element(
      "div",
      "photo-detail",
      `${photo.width} × ${photo.height}${photo.media_kind === "video" ? ` · ${photo.video_codec.toUpperCase()} · ${Math.round(photo.fps * 10) / 10} fps` : ""} · ${date}`,
    ),
  );
  const hints = [
    photo.quality_hint,
    photo.faces > 0 && photo.eye_score > 0.65 ? "Возможно, закрыты глаза" : "",
  ].filter(Boolean);
  if (hints.length)
    item.append(element("div", "quality-hint", hints.join(" · ")));
  if (photo.decision !== "pending")
    item.append(
      element(
        "span",
        "decision-badge",
        { keep: "✓ Оставить", reject: "× Отклонён", skip: "→ Просмотрен" }[
          photo.decision
        ] || "",
      ),
    );
  return item;
}
async function load() {
  const request = ++state.request;
  try {
    const query = new URLSearchParams({
      view: state.view,
      offset: state.offset,
      search: $("#search").value,
      sort: $("#sort").value,
      root: $("#root-filter").value,
    });
    const data = await api(`/api/photos?${query}`);
    if (request !== state.request) return;
    if (!data.photos.length && state.offset > 0) {
      state.offset = Math.max(0, state.offset - 80);
      return load();
    }
    state.photos = data.photos;
    state.displayPhotos = visibleAssets(data.photos);
    for (const key of Object.keys(titles))
      $(`#count-${key}`).textContent = number(data.counts[key]);
    $("#stat-total").replaceChildren(
      document.createTextNode(number(data.counts.all) + " "),
      element("small", "", fileWord(data.counts.all)),
    );
    $("#stat-similar").textContent = number(data.counts.similar);
    $("#stat-duplicates").textContent = number(data.counts.duplicates);
    $("#stat-favorites").replaceChildren(
      document.createTextNode(number(data.counts.favorites) + " "),
      element("span", "", "♡"),
    );
    $("#stat-bytes").textContent = `${size(data.counts.bytes)} на диске`;
    $("#total-label").textContent = `${number(data.total)} ${fileWord(data.total)}`;
    $("#gallery").replaceChildren(...state.displayPhotos.map(card));
    $("#empty").hidden = data.total !== 0;
    const first =
      state.view === "all" && !$("#search").value && !data.counts.all;
    $("#empty-title").textContent = first
      ? "Всё начинается с одной папки"
      : state.view === "trash"
        ? "Корзина пуста"
        : "Здесь пока нет файлов";
    $("#empty-description").textContent = first
      ? "Добавьте фотографии и видео. CleanerUltra найдёт дубли и похожие файлы и поможет оставить самое ценное."
      : state.view === "trash"
        ? "Перемещённые сюда файлы можно восстановить в исходную папку."
        : "Попробуйте другой поиск или добавьте файлы в библиотеку.";
    $("#empty-import").hidden = !first;
    $("#previous").disabled = state.offset === 0;
    $("#next").disabled = state.offset + 80 >= data.total;
    $("#page-number").textContent = String(
      Math.floor(state.offset / 80) + 1,
    ).padStart(2, "0");
    $("#footer-text").textContent = data.total
      ? `${state.offset + 1}–${Math.min(state.offset + 80, data.total)} из ${number(data.total)}`
      : "Порядок начинается здесь";
    $("#review-progress").textContent = data.counts.all
      ? `Осталось разобрать: ${number(data.counts.pending)}`
      : "";
    $("#quick-review-count").textContent = data.counts.all
      ? `${number(data.counts.similar)} похожих · ${number(data.counts.duplicates)} точных дублей`
      : "Похожие группы появятся после сканирования";
    progress(data.progress);
    updateSelection();
    if (data.counts.similar && !data.progress.running) refreshQuickReview();
    else if (!data.counts.similar) clearQuickReview();
  } catch (error) {
    toast(`Не удалось загрузить библиотеку: ${error.message}`);
  }
}
function progress(data) {
  state.running = data.running;
  $("#scan-status").hidden = !data.running && !data.paused && !data.errors;
  $("#scan-text").textContent =
    `${data.paused ? "Пауза · " : data.running ? "Сканирование · " : "Готово · "}${number(data.done)}${data.total ? ` из ${number(data.total)}` : ""}`;
  $("#scan-detail").textContent =
    `${data.message}${data.errors ? ` · ошибок: ${data.errors}` : ""}`;
  const percent = data.total ? Math.min(100, Math.round((100 * data.done) / data.total)) : 0;
  $("#scan-progress-fill").style.width = `${percent}%`;
  $("#scan-percent").textContent = `${percent}%`;
  $(".scan-progress-track").setAttribute("aria-valuenow", String(percent));
  $("#scan-errors-button").hidden = !data.errors;
  $("#scan-errors-button").textContent = `${number(data.errors)} ${data.errors === 1 ? "ошибка" : "ошибок"}`;
  $("#stop-scan").hidden = !data.running;
  $("#resume-scan").hidden = !data.paused;
  $("#scan-status").classList.toggle("paused", data.paused);
}
function clearQuickReview(message = "Добавьте папку, чтобы найти похожие кадры и серии.") {
  state.reviewRequest++;
  $("#quick-review-items").replaceChildren();
  $("#quick-review-hint").textContent = message;
  $("#quick-group-progress").textContent = "";
  $("#quick-review-open").disabled = true;
}
function updateReviewScope() {
  const root = $("#root-filter").value;
  const short = root.replaceAll("\\", "/").split("/").filter(Boolean).slice(-2).join("/");
  $("#review-scope").textContent = root ? `Область: ${short}` : "Область: все папки";
  $("#review-scope").title = root;
}
async function refreshQuickReview(force = false) {
  if (!force && Date.now() - state.reviewUpdatedAt < 8000) return;
  state.reviewUpdatedAt = Date.now();
  const request = ++state.reviewRequest;
  const root = $("#root-filter").value;
  try {
    const query = new URLSearchParams({ kind: "similar", root });
    const group = await api(`/api/group?${query}`);
    if (request !== state.reviewRequest || root !== $("#root-filter").value) return;
    const photos = visibleAssets(group.photos || []).slice(0, 2);
    $("#quick-review-items").replaceChildren(...photos.map((photo) => {
      const item = element("div", "quick-review-item");
      const image = element("img");
      image.src = `/media/${photo.id}`;
      image.alt = photo.name;
      image.loading = "lazy";
      item.append(image, element("strong", "", photo.name), element("small", "", size(photo.size)));
      return item;
    }));
    $("#quick-group-progress").textContent = group.total && group.remaining
      ? `${number(group.total - group.remaining + 1)} из ${number(group.total)}` : "";
    $("#quick-review-hint").textContent = photos.length
      ? "Сравните всю группу и выберите лучший кадр. Исходные файлы остаются на месте."
      : "Все найденные похожие группы уже просмотрены.";
    $("#quick-review-open").disabled = !photos.length;
  } catch (_) {
    if (request === state.reviewRequest)
      clearQuickReview("Не удалось загрузить группу. Попробуйте открыть разбор ещё раз.");
  }
}
function setView(viewName, save = true) {
  state.view = viewName;
  state.offset = 0;
  state.selected.clear();
  document
    .querySelectorAll("[data-view]")
    .forEach((b) => b.classList.toggle("active", b.dataset.view === viewName));
  $("#page-title").replaceChildren(
    document.createTextNode(titles[viewName]),
    element("span", "title-dot", "."),
  );
  $("#crumb").textContent = titles[viewName];
  $("#collection-name").textContent =
    viewName === "all" ? "Коллекция" : titles[viewName];
  const hints = {
    similar:
      "Фото сравниваются по отпечаткам изображения, видео — по трём кадрам и длительности. Сходство — подсказка: проверяйте содержимое перед выбором.",
    videos:
      "Видео обрабатывается локально. Если кодек не поддерживается проигрывателем, подготовьте совместимое превью; оригинал останется без изменений.",
    pending:
      "Здесь фото и видео без вашего решения. Нажмите P, чтобы оставить, X — чтобы отклонить, или S — чтобы пропустить.",
    bursts:
      "Кадры из одной папки с интервалом до 8 секунд по EXIF. RAW+JPEG выбираются вместе.",
    duplicates:
      "Точные дубли совпадают по SHA-256. «Разобрать дубли» предложит сохраняемые копии с учётом избранного и предпочтительной папки.",
    trash:
      "Выделите кадры для восстановления. Автоматической очистки нет. Ctrl+Z отменяет последнее действие.",
  };
  $("#view-hint").hidden = !hints[viewName];
  $("#view-hint").textContent = hints[viewName] || "";
  load();
  if (save) persist();
}
function applyZoom(stage = null) {
  for (const node of document.querySelectorAll(".zoom-stage img")) {
    if (stage && !$("#sync-zoom").checked && node.parentElement !== stage)
      continue;
    const magnification = pixel
      ? node.naturalWidth / Math.max(node.clientWidth, 1)
      : zoom;
    node.style.transform = `translate(${pan.x * 100}%,${pan.y * 100}%) scale(${magnification})`;
  }
  $("#zoom-label").textContent = pixel ? "1:1" : `${Math.round(zoom * 100)}%`;
}
function resetZoom() {
  zoom = 1;
  pixel = false;
  pan = { x: 0, y: 0 };
  $("#zoom").value = "100";
  applyZoom();
}
function stageFor(photo) {
  if (photo.media_kind === "video") return videoFigure(photo);
  const figure = element("figure"),
    stage = element("div", "zoom-stage"),
    image = element("img");
  image.src = `/media/${photo.id}?full=1`;
  image.alt = photo.name;
  image.draggable = false;
  image.onload = () => applyZoom();
  stage.append(image);
  stage.onwheel = (event) => {
    event.preventDefault();
    pixel = false;
    zoom = Math.max(1, Math.min(5, zoom * (event.deltaY < 0 ? 1.13 : 0.88)));
    $("#zoom").value = String(Math.round(zoom * 100));
    applyZoom(stage);
  };
  stage.onpointerdown = (event) => {
    drag = { x: event.clientX, y: event.clientY, pan: { ...pan }, stage };
    stage.setPointerCapture(event.pointerId);
  };
  stage.onpointermove = (event) => {
    if (drag?.stage !== stage) return;
    pan = {
      x: Math.max(
        -2,
        Math.min(2, drag.pan.x + (event.clientX - drag.x) / stage.clientWidth),
      ),
      y: Math.max(
        -2,
        Math.min(2, drag.pan.y + (event.clientY - drag.y) / stage.clientHeight),
      ),
    };
    applyZoom(stage);
  };
  stage.onpointerup = () => (drag = null);
  stage.onpointercancel = () => (drag = null);
  figure.append(
    stage,
    element(
      "figcaption",
      "",
      `${photo.name} · ${photo.width} × ${photo.height} · резкость ${Math.round(photo.sharpness)}${photo.quality_hint ? " · " + photo.quality_hint : ""}${photo.faces > 0 && photo.eye_score > 0.65 ? " · Возможно, закрыты глаза" : ""}`,
    ),
  );
  if (state.comparing) {
    const button = element("button", "button secondary", "♡ Оставить этот");
    button.onclick = () =>
      action(async () => {
        await api("/api/decision", { ids: [photo.id], decision: "keep" });
        await load();
        await status();
        toast(`Сохранён выбор: ${photo.name}`);
      });
    figure.append(button);
  }
  return figure;
}
function stopVideos() {
  for (const player of document.querySelectorAll("#viewer video")) {
    player.pause();
    player.removeAttribute("src");
    player.load();
  }
}
function updateVideoSamples() {
  const players = [...document.querySelectorAll("#viewer video")];
  const ready = !$("#video-toolbar").hidden && players.length === 2 &&
    players.every((player) => Number.isFinite(player.duration) && player.duration > 0);
  $("#video-samples").querySelectorAll("button").forEach((button) => button.disabled = !ready);
}
$("#video-samples").onclick = (event) => {
  const button = event.target.closest("button[data-scene]");
  if (!button || button.disabled) return;
  const fraction = Number(button.dataset.scene);
  for (const player of document.querySelectorAll("#viewer video")) {
    player.pause();
    player.currentTime = Math.min(player.duration, Math.max(0, player.duration * fraction));
  }
};
function videoFigure(photo) {
  const figure = element("figure", "video-figure");
  const player = element("video", "video-player");
  player.controls = true;
  player.preload = "metadata";
  player.playsInline = true;
  player.tabIndex = 0;
  player.setAttribute("aria-label", `Воспроизвести ${photo.name}`);
  player.poster = `/media/${photo.id}`;
  player.src = `/video/${photo.id}`;
  player.onloadedmetadata = updateVideoSamples;
  player.ondurationchange = updateVideoSamples;
  const message = element("p", "video-status", "");
  message.setAttribute("role", "status");
  player.onerror = () => {
    if (figure.isConnected && $("#viewer").open)
      message.textContent =
        "Этот кодек не воспроизводится напрямую. Подготовьте совместимое превью ниже.";
  };
  const others = () =>
    $("#sync-video").checked && !$("#video-toolbar").hidden
      ? [...document.querySelectorAll("#viewer video")].filter(
          (other) => other !== player,
        )
      : [];
  player.onplay = () => {
    for (const other of others())
      if (other.paused) other.play().catch(() => {});
  };
  player.onpause = () => {
    for (const other of others()) if (!other.paused) other.pause();
  };
  player.onseeking = () => {
    if (!Number.isFinite(player.duration) || player.duration <= 0) return;
    for (const other of others()) {
      const time = (player.currentTime / player.duration) * other.duration;
      if (Number.isFinite(time) && Math.abs(other.currentTime - time) > 0.15)
        other.currentTime = time;
    }
  };
  const compatible = element(
    "button",
    "button secondary",
    "Подготовить совместимое превью",
  );
  compatible.title =
    "Создать временный WebM для проигрывателя. Оригинал не изменяется.";
  compatible.onclick = () =>
    action(async () => {
      compatible.disabled = true;
      message.textContent =
        "Готовим видео локально. Для длинного ролика это может занять несколько минут…";
      try {
        const result = await api("/api/video-preview", { id: photo.id });
        if (!figure.isConnected || !$("#viewer").open) return;
        player.pause();
        player.src = result.url;
        player.load();
        message.textContent =
          "Совместимое превью готово. Нажмите ▶ для воспроизведения.";
      } catch (error) {
        if (figure.isConnected) message.textContent = error.message;
      } finally {
        compatible.disabled = false;
      }
    });
  figure.append(
    player,
    element(
      "figcaption",
      "",
      `${photo.name} · ${duration(photo.duration)} · ${photo.width} × ${photo.height} · ${photo.video_codec.toUpperCase()} · ${Math.round(photo.fps * 10) / 10} fps`,
    ),
    message,
    compatible,
  );
  if (state.comparing) {
    const keep = element("button", "button secondary", "♡ Оставить этот");
    keep.onclick = () =>
      action(async () => {
        await api("/api/decision", { ids: [photo.id], decision: "keep" });
        await load();
        await status();
        toast(`Сохранён выбор: ${photo.name}`);
      });
    figure.append(keep);
  }
  return figure;
}
$("#viewer").addEventListener("close", stopVideos);
function showPhotos(photos, index = 0) {
  stopVideos();
  state.viewerIndex = index;
  state.comparing = photos.length === 2;
  state.openPhotos = photos;
  $("#viewer-title").textContent = state.culling
    ? `Выбор лучшего · ${photos[0].name}`
    : state.comparing
      ? "Сравнение кадров"
      : photos[0].name;
  $("#viewer-images").replaceChildren(...photos.map(stageFor));
  $(".zoom-toolbar").hidden = photos.some(
    (photo) => photo.media_kind === "video",
  );
  $("#video-toolbar").hidden =
    photos.length !== 2 || photos.some((photo) => photo.media_kind !== "video");
  updateVideoSamples();
  $("#viewer-favorite").hidden = state.comparing || state.culling;
  $("#viewer-favorite").textContent = photos[0].favorite
    ? "♥ В избранном"
    : "♡ В избранное";
  $("#viewer-prev").disabled = state.comparing || index === 0;
  $("#viewer-next").disabled =
    state.comparing || index >= state.viewerPhotos.length - 1;
  $("#cull-controls").hidden = !state.culling;
  $("#filmstrip").hidden = !state.culling;
  resetZoom();
  if (state.culling) {
    $("#filmstrip").replaceChildren(
      ...state.viewerPhotos.map((p, i) => {
        const button = element(
          "button",
          i === index ? "film-frame selected" : "film-frame",
        );
        const img = element("img");
        img.src = `/media/${p.id}`;
        img.alt = p.name;
        button.title = p.name;
        button.append(img, element("span", "", String(i + 1)));
        button.onclick = () => showPhotos([p], i);
        return button;
      }),
    );
    $("#group-progress").textContent =
      `Групп осталось: ${state.group.remaining} из ${state.group.total}`;
  }
  if (!$("#viewer").open) $("#viewer").showModal();
}
async function nextGroup() {
  const query = new URLSearchParams({
    kind: state.cullKind,
    root: $("#root-filter").value,
  });
  const data = await api(`/api/group?${query}`);
  state.group = data;
  if (!data.photos.length) {
    $("#viewer").close();
    state.culling = false;
    toast("Все группы просмотрены. Решения сохранены.");
    return;
  }
  state.culling = true;
  state.viewerPhotos = visibleAssets(data.photos);
  showPhotos([state.viewerPhotos[0]], 0);
  persist();
}
async function choose(skip = false) {
  return action(async () => {
    const result = await api("/api/choose", {
      ids: state.group.photos.map((p) => p.id),
      winner: skip ? null : state.viewerPhotos[state.viewerIndex].id,
      trash_others: $("#trash-others").checked,
    });
    if (result.errors?.length) outcome(result, "Выбор сохранён");
    await nextGroup();
    await load();
    await status();
    refreshQuickReview(true);
  });
}
async function decide(decision) {
  const p = state.viewerPhotos[state.viewerIndex];
  return action(async () => {
    await api("/api/decision", {
      ids: $("#viewer").open ? [p.id] : [...state.selected],
      decision,
    });
    if ($("#viewer").open && state.viewerIndex < state.viewerPhotos.length - 1)
      showPhotos(
        [state.viewerPhotos[state.viewerIndex + 1]],
        state.viewerIndex + 1,
      );
    state.selected.clear();
    await load();
    await status();
  });
}
async function undo() {
  return action(async () => {
    const result = await api("/api/undo", {});
    outcome(result, "Отменено");
    await load();
    await status();
    if (state.culling) await nextGroup();
  });
}
async function doMove(restore) {
  return action(async () => {
    $("#confirm-dialog").close();
    const result = await api(restore ? "/api/restore" : "/api/trash", {
      ids: [...state.selected],
    });
    state.selected.clear();
    await load();
    await status();
    outcome(result, restore ? "Восстановлено" : "В корзине");
  });
}
async function pick(input) {
  return action(async () => {
    const result = await api("/api/pick", {});
    if (result.path) $(input).value = result.path;
  });
}
async function analyzeEyes() {
  if (eyeQueue.length) {
    toast("Проверка лиц уже выполняется");
    return;
  }
  eyeQueue = (
    state.selected.size
      ? state.photos.filter((p) => state.selected.has(p.id))
      : state.displayPhotos
  )
    .filter((p) => p.media_kind !== "video")
    .map((p) => p.id);
  if (!eyeQueue.length) {
    toast("Выберите фотографии или откройте страницу с кадрами");
    return;
  }
  if (!eyeWorker) {
    eyeWorker = new Worker("/face-worker.js");
    eyeWorker.onmessage = async ({ data }) => {
      if (data.error) {
        eyeQueue = [];
        $("#check-eyes").disabled = false;
        toast(`Проверка лиц недоступна: ${data.error}`);
        return;
      }
      try {
        await api("/api/eyes", {
          id: data.id,
          faces: data.faces,
          score: data.score,
        });
        eyeQueue.shift();
        if (eyeQueue.length) {
          $("#check-eyes").textContent = `Проверка лиц · ${eyeQueue.length}`;
          eyeWorker.postMessage({ id: eyeQueue[0] });
        } else {
          $("#check-eyes").disabled = false;
          $("#check-eyes").textContent = "Проверить лица";
          await load();
          toast(
            "Локальная проверка лиц завершена. Результаты — подсказки, проверьте их визуально.",
          );
        }
      } catch (error) {
        eyeQueue = [];
        $("#check-eyes").disabled = false;
        toast(error.message);
      }
    };
    eyeWorker.onerror = () => {
      eyeQueue = [];
      $("#check-eyes").disabled = false;
      toast("Не удалось запустить локальную проверку лиц");
    };
  }
  $("#check-eyes").disabled = true;
  eyeWorker.postMessage({ id: eyeQueue[0] });
}
$("#navigation").onclick = (event) => {
  const button = event.target.closest("[data-view]");
  if (button) setView(button.dataset.view);
};
$(".quick-tabs").onclick = (event) => {
  const button = event.target.closest("[data-view]");
  if (button) setView(button.dataset.view);
};
$("#quick-review-open").onclick = () =>
  action(async () => {
    state.cullKind = "similar";
    await nextGroup();
  });
$("#quick-duplicates-open").onclick = () => $("#duplicates-plan").click();
for (const selector of ["#import", "#empty-import"])
  $(selector).onclick = () => $("#import-dialog").showModal();
for (const selector of ["#help", "#help-top"])
  $(selector).onclick = () => $("#help-dialog").showModal();
document.querySelectorAll(".close-dialog").forEach(
  (button) =>
    (button.onclick = () => {
      const dialog = button.closest("dialog");
      dialog.close();
      if (dialog.id === "viewer") state.culling = false;
    }),
);
$("#browse").onclick = () => pick("#folder-path");
$("#browse-export").onclick = () => pick("#export-path");
$("#import-form").onsubmit = (event) => {
  event.preventDefault();
  action(async () => {
    await api("/api/scan", { path: $("#folder-path").value.trim() });
    $("#import-dialog").close();
    state.running = true;
    await load();
  });
};
$("#stop-scan").onclick = () =>
  action(async () => {
    await api("/api/pause", {});
    toast("Пауза после текущей группы файлов…");
  });
$("#resume-scan").onclick = () =>
  action(async () => {
    await api("/api/resume", {});
    state.running = true;
    await load();
  });
async function loadScanErrors(reset = false) {
    const list = $("#scan-errors-list");
    const offset = reset ? 0 : list.children.length;
    const data = await api(`/api/scan/errors?limit=500&offset=${offset}`);
    state.scanErrorsRetryAllowed = Boolean(data.retry_allowed);
    const errors = data.errors || [];
    if (reset) list.replaceChildren();
    list.append(...errors.map((entry) => {
      const row = element("label", "scan-error-row");
      const check = element("input");
      check.type = "checkbox";
      check.value = entry.path;
      check.onchange = () => {
        $("#scan-errors-retry").disabled = state.running || !state.scanErrorsRetryAllowed || !$("#scan-errors-list input:checked");
      };
      const body = element("span");
      body.append(element("strong", "", entry.path), element("small", "", entry.error || "Неизвестная ошибка"));
      row.append(check, body);
      return row;
    }));
    const shown = list.children.length;
    const summary = data.total
      ? `${number(data.total)} файлов не удалось обработать. Показано ${number(shown)}. Выберите нужные и повторите после окончания сканирования.`
      : "Ошибок отдельных файлов нет.";
    const unlisted = data.unlisted_errors
      ? ` Ещё ${number(data.unlisted_errors)} ошибок обхода папок или самого сканирования. Проверьте доступ к папке и запустите импорт снова.`
      : "";
    const paused = data.job && !data.retry_allowed
      ? " Сначала завершите текущее сканирование, затем повторите выбранные файлы."
      : "";
    $("#scan-errors-summary").textContent = summary + unlisted + paused;
    $("#scan-errors-more").hidden = shown >= data.total || !errors.length;
    $("#scan-errors-select-all").disabled = !shown;
    $("#scan-errors-retry").disabled = state.running || !state.scanErrorsRetryAllowed || !list.querySelector("input:checked");
}
$("#scan-errors-button").onclick = () =>
  action(async () => {
    await loadScanErrors(true);
    $("#scan-errors-dialog").showModal();
  });
$("#scan-errors-more").onclick = () => action(() => loadScanErrors());
$("#scan-errors-select-all").onclick = () => {
  $("#scan-errors-list").querySelectorAll("input").forEach((input) => input.checked = true);
  $("#scan-errors-retry").disabled = state.running || !state.scanErrorsRetryAllowed || !$("#scan-errors-list input:checked");
};
$("#scan-errors-retry").onclick = () =>
  action(async () => {
    const paths = [...$("#scan-errors-list").querySelectorAll("input:checked")].map((input) => input.value);
    if (!paths.length) return;
    if (JSON.stringify({ paths }).length > 3_500_000) {
      toast("Слишком много файлов для одного повтора. Выберите меньшую группу.");
      return;
    }
    await api("/api/scan/retry", { paths });
    $("#scan-errors-dialog").close();
    state.running = true;
    await status();
    await load();
    toast(`Повторное сканирование ${number(paths.length)} файлов запущено`);
  });
$("#search").oninput = () => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => {
    state.offset = 0;
    state.selected.clear();
    load();
    persist();
  }, 220);
};
$("#sort").onchange = () => {
  state.offset = 0;
  state.selected.clear();
  load();
  persist();
};
$("#root-filter").onchange = () => {
  state.offset = 0;
  state.selected.clear();
  $("#root-filter").title = $("#root-filter").value || "Показывать файлы из папки";
  updateReviewScope();
  load();
  refreshQuickReview(true);
  persist();
};
$("#density").onclick = () => {
  state.settings.density =
    state.settings.density === "large" ? "normal" : "large";
  applyTheme();
  persist();
};
$("#previous").onclick = () => {
  state.offset = Math.max(0, state.offset - 80);
  state.selected.clear();
  load();
  persist();
};
$("#next").onclick = () => {
  state.offset += 80;
  state.selected.clear();
  load();
  persist();
};
$("#clear-selection").onclick = () => {
  state.selected.clear();
  updateSelection();
};
$("#select-page").onclick = () => {
  state.displayPhotos.forEach((p) => state.selected.add(p.id));
  updateSelection();
};
$("#favorite-selected").onclick = () =>
  action(async () => {
    await api("/api/favorite", {
      ids: [...state.selected],
      value: state.view !== "favorites",
    });
    state.selected.clear();
    await load();
    await status();
  });
$("#keep-selected").onclick = () => decide("keep");
$("#reject-selected").onclick = () => decide("reject");
$("#compare-selected").onclick = () => {
  state.culling = false;
  state.viewerPhotos = state.displayPhotos;
  showPhotos(state.displayPhotos.filter((p) => state.selected.has(p.id)));
};
$("#trash-selected").onclick = () => {
  if (state.view === "trash") return doMove(true);
  $("#confirm-text").textContent =
    `Выбрано: ${state.selected.size}. Связанные RAW+JPEG будут перемещены вместе. Всё можно восстановить из корзины или отменить через Ctrl+Z.`;
  $("#confirm-dialog").showModal();
};
$("#confirm-trash").onclick = () => doMove(false);
$("#cancel-trash").onclick = () => $("#confirm-dialog").close();
$("#viewer-prev").onclick = () => {
  if (state.viewerIndex > 0)
    showPhotos(
      [state.viewerPhotos[state.viewerIndex - 1]],
      state.viewerIndex - 1,
    );
};
$("#viewer-next").onclick = () => {
  if (state.viewerIndex < state.viewerPhotos.length - 1)
    showPhotos(
      [state.viewerPhotos[state.viewerIndex + 1]],
      state.viewerIndex + 1,
    );
};
$("#viewer-favorite").onclick = () =>
  action(async () => {
    const p = state.viewerPhotos[state.viewerIndex];
    await api("/api/favorite", { ids: [p.id], value: !p.favorite });
    p.favorite = !p.favorite;
    showPhotos([p], state.viewerIndex);
    await load();
    await status();
  });
$("#zoom").oninput = () => {
  pixel = false;
  zoom = Number($("#zoom").value) / 100;
  applyZoom();
};
$("#zoom-fit").onclick = resetZoom;
$("#zoom-pixel").onclick = () => {
  pixel = true;
  pan = { x: 0, y: 0 };
  applyZoom();
};
$("#fullscreen").onclick = () => {
  const viewer = $("#viewer");
  const expanded = viewer.classList.toggle("fullscreen");
  if (expanded) viewer.requestFullscreen?.().catch(() => {});
  else if (document.fullscreenElement) document.exitFullscreen();
  requestAnimationFrame(() => applyZoom());
};
window.addEventListener("resize", () => applyZoom());
$("#start-cull").onclick = () =>
  action(async () => {
    state.cullKind = ["similar", "bursts", "duplicates"].includes(state.view)
      ? state.view
      : state.settings.ui_state.cullKind || "similar";
    await nextGroup();
  });
$("#choose-winner").onclick = () => choose();
$("#skip-group").onclick = () => choose(true);
$("#undo").onclick = undo;
$("#check-eyes").onclick = analyzeEyes;
$("#settings").onclick = () => {
  $("#theme").value = state.settings.theme;
  $("#sensitivity").value = state.settings.sensitivity;
  $("#sensitivity-label").textContent = state.settings.sensitivity;
  $("#cache-limit").value = state.settings.cache_mb;
  $("#preferred-folder").value = state.settings.preferred_folder;
  $("#settings-dialog").showModal();
};
$("#sensitivity").oninput = () =>
  ($("#sensitivity-label").textContent = $("#sensitivity").value);
$("#settings-form").onsubmit = (event) => {
  event.preventDefault();
  action(async () => {
    const settings = {
      ...state.settings,
      theme: $("#theme").value,
      sensitivity: Number($("#sensitivity").value),
      cache_mb: Number($("#cache-limit").value),
      preferred_folder: $("#preferred-folder").value.trim(),
    };
    await api("/api/settings", settings);
    state.settings = settings;
    applyTheme();
    $("#settings-dialog").close();
    await load();
    toast("Настройки сохранены");
  });
};
$("#export").onclick = () => {
  $("#export-scope").textContent = state.selected.size
    ? `Будут скопированы выбранные кадры (${state.selected.size}) и их RAW+JPEG пары.`
    : "Будут скопированы все избранные кадры и их RAW+JPEG пары.";
  $("#export-dialog").showModal();
};
$("#export-form").onsubmit = (event) => {
  event.preventDefault();
  action(async () => {
    const result = await api("/api/export", {
      ids: [...state.selected],
      path: $("#export-path").value.trim(),
      structure: $("#export-structure").checked,
    });
    $("#export-dialog").close();
    outcome(result, "Экспортировано");
  });
};
function duplicateGroups(plan) {
  const groups = new Map();
  const get = (hash) => {
    if (!groups.has(hash)) groups.set(hash, { hash, keep: [], remove: [] });
    return groups.get(hash);
  };
  for (const item of plan.keep || []) get(item.hash || `keep-${item.id}`).keep.push(item);
  for (const item of plan.remove || []) get(item.hash || `copy-${item.id}`).remove.push(item);
  return [...groups.values()];
}
function planFile(item, retained = false) {
  const row = element("div", retained ? "plan-file plan-keep-row" : "plan-file");
  if (retained) row.append(element("span", "", "✓"));
  else {
    const check = element("input");
    check.type = "checkbox";
    check.dataset.planId = item.id;
    check.checked = state.duplicateSelected.has(item.id);
    check.setAttribute("aria-label", `Переместить копию ${item.path} в корзину`);
    check.onchange = () => {
      if (check.checked) state.duplicateSelected.add(item.id);
      else state.duplicateSelected.delete(item.id);
      updateDuplicateSummary();
    };
    row.append(check);
  }
  const thumbnail = element("img");
  thumbnail.src = `/media/${item.id}`;
  thumbnail.alt = "";
  thumbnail.loading = "lazy";
  const body = element("div", "plan-file-body");
  body.append(element("strong", "", retained ? `Сохранить · ${item.name}` : item.name));
  const path = element("span", "", item.path);
  path.title = item.path;
  body.append(path);
  row.append(thumbnail, body, element("span", "plan-file-size", retained ? "" : planSize(item.size)));
  return row;
}
function updateDuplicateSummary() {
  const plan = state.duplicatePlan;
  if (!plan) return;
  const selected = (plan.remove || []).filter((item) => state.duplicateSelected.has(item.id));
  const bytes = selected.reduce((total, item) => total + Number(item.size || 0), 0);
  const selectedText = selected.length
    ? `Выбрано ${number(selected.length)} из ${number(plan.remove.length)} · ${planSize(bytes)}`
    : `Ничего не выбрано · всего ${number(plan.remove.length)} кандидатов`;
  $("#duplicates-summary").textContent =
    `${selectedText}. Сохраняемых копий: ${number(plan.keep.length)}. Защищённых избранных: ${number(plan.protected_favorites)}.`;
  $("#apply-duplicates").disabled = !selected.length;
  $("#apply-duplicates").textContent = selected.length
    ? `Проверить ${number(selected.length)} выбранных файлов`
    : "Выберите файлы для проверки";
}
function renderDuplicatePlan() {
  const plan = state.duplicatePlan;
  const list = $("#duplicates-list");
  const fragment = document.createDocumentFragment();
  let visible = 0;
  let groupNumber = 0;
  for (const group of duplicateGroups(plan)) {
    const available = Math.max(0, state.duplicateVisible - visible);
    const candidates = group.remove.slice(0, available);
    if (group.remove.length && !candidates.length) continue;
    groupNumber++;
    const section = element("section", "plan-group");
    section.append(element("div", "plan-group-header", `Группа ${number(groupNumber)} · SHA-256 ${group.hash.slice(0, 16)} · ${number(group.remove.length)} кандидатов`));
    for (const retained of group.keep) section.append(planFile(retained, true));
    for (const candidate of candidates) section.append(planFile(candidate));
    if (candidates.length < group.remove.length)
      section.append(element("div", "plan-group-header", `Ещё ${number(group.remove.length - candidates.length)} копий не показаны и не выбраны`));
    fragment.append(section);
    visible += candidates.length;
  }
  list.replaceChildren(fragment);
  $("#duplicates-visible-count").textContent = `Показано ${number(visible)} из ${number(plan.remove.length)} кандидатов`;
  $("#more-duplicates").hidden = visible >= plan.remove.length;
  updateDuplicateSummary();
}
$("#duplicates-plan").onclick = () =>
  action(async () => {
    const plan = await api("/api/duplicates/plan", {
      preferred_folder: state.settings.preferred_folder,
    });
    state.duplicatePlan = plan;
    state.duplicateSelected.clear();
    state.duplicateVisible = 200;
    renderDuplicatePlan();
    $("#duplicates-dialog").showModal();
  });
$("#more-duplicates").onclick = () => {
  state.duplicateVisible += 200;
  renderDuplicatePlan();
};
$("#select-visible-duplicates").onclick = () => {
  $("#duplicates-list").querySelectorAll("input[data-plan-id]").forEach((check) => {
    check.checked = true;
    state.duplicateSelected.add(Number(check.dataset.planId));
  });
  updateDuplicateSummary();
};
$("#clear-duplicates").onclick = () => {
  state.duplicateSelected.clear();
  $("#duplicates-list").querySelectorAll("input[data-plan-id]").forEach((check) => check.checked = false);
  updateDuplicateSummary();
};
$("#apply-duplicates").onclick = () => {
  if (!state.duplicateSelected.size) return;
  const selected = state.duplicatePlan.remove.filter((item) => state.duplicateSelected.has(item.id));
  const bytes = selected.reduce((total, item) => total + Number(item.size || 0), 0);
  $("#duplicates-confirm-summary").textContent =
    `Будут перемещены ${number(selected.length)} ${fileWord(selected.length)} общим объёмом ${planSize(bytes)}. Проверьте выбор в полном плане; действие можно отменить или восстановить файлы из корзины.`;
  $("#duplicates-confirm-dialog").showModal();
};
$("#duplicates-confirm-cancel").onclick = () => $("#duplicates-confirm-dialog").close();
$("#duplicates-confirm-apply").onclick = () =>
  action(async () => {
    const ids = [...state.duplicateSelected];
    if (!ids.length) return;
    const result = await api("/api/duplicates/apply", { ids });
    $("#duplicates-confirm-dialog").close();
    $("#duplicates-dialog").close();
    state.duplicatePlan = null;
    state.duplicateSelected.clear();
    await load();
    await status();
    outcome(result, "В корзине");
  });
document.addEventListener("keydown", (event) => {
  if (["INPUT", "TEXTAREA", "SELECT"].includes(document.activeElement.tagName))
    return;
  // Let the focused player handle seeking, volume and playback itself.
  if (
    document.activeElement?.closest("video") &&
    ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", " ", "Home", "End"].includes(
      event.key,
    )
  )
    return;
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "z") {
    event.preventDefault();
    undo();
    return;
  }
  if ($("#viewer").open) {
    if (event.key === "ArrowLeft" && !$("#viewer-prev").disabled)
      $("#viewer-prev").click();
    if (event.key === "ArrowRight" && !$("#viewer-next").disabled)
      $("#viewer-next").click();
    if (event.key.toLowerCase() === "f" && !state.comparing && !state.culling)
      $("#viewer-favorite").click();
    if (event.key.toLowerCase() === "p") {
      if (state.culling) choose();
      else decide("keep");
    }
    if (event.key.toLowerCase() === "x" && !state.comparing) decide("reject");
    if (event.key.toLowerCase() === "s") {
      if (state.culling) choose(true);
      else decide("skip");
    }
    if (event.key === "Escape") state.culling = false;
    return;
  }
  if (document.querySelector("dialog[open]")) return;
  if (event.key === "/") {
    event.preventDefault();
    $("#search").focus();
  }
  if (event.key === "?") $("#help-dialog").showModal();
  if (event.key === "Escape") {
    state.selected.clear();
    updateSelection();
  }
});
matchMedia("(prefers-color-scheme:dark)").addEventListener(
  "change",
  applyTheme,
);
async function poll() {
  try {
    const p = await api("/api/progress");
    const wasRunning = state.running;
    progress(p);
    if (p.running || wasRunning) {
      await load();
      if (wasRunning && !p.running) {
        toast(
          `${p.message}. Файлов: ${p.done}. Ошибок: ${p.errors}${p.last_error ? ` — ${p.last_error}` : ""}`,
        );
        await status();
      }
    }
  } catch (error) {
    if (state.running) toast("Связь с приложением потеряна");
  } finally {
    setTimeout(poll, 1500);
  }
}
async function init() {
  try {
    state.token = (await api("/api/session")).token;
    await status();
    const saved = state.settings.ui_state || {};
    $("#sort").value = saved.sort || "date";
    $("#search").value = saved.search || "";
    setView(titles[saved.view] ? saved.view : "all", false);
    state.offset = Number(saved.offset) || 0;
    await load();
    poll();
  } catch (error) {
    toast(`Не удалось подключиться: ${error.message}`);
  }
}
init();
