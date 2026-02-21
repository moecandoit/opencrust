const chatEl = document.getElementById("chat");
const inputEl = document.getElementById("input");
const sendBtn = document.getElementById("send");
const reconnectBtn = document.getElementById("reconnect");
const refreshBtn = document.getElementById("refresh");
const clearBtn = document.getElementById("clear");
const connPill = document.getElementById("conn-pill");
const themeToggleBtn = document.getElementById("theme-toggle");
const sessionEl = document.getElementById("session");
const apiStatusEl = document.getElementById("api-status");
const sessionCountEl = document.getElementById("session-count");
const channelListEl = document.getElementById("channel-list");
const providerSelect = document.getElementById("provider-select");
const providerStatus = document.getElementById("provider-status");
const providerKeySection = document.getElementById("provider-key-section");
const providerApiKey = document.getElementById("provider-api-key");
const providerActivateBtn = document.getElementById("provider-activate");
const authSection = document.getElementById("auth-section");
const keyGatewayEl = document.getElementById("key-gateway");
const authConnectBtn = document.getElementById("auth-connect");
const updateBanner = document.getElementById("update-banner");
const updateBannerText = document.getElementById("update-banner-text");
const updateBannerClose = document.getElementById("update-banner-close");

const storageKey = "opencrust.session_id";
const keyStorage = "opencrust.gateway_key";
const providerStorage = "opencrust.provider";
const themeStorageKey = "opencrust.ui.theme";
let sessionId = localStorage.getItem(storageKey) || "";
let gatewayKey = localStorage.getItem(keyStorage) || "";
let selectedProvider = localStorage.getItem(providerStorage) || "";
let authRequired = false;
let socket = null;
let reconnectTimer = null;
let providerData = [];

let nanoTimerInterval = null;
let nanoElapsed = 0;
let thinkingTimeout = null;

function setAgentThinking(thinking) {
  const widget = document.getElementById('nano-agents');
  const timeEl = document.getElementById('nano-time');
  if (!widget) return;

  if (thinking) {
    widget.style.display = 'inline-flex';
    if (!nanoTimerInterval) {
      nanoElapsed = 0;
      if (timeEl) timeEl.textContent = '0s';
      nanoTimerInterval = setInterval(() => {
        nanoElapsed++;
        if (timeEl) timeEl.textContent = nanoElapsed + 's';
      }, 1000);
    }
  } else {
    widget.style.display = 'none';
    if (nanoTimerInterval) {
      clearInterval(nanoTimerInterval);
      nanoTimerInterval = null;
    }
    if (thinkingTimeout) {
      clearTimeout(thinkingTimeout);
      thinkingTimeout = null;
    }
  }
}

function resetThinkingDebounce() {
  setAgentThinking(true);
  if (thinkingTimeout) clearTimeout(thinkingTimeout);
  thinkingTimeout = setTimeout(() => {
    setAgentThinking(false);
  }, 1500);
}

// Pre-fill saved key
keyGatewayEl.value = gatewayKey;

function setTheme(theme, persist = true) {
  const selected = theme === "dark" ? "dark" : "light";
  document.documentElement.setAttribute("data-theme", selected);
  if (themeToggleBtn) {
    themeToggleBtn.textContent = selected === "dark" ? "Light Mode" : "Dark Mode";
  }
  if (persist) {
    localStorage.setItem(themeStorageKey, selected);
  }
}

function initTheme() {
  const stored = localStorage.getItem(themeStorageKey);
  if (stored === "light" || stored === "dark") {
    setTheme(stored, false);
    return;
  }
  const prefersDark = window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches;
  setTheme(prefersDark ? "dark" : "light");
}

function wsUrl() {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  const base = `${proto}//${location.host}/ws`;
  const key = gatewayKey || keyGatewayEl.value.trim();
  return key ? `${base}?token=${encodeURIComponent(key)}` : base;
}

function setConnectionState(isConnected) {
  connPill.innerHTML = isConnected
    ? '<span class="online">Connected</span>'
    : '<span class="offline">Disconnected</span>';
}

function setSession(id) {
  sessionId = id || "";
  if (sessionId) {
    localStorage.setItem(storageKey, sessionId);
    sessionEl.textContent = sessionId.slice(0, 8) + "...";
    sessionEl.title = sessionId;
  } else {
    localStorage.removeItem(storageKey);
    sessionEl.textContent = "none";
    sessionEl.title = "";
  }
}

function appendMessage(kind, text) {
  const div = document.createElement("div");
  div.className = `msg ${kind}`;
  div.textContent = text;
  chatEl.appendChild(div);
  chatEl.scrollTop = chatEl.scrollHeight;
}

function appendOrUpdateStreamMessage(role, text) {
  let isStreamChunk = false;
  let parsedContent = "";
  try {
    const lines = text.split('\n');
    for (const line of lines) {
      if (line.trim().startsWith('{') && line.trim().endsWith('}')) {
        const data = JSON.parse(line);
        if (data.content !== undefined) {
          isStreamChunk = true;
          parsedContent += data.content;
        }
      }
    }
  } catch (e) { }

  if (!isStreamChunk) {
    appendMessage(role, text);
    setAgentThinking(false);
    return;
  }

  resetThinkingDebounce();

  const msgs = chatEl.querySelectorAll('.msg.assistant');
  if (msgs.length > 0) {
    const lastMsg = msgs[msgs.length - 1];
    lastMsg.textContent += parsedContent;
    chatEl.scrollTop = chatEl.scrollHeight;
  } else {
    appendMessage(role, parsedContent);
  }
}

async function refreshStatus() {
  try {
    const r = await fetch("/api/status");
    const j = await r.json();
    apiStatusEl.innerHTML = `<span class="status-dot dot-ok"></span>${j.status}`;
    sessionCountEl.textContent = j.sessions;

    // Show update banner if a newer version is available
    if (j.latest_version && j.version) {
      const dismissed = sessionStorage.getItem("opencrust.update_dismissed");
      if (dismissed !== j.latest_version) {
        updateBannerText.innerHTML =
          `Update available: v${j.version} &rarr; v${j.latest_version.replace(/^v/, "")} &mdash; run <code>opencrust update</code>`;
        updateBanner.style.display = "";
      }
    } else {
      updateBanner.style.display = "none";
    }

    if (j.channels && j.channels.length > 0) {
      channelListEl.innerHTML = j.channels
        .map(ch => `<span class="channel-tag"><span class="status-dot dot-ok"></span>${ch}</span>`)
        .join("");
    } else {
      channelListEl.innerHTML = '<span class="no-channels">None configured</span>';
    }
  } catch {
    apiStatusEl.innerHTML = '<span class="status-dot dot-off"></span>unavailable';
    sessionCountEl.textContent = "-";
    channelListEl.innerHTML = '<span class="no-channels">-</span>';
  }

  loadProviders();
}

async function loadProviders() {
  try {
    const r = await fetch("/api/providers");
    const j = await r.json();
    providerData = j.providers || [];

    providerSelect.innerHTML = "";
    for (const p of providerData) {
      const opt = document.createElement("option");
      opt.value = p.id;
      opt.textContent = p.active ? p.display_name : `${p.display_name} (not configured)`;
      providerSelect.appendChild(opt);
    }

    // Restore saved selection, or pick the default
    const defaultProvider = providerData.find(p => p.is_default);
    const saved = selectedProvider || (defaultProvider ? defaultProvider.id : "");
    if (saved && [...providerSelect.options].some(o => o.value === saved)) {
      providerSelect.value = saved;
    }
    updateProviderUI();
  } catch {
    providerSelect.innerHTML = '<option value="">unavailable</option>';
    providerStatus.textContent = "";
  }
}

function updateProviderUI() {
  const id = providerSelect.value;
  const p = providerData.find(x => x.id === id);
  if (!p) {
    providerStatus.textContent = "";
    providerKeySection.style.display = "none";
    return;
  }
  if (p.active) {
    const tag = p.is_default ? "active, default" : "active";
    providerStatus.innerHTML = `<span class="status-dot dot-ok"></span>${tag}`;
    providerKeySection.style.display = "none";
  } else {
    providerStatus.innerHTML = `<span class="status-dot dot-off"></span>not configured`;
    providerKeySection.style.display = p.needs_api_key ? "" : "none";
  }
  selectedProvider = id;
  localStorage.setItem(providerStorage, id);
}

providerSelect.addEventListener("change", () => {
  updateProviderUI();
  // If switching to an active provider, tell the backend to use it as default
  const p = providerData.find(x => x.id === providerSelect.value);
  if (p && p.active) {
    fetch("/api/providers", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ provider_type: p.id, set_default: true }),
    }).then(() => loadProviders());
  }
});

providerActivateBtn.addEventListener("click", async () => {
  const id = providerSelect.value;
  const key = providerApiKey.value.trim();
  if (!key) return;

  providerActivateBtn.textContent = "Activating...";
  providerActivateBtn.disabled = true;

  try {
    const r = await fetch("/api/providers", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ provider_type: id, api_key: key, set_default: true }),
    });
    const j = await r.json();
    if (r.ok) {
      providerApiKey.value = "";
      appendMessage("sys", `Provider ${id} activated.`);
      await loadProviders();
    } else {
      appendMessage("error", j.message || "Failed to activate provider.");
    }
  } catch (e) {
    appendMessage("error", `Failed to activate provider: ${e}`);
  } finally {
    providerActivateBtn.textContent = "Save & Activate";
    providerActivateBtn.disabled = false;
  }
});

function scheduleReconnect() {
  if (reconnectTimer) return;
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    connect();
  }, 2000);
}

function handleServerEvent(raw) {
  let evt;
  try {
    evt = JSON.parse(raw);
  } catch {
    appendMessage("sys", `Raw: ${raw}`);
    return;
  }

  if (evt.session_id) setSession(evt.session_id);

  switch (evt.type) {
    case "connected":
      if (evt.note) {
        appendMessage("sys", `Connected (${evt.note}).`);
      }
      refreshStatus();
      break;
    case "resumed":
      appendMessage("sys", `Session resumed (${evt.history_length ?? 0} messages in history).`);
      refreshStatus();
      break;
    case "message":
      appendOrUpdateStreamMessage("assistant", evt.content || "(empty response)");
      break;
    case "error":
      setAgentThinking(false);
      appendMessage("error", `${evt.code || "error"}: ${evt.message || "unknown error"}`);
      break;
    default:
      appendMessage("sys", `Event ${evt.type || "unknown"}: ${JSON.stringify(evt)}`);
  }
}

function connect() {
  if (socket && (socket.readyState === WebSocket.OPEN || socket.readyState === WebSocket.CONNECTING)) {
    return;
  }

  socket = new WebSocket(wsUrl());

  socket.onopen = () => {
    setConnectionState(true);
    if (sessionId) {
      socket.send(JSON.stringify({ type: "resume", session_id: sessionId }));
    } else {
      socket.send(JSON.stringify({ type: "init" }));
    }
  };

  socket.onmessage = (ev) => handleServerEvent(ev.data);

  socket.onclose = () => {
    setConnectionState(false);
    scheduleReconnect();
  };

  socket.onerror = () => {
    setConnectionState(false);
  };
}

function reconnectFresh() {
  if (reconnectTimer) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }
  if (socket) {
    socket.onclose = null;
    try { socket.close(); } catch { }
    socket = null;
  }
  setConnectionState(false);
  connect();
}

function sendMessage() {
  const content = inputEl.value.trim();
  if (!content) return;

  if (!socket || socket.readyState !== WebSocket.OPEN) {
    appendMessage("error", "Not connected. Click Reconnect to try again.");
    return;
  }

  appendMessage("user", content);
  setAgentThinking(true);
  const msg = { content };
  const pid = providerSelect.value;
  if (pid) msg.provider = pid;
  socket.send(JSON.stringify(msg));
  inputEl.value = "";
  inputEl.focus();
}

sendBtn.addEventListener("click", sendMessage);
inputEl.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    sendMessage();
  }
});

reconnectBtn.addEventListener("click", () => {
  reconnectFresh();
});

clearBtn.addEventListener("click", () => {
  chatEl.innerHTML = "";
  setSession("");
  reconnectFresh();
});

refreshBtn.addEventListener("click", refreshStatus);

updateBannerClose.addEventListener("click", () => {
  updateBanner.style.display = "none";
  const ver = updateBannerText.textContent;
  // Extract version to remember dismissal for this version only
  const match = updateBannerText.innerHTML.match(/v([\d.]+)\s/);
  if (match) sessionStorage.setItem("opencrust.update_dismissed", match[1]);
});

authConnectBtn.addEventListener("click", () => {
  gatewayKey = keyGatewayEl.value.trim();
  if (gatewayKey) {
    localStorage.setItem(keyStorage, gatewayKey);
  }
  reconnectFresh();
});

if (themeToggleBtn) {
  themeToggleBtn.addEventListener("click", () => {
    const current = document.documentElement.getAttribute("data-theme") || "light";
    setTheme(current === "dark" ? "light" : "dark");
  });
}

// Navigation logic
const navItems = {
  chat: document.getElementById("nav-chat"),
  mcps: document.getElementById("nav-mcps"),
  extensions: document.getElementById("nav-extensions")
};

const views = {
  chat: document.getElementById("view-chat"),
  mcps: document.getElementById("view-mcps"),
  extensions: document.getElementById("view-extensions")
};

function switchView(viewId) {
  // Update nav active states
  Object.entries(navItems).forEach(([id, el]) => {
    if (id === viewId) {
      el.classList.add("active");
    } else {
      el.classList.remove("active");
    }
  });

  // Update view visibility
  Object.entries(views).forEach(([id, el]) => {
    if (id === viewId) {
      el.style.display = "";
    } else {
      el.style.display = "none";
    }
  });
}

navItems.chat.addEventListener("click", () => switchView("chat"));
navItems.mcps.addEventListener("click", () => switchView("mcps"));
navItems.extensions.addEventListener("click", () => switchView("extensions"));

// Boot: check if auth is required, then connect
async function boot() {
  initTheme();
  requestAnimationFrame(() => {
    document.body.classList.add("ready");
  });
  setConnectionState(false);
  setSession(sessionId);
  refreshStatus();

  try {
    const r = await fetch("/api/auth-check");
    const j = await r.json();
    authRequired = j.auth_required;
  } catch {
    authRequired = false;
  }

  if (authRequired) {
    authSection.style.display = "";
    if (gatewayKey) {
      // Have a saved key — try connecting with it
      connect();
    } else {
      appendMessage("sys", "This gateway requires an API key. Enter it in the sidebar.");
    }
  } else {
    authSection.style.display = "none";
    connect();
  }
} // end of boot()
function initNanoAgents() {
  const bg = document.getElementById('nano-bg');
  const grid = document.getElementById('nano-grid');
  const widget = document.getElementById('nano-agents');

  if (!bg || !grid) return;
  if (widget) widget.style.display = 'none'; // hidden initially

  // Background bits
  const colors = ['var(--brand)', 'var(--brand-2)', 'var(--online)', 'var(--accent-line)'];
  const bits = [];
  for (let i = 0; i < 6; i++) {
    const bit = document.createElement('div');
    bit.className = 'nano-bit';
    const size = Math.random() * 2 + 1;
    bit.style.width = size + 'px';
    bit.style.height = size + 'px';
    bit.style.left = (Math.random() * 100) + '%';
    bit.style.top = (Math.random() * 100) + '%';
    bit.style.backgroundColor = colors[Math.floor(Math.random() * colors.length)];
    bit.style.opacity = 0;
    bg.appendChild(bit);
    bits.push({ el: bit, id: Math.random() });
  }

  setInterval(() => {
    bits.forEach(b => {
      if (Math.random() > 0.95) {
        b.el.style.left = (Math.random() * 100) + '%';
        b.el.style.top = (Math.random() * 100) + '%';
        b.el.style.opacity = 0;
      } else {
        b.el.style.opacity = Math.sin(Date.now() / 1500 + b.id * 10) * 0.1 + 0.1;
      }
    });
  }, 250);

  // 4 pixel agents
  const agentColors = [
    ['var(--brand)', 'var(--brand-2)'],
    ['var(--online)', '#4ade80'],
    ['var(--warn-text)', 'var(--warn-edge)'],
    ['var(--ink-soft)', 'var(--ink)']
  ];

  const agentEls = [];
  let positions = [0, 1, 2, 3];

  for (let i = 0; i < 4; i++) {
    const agent = document.createElement('div');
    agent.className = 'nano-agent';
    for (let p = 0; p < 4; p++) {
      const pixel = document.createElement('div');
      pixel.className = 'nano-pixel';
      agent.appendChild(pixel);
    }
    grid.appendChild(agent);
    agentEls.push({ el: agent, colors: agentColors[i] });
  }

  setInterval(() => {
    agentEls.forEach(a => {
      const pixels = a.el.children;
      for (let i = 0; i < pixels.length; i++) {
        pixels[i].style.backgroundColor = a.colors[Math.floor(Math.random() * a.colors.length)];
        pixels[i].style.opacity = 0.7 + Math.random() * 0.3;
      }
    });
  }, 700);

  function getCoords(index) {
    return { x: (index % 2) * 12, y: Math.floor(index / 2) * 12 };
  }

  function updatePositions() {
    agentEls.forEach((a, i) => {
      const pos = positions[i];
      const coords = getCoords(pos);
      a.el.style.transform = `translate(${coords.x}px, ${coords.y}px)`;
    });
  }

  updatePositions();
  setInterval(() => {
    const next = [...positions];
    next.unshift(next.pop());
    positions = next;
    updatePositions();
  }, 2700);
}

boot();
initNanoAgents();
