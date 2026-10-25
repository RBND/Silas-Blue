import { Devvit } from '@devvit/public-api';

// Configure Devvit capabilities
Devvit.configure({
  http: true,
  redditAPI: true,
});

// Add App Configuration Settings for Subreddit Moderators
Devvit.addSettings([
  {
    type: 'string',
    name: 'silasWebhookUrl',
    label: 'Silas Blue Webhook URL',
    placeholder: 'https://your-domain.example.com/api/webhooks/reddit',
    scope: 'installation',
    isSecret: false,
  },
  {
    type: 'string',
    name: 'silasWebhookSecret',
    label: 'Silas Blue Webhook Secret',
    placeholder: 'Paste the shared secret from Silas Blue Web Console',
    scope: 'installation',
    isSecret: true,
  },
]);

// Real-time Event Trigger: fired instantly when a post is created on the subreddit
Devvit.addTrigger({
  event: 'PostCreate',
  onEvent: async (event, context) => {
    if (!event.post) {
      return;
    }

    const settings = context.settings;
    const webhookUrl = (await settings.get<string>('silasWebhookUrl'))?.trim();
    const webhookSecret = (await settings.get<string>('silasWebhookSecret'))?.trim();

    if (!webhookUrl || !webhookSecret) {
      console.warn('[Silas Blue Devvit] Webhook URL or Secret not configured in subreddit settings.');
      return;
    }

    const post = event.post;
    const authorName = event.author ? event.author.name : (post.authorName || '[deleted]');
    const subredditName = context.subredditName || post.subredditName || '';

    const payload = {
      id: post.id,
      subreddit: subredditName,
      title: post.title,
      author: authorName,
      permalink: post.permalink || `/r/${subredditName}/comments/${post.id}/`,
      url: post.url || `https://www.reddit.com${post.permalink || ''}`,
      selftext: post.selftext || null,
      flair: post.linkFlair ? post.linkFlair.text : null,
      score: post.score ?? 1,
      is_nsfw: Boolean(post.nsfw),
      thumbnail_url: post.thumbnail?.url || null,
      image_url: null,
      created_utc: Math.floor(new Date(post.createdAt).getTime() / 1000),
    };

    try {
      const response = await fetch(webhookUrl, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'X-Silas-Reddit-Secret': webhookSecret,
          'User-Agent': 'SilasBlue-Devvit/1.0.0',
        },
        body: JSON.stringify(payload),
      });

      if (!response.ok) {
        const text = await response.text();
        console.error(`[Silas Blue Devvit] Webhook failed (${response.status}): ${text}`);
      } else {
        console.log(`[Silas Blue Devvit] Successfully dispatched post ${post.id} to Silas Blue.`);
      }
    } catch (err) {
      console.error('[Silas Blue Devvit] Network error sending webhook payload:', err);
    }
  },
});

export default Devvit;
