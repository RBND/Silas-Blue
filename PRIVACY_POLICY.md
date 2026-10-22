# Silas Blue — Privacy Policy

**Last Updated:** October 22, 2026

Your privacy is important to us. This Privacy Policy outlines how **Silas Blue** ("we", "us", "our", or the "Bot") collects, processes, stores, and protects information when you use the Silas Blue Discord bot or run its software.

Silas Blue is designed with a **privacy-first approach**, supporting local AI execution through backends like Ollama and Llama.cpp so that conversations do not need to leave your personal machine or private server environment.

---

## 1. Scope & Hosting Model

### A. Self-Hosted Instances (Recommended / Default Model)
Silas Blue is primarily distributed as open-source software for users to host on their own machines. When you self-host Silas Blue:
- All data, configurations, and logs remain on your local hardware.
- The repository maintainers (RobotsNeverDie) have **no access** to your bot token, server configurations, logs, or chat messages.
- The host administrator is responsible for their own data governance and compliance.

### B. Developer-Hosted / Shared Instances
If you interact with a public or shared instance operated directly by the Silas Blue developer team, this Privacy Policy governs the data processed by that specific instance.

---

## 2. Information We Process

Silas Blue only processes data necessary to provide its core features:

### A. Discord Data
- **User Identifiers:** User ID, username, and discriminator/display name.
- **Server (Guild) & Channel Identifiers:** Server ID, server name, channel ID, and role IDs.
- **Message Content:** Text prompts directed to the bot (via command prefix `!`, bot mentions `@Silas Blue`, direct messages if enabled, or configured random response channels).
- **Metadata:** Timestamps of interactions and message IDs.

### B. Configuration & Preferences
- Per-server settings stored locally in JSON files (`config/<guild_id>.json`), including:
  - Selected AI model and custom system personas
  - Pagination and non-paginated reply length limits
  - Permission lists (allowed roles and user Snowflake IDs)
  - Discord Forum auto-reply monitoring rules (monitored forum channels and tags)
  - Role-based token quota limits and reset schedules
  - Random response trigger probabilities
  - Conversation memory turn limits and inactivity TTLs

### C. Operational Logs & Quota Accounting
- **Operational Logs:** Silas Blue maintains local operational logs (`logs/` and terminal output) containing recent prompt/reply events, connection states, and diagnostic error reports stored locally on the host machine.
- **Token Quota Records (`state/usage.db`):** When role-based token quotas are enabled on a server, Silas Blue records the Discord server ID, user ID, token counts consumed, model name, and timestamp in a local SQLite database to calculate remaining allowances.
- **Web Dashboard Session Cookie (`sb_session`):** When an administrator logs into the Web Management Console, an encrypted/signed `HttpOnly` session cookie is stored in the administrator's browser to maintain authentication.

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
When real-time web search (`!search`, `!links`, or automatic web grounding) is enabled and triggered:
- Only the formulated search query string (never your Discord User ID or account credentials) is transmitted to the configured search provider (Bing, DuckDuckGo, Tavily, or Brave Search) to retrieve relevant web snippets and citations.

---

## 5. Data Storage & Retention

- **Volatile Conversation Memory:** Multi-turn conversation history is held strictly in volatile RAM (never written to disk), isolated per server channel (`guild_<id>_<channel_id>`) or per DM user (`dm_user_<id>`), and automatically purged after the configured inactivity TTL (default 30 minutes) or immediately when a user runs `!clear`.
- **Token Usage Retention:** Local SQLite token usage records in `state/usage.db` are automatically pruned after 35 days.
- **Configuration Files:** Server preferences are stored on disk in the `config/` directory until updated by an administrator or deleted.
- **Local Logs:** Diagnostic logs in the `logs/` directory are maintained on the host system for troubleshooting.

---

## 6. Data Deletion & Your Rights

You have control over your data:

- **Server Administrators:** Removing Silas Blue from a server will halt all data processing for that server. Administrators can delete their server's configuration file (`config/<guild_id>.json`) from the host machine at any time.
- **Data Deletion Requests:** For instances operated by RobotsNeverDie, users or server owners can request immediate deletion of any associated server configurations or log entries by contacting us.
- **Self-Hosted Deletion:** If you run your own instance, deleting the `config/` folder or removing the bot folder completely purges all stored state and logs.

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
