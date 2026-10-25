# Silas Blue Companion Devvit App

This companion Reddit Developer Platform (Devvit) application pushes real-time `PostCreate` notifications to your Silas Blue Discord bot via an authenticated HTTP webhook.

When installed on a subreddit you moderate, notifications are delivered to Discord within milliseconds of submission without waiting for polling intervals.

---

## Prerequisites

1. Node.js 18+ installed on your machine.
2. The Reddit Devvit CLI:
   ```bash
   npm install -g devvit
   ```
3. A Reddit account with moderator permissions on the target subreddit.
4. Your Silas Blue instance reachable via a public URL (or through Cloudflare Tunnel, Caddy, Tailscale Funnel, etc.) at `https://your-domain/api/webhooks/reddit`.

---

## Setup & Deployment

### 1. Generate Webhook Secret in Silas Blue
1. Log into your Silas Blue Web Console (`http://localhost:8080`).
2. Go to **Settings** -> **Reddit Integration**.
3. Click **Generate** next to **Devvit Webhook Secret** and click **Save Global Settings**.

### 2. Login & Upload Devvit App
From the `devvit/` directory:

```bash
cd devvit
npm install
devvit login
devvit upload
```

### 3. Install on your Subreddit
```bash
devvit install <your-subreddit-name>
```

### 4. Configure App Settings
1. On Reddit, go to your subreddit's **Mod Tools** -> **Apps** -> **Installed Apps**.
2. Select **Silas Blue Companion**.
3. Enter:
   - **Silas Blue Webhook URL**: `https://your-domain.example.com/api/webhooks/reddit`
   - **Silas Blue Webhook Secret**: Paste the secret generated in Step 1.
4. Click **Save Settings**.

### 5. Subscribe a Discord Channel
In Discord:
```text
/reddit subscribe subreddit:<your-subreddit> channel:#news
```
Or in the Silas Blue Web Console under **Server Configuration** -> **Reddit Feeds**.

Now, whenever someone posts in your subreddit, Silas Blue will immediately format and post a rich Discord embed to your channel!
