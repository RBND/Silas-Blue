//! Importer module for Carl.gg (Linus Turtle Tips) bot configuration backups.
//! Parses network endpoint dumps into Silas Blue storage models.

use serde_json::Value;
use silas_storage::{
    AutomodConfig, GreetingsConfig, LoggingConfig, NewCustomTrigger, NewReactionRoleEntry,
    NewReactionRoleGroup, PunishmentStep, Storage,
};
use std::path::Path;

#[derive(Debug, Default)]
pub struct CarlImportReport {
    pub guild_id: u64,
    pub welcome_configured: bool,
    pub welcome_channel_id: Option<u64>,
    pub logging_configured: bool,
    pub logging_channel_id: Option<u64>,
    pub automod_words_count: usize,
    pub automod_ignored_channels_count: usize,
    pub automod_ignored_roles_count: usize,
    pub reaction_role_groups_count: usize,
    pub reaction_role_entries_count: usize,
    pub triggers_count: usize,
    pub tags_count: usize,
}

pub struct ParsedCarlData {
    pub guild_id: u64,
    pub greetings: GreetingsConfig,
    pub logging: LoggingConfig,
    pub automod: AutomodConfig,
    pub reaction_roles: Vec<(NewReactionRoleGroup, Vec<NewReactionRoleEntry>)>,
    pub triggers: Vec<NewCustomTrigger>,
}

/// Normalizes Carl.gg TagScript placeholders to Silas Blue syntax (e.g. `{mention}` -> `{user}`).
pub fn normalize_tagscript(text: &str) -> String {
    text.replace("{mention}", "{user}")
        .replace("{user.mention}", "{user}")
}

/// Extracts a named endpoint from `captured_network` in the Carl.gg export JSON.
fn extract_endpoint<'a>(json: &'a Value, guild_id: u64, name: &str) -> Option<&'a Value> {
    let captured = json.get("captured_network")?.as_object()?;
    let suffix = format!("/{name}");
    for (k, v) in captured {
        if k.contains(&guild_id.to_string()) && k.ends_with(&suffix) {
            return Some(v);
        }
    }
    // Also try without guild ID in key
    for (k, v) in captured {
        if k.ends_with(&suffix) {
            return Some(v);
        }
    }
    None
}

/// Parses the Carl.gg backup JSON into typed Silas Blue configuration objects.
pub fn parse_carl_backup(raw_json: &str, target_guild_id: u64) -> Result<ParsedCarlData, String> {
    let root: Value = serde_json::from_str(raw_json).map_err(|e| format!("Invalid JSON: {e}"))?;

    // Determine actual guild ID
    let guild_id = if target_guild_id > 0 {
        target_guild_id
    } else {
        root.get("_metadata")
            .and_then(|m| m.get("guild_id"))
            .and_then(|g| g.as_str())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
    };

    if guild_id == 0 {
        return Err("Could not determine Guild ID from backup. Pass --guild-id explicitly.".to_string());
    }

    // 1. Welcome / Greetings
    let mut greetings = GreetingsConfig::default_for_guild(guild_id);
    if let Some(welcome_val) = extract_endpoint(&root, guild_id, "welcome") {
        if let Some(ch_str) = welcome_val.get("greet_channel").and_then(|v| v.as_str()) {
            if let Ok(ch_id) = ch_str.trim().parse::<u64>() {
                if ch_id > 0 {
                    greetings.welcome_channel_id = Some(ch_id);
                }
            }
        }
        if let Some(msg) = welcome_val.get("greet_message").and_then(|v| v.as_str()) {
            let trimmed = msg.trim();
            if !trimmed.is_empty() {
                greetings.welcome_message = normalize_tagscript(trimmed);
            }
        }
    }

    // 2. Logging
    let mut logging = LoggingConfig::default_for_guild(guild_id);
    if let Some(log_val) = extract_endpoint(&root, guild_id, "logging") {
        if let Some(ch_str) = log_val.get("log_channel").and_then(|v| v.as_str()) {
            if let Ok(ch_id) = ch_str.trim().parse::<u64>() {
                if ch_id > 0 {
                    logging.unified_channel_id = Some(ch_id);
                }
            }
        }
    }

    // 3. Automod
    let mut automod = AutomodConfig::default_for_guild(guild_id);
    if let Some(am_val) = extract_endpoint(&root, guild_id, "automod") {
        if let Some(settings) = am_val.get("settings").and_then(|v| v.as_object()) {
            automod.enabled = true;
            automod.anti_spam_enabled = true;
            automod.anti_invites_enabled = true;

            // Blocked words
            if let Some(words_arr) = settings.get("bad_words").and_then(|v| v.as_array()) {
                let mut words = Vec::new();
                for w in words_arr {
                    if let Some(s) = w.as_str() {
                        let trimmed = s.trim();
                        if !trimmed.is_empty() {
                            words.push(trimmed.to_string());
                        }
                    }
                }
                automod.blocked_words = words;
            }

            // Ignored channels whitelist
            if let Some(ch_wl) = settings.get("channel_wl").and_then(|v| v.as_array()) {
                let channels: Vec<u64> = ch_wl
                    .iter()
                    .filter_map(|v| v.as_str().and_then(|s| s.parse::<u64>().ok()))
                    .collect();
                automod.ignored_channels = channels;
            }

            // Ignored roles whitelist
            if let Some(ro_wl) = settings.get("role_wl").and_then(|v| v.as_array()) {
                let roles: Vec<u64> = ro_wl
                    .iter()
                    .filter_map(|v| v.as_str().and_then(|s| s.parse::<u64>().ok()))
                    .collect();
                automod.ignored_roles = roles;
            }

            // Anti-spam slowmode [max_messages, interval_seconds]
            if let Some(slowmode) = settings.get("slowmode").and_then(|v| v.as_array()) {
                if let (Some(m), Some(i)) = (
                    slowmode.first().and_then(|v| v.as_i64()),
                    slowmode.get(1).and_then(|v| v.as_i64()),
                ) {
                    automod.anti_spam_max_messages = m.clamp(2, 50);
                    automod.anti_spam_interval_seconds = i.clamp(1, 60);
                }
            }

            // Anti-mention spam [max_mentions, ...]
            if let Some(mentions) = settings.get("mentionspam").and_then(|v| v.as_array()) {
                if let Some(m) = mentions.first().and_then(|v| v.as_i64()) {
                    automod.anti_mention_max = m.clamp(2, 50);
                }
            }

            // Default punishment ladder with warn decay of 7 days (168h)
            automod.strike_decay_hours = 168;
            automod.punishment_ladder = vec![
                PunishmentStep {
                    strikes: 2,
                    action: "warn".to_string(),
                    duration_seconds: None,
                },
                PunishmentStep {
                    strikes: 4,
                    action: "timeout".to_string(),
                    duration_seconds: Some(3600),
                },
                PunishmentStep {
                    strikes: 8,
                    action: "kick".to_string(),
                    duration_seconds: None,
                },
                PunishmentStep {
                    strikes: 10,
                    action: "ban".to_string(),
                    duration_seconds: None,
                },
            ];
        }
    }

    // 4. Reaction Roles
    let mut reaction_roles = Vec::new();
    if let Some(rr_val) = extract_endpoint(&root, guild_id, "reactionroles") {
        if let Some(rr_list) = rr_val.get("rr").and_then(|v| v.as_array()) {
            for item in rr_list {
                let msg_id_str = item
                    .get("message_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let message_id = msg_id_str.parse::<u64>().unwrap_or(0);
                if message_id == 0 {
                    continue;
                }

                let messages_arr = match item.get("messages").and_then(|v| v.as_array()) {
                    Some(arr) => arr,
                    None => continue,
                };

                let channel_id = messages_arr
                    .first()
                    .and_then(|m| m.get("channel_id"))
                    .and_then(|c| c.as_str())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);

                if channel_id == 0 {
                    continue;
                }

                let group = NewReactionRoleGroup {
                    guild_id,
                    channel_id,
                    message_id,
                    group_type: "normal".to_string(),
                    required_role_id: None,
                    component_type: "reaction".to_string(),
                };

                let mut entries = Vec::new();
                for m in messages_arr {
                    let emoji_or_id = m
                        .get("reaction_emoji")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let role_id = m
                        .get("role_id")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(0);

                    if !emoji_or_id.is_empty() && role_id > 0 {
                        entries.push(NewReactionRoleEntry {
                            group_id: 0, // Assigned on group insertion
                            emoji_or_custom_id: emoji_or_id,
                            role_id,
                            button_label: None,
                            button_style: None,
                        });
                    }
                }

                if !entries.is_empty() {
                    reaction_roles.push((group, entries));
                }
            }
        }
    }

    // 5. Triggers
    let mut triggers = Vec::new();
    if let Some(trig_val) = extract_endpoint(&root, guild_id, "triggers") {
        if let Some(trig_list) = trig_val.get("triggers").and_then(|v| v.as_array()) {
            for t in trig_list {
                let name = t.get("trigger").and_then(|v| v.as_str()).unwrap_or("").trim();
                let response = t.get("response").and_then(|v| v.as_str()).unwrap_or("").trim();
                let match_type = t.get("match_type").and_then(|v| v.as_i64()).unwrap_or(1);

                if name.is_empty() || response.is_empty() {
                    continue;
                }

                let match_mode = match match_type {
                    1 => "exact",
                    2 => "contains",
                    3 => "contains",
                    _ => "exact",
                };

                triggers.push(NewCustomTrigger {
                    guild_id,
                    trigger_name: name.to_string(),
                    match_mode: match_mode.to_string(),
                    response_template: normalize_tagscript(response),
                    cooldown_seconds: 0,
                    allowed_roles: Vec::new(),
                    ignored_channels: Vec::new(),
                    delete_trigger_message: false,
                    reply_in_dm: false,
                    enabled: true,
                });
            }
        }
    }

    // 6. Tags (imported as exact-match triggers)
    if let Some(tag_val) = extract_endpoint(&root, guild_id, "tags") {
        if let Some(tag_list) = tag_val.get("tags").and_then(|v| v.as_array()) {
            for tag in tag_list {
                let name = tag.get("name").and_then(|v| v.as_str()).unwrap_or("").trim();
                let content = tag.get("content").and_then(|v| v.as_str()).unwrap_or("").trim();

                if name.is_empty() || content.is_empty() {
                    continue;
                }

                // If trigger already exists under this name, do not duplicate
                if !triggers.iter().any(|t| t.trigger_name.eq_ignore_ascii_case(name)) {
                    triggers.push(NewCustomTrigger {
                        guild_id,
                        trigger_name: name.to_string(),
                        match_mode: "exact".to_string(),
                        response_template: normalize_tagscript(content),
                        cooldown_seconds: 0,
                        allowed_roles: Vec::new(),
                        ignored_channels: Vec::new(),
                        delete_trigger_message: false,
                        reply_in_dm: false,
                        enabled: true,
                    });
                }
            }
        }
    }

    Ok(ParsedCarlData {
        guild_id,
        greetings,
        logging,
        automod,
        reaction_roles,
        triggers,
    })
}

/// Executes the migration into `silas.db` or simulates in dry-run mode.
pub fn execute_carl_import(
    data: &ParsedCarlData,
    db_path: &Path,
    dry_run: bool,
) -> Result<CarlImportReport, Box<dyn std::error::Error>> {
    let report = CarlImportReport {
        guild_id: data.guild_id,
        welcome_configured: data.greetings.welcome_channel_id.is_some(),
        welcome_channel_id: data.greetings.welcome_channel_id,
        logging_configured: data.logging.unified_channel_id.is_some(),
        logging_channel_id: data.logging.unified_channel_id,
        automod_words_count: data.automod.blocked_words.len(),
        automod_ignored_channels_count: data.automod.ignored_channels.len(),
        automod_ignored_roles_count: data.automod.ignored_roles.len(),
        reaction_role_groups_count: data.reaction_roles.len(),
        reaction_role_entries_count: data.reaction_roles.iter().map(|(_, e)| e.len()).sum(),
        triggers_count: data.triggers.iter().filter(|t| t.match_mode != "exact" || t.trigger_name.starts_with('!') || t.trigger_name.contains(' ')).count(),
        tags_count: data.triggers.len() - data.triggers.iter().filter(|t| t.match_mode != "exact" || t.trigger_name.starts_with('!') || t.trigger_name.contains(' ')).count(),
    };

    if dry_run {
        return Ok(report);
    }

    // Apply directly using Storage API
    let storage = Storage::open(db_path)?;

    // 1. Greetings
    storage.save_greetings_config(&data.greetings)?;

    // 2. Logging
    storage.save_logging_config(&data.logging)?;

    // 3. Automod
    storage.save_automod_config(&data.automod)?;

    // 4. Reaction Roles
    // Remove existing reaction role groups for this guild and recreated from backup
    let existing_groups = storage.get_guild_reaction_roles(data.guild_id)?;
    for g in existing_groups {
        let _ = storage.delete_reaction_role_group(g.id);
    }
    for (group_def, entries) in &data.reaction_roles {
        let created_group = storage.create_reaction_role_group(group_def)?;
        for entry_def in entries {
            let mut ent = entry_def.clone();
            ent.group_id = created_group.id;
            storage.add_reaction_role_entry(&ent)?;
        }
    }

    // 5. Triggers & Tags
    for trigger in &data.triggers {
        storage.create_custom_trigger(trigger)?;
    }

    Ok(report)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_tagscript_replaces_mention() {
        let raw = "Welcome {mention} to {server}!";
        let normalized = normalize_tagscript(raw);
        assert_eq!(normalized, "Welcome {user} to {server}!");
    }

    #[test]
    fn test_parse_carl_backup_extracts_all_modules() {
        let sample_json = r#"{
            "_metadata": { "guild_id": "295767175426473984" },
            "captured_network": {
                "/api/v1/servers/295767175426473984/welcome": {
                    "greet_channel": "295767175426473984",
                    "greet_message": "Welcome {mention}!"
                },
                "/api/v1/servers/295767175426473984/logging": {
                    "log_channel": "824319214248132758"
                },
                "/api/v1/servers/295767175426473984/automod": {
                    "settings": {
                        "bad_words": ["bad1", "bad2"],
                        "channel_wl": ["123"],
                        "role_wl": ["456"],
                        "slowmode": [5, 5],
                        "mentionspam": [10, 10]
                    }
                },
                "/api/v1/servers/295767175426473984/reactionroles": {
                    "rr": [
                        {
                            "message_id": "111",
                            "messages": [
                                {
                                    "channel_id": "222",
                                    "reaction_emoji": "👍",
                                    "role_id": "333"
                                }
                            ]
                        }
                    ]
                },
                "/api/v1/servers/295767175426473984/triggers": {
                    "triggers": [
                        {
                            "trigger": "aasp",
                            "match_type": 1,
                            "response": "Apple Authorized"
                        }
                    ]
                },
                "/api/v1/servers/295767175426473984/tags": {
                    "tags": [
                        {
                            "name": "faq",
                            "content": "Read here"
                        }
                    ]
                }
            }
        }"#;

        let parsed = parse_carl_backup(sample_json, 295767175426473984).expect("valid parse");
        assert_eq!(parsed.guild_id, 295767175426473984);
        assert_eq!(parsed.greetings.welcome_channel_id, Some(295767175426473984));
        assert_eq!(parsed.greetings.welcome_message, "Welcome {user}!");
        assert_eq!(parsed.logging.unified_channel_id, Some(824319214248132758));
        assert_eq!(parsed.automod.blocked_words.len(), 2);
        assert_eq!(parsed.automod.ignored_channels, vec![123]);
        assert_eq!(parsed.automod.ignored_roles, vec![456]);
        assert_eq!(parsed.reaction_roles.len(), 1);
        assert_eq!(parsed.triggers.len(), 2); // 1 trigger + 1 tag
    }
}
