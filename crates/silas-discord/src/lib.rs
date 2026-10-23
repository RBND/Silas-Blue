//! Discord gateway client, event handlers, interactive pagination, and hybrid Poise commands.

pub mod bot;
pub mod commands;
pub mod handlers;
pub mod pagination_ui;

pub use bot::{
    BotController, BotState, BotUserIdentity, ConnectedGuildInfo, GuildChannelInfo, GuildRoleInfo,
};
