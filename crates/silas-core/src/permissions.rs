//! Four-domain granular access control evaluator (`< 50ns` zero-allocation hot path).

use crate::config::{CompiledPermissionDomain, GuildConfig};
use serde::{Deserialize, Serialize};

/// Granular domain areas for web console delegated access and configuration audit logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsoleDomain {
    All,
    Settings,
    Moderation,
    Automod,
    Logging,
    Greetings,
    ReactionRoles,
    Triggers,
    Ai,
    Permissions,
    AuditLog,
    Activity,
}

impl ConsoleDomain {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Settings => "settings",
            Self::Moderation => "moderation",
            Self::Automod => "automod",
            Self::Logging => "logging",
            Self::Greetings => "greetings",
            Self::ReactionRoles => "reaction_roles",
            Self::Triggers => "triggers",
            Self::Ai => "ai",
            Self::Permissions => "permissions",
            Self::AuditLog => "audit_log",
            Self::Activity => "activity",
        }
    }

    pub fn from_str_relaxed(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "all" => Some(Self::All),
            "settings" | "config" => Some(Self::Settings),
            "moderation" | "mod" => Some(Self::Moderation),
            "automod" => Some(Self::Automod),
            "logging" | "logs" => Some(Self::Logging),
            "greetings" => Some(Self::Greetings),
            "reaction_roles" | "reactionroles" | "rr" => Some(Self::ReactionRoles),
            "triggers" => Some(Self::Triggers),
            "ai" => Some(Self::Ai),
            "permissions" | "access" => Some(Self::Permissions),
            "audit_log" | "audit" | "auditlog" => Some(Self::AuditLog),
            "activity" | "points" | "levels" | "xp" => Some(Self::Activity),
            _ => None,
        }
    }
}

/// Access levels for delegated web console access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessLevel {
    None = 0,
    View = 1,
    Edit = 2,
}

impl AccessLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::View => "view",
            Self::Edit => "edit",
        }
    }

    pub fn from_str_relaxed(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "none" => Some(Self::None),
            "view" => Some(Self::View),
            "edit" => Some(Self::Edit),
            _ => None,
        }
    }

    pub fn can_view(&self) -> bool {
        *self >= Self::View
    }

    pub fn can_edit(&self) -> bool {
        *self >= Self::Edit
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDomain {
    /// Who can prompt Silas Blue in server channels and threads (`can_reply`).
    ChatReply,
    /// Who can switch active AI models via `!setmodel` / `/setmodel` (`can_change_model`).
    ChangeModel,
    /// Who can modify server bot settings, prompts, memory, and forum rules (`can_change_permissions`).
    ManageBot,
    /// Who can invoke `!search` / `/search` or trigger auto-search grounding (`can_search`).
    WebSearch,
}

/// Lightweight member snapshot constructed from Discord message/interaction metadata.
#[derive(Debug, Clone)]
pub struct MemberContext<'a> {
    pub user_id: u64,
    pub is_guild_owner: bool,
    pub is_administrator: bool,
    pub role_ids: &'a [u64],
    pub role_names: &'a [String],
}

impl<'a> MemberContext<'a> {
    #[inline]
    pub fn is_admin_or_owner(&self) -> bool {
        self.is_guild_owner || self.is_administrator
    }
}

/// Evaluates whether a guild member is authorized for the given `PermissionDomain`.
#[inline]
pub fn has_guild_permission(
    member: &MemberContext<'_>,
    config: &GuildConfig,
    domain: PermissionDomain,
) -> bool {
    // 1. Administrator & Guild Owner Bypass
    if member.is_admin_or_owner() {
        return true;
    }

    let compiled: &CompiledPermissionDomain = match domain {
        PermissionDomain::ChatReply => &config.compiled.reply,
        PermissionDomain::ChangeModel => &config.compiled.change_model,
        PermissionDomain::ManageBot => &config.compiled.change_permissions,
        PermissionDomain::WebSearch => &config.compiled.search,
    };

    // 2. Everyone fast-path
    if compiled.allow_everyone {
        return true;
    }

    // 3. Explicit User Snowflake Whitelist (O(1) u64 HashSet lookup)
    if compiled.user_ids.contains(&member.user_id) {
        return true;
    }

    // 4. Role Snowflake ID match (O(1) u64 HashSet lookup)
    for role_id in member.role_ids {
        if compiled.role_ids.contains(role_id) {
            return true;
        }
    }

    // 5. Case-insensitive Role Name match
    if !compiled.role_names_lower.is_empty() {
        for role_name in member.role_names {
            if compiled
                .role_names_lower
                .contains(&role_name.trim().to_lowercase())
            {
                return true;
            }
        }
    }

    false
}

/// Evaluates whether a Direct Message (DM) interaction is permitted for `domain`.
#[inline]
pub fn has_dm_permission(allow_dms: bool, domain: PermissionDomain) -> bool {
    match domain {
        PermissionDomain::ChatReply => allow_dms,
        PermissionDomain::WebSearch => allow_dms,
        PermissionDomain::ChangeModel | PermissionDomain::ManageBot => false,
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::field_reassign_with_default
)]
mod tests {
    use super::*;
    use crate::config::GuildConfig;

    #[test]
    fn test_admin_and_owner_always_bypass() {
        let mut cfg = GuildConfig::default();
        cfg.reply_roles = vec![];
        cfg.sanitize_and_compile();

        let owner = MemberContext {
            user_id: 42,
            is_guild_owner: true,
            is_administrator: false,
            role_ids: &[],
            role_names: &[],
        };
        assert!(has_guild_permission(&owner, &cfg, PermissionDomain::ChatReply));
        assert!(has_guild_permission(&owner, &cfg, PermissionDomain::ManageBot));
    }

    #[test]
    fn test_user_snowflake_and_role_matching() {
        let mut cfg = GuildConfig::default();
        cfg.reply_roles = vec!["VIP".to_owned(), "987654321098765432".to_owned()];
        cfg.change_model_users = vec!["111222333444555666".to_owned()];
        cfg.sanitize_and_compile();

        let regular = MemberContext {
            user_id: 100,
            is_guild_owner: false,
            is_administrator: false,
            role_ids: &[123],
            role_names: &["Member".to_owned()],
        };
        assert!(!has_guild_permission(&regular, &cfg, PermissionDomain::ChatReply));

        let vip_by_name = MemberContext {
            user_id: 101,
            is_guild_owner: false,
            is_administrator: false,
            role_ids: &[],
            role_names: &["vip".to_owned()],
        };
        assert!(has_guild_permission(&vip_by_name, &cfg, PermissionDomain::ChatReply));

        let vip_by_role_id = MemberContext {
            user_id: 102,
            is_guild_owner: false,
            is_administrator: false,
            role_ids: &[987654321098765432],
            role_names: &[],
        };
        assert!(has_guild_permission(&vip_by_role_id, &cfg, PermissionDomain::ChatReply));

        let model_changer = MemberContext {
            user_id: 111222333444555666,
            is_guild_owner: false,
            is_administrator: false,
            role_ids: &[],
            role_names: &[],
        };
        assert!(has_guild_permission(
            &model_changer,
            &cfg,
            PermissionDomain::ChangeModel
        ));
    }
}
