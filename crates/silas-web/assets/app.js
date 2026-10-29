// Silas Blue v4.0 Clean-Room Administration Console Client
// Security: Strictly uses textContent / createElement (zero innerHTML) and sends X-Requested-With: SilasBlue-SPA.

(function () {
  "use strict";

  const CSRF_HEADER = { "X-Requested-With": "SilasBlue-SPA" };
  let remotePresets = [];
  let currentGuildId = null;
  let currentUserSession = null;
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
        if (currentUserSession && currentUserSession.is_owner) {
          apiFetch("/api/settings", {
            method: "PUT",
            body: JSON.stringify({ theme: sel.value }),
          }).catch(() => {});
        }
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
  // Discord Guild Cache & Data Providers
  // ==========================================================================
  const discordGuildData = {
    roles: new Map(),
    channels: new Map(),
    emojis: new Map(),
  };

  async function getGuildRoles(guildId) {
    if (!guildId || guildId === "0") return [];
    if (discordGuildData.roles.has(guildId)) {
      return discordGuildData.roles.get(guildId);
    }
    try {
      const res = await apiFetch(`/api/guilds/${guildId}/roles`);
      const roles = res.roles || [];
      discordGuildData.roles.set(guildId, roles);
      return roles;
    } catch (_) {
      return [];
    }
  }

  async function getGuildChannels(guildId) {
    if (!guildId || guildId === "0") return [];
    if (discordGuildData.channels.has(guildId)) {
      return discordGuildData.channels.get(guildId);
    }
    try {
      const res = await apiFetch(`/api/guilds/${guildId}/channels`);
      const channels = res.channels || [];
      discordGuildData.channels.set(guildId, channels);
      return channels;
    } catch (_) {
      return [];
    }
  }

  async function getGuildEmojis(guildId) {
    if (!guildId || guildId === "0") return [];
    if (discordGuildData.emojis.has(guildId)) {
      return discordGuildData.emojis.get(guildId);
    }
    try {
      const res = await apiFetch(`/api/guilds/${guildId}/emojis`);
      const emojis = res.emojis || [];
      discordGuildData.emojis.set(guildId, emojis);
      return emojis;
    } catch (_) {
      return [];
    }
  }

  async function searchGuildMembers(guildId, query) {
    if (!guildId || guildId === "0" || !query || query.trim().length < 2) return [];
    try {
      const res = await apiFetch(`/api/guilds/${guildId}/members?q=${encodeURIComponent(query.trim())}`);
      return res.members || [];
    } catch (_) {
      return [];
    }
  }

  function formatChannelKind(kind) {
    switch ((kind || "").toLowerCase()) {
      case "text": return "(Text)";
      case "voice": return "(Voice)";
      case "category": return "(Category)";
      case "announcement": return "(Announcement)";
      case "stage": return "(Stage)";
      case "forum": return "(Forum)";
      case "thread": return "(Thread)";
      default: return "(Channel)";
    }
  }

  // ==========================================================================
  // Reusable Combobox Component & Autocomplete Registries
  // ==========================================================================
  const comboboxRegistry = new Map();
  const memberCache = new Map();

  function resolveRoleLabel(id) {
    if (!id) return "";
    const roles = discordGuildData.roles.get(currentGuildId) || [];
    const r = roles.find((x) => String(x.id) === String(id));
    return r ? `@${r.name}` : `Role (${id})`;
  }

  function resolveChannelLabel(id) {
    if (!id) return "";
    const channels = discordGuildData.channels.get(currentGuildId) || [];
    const ch = channels.find((x) => String(x.id) === String(id));
    return ch ? `#${ch.name} ${formatChannelKind(ch.kind)}` : `Channel (${id})`;
  }

  function resolveMemberLabel(id) {
    if (!id) return "";
    const m = memberCache.get(String(id));
    if (m) {
      return m.display_name ? `${m.display_name} (@${m.username})` : `@${m.username}`;
    }
    return `User (${id})`;
  }

  async function getRoleItems(query) {
    const roles = await getGuildRoles(currentGuildId);
    const q = (query || "").trim().toLowerCase();
    return roles
      .filter((r) => !q || (r.name || "").toLowerCase().includes(q) || String(r.id).includes(q))
      .map((r) => ({
        id: r.id,
        label: `@${r.name}`,
        sublabel: r.managed ? "Managed Integration" : (r.color ? `#${r.color}` : undefined),
      }));
  }

  function getChannelItemsFetcher(filterKinds = null) {
    return async function (query) {
      const channels = await getGuildChannels(currentGuildId);
      const q = (query || "").trim().toLowerCase();
      return channels
        .filter((ch) => {
          if (filterKinds && !filterKinds.includes(ch.kind)) return false;
          if (!q) return true;
          return (ch.name || "").toLowerCase().includes(q) || String(ch.id).includes(q);
        })
        .map((ch) => ({
          id: ch.id,
          label: `#${ch.name} ${formatChannelKind(ch.kind)}`,
          sublabel: ch.kind,
        }));
    };
  }

  async function getMemberItems(query) {
    const q = (query || "").trim();
    if (q.length < 2) return [];
    const members = await searchGuildMembers(currentGuildId, q);
    members.forEach((m) => memberCache.set(String(m.id), m));
    return members.map((m) => ({
      id: m.id,
      label: m.display_name ? `${m.display_name} (@${m.username})` : `@${m.username}`,
      sublabel: `ID: ${m.id}`,
      icon: m.avatar_url,
    }));
  }

  function createCombobox({
    container,
    placeholder = "Search...",
    multiSelect = false,
    getItems,
    initialValue = null,
    onChange,
    resolveLabel,
  }) {
    if (!container) return null;
    container.replaceChildren();

    let selectedIds = multiSelect
      ? new Set(Array.isArray(initialValue) ? initialValue.map(String) : [])
      : (initialValue ? String(initialValue) : null);

    let highlightedIndex = -1;
    let currentResults = [];
    let debounceTimer = null;

    const wrapper = document.createElement("div");
    wrapper.className = "combobox-input-wrapper";

    const chipsContainer = document.createElement("div");
    chipsContainer.className = "combobox-chips";
    if (!multiSelect) chipsContainer.style.display = "none";
    wrapper.appendChild(chipsContainer);

    const input = document.createElement("input");
    input.type = "text";
    input.className = "combobox-input";
    input.placeholder = placeholder;
    input.setAttribute("role", "combobox");
    input.setAttribute("aria-autocomplete", "list");
    input.setAttribute("aria-expanded", "false");
    input.setAttribute("aria-haspopup", "listbox");
    wrapper.appendChild(input);

    const dropdown = document.createElement("div");
    dropdown.className = "combobox-dropdown hidden";
    dropdown.setAttribute("role", "listbox");

    container.appendChild(wrapper);
    container.appendChild(dropdown);

    function updateChips() {
      if (!multiSelect) return;
      chipsContainer.replaceChildren();
      selectedIds.forEach((id) => {
        const chip = document.createElement("span");
        chip.className = "combobox-chip";

        const textSpan = document.createElement("span");
        const display = resolveLabel ? resolveLabel(id) : `ID: ${id}`;
        textSpan.textContent = display || `ID: ${id}`;
        chip.appendChild(textSpan);

        const btnRemove = document.createElement("button");
        btnRemove.type = "button";
        btnRemove.className = "combobox-chip-remove";
        btnRemove.setAttribute("aria-label", `Remove ${textSpan.textContent}`);
        btnRemove.textContent = "×";
        btnRemove.addEventListener("click", (e) => {
          e.stopPropagation();
          selectedIds.delete(id);
          updateChips();
          if (onChange) onChange(Array.from(selectedIds));
        });
        chip.appendChild(btnRemove);

        chipsContainer.appendChild(chip);
      });
    }

    function updateSingleDisplay() {
      if (multiSelect) return;
      if (selectedIds) {
        const display = resolveLabel ? resolveLabel(selectedIds) : `ID: ${selectedIds}`;
        input.value = display || `ID: ${selectedIds}`;
      } else {
        input.value = "";
      }
    }

    function openDropdown() {
      dropdown.classList.remove("hidden");
      input.setAttribute("aria-expanded", "true");
    }

    function closeDropdown() {
      dropdown.classList.add("hidden");
      input.setAttribute("aria-expanded", "false");
      highlightedIndex = -1;
    }

    function renderOptions(items) {
      currentResults = items.slice(0, 25);
      dropdown.replaceChildren();
      highlightedIndex = -1;

      if (currentResults.length === 0) {
        const empty = document.createElement("div");
        empty.className = "combobox-empty";
        empty.textContent = "No matches found.";
        dropdown.appendChild(empty);
        openDropdown();
        return;
      }

      currentResults.forEach((item) => {
        const opt = document.createElement("div");
        opt.className = "combobox-option";
        opt.setAttribute("role", "option");
        opt.setAttribute("data-id", String(item.id));

        const isSelected = multiSelect
          ? selectedIds.has(String(item.id))
          : selectedIds === String(item.id);

        if (isSelected) {
          opt.classList.add("combobox-option-selected");
          opt.setAttribute("aria-selected", "true");
        } else {
          opt.setAttribute("aria-selected", "false");
        }

        const left = document.createElement("div");
        left.style.display = "flex";
        left.style.alignItems = "center";
        left.style.gap = "0.5rem";

        if (item.icon) {
          const img = document.createElement("img");
          img.src = item.icon;
          img.alt = "";
          img.style.width = "18px";
          img.style.height = "18px";
          img.style.borderRadius = "3px";
          left.appendChild(img);
        }

        const nameSpan = document.createElement("span");
        nameSpan.textContent = item.label;
        left.appendChild(nameSpan);

        opt.appendChild(left);

        if (item.sublabel) {
          const sub = document.createElement("span");
          sub.className = "combobox-option-sublabel";
          sub.textContent = item.sublabel;
          opt.appendChild(sub);
        }

        opt.addEventListener("mousedown", (e) => {
          e.preventDefault();
          selectItem(item);
        });

        dropdown.appendChild(opt);
      });

      openDropdown();
    }

    function selectItem(item) {
      const idStr = String(item.id);
      if (multiSelect) {
        if (selectedIds.has(idStr)) {
          selectedIds.delete(idStr);
        } else {
          selectedIds.add(idStr);
        }
        updateChips();
        input.value = "";
        closeDropdown();
        if (onChange) onChange(Array.from(selectedIds));
      } else {
        selectedIds = idStr;
        updateSingleDisplay();
        closeDropdown();
        if (onChange) onChange(selectedIds);
      }
    }

    input.addEventListener("focus", async () => {
      if (!multiSelect && selectedIds) {
        input.select();
      }
      const items = await getItems("");
      renderOptions(items);
    });

    input.addEventListener("input", () => {
      if (!multiSelect && input.value.trim() === "") {
        selectedIds = null;
        if (onChange) onChange(null);
      }
      if (debounceTimer) clearTimeout(debounceTimer);
      debounceTimer = setTimeout(async () => {
        const items = await getItems(input.value);
        renderOptions(items);
      }, 200);
    });

    input.addEventListener("keydown", (e) => {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        if (dropdown.classList.contains("hidden")) {
          openDropdown();
          return;
        }
        highlightedIndex = Math.min(highlightedIndex + 1, currentResults.length - 1);
        updateHighlight();
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        highlightedIndex = Math.max(highlightedIndex - 1, 0);
        updateHighlight();
      } else if (e.key === "Enter") {
        e.preventDefault();
        if (highlightedIndex >= 0 && highlightedIndex < currentResults.length) {
          selectItem(currentResults[highlightedIndex]);
        }
      } else if (e.key === "Escape") {
        closeDropdown();
      } else if (e.key === "Backspace" && multiSelect && input.value === "" && selectedIds.size > 0) {
        const arr = Array.from(selectedIds);
        const lastId = arr[arr.length - 1];
        selectedIds.delete(lastId);
        updateChips();
        if (onChange) onChange(Array.from(selectedIds));
      }
    });

    function updateHighlight() {
      const opts = dropdown.querySelectorAll(".combobox-option");
      opts.forEach((o, i) => {
        if (i === highlightedIndex) {
          o.classList.add("highlighted");
          o.scrollIntoView({ block: "nearest" });
        } else {
          o.classList.remove("highlighted");
        }
      });
    }

    document.addEventListener("click", (e) => {
      if (!container.contains(e.target)) {
        closeDropdown();
      }
    });

    if (multiSelect) {
      updateChips();
    } else {
      updateSingleDisplay();
    }

    return {
      isMulti: multiSelect,
      getValue: () => (multiSelect ? Array.from(selectedIds) : selectedIds),
      setValue: (val) => {
        if (multiSelect) {
          selectedIds = new Set(Array.isArray(val) ? val.map(String) : []);
          updateChips();
          input.value = "";
        } else {
          selectedIds = val !== undefined && val !== null && String(val).trim() !== "" ? String(val).trim() : null;
          updateSingleDisplay();
        }
      },
      refresh: () => {
        if (multiSelect) updateChips();
        else updateSingleDisplay();
      },
    };
  }

  function initRoleCombobox(containerOrId, { multiSelect = false, placeholder = "Search roles...", initialValue = null, onChange = null } = {}) {
    const container = typeof containerOrId === "string" ? document.getElementById(containerOrId) : containerOrId;
    if (!container) return null;
    const cb = createCombobox({
      container,
      placeholder,
      multiSelect,
      getItems: getRoleItems,
      resolveLabel: resolveRoleLabel,
      initialValue,
      onChange,
    });
    if (typeof containerOrId === "string") {
      comboboxRegistry.set(containerOrId, cb);
    }
    return cb;
  }

  function initChannelCombobox(containerOrId, { multiSelect = false, filterKinds = null, placeholder = "Search channels...", initialValue = null, onChange = null } = {}) {
    const container = typeof containerOrId === "string" ? document.getElementById(containerOrId) : containerOrId;
    if (!container) return null;
    const cb = createCombobox({
      container,
      placeholder,
      multiSelect,
      getItems: getChannelItemsFetcher(filterKinds),
      resolveLabel: resolveChannelLabel,
      initialValue,
      onChange,
    });
    if (typeof containerOrId === "string") {
      comboboxRegistry.set(containerOrId, cb);
    }
    return cb;
  }

  function initUserCombobox(containerOrId, { placeholder = "Search member by username or display name...", initialValue = null, onChange = null } = {}) {
    const container = typeof containerOrId === "string" ? document.getElementById(containerOrId) : containerOrId;
    if (!container) return null;
    const cb = createCombobox({
      container,
      placeholder,
      multiSelect: false,
      getItems: getMemberItems,
      resolveLabel: resolveMemberLabel,
      initialValue,
      onChange,
    });
    if (typeof containerOrId === "string") {
      comboboxRegistry.set(containerOrId, cb);
    }
    return cb;
  }

  // ==========================================================================
  // Duration Parsing & Formatting
  // ==========================================================================
  function parseDurationJs(str) {
    if (!str || !str.trim()) {
      return { valid: true, seconds: null, formatted: "Permanent" };
    }
    const s = str.trim().toLowerCase();
    if (s === "permanent" || s === "perm" || s === "0") {
      return { valid: true, seconds: null, formatted: "Permanent" };
    }

    const clean = s.replace(/,/g, " ").replace(/\s+/g, " ");
    const tokens = clean.split(" ");
    let totalSecs = 0;
    let found = 0;

    for (const tok of tokens) {
      if (!tok) continue;
      const m = tok.match(/^(\d+)(s(?:ecs?)?|m(?:in(?:ute)?s?)?|h(?:(?:ou)?rs?)?|d(?:ays?)?|w(?:eeks?)?)?$/);
      if (!m) {
        return { valid: false, error: `Invalid duration format: "${tok}"` };
      }
      const val = parseInt(m[1], 10);
      const unit = m[2] || "s";
      let multiplier = 1;
      if (unit.startsWith("m") && !unit.startsWith("mo")) {
        multiplier = 60;
      } else if (unit.startsWith("h")) {
        multiplier = 3600;
      } else if (unit.startsWith("d")) {
        multiplier = 86400;
      } else if (unit.startsWith("w")) {
        multiplier = 604800;
      } else if (unit.startsWith("s")) {
        multiplier = 1;
      }
      totalSecs += val * multiplier;
      found++;
    }

    if (found === 0) {
      return { valid: false, error: "Empty duration." };
    }

    return { valid: true, seconds: totalSecs, formatted: formatDurationHuman(totalSecs) };
  }

  function formatDurationHuman(secs) {
    if (secs === null || secs === undefined || secs === 0) return "Permanent";
    let rem = secs;
    const weeks = Math.floor(rem / 604800);
    rem %= 604800;
    const days = Math.floor(rem / 86400);
    rem %= 86400;
    const hours = Math.floor(rem / 3600);
    rem %= 3600;
    const mins = Math.floor(rem / 60);
    const seconds = rem % 60;

    const parts = [];
    if (weeks > 0) parts.push(`${weeks} ${weeks === 1 ? "week" : "weeks"}`);
    if (days > 0) parts.push(`${days} ${days === 1 ? "day" : "days"}`);
    if (hours > 0) parts.push(`${hours} ${hours === 1 ? "hour" : "hours"}`);
    if (mins > 0) parts.push(`${mins} ${mins === 1 ? "minute" : "minutes"}`);
    if (seconds > 0 && parts.length === 0) parts.push(`${seconds} ${seconds === 1 ? "second" : "seconds"}`);

    return parts.length > 0 ? parts.join(", ") : "0 seconds";
  }

  function bindDurationPreview(inputId, previewId) {
    const input = document.getElementById(inputId);
    const preview = document.getElementById(previewId);
    if (!input || !preview) return;

    input.addEventListener("input", () => {
      const res = parseDurationJs(input.value);
      if (res.valid) {
        preview.className = "duration-preview-text";
        preview.textContent = `Preview: ${res.formatted}`;
      } else {
        preview.className = "duration-preview-text duration-preview-error";
        preview.textContent = `Error: ${res.error}`;
      }
    });
  }

  // ==========================================================================
  // Login Page Handling
  // ==========================================================================
  function initLoginPage() {
    const alertEl = document.getElementById("login-alert");
    if (!alertEl) return;

    // Synchronously parse and display URL error query parameters
    const params = new URLSearchParams(window.location.search);
    const err = params.get("error");
    if (err) {
      const messages = {
        missing_token: "Missing One-Time Login token parameter.",
        invalid_or_expired_otp:
          "That One-Time Login link or token has expired or has already been used. Check your latest terminal stdout or sign in with Discord.",
        oauth_not_configured:
          "Discord OAuth2 Client ID & Secret are not configured yet. Sign in with Host OTP below to configure Discord OAuth2 in Console Settings.",
        oauth_state_missing: "OAuth2 CSRF state cookie expired. Please try again.",
        oauth_state_mismatch: "OAuth2 CSRF state mismatch. Please try again.",
        oauth_cancelled: "Discord OAuth2 sign-in was cancelled.",
        session_error: "Unable to create authenticated session token. Please try again.",
        unauthorized_discord_account:
          "Authentication failed or your Discord account was not authorized. Please check your credentials or contact a bot administrator.",
      };
      alertEl.textContent = messages[err] || `Authentication notice: ${err}`;
      alertEl.classList.remove("hidden");
    }

    // Synchronously wire OTP form submit validation
    const otpForm = document.getElementById("otp-form");
    const otpInput = document.getElementById("otp-token-input");
    if (otpForm && otpInput) {
      otpForm.addEventListener("submit", (e) => {
        const token = otpInput.value.trim();
        if (!token) {
          e.preventDefault();
          otpInput.focus();
        } else {
          otpInput.value = token;
        }
      });
    }

    // Synchronously wire Active Session sign-out button
    const signoutBtn = document.getElementById("login-signout-btn");
    if (signoutBtn) {
      signoutBtn.addEventListener("click", async () => {
        try {
          await apiFetch("/auth/logout", { method: "POST" });
        } catch (_) {}
        window.location.reload();
      });
    }

    // Asynchronously check active user session & OAuth configuration
    apiFetch("/api/auth/session")
      .then((sess) => {
        if (!sess) return;

        // If user already holds a valid session cookie, display active session card
        if (sess.authenticated) {
          const sessionCard = document.getElementById("active-session-card");
          const sessionUser = document.getElementById("active-session-user");
          const sessionRole = document.getElementById("active-session-role");
          if (sessionCard && sessionUser && sessionRole) {
            sessionUser.textContent = sess.username || "User";
            sessionRole.textContent = sess.is_owner ? "Bot Owner / Admin" : "Server Manager";
            sessionCard.classList.remove("hidden");
          }
        }

        // If OAuth2 is not configured, inform the user
        if (!sess.oauth_enabled) {
          const noteEl = document.getElementById("oauth-status-note");
          if (noteEl) {
            noteEl.textContent =
              "Discord OAuth2 setup is pending. Use the Host One-Time Token below to authenticate as bot administrator and set credentials.";
            noteEl.classList.remove("hidden");
          }
        }
      })
      .catch(() => {});
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

  function updateActiveProviderDisplay() {
    const providerSel = document.getElementById("cfg-ai-provider");
    const badge = document.getElementById("active-provider-badge");
    if (!providerSel || !badge) return;

    const val = providerSel.value;
    if (val === "ollama") {
      badge.className = "badge badge-success";
      badge.textContent = "Local: Ollama";
    } else if (val === "llamacpp") {
      badge.className = "badge badge-cyan";
      badge.textContent = "Local: llama.cpp";
    } else {
      badge.className = "badge badge-purple";
      badge.textContent = "Remote Cloud API";
    }
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
      setBadgeState("badge-has-reddit-webhook-secret", Boolean(s.reddit_webhook_secret));

      setValue("cfg-owner-ids", s.bot_owner_id || "");
      setValue("cfg-ai-provider", s.backend_type || "ollama");
      updateActiveProviderDisplay();
      setValue("cfg-ollama-url", s.ollama_url || "http://127.0.0.1:11434");
      setValue("cfg-llamacpp-url", s.llamacpp_url || "http://127.0.0.1:8081");
      setValue("cfg-remote-preset", s.remote_preset || "Xiaomi MiMo");
      setValue("cfg-remote-base-url", s.remote_base_url || "");
      setValue("cfg-remote-model", s.remote_model || "");
      setValue("cfg-search-provider", s.web_search_provider || "bing");
      const rawCh = (s.update_channel || "stable").toLowerCase();
      const normCh = (rawCh === "latest" || rawCh === "prerelease") ? rawCh : "stable";
      setValue("update-channel-select", normCh);
      const chBadge = document.getElementById("channel-status-badge");
      if (chBadge) {
        if (normCh === "latest" || normCh === "prerelease") {
          chBadge.className = "badge badge-warning";
          chBadge.textContent = "Active: Pre-release";
        } else {
          chBadge.className = "badge badge-success";
          chBadge.textContent = "Active: Stable";
        }
      }

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
    const ownerIdRaw = getValue("cfg-owner-ids").trim();

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
      setValue("cfg-reddit-webhook-secret", "");
      showToast("Saved global settings.");
      await loadGlobalSettings();
      await refreshStatus();
    } catch (e) {
      showToast(`Error saving settings: ${e.message}`);
    }
  }

  // ==========================================================================
  // Server Configuration & Role-Based UI Governance
  // ==========================================================================
  const guildPermissionsMap = new Map();

  function hasGuildDomainAccess(guildId, domain, requiredLevel) {
    const isOwner = Boolean(currentUserSession && currentUserSession.is_owner);
    if (isOwner) return true;
    const entry = guildPermissionsMap.get(String(guildId));
    if (!entry) return false;
    if (entry.role === "owner" || entry.role === "admin") return true;
    const perms = entry.permissions || {};
    const raw = (perms[domain] || perms["*"] || "none").toLowerCase();
    if (requiredLevel === "edit") {
      return raw === "edit" || raw === "write" || raw === "admin";
    }
    return raw === "view" || raw === "read" || raw === "edit" || raw === "write" || raw === "admin";
  }

  function applyGuildRolePermissions(guildId) {
    const isOwner = Boolean(currentUserSession && currentUserSession.is_owner);
    const entry = guildPermissionsMap.get(String(guildId)) || {
      role: isOwner ? "owner" : "delegated",
      permissions: {},
    };
    const roleLabelMap = {
      owner: "Bot Owner",
      admin: "Server Admin",
      moderator: "Moderator",
      delegated: "Delegated",
    };
    const roleLabel = roleLabelMap[isOwner ? "owner" : entry.role] || "Member";
    const uname = (currentUserSession && currentUserSession.username) || "User";
    setText("session-username", `${uname} (${roleLabel})`);

    const canView = (dom) => hasGuildDomainAccess(guildId, dom, "view");
    const canEdit = (dom) => hasGuildDomainAccess(guildId, dom, "edit");

    const viewVisibility = {
      overview: isOwner,
      backends: isOwner,
      updates: isOwner,
      invite: isOwner,
      servers: canView("settings"),
      reddit: canView("reddit") || canView("settings"),
      moderation: canView("moderation"),
      automod: canView("automod"),
      logging: canView("logging"),
      greetings: canView("greetings"),
      "reaction-roles": canView("reaction_roles"),
      triggers: canView("triggers"),
      activity: canView("activity"),
      grants: canView("permissions"),
      "audit-log": canView("audit_log"),
    };

    const navButtons = document.querySelectorAll(".nav-item[data-view]");
    navButtons.forEach((btn) => {
      const v = btn.getAttribute("data-view");
      const allowed = Boolean(viewVisibility[v]);
      if (allowed) {
        btn.classList.remove("hidden");
      } else {
        btn.classList.add("hidden");
      }
    });

    const toggleBtn = (id, allowed) => {
      const el = document.getElementById(id);
      if (!el) return;
      el.disabled = !allowed;
    };

    toggleBtn("btn-save-guild-config", canEdit("settings"));
    toggleBtn("btn-apply-guild-icon", canEdit("settings"));
    toggleBtn("btn-reset-guild-icon", canEdit("settings"));
    toggleBtn("btn-reset-guild-quota", canEdit("settings"));
    toggleBtn("btn-clear-guild-memory", canEdit("settings"));
    toggleBtn("btn-add-reddit-sub", canEdit("reddit") || canEdit("settings"));
    toggleBtn("btn-add-reddit-companion-sub", canEdit("reddit") || canEdit("settings"));
    toggleBtn("btn-open-manual-case", canEdit("moderation"));
    toggleBtn("btn-trigger-strike-decay", canEdit("moderation") || canEdit("automod"));
    toggleBtn("btn-save-automod", canEdit("automod"));
    toggleBtn("btn-add-ladder-step", canEdit("automod"));
    toggleBtn("btn-save-logging", canEdit("logging"));
    toggleBtn("btn-save-greetings", canEdit("greetings"));
    toggleBtn("btn-open-rr-modal", canEdit("reaction_roles"));
    toggleBtn("btn-open-trigger-modal", canEdit("triggers"));
    toggleBtn("btn-save-activity-config", canEdit("activity"));
    toggleBtn("btn-open-add-reward-modal", canEdit("activity"));
    toggleBtn("btn-save-cmd-channels", canEdit("activity") || canEdit("settings"));
    toggleBtn("btn-add-default-channel", canEdit("activity") || canEdit("settings"));
    toggleBtn("btn-add-cmd-override", canEdit("activity") || canEdit("settings"));
    toggleBtn("btn-submit-point-adjust", canEdit("activity") || canEdit("moderation"));
    toggleBtn("btn-create-grant", canEdit("permissions"));

    const activeNav = document.querySelector(".nav-item.active[data-view]");
    if (!activeNav || activeNav.classList.contains("hidden")) {
      const firstVisible = document.querySelector(".nav-item[data-view]:not(.hidden)");
      if (firstVisible) {
        const target = firstVisible.getAttribute("data-view");
        navButtons.forEach((b) => b.classList.remove("active"));
        firstVisible.classList.add("active");
        document.querySelectorAll(".view-panel").forEach((panel) => {
          if (panel.id === `view-${target}`) {
            panel.classList.remove("hidden");
          } else {
            panel.classList.add("hidden");
          }
        });
      }
    }
  }

  async function loadGuildList() {
    const selectors = document.querySelectorAll("#guild-selector, .guild-selector-sync");
    if (selectors.length === 0) return;
    try {
      const res = await apiFetch("/api/guilds");
      const guilds = res.guilds || [];
      guildPermissionsMap.clear();
      guilds.forEach((g) => {
        guildPermissionsMap.set(String(g.id), {
          role: g.my_role || "delegated",
          permissions: g.permissions || {},
        });
      });

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
            const roleTag = g.my_role ? ` [${g.my_role.toUpperCase()}]` : "";
            opt.textContent = g.connected
              ? `${g.name} (${g.id})${roleTag}`
              : `${g.name} [Offline]${roleTag}`;
            sel.appendChild(opt);
          });
          if (!currentGuildId || !guilds.some((g) => g.id === currentGuildId)) {
            currentGuildId = guilds[0].id;
          }
          sel.value = currentGuildId;
        }
      });

      applyGuildRolePermissions(currentGuildId);
      await refreshActiveModule();
    } catch (_) {}
  }

  function syncAllGuildSelectors(newGuildId) {
    currentGuildId = newGuildId;
    const selectors = document.querySelectorAll("#guild-selector, .guild-selector-sync");
    selectors.forEach((sel) => {
      sel.value = currentGuildId;
    });
    applyGuildRolePermissions(currentGuildId);
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
      case "activity":
        await loadActivityModule();
        break;
      case "reddit":
        await loadRedditModule();
        break;
      case "grants":
        await loadGrantsModule();
        break;
      case "audit-log":
        await loadAuditLogModule();
        break;
    }
  }

  async function loadSelectedGuildConfig() {
    if (!currentGuildId) return;
    try {
      await Promise.all([getGuildRoles(currentGuildId), getGuildChannels(currentGuildId)]);
      const res = await apiFetch(`/api/guilds/${currentGuildId}/config`);
      const c = res.config || {};
      const meta = res.metadata || null;
      const qw = res.quota_window || null;

      setValue("gcfg-bot-nickname", (meta && meta.bot_nickname) || "");
      updateBotIconPreview(meta ? meta.bot_avatar_url : null, res.default_avatar_url || null);
      setValue("gcfg-default-model", c.default_model || "");
      setValue("gcfg-allowed-channels", c.allowed_channels || []);
      setValue("gcfg-prompt-roles", c.reply_roles || []);
      setValue("gcfg-model-roles", c.change_model_roles || []);
      setValue("gcfg-admin-roles", c.change_permission_roles || []);

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
      setValue("gcfg-forum-channels", c.forum_monitored_channels || []);
      setValue("gcfg-forum-prompt", c.forum_system_prompt || "");

      comboboxRegistry.forEach((cb) => cb.refresh());
      renderQuotaUsage(qw);
      await loadGuildRedditSubscriptions(currentGuildId);
    } catch (e) {
      showToast(`Failed to load server config: ${e.message}`);
    }
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

  function updateBotIconPreview(customUrl, defaultUrl) {
    const previewImg = document.getElementById("gcfg-bot-icon-preview");
    const badge = document.getElementById("gcfg-bot-icon-badge");
    const urlInput = document.getElementById("gcfg-bot-icon-url");
    const fileInput = document.getElementById("gcfg-bot-icon-file");

    if (urlInput) urlInput.value = "";
    if (fileInput) fileInput.value = "";

    if (!previewImg) return;

    if (customUrl) {
      previewImg.src = customUrl;
      if (badge) {
        badge.textContent = "Custom Server Icon";
        badge.className = "badge badge-accent";
      }
    } else if (defaultUrl) {
      previewImg.src = defaultUrl;
      if (badge) {
        badge.textContent = "Default Bot Icon";
        badge.className = "badge badge-neutral";
      }
    } else {
      previewImg.src = "/favicon.svg";
      if (badge) {
        badge.textContent = "Default Bot Icon";
        badge.className = "badge badge-neutral";
      }
    }
  }

  async function applyGuildIcon() {
    if (!currentGuildId) return;
    const urlInput = getValue("gcfg-bot-icon-url").trim();
    const fileInput = document.getElementById("gcfg-bot-icon-file");
    const file = fileInput && fileInput.files && fileInput.files[0];

    let iconData = null;
    if (file) {
      if (file.size > 10 * 1024 * 1024) {
        showToast("Icon image must be smaller than 10 MB.");
        return;
      }
      try {
        iconData = await new Promise((resolve, reject) => {
          const reader = new FileReader();
          reader.onload = (e) => resolve(e.target ? e.target.result : null);
          reader.onerror = () => reject(new Error("Failed to read image file"));
          reader.readAsDataURL(file);
        });
      } catch (err) {
        showToast(`Failed to read file: ${err.message}`);
        return;
      }
    } else if (urlInput) {
      iconData = urlInput;
    } else {
      showToast("Please enter an image URL or choose a file first.");
      return;
    }

    const btn = document.getElementById("btn-apply-guild-icon");
    if (btn) btn.disabled = true;
    try {
      await apiFetch(`/api/guilds/${currentGuildId}/icon`, {
        method: "POST",
        body: JSON.stringify({ icon: iconData }),
      });
      showToast("Server icon updated successfully!");
      await loadSelectedGuildConfig();
    } catch (e) {
      showToast(`Failed to update server icon: ${e.message}`);
    } finally {
      if (btn) btn.disabled = false;
    }
  }

  async function resetGuildIcon() {
    if (!currentGuildId) return;
    if (!confirm("Reset Silas Blue's icon in this server back to the default bot avatar?")) return;
    const btn = document.getElementById("btn-reset-guild-icon");
    if (btn) btn.disabled = true;
    try {
      await apiFetch(`/api/guilds/${currentGuildId}/icon`, {
        method: "POST",
        body: JSON.stringify({ icon: "reset" }),
      });
      showToast("Server icon reset to default avatar.");
      await loadSelectedGuildConfig();
    } catch (e) {
      showToast(`Failed to reset server icon: ${e.message}`);
    } finally {
      if (btn) btn.disabled = false;
    }
  }

  async function saveGuildConfig() {
    if (!currentGuildId) return;
    const memEnabled = getChecked("gcfg-memory-enabled");
    const memTurns = memEnabled ? Number(getValue("gcfg-memory-max")) || 6 : 0;
    const defaultLim = Number(getValue("gcfg-token-limit")) || 10000;

    const iconUrl = getValue("gcfg-bot-icon-url").trim();
    const fileInput = document.getElementById("gcfg-bot-icon-file");
    const file = fileInput && fileInput.files && fileInput.files[0];
    let botAvatar = undefined;
    if (file) {
      if (file.size > 10 * 1024 * 1024) {
        showToast("Icon image must be smaller than 10 MB.");
        return;
      }
      try {
        botAvatar = await new Promise((resolve, reject) => {
          const reader = new FileReader();
          reader.onload = (e) => resolve(e.target ? e.target.result : null);
          reader.onerror = () => reject(new Error("Failed to read image file"));
          reader.readAsDataURL(file);
        });
      } catch (err) {
        showToast(`Failed to read icon file: ${err.message}`);
        return;
      }
    } else if (iconUrl) {
      botAvatar = iconUrl;
    }

    const patch = {
      bot_nickname: getValue("gcfg-bot-nickname"),
      bot_avatar: botAvatar,
      default_model: getValue("gcfg-default-model"),
      allowed_channels: parseCsvList("gcfg-allowed-channels"),
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
      const p = document.createElement("p");
      p.className = "muted";
      p.textContent = "Select a connected server to manage subreddit subscriptions.";
      listEl.replaceChildren(p);
      return;
    }
    try {
      const res = await apiFetch(`/api/guilds/${guildId}/reddit`);
      const subs = res.subscriptions || [];
      if (subs.length === 0) {
        const p = document.createElement("p");
        p.className = "muted";
        p.textContent = "No active subreddit monitors for this server.";
        listEl.replaceChildren(p);
        return;
      }
      listEl.replaceChildren();
      subs.forEach((sub) => {
        const row = document.createElement("div");
        row.className = "reddit-sub-item";
        row.style.cssText = "display: flex; justify-content: space-between; align-items: center; padding: 0.6rem 0.8rem; background: var(--bg-card-sub, #1f2430); border-radius: 6px; margin-bottom: 0.5rem;";
        
        const details = document.createElement("div");
        const strong = document.createElement("strong");
        strong.textContent = `r/${sub.subreddit}`;
        details.appendChild(strong);

        const arrow = document.createTextNode(" ➔ ");
        details.appendChild(arrow);

        const chBadge = document.createElement("span");
        chBadge.className = "badge badge-neutral";
        chBadge.textContent = resolveChannelLabel(sub.channel_id);
        details.appendChild(chBadge);

        let filters = [];
        if (sub.filter_nsfw) filters.push("NSFW Filter");
        if (sub.min_score > 0) filters.push(`Min Score: ${sub.min_score}`);
        if (sub.flair_filter) filters.push(`Flairs: ${sub.flair_filter}`);
        if (sub.keyword_filter) filters.push(`Keywords: ${sub.keyword_filter}`);

        if (filters.length > 0) {
          const small = document.createElement("small");
          small.style.cssText = "margin-left: 0.5rem; opacity: 0.8";
          small.textContent = `(${filters.join(", ")})`;
          details.appendChild(small);
        }
        
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
      const p = document.createElement("p");
      p.className = "muted";
      p.textContent = `Error loading subscriptions: ${e.message}`;
      listEl.replaceChildren(p);
    }
  }

  async function addServerSubtabRedditSubscription() {
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
  let telemetryReconnectAttempts = 0;
  let telemetryReconnectTimer = null;

  function updateTelemetryStatus(connected) {
    const dot = document.getElementById("telemetry-dot");
    const txt = document.getElementById("telemetry-text");
    if (!dot || !txt) return;

    if (connected) {
      dot.className = "status-dot status-dot-connected";
      txt.textContent = "Connected";
    } else {
      dot.className = "status-dot status-dot-disconnected";
      txt.textContent = "Disconnected";
    }
  }

  function connectTelemetryWebSocket() {
    const logBox = document.getElementById("telemetry-log-box");
    if (!logBox) return;

    if (telemetryReconnectTimer) {
      clearTimeout(telemetryReconnectTimer);
      telemetryReconnectTimer = null;
    }

    const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
    const wsUrl = `${proto}//${window.location.host}/api/ws/logs`;

    try {
      wsConnection = new WebSocket(wsUrl);
    } catch (_) {
      scheduleTelemetryReconnect();
      return;
    }

    wsConnection.addEventListener("open", () => {
      telemetryReconnectAttempts = 0;
      updateTelemetryStatus(true);
    });

    wsConnection.addEventListener("message", (evt) => {
      try {
        const payload = JSON.parse(evt.data);
        if (
          payload.type === "system" &&
          payload.formatted &&
          payload.formatted.includes("Connected to Silas Blue live telemetry")
        ) {
          return;
        }
        telemetryLogs.push(payload);
        if (telemetryLogs.length > 400) {
          telemetryLogs.shift();
        }
        renderTelemetryConsole();
      } catch (_) {}
    });

    wsConnection.addEventListener("close", () => {
      updateTelemetryStatus(false);
      scheduleTelemetryReconnect();
    });

    wsConnection.addEventListener("error", () => {
      updateTelemetryStatus(false);
    });
  }

  function scheduleTelemetryReconnect() {
    telemetryReconnectAttempts++;
    const delay = Math.min(1000 * Math.pow(2, telemetryReconnectAttempts - 1), 30000);
    telemetryReconnectTimer = setTimeout(connectTelemetryWebSocket, delay);
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
    if (comboboxRegistry.has(id)) {
      const cb = comboboxRegistry.get(id);
      const val = cb.getValue();
      if (Array.isArray(val)) {
        return val.join(", ");
      }
      return val || "";
    }
    const el = document.getElementById(id);
    return el ? el.value.trim() : "";
  }

  function setValue(id, val) {
    if (comboboxRegistry.has(id)) {
      const cb = comboboxRegistry.get(id);
      if (cb.isMulti) {
        if (Array.isArray(val)) {
          cb.setValue(val);
        } else if (typeof val === "string") {
          const items = val.split(",").map((s) => s.trim()).filter((s) => s.length > 0);
          cb.setValue(items);
        } else {
          cb.setValue([]);
        }
      } else {
        cb.setValue(val !== undefined && val !== null ? String(val) : null);
      }
      return;
    }
    const el = document.getElementById(id);
    if (el) el.value = val !== undefined && val !== null ? val : "";
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

  function wireAllEventListeners() {
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
          syncAllGuildSelectors(guildSel.value);
        });
      }
  
      // Guild synchronization across all module headers
      document.querySelectorAll(".guild-selector-sync").forEach((sel) => {
        sel.addEventListener("change", () => syncAllGuildSelectors(sel.value));
      });
  
      // Modals
  
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
  
      // Activity & Rewards Event Listeners
      bindClick("btn-save-activity-config", saveActivityConfig);
      bindClick("btn-save-cmd-channels", saveCommandChannelsConfig);
      bindClick("btn-add-default-channel", addDefaultCommandChannel);
      bindClick("btn-add-cmd-override", addCommandChannelOverride);
      bindClick("btn-open-add-reward-modal", () => {
        setValue("reward-modal-target", "500");
        setChecked("reward-modal-drop", true);
        openModal("modal-add-reward-rule");
      });
      bindClick("btn-submit-add-reward", submitAddRewardRule);
      bindClick("btn-refresh-leaderboard", loadActivityLeaderboard);
      bindClick("btn-submit-point-adjust", submitPointAdjustment);

      const lbTimeframe = document.getElementById("leaderboard-timeframe-select");
      if (lbTimeframe) {
        lbTimeframe.addEventListener("change", loadActivityLeaderboard);
      }

      const rewardTypeSel = document.getElementById("reward-modal-type");
      if (rewardTypeSel) {
        rewardTypeSel.addEventListener("change", () => {
          const isTop = rewardTypeSel.value === "top_rank";
          const lbl = document.getElementById("reward-modal-target-label");
          const hint = document.getElementById("reward-modal-target-hint");
          const targetIn = document.getElementById("reward-modal-target");
          if (isTop) {
            if (lbl) lbl.textContent = "Top Leaderboard Rank Position *";
            if (hint) hint.textContent = "Top N members (e.g. 5 for top 5, 10 for top 10) in the timeframe will receive the role.";
            if (targetIn && (!targetIn.value || targetIn.value === "500")) targetIn.value = "5";
          } else {
            if (lbl) lbl.textContent = "Threshold Points Required *";
            if (hint) hint.textContent = "Members who earn this many points in the selected timeframe will receive the role.";
            if (targetIn && targetIn.value === "5") targetIn.value = "500";
          }
        });
      }

      bindClick("btn-save-guild-config", saveGuildConfig);
      bindClick("btn-apply-guild-icon", applyGuildIcon);
      bindClick("btn-reset-guild-icon", resetGuildIcon);

      const guildIconFile = document.getElementById("gcfg-bot-icon-file");
      if (guildIconFile) {
        guildIconFile.addEventListener("change", () => {
          const file = guildIconFile.files && guildIconFile.files[0];
          if (file) {
            if (file.size > 10 * 1024 * 1024) {
              showToast("Icon image must be smaller than 10 MB.");
              guildIconFile.value = "";
              return;
            }
            const reader = new FileReader();
            reader.onload = (e) => {
              const previewImg = document.getElementById("gcfg-bot-icon-preview");
              if (previewImg && e.target && e.target.result) {
                previewImg.src = e.target.result;
                const badge = document.getElementById("gcfg-bot-icon-badge");
                if (badge) {
                  badge.textContent = "Selected (Unsaved)";
                  badge.className = "badge badge-cyan";
                }
              }
            };
            reader.readAsDataURL(file);
          }
        });
      }

      const guildIconUrl = document.getElementById("gcfg-bot-icon-url");
      if (guildIconUrl) {
        guildIconUrl.addEventListener("input", () => {
          const val = guildIconUrl.value.trim();
          if (val.startsWith("http://") || val.startsWith("https://") || val.startsWith("data:image/")) {
            const previewImg = document.getElementById("gcfg-bot-icon-preview");
            if (previewImg) {
              previewImg.src = val;
              const badge = document.getElementById("gcfg-bot-icon-badge");
              if (badge) {
                badge.textContent = "URL Preview (Unsaved)";
                badge.className = "badge badge-cyan";
              }
            }
          }
        });
      }
      bindClick("btn-add-reddit-sub", addServerSubtabRedditSubscription);
  
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

      bindClick("btn-gen-reddit-global-secret", async () => {
        try {
          const res = await apiFetch("/api/reddit/generate-secret", { method: "POST" });
          if (res.secret) {
            setValue("reddit-global-webhook-secret", res.secret);
            showToast("Generated new webhook secret (click Save Global Secret to persist).");
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

      wireUpdateChannelAutosave();
      bindDurationPreview("am-new-step-duration", "am-new-step-duration-preview");
      bindDurationPreview("edit-ladder-duration", "edit-ladder-duration-preview");
      bindDurationPreview("mcase-duration", "mcase-duration-preview");
      wireReactionRoleMessageLink();

      const pSel = document.getElementById("cfg-ai-provider");
      if (pSel) pSel.addEventListener("change", updateActiveProviderDisplay);

      bindClick("btn-submit-edit-ladder", submitEditLadderStep);

      // Reddit & Devvit
      bindClick("btn-refresh-reddit", loadRedditModule);
      bindClick("btn-save-reddit-global", saveRedditGlobalSettings);
      bindClick("btn-add-reddit-companion-sub", addRedditSubscription);

      // Grants
      bindClick("btn-refresh-grants", loadGrantsModule);
      bindClick("btn-create-grant", createGrant);

      // Audit Log
      bindClick("btn-refresh-audit", loadAuditLogModule);
      const auditDomainSel = document.getElementById("audit-filter-domain");
      if (auditDomainSel) {
        auditDomainSel.addEventListener("change", () => {
          auditCurrentOffset = 0;
          loadAuditLogModule();
        });
      }
      bindClick("btn-audit-prev", () => {
        if (auditCurrentOffset >= AUDIT_PAGE_SIZE) {
          auditCurrentOffset -= AUDIT_PAGE_SIZE;
          loadAuditLogModule();
        }
      });
      bindClick("btn-audit-next", () => {
        auditCurrentOffset += AUDIT_PAGE_SIZE;
        loadAuditLogModule();
      });
  }

  async function checkSessionAndLoadData() {
    try {
      const sess = await apiFetch('/api/auth/session');
      currentUserSession = sess;
      if (!sess.authenticated) {
        window.location.href = '/login.html';
        return;
      }
      const roleSuffix = sess.is_owner ? ' (Bot Owner)' : '';
      setText('session-username', (sess.username || 'Admin') + roleSuffix);
    } catch (_) {
      window.location.href = '/login.html';
      return;
    }

    if (currentUserSession && currentUserSession.is_owner) {
      try {
        await Promise.allSettled([
          refreshStatus(),
          loadGlobalSettings(),
          loadGuildList(),
          loadSnapshots(),
        ]);
      } catch (_) {}
      connectTelemetryWebSocket();
    } else {
      ["overview", "backends", "updates", "invite"].forEach((v) => {
        const navBtn = document.querySelector(`.nav-item[data-view="${v}"]`);
        if (navBtn) navBtn.classList.add("hidden");
      });
      try {
        await Promise.allSettled([
          refreshStatus(),
          loadGuildList(),
        ]);
      } catch (_) {}
    }

    setInterval(refreshStatus, 10000);
  }


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
        tdUser.textContent = resolveMemberLabel(c.user_id);
        tdUser.title = `ID: ${c.user_id}`;
        tr.appendChild(tdUser);

        // Moderator
        const tdMod = document.createElement("td");
        tdMod.textContent = c.moderator_id === 0 ? "System / Automod" : resolveMemberLabel(c.moderator_id);
        tdMod.title = `ID: ${c.moderator_id}`;
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
          tdDuration.textContent = formatDurationHuman(c.duration_seconds);
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
    let durationSeconds = null;
    if (rawDuration && rawDuration.trim()) {
      const parsed = parseDurationJs(rawDuration);
      if (!parsed.valid) {
        showToast(`Invalid duration: ${parsed.error}`);
        return;
      }
      durationSeconds = parsed.seconds;
    }

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
      const preview = document.getElementById("mcase-duration-preview");
      if (preview) {
        preview.className = "duration-preview-text";
        preview.textContent = "Preview: Permanent";
      }
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
      await Promise.all([getGuildChannels(currentGuildId), getGuildRoles(currentGuildId)]);
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
      setChecked("am-caps-pct", String(currentAutomodConfig.anti_caps_percent ?? 70));
      setChecked("am-mention-enabled", Boolean((currentAutomodConfig.anti_mention_max ?? 0) > 0));
      setValue("am-mention-max", String(currentAutomodConfig.anti_mention_max ?? 5));

      setValue("am-blocked-words", (currentAutomodConfig.blocked_words || []).join(", "));
      setValue("am-whitelisted-domains", (currentAutomodConfig.whitelisted_domains || []).join(", "));
      setValue("am-ignored-channels", currentAutomodConfig.ignored_channels || []);
      setValue("am-ignored-roles", currentAutomodConfig.ignored_roles || []);

      const weights = currentAutomodConfig.strike_weights || {};
      setValue("am-weight-spam", String(weights.spam ?? 2));
      setValue("am-weight-invite", String(weights.invite ?? 3));
      setValue("am-weight-badword", String(weights.bad_word ?? 2));
      setValue("am-weight-caps", String(weights.caps ?? 1));
      setValue("am-weight-mention", String(weights.mention ?? 3));
      setValue("am-decay-hours", String(currentAutomodConfig.strike_decay_hours ?? 168));

      comboboxRegistry.forEach((cb) => cb.refresh());
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
      else if (act === "warn") bClass = "badge-purple";
      badgeA.className = `badge ${bClass}`;
      badgeA.textContent = step.action.toUpperCase();
      tdAction.appendChild(badgeA);
      tr.appendChild(tdAction);

      const tdDur = document.createElement("td");
      if (act === "warn" || act === "kick") {
        tdDur.textContent = "N/A";
      } else if (!step.duration_seconds || step.duration_seconds === 0) {
        tdDur.textContent = "Permanent";
      } else {
        tdDur.textContent = formatDurationHuman(step.duration_seconds);
      }
      tr.appendChild(tdDur);

      const tdActions = document.createElement("td");
      tdActions.style.display = "flex";
      tdActions.style.gap = "0.4rem";

      const btnEdit = document.createElement("button");
      btnEdit.type = "button";
      btnEdit.className = "btn btn-secondary btn-sm";
      btnEdit.textContent = "Edit";
      btnEdit.addEventListener("click", () => {
        openEditLadderModal(idx, step);
      });
      tdActions.appendChild(btnEdit);

      const btnDel = document.createElement("button");
      btnDel.type = "button";
      btnDel.className = "btn btn-danger btn-sm";
      btnDel.textContent = "Remove";
      btnDel.addEventListener("click", () => {
        currentAutomodConfig.punishment_ladder.splice(idx, 1);
        renderLadderTable();
      });
      tdActions.appendChild(btnDel);
      tr.appendChild(tdActions);

      tbody.appendChild(tr);
    });
  }

  function openEditLadderModal(idx, step) {
    setValue("edit-ladder-index", String(idx));
    setValue("edit-ladder-strikes", String(step.strikes));
    setValue("edit-ladder-action", (step.action || "warn").toLowerCase());
    const durStr = step.duration_seconds ? formatDurationHuman(step.duration_seconds) : "";
    setValue("edit-ladder-duration", durStr);
    const prev = document.getElementById("edit-ladder-duration-preview");
    if (prev) {
      prev.className = "duration-preview-text";
      prev.textContent = `Preview: ${step.duration_seconds ? durStr : "Permanent"}`;
    }
    openModal("modal-edit-ladder-step");
  }

  function submitEditLadderStep() {
    if (!currentAutomodConfig) return;
    const idx = parseInt(getValue("edit-ladder-index"), 10);
    if (isNaN(idx) || idx < 0 || idx >= (currentAutomodConfig.punishment_ladder || []).length) {
      closeModal("modal-edit-ladder-step");
      return;
    }
    const strikes = parseInt(getValue("edit-ladder-strikes"), 10) || 1;
    const action = getValue("edit-ladder-action") || "warn";
    const durRaw = getValue("edit-ladder-duration");
    const durParsed = parseDurationJs(durRaw);
    if (!durParsed.valid) {
      showToast(`Invalid duration: ${durParsed.error}`);
      return;
    }

    currentAutomodConfig.punishment_ladder[idx] = {
      strikes,
      action,
      duration_seconds: durParsed.seconds,
    };
    currentAutomodConfig.punishment_ladder.sort((a, b) => a.strikes - b.strikes);
    renderLadderTable();
    closeModal("modal-edit-ladder-step");
    showToast("Updated punishment ladder step.");
  }

  function addLadderStep() {
    if (!currentAutomodConfig) return;
    const strikes = parseInt(getValue("am-new-step-strikes"), 10) || 1;
    const action = getValue("am-new-step-action") || "warn";
    const durRaw = getValue("am-new-step-duration");
    const durParsed = parseDurationJs(durRaw);
    if (!durParsed.valid) {
      showToast(`Invalid duration: ${durParsed.error}`);
      return;
    }

    if (!currentAutomodConfig.punishment_ladder) {
      currentAutomodConfig.punishment_ladder = [];
    }
    currentAutomodConfig.punishment_ladder.push({
      strikes,
      action,
      duration_seconds: durParsed.seconds,
    });
    currentAutomodConfig.punishment_ladder.sort((a, b) => a.strikes - b.strikes);
    renderLadderTable();
    setValue("am-new-step-duration", "");
    const preview = document.getElementById("am-new-step-duration-preview");
    if (preview) {
      preview.className = "duration-preview-text";
      preview.textContent = "Preview: Permanent";
    }
  }

  async function saveAutomodConfig() {
    if (!currentGuildId || !currentAutomodConfig) return;

    const parseList = (val) =>
      (val || "")
        .split(/[,\n]+/)
        .map((s) => s.trim())
        .filter((s) => s.length > 0);

    const parseSnowflakeList = (val) => {
      if (Array.isArray(val)) return val.map(String).map((s) => s.trim()).filter((s) => s.length > 0);
      return parseList(val);
    };

    const weights = {
      spam: parseInt(getValue("am-weight-spam"), 10) || 2,
      invite: parseInt(getValue("am-weight-invite"), 10) || 3,
      bad_word: parseInt(getValue("am-weight-badword"), 10) || 2,
      caps: parseInt(getValue("am-weight-caps"), 10) || 1,
      mention: parseInt(getValue("am-weight-mention"), 10) || 3,
    };

    const payload = {
      guild_id: String(currentGuildId),
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
      ignored_channels: parseSnowflakeList(comboboxRegistry.has("am-ignored-channels") ? comboboxRegistry.get("am-ignored-channels").getValue() : getValue("am-ignored-channels")),
      ignored_roles: parseSnowflakeList(comboboxRegistry.has("am-ignored-roles") ? comboboxRegistry.get("am-ignored-roles").getValue() : getValue("am-ignored-roles")),
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
      await Promise.all([getGuildChannels(currentGuildId), getGuildRoles(currentGuildId)]);
      const res = await apiFetch(`/api/guilds/${currentGuildId}/logging`);
      const cfg = res.config || res;

      setValue("log-unified-channel", cfg.unified_channel_id ? String(cfg.unified_channel_id) : "");
      setValue("log-mod-channel", cfg.mod_channel_id ? String(cfg.mod_channel_id) : "");
      setValue("log-message-channel", cfg.message_channel_id ? String(cfg.message_channel_id) : "");
      setValue("log-member-channel", cfg.member_channel_id ? String(cfg.member_channel_id) : "");
      setValue("log-server-channel", cfg.server_channel_id ? String(cfg.server_channel_id) : "");
      setValue("log-voice-channel", cfg.voice_channel_id ? String(cfg.voice_channel_id) : "");

      setChecked("log-bots", Boolean(cfg.log_bots));
      setValue("log-ignored-channels", cfg.ignored_channels || []);
      setValue("log-ignored-roles", cfg.ignored_roles || []);

      const colNum = cfg.embed_color || 3553599;
      const hex = `#${colNum.toString(16).padStart(6, "0")}`;
      setValue("log-embed-color-picker", hex);
      setValue("log-embed-color-hex", hex);

      comboboxRegistry.forEach((cb) => cb.refresh());
    } catch (err) {
      showToast(`Failed to load logging config: ${err.message}`);
    }
  }

  async function saveLoggingConfig() {
    if (!currentGuildId) return;

    const parseOptId = (val) => {
      const trimmed = val ? String(val).trim() : "";
      return trimmed.length > 0 ? trimmed : null;
    };

    const parseSnowflakeList = (val) => {
      if (Array.isArray(val)) return val.map(String).map((s) => s.trim()).filter((s) => s.length > 0);
      return (val || "")
        .split(",")
        .map((s) => s.trim())
        .filter((s) => s.length > 0);
    };

    const hexVal = (getValue("log-embed-color-hex") || "#36393f").replace("#", "");
    const embedColor = parseInt(hexVal, 16) || 3553599;

    const payload = {
      guild_id: String(currentGuildId),
      unified_channel_id: parseOptId(getValue("log-unified-channel")),
      mod_channel_id: parseOptId(getValue("log-mod-channel")),
      message_channel_id: parseOptId(getValue("log-message-channel")),
      member_channel_id: parseOptId(getValue("log-member-channel")),
      server_channel_id: parseOptId(getValue("log-server-channel")),
      voice_channel_id: parseOptId(getValue("log-voice-channel")),
      ignored_channels: parseSnowflakeList(comboboxRegistry.has("log-ignored-channels") ? comboboxRegistry.get("log-ignored-channels").getValue() : getValue("log-ignored-channels")),
      ignored_roles: parseSnowflakeList(comboboxRegistry.has("log-ignored-roles") ? comboboxRegistry.get("log-ignored-roles").getValue() : getValue("log-ignored-roles")),
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
      await Promise.all([getGuildChannels(currentGuildId), getGuildRoles(currentGuildId)]);
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

      setValue("greet-member-roles", cfg.member_autorole_ids || []);
      setValue("greet-bot-roles", cfg.bot_autorole_ids || []);
      setValue("greet-delay-seconds", String(cfg.autorole_delay_seconds ?? 0));

      comboboxRegistry.forEach((cb) => cb.refresh());
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
      const trimmed = val ? String(val).trim() : "";
      return trimmed.length > 0 ? trimmed : null;
    };

    const parseSnowflakeList = (val) => {
      if (Array.isArray(val)) return val.map(String).map((s) => s.trim()).filter((s) => s.length > 0);
      return (val || "")
        .split(",")
        .map((s) => s.trim())
        .filter((s) => s.length > 0);
    };

    const payload = {
      guild_id: String(currentGuildId),
      welcome_channel_id: parseOptId(getValue("greet-welcome-channel")),
      welcome_message: getValue("greet-welcome-msg") || "",
      welcome_embed_json: getValue("greet-welcome-embed") || "",
      welcome_dm_enabled: getChecked("greet-dm-enabled"),
      welcome_dm_message: getValue("greet-dm-msg") || "",
      farewell_channel_id: parseOptId(getValue("greet-farewell-channel")),
      farewell_message: getValue("greet-farewell-msg") || "",
      farewell_embed_json: getValue("greet-farewell-embed") || "",
      member_autorole_ids: parseSnowflakeList(comboboxRegistry.has("greet-member-roles") ? comboboxRegistry.get("greet-member-roles").getValue() : getValue("greet-member-roles")),
      bot_autorole_ids: parseSnowflakeList(comboboxRegistry.has("greet-bot-roles") ? comboboxRegistry.get("greet-bot-roles").getValue() : getValue("greet-bot-roles")),
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
        titleH4.textContent = resolveChannelLabel(g.channel_id);
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
          reqDiv.textContent = `Requires Role: ${resolveRoleLabel(g.required_role_id)}`;
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
            chip.textContent = `${e.emoji_or_custom_id} ➔ ${resolveRoleLabel(e.role_id)}${label}`;
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
    row.style.gap = "0.5rem";

    const inEmoji = document.createElement("input");
    inEmoji.type = "text";
    inEmoji.className = "input-text";
    inEmoji.placeholder = "Emoji (e.g. 👍)";
    inEmoji.value = emoji;
    inEmoji.style.width = "100px";
    row.appendChild(inEmoji);

    const cbRoleContainer = document.createElement("div");
    cbRoleContainer.className = "combobox-container";
    cbRoleContainer.style.flex = "2";
    row.appendChild(cbRoleContainer);

    const cbRole = initRoleCombobox(cbRoleContainer, {
      multiSelect: false,
      placeholder: "Select target role...",
      initialValue: roleId || null,
    });
    row._cbRole = cbRole;

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
      const emoji = inputs[0] ? inputs[0].value.trim() : "";
      const role = r._cbRole ? (r._cbRole.getValue() || "") : "";
      const lbl = inputs[1] ? inputs[1].value.trim() : "";
      const sel = r.querySelector("select");
      const style = sel ? sel.value : "primary";
      if (emoji && role) {
        entries.push({
          emoji_or_custom_id: emoji,
          role_id: String(role).trim(),
          button_label: lbl.length > 0 ? lbl : null,
          button_style: style,
        });
      }
    });

    const payload = {
      channel_id: String(chId).trim(),
      message_id: String(msgId).trim(),
      group_type: groupType,
      required_role_id: requiredRoleId && String(requiredRoleId).trim().length > 0 ? String(requiredRoleId).trim() : null,
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

    const parseSnowflakeList = (val) => {
      if (Array.isArray(val)) return val.map(String).map((s) => s.trim()).filter((s) => s.length > 0);
      return (val || "")
        .split(",")
        .map((s) => s.trim())
        .filter((s) => s.length > 0);
    };

    const payload = {
      trigger_name: name.trim(),
      match_mode: getValue("trig-mode") || "prefix",
      response_template: response,
      cooldown_seconds: parseInt(getValue("trig-cooldown"), 10) || 0,
      allowed_roles: parseSnowflakeList(comboboxRegistry.has("trig-roles") ? comboboxRegistry.get("trig-roles").getValue() : getValue("trig-roles")),
      ignored_channels: parseSnowflakeList(comboboxRegistry.has("trig-channels") ? comboboxRegistry.get("trig-channels").getValue() : getValue("trig-channels")),
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

  // ==========================================================================
  // Release Channel Autosave
  // ==========================================================================
  function wireUpdateChannelAutosave() {
    const sel = document.getElementById("update-channel-select");
    const badge = document.getElementById("channel-status-badge");
    if (!sel) return;

    let previousVal = sel.value;

    function updateBadge(val) {
      if (!badge) return;
      if (val === "prerelease" || val === "latest") {
        badge.className = "badge badge-warning";
        badge.textContent = "Active: Pre-release";
      } else {
        badge.className = "badge badge-success";
        badge.textContent = "Active: Stable";
      }
    }

    updateBadge(sel.value);

    sel.addEventListener("change", async () => {
      const newVal = sel.value;
      updateBadge(newVal);
      try {
        await apiFetch("/api/settings", {
          method: "PUT",
          body: JSON.stringify({ update_channel: newVal }),
        });
        previousVal = newVal;
        showToast(`Release channel updated to: ${newVal === "prerelease" ? "Pre-release" : "Stable"}`);
      } catch (err) {
        sel.value = previousVal;
        updateBadge(previousVal);
        showToast(`Failed to update release channel: ${err.message}`);
      }
    });
  }

  // ==========================================================================
  // Reaction Roles Message Link Parser
  // ==========================================================================
  function wireReactionRoleMessageLink() {
    const linkInput = document.getElementById("rr-input-message-link");
    const statusEl = document.getElementById("rr-link-status");
    const channelInput = document.getElementById("rr-input-channel");
    const messageInput = document.getElementById("rr-input-message");
    if (!linkInput) return;

    linkInput.addEventListener("input", () => {
      const url = linkInput.value.trim();
      const match = url.match(/discord(?:app)?\.com\/channels\/(\d+)\/(\d+)\/(\d+)/);
      if (match) {
        const [, , channelId, messageId] = match;
        if (messageInput) messageInput.value = messageId;
        if (channelInput) {
          channelInput.value = channelId;
          if (channelInput._comboboxSet) {
            channelInput._comboboxSet(channelId);
          }
        }
        if (statusEl) {
          statusEl.classList.remove("hidden");
          statusEl.className = "duration-preview-text";
          statusEl.textContent = `Extracted: Channel ${channelId}, Message ${messageId}`;
        }
      } else if (url.length > 0 && statusEl) {
        statusEl.classList.remove("hidden");
        statusEl.className = "duration-preview-text duration-preview-error";
        statusEl.textContent = "Invalid message link format.";
      } else if (statusEl) {
        statusEl.classList.add("hidden");
      }
    });
  }

  // ==========================================================================
  // Reddit & Devvit Companion Module
  // ==========================================================================
  async function loadRedditModule() {
    if (!currentGuildId) return;

    const globalCard = document.getElementById("reddit-global-card");
    if (globalCard) {
      if (currentUserSession && currentUserSession.is_owner) {
        globalCard.classList.remove("hidden");
        try {
          const s = await apiFetch("/api/settings");
          const setVal = (id, val) => {
            const el = document.getElementById(id);
            if (el) el.value = val;
          };
          setVal("reddit-global-webhook-secret", s.reddit_webhook_secret || "");
        } catch (_) {}
      } else {
        globalCard.classList.add("hidden");
      }
    }

    const tbody = document.getElementById("reddit-subs-table-body");
    const countBadge = document.getElementById("reddit-subs-count-badge");
    if (!tbody) return;

    tbody.replaceChildren();

    const cbChannelContainer = document.getElementById("cb-reddit-channel");
    if (cbChannelContainer && !cbChannelContainer._combobox) {
      cbChannelContainer._combobox = createCombobox({
        container: cbChannelContainer,
        placeholder: "Search text channel...",
        multiSelect: false,
        getItems: async (q) => {
          const channels = await getGuildChannels(currentGuildId);
          return channels
            .filter((c) => c.kind === "text" || c.kind === "announcement")
            .filter((c) => !q || c.name.toLowerCase().includes(q.toLowerCase()))
            .map((c) => ({
              id: c.id,
              label: `#${c.name} ${formatChannelKind(c.kind)}`,
              sublabel: c.category_name ? `in ${c.category_name}` : undefined,
            }));
        },
        resolveLabel: (id) => {
          const channels = discordGuildData.channels.get(currentGuildId) || [];
          const found = channels.find((c) => String(c.id) === String(id));
          return found ? `#${found.name} ${formatChannelKind(found.kind)}` : `Channel ${id}`;
        },
        onChange: (id) => {
          cbChannelContainer._selectedChannelId = id;
        },
      });
    }

    try {
      const res = await apiFetch(`/api/guilds/${currentGuildId}/reddit`);
      const subs = res.subscriptions || [];
      if (countBadge) countBadge.textContent = `${subs.length} ${subs.length === 1 ? "feed" : "feeds"}`;

      if (subs.length === 0) {
        const tr = document.createElement("tr");
        const td = document.createElement("td");
        td.colSpan = 4;
        td.className = "muted text-center";
        td.textContent = "No subreddit feeds configured for this server. Add one below.";
        tr.appendChild(td);
        tbody.appendChild(tr);
        return;
      }

      const channels = await getGuildChannels(currentGuildId);

      subs.forEach((sub) => {
        const tr = document.createElement("tr");

        const tdSub = document.createElement("td");
        tdSub.textContent = `r/${sub.subreddit}`;
        tr.appendChild(tdSub);

        const tdCh = document.createElement("td");
        const ch = channels.find((c) => String(c.id) === String(sub.channel_id));
        tdCh.textContent = ch ? `#${ch.name} ${formatChannelKind(ch.kind)}` : `Channel ${sub.channel_id}`;
        tr.appendChild(tdCh);

        const tdFilter = document.createElement("td");
        const filterBadge = document.createElement("span");
        filterBadge.className = "badge badge-neutral";
        filterBadge.textContent = sub.post_filter || "new";
        tdFilter.appendChild(filterBadge);
        tr.appendChild(tdFilter);

        const tdAction = document.createElement("td");
        const btnDel = document.createElement("button");
        btnDel.type = "button";
        btnDel.className = "btn btn-danger btn-sm";
        btnDel.textContent = "Delete";
        btnDel.addEventListener("click", async () => {
          try {
            await apiFetch(`/api/guilds/${currentGuildId}/reddit/${sub.id}`, {
              method: "DELETE",
            });
            showToast(`Deleted feed r/${sub.subreddit}.`);
            await loadRedditModule();
          } catch (e) {
            showToast(`Failed to delete feed: ${e.message}`);
          }
        });
        tdAction.appendChild(btnDel);
        tr.appendChild(tdAction);

        tbody.appendChild(tr);
      });
    } catch (e) {
      showToast(`Failed to load Reddit feeds: ${e.message}`);
    }
  }

  async function saveRedditGlobalSettings() {
    const webhookSecret = getValue("reddit-global-webhook-secret");

    try {
      await apiFetch("/api/settings", {
        method: "PUT",
        body: JSON.stringify({
          reddit_webhook_secret: webhookSecret,
        }),
      });
      showToast("Saved global companion webhook secret.");
    } catch (e) {
      showToast(`Failed to save settings: ${e.message}`);
    }
  }

  async function addRedditSubscription() {
    if (!currentGuildId) return;
    const subName = getValue("reddit-new-sub-name");
    const cbChannelContainer = document.getElementById("cb-reddit-channel");
    const channelId = cbChannelContainer ? cbChannelContainer._selectedChannelId : null;
    const filter = getValue("reddit-new-sub-filter") || "new";

    if (!subName || !channelId) {
      showToast("Subreddit name and destination channel are required.");
      return;
    }

    try {
      await apiFetch(`/api/guilds/${currentGuildId}/reddit`, {
        method: "POST",
        body: JSON.stringify({
          subreddit: subName.trim(),
          channel_id: String(channelId),
          post_filter: filter,
        }),
      });
      showToast(`Added feed r/${subName.trim()}.`);
      setValue("reddit-new-sub-name", "");
      await loadRedditModule();
    } catch (e) {
      showToast(`Failed to add feed: ${e.message}`);
    }
  }

  // ==========================================================================
  // Governance & Delegated Access Module (Feature F1)
  // ==========================================================================
  async function loadGrantsModule() {
    if (!currentGuildId) return;

    const tbody = document.getElementById("grants-table-body");
    const countBadge = document.getElementById("grants-count-badge");
    if (!tbody) return;

    tbody.replaceChildren();

    const cbPrincipalContainer = document.getElementById("cb-grant-principal");
    const typeSel = document.getElementById("grant-principal-type");

    function setupGrantPrincipalCombobox() {
      if (!cbPrincipalContainer) return;
      const isRole = !typeSel || typeSel.value === "role";

      cbPrincipalContainer._combobox = createCombobox({
        container: cbPrincipalContainer,
        placeholder: isRole ? "Search role..." : "Search member by name (min 2 chars)...",
        multiSelect: false,
        getItems: async (q) => {
          if (isRole) {
            const roles = await getGuildRoles(currentGuildId);
            return roles
              .filter((r) => !q || r.name.toLowerCase().includes(q.toLowerCase()))
              .map((r) => ({
                id: r.id,
                label: `@${r.name}`,
              }));
          } else {
            const members = await searchGuildMembers(currentGuildId, q);
            return members.map((m) => {
              const display = m.nick || m.global_name || m.username;
              memberCache.set(String(m.id), {
                id: m.id,
                username: m.username,
                display_name: display,
              });
              return {
                id: m.id,
                label: `${display} (@${m.username})`,
              };
            });
          }
        },
        resolveLabel: (id) => {
          if (isRole) {
            const roles = discordGuildData.roles.get(currentGuildId) || [];
            const r = roles.find((x) => String(x.id) === String(id));
            return r ? `@${r.name}` : `Role ${id}`;
          } else {
            return resolveMemberLabel(id);
          }
        },
        onChange: (id) => {
          cbPrincipalContainer._selectedPrincipalId = id;
        },
      });
    }

    if (typeSel && !typeSel._hasGrantListener) {
      typeSel._hasGrantListener = true;
      typeSel.addEventListener("change", () => {
        setupGrantPrincipalCombobox();
      });
    }
    setupGrantPrincipalCombobox();

    try {
      const res = await apiFetch(`/api/guilds/${currentGuildId}/grants`);
      const grants = res.grants || [];
      if (countBadge) countBadge.textContent = `${grants.length} ${grants.length === 1 ? "grant" : "grants"}`;

      if (grants.length === 0) {
        const tr = document.createElement("tr");
        const td = document.createElement("td");
        td.colSpan = 6;
        td.className = "muted text-center";
        td.textContent = "No delegated access grants configured. Administrators and Bot Owners have bypass.";
        tr.appendChild(td);
        tbody.appendChild(tr);
        return;
      }

      const roles = await getGuildRoles(currentGuildId);

      grants.forEach((grant) => {
        const tr = document.createElement("tr");
        const pType = (grant.target_type || grant.principal_type || "user").toLowerCase();
        const pId = grant.target_id || grant.principal_id || "";
        const pName = grant.target_name || "";

        const tdType = document.createElement("td");
        const typeBadge = document.createElement("span");
        typeBadge.className = pType === "role" ? "badge badge-cyan" : "badge badge-purple";
        typeBadge.textContent = pType === "role" ? "ROLE" : "USER";
        tdType.appendChild(typeBadge);
        tr.appendChild(tdType);

        const tdPrincipal = document.createElement("td");
        if (pType === "role") {
          const r = roles.find((x) => String(x.id) === String(pId));
          if (r) {
            tdPrincipal.textContent = `@${r.name} (${pId})`;
          } else if (pName && pName !== "Unknown Target") {
            tdPrincipal.textContent = `${pName} (${pId})`;
          } else {
            tdPrincipal.textContent = `Role ID ${pId}`;
          }
        } else if (pName && pName !== "Unknown Target") {
          tdPrincipal.textContent = `${pName} (${pId})`;
        } else {
          tdPrincipal.textContent = `User ID ${pId}`;
        }
        tr.appendChild(tdPrincipal);

        const tdDomain = document.createElement("td");
        const domBadge = document.createElement("span");
        domBadge.className = "badge badge-neutral";
        domBadge.textContent = grant.domain === "*" ? "ALL (*)" : String(grant.domain || "").toUpperCase();
        tdDomain.appendChild(domBadge);
        tr.appendChild(tdDomain);

        const tdLevel = document.createElement("td");
        const lvlBadge = document.createElement("span");
        const normLvl = String(grant.access_level || "view").toLowerCase();
        if (normLvl === "admin" || normLvl === "edit" || normLvl === "write") {
          lvlBadge.className = "badge badge-warning";
          lvlBadge.textContent = "EDIT (READ & WRITE)";
        } else {
          lvlBadge.className = "badge badge-success";
          lvlBadge.textContent = "VIEW ONLY";
        }
        tdLevel.appendChild(lvlBadge);
        tr.appendChild(tdLevel);

        const tdDate = document.createElement("td");
        tdDate.textContent = grant.updated_at
          ? new Date(grant.updated_at * 1000).toLocaleString()
          : grant.created_at
          ? new Date(grant.created_at * 1000).toLocaleString()
          : "N/A";
        tr.appendChild(tdDate);

        const tdAction = document.createElement("td");
        const btnRevoke = document.createElement("button");
        btnRevoke.type = "button";
        btnRevoke.className = "btn btn-danger btn-sm";
        btnRevoke.textContent = "Revoke";
        btnRevoke.disabled = !hasGuildDomainAccess(currentGuildId, "permissions", "edit");
        btnRevoke.addEventListener("click", async () => {
          try {
            await apiFetch(`/api/guilds/${currentGuildId}/grants/${grant.id}`, {
              method: "DELETE",
            });
            showToast("Revoked console access grant.");
            await loadGrantsModule();
          } catch (e) {
            showToast(`Failed to revoke grant: ${e.message}`);
          }
        });
        tdAction.appendChild(btnRevoke);
        tr.appendChild(tdAction);

        tbody.appendChild(tr);
      });
    } catch (e) {
      showToast(`Failed to load access grants: ${e.message}`);
    }
  }

  async function createGrant() {
    if (!currentGuildId) return;
    const typeSel = document.getElementById("grant-principal-type");
    const cbPrincipalContainer = document.getElementById("cb-grant-principal");
    const domainSel = document.getElementById("grant-domain");
    const levelSel = document.getElementById("grant-level");

    const principalType = typeSel ? typeSel.value : "role";
    const principalId = cbPrincipalContainer ? cbPrincipalContainer._selectedPrincipalId : null;
    const domain = domainSel ? domainSel.value : "*";
    const accessLevel = levelSel ? levelSel.value : "view";

    if (!principalId) {
      showToast("Please select a Discord role or member.");
      return;
    }

    let targetName = "Unknown Target";
    if (principalType === "role") {
      const roles = discordGuildData.roles.get(currentGuildId) || [];
      const r = roles.find((x) => String(x.id) === String(principalId));
      if (r && r.name) targetName = `@${r.name}`;
    } else {
      const m = memberCache.get(String(principalId));
      if (m) targetName = m.display_name ? `${m.display_name} (@${m.username})` : `@${m.username}`;
    }

    try {
      await apiFetch(`/api/guilds/${currentGuildId}/grants`, {
        method: "POST",
        body: JSON.stringify({
          target_type: principalType,
          target_id: String(principalId),
          target_name: targetName,
          domain,
          access_level: accessLevel,
        }),
      });
      showToast("Access grant created successfully.");
      await loadGrantsModule();
    } catch (e) {
      showToast(`Failed to create grant: ${e.message}`);
    }
  }

  // ==========================================================================
  // Configuration Audit Trail Module (Feature F2)
  // ==========================================================================
  let auditCurrentOffset = 0;
  const AUDIT_PAGE_SIZE = 25;

  async function loadAuditLogModule() {
    if (!currentGuildId) return;

    const tbody = document.getElementById("audit-table-body");
    const domainSel = document.getElementById("audit-filter-domain");
    const pageInd = document.getElementById("audit-page-indicator");
    const btnPrev = document.getElementById("btn-audit-prev");
    const btnNext = document.getElementById("btn-audit-next");
    if (!tbody) return;

    tbody.replaceChildren();

    const domain = domainSel ? domainSel.value : "";
    let url = `/api/guilds/${currentGuildId}/audit-log?limit=${AUDIT_PAGE_SIZE}&offset=${auditCurrentOffset}`;
    if (domain) {
      url += `&domain=${encodeURIComponent(domain)}`;
    }

    if (pageInd) {
      const pageNum = Math.floor(auditCurrentOffset / AUDIT_PAGE_SIZE) + 1;
      pageInd.textContent = `Page ${pageNum}`;
    }
    if (btnPrev) btnPrev.disabled = auditCurrentOffset === 0;

    try {
      const res = await apiFetch(url);
      const logs = res.logs || [];

      if (btnNext) btnNext.disabled = logs.length < AUDIT_PAGE_SIZE;

      if (logs.length === 0) {
        const tr = document.createElement("tr");
        const td = document.createElement("td");
        td.colSpan = 7;
        td.className = "muted text-center";
        td.textContent = "No audit log events found matching criteria.";
        tr.appendChild(td);
        tbody.appendChild(tr);
        return;
      }

      logs.forEach((log) => {
        const tr = document.createElement("tr");

        const tdTime = document.createElement("td");
        tdTime.textContent = log.created_at ? new Date(log.created_at * 1000).toLocaleString() : "N/A";
        tr.appendChild(tdTime);

        const tdActor = document.createElement("td");
        tdActor.textContent = `${log.user_name || "Unknown"} (${log.user_id})`;
        tr.appendChild(tdActor);

        const tdDomain = document.createElement("td");
        const dBadge = document.createElement("span");
        dBadge.className = "badge badge-neutral";
        dBadge.textContent = log.domain.toUpperCase();
        tdDomain.appendChild(dBadge);
        tr.appendChild(tdDomain);

        const tdAction = document.createElement("td");
        tdAction.textContent = log.action;
        tr.appendChild(tdAction);

        const tdStatus = document.createElement("td");
        const sBadge = document.createElement("span");
        sBadge.className = log.status === "success" ? "badge badge-success" : "badge badge-danger";
        sBadge.textContent = log.status.toUpperCase();
        tdStatus.appendChild(sBadge);
        tr.appendChild(tdStatus);

        const tdIp = document.createElement("td");
        tdIp.textContent = log.ip_address || "127.0.0.1";
        tr.appendChild(tdIp);

        const tdDiff = document.createElement("td");
        if (log.before_json || log.after_json) {
          const btnDiff = document.createElement("button");
          btnDiff.type = "button";
          btnDiff.className = "btn btn-secondary btn-sm";
          btnDiff.textContent = "View Diff";
          btnDiff.addEventListener("click", () => {
            openAuditDiffModal(log);
          });
          tdDiff.appendChild(btnDiff);
        } else {
          tdDiff.textContent = "-";
        }
        tr.appendChild(tdDiff);

        tbody.appendChild(tr);
      });
    } catch (e) {
      showToast(`Failed to load audit logs: ${e.message}`);
    }
  }

  function openAuditDiffModal(log) {
    const meta = document.getElementById("audit-diff-meta");
    const preBefore = document.getElementById("audit-diff-before");
    const preAfter = document.getElementById("audit-diff-after");

    if (meta) {
      meta.textContent = `Action: ${log.action} | Domain: ${log.domain} | Actor: ${log.user_name} (${log.user_id}) | Time: ${new Date(log.created_at * 1000).toLocaleString()}`;
    }

    if (preBefore) {
      try {
        preBefore.textContent = log.before_json ? JSON.stringify(JSON.parse(log.before_json), null, 2) : "None (New Resource)";
      } catch (_) {
        preBefore.textContent = log.before_json || "None";
      }
    }

    if (preAfter) {
      try {
        preAfter.textContent = log.after_json ? JSON.stringify(JSON.parse(log.after_json), null, 2) : "None (Deleted Resource)";
      } catch (_) {
        preAfter.textContent = log.after_json || "None";
      }
    }

    openModal("modal-audit-diff");
  }


  // ============================================================================
  // Activity, Levels & Rewards Module
  // ============================================================================

  let currentCommandRestrictions = {
    guild_id: "",
    default_allowed_channels: [],
    enabled: false,
  };
  let currentCommandOverrides = [];

  async function loadActivityModule() {
    if (!currentGuildId || currentGuildId === "0") {
      const sel = document.getElementById("activity-guild-selector") || document.getElementById("guild-selector");
      if (sel && sel.value && sel.value !== "0") {
        currentGuildId = sel.value;
      }
    }

    const noServerEl = document.getElementById("activity-no-server");
    const contentEl = document.getElementById("activity-content");

    if (!currentGuildId || currentGuildId === "0") {
      if (noServerEl) noServerEl.classList.remove("hidden");
      if (contentEl) contentEl.classList.add("hidden");
      return;
    }

    if (noServerEl) noServerEl.classList.add("hidden");
    if (contentEl) contentEl.classList.remove("hidden");

    try {
      await populateActivityRoleAndChannelDropdowns();
      await Promise.all([
        loadActivityConfig(),
        loadCommandChannelsConfig(),
        loadActivityRewards(),
        loadActivityLeaderboard(),
      ]);
    } catch (err) {
      showToast(`Error loading activity module: ${err.message}`);
    }
  }

  async function populateActivityRoleAndChannelDropdowns() {
    if (!currentGuildId) return;
    try {
      const [roles, channels] = await Promise.all([
        getGuildRoles(currentGuildId),
        getGuildChannels(currentGuildId),
      ]);

      // Populate Announcement Channel Dropdown
      const annSel = document.getElementById("activity-announce-channel");
      if (annSel) {
        annSel.replaceChildren();
        const defOpt = document.createElement("option");
        defOpt.value = "";
        defOpt.textContent = "Disabled (Silent)";
        annSel.appendChild(defOpt);

        channels.forEach((ch) => {
          const opt = document.createElement("option");
          opt.value = ch.id;
          opt.textContent = `#${ch.name}`;
          annSel.appendChild(opt);
        });
      }

      // Populate Command Channel Selectors
      const cmdDefSel = document.getElementById("cmd-default-channel-select");
      if (cmdDefSel) {
        cmdDefSel.replaceChildren();
        channels.forEach((ch) => {
          const opt = document.createElement("option");
          opt.value = ch.id;
          opt.textContent = `#${ch.name}`;
          cmdDefSel.appendChild(opt);
        });
      }

      const cmdOverSel = document.getElementById("cmd-override-channel-select");
      if (cmdOverSel) {
        cmdOverSel.replaceChildren();
        channels.forEach((ch) => {
          const opt = document.createElement("option");
          opt.value = ch.id;
          opt.textContent = `#${ch.name}`;
          cmdOverSel.appendChild(opt);
        });
      }

      // Populate Reward Modal Role Select
      const rewRoleSel = document.getElementById("reward-modal-role");
      if (rewRoleSel) {
        rewRoleSel.replaceChildren();
        roles.forEach((r) => {
          const opt = document.createElement("option");
          opt.value = r.id;
          opt.textContent = `@${r.name}`;
          rewRoleSel.appendChild(opt);
        });
      }
    } catch (err) {
      console.warn("Failed to populate role/channel dropdowns:", err);
    }
  }

  async function loadActivityConfig() {
    if (!currentGuildId) return;
    const res = await apiFetch(`/api/guilds/${currentGuildId}/activity/config`);
    const cfg = res.config || {};

    setChecked("activity-enabled", Boolean(cfg.enabled));
    setValue("activity-text-min", String(cfg.text_points_min ?? 15));
    setValue("activity-text-max", String(cfg.text_points_max ?? 25));
    setValue("activity-text-cooldown", String(cfg.text_cooldown_seconds ?? 60));
    setValue("activity-text-min-chars", String(cfg.text_min_chars ?? 5));
    setChecked("activity-voice-enabled", Boolean(cfg.voice_enabled));
    setValue("activity-voice-points", String(cfg.voice_points_per_interval ?? 5));
    setChecked("activity-voice-ignore-afk", Boolean(cfg.voice_ignore_afk));
    setChecked("activity-voice-ignore-deaf", Boolean(cfg.voice_ignore_deafened));
    setChecked("activity-reaction-enabled", Boolean(cfg.reaction_enabled));
    setValue("activity-reaction-give", String(cfg.reaction_give_points ?? 2));
    setValue("activity-reaction-recv", String(cfg.reaction_receive_points ?? 3));
    setValue("activity-reaction-cooldown", String(cfg.reaction_cooldown_seconds ?? 30));
    setValue("activity-announce-channel", cfg.announcement_channel_id || "");
    setChecked("activity-announce-promotions", Boolean(cfg.announce_promotions));
  }

  async function saveActivityConfig() {
    if (!currentGuildId) return;
    try {
      const payload = {
        guild_id: currentGuildId,
        enabled: getChecked("activity-enabled"),
        text_min_chars: parseInt(getValue("activity-text-min-chars"), 10) || 5,
        text_cooldown_seconds: parseInt(getValue("activity-text-cooldown"), 10) || 60,
        text_points_min: parseInt(getValue("activity-text-min"), 10) || 15,
        text_points_max: parseInt(getValue("activity-text-max"), 10) || 25,
        voice_enabled: getChecked("activity-voice-enabled"),
        voice_interval_minutes: 1,
        voice_points_per_interval: parseInt(getValue("activity-voice-points"), 10) || 5,
        voice_ignore_afk: getChecked("activity-voice-ignore-afk"),
        voice_ignore_deafened: getChecked("activity-voice-ignore-deaf"),
        reaction_enabled: getChecked("activity-reaction-enabled"),
        reaction_give_points: parseInt(getValue("activity-reaction-give"), 10) || 2,
        reaction_receive_points: parseInt(getValue("activity-reaction-recv"), 10) || 3,
        reaction_cooldown_seconds: parseInt(getValue("activity-reaction-cooldown"), 10) || 30,
        announcement_channel_id: getValue("activity-announce-channel") || null,
        announce_promotions: getChecked("activity-announce-promotions"),
        ignored_channels: [],
        ignored_roles: [],
        role_multipliers: {},
        channel_multipliers: {},
      };

      await apiFetch(`/api/guilds/${currentGuildId}/activity/config`, {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });

      showToast("Activity and participation settings saved successfully.");
    } catch (err) {
      showToast(`Failed to save activity settings: ${err.message}`);
    }
  }

  async function loadCommandChannelsConfig() {
    if (!currentGuildId) return;
    try {
      const res = await apiFetch(`/api/guilds/${currentGuildId}/command-channels`);
      currentCommandRestrictions = res.restrictions || {
        guild_id: currentGuildId,
        default_allowed_channels: [],
        enabled: false,
      };
      currentCommandOverrides = res.rules || [];

      setChecked("cmd-restr-enabled", Boolean(currentCommandRestrictions.enabled));
      renderDefaultCommandChannelsList();
      renderCommandOverridesTable();
    } catch (err) {
      console.warn("Failed to load command channel restrictions:", err);
    }
  }

  function renderDefaultCommandChannelsList() {
    const container = document.getElementById("cmd-default-channels-list");
    if (!container) return;
    container.replaceChildren();

    const channels = currentCommandRestrictions.default_allowed_channels || [];
    if (channels.length === 0) {
      const span = document.createElement("span");
      span.className = "muted";
      span.textContent = "No default command channels configured (all channels permitted).";
      container.appendChild(span);
      return;
    }

    channels.forEach((cid) => {
      const badge = document.createElement("span");
      badge.className = "badge badge-neutral";
      badge.style.display = "inline-flex";
      badge.style.alignItems = "center";
      badge.style.gap = "0.35rem";

      const txt = document.createElement("span");
      txt.textContent = resolveChannelLabel(cid);
      badge.title = `Channel ID: ${cid}`;
      badge.appendChild(txt);

      const delBtn = document.createElement("button");
      delBtn.type = "button";
      delBtn.textContent = "×";
      delBtn.style.cursor = "pointer";
      delBtn.style.background = "none";
      delBtn.style.border = "none";
      delBtn.style.color = "inherit";
      delBtn.style.fontWeight = "bold";
      delBtn.addEventListener("click", () => {
        currentCommandRestrictions.default_allowed_channels =
          currentCommandRestrictions.default_allowed_channels.filter((c) => c !== cid);
        renderDefaultCommandChannelsList();
      });
      badge.appendChild(delBtn);

      container.appendChild(badge);
    });
  }

  function addDefaultCommandChannel() {
    const sel = document.getElementById("cmd-default-channel-select");
    if (!sel || !sel.value) return;
    const cid = sel.value;
    if (!currentCommandRestrictions.default_allowed_channels.includes(cid)) {
      currentCommandRestrictions.default_allowed_channels.push(cid);
      renderDefaultCommandChannelsList();
    }
  }

  function renderCommandOverridesTable() {
    const tbody = document.getElementById("cmd-overrides-tbody");
    if (!tbody) return;
    tbody.replaceChildren();

    if (currentCommandOverrides.length === 0) {
      const tr = document.createElement("tr");
      const td = document.createElement("td");
      td.colSpan = 3;
      td.className = "text-center muted";
      td.textContent = "No command overrides configured.";
      tr.appendChild(td);
      tbody.appendChild(tr);
      return;
    }

    currentCommandOverrides.forEach((rule, idx) => {
      const tr = document.createElement("tr");

      const tdCmd = document.createElement("td");
      const code = document.createElement("code");
      code.textContent = `/${rule.command_name}`;
      tdCmd.appendChild(code);
      tr.appendChild(tdCmd);

      const tdCh = document.createElement("td");
      const channels = rule.allowed_channels || [];
      tdCh.textContent = channels.length > 0 ? channels.map((c) => resolveChannelLabel(c)).join(", ") : "-";
      tr.appendChild(tdCh);

      const tdAct = document.createElement("td");
      const delBtn = document.createElement("button");
      delBtn.type = "button";
      delBtn.className = "btn btn-danger btn-sm";
      delBtn.textContent = "Remove";
      delBtn.addEventListener("click", () => {
        currentCommandOverrides.splice(idx, 1);
        renderCommandOverridesTable();
      });
      tdAct.appendChild(delBtn);
      tr.appendChild(tdAct);

      tbody.appendChild(tr);
    });
  }

  function addCommandChannelOverride() {
    const nameIn = document.getElementById("cmd-override-name");
    const chanSel = document.getElementById("cmd-override-channel-select");
    if (!nameIn || !chanSel || !nameIn.value.trim() || !chanSel.value) {
      showToast("Please enter a command name and select an allowed channel.");
      return;
    }

    const cmdName = nameIn.value.trim().replace(/^\//, "").toLowerCase();
    const cid = chanSel.value;

    const existing = currentCommandOverrides.find((r) => r.command_name === cmdName);
    if (existing) {
      if (!existing.allowed_channels.includes(cid)) {
        existing.allowed_channels.push(cid);
      }
    } else {
      currentCommandOverrides.push({
        guild_id: currentGuildId,
        command_name: cmdName,
        allowed_channels: [cid],
        denied_channels: [],
      });
    }

    nameIn.value = "";
    renderCommandOverridesTable();
  }

  async function saveCommandChannelsConfig() {
    if (!currentGuildId) return;
    try {
      currentCommandRestrictions.enabled = getChecked("cmd-restr-enabled");
      const payload = {
        restrictions: currentCommandRestrictions,
        rules: currentCommandOverrides,
      };

      await apiFetch(`/api/guilds/${currentGuildId}/command-channels`, {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });

      showToast("Command channel restrictions saved.");
    } catch (err) {
      showToast(`Failed to save command channel restrictions: ${err.message}`);
    }
  }

  async function loadActivityRewards() {
    if (!currentGuildId) return;
    const tbody = document.getElementById("activity-rewards-tbody");
    if (!tbody) return;
    tbody.replaceChildren();

    try {
      const res = await apiFetch(`/api/guilds/${currentGuildId}/activity/rewards`);
      const rules = res.rules || [];

      if (rules.length === 0) {
        const tr = document.createElement("tr");
        const td = document.createElement("td");
        td.colSpan = 6;
        td.className = "text-center muted";
        td.textContent = "No automated reward rules configured. Click 'Add Reward Rule' above.";
        tr.appendChild(td);
        tbody.appendChild(tr);
        return;
      }

      rules.forEach((rule) => {
        const tr = document.createElement("tr");

        const tdRole = document.createElement("td");
        const roleBadge = document.createElement("span");
        roleBadge.className = "badge badge-neutral";
        roleBadge.textContent = rule.role_name ? `@${rule.role_name}` : resolveRoleLabel(rule.role_id);
        roleBadge.title = `Role ID: ${rule.role_id}`;
        tdRole.appendChild(roleBadge);
        tr.appendChild(tdRole);

        const tdType = document.createElement("td");
        tdType.textContent = rule.rule_type === "top_rank" ? "Top Rank" : "Threshold";
        tr.appendChild(tdType);

        const tdTime = document.createElement("td");
        tdTime.textContent = rule.timeframe;
        tr.appendChild(tdTime);

        const tdReq = document.createElement("td");
        tdReq.textContent = rule.rule_type === "top_rank" ? `Top #${rule.top_rank}` : `${rule.threshold_points} pts`;
        tr.appendChild(tdReq);

        const tdDrop = document.createElement("td");
        tdDrop.textContent = rule.remove_on_drop ? "Yes" : "No";
        tr.appendChild(tdDrop);

        const tdAct = document.createElement("td");
        const delBtn = document.createElement("button");
        delBtn.type = "button";
        delBtn.className = "btn btn-danger btn-sm";
        delBtn.textContent = "Delete";
        delBtn.addEventListener("click", async () => {
          try {
            await apiFetch(`/api/guilds/${currentGuildId}/activity/rewards/${encodeURIComponent(rule.id)}`, {
              method: "DELETE",
            });
            showToast("Reward rule deleted.");
            await loadActivityRewards();
          } catch (err) {
            showToast(`Failed to delete rule: ${err.message}`);
          }
        });
        tdAct.appendChild(delBtn);
        tr.appendChild(tdAct);

        tbody.appendChild(tr);
      });
    } catch (err) {
      console.warn("Failed to load activity rewards:", err);
    }
  }

  async function submitAddRewardRule() {
    if (!currentGuildId) return;
    const roleId = getValue("reward-modal-role");
    const ruleType = getValue("reward-modal-type");
    const timeframe = getValue("reward-modal-timeframe");
    const targetVal = parseInt(getValue("reward-modal-target"), 10) || 1;
    const removeOnDrop = getChecked("reward-modal-drop");

    if (!roleId) {
      showToast("Please select a role to award.");
      return;
    }

    try {
      const payload = {
        guild_id: currentGuildId,
        role_id: roleId,
        rule_type: ruleType,
        timeframe: timeframe,
        threshold_points: ruleType === "threshold" ? targetVal : 0,
        top_rank: ruleType === "top_rank" ? targetVal : 0,
        remove_on_drop: removeOnDrop,
      };

      await apiFetch(`/api/guilds/${currentGuildId}/activity/rewards`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });

      closeModal("modal-add-reward-rule");
      showToast("Created reward rule successfully.");
      await loadActivityRewards();
    } catch (err) {
      showToast(`Failed to create reward rule: ${err.message}`);
    }
  }

  async function loadActivityLeaderboard() {
    if (!currentGuildId) return;
    const tbody = document.getElementById("leaderboard-tbody");
    if (!tbody) return;
    tbody.replaceChildren();

    const tf = getValue("leaderboard-timeframe-select") || "lifetime";

    try {
      const res = await apiFetch(`/api/guilds/${currentGuildId}/activity/leaderboard?timeframe=${encodeURIComponent(tf)}&limit=20`);
      const entries = res.entries || [];

      if (entries.length === 0) {
        const tr = document.createElement("tr");
        const td = document.createElement("td");
        td.colSpan = 6;
        td.className = "text-center muted";
        td.textContent = "No participants recorded for this timeframe yet.";
        tr.appendChild(td);
        tbody.appendChild(tr);
        return;
      }

      entries.forEach((e) => {
        if (e.user_id) {
          memberCache.set(String(e.user_id), {
            id: e.user_id,
            username: e.username || e.user_id,
            display_name: e.display_name || e.username || e.user_id,
            avatar_url: e.avatar_url,
          });
        }

        const tr = document.createElement("tr");

        const tdRank = document.createElement("td");
        const medal = e.rank === 1 ? "🥇 " : e.rank === 2 ? "🥈 " : e.rank === 3 ? "🥉 " : "";
        tdRank.textContent = `${medal}#${e.rank}`;
        tr.appendChild(tdRank);

        const tdUser = document.createElement("td");
        tdUser.style.cursor = "pointer";
        tdUser.title = `Click to select for points adjustment (ID: ${e.user_id})`;
        tdUser.addEventListener("click", () => {
          const uidIn = document.getElementById("adjust-user-id");
          if (uidIn) {
            uidIn.value = e.user_id;
            uidIn.focus();
            showToast(`Selected ${e.display_name || e.username || e.user_id} for point adjustment.`);
          }
        });

        const userRow = document.createElement("div");
        userRow.style.display = "inline-flex";
        userRow.style.alignItems = "center";
        userRow.style.gap = "0.6rem";

        if (e.avatar_url) {
          const img = document.createElement("img");
          img.src = e.avatar_url;
          img.alt = "";
          img.style.width = "26px";
          img.style.height = "26px";
          img.style.borderRadius = "50%";
          img.style.flexShrink = "0";
          userRow.appendChild(img);
        }

        const nameWrapper = document.createElement("div");
        nameWrapper.style.display = "inline-flex";
        nameWrapper.style.flexDirection = "column";

        const primaryName = document.createElement("strong");
        const displayName = e.display_name || e.username || resolveMemberLabel(e.user_id);
        primaryName.textContent = displayName;
        nameWrapper.appendChild(primaryName);

        if (e.username && e.display_name && e.display_name !== e.username) {
          const subName = document.createElement("span");
          subName.className = "muted";
          subName.style.fontSize = "0.75rem";
          subName.textContent = `@${e.username}`;
          nameWrapper.appendChild(subName);
        }

        userRow.appendChild(nameWrapper);
        tdUser.appendChild(userRow);
        tr.appendChild(tdUser);

        const tdPts = document.createElement("td");
        const bold = document.createElement("strong");
        bold.textContent = String(e.points);
        tdPts.appendChild(bold);
        tr.appendChild(tdPts);

        const tdText = document.createElement("td");
        tdText.textContent = String(e.text_points);
        tr.appendChild(tdText);

        const tdVoice = document.createElement("td");
        tdVoice.textContent = String(e.voice_points);
        tr.appendChild(tdVoice);

        const tdReact = document.createElement("td");
        tdReact.textContent = String(e.reaction_points);
        tr.appendChild(tdReact);

        tbody.appendChild(tr);
      });
    } catch (err) {
      console.warn("Failed to load leaderboard:", err);
    }
  }

  async function submitPointAdjustment() {
    if (!currentGuildId) return;
    const uidIn = document.getElementById("adjust-user-id");
    const amtIn = document.getElementById("adjust-amount");
    if (!uidIn || !amtIn || !uidIn.value.trim() || !amtIn.value.trim()) {
      showToast("Please enter a user ID and point amount.");
      return;
    }

    const userId = uidIn.value.trim();
    const amount = parseInt(amtIn.value.trim(), 10);
    if (!amount) {
      showToast("Adjustment amount cannot be zero.");
      return;
    }

    try {
      const res = await apiFetch(`/api/guilds/${currentGuildId}/activity/points/adjust`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ user_id: userId, amount: amount }),
      });

      showToast(`Points adjusted successfully. New total: ${res.new_total} pts`);
      amtIn.value = "";
      await loadActivityLeaderboard();
    } catch (err) {
      showToast(`Failed to adjust points: ${err.message}`);
    }
  }

  function initAllComboboxes() {
    // Server Configuration > Permissions & Channels
    initChannelCombobox("gcfg-allowed-channels", { multiSelect: true, placeholder: "Search channels..." });
    initRoleCombobox("gcfg-prompt-roles", { multiSelect: true, placeholder: "Search prompt roles..." });
    initRoleCombobox("gcfg-model-roles", { multiSelect: true, placeholder: "Search model switch roles..." });
    initRoleCombobox("gcfg-admin-roles", { multiSelect: true, placeholder: "Search admin roles..." });

    // Server Configuration > Forum Auto-Reply
    initChannelCombobox("gcfg-forum-channels", { multiSelect: true, filterKinds: ["forum"], placeholder: "Search forum channels..." });

    // Moderation & Cases
    initUserCombobox("mod-filter-user", { placeholder: "Search member to filter..." });
    initUserCombobox("mcase-user-id", { placeholder: "Search member by username or display name..." });
    bindDurationPreview("mcase-duration", "mcase-duration-preview");

    // Automod & Strikes
    initChannelCombobox("am-ignored-channels", { multiSelect: true, placeholder: "Search channels to ignore..." });
    initRoleCombobox("am-ignored-roles", { multiSelect: true, placeholder: "Search roles to ignore..." });

    // Channel Logging
    initChannelCombobox("log-unified-channel", { multiSelect: false, filterKinds: ["text", "announcement"], placeholder: "Select unified log channel..." });
    initChannelCombobox("log-mod-channel", { multiSelect: false, filterKinds: ["text", "announcement"], placeholder: "Select mod log channel..." });
    initChannelCombobox("log-message-channel", { multiSelect: false, filterKinds: ["text", "announcement"], placeholder: "Select message log channel..." });
    initChannelCombobox("log-member-channel", { multiSelect: false, filterKinds: ["text", "announcement"], placeholder: "Select member lifecycle channel..." });
    initChannelCombobox("log-server-channel", { multiSelect: false, filterKinds: ["text", "announcement"], placeholder: "Select server structure channel..." });
    initChannelCombobox("log-voice-channel", { multiSelect: false, filterKinds: ["text", "voice"], placeholder: "Select voice activity channel..." });
    initChannelCombobox("log-ignored-channels", { multiSelect: true, placeholder: "Search channels to ignore..." });
    initRoleCombobox("log-ignored-roles", { multiSelect: true, placeholder: "Search roles to ignore..." });

    // Greetings & Autorole
    initChannelCombobox("greet-welcome-channel", { multiSelect: false, filterKinds: ["text", "announcement"], placeholder: "Select welcome channel..." });
    initChannelCombobox("greet-farewell-channel", { multiSelect: false, filterKinds: ["text", "announcement"], placeholder: "Select farewell channel..." });
    initRoleCombobox("greet-member-roles", { multiSelect: true, placeholder: "Search autoroles for members..." });
    initRoleCombobox("greet-bot-roles", { multiSelect: true, placeholder: "Search autoroles for bots..." });

    // Reaction Roles Modal
    initChannelCombobox("cb-rr-channel", {
      multiSelect: false,
      filterKinds: ["text", "announcement"],
      placeholder: "Select target text channel...",
      onChange: (id) => setValue("rr-input-channel", id || ""),
    });
    initRoleCombobox("rr-input-reqrole", { multiSelect: false, placeholder: "Select prerequisite role (optional)..." });

    // Custom Triggers Modal
    initRoleCombobox("trig-roles", { multiSelect: true, placeholder: "Search allowed roles (empty = all)..." });
    initChannelCombobox("trig-channels", { multiSelect: true, placeholder: "Search ignored channels..." });

    // Reddit Feeds Modal
    initChannelCombobox("cb-reddit-channel", {
      multiSelect: false,
      filterKinds: ["text", "announcement"],
      placeholder: "Select target text channel...",
      onChange: (id) => setValue("reddit-new-channel", id || ""),
    });
  }

  document.addEventListener('DOMContentLoaded', () => {
    initTheme();
    initLoginPage();

    if (!document.getElementById('view-overview')) {
      return;
    }

    initNavigation();
    initModals();
    initAllComboboxes();
    wireAllEventListeners();

    checkSessionAndLoadData();
  });
})();
