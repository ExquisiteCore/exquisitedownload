// ExquisiteDownload Chrome Extension - Background Service Worker

const DEFAULT_SETTINGS = {
  rpcUrl: "http://127.0.0.1:6800/jsonrpc",
  rpcSecret: "",
  autoIntercept: true,
  minFileSize: 1048576, // 1MB
  fileTypes: ".zip,.rar,.7z,.exe,.dmg,.iso,.tar.gz,.tgz,.bz2,.xz,.mp4,.mkv,.avi,.mov,.wmv,.flv,.mp3,.flac,.wav,.apk,.msi,.deb,.rpm,.pdf,.doc,.docx,.xls,.xlsx",
};

// --- Settings ---

async function getSettings() {
  return new Promise((resolve) => {
    chrome.storage.sync.get(DEFAULT_SETTINGS, resolve);
  });
}

// --- WebSocket State ---

let ws = null;
let wsConnected = false;
let reconnectTimer = null;
let reconnectDelay = 1000;
const MAX_RECONNECT_DELAY = 10000;

// --- Task Cache ---

const taskCache = new Map();
let globalStats = { active: 0, waiting: 0, stopped: 0 };

// --- WS RPC Pending Requests ---

const pendingWsRpc = new Map();
let wsRpcIdCounter = 0;

// --- Popup Ports ---

const popupPorts = new Set();

// --- WebSocket Connection Management ---

function deriveWsUrl(rpcUrl) {
  try {
    const url = new URL(rpcUrl);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    url.pathname = "/ws";
    return url.toString();
  } catch {
    return "ws://127.0.0.1:6800/ws";
  }
}

async function connectWs() {
  if (ws && (ws.readyState === WebSocket.CONNECTING || ws.readyState === WebSocket.OPEN)) {
    return;
  }

  const settings = await getSettings();
  const wsUrl = deriveWsUrl(settings.rpcUrl);

  try {
    ws = new WebSocket(wsUrl);
  } catch {
    scheduleReconnect();
    return;
  }

  ws.onopen = () => {
    wsConnected = true;
    reconnectDelay = 1000;
    broadcastToPopups({ type: "wsStatus", connected: true });
    refreshFullState();
  };

  ws.onmessage = (event) => {
    try {
      const msg = JSON.parse(event.data);

      // JSON-RPC response
      if (msg.jsonrpc === "2.0" && msg.id != null) {
        const pending = pendingWsRpc.get(String(msg.id));
        if (pending) {
          pendingWsRpc.delete(String(msg.id));
          msg.error
            ? pending.reject(new Error(msg.error.message))
            : pending.resolve(msg.result);
        }
        return;
      }

      // EngineEvent (has type field, no jsonrpc)
      if (msg.type) {
        handleEngineEvent(msg);
      }
    } catch {
      // ignore parse errors
    }
  };

  ws.onclose = () => {
    wsConnected = false;
    for (const [, pending] of pendingWsRpc) {
      pending.reject(new Error("WebSocket closed"));
    }
    pendingWsRpc.clear();
    broadcastToPopups({ type: "wsStatus", connected: false });
    scheduleReconnect();
  };

  ws.onerror = () => {};
}

function disconnectWs() {
  if (reconnectTimer) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }
  if (ws) {
    ws.onclose = null;
    ws.onerror = null;
    ws.close();
    ws = null;
  }
  wsConnected = false;
}

function scheduleReconnect() {
  if (reconnectTimer) return;
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    connectWs();
  }, reconnectDelay);
  reconnectDelay = Math.min(reconnectDelay * 2, MAX_RECONNECT_DELAY);
}

// --- Engine Event Handling ---

function handleEngineEvent(event) {
  switch (event.type) {
    case "TaskProgress": {
      const task = taskCache.get(event.task_id);
      if (task) {
        task._downloaded = event.downloaded;
        if (event.total != null) task.total_size = event.total;
      }
      break;
    }
    case "TaskStarted": {
      rpcCall("tellStatus", [event.task_id])
        .then((task) => {
          taskCache.set(String(task.id), task);
          recomputeGlobalStats();
          broadcastState();
        })
        .catch(() => {});
      return; // wait for tellStatus before broadcasting
    }
    case "TaskCompleted": {
      const task = taskCache.get(event.task_id);
      if (task) {
        task.status = "Completed";
        task._downloaded = event.size;
        task.total_size = event.size;
      }
      recomputeGlobalStats();
      break;
    }
    case "TaskPaused": {
      const task = taskCache.get(event.task_id);
      if (task) task.status = "Paused";
      recomputeGlobalStats();
      break;
    }
    case "TaskError": {
      const task = taskCache.get(event.task_id);
      if (task) task.status = "Error";
      recomputeGlobalStats();
      break;
    }
  }

  broadcastToPopups({ type: "event", event });
}

// --- Task Cache Helpers ---

function recomputeGlobalStats() {
  let active = 0, waiting = 0, stopped = 0;
  for (const task of taskCache.values()) {
    switch (task.status) {
      case "Downloading": active++; break;
      case "Pending": waiting++; break;
      default: stopped++; break;
    }
  }
  globalStats = { active, waiting, stopped };
}

async function refreshFullState() {
  try {
    const [stats, active, waiting, stopped] = await Promise.all([
      rpcCall("getGlobalStat"),
      rpcCall("tellActive"),
      rpcCall("tellWaiting"),
      rpcCall("tellStopped"),
    ]);

    globalStats = stats;
    taskCache.clear();
    for (const task of [...(active || []), ...(waiting || []), ...(stopped || [])]) {
      taskCache.set(String(task.id), task);
    }

    broadcastState();
  } catch {
    // Server not available
  }
}

function broadcastState() {
  broadcastToPopups({
    type: "state",
    tasks: [...taskCache.values()],
    stats: globalStats,
    wsConnected,
  });
}

// --- Popup Port Management ---

function broadcastToPopups(msg) {
  for (const port of popupPorts) {
    try {
      port.postMessage(msg);
    } catch {
      popupPorts.delete(port);
    }
  }
}

chrome.runtime.onConnect.addListener((port) => {
  if (port.name !== "popup") return;

  popupPorts.add(port);
  port.onDisconnect.addListener(() => popupPorts.delete(port));

  port.onMessage.addListener(async (msg) => {
    if (msg.type === "getState") {
      if (taskCache.size === 0) {
        await refreshFullState();
      }
      port.postMessage({
        type: "state",
        tasks: [...taskCache.values()],
        stats: globalStats,
        wsConnected,
      });
    } else if (msg.type === "rpc") {
      try {
        const result = await rpcCall(msg.method, msg.params || []);

        // Optimistic cache update for mutations
        const targetId = msg.params?.[0] != null ? String(msg.params[0]) : null;
        if (msg.method === "remove" && targetId) {
          taskCache.delete(targetId);
          recomputeGlobalStats();
          broadcastState();
        } else if (msg.method === "unpause" && targetId) {
          const task = taskCache.get(targetId);
          if (task) task.status = "Downloading";
          recomputeGlobalStats();
          broadcastState();
        } else if (msg.method === "pause" && targetId) {
          const task = taskCache.get(targetId);
          if (task) task.status = "Paused";
          recomputeGlobalStats();
          broadcastState();
        }

        port.postMessage({ type: "rpcResponse", id: msg.id, success: true, result });
      } catch (e) {
        port.postMessage({ type: "rpcResponse", id: msg.id, success: false, error: e.message });
      }
    }
  });
});

// --- RPC Communication ---

function wsRpcSend(method, params) {
  return new Promise((resolve, reject) => {
    const id = String(++wsRpcIdCounter);
    const timeout = setTimeout(() => {
      pendingWsRpc.delete(id);
      reject(new Error("WS RPC timeout"));
    }, 10000);

    pendingWsRpc.set(id, {
      resolve: (result) => { clearTimeout(timeout); resolve(result); },
      reject: (err) => { clearTimeout(timeout); reject(err); },
    });

    ws.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
  });
}

async function httpRpcSend(rpcUrl, method, params) {
  const response = await fetch(rpcUrl, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: Date.now().toString(),
      method,
      params,
    }),
  });
  const data = await response.json();
  if (data.error) throw new Error(data.error.message);
  return data.result;
}

async function rpcCall(method, params = []) {
  const settings = await getSettings();
  const rpcParams = settings.rpcSecret
    ? [`token:${settings.rpcSecret}`, ...params]
    : params;

  if (ws && ws.readyState === WebSocket.OPEN) {
    try {
      return await wsRpcSend(method, rpcParams);
    } catch {
      // Fall through to HTTP
    }
  }

  return httpRpcSend(settings.rpcUrl, method, rpcParams);
}

// --- Helpers ---

async function getCookiesForUrl(url) {
  try {
    const cookies = await chrome.cookies.getAll({ url });
    if (cookies.length === 0) return null;
    return cookies.map((c) => `${c.name}=${c.value}`).join("; ");
  } catch {
    return null;
  }
}

function buildOptions(cookie, referrer) {
  const opts = {};
  const headers = [];
  if (referrer) headers.push(`Referer: ${referrer}`);
  if (headers.length > 0) opts.headers = headers;
  if (cookie) opts.cookie = cookie;
  return Object.keys(opts).length > 0 ? opts : null;
}

// --- Context Menu ---

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({
    id: "edl-download",
    title: "使用 edl 下载",
    contexts: ["link"],
  });
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (info.menuItemId === "edl-download" && info.linkUrl) {
    try {
      const cookie = await getCookiesForUrl(info.linkUrl);
      const referrer = tab?.url || info.pageUrl;
      const opts = buildOptions(cookie, referrer);
      const params = opts ? [info.linkUrl, opts] : [info.linkUrl];
      await rpcCall("addUri", params);
      chrome.notifications.create({
        type: "basic",
        iconUrl: "icons/icon128.png",
        title: "edl - 下载已添加",
        message: info.linkUrl.split("/").pop() || info.linkUrl,
      });
    } catch (e) {
      chrome.notifications.create({
        type: "basic",
        iconUrl: "icons/icon128.png",
        title: "edl - 连接失败",
        message: "请确认 edl rpc 服务已启动\n" + e.message,
      });
    }
  }
});

// --- Download Interception ---

function getFileExtension(url) {
  try {
    const pathname = new URL(url).pathname;
    const filename = pathname.split("/").pop() || "";
    const dotIndex = filename.lastIndexOf(".");
    if (dotIndex === -1) return "";
    return filename.substring(dotIndex).toLowerCase();
  } catch {
    return "";
  }
}

function shouldIntercept(url, fileSize, settings) {
  if (!settings.autoIntercept) return false;
  if (url.startsWith("blob:") || url.startsWith("data:")) return false;
  if (fileSize > 0 && fileSize < settings.minFileSize) return false;

  const ext = getFileExtension(url);
  if (!ext) return fileSize > 0 && fileSize >= settings.minFileSize;

  const allowedTypes = settings.fileTypes
    .split(",")
    .map((t) => t.trim().toLowerCase());
  return allowedTypes.some((t) => ext === t || ext.endsWith(t));
}

chrome.downloads.onCreated.addListener(async (downloadItem) => {
  const settings = await getSettings();
  const url = downloadItem.url;
  const fileSize = downloadItem.totalBytes || 0;

  if (!shouldIntercept(url, fileSize, settings)) return;

  chrome.downloads.cancel(downloadItem.id);
  chrome.downloads.erase({ id: downloadItem.id });

  const cookie = await getCookiesForUrl(url);
  const referrer = downloadItem.referrer || undefined;
  const opts = buildOptions(cookie, referrer);
  const params = opts ? [url, opts] : [url];

  try {
    await rpcCall("addUri", params);
    chrome.notifications.create({
      type: "basic",
      iconUrl: "icons/icon128.png",
      title: "edl - 下载已接管",
      message: downloadItem.filename?.split(/[/\\]/).pop() || url.split("/").pop() || url,
    });
  } catch (e) {
    chrome.notifications.create({
      type: "basic",
      iconUrl: "icons/icon128.png",
      title: "edl - 连接失败，使用浏览器下载",
      message: "请确认 edl rpc 服务已启动",
    });
    chrome.downloads.download({ url });
  }
});

// --- Message Handler (backward compat for options page) ---

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message.type === "rpc") {
    rpcCall(message.method, message.params || [])
      .then((result) => sendResponse({ success: true, result }))
      .catch((e) => sendResponse({ success: false, error: e.message }));
    return true;
  }
});

// --- Settings Change Listener ---

chrome.storage.onChanged.addListener((changes) => {
  if (changes.rpcUrl || changes.rpcSecret) {
    disconnectWs();
    connectWs();
  }
});

// --- Initialize ---

connectWs();
