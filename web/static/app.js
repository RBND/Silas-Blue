// Silas Blue Web Dashboard Client Logic

let activePage = 'dashboard';
let ws = null;
let reconnectTimer = null;
let currentGuilds = [];

// --- Toast Notifications ---
function showToast(message, type = 'success', duration = 3500) {
    const container = document.getElementById('toast-container');
    if (!container) return;

    const toast = document.createElement('div');
    toast.className = `toast ${type}`;

    let icon = '✓';
    if (type === 'error') icon = '✕';
    else if (type === 'warning') icon = '⚠';
    else if (type === 'info') icon = 'ℹ';

    toast.innerHTML = `
        <span class="toast-icon">${icon}</span>
        <span class="toast-message">${message}</span>
    `;

    toast.onclick = () => dismissToast(toast);

    container.appendChild(toast);

    requestAnimationFrame(() => {
        toast.classList.add('show');
    });

    setTimeout(() => {
        dismissToast(toast);
    }, duration);
}

function dismissToast(toast) {
    if (!toast || toast.classList.contains('hide')) return;
    toast.classList.remove('show');
    toast.classList.add('hide');
    setTimeout(() => {
        if (toast.parentNode) {
            toast.parentNode.removeChild(toast);
        }
    }, 300);
}

// --- Navigation ---
function switchPage(pageId) {
    activePage = pageId;
    document.querySelectorAll('.page').forEach(el => el.classList.remove('active'));
    document.querySelectorAll('.nav-item').forEach(el => el.classList.remove('active'));

    const pageEl = document.getElementById(`page-${pageId}`);
    if (pageEl) pageEl.classList.add('active');

    const navBtn = Array.from(document.querySelectorAll('.nav-item')).find(btn => 
        btn.getAttribute('onclick')?.includes(pageId)
    );
    if (navBtn) navBtn.classList.add('active');

    if (pageId === 'ai') loadAiSettings();
    if (pageId === 'servers') loadGuildsList();
    if (pageId === 'updates') loadUpdateStatus();
}

// --- WebSocket Real-Time Log Streaming ---
function connectWebSocket() {
    if (ws) {
        try { ws.close(); } catch(e) {}
    }

    const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const wsUrl = `${proto}//${window.location.host}/api/ws/logs`;

    ws = new WebSocket(wsUrl);

    ws.onopen = () => {
        console.log('[WS] Connected to Silas Blue event stream');
    };

    ws.onmessage = (event) => {
        try {
            const data = JSON.parse(event.data);
            appendLog(data);
        } catch (e) {
            console.error('Failed to parse WS log:', e);
        }
    };

    ws.onclose = () => {
        console.log('[WS] Disconnected. Reconnecting in 3s...');
        clearTimeout(reconnectTimer);
        reconnectTimer = setTimeout(connectWebSocket, 3000);
    };

    ws.onerror = (err) => {
        console.error('[WS Error]', err);
    };
}

function appendLog(logData) {
    const fullTerminal = document.getElementById('full-terminal-logs');
    const dashRecent = document.getElementById('dash-recent-logs');
    const autoscroll = document.getElementById('autoscroll-toggle')?.checked;

    const entry = document.createElement('div');
    entry.className = `log-entry ${logData.type || 'system'}`;
    entry.textContent = logData.formatted || logData.message || JSON.stringify(logData);

    if (fullTerminal) {
        fullTerminal.appendChild(entry.cloneNode(true));
        // Keep max 500 lines in full terminal
        if (fullTerminal.children.length > 500) {
            fullTerminal.removeChild(fullTerminal.firstChild);
        }
        if (autoscroll) {
            fullTerminal.scrollTop = fullTerminal.scrollHeight;
        }
    }

    if (dashRecent) {
        dashRecent.appendChild(entry);
        if (dashRecent.children.length > 100) {
            dashRecent.removeChild(dashRecent.firstChild);
        }
        dashRecent.scrollTop = dashRecent.scrollHeight;
    }
}

function clearConsoleLogs() {
    const fullTerminal = document.getElementById('full-terminal-logs');
    if (fullTerminal) fullTerminal.innerHTML = '';
}

function filterLogs() {
    const filter = document.getElementById('log-filter-input').value.toLowerCase();
    const entries = document.querySelectorAll('#full-terminal-logs .log-entry');
    entries.forEach(entry => {
        const text = entry.textContent.toLowerCase();
        entry.style.display = text.includes(filter) ? '' : 'none';
    });
}

// --- Status & KPI Fetching ---
async function refreshStatus() {
    try {
        const res = await fetch('/api/status');
        if (res.status === 401) {
            window.location.href = '/login';
            return;
        }
        const data = await res.json();
        updateDashboardUI(data);
    } catch (e) {
        console.error('Status fetch failed:', e);
    }
}

function updateDashboardUI(data) {
    // User header
    const userDisplay = document.getElementById('user-display');
    if (userDisplay && data.user) {
        userDisplay.textContent = data.user.username || 'Admin';
    }

    // Bot Daemon KPI
    const botPill = document.getElementById('kpi-bot-pill');
    const sidePill = document.getElementById('sidebar-bot-pill');
    const botSub = document.getElementById('kpi-bot-sub');

    const isRunning = data.bot.running;
    const pillClass = isRunning ? 'status-pill online' : 'status-pill offline';
    const pillText = isRunning ? 'ONLINE' : 'STOPPED';

    if (botPill) {
        botPill.className = pillClass;
        botPill.textContent = pillText;
    }
    if (sidePill) {
        sidePill.className = pillClass;
        sidePill.textContent = pillText;
    }
    if (botSub) {
        botSub.textContent = isRunning ? (data.bot.user || 'Connected to Discord') : 'Discord Gateway Offline';
    }

    // AI Backend KPI
    const aiPill = document.getElementById('kpi-ai-pill');
    const aiName = document.getElementById('kpi-ai-name');
    const aiUrl = document.getElementById('kpi-ai-url');

    if (aiPill) {
        aiPill.className = data.ai.online ? 'status-pill online' : 'status-pill offline';
        aiPill.textContent = data.ai.online ? 'ONLINE' : 'OFFLINE';
    }
    if (aiName) aiName.textContent = data.ai.name;
    if (aiUrl) aiUrl.textContent = data.ai.base_url || (data.ai.is_local ? 'Local Service' : 'Remote API');

    // Active Model KPI
    const activeModel = document.getElementById('kpi-active-model');
    if (activeModel) activeModel.textContent = data.ai.active_model || 'None';

    // Servers KPI
    const serversCount = document.getElementById('kpi-servers-count');
    if (serversCount) {
        const count = data.bot.servers_count || 0;
        serversCount.textContent = `${count} ${count === 1 ? 'Guild' : 'Guilds'}`;
    }

    currentGuilds = data.guilds || [];
}

// --- Bot Daemon Actions ---
async function botAction(action) {
    const feedback = document.getElementById('bot-action-feedback');
    if (feedback) feedback.textContent = `Executing ${action}...`;

    try {
        const res = await fetch('/api/bot/action', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ action })
        });
        const result = await res.json();
        if (feedback) feedback.textContent = result.message || 'Action executed';
        setTimeout(refreshStatus, 1500);
    } catch (e) {
        if (feedback) feedback.textContent = `Error: ${e.message}`;
    }
}

// --- Model Management ---
async function loadAvailableModels() {
    try {
        const res = await fetch('/api/models');
        const data = await res.json();
        const select = document.getElementById('quick-model-select');
        if (!select) return;

        select.innerHTML = '';
        (data.models || []).forEach(m => {
            const opt = document.createElement('option');
            opt.value = m;
            opt.textContent = m;
            if (m === data.active) opt.selected = true;
            select.appendChild(opt);
        });
    } catch (e) {
        console.error('Failed to load models:', e);
    }
}

async function changeActiveModel() {
    const select = document.getElementById('quick-model-select');
    const model = select?.value;
    if (!model) return;

    try {
        const res = await fetch('/api/models/active', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ model })
        });
        const result = await res.json();
        if (result.success) {
            refreshStatus();
            showToast(`Active model switched to: ${model}`, 'success');
        }
    } catch (e) {
        showToast('Failed to change model', 'error');
    }
}

async function pullModel() {
    const input = document.getElementById('quick-pull-input');
    const model = input?.value.trim();
    if (!model) return;

    try {
        const res = await fetch('/api/models/pull', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ model })
        });
        const result = await res.json();
        showToast(result.message || 'Pull initiated. Check live logs for progress.', 'info');
        input.value = '';
    } catch (e) {
        showToast('Failed to initiate pull', 'error');
    }
}

// --- AI Configuration ---
function onBackendTypeChange() {
    const type = document.getElementById('ai-provider-type').value;
    document.getElementById('fields-ollama').style.display = (type === 'ollama') ? 'block' : 'none';
    document.getElementById('fields-llamacpp').style.display = (type === 'llamacpp') ? 'block' : 'none';
    document.getElementById('fields-remote').style.display = (type === 'remote_openai') ? 'block' : 'none';
}

async function loadAiSettings() {
    try {
        const res = await fetch('/api/ai/config');
        const data = await res.json();
        const cfg = data.config || {};

        document.getElementById('ai-provider-type').value = cfg.backend_type || 'ollama';
        onBackendTypeChange();

        document.getElementById('cfg-ollama-url').value = cfg.ollama_url || '';
        document.getElementById('cfg-ollama-key').value = cfg.api_key || '';

        document.getElementById('cfg-llamacpp-url').value = cfg.llamacpp_url || '';
        document.getElementById('cfg-llamacpp-bin').value = cfg.llamacpp_binary_path || '';
        document.getElementById('cfg-llamacpp-model').value = cfg.llamacpp_model_path || '';

        const presetSel = document.getElementById('cfg-remote-preset');
        presetSel.innerHTML = '';
        const presetDetails = data.preset_details || {};
        (data.presets || []).forEach(p => {
            const opt = document.createElement('option');
            opt.value = p;
            opt.textContent = p;
            if (p === cfg.remote_preset) opt.selected = true;
            presetSel.appendChild(opt);
        });

        function updateRemoteModelDatalist(selectedPreset) {
            const datalist = document.getElementById('remote-models-datalist');
            if (!datalist) return;
            datalist.innerHTML = '';
            const models = presetDetails[selectedPreset]?.models || [];
            models.forEach(m => {
                const opt = document.createElement('option');
                opt.value = m;
                datalist.appendChild(opt);
            });
        }

        presetSel.onchange = () => {
            const p = presetSel.value;
            const details = presetDetails[p];
            if (details) {
                if (details.base_url) document.getElementById('cfg-remote-url').value = details.base_url;
                if (details.default_model) document.getElementById('cfg-remote-model').value = details.default_model;
            }
            updateRemoteModelDatalist(p);
        };

        document.getElementById('cfg-remote-url').value = cfg.remote_base_url || '';
        document.getElementById('cfg-remote-key').value = cfg.remote_api_key || '';
        document.getElementById('cfg-remote-model').value = cfg.remote_model || '';
        updateRemoteModelDatalist(cfg.remote_preset || presetSel.value);

        document.getElementById('cfg-system-prompt').value = cfg.system_prompt || '';
        document.getElementById('cfg-allow-dms').checked = cfg.allow_dms !== false;
        document.getElementById('cfg-dm-model').value = cfg.dm_default_model || '';
        document.getElementById('cfg-dm-memory-turns').value = cfg.dm_memory_turns !== undefined ? cfg.dm_memory_turns : 6;
        document.getElementById('cfg-dm-memory-ttl').value = cfg.dm_memory_ttl_minutes !== undefined ? cfg.dm_memory_ttl_minutes : 30;
    } catch (e) {
        console.error('Failed to load AI settings:', e);
    }
}

async function saveAiSettings() {
    const payload = {
        backend_type: document.getElementById('ai-provider-type').value,
        ollama_url: document.getElementById('cfg-ollama-url').value.trim(),
        api_key: document.getElementById('cfg-ollama-key').value.trim(),
        llamacpp_url: document.getElementById('cfg-llamacpp-url').value.trim(),
        llamacpp_binary_path: document.getElementById('cfg-llamacpp-bin').value.trim(),
        llamacpp_model_path: document.getElementById('cfg-llamacpp-model').value.trim(),
        remote_preset: document.getElementById('cfg-remote-preset').value,
        remote_base_url: document.getElementById('cfg-remote-url').value.trim(),
        remote_api_key: document.getElementById('cfg-remote-key').value.trim(),
        remote_model: document.getElementById('cfg-remote-model').value.trim(),
        system_prompt: document.getElementById('cfg-system-prompt').value.trim(),
        allow_dms: document.getElementById('cfg-allow-dms').checked,
        dm_default_model: document.getElementById('cfg-dm-model').value.trim(),
        dm_memory_turns: parseInt(document.getElementById('cfg-dm-memory-turns').value) || 0,
        dm_memory_ttl_minutes: parseInt(document.getElementById('cfg-dm-memory-ttl').value) || 30
    };

    try {
        const res = await fetch('/api/ai/config', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload)
        });
        const result = await res.json();
        showToast(result.message || 'Settings saved!', 'success');
        refreshStatus();
    } catch (e) {
        showToast('Failed to save AI settings', 'error');
    }
}

// --- Server Configuration & Permission Management ---
const SNOWFLAKE_REGEX = /^[1-9]\d{16,19}$/;
const USER_MENTION_REGEX = /^<@!?([1-9]\d{16,19})>$/;
const ROLE_MENTION_REGEX = /^<@&([1-9]\d{16,19})>$/;

let permData = {
    reply: { roles: ['everyone'], users: [] },
    model: { roles: [], users: [] },
    perm: { roles: [], users: [] },
    search: { roles: ['everyone'], users: [] }
};
let serverAvailableRoles = [];

async function loadGuildsList() {
    const select = document.getElementById('guild-select');
    select.innerHTML = '';
    currentGuilds.forEach(g => {
        const opt = document.createElement('option');
        opt.value = g.id;
        opt.textContent = `${g.name} (${g.id})`;
        select.appendChild(opt);
    });
    if (currentGuilds.length > 0) {
        loadSelectedGuildConfig();
    }
}

async function loadSelectedGuildConfig() {
    const guildId = document.getElementById('guild-select').value;
    if (!guildId) return;

    try {
        const res = await fetch(`/api/guild/${guildId}`);
        const data = await res.json();
        const cfg = data.config || {};
        serverAvailableRoles = data.available_roles || [];

        // Update datalist for role autocompletion
        const datalist = document.getElementById('available-roles-datalist');
        if (datalist) {
            datalist.innerHTML = '';
            serverAvailableRoles.forEach(r => {
                if (r.name && r.name !== '@everyone') {
                    const opt = document.createElement('option');
                    opt.value = r.name;
                    datalist.appendChild(opt);
                }
            });
        }

        permData = {
            reply: {
                roles: Array.isArray(cfg.reply_roles) ? [...cfg.reply_roles] : ['everyone'],
                users: Array.isArray(cfg.reply_users) ? [...cfg.reply_users] : []
            },
            model: {
                roles: Array.isArray(cfg.change_model_roles) ? [...cfg.change_model_roles] : [],
                users: Array.isArray(cfg.change_model_users) ? [...cfg.change_model_users] : []
            },
            perm: {
                roles: Array.isArray(cfg.change_permission_roles) ? [...cfg.change_permission_roles] : [],
                users: Array.isArray(cfg.change_permission_users) ? [...cfg.change_permission_users] : []
            },
            search: {
                roles: Array.isArray(cfg.search_roles) ? [...cfg.search_roles] : ['everyone'],
                users: Array.isArray(cfg.search_users) ? [...cfg.search_users] : []
            }
        };

        ['reply', 'model', 'perm', 'search'].forEach(scope => renderPermScope(scope));

        document.getElementById('cfg-pagination').checked = cfg.pagination_enabled !== false;
        document.getElementById('cfg-max-chars').value = cfg.pagination_max_chars || 1000;
        document.getElementById('cfg-server-system-prompt').value = cfg.system_prompt || '';
        document.getElementById('cfg-guild-memory-turns').value = cfg.memory_turns !== undefined ? cfg.memory_turns : 6;
        document.getElementById('cfg-guild-memory-ttl').value = cfg.memory_ttl_minutes !== undefined ? cfg.memory_ttl_minutes : 30;
    } catch (e) {
        console.error('Failed to load guild config:', e);
    }
}

function renderPermScope(scope) {
    const scopeData = permData[scope];
    if (!scopeData) return;

    const isEveryone = scopeData.roles.some(r => typeof r === 'string' && ['everyone', '@everyone'].includes(r.toLowerCase()));

    // Set radio buttons
    const rbEveryone = document.querySelector(`input[name="perm-mode-${scope}"][value="everyone"]`);
    const rbRestricted = document.querySelector(`input[name="perm-mode-${scope}"][value="restricted"]`);
    if (rbEveryone && rbRestricted) {
        rbEveryone.checked = isEveryone;
        rbRestricted.checked = !isEveryone;
    }

    const chipsContainer = document.getElementById(`chips-${scope}`);
    const emptyState = document.getElementById(`empty-${scope}`);
    const inputRow = document.getElementById(`input-row-${scope}`);

    if (isEveryone) {
        if (chipsContainer) chipsContainer.style.display = 'none';
        if (emptyState) emptyState.style.display = 'none';
        if (inputRow) inputRow.style.display = 'none';
        return;
    }

    // Restricted mode: show chips and input
    if (chipsContainer) chipsContainer.style.display = 'flex';
    if (inputRow) inputRow.style.display = 'flex';

    if (chipsContainer) {
        chipsContainer.innerHTML = '';
        const cleanRoles = scopeData.roles.filter(r => typeof r === 'string' && !['everyone', '@everyone'].includes(r.toLowerCase()));

        cleanRoles.forEach(r => {
            const chip = document.createElement('div');
            chip.className = 'perm-chip role';
            const span = document.createElement('span');
            span.textContent = `🛡️ ${r}`;
            const btn = document.createElement('button');
            btn.className = 'del-btn';
            btn.textContent = '✕';
            btn.title = 'Remove role grant';
            btn.onclick = () => removePermGrant(scope, 'role', r);
            chip.appendChild(span);
            chip.appendChild(btn);
            chipsContainer.appendChild(chip);
        });

        scopeData.users.forEach(u => {
            const chip = document.createElement('div');
            chip.className = 'perm-chip user';
            const span = document.createElement('span');
            span.textContent = `👤 User (${String(u).slice(-4)})`;
            span.title = `Discord User ID: ${u}`;
            const btn = document.createElement('button');
            btn.className = 'del-btn';
            btn.textContent = '✕';
            btn.title = 'Remove user grant';
            btn.onclick = () => removePermGrant(scope, 'user', u);
            chip.appendChild(span);
            chip.appendChild(btn);
            chipsContainer.appendChild(chip);
        });

        if (emptyState) {
            emptyState.style.display = (cleanRoles.length === 0 && scopeData.users.length === 0) ? 'block' : 'none';
        }
    }
}

function togglePermMode(scope, mode) {
    if (!permData[scope]) return;
    if (mode === 'everyone') {
        permData[scope].roles = ['everyone'];
        permData[scope].users = [];
    } else {
        permData[scope].roles = permData[scope].roles.filter(r => typeof r === 'string' && !['everyone', '@everyone'].includes(r.toLowerCase()));
    }
    renderPermScope(scope);
}

function addPermGrant(scope) {
    const input = document.getElementById(`input-${scope}`);
    const status = document.getElementById(`status-${scope}`);
    if (!input || !permData[scope]) return;

    const val = input.value.trim();
    if (!val) return;

    if (status) {
        status.style.display = 'none';
        status.textContent = '';
    }

    // 1. User Mention <@123456789...> or <@!123456789...>
    const userMatch = val.match(USER_MENTION_REGEX);
    if (userMatch) {
        const uid = userMatch[1];
        if (!permData[scope].users.includes(uid)) {
            permData[scope].users.push(uid);
        }
        input.value = '';
        renderPermScope(scope);
        return;
    }

    // 2. Role Mention <@&123456789...>
    const roleMatch = val.match(ROLE_MENTION_REGEX);
    if (roleMatch) {
        const rid = roleMatch[1];
        const found = serverAvailableRoles.find(r => String(r.id) === rid);
        const roleName = found ? found.name : rid;
        if (!permData[scope].roles.some(r => r.toLowerCase() === roleName.toLowerCase())) {
            permData[scope].roles.push(roleName);
        }
        input.value = '';
        renderPermScope(scope);
        return;
    }

    // 3. Raw Snowflake (17-20 digits)
    if (SNOWFLAKE_REGEX.test(val)) {
        // If matches available server role ID, add role name; otherwise add user snowflake
        const found = serverAvailableRoles.find(r => String(r.id) === val);
        if (found) {
            if (!permData[scope].roles.some(r => r.toLowerCase() === found.name.toLowerCase())) {
                permData[scope].roles.push(found.name);
            }
        } else {
            if (!permData[scope].users.includes(val)) {
                permData[scope].users.push(val);
            }
        }
        input.value = '';
        renderPermScope(scope);
        return;
    }

    // 4. Role Name
    const matchedRole = serverAvailableRoles.find(r => r.name.toLowerCase() === val.toLowerCase());
    const roleToAdd = matchedRole ? matchedRole.name : val;
    if (permData[scope].roles.some(r => r.toLowerCase() === roleToAdd.toLowerCase())) {
        if (status) {
            status.textContent = `Role '${roleToAdd}' already granted.`;
            status.style.color = 'var(--accent-cyan)';
            status.style.display = 'block';
        }
        return;
    }

    permData[scope].roles.push(roleToAdd);
    input.value = '';
    renderPermScope(scope);
}

function removePermGrant(scope, type, val) {
    if (!permData[scope]) return;
    if (type === 'user') {
        permData[scope].users = permData[scope].users.filter(u => u !== val);
    } else {
        permData[scope].roles = permData[scope].roles.filter(r => r !== val);
    }
    renderPermScope(scope);
}

function getScopeDataForSave(scope) {
    const isEveryone = permData[scope].roles.some(r => typeof r === 'string' && ['everyone', '@everyone'].includes(r.toLowerCase()));
    if (isEveryone) {
        return { roles: ['everyone'], users: [] };
    }
    return {
        roles: permData[scope].roles.filter(r => typeof r === 'string' && !['everyone', '@everyone'].includes(r.toLowerCase())),
        users: [...permData[scope].users]
    };
}

async function saveServerConfig() {
    const guildId = document.getElementById('guild-select').value;
    if (!guildId) return;

    const reply = getScopeDataForSave('reply');
    const model = getScopeDataForSave('model');
    const perm = getScopeDataForSave('perm');
    const search = getScopeDataForSave('search');

    const payload = {
        reply_roles: reply.roles,
        reply_users: reply.users,
        change_model_roles: model.roles,
        change_model_users: model.users,
        change_permission_roles: perm.roles,
        change_permission_users: perm.users,
        search_roles: search.roles,
        search_users: search.users,
        pagination_enabled: document.getElementById('cfg-pagination').checked,
        pagination_max_chars: parseInt(document.getElementById('cfg-max-chars').value) || 1000,
        system_prompt: document.getElementById('cfg-server-system-prompt').value.trim(),
        memory_turns: parseInt(document.getElementById('cfg-guild-memory-turns').value) || 0,
        memory_ttl_minutes: parseInt(document.getElementById('cfg-guild-memory-ttl').value) || 30
    };

    try {
        const res = await fetch(`/api/guild/${guildId}`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload)
        });
        const result = await res.json();
        showToast(result.message || 'Server configuration saved!', 'success');
    } catch (e) {
        showToast('Failed to save server config', 'error');
    }
}

// --- Updater Management ---
async function loadUpdateStatus() {
    try {
        const res = await fetch('/api/update/status');
        if (!res.ok) return;
        const data = await res.json();

        // Update Dashboard Summary
        const dashSummary = document.getElementById('dash-update-summary');
        const dashBtnApply = document.getElementById('btn-dash-update-apply');
        const currVer = data.current_version || '3.3.4';
        const currSha = (data.current_commit || '').substring(0, 7);

        if (dashSummary) {
            dashSummary.textContent = `Silas Blue v${currVer} (#${currSha}) • Channel: ${(data.config?.update_channel || 'stable').toUpperCase()}`;
        }

        // Update Updates Page Form & KPIs
        const updateCurrVer = document.getElementById('update-curr-version');
        const updateCurrCommit = document.getElementById('update-curr-commit');
        const updateCurrChan = document.getElementById('update-curr-channel');
        
        if (updateCurrVer) updateCurrVer.textContent = currVer;
        if (updateCurrCommit) updateCurrCommit.textContent = `#${currSha}`;
        if (updateCurrChan) updateCurrChan.textContent = (data.config?.update_channel || 'stable').toUpperCase();

        const cfg = data.config || {};
        const selChan = document.getElementById('cfg-update-channel');
        const chkAuto = document.getElementById('cfg-auto-update-enabled');
        const selMode = document.getElementById('cfg-auto-update-mode');
        const numInterval = document.getElementById('cfg-auto-update-interval');
        const txtNotifyChan = document.getElementById('cfg-update-notify-channel');
        const chkNotifyOwner = document.getElementById('cfg-update-notify-owner');

        if (selChan) selChan.value = cfg.update_channel || 'stable';
        if (chkAuto) chkAuto.checked = !!cfg.auto_update_enabled;
        if (selMode) selMode.value = cfg.auto_update_mode || 'notify';
        if (numInterval) numInterval.value = cfg.auto_update_interval_hours || 24;
        if (txtNotifyChan) txtNotifyChan.value = cfg.update_notify_channel_id || '';
        if (chkNotifyOwner) chkNotifyOwner.checked = cfg.update_notify_owner_dm !== false;

        // Render cached status if available
        if (data.cached_status) {
            renderUpdateResult(data.cached_status);
        }
    } catch (e) {
        console.error('Failed to load update status:', e);
    }
}

function renderUpdateResult(res) {
    const statusPill = document.getElementById('update-status-pill');
    const statusSub = document.getElementById('update-status-sub');
    const latestVer = document.getElementById('update-latest-version');
    const latestCommit = document.getElementById('update-latest-commit');
    const btnDashApply = document.getElementById('btn-dash-update-apply');
    const btnFullApply = document.getElementById('btn-full-update-apply');
    const changelogContainer = document.getElementById('update-changelog-container');
    const changelogList = document.getElementById('update-changelog-list');
    const actionPrompt = document.getElementById('update-action-prompt');

    if (latestVer) latestVer.textContent = res.latest_version || res.current_version;
    if (latestCommit) latestCommit.textContent = `#${(res.latest_commit || '').substring(0, 7)}`;

    if (res.update_available) {
        if (statusPill) {
            statusPill.className = 'status-pill online';
            statusPill.textContent = 'UPDATE READY';
        }
        if (statusSub) statusSub.textContent = `Newer version available`;
        if (actionPrompt) actionPrompt.innerHTML = `<b style="color:var(--accent-cyan);">A newer version (${res.latest_version}) is ready to install!</b>`;
        if (btnDashApply) btnDashApply.style.display = 'inline-block';
        if (btnFullApply) btnFullApply.style.display = 'inline-block';

        if (res.changelog && res.changelog.length > 0) {
            if (changelogContainer) changelogContainer.style.display = 'block';
            if (changelogList) {
                changelogList.innerHTML = '';
                res.changelog.forEach(item => {
                    const li = document.createElement('li');
                    li.textContent = item;
                    changelogList.appendChild(li);
                });
            }
        }
    } else {
        if (statusPill) {
            statusPill.className = 'status-pill online';
            statusPill.textContent = 'UP TO DATE';
        }
        if (statusSub) statusSub.textContent = 'No updates pending';
        if (actionPrompt) actionPrompt.textContent = 'Silas Blue is running the latest version.';
        if (btnDashApply) btnDashApply.style.display = 'none';
        if (btnFullApply) btnFullApply.style.display = 'none';
        if (changelogContainer) changelogContainer.style.display = 'none';
    }
}

async function checkUpdates(userTriggered = false) {
    const chan = document.getElementById('cfg-update-channel')?.value || 'stable';
    const actionPrompt = document.getElementById('update-action-prompt');
    if (actionPrompt) actionPrompt.textContent = 'Contacting GitHub and checking repository for updates...';

    try {
        const res = await fetch('/api/update/check', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ channel: chan })
        });
        const result = await res.json();
        renderUpdateResult(result);
        if (userTriggered) {
            if (result.update_available) {
                showToast(`Update Available! Latest: ${result.latest_version || result.latest_commit}`, 'info');
            } else {
                showToast('Silas Blue is already up to date!', 'success');
            }
        }
    } catch (e) {
        console.error('Update check failed:', e);
        if (actionPrompt) actionPrompt.textContent = 'Failed to check for updates. Check server logs.';
        showToast('Failed to check for updates', 'error');
    }
}

function setUpdateStep(stepId, state, labelText) {
    const el = document.getElementById(stepId);
    if (!el) return;

    el.classList.remove('pending', 'active', 'done', 'error');
    el.classList.add(state);

    const iconEl = el.querySelector('.step-icon');
    const labelEl = el.querySelector('.step-label');

    if (iconEl) {
        if (state === 'active') iconEl.textContent = '⏳';
        else if (state === 'done') iconEl.textContent = '✓';
        else if (state === 'error') iconEl.textContent = '✕';
        else iconEl.textContent = '○';
    }

    if (labelText && labelEl) {
        labelEl.textContent = labelText;
    }
}

async function applyUpdate() {
    const chan = document.getElementById('cfg-update-channel')?.value || 'stable';
    if (!confirm(`Are you sure you want to install the update from channel '${chan}' and restart Silas Blue?`)) {
        return;
    }

    const modal = document.getElementById('update-progress-modal');
    const progressFill = document.getElementById('update-progress-fill');
    const statusSub = document.getElementById('update-modal-status-sub');
    const modalTitle = document.getElementById('update-modal-title');
    const modalSpinner = document.getElementById('update-modal-spinner');

    if (modal) modal.style.display = 'flex';
    if (modalTitle) modalTitle.textContent = 'Installing Update';
    if (modalSpinner) modalSpinner.textContent = '🚀';

    // Step 1: Backup
    setUpdateStep('step-backup', 'active', 'Creating configuration & state backup snapshot...');
    setUpdateStep('step-pull', 'pending', 'Pulling repository changes & dependencies');
    setUpdateStep('step-restart', 'pending', 'Restarting Silas Blue service');
    setUpdateStep('step-reconnect', 'pending', 'Verifying boot health & reconnecting');
    if (progressFill) progressFill.style.width = '25%';
    if (statusSub) statusSub.textContent = 'Snapshotting local state to backups/...';

    // Transition visually to pull step after 1s
    const stepTimer = setTimeout(() => {
        setUpdateStep('step-backup', 'done', 'Configuration & state safely backed up');
        setUpdateStep('step-pull', 'active', 'Pulling Git updates & verifying dependencies...');
        if (progressFill) progressFill.style.width = '50%';
        if (statusSub) statusSub.textContent = 'Downloading latest repository commits...';
    }, 1200);

    try {
        const res = await fetch('/api/update/apply', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ channel: chan })
        });
        clearTimeout(stepTimer);
        const result = await res.json();

        if (result.success) {
            setUpdateStep('step-backup', 'done', 'Configuration & state safely backed up');
            setUpdateStep('step-pull', 'done', 'Updates and packages installed successfully');
            setUpdateStep('step-restart', 'active', 'Restarting Silas Blue service & container...');
            if (progressFill) progressFill.style.width = '75%';
            if (statusSub) statusSub.textContent = 'Initiating service reboot...';

            setTimeout(() => {
                setUpdateStep('step-restart', 'done', 'Silas Blue service restarted');
                setUpdateStep('step-reconnect', 'active', 'Verifying gateway and waiting for backend...');
                if (progressFill) progressFill.style.width = '88%';

                let attempts = 0;
                const maxAttempts = 35;
                const pollForOnline = async () => {
                    attempts++;
                    if (statusSub) {
                        statusSub.textContent = `Connecting to gateway (attempt ${attempts}/${maxAttempts})...`;
                    }
                    try {
                        const check = await fetch('/api/status', { cache: 'no-store' });
                        if (check.ok) {
                            setUpdateStep('step-reconnect', 'done', 'Silas Blue is online and healthy!');
                            if (progressFill) progressFill.style.width = '100%';
                            if (modalTitle) modalTitle.textContent = '🎉 Update Complete!';
                            if (modalSpinner) modalSpinner.textContent = '✅';
                            if (statusSub) statusSub.textContent = 'Reloading dashboard...';
                            showToast('Update complete! Silas Blue is back online.', 'success');
                            setTimeout(() => window.location.reload(), 1200);
                            return;
                        }
                    } catch (e) {}

                    if (attempts < maxAttempts) {
                        setTimeout(pollForOnline, 1800);
                    } else {
                        if (statusSub) statusSub.textContent = 'Reloading page...';
                        window.location.reload();
                    }
                };
                setTimeout(pollForOnline, 2500);
            }, 2000);

        } else {
            clearTimeout(stepTimer);
            setUpdateStep('step-pull', 'error', `Update failed: ${result.error || 'Unknown error'}`);
            if (modalTitle) modalTitle.textContent = '❌ Update Failed';
            if (modalSpinner) modalSpinner.textContent = '⚠️';
            if (statusSub) statusSub.innerHTML = `<span style="color:#ff5555">${result.error || 'Unknown error'}</span><br><button onclick="document.getElementById('update-progress-modal').style.display='none'" style="margin-top:10px;">Close</button>`;
            showToast(`Update failed: ${result.error || 'Unknown error'}`, 'error', 6000);
        }
    } catch (e) {
        clearTimeout(stepTimer);
        // If fetch aborted/failed because server rebooted immediately
        setUpdateStep('step-backup', 'done', 'Configuration & state backed up');
        setUpdateStep('step-pull', 'done', 'Updates applied');
        setUpdateStep('step-restart', 'done', 'Service reboot triggered');
        setUpdateStep('step-reconnect', 'active', 'Reconnecting to Silas Blue gateway...');
        if (progressFill) progressFill.style.width = '88%';

        let attempts = 0;
        const maxAttempts = 35;
        const pollForOnline = async () => {
            attempts++;
            if (statusSub) {
                statusSub.textContent = `Connecting to gateway (attempt ${attempts}/${maxAttempts})...`;
            }
            try {
                const check = await fetch('/api/status', { cache: 'no-store' });
                if (check.ok) {
                    setUpdateStep('step-reconnect', 'done', 'Silas Blue is back online!');
                    if (progressFill) progressFill.style.width = '100%';
                    if (modalTitle) modalTitle.textContent = '🎉 Update Complete!';
                    if (modalSpinner) modalSpinner.textContent = '✅';
                    setTimeout(() => window.location.reload(), 1200);
                    return;
                }
            } catch (err) {}

            if (attempts < maxAttempts) {
                setTimeout(pollForOnline, 1800);
            } else {
                window.location.reload();
            }
        };
        setTimeout(pollForOnline, 2500);
    }
}

async function saveUpdateSettings() {
    const payload = {
        update_channel: document.getElementById('cfg-update-channel').value,
        auto_update_enabled: document.getElementById('cfg-auto-update-enabled').checked,
        auto_update_mode: document.getElementById('cfg-auto-update-mode').value,
        auto_update_interval_hours: parseInt(document.getElementById('cfg-auto-update-interval').value) || 24,
        update_notify_channel_id: document.getElementById('cfg-update-notify-channel').value.trim() || null,
        update_notify_owner_dm: document.getElementById('cfg-update-notify-owner').checked
    };

    try {
        const res = await fetch('/api/update/config', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload)
        });
        const result = await res.json();
        showToast(result.message || 'Updater settings saved!', 'success');
        loadUpdateStatus();
    } catch (e) {
        showToast('Failed to save updater configuration', 'error');
    }
}

// --- Bot Invite Modal ---
let activeInviteUrl = '';

async function openInviteModal() {
    const modal = document.getElementById('invite-modal');
    if (!modal) return;
    modal.style.display = 'flex';

    try {
        const res = await fetch('/api/bot/invite');
        if (res.ok) {
            const data = await res.json();
            activeInviteUrl = data.invite_url || '';
            document.getElementById('invite-client-id').value = data.client_id || 'Not configured';
            document.getElementById('invite-url-input').value = activeInviteUrl || 'Token not loaded';
        }
    } catch (e) {
        console.error('Failed to fetch invite details:', e);
    }
}

function closeInviteModal() {
    const modal = document.getElementById('invite-modal');
    if (modal) modal.style.display = 'none';
}

function copyInviteLink() {
    const input = document.getElementById('invite-url-input');
    if (input && input.value) {
        navigator.clipboard.writeText(input.value);
        showToast('Invite link copied to clipboard!', 'success');
    }
}

function openInviteInBrowser() {
    if (activeInviteUrl) {
        window.open(activeInviteUrl, '_blank');
    } else {
        showToast('Invite link not available', 'error');
    }
}

// --- Initialization ---
window.addEventListener('DOMContentLoaded', () => {
    refreshStatus();
    loadAvailableModels();
    connectWebSocket();
    loadUpdateStatus();

    // Periodic status refresh
    setInterval(refreshStatus, 8000);
});
