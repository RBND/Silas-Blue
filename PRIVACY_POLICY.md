# Silas Blue — Privacy Policy

**Last Updated:** October 23, 2026

Your privacy is important to us. This Privacy Policy outlines how **Silas Blue v4.0** ("we", "us", "our", or the "Bot") collects, processes, stores, and protects information when you use the Silas Blue Discord bot or run its software.

Silas Blue is designed with a **privacy-first approach**, supporting local AI execution through backends like Ollama and Llama.cpp so that conversations do not need to leave your personal machine or private server environment.

---

## 1. Scope & Hosting Model

### A. Self-Hosted Instances (Recommended / Default Model)
Silas Blue is primarily distributed as open-source software for users to host on their own machines. When you self-host Silas Blue:
- All data, configurations, and logs remain on your local hardware (`data/silas.db`).
- The repository maintainers (RobotsNeverDie) have **no access** to your bot token, server configurations, logs, or chat messages.
- The host administrator is responsible for their own data governance and compliance.

### B. Developer-Hosted / Shared Instances
If you interact with a public or shared instance operated directly by the Silas Blue developer team, this Privacy Policy governs the data processed by that specific instance.

---

## 2. Information We Process

Silas Blue only processes data necessary to provide its core features:

### A. Discord Data
- **User Identifiers:** User ID, username, and display name.
- **Server (Guild) & Channel Identifiers:** Server ID, server name, channel ID, and role IDs.
- **Message Content:** Text prompts directed to the bot (via command prefix `!`, slash commands `/`, bot mentions `@Silas Blue`, direct messages if enabled, or configured spontaneous response channels).
- **Metadata:** Timestamps of interactions and message IDs.

### B. Configuration & Preferences
- Per-server settings stored locally in the unified SQLite database (`data/silas.db` `guild_configs` table), including:
  - Selected AI model and custom system personas
  - Pagination and non-paginated reply length limits
  - Permission lists (allowed roles and user Snowflake IDs)
  - Discord Forum auto-reply monitoring rules (monitored forum channels and tags)
  - Role-based token quota limits and reset schedules
  - Spontaneous response trigger modes
  - Conversation memory turn limits and inactivity TTLs

### C. Operational Logs & Quota Accounting
- **Live Telemetry:** Silas Blue streams truncated diagnostic previews over an authenticated WebSocket to the local Web Console without persisting raw user message histories to disk logs.
- **Token Quota Records (`data/silas.db` `token_usage_ledger`):** When role-based token quotas are enabled on a server, Silas Blue records the Discord server ID, user ID (or irreversible HMAC pseudonym if `/forget_me` was invoked), token counts consumed, model name, and timestamp to calculate remaining allowances.
- **Web Dashboard Session Cookie (`silas_session`):** When an administrator logs into the Web Administration Console, a signed HMAC-SHA256 `HttpOnly; SameSite=Lax` session cookie is stored in the administrator's browser to maintain authentication.

---

## 3. How Information is Used

We use processed information solely to:
1. Receive user prompts and deliver AI-generated replies in Discord text channels or direct messages.
2. Enforce server-level access controls and custom permissions set by server administrators.
3. Format and paginate long responses to fit Discord's message length limits.
4. Diagnose technical errors, prevent crashes, and improve bot performance.

We **do not**:
- Sell, rent, trade, or monetize your personal data.
- Build advertising or marketing profiles on users.
- Request privileged Presence (`GUILD_PRESENCES`) or Server Members (`GUILD_MEMBERS`) Gateway Intents.
- Read or store messages in channels where the bot is not summoned, mentioned, or configured to respond.

---

## 4. AI Backends & Data Transmission

Silas Blue allows server administrators to choose between local and remote AI inference engines. How your prompt data travels depends on the configured backend:

### A. Local AI Providers (Ollama & Llama.cpp)
- **100% On-Device:** Prompts and responses are processed entirely on the host machine running Silas Blue.
- **Zero Third-Party Transmission:** No prompt data, user IDs, or text content are transmitted across the internet to any AI cloud provider.

### B. Cloud / Remote AI Providers (Optional)
If a server administrator or bot operator chooses to configure an external AI provider (such as Xiaomi MiMo, OpenAI, OpenRouter, Groq, DeepSeek, xAI, or custom OpenAI-compatible endpoints):
- Your prompt text (and any active conversation history or system prompt) is transmitted via encrypted HTTPS to the selected third-party provider's API to generate a response.
- Data sent to external AI providers is governed by their respective privacy policies and data retention agreements:
  - [Xiaomi MiMo Platform](https://api.xiaomimimo.com)
  - [OpenAI Privacy Policy](https://openai.com/policies/privacy-policy)
  - [Groq Privacy Policy](https://groq.com/privacy-policy/)
  - [OpenRouter Privacy Policy](https://openrouter.ai/privacy)
  - [DeepSeek Privacy Policy](https://www.deepseek.com/privacy)
  - [xAI Privacy Policy](https://x.ai/privacy)

### C. Web Search Grounding (Optional)
When real-time web search (`!search`, `/search`, `!links`, `/links`, or automatic web grounding) is enabled and triggered:
- Only the formulated search query string (never your Discord User ID or account credentials) is transmitted to the configured search provider (Bing, DuckDuckGo, Tavily, or Brave Search) to retrieve relevant web snippets and citations.

---

## 5. Data Storage, Event-Driven Eviction & Retention

- **Volatile Conversation Memory:** Multi-turn conversation history is held strictly in volatile RAM (never written to disk), isolated per server channel or per DM user, and automatically purged after the configured inactivity TTL (default 30 minutes) or immediately when a user runs `!clear` / `/clear`.
- **Real-Time Deletion Synchronization:** In strict compliance with Discord Developer Policy:
  - Deleting a message on Discord (`MESSAGE_DELETE` / `MESSAGE_DELETE_BULK`) immediately evicts that turn from volatile RAM.
  - Deleting a channel or thread (`CHANNEL_DELETE` / `THREAD_DELETE`) immediately purges that channel's memory buffer.
  - Removing Silas Blue from a server (`GUILD_DELETE`) automatically purges that server's memory buffers, `guild_configs` row, and `token_usage_ledger` records.
- **Token Usage Retention:** Local SQLite token usage records in `data/silas.db` are automatically pruned after 35 days.

---

## 6. Self-Service Data Deletion (`/forget_me`) & Your Rights

You have immediate, self-service control over your data:

- **Self-Service Erasure (`/forget_me` or `!forget_me`):** Any user can run `/forget_me` in any server channel or DM at any time to immediately erase all of their message turns across all volatile memory buffers and irreversibly pseudonymize their User ID in `token_usage_ledger` via HMAC-SHA256.
- **Server Administrators:** Removing Silas Blue from a server automatically deletes all stored server configurations and token ledger entries for that server.
- **Self-Hosted Deletion:** If you run your own instance, deleting `data/silas.db` completely purges all stored state.

---

## 7. Children's Privacy

Silas Blue is not directed at or intended for children under 13 years of age (or the minimum legal age in your jurisdiction). We do not knowingly collect personal information from children. If you believe a child has provided personal information through the Bot, please contact us so we can promptly remove it.

---

## 8. Changes to This Privacy Policy

We may update this Privacy Policy periodically to reflect new features, AI integrations, or regulatory requirements. Any modifications will be posted directly to this file in the official repository.

---

## 9. Contact Information

If you have questions, feedback, or data privacy requests, please reach out via:

- **Discord:** [RobotsNeverDie (User ID: 296353246920835074)](https://discord.com/users/296353246920835074)
- **GitHub Repository:** [https://github.com/RBND/Silas-Blue](https://github.com/RBND/Silas-Blue)
