//! Interactive button pagination manager (`sb_page:{uuid}:prev` / `sb_page:{uuid}:next`).

use dashmap::DashMap;
use serenity::all::{
    ButtonStyle, ComponentInteraction, Context, CreateActionRow, CreateButton,
    CreateInteractionResponse, CreateInteractionResponseMessage,
};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Maximum concurrent active paginated views in memory.
pub const MAX_ACTIVE_PAGINATIONS: usize = 500;

/// Time-to-live for an interactive paginated message (15 minutes).
pub const PAGINATION_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone)]
pub struct PaginatedView {
    pub id: Uuid,
    pub owner_user_id: u64,
    pub pages: Vec<String>,
    pub current_page: usize,
    pub created_at: Instant,
}

#[derive(Default)]
pub struct PaginationStore {
    views: DashMap<Uuid, PaginatedView>,
}

impl PaginationStore {
    pub fn new() -> Self {
        Self {
            views: DashMap::new(),
        }
    }

    /// Register a multi-page response and return `(initial_page_content, action_row)`.
    pub fn create_view(
        &self,
        owner_user_id: u64,
        pages: Vec<String>,
    ) -> Option<(String, Vec<CreateActionRow>)> {
        let first_page = pages.first()?.clone();
        if pages.len() <= 1 {
            return Some((first_page, Vec::new()));
        }

        self.evict_expired_or_overflow();

        let id = Uuid::new_v4();
        let total = pages.len();
        let view = PaginatedView {
            id,
            owner_user_id,
            pages,
            current_page: 0,
            created_at: Instant::now(),
        };
        self.views.insert(id, view);

        let rows = build_pagination_buttons(id, 0, total);
        Some((first_page, rows))
    }

    /// Handle a `ComponentInteraction` if its `custom_id` starts with `sb_page:`.
    /// Returns `true` if the interaction was handled by the pagination system.
    pub async fn handle_interaction(
        &self,
        ctx: &Context,
        interaction: &ComponentInteraction,
    ) -> bool {
        let custom_id = interaction.data.custom_id.as_str();
        let Some(rest) = custom_id.strip_prefix("sb_page:") else {
            return false;
        };

        let mut parts = rest.split(':');
        let Some(uuid_str) = parts.next() else {
            return false;
        };
        let Some(direction) = parts.next() else {
            return false;
        };
        let Ok(view_id) = Uuid::parse_str(uuid_str) else {
            return false;
        };

        self.evict_expired_or_overflow();

        let Some(mut entry) = self.views.get_mut(&view_id) else {
            let msg = CreateInteractionResponseMessage::new()
                .content("This paginated view has expired.")
                .ephemeral(true);
            let _ = interaction
                .create_response(&ctx.http, CreateInteractionResponse::Message(msg))
                .await;
            return true;
        };

        let caller_id = interaction.user.id.get();
        let is_admin = interaction
            .member
            .as_ref()
            .and_then(|m| m.permissions)
            .map(|p| p.administrator() || p.manage_guild())
            .unwrap_or(false);

        if caller_id != entry.owner_user_id && !is_admin {
            drop(entry);
            let msg = CreateInteractionResponseMessage::new()
                .content("Only the user who requested this response (or a server admin) can turn pages.")
                .ephemeral(true);
            let _ = interaction
                .create_response(&ctx.http, CreateInteractionResponse::Message(msg))
                .await;
            return true;
        }

        let total = entry.pages.len();
        if direction == "prev" && entry.current_page > 0 {
            entry.current_page = entry.current_page.saturating_sub(1);
        } else if direction == "next" && entry.current_page.saturating_add(1) < total {
            entry.current_page = entry.current_page.saturating_add(1);
        }

        let current_idx = entry.current_page;
        let page_text = entry
            .pages
            .get(current_idx)
            .cloned()
            .unwrap_or_default();
        drop(entry);

        let components = build_pagination_buttons(view_id, current_idx, total);
        let update_msg = CreateInteractionResponseMessage::new()
            .content(page_text)
            .components(components);
        let _ = interaction
            .create_response(&ctx.http, CreateInteractionResponse::UpdateMessage(update_msg))
            .await;

        true
    }

    fn evict_expired_or_overflow(&self) {
        let now = Instant::now();
        self.views
            .retain(|_, v| now.duration_since(v.created_at) < PAGINATION_TTL);

        if self.views.len() >= MAX_ACTIVE_PAGINATIONS {
            let oldest = self
                .views
                .iter()
                .min_by_key(|entry| entry.value().created_at)
                .map(|entry| *entry.key());
            if let Some(key) = oldest {
                self.views.remove(&key);
            }
        }
    }
}

pub fn build_pagination_buttons(
    id: Uuid,
    current_page: usize,
    total_pages: usize,
) -> Vec<CreateActionRow> {
    let prev_btn = CreateButton::new(format!("sb_page:{id}:prev"))
        .label("◀ Previous")
        .style(ButtonStyle::Secondary)
        .disabled(current_page == 0);

    let indicator_btn = CreateButton::new(format!("sb_page:{id}:noop"))
        .label(format!(
            "Page {} / {}",
            current_page.saturating_add(1),
            total_pages
        ))
        .style(ButtonStyle::Secondary)
        .disabled(true);

    let next_btn = CreateButton::new(format!("sb_page:{id}:next"))
        .label("Next ▶")
        .style(ButtonStyle::Primary)
        .disabled(current_page.saturating_add(1) >= total_pages);

    vec![CreateActionRow::Buttons(vec![
        prev_btn,
        indicator_btn,
        next_btn,
    ])]
}
