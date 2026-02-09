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

async function rpc(method, params = []) {
  return new Promise((resolve, reject) => {
    chrome.runtime.sendMessage(
      { type: "rpc", method, params },
      (response) => {
        if (chrome.runtime.lastError) {
          reject(new Error(chrome.runtime.lastError.message));
        } else if (response && response.success) {
          resolve(response.result);
        } else {
          reject(new Error(response?.error || "RPC failed"));
        }
      }
    );
  });
}

// SVG icons for action buttons
const ICON_PAUSE = `<svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="4" width="4" height="16" rx="1"/><rect x="14" y="4" width="4" height="16" rx="1"/></svg>`;
const ICON_RESUME = `<svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor"><polygon points="6,4 20,12 6,20"/></svg>`;
const ICON_REMOVE = `<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>`;

// Compute display values from a task object
function taskProgress(task) {
  const totalSize = task.total_size || 0;
  const downloaded = (task.segments || []).reduce(
    (sum, s) => sum + (s.downloaded || 0),
    0
  );
  const status = task.status || "Pending";
  const isCompleted = status === "Completed";
  const isPausable = status === "Downloading";
  const isResumable = status === "Paused" || status === "Error";
  const speed = task.speed || 0;

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

async function refresh() {
  try {
    const [stats, active, waiting, stopped] = await Promise.all([
      rpc("getGlobalStat"),
      rpc("tellActive"),
      rpc("tellWaiting"),
      rpc("tellStopped"),
    ]);

    $("statusDot").className = "status-dot online";
    $("statActive").textContent = stats.active || 0;
    $("statWaiting").textContent = stats.waiting || 0;
    $("statStopped").textContent = stats.stopped || 0;

    const allTasks = [
      ...(active || []),
      ...(waiting || []),
      ...(stopped || []),
    ];

    const taskList = $("taskList");

    if (allTasks.length === 0) {
      taskList.innerHTML = EMPTY_TASKS_HTML;
      return;
    }

    // Remove empty placeholder if present
    const emptyEl = taskList.querySelector(".empty");
    if (emptyEl) emptyEl.remove();

    // Track current task IDs for cleanup
    const currentIds = new Set(allTasks.map((t) => String(t.id)));

    // Remove elements for tasks that no longer exist
    taskList.querySelectorAll(".task-item").forEach((el) => {
      if (!currentIds.has(el.dataset.id)) el.remove();
    });

    // Update existing or create new task elements
    allTasks.forEach((task) => {
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
    allTasks.forEach((task) => {
      const id = CSS.escape(String(task.id));
      const el = taskList.querySelector(`.task-item[data-id="${id}"]`);
      if (el) taskList.appendChild(el);
    });
  } catch (e) {
    $("statusDot").className = "status-dot offline";
    $("taskList").innerHTML = OFFLINE_HTML;
  }
}

async function addDownload() {
  const input = $("urlInput");
  const url = input.value.trim();
  if (!url) return;
  const btn = $("addBtn");
  btn.disabled = true;
  try {
    await rpc("addUri", [url]);
    input.value = "";
    refresh();
  } catch (e) {
    alert("添加失败: " + e.message);
  } finally {
    btn.disabled = false;
  }
}

async function pauseTask(id) {
  try {
    await rpc("pause", [id]);
    refresh();
  } catch (e) {
    console.error("Pause failed:", e);
  }
}

async function resumeTask(id) {
  try {
    await rpc("unpause", [id]);
    refresh();
  } catch (e) {
    console.error("Resume failed:", e);
  }
}

async function removeTask(id) {
  try {
    await rpc("remove", [id]);
    refresh();
  } catch (e) {
    console.error("Remove failed:", e);
  }
}

// Event delegation for task action buttons (avoids inline onclick blocked by CSP)
$("taskList").addEventListener("click", (e) => {
  const taskItem = e.target.closest(".task-item");
  if (!taskItem) return;
  const id = taskItem.dataset.id;

  if (e.target.closest(".btn-pause")) {
    pauseTask(id);
  } else if (e.target.closest(".btn-resume")) {
    resumeTask(id);
  } else if (e.target.closest(".btn-remove")) {
    removeTask(id);
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

refresh();
setInterval(refresh, 1500);
