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
        refreshActiveModule(target);
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
      setBadgeState("badge-has-reddit-secret", Boolean(s.reddit_client_secret));
      setBadgeState("badge-has-reddit-webhook-secret", Boolean(s.reddit_webhook_secret));

      setValue("cfg-owner-ids", s.bot_owner_id || "");
      setValue("cfg-client-id", s.discord_oauth_client_id || "");
      setValue("cfg-reddit-client-id", s.reddit_client_id || "");
      setValue("cfg-reddit-poll-interval", s.reddit_poll_interval_minutes || 2);
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

    payload.reddit_client_id = getValue("cfg-reddit-client-id");
    payload.reddit_poll_interval_minutes = parseInt(getValue("cfg-reddit-poll-interval"), 10) || 2;
    const redSec = getValue("cfg-reddit-client-secret");
    if (redSec) payload.reddit_client_secret = redSec;
    const redWebhookSec = getValue("cfg-reddit-webhook-secret");
    if (redWebhookSec) payload.reddit_webhook_secret = redWebhookSec;

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
      setValue("cfg-reddit-client-secret", "");
      setValue("cfg-reddit-webhook-secret", "");
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
    const selectors = document.querySelectorAll("#guild-selector, .guild-selector-sync");
    if (selectors.length === 0) return;
    try {
      const res = await apiFetch("/api/guilds");
      const guilds = res.guilds || [];

      selectors.forEach((sel) => {
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
      });

      await loadSelectedGuildConfig();
    } catch (_) {}
  }

  function syncAllGuildSelectors(newGuildId) {
    currentGuildId = newGuildId;
    const selectors = document.querySelectorAll("#guild-selector, .guild-selector-sync");
    selectors.forEach((sel) => {
      sel.value = currentGuildId;
    });
    refreshActiveModule();
  }

  async function refreshActiveModule(optView) {
    const activeNav = document.querySelector(".nav-item.active[data-view]");
    const view = optView || (activeNav ? activeNav.getAttribute("data-view") : "");
    switch (view) {
      case "servers":
        await loadSelectedGuildConfig();
        break;
      case "moderation":
        await loadModCases();
        break;
      case "automod":
        await loadAutomodConfig();
        break;
      case "logging":
        await loadLoggingConfig();
        break;
      case "greetings":
        await loadGreetingsConfig();
        break;
      case "reaction-roles":
        await loadReactionRoles();
        break;
      case "triggers":
        await loadCustomTriggers();
        break;
    }
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
      await loadGuildRedditSubscriptions(currentGuildId);
    } catch (e) {
      showToast(`Failed to load server config: ${e.message}`);
    }
  }

  function renderMetadataChips(meta) {
    const textChContainer = document.getElementById("chips-text-channels");
    const forumChContainer = document.getElementById("chips-forum-channels");
    const roleContainer = document.getElementById("chips-roles");
    const redditChSelect = document.getElementById("reddit-new-channel");

    if (textChContainer) textChContainer.replaceChildren();
    if (forumChContainer) forumChContainer.replaceChildren();
    if (roleContainer) roleContainer.replaceChildren();
    if (redditChSelect) redditChSelect.replaceChildren();

    if (!meta) return;

    (meta.channels || []).forEach((ch) => {
      if (ch.kind !== "forum" && ch.kind !== "category" && redditChSelect) {
        const opt = document.createElement("option");
        opt.value = ch.id;
        opt.textContent = `#${ch.name}`;
        redditChSelect.appendChild(opt);
      }
    });

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

  async function loadGuildRedditSubscriptions(guildId) {
    const listEl = document.getElementById("reddit-subs-list");
    if (!listEl) return;
    if (!guildId || guildId === "0") {
      listEl.innerHTML = '<p class="muted">Select a connected server to manage subreddit subscriptions.</p>';
      return;
    }
    try {
      const res = await apiFetch(`/api/guilds/${guildId}/reddit`);
      const subs = res.subscriptions || [];
      if (subs.length === 0) {
        listEl.innerHTML = '<p class="muted">No active subreddit monitors for this server.</p>';
        return;
      }
      listEl.replaceChildren();
      subs.forEach((sub) => {
        const row = document.createElement("div");
        row.className = "reddit-sub-item";
        row.style.cssText = "display: flex; justify-content: space-between; align-items: center; padding: 0.6rem 0.8rem; background: var(--bg-card-sub, #1f2430); border-radius: 6px; margin-bottom: 0.5rem;";
        
        const details = document.createElement("div");
        let filters = [];
        if (sub.filter_nsfw) filters.push("NSFW Filter");
        if (sub.min_score > 0) filters.push(`Min Score: ${sub.min_score}`);
        if (sub.flair_filter) filters.push(`Flairs: ${sub.flair_filter}`);
        if (sub.keyword_filter) filters.push(`Keywords: ${sub.keyword_filter}`);
        
        details.innerHTML = `<strong>r/${sub.subreddit}</strong> ➔ <span class="badge badge-neutral">&lt;#${sub.channel_id}&gt;</span> ${filters.length ? `<small style="margin-left: 0.5rem; opacity: 0.8">(${filters.join(", ")})</small>` : ""}`;
        
        const delBtn = document.createElement("button");
        delBtn.type = "button";
        delBtn.className = "btn btn-danger btn-sm";
        delBtn.textContent = "Unsubscribe";
        delBtn.addEventListener("click", async () => {
          if (!confirm(`Delete feed subscription for r/${sub.subreddit}?`)) return;
          try {
            await apiFetch(`/api/guilds/${guildId}/reddit/${sub.id}`, { method: "DELETE" });
            showToast(`Unsubscribed from r/${sub.subreddit}`);
            await loadGuildRedditSubscriptions(guildId);
          } catch (e) {
            showToast(`Failed to unsubscribe: ${e.message}`);
          }
        });

        row.appendChild(details);
        row.appendChild(delBtn);
        listEl.appendChild(row);
      });
    } catch (e) {
      listEl.innerHTML = `<p class="muted">Error loading subscriptions: ${e.message}</p>`;
    }
  }

  async function addRedditSubscription() {
    if (!currentGuildId || currentGuildId === "0") {
      showToast("Please select a connected Discord server first.");
      return;
    }
    const subName = getValue("reddit-new-sub").trim();
    const channelId = getValue("reddit-new-channel");
    if (!subName) {
      showToast("Please enter a subreddit name.");
      return;
    }
    if (!channelId) {
      showToast("Please select a target channel.");
      return;
    }
    const minScore = parseInt(getValue("reddit-new-min-score"), 10) || 0;
    const backfill = parseInt(getValue("reddit-new-backfill"), 10) || 0;
    const flairs = getValue("reddit-new-flairs").trim() || null;
    const keywords = getValue("reddit-new-keywords").trim() || null;
    const filterNsfw = getChecked("reddit-new-nsfw");

    try {
      await apiFetch(`/api/guilds/${currentGuildId}/reddit`, {
        method: "POST",
        body: JSON.stringify({
          guild_id: parseInt(currentGuildId, 10),
          channel_id: parseInt(channelId, 10),
          subreddit: subName,
          filter_nsfw: filterNsfw,
          min_score: minScore,
          flair_filter: flairs,
          keyword_filter: keywords,
          backfill_count: backfill,
        }),
      });
      showToast(`Successfully subscribed to r/${subName}`);
      setValue("reddit-new-sub", "");
      setValue("reddit-new-flairs", "");
      setValue("reddit-new-keywords", "");
      setValue("reddit-new-backfill", "0");
      await loadGuildRedditSubscriptions(currentGuildId);
    } catch (e) {
      showToast(`Failed to subscribe: ${e.message}`);
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

  // ==========================================================================
  // Modal Helpers
  // ==========================================================================
  function openModal(modalId) {
    const el = document.getElementById(modalId);
    if (el) el.classList.remove("hidden");
  }

  function closeModal(modalId) {
    const el = document.getElementById(modalId);
    if (el) el.classList.add("hidden");
  }

  function initModals() {
    document.querySelectorAll("[data-modal]").forEach((btn) => {
      btn.addEventListener("click", () => {
        const id = btn.getAttribute("data-modal");
        closeModal(id);
      });
    });
    window.addEventListener("keydown", (e) => {
      if (e.key === "Escape") {
        document.querySelectorAll(".modal-backdrop:not(.hidden)").forEach((m) => {
          m.classList.add("hidden");
        });
      }
    });
  }

  // ==========================================================================
  // 1. Moderation Cases & Strikes Module
  // ==========================================================================
  async function loadModCases(filterUserId = null) {
    const tbody = document.getElementById("mod-cases-table-body");
    const countBadge = document.getElementById("mod-cases-count-badge");
    const summaryBox = document.getElementById("mod-strikes-summary");
    if (!tbody || !currentGuildId) return;

    tbody.replaceChildren();
    const loadingTr = document.createElement("tr");
    const loadingTd = document.createElement("td");
    loadingTd.colSpan = 8;
    loadingTd.className = "muted text-center";
    loadingTd.textContent = "Loading cases & strikes...";
    loadingTr.appendChild(loadingTd);
    tbody.appendChild(loadingTr);

    try {
      const casesUrl = filterUserId
        ? `/api/guilds/${currentGuildId}/cases?user_id=${encodeURIComponent(filterUserId)}`
        : `/api/guilds/${currentGuildId}/cases`;
      const strikesUrl = filterUserId
        ? `/api/guilds/${currentGuildId}/strikes?user_id=${encodeURIComponent(filterUserId)}`
        : `/api/guilds/${currentGuildId}/strikes`;

      const [casesRes, strikesRes] = await Promise.all([
        apiFetch(casesUrl),
        apiFetch(strikesUrl),
      ]);

      const cases = casesRes.cases || [];
      if (countBadge) countBadge.textContent = `${cases.length} cases`;

      if (summaryBox) {
        if (filterUserId) {
          const actCount = strikesRes.active_count ?? 0;
          const totalStrikes = (strikesRes.strikes || []).length;
          summaryBox.textContent = `User ${filterUserId} has ${actCount} active strike points (${totalStrikes} total recorded strike entries).`;
        } else {
          const totalStrikes = (strikesRes.strikes || []).length;
          summaryBox.textContent = `Server has ${totalStrikes} total recorded strike entries across all members.`;
        }
      }

      tbody.replaceChildren();
      if (cases.length === 0) {
        const tr = document.createElement("tr");
        const td = document.createElement("td");
        td.colSpan = 8;
        td.className = "muted text-center";
        td.textContent = filterUserId
          ? `No cases found for user ${filterUserId}.`
          : "No moderation cases recorded for this server.";
        tr.appendChild(td);
        tbody.appendChild(tr);
        return;
      }

      cases.forEach((c) => {
        const tr = document.createElement("tr");

        // Case Number
        const tdNum = document.createElement("td");
        const badgeNum = document.createElement("span");
        badgeNum.className = "badge badge-neutral";
        badgeNum.textContent = `#${c.case_number}`;
        tdNum.appendChild(badgeNum);
        tr.appendChild(tdNum);

        // Target User
        const tdUser = document.createElement("td");
        const codeUser = document.createElement("code");
        codeUser.textContent = String(c.user_id);
        tdUser.appendChild(codeUser);
        tr.appendChild(tdUser);

        // Moderator
        const tdMod = document.createElement("td");
        tdMod.textContent = c.moderator_id === 0 ? "System / Automod" : String(c.moderator_id);
        tr.appendChild(tdMod);

        // Action Type
        const tdAction = document.createElement("td");
        const badgeAction = document.createElement("span");
        const act = (c.action_type || "").toLowerCase();
        let badgeClass = "badge-neutral";
        if (act === "ban" || act === "kick") badgeClass = "badge-danger";
        else if (act === "timeout" || act === "tempban") badgeClass = "badge-warning";
        else if (act === "softban") badgeClass = "badge-amber";
        badgeAction.className = `badge ${badgeClass}`;
        badgeAction.textContent = c.action_type;
        tdAction.appendChild(badgeAction);
        tr.appendChild(tdAction);

        // Reason
        const tdReason = document.createElement("td");
        tdReason.textContent = c.reason || "No reason provided";
        tr.appendChild(tdReason);

        // Duration / Expires
        const tdDuration = document.createElement("td");
        if (c.duration_seconds) {
          tdDuration.textContent = `${c.duration_seconds}s`;
        } else if (c.expires_at) {
          const exp = new Date(c.expires_at * 1000).toLocaleString();
          tdDuration.textContent = `Until ${exp}`;
        } else {
          tdDuration.textContent = "Permanent / N/A";
        }
        tr.appendChild(tdDuration);

        // Status
        const tdStatus = document.createElement("td");
        const badgeStatus = document.createElement("span");
        badgeStatus.className = c.active ? "badge badge-online" : "badge badge-offline";
        badgeStatus.textContent = c.active ? "Active" : "Inactive";
        tdStatus.appendChild(badgeStatus);
        tr.appendChild(tdStatus);

        // Actions
        const tdActions = document.createElement("td");
        const actionsWrap = document.createElement("div");
        actionsWrap.style.display = "flex";
        actionsWrap.style.gap = "0.35rem";

        const btnEdit = document.createElement("button");
        btnEdit.type = "button";
        btnEdit.className = "btn btn-secondary btn-sm";
        btnEdit.textContent = "Edit Reason";
        btnEdit.addEventListener("click", () => {
          setValue("edit-reason-case-id", String(c.id));
          setValue("edit-reason-text", c.reason || "");
          openModal("modal-edit-reason");
        });
        actionsWrap.appendChild(btnEdit);

        if (c.active) {
          const btnDeact = document.createElement("button");
          btnDeact.type = "button";
          btnDeact.className = "btn btn-danger btn-sm";
          btnDeact.textContent = "Deactivate";
          btnDeact.addEventListener("click", async () => {
            try {
              await apiFetch(`/api/guilds/${currentGuildId}/cases/${c.id}`, {
                method: "DELETE",
              });
              showToast(`Deactivated case #${c.case_number}.`);
              const fUser = getValue("mod-filter-user");
              await loadModCases(fUser || null);
            } catch (err) {
              showToast(`Failed to deactivate case: ${err.message}`);
            }
          });
          actionsWrap.appendChild(btnDeact);
        }

        tdActions.appendChild(actionsWrap);
        tr.appendChild(tdActions);

        tbody.appendChild(tr);
      });
    } catch (err) {
      tbody.replaceChildren();
      const tr = document.createElement("tr");
      const td = document.createElement("td");
      td.colSpan = 8;
      td.className = "muted text-center";
      td.textContent = `Error loading cases: ${err.message}`;
      tr.appendChild(td);
      tbody.appendChild(tr);
    }
  }

  async function submitManualCase() {
    const rawUserId = getValue("mcase-user-id");
    if (!rawUserId) {
      showToast("Target Discord User ID is required.");
      return;
    }
    const actionType = getValue("mcase-action-type") || "warn";
    const reason = getValue("mcase-reason") || "Manual action";
    const rawDuration = getValue("mcase-duration");
    const durationSeconds = rawDuration ? parseInt(rawDuration, 10) : null;

    try {
      await apiFetch(`/api/guilds/${currentGuildId}/cases`, {
        method: "POST",
        body: JSON.stringify({
          user_id: rawUserId.trim(),
          action_type: actionType,
          reason: reason.trim(),
          duration_seconds: durationSeconds,
        }),
      });
      showToast(`Created ${actionType} case for user ${rawUserId}.`);
      closeModal("modal-manual-case");
      setValue("mcase-user-id", "");
      setValue("mcase-reason", "Manual disciplinary action");
      setValue("mcase-duration", "");
      const fUser = getValue("mod-filter-user");
      await loadModCases(fUser || null);
    } catch (err) {
      showToast(`Failed to create case: ${err.message}`);
    }
  }

  async function submitEditReason() {
    const caseId = getValue("edit-reason-case-id");
    const newReason = getValue("edit-reason-text");
    if (!caseId) return;

    try {
      await apiFetch(`/api/guilds/${currentGuildId}/cases/${caseId}/reason`, {
        method: "PUT",
        body: JSON.stringify({ reason: newReason.trim() }),
      });
      showToast("Updated case reason.");
      closeModal("modal-edit-reason");
      const fUser = getValue("mod-filter-user");
      await loadModCases(fUser || null);
    } catch (err) {
      showToast(`Failed to update reason: ${err.message}`);
    }
  }

  async function triggerManualStrikeDecay() {
    if (!currentGuildId) return;
    try {
      const res = await apiFetch(`/api/guilds/${currentGuildId}/strikes/decay`, {
        method: "POST",
        body: JSON.stringify({}),
      });
      showToast(`Decayed ${res.decayed_count || 0} expired strikes.`);
      const fUser = getValue("mod-filter-user");
      await loadModCases(fUser || null);
    } catch (err) {
      showToast(`Failed to decay strikes: ${err.message}`);
    }
  }

  // ==========================================================================
  // 2. Automod & Strike Ladder Module
  // ==========================================================================
  let currentAutomodConfig = null;

  async function loadAutomodConfig() {
    if (!currentGuildId) return;
    try {
      const res = await apiFetch(`/api/guilds/${currentGuildId}/automod`);
      currentAutomodConfig = res.config || res;

      setChecked("am-enabled", Boolean(currentAutomodConfig.enabled));
      setChecked("am-ai-mod", Boolean(currentAutomodConfig.ai_moderation_enabled));
      setChecked("am-anti-spam", Boolean(currentAutomodConfig.anti_spam_enabled));
      setValue("am-spam-max", String(currentAutomodConfig.anti_spam_max_messages ?? 5));
      setValue("am-spam-interval", String(currentAutomodConfig.anti_spam_interval_seconds ?? 5));
      setChecked("am-anti-invites", Boolean(currentAutomodConfig.anti_invites_enabled));
      setChecked("am-anti-links", Boolean(currentAutomodConfig.anti_links_enabled));
      setChecked("am-anti-caps", Boolean(currentAutomodConfig.anti_caps_enabled));
      setValue("am-caps-min", String(currentAutomodConfig.anti_caps_min_chars ?? 10));
      setValue("am-caps-pct", String(currentAutomodConfig.anti_caps_percent ?? 70));
      setValue("am-mention-max", String(currentAutomodConfig.anti_mention_max ?? 5));

      setValue("am-blocked-words", (currentAutomodConfig.blocked_words || []).join(", "));
      setValue("am-whitelisted-domains", (currentAutomodConfig.whitelisted_domains || []).join(", "));
      setValue("am-ignored-channels", (currentAutomodConfig.ignored_channels || []).join(", "));
      setValue("am-ignored-roles", (currentAutomodConfig.ignored_roles || []).join(", "));

      const weights = currentAutomodConfig.strike_weights || {};
      setValue("am-weight-spam", String(weights.spam ?? 2));
      setValue("am-weight-invite", String(weights.invite ?? 3));
      setValue("am-weight-badword", String(weights.bad_word ?? 2));
      setValue("am-weight-caps", String(weights.caps ?? 1));
      setValue("am-weight-mention", String(weights.mention ?? 3));
      setValue("am-decay-hours", String(currentAutomodConfig.strike_decay_hours ?? 168));

      renderLadderTable();
    } catch (err) {
      showToast(`Failed to load automod config: ${err.message}`);
    }
  }

  function renderLadderTable() {
    const tbody = document.getElementById("am-ladder-table-body");
    if (!tbody || !currentAutomodConfig) return;
    tbody.replaceChildren();

    const ladder = currentAutomodConfig.punishment_ladder || [];
    if (ladder.length === 0) {
      const tr = document.createElement("tr");
      const td = document.createElement("td");
      td.colSpan = 4;
      td.className = "muted text-center";
      td.textContent = "No escalation steps configured. Add a step below.";
      tr.appendChild(td);
      tbody.appendChild(tr);
      return;
    }

    ladder.forEach((step, idx) => {
      const tr = document.createElement("tr");

      const tdStrikes = document.createElement("td");
      const badgeS = document.createElement("span");
      badgeS.className = "badge badge-neutral";
      badgeS.textContent = `${step.strikes} Strikes`;
      tdStrikes.appendChild(badgeS);
      tr.appendChild(tdStrikes);

      const tdAction = document.createElement("td");
      const badgeA = document.createElement("span");
      const act = (step.action || "").toLowerCase();
      let bClass = "badge-neutral";
      if (act === "ban" || act === "kick") bClass = "badge-danger";
      else if (act === "timeout") bClass = "badge-warning";
      badgeA.className = `badge ${bClass}`;
      badgeA.textContent = step.action.toUpperCase();
      tdAction.appendChild(badgeA);
      tr.appendChild(tdAction);

      const tdDur = document.createElement("td");
      tdDur.textContent = step.duration_seconds ? `${step.duration_seconds}s` : "Permanent / N/A";
      tr.appendChild(tdDur);

      const tdDel = document.createElement("td");
      const btnDel = document.createElement("button");
      btnDel.type = "button";
      btnDel.className = "btn btn-danger btn-sm";
      btnDel.textContent = "Remove";
      btnDel.addEventListener("click", () => {
        currentAutomodConfig.punishment_ladder.splice(idx, 1);
        renderLadderTable();
      });
      tdDel.appendChild(btnDel);
      tr.appendChild(tdDel);

      tbody.appendChild(tr);
    });
  }

  function addLadderStep() {
    if (!currentAutomodConfig) return;
    const strikes = parseInt(getValue("am-new-step-strikes"), 10) || 1;
    const action = getValue("am-new-step-action") || "warn";
    const durRaw = getValue("am-new-step-duration");
    const duration_seconds = durRaw ? parseInt(durRaw, 10) : null;

    if (!currentAutomodConfig.punishment_ladder) {
      currentAutomodConfig.punishment_ladder = [];
    }
    currentAutomodConfig.punishment_ladder.push({
      strikes,
      action,
      duration_seconds,
    });
    currentAutomodConfig.punishment_ladder.sort((a, b) => a.strikes - b.strikes);
    renderLadderTable();
    setValue("am-new-step-duration", "");
  }

  async function saveAutomodConfig() {
    if (!currentGuildId || !currentAutomodConfig) return;

    const parseList = (val) =>
      val
        .split(/[,
]/)
        .map((s) => s.trim())
        .filter((s) => s.length > 0);

    const parseNumList = (val) =>
      parseList(val)
        .map((s) => parseInt(s, 10))
        .filter((n) => !isNaN(n));

    const weights = {
      spam: parseInt(getValue("am-weight-spam"), 10) || 2,
      invite: parseInt(getValue("am-weight-invite"), 10) || 3,
      bad_word: parseInt(getValue("am-weight-badword"), 10) || 2,
      caps: parseInt(getValue("am-weight-caps"), 10) || 1,
      mention: parseInt(getValue("am-weight-mention"), 10) || 3,
    };

    const payload = {
      guild_id: parseInt(currentGuildId, 10),
      enabled: getChecked("am-enabled"),
      anti_spam_enabled: getChecked("am-anti-spam"),
      anti_spam_max_messages: parseInt(getValue("am-spam-max"), 10) || 5,
      anti_spam_interval_seconds: parseInt(getValue("am-spam-interval"), 10) || 5,
      anti_invites_enabled: getChecked("am-anti-invites"),
      anti_links_enabled: getChecked("am-anti-links"),
      anti_caps_enabled: getChecked("am-anti-caps"),
      anti_caps_min_chars: parseInt(getValue("am-caps-min"), 10) || 10,
      anti_caps_percent: parseInt(getValue("am-caps-pct"), 10) || 70,
      anti_mention_max: parseInt(getValue("am-mention-max"), 10) || 5,
      blocked_words: parseList(getValue("am-blocked-words")),
      whitelisted_domains: parseList(getValue("am-whitelisted-domains")),
      ignored_channels: parseNumList(getValue("am-ignored-channels")),
      ignored_roles: parseNumList(getValue("am-ignored-roles")),
      strike_weights: weights,
      strike_decay_hours: parseInt(getValue("am-decay-hours"), 10) || 168,
      punishment_ladder: currentAutomodConfig.punishment_ladder || [],
      ai_moderation_enabled: getChecked("am-ai-mod"),
    };

    try {
      await apiFetch(`/api/guilds/${currentGuildId}/automod`, {
        method: "PUT",
        body: JSON.stringify(payload),
      });
      showToast("Automod configuration saved.");
    } catch (err) {
      showToast(`Failed to save automod config: ${err.message}`);
    }
  }

  // ==========================================================================
  // 3. Audit Logging Module
  // ==========================================================================
  async function loadLoggingConfig() {
    if (!currentGuildId) return;
    try {
      const res = await apiFetch(`/api/guilds/${currentGuildId}/logging`);
      const cfg = res.config || res;

      setValue("log-unified-channel", cfg.unified_channel_id ? String(cfg.unified_channel_id) : "");
      setValue("log-mod-channel", cfg.mod_channel_id ? String(cfg.mod_channel_id) : "");
      setValue("log-message-channel", cfg.message_channel_id ? String(cfg.message_channel_id) : "");
      setValue("log-member-channel", cfg.member_channel_id ? String(cfg.member_channel_id) : "");
      setValue("log-server-channel", cfg.server_channel_id ? String(cfg.server_channel_id) : "");
      setValue("log-voice-channel", cfg.voice_channel_id ? String(cfg.voice_channel_id) : "");

      setChecked("log-bots", Boolean(cfg.log_bots));
      setValue("log-ignored-channels", (cfg.ignored_channels || []).join(", "));
      setValue("log-ignored-roles", (cfg.ignored_roles || []).join(", "));

      const colNum = cfg.embed_color || 3553599;
      const hex = `#${colNum.toString(16).padStart(6, "0")}`;
      setValue("log-embed-color-picker", hex);
      setValue("log-embed-color-hex", hex);
    } catch (err) {
      showToast(`Failed to load logging config: ${err.message}`);
    }
  }

  async function saveLoggingConfig() {
    if (!currentGuildId) return;

    const parseOptId = (val) => {
      const trimmed = val ? val.trim() : "";
      return trimmed.length > 0 ? trimmed : null;
    };

    const parseNumList = (val) =>
      (val || "")
        .split(",")
        .map((s) => s.trim())
        .filter((s) => s.length > 0)
        .map((s) => parseInt(s, 10))
        .filter((n) => !isNaN(n));

    const hexVal = (getValue("log-embed-color-hex") || "#36393f").replace("#", "");
    const embedColor = parseInt(hexVal, 16) || 3553599;

    const payload = {
      guild_id: parseInt(currentGuildId, 10),
      unified_channel_id: parseOptId(getValue("log-unified-channel")),
      mod_channel_id: parseOptId(getValue("log-mod-channel")),
      message_channel_id: parseOptId(getValue("log-message-channel")),
      member_channel_id: parseOptId(getValue("log-member-channel")),
      server_channel_id: parseOptId(getValue("log-server-channel")),
      voice_channel_id: parseOptId(getValue("log-voice-channel")),
      ignored_channels: parseNumList(getValue("log-ignored-channels")),
      ignored_roles: parseNumList(getValue("log-ignored-roles")),
      log_bots: getChecked("log-bots"),
      embed_color: embedColor,
    };

    try {
      await apiFetch(`/api/guilds/${currentGuildId}/logging`, {
        method: "PUT",
        body: JSON.stringify(payload),
      });
      showToast("Logging configuration saved.");
    } catch (err) {
      showToast(`Failed to save logging config: ${err.message}`);
    }
  }

  // ==========================================================================
  // 4. Greetings, Farewells & Autorole Module
  // ==========================================================================
  async function loadGreetingsConfig() {
    if (!currentGuildId) return;
    try {
      const res = await apiFetch(`/api/guilds/${currentGuildId}/greetings`);
      const cfg = res.config || res;

      setValue("greet-welcome-channel", cfg.welcome_channel_id ? String(cfg.welcome_channel_id) : "");
      setValue("greet-welcome-msg", cfg.welcome_message || "");
      setValue("greet-welcome-embed", cfg.welcome_embed_json || "");
      setChecked("greet-dm-enabled", Boolean(cfg.welcome_dm_enabled));
      setValue("greet-dm-msg", cfg.welcome_dm_message || "");

      setValue("greet-farewell-channel", cfg.farewell_channel_id ? String(cfg.farewell_channel_id) : "");
      setValue("greet-farewell-msg", cfg.farewell_message || "");
      setValue("greet-farewell-embed", cfg.farewell_embed_json || "");

      setValue("greet-member-roles", (cfg.member_autorole_ids || []).join(", "));
      setValue("greet-bot-roles", (cfg.bot_autorole_ids || []).join(", "));
      setValue("greet-delay-seconds", String(cfg.autorole_delay_seconds ?? 0));

      updateGreetingsEmbedPreview();
    } catch (err) {
      showToast(`Failed to load greetings config: ${err.message}`);
    }
  }

  function updateGreetingsEmbedPreview() {
    const rawEmbed = getValue("greet-welcome-embed") || "";
    const rawMsg = getValue("greet-welcome-msg") || "";

    const titleEl = document.getElementById("embed-preview-title");
    const descEl = document.getElementById("embed-preview-desc");
    const footerEl = document.getElementById("embed-preview-footer");
    const barEl = document.getElementById("embed-preview-bar");

    let title = "Welcome to the Server!";
    let desc = rawMsg || "Welcome {user} to {server}!";
    let footer = "Silas Blue • Member #1337";
    let color = "#38bdf8";

    // Check for TagScript embed directives
    const titleMatch = rawEmbed.match(/\{title:\s*([^}]+)\}/i);
    if (titleMatch) title = titleMatch[1].trim();

    const descMatch = rawEmbed.match(/\{description:\s*([^}]+)\}/i);
    if (descMatch) desc = descMatch[1].trim();

    const footerMatch = rawEmbed.match(/\{footer:\s*([^}]+)\}/i);
    if (footerMatch) footer = footerMatch[1].trim();

    const colorMatch = rawEmbed.match(/\{color:\s*#?([0-9a-fA-F]{6})\}/i);
    if (colorMatch) color = `#${colorMatch[1]}`;

    if (titleEl) titleEl.textContent = title;
    if (descEl) descEl.textContent = desc;
    if (footerEl) footerEl.textContent = footer;
    if (barEl) barEl.style.backgroundColor = color;
  }

  async function saveGreetingsConfig() {
    if (!currentGuildId) return;

    const parseOptId = (val) => {
      const trimmed = val ? val.trim() : "";
      return trimmed.length > 0 ? trimmed : null;
    };

    const parseNumList = (val) =>
      (val || "")
        .split(",")
        .map((s) => s.trim())
        .filter((s) => s.length > 0)
        .map((s) => parseInt(s, 10))
        .filter((n) => !isNaN(n));

    const payload = {
      guild_id: parseInt(currentGuildId, 10),
      welcome_channel_id: parseOptId(getValue("greet-welcome-channel")),
      welcome_message: getValue("greet-welcome-msg") || "",
      welcome_embed_json: getValue("greet-welcome-embed") || "",
      welcome_dm_enabled: getChecked("greet-dm-enabled"),
      welcome_dm_message: getValue("greet-dm-msg") || "",
      farewell_channel_id: parseOptId(getValue("greet-farewell-channel")),
      farewell_message: getValue("greet-farewell-msg") || "",
      farewell_embed_json: getValue("greet-farewell-embed") || "",
      member_autorole_ids: parseNumList(getValue("greet-member-roles")),
      bot_autorole_ids: parseNumList(getValue("greet-bot-roles")),
      autorole_delay_seconds: parseInt(getValue("greet-delay-seconds"), 10) || 0,
    };

    try {
      await apiFetch(`/api/guilds/${currentGuildId}/greetings`, {
        method: "PUT",
        body: JSON.stringify(payload),
      });
      showToast("Greetings & Autorole configuration saved.");
    } catch (err) {
      showToast(`Failed to save greetings config: ${err.message}`);
    }
  }

  // ==========================================================================
  // 5. Reaction Roles Module
  // ==========================================================================
  let rrEntriesBuilder = [];

  async function loadReactionRoles() {
    const container = document.getElementById("rr-groups-container");
    const countBadge = document.getElementById("rr-groups-count-badge");
    if (!container || !currentGuildId) return;

    container.replaceChildren();
    const loadingP = document.createElement("p");
    loadingP.className = "muted";
    loadingP.textContent = "Loading reaction role groups...";
    container.appendChild(loadingP);

    try {
      const res = await apiFetch(`/api/guilds/${currentGuildId}/reaction-roles`);
      const groups = res.groups || [];
      if (countBadge) countBadge.textContent = `${groups.length} groups`;

      container.replaceChildren();
      if (groups.length === 0) {
        const p = document.createElement("p");
        p.className = "muted";
        p.textContent = "No reaction role groups created yet. Click '+ Create Reaction Role Group' to add one.";
        container.appendChild(p);
        return;
      }

      groups.forEach((g) => {
        const card = document.createElement("div");
        card.className = "rr-group-card";

        // Top Row: Channel & Message
        const topRow = document.createElement("div");
        topRow.className = "card-header-split";

        const titleDiv = document.createElement("div");
        const titleH4 = document.createElement("strong");
        titleH4.textContent = `Channel #${g.channel_id}`;
        titleDiv.appendChild(titleH4);

        const subP = document.createElement("div");
        subP.className = "help-text";
        subP.textContent = `Message ID: ${g.message_id}`;
        titleDiv.appendChild(subP);
        topRow.appendChild(titleDiv);

        const badgesDiv = document.createElement("div");
        badgesDiv.style.display = "flex";
        badgesDiv.style.gap = "0.35rem";

        const badgeType = document.createElement("span");
        badgeType.className = "badge badge-cyan";
        badgeType.textContent = (g.group_type || "normal").toUpperCase();
        badgesDiv.appendChild(badgeType);

        const badgeComp = document.createElement("span");
        badgeComp.className = "badge badge-purple";
        badgeComp.textContent = (g.component_type || "reaction").toUpperCase();
        badgesDiv.appendChild(badgeComp);

        topRow.appendChild(badgesDiv);
        card.appendChild(topRow);

        if (g.required_role_id) {
          const reqDiv = document.createElement("div");
          reqDiv.className = "status-note";
          reqDiv.textContent = `Requires Role ID: ${g.required_role_id}`;
          card.appendChild(reqDiv);
        }

        // Entries List
        const entriesTitle = document.createElement("strong");
        entriesTitle.style.fontSize = "0.82rem";
        entriesTitle.style.color = "var(--text-secondary)";
        entriesTitle.textContent = "Role Mappings:";
        card.appendChild(entriesTitle);

        const entriesList = document.createElement("div");
        entriesList.className = "rr-entries-list";

        const entries = g.entries || [];
        if (entries.length === 0) {
          const noEnt = document.createElement("span");
          noEnt.className = "muted";
          noEnt.textContent = "No mapped roles.";
          entriesList.appendChild(noEnt);
        } else {
          entries.forEach((e) => {
            const chip = document.createElement("span");
            chip.className = "rr-entry-badge";
            const label = e.button_label ? ` "${e.button_label}"` : "";
            chip.textContent = `${e.emoji_or_custom_id} ➔ Role ${e.role_id}${label}`;
            entriesList.appendChild(chip);
          });
        }
        card.appendChild(entriesList);

        // Delete Button
        const botRow = document.createElement("div");
        botRow.style.display = "flex";
        botRow.style.justifyContent = "flex-end";
        botRow.style.marginTop = "0.5rem";

        const btnDel = document.createElement("button");
        btnDel.type = "button";
        btnDel.className = "btn btn-danger btn-sm";
        btnDel.textContent = "Delete Group";
        btnDel.addEventListener("click", async () => {
          try {
            await apiFetch(`/api/guilds/${currentGuildId}/reaction-roles/${g.id}`, {
              method: "DELETE",
            });
            showToast("Reaction role group deleted.");
            await loadReactionRoles();
          } catch (err) {
            showToast(`Failed to delete group: ${err.message}`);
          }
        });
        botRow.appendChild(btnDel);
        card.appendChild(botRow);

        container.appendChild(card);
      });
    } catch (err) {
      container.replaceChildren();
      const p = document.createElement("p");
      p.className = "muted";
      p.textContent = `Error loading reaction roles: ${err.message}`;
      container.appendChild(p);
    }
  }

  function addRrEntryRow(emoji = "", roleId = "", label = "", style = "primary") {
    const list = document.getElementById("rr-entries-builder");
    if (!list) return;

    const row = document.createElement("div");
    row.className = "form-row";
    row.style.alignItems = "center";

    const inEmoji = document.createElement("input");
    inEmoji.type = "text";
    inEmoji.className = "input-text";
    inEmoji.placeholder = "Emoji / Custom ID (e.g. 👍)";
    inEmoji.value = emoji;
    inEmoji.style.flex = "1";
    row.appendChild(inEmoji);

    const inRole = document.createElement("input");
    inRole.type = "text";
    inRole.className = "input-text";
    inRole.placeholder = "Role ID";
    inRole.value = roleId;
    inRole.style.flex = "1";
    row.appendChild(inRole);

    const inLabel = document.createElement("input");
    inLabel.type = "text";
    inLabel.className = "input-text";
    inLabel.placeholder = "Button Label (optional)";
    inLabel.value = label;
    inLabel.style.flex = "1";
    row.appendChild(inLabel);

    const selStyle = document.createElement("select");
    selStyle.className = "input-select";
    selStyle.style.width = "110px";
    ["primary", "secondary", "success", "danger"].forEach((st) => {
      const opt = document.createElement("option");
      opt.value = st;
      opt.textContent = st;
      if (st === style) opt.selected = true;
      selStyle.appendChild(opt);
    });
    row.appendChild(selStyle);

    const btnRemove = document.createElement("button");
    btnRemove.type = "button";
    btnRemove.className = "btn btn-danger btn-sm";
    btnRemove.textContent = "×";
        btnRemove.addEventListener("click", () => row.remove());
    row.appendChild(btnRemove);

    list.appendChild(row);
  }

  async function submitCreateReactionRole() {
    const chId = getValue("rr-input-channel");
    const msgId = getValue("rr-input-message");
    if (!chId || !msgId) {
      showToast("Channel ID and Message ID are required.");
      return;
    }

    const groupType = getValue("rr-input-grouptype") || "normal";
    const compType = getValue("rr-input-component") || "reaction";
    const reqRoleRaw = getValue("rr-input-reqrole");
    const requiredRoleId = reqRoleRaw ? reqRoleRaw.trim() : null;

    const entries = [];
    const rows = document.querySelectorAll("#rr-entries-builder .form-row");
    rows.forEach((r) => {
      const inputs = r.querySelectorAll("input");
      const sel = r.querySelector("select");
      if (inputs.length >= 2) {
        const emoji = inputs[0].value.trim();
        const role = inputs[1].value.trim();
        const lbl = inputs[2] ? inputs[2].value.trim() : "";
        const style = sel ? sel.value : "primary";
        if (emoji && role) {
          entries.push({
            emoji_or_custom_id: emoji,
            role_id: role,
            button_label: lbl.length > 0 ? lbl : null,
            button_style: style,
          });
        }
      }
    });

    const payload = {
      channel_id: chId.trim(),
      message_id: msgId.trim(),
      group_type: groupType,
      required_role_id: requiredRoleId,
      component_type: compType,
      entries,
    };

    try {
      await apiFetch(`/api/guilds/${currentGuildId}/reaction-roles`, {
        method: "POST",
        body: JSON.stringify(payload),
      });
      showToast("Reaction role group created.");
      closeModal("modal-create-rr");
      setValue("rr-input-channel", "");
      setValue("rr-input-message", "");
      setValue("rr-input-reqrole", "");
      const bList = document.getElementById("rr-entries-builder");
      if (bList) bList.replaceChildren();
      await loadReactionRoles();
    } catch (err) {
      showToast(`Failed to create reaction role group: ${err.message}`);
    }
  }

  // ==========================================================================
  // 6. Custom Triggers & TagScript Simulator Module
  // ==========================================================================
  async function loadCustomTriggers() {
    const tbody = document.getElementById("triggers-table-body");
    const countBadge = document.getElementById("triggers-count-badge");
    if (!tbody || !currentGuildId) return;

    tbody.replaceChildren();
    const loadingTr = document.createElement("tr");
    const loadingTd = document.createElement("td");
    loadingTd.colSpan = 7;
    loadingTd.className = "muted text-center";
    loadingTd.textContent = "Loading custom triggers...";
    loadingTr.appendChild(loadingTd);
    tbody.appendChild(loadingTr);

    try {
      const res = await apiFetch(`/api/guilds/${currentGuildId}/triggers`);
      const triggers = res.triggers || [];
      if (countBadge) countBadge.textContent = `${triggers.length} triggers`;

      tbody.replaceChildren();
      if (triggers.length === 0) {
        const tr = document.createElement("tr");
        const td = document.createElement("td");
        td.colSpan = 7;
        td.className = "muted text-center";
        td.textContent = "No custom triggers configured. Click '+ Create Custom Trigger' to add one.";
        tr.appendChild(td);
        tbody.appendChild(tr);
        return;
      }

      triggers.forEach((t) => {
        const tr = document.createElement("tr");

        // Name
        const tdName = document.createElement("td");
        const codeName = document.createElement("code");
        codeName.textContent = t.trigger_name;
        tdName.appendChild(codeName);
        tr.appendChild(tdName);

        // Match Mode
        const tdMode = document.createElement("td");
        const badgeMode = document.createElement("span");
        badgeMode.className = "badge badge-cyan";
        badgeMode.textContent = (t.match_mode || "prefix").toUpperCase();
        tdMode.appendChild(badgeMode);
        tr.appendChild(tdMode);

        // Response Preview
        const tdResp = document.createElement("td");
        const respText = t.response_template || "";
        tdResp.textContent = respText.length > 55 ? `${respText.substring(0, 55)}...` : respText;
        tr.appendChild(tdResp);

        // Cooldown
        const tdCd = document.createElement("td");
        tdCd.textContent = `${t.cooldown_seconds || 0}s`;
        tr.appendChild(tdCd);

        // Options Flags
        const tdFlags = document.createElement("td");
        const flagsDiv = document.createElement("div");
        flagsDiv.style.display = "flex";
        flagsDiv.style.gap = "0.25rem";
        if (t.delete_trigger_message) {
          const b = document.createElement("span");
          b.className = "badge badge-neutral";
          b.textContent = "Del Msg";
          flagsDiv.appendChild(b);
        }
        if (t.reply_in_dm) {
          const b = document.createElement("span");
          b.className = "badge badge-neutral";
          b.textContent = "DM";
          flagsDiv.appendChild(b);
        }
        if (!t.delete_trigger_message && !t.reply_in_dm) {
          flagsDiv.textContent = "-";
        }
        tdFlags.appendChild(flagsDiv);
        tr.appendChild(tdFlags);

        // Status
        const tdStatus = document.createElement("td");
        const badgeEn = document.createElement("span");
        badgeEn.className = t.enabled ? "badge badge-online" : "badge badge-offline";
        badgeEn.textContent = t.enabled ? "Enabled" : "Disabled";
        tdStatus.appendChild(badgeEn);
        tr.appendChild(tdStatus);

        // Actions
        const tdActions = document.createElement("td");
        const actionsWrap = document.createElement("div");
        actionsWrap.style.display = "flex";
        actionsWrap.style.gap = "0.35rem";

        const btnEdit = document.createElement("button");
        btnEdit.type = "button";
        btnEdit.className = "btn btn-secondary btn-sm";
        btnEdit.textContent = "Edit";
        btnEdit.addEventListener("click", () => {
          setValue("trig-name", t.trigger_name);
          setValue("trig-mode", t.match_mode || "prefix");
          setValue("trig-cooldown", String(t.cooldown_seconds || 0));
          setChecked("trig-enabled", Boolean(t.enabled));
          setChecked("trig-delete-msg", Boolean(t.delete_trigger_message));
          setChecked("trig-reply-dm", Boolean(t.reply_in_dm));
          setValue("trig-roles", (t.allowed_roles || []).join(", "));
          setValue("trig-channels", (t.ignored_channels || []).join(", "));
          setValue("trig-response", t.response_template || "");
          openModal("modal-create-trigger");
        });
        actionsWrap.appendChild(btnEdit);

        const btnDel = document.createElement("button");
        btnDel.type = "button";
        btnDel.className = "btn btn-danger btn-sm";
        btnDel.textContent = "Delete";
        btnDel.addEventListener("click", async () => {
          try {
            await apiFetch(`/api/guilds/${currentGuildId}/triggers/${encodeURIComponent(t.trigger_name)}`, {
              method: "DELETE",
            });
            showToast(`Deleted trigger "${t.trigger_name}".`);
            await loadCustomTriggers();
          } catch (err) {
            showToast(`Failed to delete trigger: ${err.message}`);
          }
        });
        actionsWrap.appendChild(btnDel);

        tdActions.appendChild(actionsWrap);
        tr.appendChild(tdActions);

        tbody.appendChild(tr);
      });
    } catch (err) {
      tbody.replaceChildren();
      const tr = document.createElement("tr");
      const td = document.createElement("td");
      td.colSpan = 7;
      td.className = "muted text-center";
      td.textContent = `Error loading triggers: ${err.message}`;
      tr.appendChild(td);
      tbody.appendChild(tr);
    }
  }

  async function submitSaveTrigger() {
    const name = getValue("trig-name");
    const response = getValue("trig-response");
    if (!name || !response) {
      showToast("Trigger Name and Response Template are required.");
      return;
    }

    const parseNumList = (val) =>
      (val || "")
        .split(",")
        .map((s) => s.trim())
        .filter((s) => s.length > 0)
        .map((s) => parseInt(s, 10))
        .filter((n) => !isNaN(n));

    const payload = {
      trigger_name: name.trim(),
      match_mode: getValue("trig-mode") || "prefix",
      response_template: response,
      cooldown_seconds: parseInt(getValue("trig-cooldown"), 10) || 0,
      allowed_roles: parseNumList(getValue("trig-roles")),
      ignored_channels: parseNumList(getValue("trig-channels")),
      delete_trigger_message: getChecked("trig-delete-msg"),
      reply_in_dm: getChecked("trig-reply-dm"),
      enabled: getChecked("trig-enabled"),
    };

    try {
      await apiFetch(`/api/guilds/${currentGuildId}/triggers`, {
        method: "POST",
        body: JSON.stringify(payload),
      });
      showToast(`Saved trigger "${name}".`);
      closeModal("modal-create-trigger");
      setValue("trig-name", "");
      setValue("trig-response", "");
      await loadCustomTriggers();
    } catch (err) {
      showToast(`Failed to save trigger: ${err.message}`);
    }
  }

  async function runTagScriptSimulator() {
    const tmpl = getValue("tagscript-bench-input") || "";
    const outBody = document.getElementById("tagscript-out-body");
    const outEmbed = document.getElementById("tagscript-out-embed");
    const noEmbed = document.getElementById("tagscript-out-no-embed");
    const outActions = document.getElementById("tagscript-out-actions");
    const outAi = document.getElementById("tagscript-out-ai");

    if (outBody) outBody.textContent = "Evaluating TagScript template...";

    try {
      const res = await apiFetch("/api/tagscript/preview", {
        method: "POST",
        body: JSON.stringify({
          template: tmpl,
          author_name: "CyberUser#0001",
          guild_name: "Neon Sanctuary",
          guild_members: 1337,
        }),
      });

      const result = res.result || {};
      if (outBody) outBody.textContent = result.body || "(No message body rendered)";

      // Embed
      if (result.embed && outEmbed && noEmbed) {
        outEmbed.classList.remove("hidden");
        noEmbed.classList.add("hidden");

        const tTitle = document.getElementById("ts-embed-title");
        const tDesc = document.getElementById("ts-embed-desc");
        const tFooter = document.getElementById("ts-embed-footer");
        const tBar = document.getElementById("ts-embed-bar");

        if (tTitle) tTitle.textContent = result.embed.title || "";
        if (tDesc) tDesc.textContent = result.embed.description || "";
        if (tFooter) tFooter.textContent = result.embed.footer || "";
        if (tBar) {
          const col = result.embed.color
            ? `#${result.embed.color.toString(16).padStart(6, "0")}`
            : "var(--accent-primary)";
          tBar.style.backgroundColor = col;
        }
      } else if (outEmbed && noEmbed) {
        outEmbed.classList.add("hidden");
        noEmbed.classList.remove("hidden");
      }

      // Actions
      if (outActions) {
        outActions.replaceChildren();
        const actions = result.actions || [];
        if (actions.length === 0) {
          const sp = document.createElement("span");
          sp.className = "muted";
          sp.textContent = "No action tags executed.";
          outActions.appendChild(sp);
        } else {
          actions.forEach((a) => {
            const b = document.createElement("span");
            b.className = "badge badge-purple";
            if (typeof a === "string") {
              b.textContent = a;
            } else if (a.AddReaction) {
              b.textContent = `React: ${a.AddReaction}`;
            } else if (a.AddRole) {
              b.textContent = `Role(+): ${a.AddRole}`;
            } else if (a.RemoveRole) {
              b.textContent = `Role(-): ${a.RemoveRole}`;
            } else {
              b.textContent = JSON.stringify(a);
            }
            outActions.appendChild(b);
          });
        }
      }

      // AI Directive
      if (outAi) {
        if (result.ai_prompt) {
          outAi.textContent = `Directive: "${result.ai_prompt}"`;
          outAi.className = "status-note";
        } else {
          outAi.textContent = "None detected.";
          outAi.className = "status-note";
        }
      }
    } catch (err) {
      if (outBody) outBody.textContent = `Evaluation failed: ${err.message}`;
    }
  }

    const guildSel = document.getElementById("guild-selector");
    if (guildSel) {
      guildSel.addEventListener("change", async () => {
        currentGuildId = guildSel.value;
        await loadSelectedGuildConfig();
      });
    }

    // Guild synchronization across all module headers
    document.querySelectorAll(".guild-selector-sync").forEach((sel) => {
      sel.addEventListener("change", () => syncAllGuildSelectors(sel.value));
    });

    // Modals
    initModals();

    // Module 1: Moderation
    bindClick("btn-refresh-cases", () => {
      const fUser = getValue("mod-filter-user");
      loadModCases(fUser || null);
    });
    bindClick("btn-apply-case-filter", () => {
      const fUser = getValue("mod-filter-user");
      loadModCases(fUser || null);
    });
    bindClick("btn-reset-case-filter", () => {
      setValue("mod-filter-user", "");
      loadModCases();
    });
    bindClick("btn-open-manual-case", () => openModal("modal-manual-case"));
    bindClick("btn-submit-manual-case", submitManualCase);
    bindClick("btn-submit-edit-reason", submitEditReason);
    bindClick("btn-trigger-strike-decay", triggerManualStrikeDecay);

    // Module 2: Automod
    bindClick("btn-save-automod", saveAutomodConfig);
    bindClick("btn-add-ladder-step", addLadderStep);

    // Module 3: Audit Logging
    bindClick("btn-save-logging", saveLoggingConfig);
    const colorPicker = document.getElementById("log-embed-color-picker");
    const colorHex = document.getElementById("log-embed-color-hex");
    if (colorPicker && colorHex) {
      colorPicker.addEventListener("input", () => {
        colorHex.value = colorPicker.value;
      });
      colorHex.addEventListener("input", () => {
        if (/^#[0-9a-fA-F]{6}$/.test(colorHex.value)) {
          colorPicker.value = colorHex.value;
        }
      });
    }

    // Module 4: Greetings & Autorole
    bindClick("btn-save-greetings", saveGreetingsConfig);
    bindClick("btn-preview-welcome-embed", updateGreetingsEmbedPreview);
    const greetEmbedIn = document.getElementById("greet-welcome-embed");
    const greetMsgIn = document.getElementById("greet-welcome-msg");
    if (greetEmbedIn) greetEmbedIn.addEventListener("input", updateGreetingsEmbedPreview);
    if (greetMsgIn) greetMsgIn.addEventListener("input", updateGreetingsEmbedPreview);

    // Module 5: Reaction Roles
    bindClick("btn-refresh-rr", loadReactionRoles);
    bindClick("btn-open-rr-modal", () => {
      const bList = document.getElementById("rr-entries-builder");
      if (bList) bList.replaceChildren();
      addRrEntryRow("👍", "", "Role Button", "primary");
      openModal("modal-create-rr");
    });
    bindClick("btn-add-rr-entry-row", () => addRrEntryRow());
    bindClick("btn-submit-create-rr", submitCreateReactionRole);

    // Module 6: Custom Triggers & TagScript Simulator
    bindClick("btn-refresh-triggers", loadCustomTriggers);
    bindClick("btn-open-trigger-modal", () => {
      setValue("trig-name", "");
      setValue("trig-response", "");
      openModal("modal-create-trigger");
    });
    bindClick("btn-submit-save-trigger", submitSaveTrigger);
    bindClick("btn-eval-tagscript", runTagScriptSimulator);

    // TagScript Chip Insertions
    document.querySelectorAll(".tagscript-chip[data-insert]").forEach((chip) => {
      chip.addEventListener("click", () => {
        const token = chip.getAttribute("data-insert");
        const area = document.getElementById("tagscript-bench-input");
        if (area && token) {
          area.value = area.value ? `${area.value} ${token}` : token;
          area.focus();
        }
      });
    });

    bindClick("btn-save-guild-config", saveGuildConfig);
    bindClick("btn-add-reddit-sub", addRedditSubscription);

    bindClick("btn-gen-reddit-secret", async () => {
      try {
        const res = await apiFetch("/api/reddit/generate-secret", { method: "POST" });
        if (res.secret) {
          setValue("cfg-reddit-webhook-secret", res.secret);
          showToast("Generated new webhook secret (click Save Global Settings to persist).");
        }
      } catch (e) {
        showToast(`Failed to generate secret: ${e.message}`);
      }
    });

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
