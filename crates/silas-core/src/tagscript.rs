//! Bounded, safe TagScript parser for Discord dynamic responses, embeds, actions, and AI directives.

use std::collections::HashMap;

use chrono::Utc;
use rand::Rng;
use serde::{Deserialize, Serialize};

/// Maximum nesting depth for TagScript tag evaluation to prevent recursion exhaustion.
pub const MAX_NESTING_DEPTH: usize = 5;

/// Maximum length of the generated TagScript body text.
pub const MAX_OUTPUT_LENGTH: usize = 4_000;

/// Evaluation context supplied to the TagScript engine.
#[derive(Debug, Clone, Default)]
pub struct TagScriptContext {
    pub author_id: u64,
    pub author_name: String,
    pub author_avatar: Option<String>,
    pub author_created_at: Option<String>,
    pub target_id: Option<u64>,
    pub target_name: Option<String>,
    pub guild_id: Option<u64>,
    pub guild_name: Option<String>,
    pub guild_members: Option<u64>,
    pub channel_id: Option<u64>,
    pub channel_name: Option<String>,
    pub custom_vars: HashMap<String, String>,
}

/// A field within a TagScript-generated embed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagScriptEmbedField {
    pub name: String,
    pub value: String,
    pub inline: bool,
}

/// Embed data constructed by TagScript tags.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagScriptEmbed {
    pub title: Option<String>,
    pub description: Option<String>,
    pub color: Option<u32>,
    pub image_url: Option<String>,
    pub thumbnail_url: Option<String>,
    pub footer: Option<String>,
    pub fields: Vec<TagScriptEmbedField>,
}

/// Action directives parsed out of a TagScript template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TagScriptAction {
    DeleteMessage,
    SendDm,
    AddReaction(String),
    AddRole(String),
    RemoveRole(String),
}

/// The evaluated result of a TagScript template.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagScriptOutput {
    pub body: String,
    pub embed: Option<TagScriptEmbed>,
    pub actions: Vec<TagScriptAction>,
    pub ai_prompt: Option<String>,
}

#[derive(Default)]
struct TagScriptCollector {
    embed: Option<TagScriptEmbed>,
    actions: Vec<TagScriptAction>,
    ai_prompt: Option<String>,
}

/// TagScript processing engine.
pub struct TagScriptEngine;

impl TagScriptEngine {
    /// Evaluates `template` with the given `TagScriptContext`.
    pub fn eval(template: &str, ctx: &TagScriptContext) -> TagScriptOutput {
        let mut collector = TagScriptCollector::default();
        let body = parse_tags(template, ctx, 0, &mut collector);
        TagScriptOutput {
            body,
            embed: collector.embed,
            actions: collector.actions,
            ai_prompt: collector.ai_prompt,
        }
    }
}

/// Convenience function to evaluate a TagScript template.
pub fn process_tagscript(template: &str, ctx: &TagScriptContext) -> TagScriptOutput {
    TagScriptEngine::eval(template, ctx)
}

fn parse_tags(
    input: &str,
    ctx: &TagScriptContext,
    depth: usize,
    collector: &mut TagScriptCollector,
) -> String {
    if depth > MAX_NESTING_DEPTH {
        return input.to_string();
    }

    let mut result = String::with_capacity(input.len());
    let mut chars = input.char_indices().peekable();

    while let Some((idx, ch)) = chars.next() {
        if result.len() >= MAX_OUTPUT_LENGTH {
            break;
        }

        if ch == '{' {
            let mut brace_depth: usize = 1;
            let mut end_idx = None;
            let inner_start = idx.saturating_add(1);

            for (next_idx, next_ch) in chars.by_ref() {
                if next_ch == '{' {
                    brace_depth = brace_depth.saturating_add(1);
                } else if next_ch == '}' {
                    brace_depth = brace_depth.saturating_sub(1);
                    if brace_depth == 0 {
                        end_idx = Some(next_idx);
                        break;
                    }
                }
            }

            if let Some(end) = end_idx {
                if let Some(tag_content) = input.get(inner_start..end) {
                    let evaluated = eval_single_tag(tag_content, ctx, depth, collector);
                    result.push_str(&evaluated);
                }
            } else {
                result.push('{');
                if let Some(rest) = input.get(inner_start..) {
                    result.push_str(rest);
                }
                break;
            }
        } else {
            result.push(ch);
        }
    }

    truncate_to_max(&result, MAX_OUTPUT_LENGTH)
}

fn eval_single_tag(
    tag_content: &str,
    ctx: &TagScriptContext,
    depth: usize,
    collector: &mut TagScriptCollector,
) -> String {
    let trimmed = tag_content.trim();

    // 1. Control & Logic: if condition
    if trimmed.starts_with("if(") || trimmed.starts_with("if ") {
        return eval_if_tag(trimmed, ctx, depth, collector);
    }

    // 2. Control & Logic: any choice
    if let Some(rest) = trimmed.strip_prefix("any:") {
        let options = split_top_level(rest, '|');
        if options.is_empty() {
            return String::new();
        }
        let pick_idx = rand::thread_rng().gen_range(0..options.len());
        if let Some(chosen) = options.get(pick_idx) {
            return parse_tags(chosen.trim(), ctx, depth.saturating_add(1), collector);
        }
        return String::new();
    }

    // 3. Control & Logic: percentage chance e.g. {50: heads | tails}
    if let Some(colon_pos) = trimmed.find(':') {
        if let Some(prefix) = trimmed.get(..colon_pos) {
            let prefix_trimmed = prefix.trim();
            if !prefix_trimmed.is_empty() && prefix_trimmed.chars().all(|c| c.is_ascii_digit()) {
                if let Ok(pct) = prefix_trimmed.parse::<u32>() {
                    let pct_clamped = pct.min(100);
                    let rest_pos = colon_pos.saturating_add(1);
                    let rest = trimmed.get(rest_pos..).unwrap_or("");
                    let branches = split_top_level(rest, '|');
                    let roll = rand::thread_rng().gen_range(0..100);
                    let chosen = if roll < pct_clamped {
                        branches.first().copied().unwrap_or("")
                    } else {
                        branches.get(1).copied().unwrap_or("")
                    };
                    return parse_tags(chosen.trim(), ctx, depth.saturating_add(1), collector);
                }
            }
        }
    }

    // 4. Action Directives
    if trimmed == "delete" {
        collector.actions.push(TagScriptAction::DeleteMessage);
        return String::new();
    }
    if trimmed == "dm" {
        collector.actions.push(TagScriptAction::SendDm);
        return String::new();
    }
    if let Some(rest) = trimmed.strip_prefix("react:") {
        let emoji = parse_tags(rest, ctx, depth.saturating_add(1), collector);
        let clean_emoji = emoji.trim();
        if !clean_emoji.is_empty() {
            collector
                .actions
                .push(TagScriptAction::AddReaction(clean_emoji.to_string()));
        }
        return String::new();
    }
    if let Some(role_name) = extract_role_arg(trimmed, '+') {
        let evaluated_role = parse_tags(&role_name, ctx, depth.saturating_add(1), collector);
        let clean = evaluated_role.trim();
        if !clean.is_empty() {
            collector
                .actions
                .push(TagScriptAction::AddRole(clean.to_string()));
        }
        return String::new();
    }
    if let Some(role_name) = extract_role_arg(trimmed, '-') {
        let evaluated_role = parse_tags(&role_name, ctx, depth.saturating_add(1), collector);
        let clean = evaluated_role.trim();
        if !clean.is_empty() {
            collector
                .actions
                .push(TagScriptAction::RemoveRole(clean.to_string()));
        }
        return String::new();
    }

    // 5. Silas AI Tag
    if let Some(rest) = trimmed.strip_prefix("ai:") {
        let prompt = parse_tags(rest, ctx, depth.saturating_add(1), collector);
        let clean = prompt.trim();
        if !clean.is_empty() {
            collector.ai_prompt = Some(clean.to_string());
        }
        return String::new();
    }

    // 6. Embed Builders
    if let Some(rest) = trimmed.strip_prefix("title:") {
        let title = parse_tags(rest, ctx, depth.saturating_add(1), collector);
        collector
            .embed
            .get_or_insert_with(TagScriptEmbed::default)
            .title = Some(title.trim().to_string());
        return String::new();
    }
    if let Some(rest) = trimmed.strip_prefix("description:") {
        let desc = parse_tags(rest, ctx, depth.saturating_add(1), collector);
        collector
            .embed
            .get_or_insert_with(TagScriptEmbed::default)
            .description = Some(desc.trim().to_string());
        return String::new();
    }
    if let Some(rest) = trimmed.strip_prefix("color:") {
        let color_str = parse_tags(rest, ctx, depth.saturating_add(1), collector);
        if let Some(col) = parse_color(&color_str) {
            collector
                .embed
                .get_or_insert_with(TagScriptEmbed::default)
                .color = Some(col);
        }
        return String::new();
    }
    if let Some(rest) = trimmed.strip_prefix("image:") {
        let url = parse_tags(rest, ctx, depth.saturating_add(1), collector);
        collector
            .embed
            .get_or_insert_with(TagScriptEmbed::default)
            .image_url = Some(url.trim().to_string());
        return String::new();
    }
    if let Some(rest) = trimmed.strip_prefix("thumbnail:") {
        let url = parse_tags(rest, ctx, depth.saturating_add(1), collector);
        collector
            .embed
            .get_or_insert_with(TagScriptEmbed::default)
            .thumbnail_url = Some(url.trim().to_string());
        return String::new();
    }
    if let Some(rest) = trimmed.strip_prefix("footer:") {
        let footer = parse_tags(rest, ctx, depth.saturating_add(1), collector);
        collector
            .embed
            .get_or_insert_with(TagScriptEmbed::default)
            .footer = Some(footer.trim().to_string());
        return String::new();
    }
    if let Some(rest) = trimmed.strip_prefix("field:") {
        let parts = split_top_level(rest, '|');
        let name = parts
            .first()
            .map(|s| parse_tags(s, ctx, depth.saturating_add(1), collector))
            .unwrap_or_default();
        let value = parts
            .get(1)
            .map(|s| parse_tags(s, ctx, depth.saturating_add(1), collector))
            .unwrap_or_default();
        let inline_raw = parts.get(2).map(|s| s.trim()).unwrap_or("false");
        let inline = matches!(inline_raw, "true" | "yes" | "1" | "inline");

        collector
            .embed
            .get_or_insert_with(TagScriptEmbed::default)
            .fields
            .push(TagScriptEmbedField {
                name: name.trim().to_string(),
                value: value.trim().to_string(),
                inline,
            });
        return String::new();
    }

    // 7. Context Variables
    match trimmed {
        "user" | "user.mention" => {
            if ctx.author_id != 0 {
                format!("<@{}>", ctx.author_id)
            } else {
                ctx.author_name.clone()
            }
        }
        "user.name" => ctx.author_name.clone(),
        "user.id" => {
            if ctx.author_id != 0 {
                ctx.author_id.to_string()
            } else {
                String::new()
            }
        }
        "user.avatar" => ctx.author_avatar.clone().unwrap_or_default(),
        "user.created_at" => ctx.author_created_at.clone().unwrap_or_default(),
        "server" => ctx.guild_name.clone().unwrap_or_default(),
        "server.id" => ctx
            .guild_id
            .map(|gid| gid.to_string())
            .unwrap_or_default(),
        "server.members" => ctx
            .guild_members
            .map(|m| m.to_string())
            .unwrap_or_default(),
        "channel" => {
            if let Some(cid) = ctx.channel_id {
                format!("<#{cid}>")
            } else if let Some(cname) = &ctx.channel_name {
                format!("#{cname}")
            } else {
                String::new()
            }
        }
        "channel.id" => ctx
            .channel_id
            .map(|cid| cid.to_string())
            .unwrap_or_default(),
        "channel.name" => ctx.channel_name.clone().unwrap_or_default(),
        "target" | "target.mention" => {
            if let Some(tid) = ctx.target_id {
                format!("<@{tid}>")
            } else if let Some(tname) = &ctx.target_name {
                tname.clone()
            } else {
                String::new()
            }
        }
        "target.name" => ctx.target_name.clone().unwrap_or_default(),
        "target.id" => ctx
            .target_id
            .map(|tid| tid.to_string())
            .unwrap_or_default(),
        "date" => Utc::now().format("%Y-%m-%d").to_string(),
        "time" => Utc::now().format("%H:%M:%S UTC").to_string(),
        other => {
            if let Some(custom) = ctx.custom_vars.get(other) {
                custom.clone()
            } else {
                // Keep unrecognized literal tag intact
                format!("{{{other}}}")
            }
        }
    }
}

fn eval_if_tag(
    trimmed: &str,
    ctx: &TagScriptContext,
    depth: usize,
    collector: &mut TagScriptCollector,
) -> String {
    let parts = split_top_level(trimmed, ':');
    if parts.len() < 2 {
        return String::new();
    }

    let cond_header = parts.first().copied().unwrap_or("");
    let branches_str = parts.get(1..).unwrap_or(&[]).join(":");

    let cond_expr = if let Some(stripped) = cond_header.strip_prefix("if(") {
        stripped.trim_end_matches(')').trim()
    } else if let Some(stripped) = cond_header.strip_prefix("if ") {
        stripped.trim()
    } else {
        ""
    };

    // Evaluate variables in the condition without firing actions
    let mut dummy_collector = TagScriptCollector::default();
    let evaluated_cond = parse_tags(
        cond_expr,
        ctx,
        depth.saturating_add(1),
        &mut dummy_collector,
    );

    let is_true = eval_condition(&evaluated_cond);
    let branches = split_top_level(&branches_str, '|');

    if is_true {
        if let Some(then_b) = branches.first() {
            parse_tags(then_b.trim(), ctx, depth.saturating_add(1), collector)
        } else {
            String::new()
        }
    } else if let Some(else_b) = branches.get(1) {
        parse_tags(else_b.trim(), ctx, depth.saturating_add(1), collector)
    } else {
        String::new()
    }
}

fn eval_condition(expr: &str) -> bool {
    let trimmed = expr.trim();
    let ops = ["!=", "==", ">=", "<=", ">", "<"];

    for op in ops {
        if let Some(pos) = trimmed.find(op) {
            let left_str = match trimmed.get(..pos) {
                Some(s) => s.trim().trim_matches(|c| c == '\'' || c == '"'),
                None => "",
            };
            let right_start = pos.saturating_add(op.len());
            let right_str = match trimmed.get(right_start..) {
                Some(s) => s.trim().trim_matches(|c| c == '\'' || c == '"'),
                None => "",
            };

            if let (Ok(l_num), Ok(r_num)) = (left_str.parse::<f64>(), right_str.parse::<f64>()) {
                return match op {
                    "==" => (l_num - r_num).abs() < f64::EPSILON,
                    "!=" => (l_num - r_num).abs() >= f64::EPSILON,
                    ">=" => l_num >= r_num,
                    "<=" => l_num <= r_num,
                    ">" => l_num > r_num,
                    "<" => l_num < r_num,
                    _ => false,
                };
            }

            return match op {
                "==" => left_str == right_str,
                "!=" => left_str != right_str,
                ">=" => left_str >= right_str,
                "<=" => left_str <= right_str,
                ">" => left_str > right_str,
                "<" => left_str < right_str,
                _ => false,
            };
        }
    }

    let lower = trimmed.to_lowercase();
    !matches!(lower.as_str(), "" | "0" | "false" | "null" | "no")
}

fn extract_role_arg(trimmed: &str, sign: char) -> Option<String> {
    let p1 = format!("role({sign}");
    if trimmed.starts_with(&p1) && trimmed.ends_with(')') {
        let inner = trimmed.get(p1.len()..trimmed.len().saturating_sub(1))?;
        return Some(inner.trim().to_string());
    }

    let p2 = format!("role: {sign}");
    if let Some(rest) = trimmed.strip_prefix(&p2) {
        return Some(rest.trim().to_string());
    }

    let p3 = format!("role:{sign}");
    if let Some(rest) = trimmed.strip_prefix(&p3) {
        return Some(rest.trim().to_string());
    }

    None
}

fn split_top_level(input: &str, delim: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth: usize = 0;
    let mut start: usize = 0;

    for (idx, ch) in input.char_indices() {
        if ch == '{' {
            depth = depth.saturating_add(1);
        } else if ch == '}' {
            depth = depth.saturating_sub(1);
        } else if ch == delim && depth == 0 {
            if let Some(part) = input.get(start..idx) {
                parts.push(part);
            }
            start = idx.saturating_add(ch.len_utf8());
        }
    }
    if let Some(part) = input.get(start..) {
        parts.push(part);
    }
    parts
}

fn parse_color(input: &str) -> Option<u32> {
    let s = input.trim().to_lowercase();
    match s.as_str() {
        "red" => Some(0xFF0000),
        "green" => Some(0x00FF00),
        "blue" => Some(0x0000FF),
        "yellow" => Some(0xFFFF00),
        "purple" => Some(0x800080),
        "blurple" => Some(0x5865F2),
        "orange" => Some(0xFFA500),
        "white" => Some(0xFFFFFF),
        "black" => Some(0x000000),
        "cyan" => Some(0x00FFFF),
        "gold" => Some(0xFFD700),
        _ => {
            let hex_clean = if let Some(stripped) = s.strip_prefix('#') {
                stripped
            } else if let Some(stripped) = s.strip_prefix("0x") {
                stripped
            } else {
                s.as_str()
            };
            if let Ok(val) = u32::from_str_radix(hex_clean, 16) {
                Some(val)
            } else {
                s.parse::<u32>().ok()
            }
        }
    }
}

fn truncate_to_max(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut end = max_len;
        while end > 0 && !s.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        match s.get(..end) {
            Some(part) => part.to_string(),
            None => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_ctx() -> TagScriptContext {
        let mut custom = HashMap::new();
        let _ = custom.insert("prefix".to_string(), "!".to_string());
        TagScriptContext {
            author_id: 1111,
            author_name: "Alice".to_string(),
            author_avatar: Some("https://example.com/avatar.png".to_string()),
            author_created_at: Some("2024-01-01".to_string()),
            target_id: Some(2222),
            target_name: Some("Bob".to_string()),
            guild_id: Some(3333),
            guild_name: Some("Silas Server".to_string()),
            guild_members: Some(42),
            channel_id: Some(4444),
            channel_name: Some("general".to_string()),
            custom_vars: custom,
        }
    }

    #[test]
    fn test_variable_interpolation() {
        let ctx = sample_ctx();
        let template = "Hello {user.name} ({user.id}) in {server} ({server.id}, {server.members} members)! Channel: {channel} Target: {target.name} Prefix: {prefix}";
        let output = process_tagscript(template, &ctx);
        assert_eq!(
            output.body,
            "Hello Alice (1111) in Silas Server (3333, 42 members)! Channel: <#4444> Target: Bob Prefix: !"
        );
    }

    #[test]
    fn test_embed_builder_tags() {
        let ctx = sample_ctx();
        let template = "Welcome text! {title: Rules of {server}} {description: Please be kind} {color: #5865f2} {field: Rule 1 | No spam | true}";
        let output = process_tagscript(template, &ctx);

        assert_eq!(output.body, "Welcome text!    ");
        assert!(output.embed.is_some());
        if let Some(embed) = output.embed {
            assert_eq!(embed.title.as_deref(), Some("Rules of Silas Server"));
            assert_eq!(embed.description.as_deref(), Some("Please be kind"));
            assert_eq!(embed.color, Some(0x5865F2));
            assert_eq!(embed.fields.len(), 1);
            if let Some(field) = embed.fields.first() {
                assert_eq!(field.name, "Rule 1");
                assert_eq!(field.value, "No spam");
                assert!(field.inline);
            }
        }
    }

    #[test]
    fn test_action_directives() {
        let ctx = sample_ctx();
        let template = "Command executed! {delete} {dm} {react: 👍} {role(+Member)} {role(-Muted)}";
        let output = process_tagscript(template, &ctx);

        assert_eq!(output.body, "Command executed!     ");
        assert_eq!(output.actions.len(), 5);
        assert!(output.actions.contains(&TagScriptAction::DeleteMessage));
        assert!(output.actions.contains(&TagScriptAction::SendDm));
        assert!(output.actions.contains(&TagScriptAction::AddReaction("👍".to_string())));
        assert!(output.actions.contains(&TagScriptAction::AddRole("Member".to_string())));
        assert!(output.actions.contains(&TagScriptAction::RemoveRole("Muted".to_string())));
    }

    #[test]
    fn test_if_condition_logic() {
        let ctx = sample_ctx();
        let t1 = "{if({user.id} == 1111): Hello Alice! | Who are you?}";
        assert_eq!(process_tagscript(t1, &ctx).body, "Hello Alice!");

        let t2 = "{if({user.id} == 9999): Hello Alice! | Who are you?}";
        assert_eq!(process_tagscript(t2, &ctx).body, "Who are you?");

        let t3 = "{if({server.members} > 10): Big server | Small server}";
        assert_eq!(process_tagscript(t3, &ctx).body, "Big server");
    }

    #[test]
    fn test_random_and_percentage_tags() {
        let ctx = sample_ctx();
        let template = "{100: guaranteed | never}";
        assert_eq!(process_tagscript(template, &ctx).body, "guaranteed");

        let t0 = "{0: never | always}";
        assert_eq!(process_tagscript(t0, &ctx).body, "always");

        let any_t = "{any: option}";
        assert_eq!(process_tagscript(any_t, &ctx).body, "option");
    }

    #[test]
    fn test_ai_directive() {
        let ctx = sample_ctx();
        let template = "Summarize this: {ai: Tell a funny joke about Rust}";
        let output = process_tagscript(template, &ctx);

        assert_eq!(output.body, "Summarize this: ");
        assert_eq!(output.ai_prompt.as_deref(), Some("Tell a funny joke about Rust"));
    }

    #[test]
    fn test_bounded_output_and_nesting() {
        let ctx = sample_ctx();
        // Deeply nested recursion test
        let nested = "{if(1 == 1): {if(1 == 1): {if(1 == 1): {if(1 == 1): {if(1 == 1): {if(1 == 1): deeply nested}}}}}}";
        let output = process_tagscript(nested, &ctx);
        assert!(!output.body.is_empty());

        // Output length clamping test
        let long_str = "a".repeat(5000);
        let output_long = process_tagscript(&long_str, &ctx);
        assert!(output_long.body.len() <= MAX_OUTPUT_LENGTH);
    }
}
