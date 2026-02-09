// ExquisiteDownload Chrome Extension - Popup

const $ = (id) => document.getElementById(id);

function formatBytes(bytes) {
  if (bytes >= 1073741824) return (bytes / 1073741824).toFixed(2) + " GB";
  if (bytes >= 1048576) return (bytes / 1048576).toFixed(2) + " MB";
  if (bytes >= 1024) return (bytes / 1024).toFixed(2) + " KB";
  return bytes + " B";
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

function renderTask(task) {
  const filename = getFilename(task.file_path);
  const totalSize = task.total_size || 0;
  const downloaded = (task.segments || []).reduce(
    (sum, s) => sum + (s.downloaded || 0),
    0
  );
  const progress =
    totalSize > 0 ? Math.min((downloaded / totalSize) * 100, 100) : 0;
  const progressText =
    totalSize > 0 ? progress.toFixed(1) + "%" : formatBytes(downloaded);
  const sizeText =
    totalSize > 0
      ? formatBytes(downloaded) + " / " + formatBytes(totalSize)
      : formatBytes(downloaded);

  const status = task.status || "Pending";
  const isPausable = status === "Downloading";
  const isResumable = status === "Paused" || status === "Error";

  return `
    <div class="task-item" data-id="${task.id}">
      <div class="task-top">
        <span class="task-name" title="${filename}">${filename}</span>
        <span class="task-status ${statusClass(status)}">${statusLabel(status)}</span>
      </div>
      <div class="task-mid">
        <div class="progress-bar">
          <div class="progress-fill" style="width:${progress}%"></div>
        </div>
        <span class="progress-text">${progressText}</span>
      </div>
      <div class="task-bottom">
        <span class="task-size">${sizeText}</span>
        <div class="task-actions">
          ${isPausable ? `<button onclick="pauseTask('${task.id}')">暂停</button>` : ""}
          ${isResumable ? `<button onclick="resumeTask('${task.id}')">恢复</button>` : ""}
          <button onclick="removeTask('${task.id}')">删除</button>
        </div>
      </div>
    </div>`;
}

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

    if (allTasks.length === 0) {
      $("taskList").innerHTML = '<div class="empty">暂无任务</div>';
    } else {
      $("taskList").innerHTML = allTasks.map(renderTask).join("");
    }
  } catch (e) {
    $("statusDot").className = "status-dot offline";
    $("taskList").innerHTML =
      '<div class="empty">无法连接 edl RPC 服务<br>请启动 edl rpc</div>';
  }
}

async function addDownload() {
  const url = $("urlInput").value.trim();
  if (!url) return;
  try {
    await rpc("addUri", [url]);
    $("urlInput").value = "";
    refresh();
  } catch (e) {
    alert("添加失败: " + e.message);
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

// Make functions available to inline onclick
window.pauseTask = pauseTask;
window.resumeTask = resumeTask;
window.removeTask = removeTask;

// Init
$("addBtn").addEventListener("click", addDownload);
$("urlInput").addEventListener("keydown", (e) => {
  if (e.key === "Enter") addDownload();
});

refresh();
setInterval(refresh, 2000);
