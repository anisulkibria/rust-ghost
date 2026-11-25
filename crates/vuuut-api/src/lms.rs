//! LMS & Online Courses: what other parts of the site call.
//!
//! * [`on_order_paid`] — a paid store order whose products are linked to
//!   courses enrols the buyer. Called from `ecom::on_paid`, so it runs
//!   however the order became paid: card, a provider's webhook, staff
//!   marking it paid, or a free checkout. Safe to call twice.
//! * [`settings`] — the tool's settings, read the one way.

use crate::AppState;
use vuuut_core::lms::Settings;
use vuuut_db::repos::{ecom as shop, lms as repo};

pub fn settings(state: &AppState) -> Settings {
    Settings::read(|k| state.settings.get_str(k))
}

/// Enrol the buyer of `order` in every course one of its products unlocks.
/// The buyer becomes a member if they were not one (through the regular
/// purchase path, which sends the usual welcome). Returns how many courses
/// the buyer is now enrolled in through this order.
pub async fn on_order_paid(state: &AppState, order: &shop::Order) -> usize {
    if order.email.is_empty() {
        return 0;
    }
    let items = shop::order_items(&state.db, &order.id).await;
    let pids: Vec<String> = items.iter().filter_map(|i| i.product_id.clone()).collect();
    let courses = repo::courses_for_products(&state.db, &pids).await;
    if courses.is_empty() {
        return 0;
    }
    let name = order.address().name();
    let Some((member_id, created)) = crate::members::ensure_member_for_purchase(state, &order.email, Some(&name)).await else { return 0 };
    let mut n = 0;
    for c in &courses {
        match repo::enroll(&state.db, c, &member_id, "purchase", Some(&order.id)).await {
            Ok(_) => n += 1,
            Err(e) => tracing::error!("lms: enrolling {} in {} failed: {}", order.email, c.slug, e.message()),
        }
    }
    if n > 0 {
        let titles = courses.iter().map(|c| c.title.as_str()).collect::<Vec<_>>().join(", ");
        shop::add_event(&state.db, &order.id, "course", &format!("{} was enrolled in {titles}.", order.email), "").await;
        let s = settings(state);
        let dashboard = s.paths.public_path("/courses/dashboard/");
        let action = if created { "signup" } else { "signin" };
        if let Some(link) = crate::members::magic_link_for(state, &order.email, action, &dashboard).await {
            let (bg, email) = (state.clone(), order.email.clone());
            tokio::spawn(async move {
                let sent = if created { crate::members::send_signup_email(&bg, &email, &link).await } else { crate::members::send_signin_email(&bg, &email, &link).await };
                if let Err(e) = sent {
                    tracing::warn!("lms: course access email to {email} failed: {}", e.message());
                }
            });
        }
    }
    n
}
