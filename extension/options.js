// ExquisiteDownload Chrome Extension - Options

const $ = (id) => document.getElementById(id);

const DEFAULT_SETTINGS = {
  rpcUrl: "http://127.0.0.1:6800/jsonrpc",
  rpcSecret: "",
  autoIntercept: true,
  minFileSize: 1048576,
  fileTypes: ".zip,.rar,.7z,.exe,.dmg,.iso,.tar.gz,.tgz,.bz2,.xz,.mp4,.mkv,.avi,.mov,.wmv,.flv,.mp3,.flac,.wav,.apk,.msi,.deb,.rpm,.pdf,.doc,.docx,.xls,.xlsx",
};

function showMsg(text, isError) {
  const msg = $("msg");
  msg.textContent = text;
  msg.className = "msg " + (isError ? "err" : "ok");
  setTimeout(() => { msg.textContent = ""; msg.className = "msg"; }, 3000);
}

// Load settings
chrome.storage.sync.get(DEFAULT_SETTINGS, (settings) => {
  $("rpcUrl").value = settings.rpcUrl;
  $("rpcSecret").value = settings.rpcSecret;
  $("autoIntercept").checked = settings.autoIntercept;
  $("minFileSize").value = settings.minFileSize;
  $("fileTypes").value = settings.fileTypes;
});

// Save
$("saveBtn").addEventListener("click", () => {
  const settings = {
    rpcUrl: $("rpcUrl").value.trim() || DEFAULT_SETTINGS.rpcUrl,
    rpcSecret: $("rpcSecret").value,
    autoIntercept: $("autoIntercept").checked,
    minFileSize: parseInt($("minFileSize").value) || DEFAULT_SETTINGS.minFileSize,
    fileTypes: $("fileTypes").value.trim() || DEFAULT_SETTINGS.fileTypes,
  };
  chrome.storage.sync.set(settings, () => {
    showMsg("设置已保存", false);
  });
});

// Test connection
$("testBtn").addEventListener("click", async () => {
  const url = $("rpcUrl").value.trim() || DEFAULT_SETTINGS.rpcUrl;
  const secret = $("rpcSecret").value;

  const params = secret
    ? [`token:${secret}`]
    : [];

  try {
    const response = await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: "test",
        method: "getGlobalStat",
        params,
      }),
    });
    const data = await response.json();
    if (data.error) {
      showMsg("认证失败: " + data.error.message, true);
    } else {
      const r = data.result;
      showMsg(`连接成功 — 活跃: ${r.active}, 等待: ${r.waiting}, 完成: ${r.stopped}`, false);
    }
  } catch (e) {
    showMsg("连接失败: " + e.message, true);
  }
});
