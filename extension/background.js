// ExquisiteDownload Chrome Extension - Background Service Worker

const DEFAULT_SETTINGS = {
  rpcUrl: "http://127.0.0.1:6800/jsonrpc",
  rpcSecret: "",
  autoIntercept: true,
  minFileSize: 1048576, // 1MB
  fileTypes: ".zip,.rar,.7z,.exe,.dmg,.iso,.tar.gz,.tgz,.bz2,.xz,.mp4,.mkv,.avi,.mov,.wmv,.flv,.mp3,.flac,.wav,.apk,.msi,.deb,.rpm,.pdf,.doc,.docx,.xls,.xlsx",
};

// --- RPC Communication ---

async function getSettings() {
  return new Promise((resolve) => {
    chrome.storage.sync.get(DEFAULT_SETTINGS, resolve);
  });
}

async function rpcCall(method, params = []) {
  const settings = await getSettings();
  const rpcParams = settings.rpcSecret
    ? [`token:${settings.rpcSecret}`, ...params]
    : params;

  const response = await fetch(settings.rpcUrl, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: Date.now().toString(),
      method,
      params: rpcParams,
    }),
  });

  const data = await response.json();
  if (data.error) {
    throw new Error(data.error.message);
  }
  return data.result;
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
      const taskId = await rpcCall("addUri", params);
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

  // Skip blob/data URLs
  if (url.startsWith("blob:") || url.startsWith("data:")) return false;

  // Check file size threshold (if known)
  if (fileSize > 0 && fileSize < settings.minFileSize) return false;

  // Check file type
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

  // Cancel browser download
  chrome.downloads.cancel(downloadItem.id);
  chrome.downloads.erase({ id: downloadItem.id });

  // Capture cookies and referrer for the download URL
  const cookie = await getCookiesForUrl(url);
  const referrer = downloadItem.referrer || undefined;
  const opts = buildOptions(cookie, referrer);
  const params = opts ? [url, opts] : [url];

  // Send to edl
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
    // Re-trigger browser download since edl is not available
    chrome.downloads.download({ url });
  }
});

// --- Message Handler (for popup/options) ---

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message.type === "rpc") {
    rpcCall(message.method, message.params || [])
      .then((result) => sendResponse({ success: true, result }))
      .catch((e) => sendResponse({ success: false, error: e.message }));
    return true; // async response
  }
});
