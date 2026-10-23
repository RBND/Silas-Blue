// Silas Blue v4.0 Clean-Room Administration Console Client
// Security: Strictly uses textContent / createElement (zero innerHTML) and sends X-Requested-With: SilasBlue-SPA.

(function () {
  "use strict";

  const CSRF_HEADER = { "X-Requested-With": "SilasBlue-SPA" };
  let remotePresets = [];
  let currentGuildId = null;
  let telemetryLogs = [];
  let wsConnection = null;

  // ==========================================================================
  // Theme Management (Following modern-web-guidance dark-mode-toggle)
  // ==========================================================================
  function applyTheme(themeVal, persist) {
    const metaScheme = document.querySelector('meta[name="color-scheme"]');
    const root = document.documentElement;

    if (!themeVal || themeVal === "system") {
      root.removeAttribute("data-theme");
      if (metaScheme) metaScheme.content = "light dark";
      if (persist) {
        try {
          localStorage.removeItem("silas-theme");
          localStorage.removeItem("color-scheme");
        } catch (_) {}
      }
      return;
    }

    root.setAttribute("data-theme", themeVal);
    const isLight = themeVal === "daylight-slate";
    const scheme = isLight ? "light" : "dark";
    if (metaScheme) metaScheme.content = scheme;

    if (persist) {
      try {
        localStorage.setItem("silas-theme", themeVal);
        localStorage.setItem("color-scheme", scheme);
      } catch (_) {}
    }
  }

  function initTheme() {
    let saved = "system";
    try {
      saved = localStorage.getItem("silas-theme") || "system";
    } catch (_) {}
    applyTheme(saved, false);

    const sel = document.getElementById("theme-selector");
    if (sel) {
      sel.value = saved;
      sel.addEventListener("change", () => {
        applyTheme(sel.value, true);
        apiFetch("/api/settings", {
          method: "PUT",
          body: JSON.stringify({ theme: sel.value }),
        }).catch(() => {});
      });
    }

    window.addEventListener("storage", (e) => {
      if (e.key === "silas-theme") {
        const next = e.newValue || "system";
        applyTheme(next, false);
        if (sel) sel.value = next;
      }
    });
  }

  // ==========================================================================
  // HTTP Helper & Toast Notifications
  // ==========================================================================
  async function apiFetch(url, options = {}) {
    const headers = Object.assign(
      { "Content-Type": "application/json" },
      CSRF_HEADER,
      options.headers || {}
    );
    const resp = await fetch(url, Object.assign({}, options, { headers }));
    if (resp.status === 401 && !window.location.pathname.endsWith("/login.html")) {
      window.location.href = "/login.html";
      throw new Error("Unauthorized");
    }
    const data = await resp.json().catch(() => ({}));
    if (!resp.ok) {
      throw new Error(data.error || `HTTP ${resp.status}`);
    }
    return data;
  }

  function showToast(message) {
    const container = document.getElementById("toast-container");
    if (!container) return;
    const toast = document.createElement("div");
    toast.className = "toast";
    toast.textContent = message;
    container.appendChild(toast);
    setTimeout(() => {
      toast.remove();
    }, 3800);
  }

  // ==========================================================================
  // Login Page Handling
  // ==========================================================================
  function initLoginPage() {
    const alertEl = document.getElementById("login-alert");
    if (!alertEl) return;

    const params = new URLSearchParams(window.location.search);
    const err = params.get("error");
    if (err) {
      const messages = {
        missing_token: "Missing One-Time Login token parameter.",
        invalid_or_expired_otp:
          "That One-Time Login link has already been used or has expired. Copy the latest URL from your terminal or sign in with Discord.",
        oauth_not_configured:
          "Discord OAuth2 Client ID & Secret are not configured yet. Use the terminal One-Time Login link first.",
        oauth_state_missing: "OAuth2 CSRF state cookie expired. Please try again.",
        oauth_state_mismatch: "OAuth2 CSRF state mismatch. Please try again.",
        unauthorized_discord_account:
          "Access denied: that Discord account is not an Application Owner/Admin or listed as bot_owner_id.",
      };
      alertEl.textContent = messages[err] || `Authentication error: ${err}`;
      alertEl.classList.remove("hidden");
    }
  }

  // ==========================================================================
  // Dashboard Navigation & Initialization
  // ==========================================================================
  function initNavigation() {
    const navButtons = document.querySelectorAll(".nav-item[data-view]");
    const panels = document.querySelectorAll(".view-panel");

    navButtons.forEach((btn) => {
      btn.addEventListener("click", () => {
        const target = btn.getAttribute("data-view");
        navButtons.forEach((b) => b.classList.remove("active"));
        btn.classList.add("active");

        panels.forEach((panel) => {
          if (panel.id === `view-${target}`) {
            panel.classList.remove("hidden");
          } else {
            panel.classList.add("hidden");
          }
        });
      });
    });

    const subtabButtons = document.querySelectorAll(".subtab-btn[data-subtab]");
    const subtabPanels = document.querySelectorAll(".subtab-panel");
    subtabButtons.forEach((btn) => {
      btn.addEventListener("click", () => {
        const target = btn.getAttribute("data-subtab");
        subtabButtons.forEach((b) => b.classList.remove("active"));
        btn.classList.add("active");
        subtabPanels.forEach((p) => {
          if (p.id === target) {
            p.classList.remove("hidden");
          } else {
            p.classList.add("hidden");
          }
        });
      });
    });
  }

  async function refreshStatus() {
    try {
      const st = await apiFetch("/api/status");
      const badge = document.getElementById("bot-status-badge");
      if (badge) {
        badge.textContent = st.bot_running
          ? `Online (${st.uptime_seconds}s)`
          : "Offline";
        badge.className = st.bot_running
          ? "badge badge-online"
          : "badge badge-offline";
      }

      setText("sidebar-version", `v${st.version}`);
      setText("stat-guilds", String(st.connected_guilds));
      setText("stat-backend", st.backend_type || "ollama");
      setText("stat-model", st.active_model || "-");
      setText("stat-memory", String(st.active_memory_channels));

      const nameInput = document.getElementById("bot-global-username");
      if (nameInput && st.bot_identity && st.bot_identity.username && !nameInput.value) {
        nameInput.value = st.bot_identity.username;
      }

      const inviteInput = document.getElementById("invite-url-display");
      if (inviteInput && st.invite_url) {
        inviteInput.value = st.invite_url;
      }
    } catch (_) {}
  }

  function setText(id, val) {
    const el = document.getElementById(id);
    if (el) el.textContent = val;
  }

  function setBadgeState(id, isConfigured) {
    const el = document.getElementById(id);
    if (!el) return;
    el.textContent = isConfigured ? "Configured" : "Unset";
    el.className = isConfigured ? "badge badge-online" : "badge badge-neutral";
  }

  // ==========================================================================
  // Global Settings & AI Backends
  // ==========================================================================
  async function loadGlobalSettings() {
    try {
      const [settingsRes, presetsRes] = await Promise.all([
        apiFetch("/api/settings"),
        apiFetch("/api/ai/presets"),
      ]);
      remotePresets = presetsRes.presets || [];
      const s = settingsRes.settings;
      if (!s) return;

      setBadgeState("badge-has-token", Boolean(s.has_discord_token));
      setBadgeState("badge-has-secret", Boolean(s.discord_oauth_client_secret));
      setBadgeState("badge-has-remote-key", Boolean(s.remote_api_key));
      setBadgeState("badge-has-tavily", Boolean(s.tavily_api_key));
      setBadgeState("badge-has-brave", Boolean(s.brave_api_key));

      setValue("cfg-owner-ids", s.bot_owner_id || "");
      setValue("cfg-client-id", s.discord_oauth_client_id || "");
      setValue("cfg-ai-provider", s.backend_type || "ollama");
      setValue("cfg-ollama-url", s.ollama_url || "http://127.0.0.1:11434");
      setValue("cfg-llamacpp-url", s.llamacpp_url || "http://127.0.0.1:8081");
      setValue("cfg-remote-preset", s.remote_preset || "Xiaomi MiMo");
      setValue("cfg-remote-base-url", s.remote_base_url || "");
      setValue("cfg-remote-model", s.remote_model || "");
      setValue("cfg-search-provider", s.web_search_provider || "bing");
      setValue("update-channel-select", s.update_channel || "stable");

      await refreshModelDropdown(s.active_model, false);
    } catch (e) {
      showToast(`Failed to load settings: ${e.message}`);
    }
  }

  async function refreshModelDropdown(selectedModel, forceRefresh) {
    const select = document.getElementById("cfg-active-model");
    if (!select) return;
    try {
      const res = await apiFetch(`/api/ai/models?refresh=${forceRefresh ? "true" : "false"}`);
      select.replaceChildren();
      const models = res.models || [];
      const seen = new Set();

      if (selectedModel) {
        const opt = document.createElement("option");
        opt.value = selectedModel;
        opt.textContent = selectedModel;
        select.appendChild(opt);
        seen.add(selectedModel);
      }

      models.forEach((m) => {
        const modelId = typeof m === "string" ? m : m.id;
        if (modelId && !seen.has(modelId)) {
          const opt = document.createElement("option");
          opt.value = modelId;
          opt.textContent = modelId;
          select.appendChild(opt);
          seen.add(modelId);
        }
      });

      if (selectedModel) select.value = selectedModel;
    } catch (_) {}
  }

  async function saveGlobalSettings() {
    const ownerIdRaw = getValue("cfg-owner-ids")
      .split(",")[0]
      .trim();

    const payload = {
      bot_owner_id: ownerIdRaw,
      discord_oauth_client_id: getValue("cfg-client-id"),
      backend_type: getValue("cfg-ai-provider"),
      active_model: getValue("cfg-active-model"),
      ollama_url: getValue("cfg-ollama-url"),
      llamacpp_url: getValue("cfg-llamacpp-url"),
      remote_preset: getValue("cfg-remote-preset"),
      remote_base_url: getValue("cfg-remote-base-url"),
      remote_model: getValue("cfg-remote-model"),
      web_search_provider: getValue("cfg-search-provider"),
    };

    const tok = getValue("cfg-discord-token");
    if (tok) payload.discord_token = tok;
    const sec = getValue("cfg-client-secret");
    if (sec) payload.discord_oauth_client_secret = sec;
    const remKey = getValue("cfg-remote-api-key");
    if (remKey) payload.remote_api_key = remKey;
    const tavKey = getValue("cfg-tavily-key");
    if (tavKey) payload.tavily_api_key = tavKey;
    const brvKey = getValue("cfg-brave-key");
    if (brvKey) payload.brave_api_key = brvKey;

    try {
      await apiFetch("/api/settings", {
        method: "PUT",
        body: JSON.stringify(payload),
      });
      setValue("cfg-discord-token", "");
      setValue("cfg-client-secret", "");
      setValue("cfg-remote-api-key", "");
      setValue("cfg-tavily-key", "");
      setValue("cfg-brave-key", "");
      showToast("Saved global settings.");
      await loadGlobalSettings();
      await refreshStatus();
    } catch (e) {
      showToast(`Error saving settings: ${e.message}`);
    }
  }

  // ==========================================================================
  // Server Configuration (4 Sub-Tabs + Chip Selectors)
  // ==========================================================================
  async function loadGuildList() {
    const sel = document.getElementById("guild-selector");
    if (!sel) return;
    try {
      const res = await apiFetch("/api/guilds");
      const guilds = res.guilds || [];
      sel.replaceChildren();

      if (guilds.length === 0) {
        const opt = document.createElement("option");
        opt.value = "0";
        opt.textContent = "Default Server Template (No Connected Guilds)";
        sel.appendChild(opt);
        currentGuildId = "0";
      } else {
        guilds.forEach((g) => {
          const opt = document.createElement("option");
          opt.value = g.id;
          opt.textContent = g.connected ? `${g.name} (${g.id})` : `${g.name} [Offline]`;
          sel.appendChild(opt);
        });
        if (!currentGuildId || !guilds.some((g) => g.id === currentGuildId)) {
          currentGuildId = guilds[0].id;
        }
        sel.value = currentGuildId;
      }

      await loadSelectedGuildConfig();
    } catch (_) {}
  }

  async function loadSelectedGuildConfig() {
    if (!currentGuildId) return;
    try {
      const res = await apiFetch(`/api/guilds/${currentGuildId}/config`);
      const c = res.config || {};
      const meta = res.metadata || null;
      const qw = res.quota_window || null;

      setValue("gcfg-bot-nickname", (meta && meta.bot_nickname) || "");
      setValue("gcfg-default-model", c.default_model || "");
      setValue("gcfg-allowed-channels", (c.reply_users || []).join(", "));
      setValue("gcfg-prompt-roles", (c.reply_roles || []).join(", "));
      setValue("gcfg-model-roles", (c.change_model_roles || []).join(", "));
      setValue("gcfg-admin-roles", (c.change_permission_roles || []).join(", "));

      const tq = c.token_quotas || {};
      const defaultLimit =
        tq.role_limits && tq.role_limits.everyone !== undefined
          ? tq.role_limits.everyone
          : 10000;
      setChecked("gcfg-token-enabled", Boolean(tq.enabled));
      setValue("gcfg-token-limit", String(defaultLimit));
      setValue("gcfg-token-schedule", tq.reset_frequency || "daily");
      setValue("gcfg-token-timezone", "UTC");

      setValue("gcfg-system-prompt", c.system_prompt || "");
      setChecked("gcfg-web-search", Boolean(c.web_search_enabled));
      setChecked("gcfg-memory-enabled", (c.memory_turns ?? 6) > 0);
      setValue("gcfg-memory-max", String(c.memory_turns ?? 6));
      setValue("gcfg-memory-ttl", String(c.memory_ttl_minutes ?? 30));
      setChecked("gcfg-pagination-enabled", Boolean(c.pagination_enabled));
      setValue("gcfg-pagination-chars", String(c.pagination_max_chars ?? 1000));
      setValue("gcfg-random-mode", c.random_reply_mode || "off");
      setValue("gcfg-random-value", String(c.random_reply_value ?? 0));

      setChecked("gcfg-forum-enabled", Boolean(c.forum_auto_reply_enabled));
      setChecked("gcfg-forum-search", Boolean(c.web_search_enabled));
      setValue("gcfg-forum-channels", (c.forum_monitored_channels || []).join(", "));
      setValue("gcfg-forum-prompt", c.forum_system_prompt || "");

      renderMetadataChips(meta);
      renderQuotaUsage(qw);
    } catch (e) {
      showToast(`Failed to load server config: ${e.message}`);
    }
  }

  function renderMetadataChips(meta) {
    const textChContainer = document.getElementById("chips-text-channels");
    const forumChContainer = document.getElementById("chips-forum-channels");
    const roleContainer = document.getElementById("chips-roles");

    if (textChContainer) textChContainer.replaceChildren();
    if (forumChContainer) forumChContainer.replaceChildren();
    if (roleContainer) roleContainer.replaceChildren();

    if (!meta) return;

    (meta.channels || []).forEach((ch) => {
      if (ch.kind === "forum" && forumChContainer) {
        forumChContainer.appendChild(
          createToggleChip(`#${ch.name}`, ch.id, "gcfg-forum-channels")
        );
      }
    });

    (meta.roles || []).forEach((r) => {
      if (roleContainer) {
        roleContainer.appendChild(
          createToggleChip(`@${r.name}`, r.id, "gcfg-prompt-roles")
        );
      }
    });
  }

  function createToggleChip(label, idValue, targetInputId) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "chip";
    btn.textContent = `${label} (${idValue})`;
    btn.addEventListener("click", () => {
      const input = document.getElementById(targetInputId);
      if (!input) return;
      const items = input.value
        .split(",")
        .map((s) => s.trim())
        .filter((s) => s.length > 0);
      const idx = items.indexOf(idValue);
      if (idx >= 0) {
        items.splice(idx, 1);
      } else {
        items.push(idValue);
      }
      input.value = items.join(", ");
    });
    return btn;
  }

  function renderQuotaUsage(qw) {
    const label = document.getElementById("quota-window-label");
    const list = document.getElementById("quota-usage-list");
    if (!list) return;
    list.replaceChildren();

    if (!qw) return;
    if (label) {
      label.textContent = `${qw.frequency || "daily"} (${qw.window_start || ""} -> ${qw.window_end || ""})`;
    }
  }

  function parseCsvList(inputId) {
    return getValue(inputId)
      .split(",")
      .map((s) => s.trim())
      .filter((s) => s.length > 0);
  }

  async function saveGuildConfig() {
    if (!currentGuildId) return;
    const memEnabled = getChecked("gcfg-memory-enabled");
    const memTurns = memEnabled ? Number(getValue("gcfg-memory-max")) || 6 : 0;
    const defaultLim = Number(getValue("gcfg-token-limit")) || 10000;

    const patch = {
      bot_nickname: getValue("gcfg-bot-nickname"),
      default_model: getValue("gcfg-default-model"),
      reply_users: parseCsvList("gcfg-allowed-channels"),
      reply_roles: parseCsvList("gcfg-prompt-roles"),
      change_model_roles: parseCsvList("gcfg-model-roles"),
      change_permission_roles: parseCsvList("gcfg-admin-roles"),
      token_quotas: {
        enabled: getChecked("gcfg-token-enabled"),
        role_limits: { everyone: defaultLim },
        reset_frequency: getValue("gcfg-token-schedule") || "daily",
      },
      system_prompt: getValue("gcfg-system-prompt"),
      web_search_enabled: getChecked("gcfg-web-search"),
      memory_turns: memTurns,
      memory_ttl_minutes: Number(getValue("gcfg-memory-ttl")) || 30,
      pagination_enabled: getChecked("gcfg-pagination-enabled"),
      pagination_max_chars: Number(getValue("gcfg-pagination-chars")) || 1000,
      random_reply_mode: getValue("gcfg-random-mode"),
      random_reply_value: Number(getValue("gcfg-random-value")) || 0,
      forum_auto_reply_enabled: getChecked("gcfg-forum-enabled"),
      forum_monitored_channels: parseCsvList("gcfg-forum-channels"),
      forum_system_prompt: getValue("gcfg-forum-prompt"),
    };

    try {
      await apiFetch(`/api/guilds/${currentGuildId}/config`, {
        method: "PUT",
        body: JSON.stringify(patch),
      });
      showToast("Saved server configuration.");
      await loadSelectedGuildConfig();
    } catch (e) {
      showToast(`Failed to save server config: ${e.message}`);
    }
  }

  // ==========================================================================
  // WebSocket Live Telemetry Console
  // ==========================================================================
  function connectTelemetryWebSocket() {
    const logBox = document.getElementById("telemetry-log-box");
    if (!logBox) return;

    const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
    const wsUrl = `${proto}//${window.location.host}/api/ws/logs`;
    wsConnection = new WebSocket(wsUrl);

    wsConnection.addEventListener("message", (evt) => {
      try {
        const payload = JSON.parse(evt.data);
        telemetryLogs.push(payload);
        if (telemetryLogs.length > 400) {
          telemetryLogs.shift();
        }
        renderTelemetryConsole();
      } catch (_) {}
    });

    wsConnection.addEventListener("close", () => {
      setTimeout(connectTelemetryWebSocket, 4000);
    });
  }

  function renderTelemetryConsole() {
    const logBox = document.getElementById("telemetry-log-box");
    const filterEl = document.getElementById("log-filter-level");
    if (!logBox) return;

    const filter = filterEl ? filterEl.value : "ALL";
    logBox.replaceChildren();

    telemetryLogs.forEach((pkt) => {
      const cat = pkt.type || "system";
      if (filter === "ACTIVITY" && cat === "system") return;

      const line = document.createElement("div");
      if (cat === "prompt" || cat === "reply" || cat === "search") {
        line.className = "log-line log-activity";
      } else {
        line.className = "log-line log-info";
      }
      line.textContent = pkt.formatted || `[${pkt.timestamp || ""}] ${JSON.stringify(pkt)}`;
      logBox.appendChild(line);
    });

    logBox.scrollTop = logBox.scrollHeight;
  }

  // ==========================================================================
  // Updates & Snapshots
  // ==========================================================================
  async function loadSnapshots() {
    const list = document.getElementById("snapshots-list");
    if (!list) return;
    try {
      const res = await apiFetch("/api/update/snapshots");
      list.replaceChildren();
      const snaps = res.snapshots || [];
      if (snaps.length === 0) {
        const p = document.createElement("p");
        p.className = "help-text";
        p.textContent = "No local snapshots created yet.";
        list.appendChild(p);
        return;
      }
      snaps.forEach((s) => {
        const row = document.createElement("div");
        row.className = "usage-row";
        const left = document.createElement("span");
        left.textContent = `${s.id} (v${s.version})`;
        const right = document.createElement("span");
        right.textContent = `${Math.round((s.size_bytes || 0) / 1024)} KB`;
        row.appendChild(left);
        row.appendChild(right);
        list.appendChild(row);
      });
    } catch (_) {}
  }

  // ==========================================================================
  // DOM Utilities & Event Wiring
  // ==========================================================================
  function getValue(id) {
    const el = document.getElementById(id);
    return el ? el.value.trim() : "";
  }

  function setValue(id, val) {
    const el = document.getElementById(id);
    if (el) el.value = val;
  }

  function getChecked(id) {
    const el = document.getElementById(id);
    return el ? Boolean(el.checked) : false;
  }

  function setChecked(id, val) {
    const el = document.getElementById(id);
    if (el) el.checked = val;
  }

  function bindClick(id, handler) {
    const el = document.getElementById(id);
    if (el) el.addEventListener("click", handler);
  }

  document.addEventListener("DOMContentLoaded", async () => {
    initTheme();
    initLoginPage();

    if (!document.getElementById("view-overview")) {
      return;
    }

    initNavigation();

    try {
      const sess = await apiFetch("/api/auth/session");
      if (!sess.authenticated) {
        window.location.href = "/login.html";
        return;
      }
      setText("session-username", sess.username || "Admin");
    } catch (_) {
      return;
    }

    await Promise.all([
      refreshStatus(),
      loadGlobalSettings(),
      loadGuildList(),
      loadSnapshots(),
    ]);

    connectTelemetryWebSocket();
    setInterval(refreshStatus, 10000);

    // Wire buttons
    bindClick("logout-btn", async () => {
      await apiFetch("/auth/logout", { method: "POST" }).catch(() => {});
      window.location.href = "/login.html";
    });

    bindClick("btn-bot-start", async () => {
      try {
        await apiFetch("/api/bot/start", { method: "POST" });
        showToast("Started Discord bot.");
        await refreshStatus();
      } catch (e) {
        showToast(`Failed to start bot: ${e.message}`);
      }
    });

    bindClick("btn-bot-stop", async () => {
      try {
        await apiFetch("/api/bot/stop", { method: "POST" });
        showToast("Stopped Discord bot.");
        await refreshStatus();
      } catch (e) {
        showToast(`Failed to stop bot: ${e.message}`);
      }
    });

    bindClick("btn-bot-restart", async () => {
      try {
        await apiFetch("/api/bot/restart", { method: "POST" });
        showToast("Restarted Discord bot.");
        await refreshStatus();
      } catch (e) {
        showToast(`Failed to restart bot: ${e.message}`);
      }
    });

    bindClick("btn-save-global-name", async () => {
      const username = getValue("bot-global-username");
      try {
        const res = await apiFetch("/api/bot/profile", {
          method: "POST",
          body: JSON.stringify({ username }),
        });
        showToast(`Updated global username to ${res.username}.`);
      } catch (e) {
        showToast(e.message);
      }
    });

    bindClick("btn-clear-logs", () => {
      telemetryLogs = [];
      renderTelemetryConsole();
    });

    const filterEl = document.getElementById("log-filter-level");
    if (filterEl) filterEl.addEventListener("change", renderTelemetryConsole);

    bindClick("btn-save-global-settings", saveGlobalSettings);
    bindClick("btn-refresh-models", () =>
      refreshModelDropdown(getValue("cfg-active-model"), true)
    );

    const presetSel = document.getElementById("cfg-remote-preset");
    if (presetSel) {
      presetSel.addEventListener("change", () => {
        const found = remotePresets.find(
          (p) => p.name.toLowerCase().includes(presetSel.value.toLowerCase())
        );
        if (found && found.base_url) {
          setValue("cfg-remote-base-url", found.base_url);
          setValue("cfg-remote-model", found.default_model);
        }
      });
    }

    bindClick("btn-detect-local", async () => {
      const box = document.getElementById("detect-report-box");
      if (!box) return;
      box.classList.remove("hidden");
      box.textContent = "Probing local Ollama and llama.cpp endpoints...";
      try {
        const res = await apiFetch("/api/ai/detect", { method: "POST" });
        const p = res.probe;
        box.textContent = `Ollama (${p.ollama_url}): online=${p.ollama_online} | llama.cpp (${p.llamacpp_url}): online=${p.llamacpp_online}`;
      } catch (e) {
        box.textContent = `Probe failed: ${e.message}`;
      }
    });

    bindClick("btn-pull-model", async () => {
      const model = getValue("pull-model-input");
      if (!model) return;
      const box = document.getElementById("pull-progress-status");
      if (box) {
        box.classList.remove("hidden");
        box.textContent = `Starting model pull for ${model}...`;
      }
      try {
        const res = await apiFetch("/api/ai/pull", {
          method: "POST",
          body: JSON.stringify({ model }),
        });
        if (box) box.textContent = res.message || `Successfully pulled ${model}.`;
        showToast(`Successfully pulled ${model}.`);
        await refreshModelDropdown(model, true);
      } catch (e) {
        showToast(`Model pull failed: ${e.message}`);
      }
    });

    bindClick("btn-test-search", async () => {
      const query = getValue("test-search-query");
      const container = document.getElementById("test-search-results");
      if (!query || !container) return;
      container.replaceChildren();
      try {
        const res = await apiFetch("/api/search/test", {
          method: "POST",
          body: JSON.stringify({ query }),
        });
        (res.results || []).forEach((item) => {
          const row = document.createElement("div");
          row.className = "usage-row";
          const title = document.createElement("strong");
          title.textContent = item.title;
          const urlSpan = document.createElement("span");
          urlSpan.textContent = item.url;
          row.appendChild(title);
          row.appendChild(urlSpan);
          container.appendChild(row);
        });
      } catch (e) {
        showToast(`Search test failed: ${e.message}`);
      }
    });

    const guildSel = document.getElementById("guild-selector");
    if (guildSel) {
      guildSel.addEventListener("change", async () => {
        currentGuildId = guildSel.value;
        await loadSelectedGuildConfig();
      });
    }

    bindClick("btn-save-guild-config", saveGuildConfig);

    bindClick("btn-reset-guild-quota", async () => {
      if (!currentGuildId) return;
      try {
        await apiFetch(`/api/guilds/${currentGuildId}/quota/reset`, {
          method: "POST",
        });
        showToast("Reset server token usage for current window.");
        await loadSelectedGuildConfig();
      } catch (e) {
        showToast(e.message);
      }
    });

    bindClick("btn-clear-guild-memory", async () => {
      if (!currentGuildId) return;
      try {
        await apiFetch(`/api/guilds/${currentGuildId}/memory/clear`, {
          method: "POST",
        });
        showToast("Cleared volatile conversation memory for server.");
      } catch (e) {
        showToast(e.message);
      }
    });

    bindClick("btn-check-update", async () => {
      const ch = getValue("update-channel-select") || "stable";
      const box = document.getElementById("update-status-box");
      if (!box) return;
      box.textContent = "Checking GitHub Releases...";
      try {
        const res = await apiFetch(`/api/update/check?channel=${encodeURIComponent(ch)}`);
        if (res.error) {
          box.textContent = `Update check error: ${res.error}`;
        } else if (res.update_available) {
          box.textContent = `Update available: v${res.latest_version} (Current: v${res.current_version}, Asset: ${res.binary_asset_name || "none"})`;
        } else {
          box.textContent = `Up to date: v${res.current_version} (${res.channel} channel).`;
        }
      } catch (e) {
        box.textContent = `Error: ${e.message}`;
      }
    });

    bindClick("btn-apply-update", async () => {
      const ch = getValue("update-channel-select") || "stable";
      const box = document.getElementById("update-status-box");
      if (box) box.textContent = "Verifying Ed25519 signature, SHA-256 digest, and staging update...";
      try {
        const res = await apiFetch("/api/update/apply", {
          method: "POST",
          body: JSON.stringify({ channel: ch }),
        });
        if (box) box.textContent = res.message || "Update staged.";
        showToast("Update verified and applied.");
        await loadSnapshots();
      } catch (e) {
        if (box) box.textContent = `Update failed: ${e.message}`;
        showToast(e.message);
      }
    });

    bindClick("btn-create-snapshot", async () => {
      try {
        await apiFetch("/api/update/snapshot", { method: "POST" });
        showToast("Created local database & binary snapshot.");
        await loadSnapshots();
      } catch (e) {
        showToast(e.message);
      }
    });

    bindClick("btn-copy-invite", async () => {
      const url = getValue("invite-url-display");
      if (url && navigator.clipboard) {
        await navigator.clipboard.writeText(url);
        showToast("Copied invite URL to clipboard.");
      }
    });
  });
})();
