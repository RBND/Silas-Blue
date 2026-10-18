// Silas Blue Web Dashboard Client Logic

let activePage = 'dashboard';
let ws = null;
let reconnectTimer = null;
let currentGuilds = [];

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
            alert(`Active model switched to: ${model}`);
        }
    } catch (e) {
        alert('Failed to change model');
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
        alert(result.message || 'Pull initiated. Check live logs for progress.');
        input.value = '';
    } catch (e) {
        alert('Failed to initiate pull');
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
        (data.presets || []).forEach(p => {
            const opt = document.createElement('option');
            opt.value = p;
            opt.textContent = p;
            if (p === cfg.remote_preset) opt.selected = true;
            presetSel.appendChild(opt);
        });

        document.getElementById('cfg-remote-url').value = cfg.remote_base_url || '';
        document.getElementById('cfg-remote-key').value = cfg.remote_api_key || '';
        document.getElementById('cfg-remote-model').value = cfg.remote_model || '';

        document.getElementById('cfg-system-prompt').value = cfg.system_prompt || '';
        document.getElementById('cfg-allow-dms').checked = cfg.allow_dms !== false;
        document.getElementById('cfg-dm-model').value = cfg.dm_default_model || '';
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
        dm_default_model: document.getElementById('cfg-dm-model').value.trim()
    };

    try {
        const res = await fetch('/api/ai/config', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload)
        });
        const result = await res.json();
        alert(result.message || 'Settings saved!');
        refreshStatus();
    } catch (e) {
        alert('Failed to save AI settings');
    }
}

// --- Server Configuration ---
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
        const roles = data.available_roles || [];

        renderRolesCheckboxes('roles-reply', roles, cfg.reply_roles || []);
        renderRolesCheckboxes('roles-model', roles, cfg.change_model_roles || []);
        renderRolesCheckboxes('roles-perm', roles, cfg.change_permission_roles || []);

        document.getElementById('cfg-pagination').checked = cfg.pagination_enabled !== false;
        document.getElementById('cfg-max-chars').value = cfg.pagination_max_chars || 1900;
        document.getElementById('cfg-server-system-prompt').value = cfg.system_prompt || '';
    } catch (e) {
        console.error('Failed to load guild config:', e);
    }
}

function renderRolesCheckboxes(containerId, roles, selectedRoles) {
    const container = document.getElementById(containerId);
    container.innerHTML = '';
    const selectedLower = selectedRoles.map(r => r.toLowerCase());

    roles.forEach(r => {
        const div = document.createElement('div');
        div.style.marginBottom = '4px';
        const cb = document.createElement('input');
        cb.type = 'checkbox';
        cb.value = r.name;
        cb.checked = selectedLower.includes(r.name.toLowerCase());
        cb.style.width = 'auto';
        cb.style.marginRight = '8px';

        const lbl = document.createElement('label');
        lbl.textContent = r.name;
        lbl.prepend(cb);
        div.appendChild(lbl);
        container.appendChild(div);
    });
}

function getSelectedRoles(containerId) {
    const checked = document.querySelectorAll(`#${containerId} input:checked`);
    return Array.from(checked).map(cb => cb.value);
}

async function saveServerConfig() {
    const guildId = document.getElementById('guild-select').value;
    if (!guildId) return;

    const payload = {
        reply_roles: getSelectedRoles('roles-reply'),
        change_model_roles: getSelectedRoles('roles-model'),
        change_permission_roles: getSelectedRoles('roles-perm'),
        pagination_enabled: document.getElementById('cfg-pagination').checked,
        pagination_max_chars: parseInt(document.getElementById('cfg-max-chars').value) || 1900,
        system_prompt: document.getElementById('cfg-server-system-prompt').value.trim()
    };

    try {
        const res = await fetch(`/api/guild/${guildId}`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload)
        });
        const result = await res.json();
        alert(result.message || 'Server configuration saved!');
    } catch (e) {
        alert('Failed to save server config');
    }
}

// --- Initialization ---
window.addEventListener('DOMContentLoaded', () => {
    refreshStatus();
    loadAvailableModels();
    connectWebSocket();

    // Periodic status refresh
    setInterval(refreshStatus, 8000);
});
