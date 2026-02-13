// ExquisiteDownload Chrome Extension - Popup

const $ = (id) => document.getElementById(id);

function escapeHtml(str) {
  const div = document.createElement("div");
  div.textContent = str;
  return div.innerHTML;
}

function formatBytes(bytes) {
  if (bytes >= 1073741824) return (bytes / 1073741824).toFixed(2) + " GB";
  if (bytes >= 1048576) return (bytes / 1048576).toFixed(2) + " MB";
  if (bytes >= 1024) return (bytes / 1024).toFixed(2) + " KB";
  return bytes + " B";
}

function formatSpeed(bytesPerSec) {
  if (!bytesPerSec || bytesPerSec <= 0) return "";
  return formatBytes(bytesPerSec) + "/s";
}

function getFilename(filePath) {
  if (!filePath) return "unknown";
  const parts = filePath.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || "unknown";
}

function statusLabel(status) {
  const map = {
    Downloading: "下载中",
    Completed: "完成",
    Paused: "暂停",
    Error: "错误",
    Pending: "等待",
  };
  return map[status] || status;
}

function statusClass(status) {
  return (status || "").toLowerCase();
}

// SVG icons for action buttons
const ICON_PAUSE = `<svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="4" width="4" height="16" rx="1"/><rect x="14" y="4" width="4" height="16" rx="1"/></svg>`;
const ICON_RESUME = `<svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor"><polygon points="6,4 20,12 6,20"/></svg>`;
const ICON_REMOVE = `<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>`;

// --- Port-based RPC ---

const port = chrome.runtime.connect({ name: "popup" });
let rpcIdCounter = 0;
const rpcCallbacks = new Map();

function rpc(method, params = []) {
  return new Promise((resolve, reject) => {
    const id = ++rpcIdCounter;
    rpcCallbacks.set(id, { resolve, reject });
    port.postMessage({ type: "rpc", id, method, params });
  });
}

// --- Speed tracking from WS events ---

const prevProgress = new Map();

function computeSpeed(taskId, downloaded) {
  const now = Date.now();
  const prev = prevProgress.get(taskId);
  let speed = 0;
  if (prev && now - prev.time > 100) {
    speed = Math.max(0, (downloaded - prev.downloaded) / ((now - prev.time) / 1000));
  }
  prevProgress.set(taskId, { downloaded, time: now });
  return speed;
}

// --- Task data ---

const taskMap = new Map();

function getDownloaded(task) {
  if (task._downloaded != null) return task._downloaded;
  return (task.segments || []).reduce((sum, s) => sum + (s.downloaded || 0), 0);
}

// Compute display values from a task object
function taskProgress(task) {
  const totalSize = task.total_size || 0;
  const downloaded = getDownloaded(task);
  const status = task.status || "Pending";
  const isCompleted = status === "Completed";
  const isPausable = status === "Downloading";
  const isResumable = status === "Paused" || status === "Error";
  const speed = task._speed || 0;

  const progress = isCompleted
    ? 100
    : totalSize > 0
      ? Math.min((downloaded / totalSize) * 100, 100)
      : 0;
  const progressText = isCompleted
    ? "100%"
    : totalSize > 0
      ? progress.toFixed(1) + "%"
      : formatBytes(downloaded);
  const sizeText = isCompleted
    ? formatBytes(totalSize || downloaded)
    : totalSize > 0
      ? formatBytes(downloaded) + " / " + formatBytes(totalSize)
      : formatBytes(downloaded);
  const speedText = isPausable && speed > 0 ? formatSpeed(speed) : "";

  return { status, progress, progressText, sizeText, speedText, isPausable, isResumable };
}

function buildActionButtons(isPausable, isResumable) {
  let html = "";
  if (isPausable) html += `<button class="btn-pause" title="暂停">${ICON_PAUSE}</button>`;
  if (isResumable) html += `<button class="btn-resume" title="恢复">${ICON_RESUME}</button>`;
  html += `<button class="btn-remove" title="删除">${ICON_REMOVE}</button>`;
  return html;
}

function renderTask(task) {
  const filename = escapeHtml(getFilename(task.file_path));
  const safeId = escapeHtml(String(task.id));
  const p = taskProgress(task);

  return `
    <div class="task-item" data-id="${safeId}" data-status="${p.status}">
      <div class="task-top">
        <span class="task-name" title="${filename}">${filename}</span>
        <span class="task-status ${statusClass(p.status)}">${statusLabel(p.status)}</span>
      </div>
      <div class="task-mid">
        <div class="progress-bar">
          <div class="progress-fill ${statusClass(p.status)}" style="width:${p.progress}%"></div>
        </div>
        <span class="progress-text">${p.progressText}</span>
      </div>
      <div class="task-bottom">
        <span class="task-size">${p.sizeText}${p.speedText ? `<span class="task-speed">${p.speedText}</span>` : ""}</span>
        <div class="task-actions">
          ${buildActionButtons(p.isPausable, p.isResumable)}
        </div>
      </div>
    </div>`;
}

// Update an existing task DOM element in place (preserves element so CSS transition works)
function updateTaskElement(el, task) {
  const p = taskProgress(task);
  const prevStatus = el.dataset.status;

  // Progress bar width — CSS transition animates this smoothly
  const fill = el.querySelector(".progress-fill");
  fill.style.width = p.progress + "%";

  // Progress text
  el.querySelector(".progress-text").textContent = p.progressText;

  // Size + speed
  const sizeEl = el.querySelector(".task-size");
  sizeEl.innerHTML = p.sizeText + (p.speedText ? `<span class="task-speed">${p.speedText}</span>` : "");

  // Only rebuild status badge and action buttons when status actually changes
  if (prevStatus !== p.status) {
    el.dataset.status = p.status;

    fill.className = "progress-fill " + statusClass(p.status);

    const statusEl = el.querySelector(".task-status");
    statusEl.textContent = statusLabel(p.status);
    statusEl.className = "task-status " + statusClass(p.status);

    el.querySelector(".task-actions").innerHTML =
      buildActionButtons(p.isPausable, p.isResumable);
  }
}

const EMPTY_TASKS_HTML = `
  <div class="empty">
    <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="#444" stroke-width="1.5" stroke-linecap="round">
      <path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/>
      <polyline points="7 10 12 15 17 10"/>
      <line x1="12" y1="15" x2="12" y2="3"/>
    </svg>
    <div>暂无下载任务</div>
    <div class="empty-hint">在下方输入 URL 或右键链接发送到 edl</div>
  </div>`;

const OFFLINE_HTML = `
  <div class="empty">
    <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="#ef4444" stroke-width="1.5" stroke-linecap="round">
      <circle cx="12" cy="12" r="10"/>
      <line x1="15" y1="9" x2="9" y2="15"/>
      <line x1="9" y1="9" x2="15" y2="15"/>
    </svg>
    <div>无法连接 edl RPC 服务</div>
    <div class="empty-hint">请确认已启动 edl rpc</div>
  </div>`;

// --- State rendering ---

function renderFullState(tasks, stats, connected) {
  $("statusDot").className = "status-dot " + (connected ? "online" : "offline");

  if (tasks.length === 0 && !connected) {
    $("taskList").innerHTML = OFFLINE_HTML;
    $("statActive").textContent = 0;
    $("statWaiting").textContent = 0;
    $("statStopped").textContent = 0;
    return;
  }

  $("statActive").textContent = stats.active || 0;
  $("statWaiting").textContent = stats.waiting || 0;
  $("statStopped").textContent = stats.stopped || 0;

  // Sync taskMap
  const newIds = new Set();
  tasks.forEach((task) => {
    const id = String(task.id);
    newIds.add(id);
    const existing = taskMap.get(id);
    if (existing?._speed) task._speed = existing._speed;
    taskMap.set(id, task);
  });
  for (const id of taskMap.keys()) {
    if (!newIds.has(id)) {
      taskMap.delete(id);
      prevProgress.delete(id);
    }
  }

  const taskList = $("taskList");

  if (tasks.length === 0) {
    taskList.innerHTML = EMPTY_TASKS_HTML;
    return;
  }

  // Remove placeholder
  const placeholder = taskList.querySelector(".empty");
  if (placeholder) placeholder.remove();

  // Remove stale elements
  taskList.querySelectorAll(".task-item").forEach((el) => {
    if (!newIds.has(el.dataset.id)) el.remove();
  });

  // Update or create
  tasks.forEach((task) => {
    const id = String(task.id);
    const el = taskList.querySelector(`.task-item[data-id="${CSS.escape(id)}"]`);
    if (el) {
      updateTaskElement(el, task);
    } else {
      const wrapper = document.createElement("div");
      wrapper.innerHTML = renderTask(task);
      taskList.appendChild(wrapper.firstElementChild);
    }
  });

  // Reorder to match server order (active -> waiting -> stopped)
  tasks.forEach((task) => {
    const id = CSS.escape(String(task.id));
    const el = taskList.querySelector(`.task-item[data-id="${id}"]`);
    if (el) taskList.appendChild(el);
  });
}

// --- Incremental event handling ---

function handleEvent(event) {
  switch (event.type) {
    case "TaskProgress": {
      const task = taskMap.get(event.task_id);
      if (!task) return;
      task._downloaded = event.downloaded;
      if (event.total != null) task.total_size = event.total;
      task._speed = computeSpeed(event.task_id, event.downloaded);
      const el = $("taskList").querySelector(`.task-item[data-id="${CSS.escape(event.task_id)}"]`);
      if (el) updateTaskElement(el, task);
      break;
    }
    case "TaskCompleted": {
      const task = taskMap.get(event.task_id);
      if (!task) return;
      task.status = "Completed";
      task._downloaded = event.size;
      task.total_size = event.size;
      task._speed = 0;
      prevProgress.delete(event.task_id);
      const el = $("taskList").querySelector(`.task-item[data-id="${CSS.escape(event.task_id)}"]`);
      if (el) updateTaskElement(el, task);
      updateStatCounters();
      break;
    }
    case "TaskPaused": {
      const task = taskMap.get(event.task_id);
      if (!task) return;
      task.status = "Paused";
      task._speed = 0;
      prevProgress.delete(event.task_id);
      const el = $("taskList").querySelector(`.task-item[data-id="${CSS.escape(event.task_id)}"]`);
      if (el) updateTaskElement(el, task);
      updateStatCounters();
      break;
    }
    case "TaskError": {
      const task = taskMap.get(event.task_id);
      if (!task) return;
      task.status = "Error";
      task._speed = 0;
      prevProgress.delete(event.task_id);
      const el = $("taskList").querySelector(`.task-item[data-id="${CSS.escape(event.task_id)}"]`);
      if (el) updateTaskElement(el, task);
      updateStatCounters();
      break;
    }
  }
}

function updateStatCounters() {
  let active = 0, waiting = 0, stopped = 0;
  for (const task of taskMap.values()) {
    switch (task.status) {
      case "Downloading": active++; break;
      case "Pending": waiting++; break;
      default: stopped++; break;
    }
  }
  $("statActive").textContent = active;
  $("statWaiting").textContent = waiting;
  $("statStopped").textContent = stopped;
}

// --- Port message handler ---

port.onMessage.addListener((msg) => {
  switch (msg.type) {
    case "state":
      renderFullState(msg.tasks || [], msg.stats || {}, msg.wsConnected);
      break;
    case "event":
      handleEvent(msg.event);
      break;
    case "wsStatus":
      $("statusDot").className = "status-dot " + (msg.connected ? "online" : "offline");
      if (msg.connected) port.postMessage({ type: "getState" });
      break;
    case "rpcResponse": {
      const cb = rpcCallbacks.get(msg.id);
      if (cb) {
        rpcCallbacks.delete(msg.id);
        msg.success ? cb.resolve(msg.result) : cb.reject(new Error(msg.error));
      }
      break;
    }
  }
});

// --- Actions ---

async function addDownload() {
  const input = $("urlInput");
  const url = input.value.trim();
  if (!url) return;
  const btn = $("addBtn");
  btn.disabled = true;
  try {
    await rpc("addUri", [url]);
    input.value = "";
  } catch (e) {
    alert("添加失败: " + e.message);
  } finally {
    btn.disabled = false;
  }
}

// Event delegation for task action buttons
$("taskList").addEventListener("click", (e) => {
  const taskItem = e.target.closest(".task-item");
  if (!taskItem) return;
  const id = taskItem.dataset.id;

  if (e.target.closest(".btn-pause")) {
    rpc("pause", [id]).catch(console.error);
  } else if (e.target.closest(".btn-resume")) {
    rpc("unpause", [id]).catch(console.error);
  } else if (e.target.closest(".btn-remove")) {
    rpc("remove", [id]).catch(console.error);
  }
});

// Init
$("addBtn").addEventListener("click", addDownload);
$("urlInput").addEventListener("keydown", (e) => {
  if (e.key === "Enter") addDownload();
});

// Open settings
$("settingsBtn").addEventListener("click", () => {
  chrome.runtime.openOptionsPage();
});

// Request initial state
port.postMessage({ type: "getState" });

// Fallback: full state refresh every 30s as safety net
setInterval(() => {
  port.postMessage({ type: "getState" });
}, 30000);
