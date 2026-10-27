//! Discord gateway client, event handlers, interactive pagination, and hybrid Poise commands.

pub mod activity;
pub mod automod;
pub mod bot;
pub mod commands;
pub mod greetings;
pub mod handlers;
pub mod logging;
pub mod moderation;
pub mod pagination_ui;
pub mod reaction_roles;
pub mod reddit;
pub mod scheduler;
pub mod triggers;

pub use bot::{
    BotController, BotState, BotUserIdentity, ConnectedGuildInfo, GuildChannelInfo, GuildRoleInfo,
};
pub use reddit::{
    deliver_reddit_post, dispatch_post_to_matching_subscriptions, spawn_reddit_poller, RedditClient,
};
