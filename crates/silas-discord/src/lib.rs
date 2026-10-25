//! Discord gateway client, event handlers, interactive pagination, and hybrid Poise commands.

pub mod bot;
pub mod commands;
pub mod handlers;
pub mod pagination_ui;
pub mod reddit;

pub use bot::{
    BotController, BotState, BotUserIdentity, ConnectedGuildInfo, GuildChannelInfo, GuildRoleInfo,
};
pub use reddit::{
    deliver_reddit_post, dispatch_post_to_matching_subscriptions, spawn_reddit_poller, RedditClient,
};
