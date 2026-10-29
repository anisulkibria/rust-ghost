//! The automations engine: the loop that walks members along a chain, and
//! the sender that delivers the emails on the way.
//!
//! Ghost drives this from a domain event plus an in-memory timer, waking on
//! each signup and at the moment the next step comes due. This port runs one
//! process serving one site, so the same job is a single task on a short
//! tick — the design the scheduled-post publisher next door already uses.
//! The tick is only a floor: [`request_poll`] nudges it awake the instant a
//! signup creates work, so an email with no wait in front of it goes out
//! immediately rather than up to a tick late.
//!
//! What the loop guarantees, and why each part is here:
//!
//! * **Nothing runs twice.** Steps are claimed with a lock before they run,
//!   and the terminal write is conditional on still holding that lock.
//! * **A member who stopped qualifying stops receiving.** Status and
//!   newsletter preference are re-read at send time, not at entry time —
//!   someone who signed up for a paid automation and then cancelled should
//!   not get day three.
//! * **A failed send is retried, then given up on.** Ten attempts ten minutes
//!   apart, matching Ghost, after which the step is marked failed and the run
//!   ends rather than looping forever on a bad address.

use ghost_core::automations::{self as rules, ActionKind};
use ghost_db::repos::automations as repo;
use ghost_db::repos::automations::StepToRun;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::AppState;

/// How often to look for due steps when nothing has asked us to.
///
/// Waits are measured in hours, so this only decides how late a step can be,
/// and the signup path wakes the loop directly for the one case where
/// promptness is visible to a reader.
const TICK: Duration = Duration::from_secs(20);

/// Set when something has created work. The loop checks it to skip its sleep.
static POLL_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Ask the loop to run now rather than at its next tick.
pub fn request_poll() {
    POLL_REQUESTED.store(true, Ordering::Relaxed);
}

/// Whether the automations system is switched on — `automations_enabled`,
/// set from Settings → Features. Stopped means stopped: no member is
/// enrolled and no due step is claimed, but nothing is cancelled, so turning
/// it back on resumes every run exactly where it stood.
///
/// Read through `get_str` rather than `get_bool`: `get_bool` calls a missing
/// row false, and the system must default to on — only an explicit "false"
/// stops it.
fn enabled(state: &AppState) -> bool {
    state.settings.get_str("automations_enabled").as_deref() != Some("false")
}

/// Start the engine. Returns immediately; the work happens on its own task
/// for the life of the process.
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        // One poll at boot: a restart may have left steps that came due while
        // the process was down.
        loop {
            poll_once(&state).await;
            // Sleep in short slices so a signup does not wait a whole tick.
            let slices = TICK.as_millis() / 250;
            for _ in 0..slices.max(1) {
                if POLL_REQUESTED.swap(false, Ordering::Relaxed) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
    });
}

/// Claim one batch of due steps and run them.
///
/// Public so the admin UI can force a pass after an edit, and so tests can
/// drive the engine without waiting on a timer.
pub async fn poll_once(state: &AppState) {
    // the system switch comes first: a stopped system claims nothing, so a
    // step that came due while it was off stays pending until it is on again
    if !enabled(state) {
        return;
    }
    let claimed = match repo::claim_steps(&state.db, repo::MAX_STEPS_PER_BATCH).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("automations: could not claim steps: {}", e.message());
            return;
        }
    };
    if claimed.steps.is_empty() {
        return;
    }
    tracing::debug!("automations: running {} step(s)", claimed.steps.len());
    for step in &claimed.steps {
        process_step(state, step).await;
    }
    // A full batch means there may be more already due.
    if claimed.steps.len() as i64 >= repo::MAX_STEPS_PER_BATCH {
        request_poll();
    }
}

/// The member facts a step needs in order to decide whether to act.
struct MemberSnapshot {
    id: String,
    uuid: String,
    email: String,
    name: Option<String>,
    status: String,
    /// `Some(_)` when the member has set the preference explicitly; `None`
    /// when it has never been touched.
    updates_preference: Option<bool>,
    newsletter_count: i64,
    tier_ids: Vec<String>,
}

impl MemberSnapshot {
    /// Ghost's `hasUpdatesAndAnnouncementsEnabled`: an explicit preference
    /// wins, and an untouched one falls back to "are they subscribed to
    /// anything at all". A member who unsubscribed from everything has
    /// effectively opted out of these too.
    fn wants_updates(&self) -> bool {
        self.updates_preference.unwrap_or(self.newsletter_count > 0)
    }
}

async fn load_member(state: &AppState, member_id: &str) -> Option<MemberSnapshot> {
    let row: (String, Option<String>, String, Option<String>, String, Option<bool>) =
        sqlx::query_as(
            "SELECT id, uuid, email, name, status, enable_updates_and_announcements \
             FROM members WHERE id = ? AND email_disabled=0 AND NOT EXISTS(SELECT 1 FROM email_suppressions es WHERE es.email=members.email)",
        )
        .bind(member_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()?;

    let newsletter_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM members_newsletters mn JOIN newsletters n ON n.id = mn.newsletter_id \
         WHERE mn.member_id = ? AND n.status = 'active'",
    )
    .bind(member_id)
    .fetch_one(&state.db)
    .await
    .unwrap_or(0);

    let tier_ids: Vec<String> =
        sqlx::query_scalar("SELECT product_id FROM members_products WHERE member_id = ? AND (expiry_at IS NULL OR datetime(expiry_at)>datetime('now'))")
            .bind(member_id)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();

    Some(MemberSnapshot {
        id: row.0,
        uuid: row.1.unwrap_or_default(),
        email: row.2,
        name: row.3,
        status: row.4,
        updates_preference: row.5,
        newsletter_count,
        tier_ids,
    })
}

/// Run one claimed step to a conclusion: act, advance, or close the run.
async fn process_step(state: &AppState, step: &StepToRun) {
    // The author may have paused the automation between the claim and now.
    if step.automation_status != "active" {
        let _ = repo::finish_step(&state.db, step, "automation disabled").await;
        return;
    }
    if step.step_attempts > repo::MAX_ATTEMPTS {
        tracing::warn!("automations: step {} exceeded max attempts", step.id);
        let _ = repo::finish_step(&state.db, step, "failed").await;
        return;
    }

    let Some(scope) = step
        .automation_trigger_tier_scope
        .as_deref()
        .and_then(rules::TriggerScope::parse)
    else {
        tracing::error!("automations: step {} has no usable trigger scope", step.id);
        let _ = repo::finish_step(&state.db, step, "failed").await;
        return;
    };

    // A deleted member leaves `member_id` null; there is nobody to continue.
    let Some(member_id) = step.member_id.as_deref() else {
        let _ = repo::finish_step(&state.db, step, "member unsubscribed").await;
        return;
    };
    let Some(member) = load_member(state, member_id).await else {
        tracing::warn!("automations: member {member_id} for step {} is gone", step.id);
        let _ = repo::finish_step(&state.db, step, "member unsubscribed").await;
        return;
    };

    // Re-check eligibility now, not at entry: the point of a multi-day chain
    // is that things change during it.
    if !still_qualifies(&member, scope, &step.automation_trigger_tier_ids) {
        let _ = repo::finish_step(&state.db, step, "member changed status").await;
        return;
    }

    match &step.kind {
        // A wait has already done its waiting — being claimed means
        // `ready_at` has passed.
        ActionKind::Wait { .. } => {}
        ActionKind::SendEmail { subject, lexical } => {
            if !member.wants_updates() {
                let _ = repo::finish_step(&state.db, step, "member unsubscribed").await;
                return;
            }
            match send_automation_email_for(state, &member, subject, lexical,&step.automation_action_revision_id,&step.id).await {
                Ok(()) => {
                    // Recording is best-effort: the email is already gone, and
                    // losing a statistic is not worth re-sending it.
                    if let Err(e) = repo::record_email_sent(
                        &state.db,
                        step,
                        &member.id,
                        &member.uuid,
                        &member.email,
                        member.name.as_deref(),
                        state.settings.get_bool("email_track_opens"),
                        state.settings.get_bool("email_track_clicks"),
                    )
                    .await
                    {
                        tracing::warn!(
                            "automations: sent step {} but could not record it: {}",
                            step.id,
                            e.message()
                        );
                    }
                }
                Err(e) => {
                    handle_failure(state, step, &e).await;
                    return;
                }
            }
        }
    }

    match repo::finish_and_enqueue_next(&state.db, step).await {
        // A next step that is due immediately — a send after a wait — should
        // not sit until the next tick.
        Ok(Some(ready_at)) if ready_at <= ghost_core::dates::now_db() => request_poll(),
        Ok(_) => {}
        Err(e) => tracing::warn!("automations: could not advance step {}: {}", step.id, e.message()),
    }
}

/// Whether the member still matches the automation's trigger.
fn still_qualifies(member: &MemberSnapshot, scope: rules::TriggerScope, tier_ids: &[String]) -> bool {
    if !rules::eligible_statuses(scope.member_status()).contains(&member.status.as_str()) {
        return false;
    }
    if scope != rules::TriggerScope::SelectedPaid {
        return true;
    }
    // A tier-scoped automation also requires the member to still hold one of
    // those tiers — downgrading should take them out of it.
    member.tier_ids.iter().any(|t| tier_ids.contains(t))
}

/// Retry a failed step, or give up on it.
async fn handle_failure(state: &AppState, step: &StepToRun, error: &str) {
    tracing::warn!("automations: step {} failed: {error}", step.id);
    if step.step_attempts >= repo::MAX_ATTEMPTS {
        tracing::warn!("automations: step {} exceeded max attempts", step.id);
        let _ = repo::finish_step(&state.db, step, "failed").await;
        return;
    }
    let retry_at = ghost_core::dates::to_db_string(
        &(chrono::Utc::now() + chrono::Duration::seconds(repo::RETRY_DELAY_SECS)),
    );
    let _ = repo::retry_step(&state.db, step, &retry_at).await;
}

// ---------------------------------------------------------------------------
// sending
// ---------------------------------------------------------------------------

/// Render and deliver one automation email.
///
/// The body is Lexical from the editor; it is rendered to HTML and dropped
/// into the newsletter shell, so an automation email carries the same header,
/// footer, typography and colours as the site's newsletter. A saved email
/// design overrides that shell using the immutable design referenced by the
/// run's action revision.
pub async fn send_automation_email(
    state: &AppState,
    member: &MemberSnapshotLike<'_>,
    subject: &str,
    lexical: &str,
) -> Result<(), String> {
    send_automation_email_with_design(state,member,subject,lexical,None).await
}
pub async fn send_automation_email_with_design(
    state:&AppState,member:&MemberSnapshotLike<'_>,subject:&str,lexical:&str,design_id:Option<&str>,
)->Result<(),String>{
    send_automation_email_tracked(state,member,subject,lexical,design_id,None).await
}
async fn send_automation_email_tracked(state:&AppState,member:&MemberSnapshotLike<'_>,subject:&str,lexical:&str,design_id:Option<&str>,revision:Option<(&str,&str)>)->Result<(),String>{
    let site_url = state.site_url();
    let title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    let timezone = state.settings.get_str("timezone").unwrap_or_else(|| "Etc/UTC".into());

    let newsletter = ghost_db::repos::newsletters::default_newsletter(&state.db)
        .await
        .ok()
        .flatten()
        .ok_or_else(|| {
            "No newsletter to send on. Create one in Settings → Newsletters.".to_string()
        })?;

    let newsletter=crate::automation_designs::apply(state,newsletter,design_id).await.map_err(|e|e.message())?;

    // The newsletter supplies the design, but not its post-shaped furniture.
    // An automation email has no post behind it, so the title block, byline,
    // "View in browser" link, feedback buttons and share button would all
    // point at the site root and ask about an article that does not exist.
    // Overriding a local copy leaves the stored newsletter untouched.
    let newsletter = ghost_db::repos::newsletters::Newsletter {
        show_post_title_section: false,
        show_feature_image: false,
        show_latest_posts: false,
        show_share_button: false,
        feedback_enabled: false,
        show_comment_cta: false,
        ..newsletter
    };

    let body_html =
        ghost_core::url::absolutize_html(&site_url, &ghost_core::models::segment_html(&ghost_render::lexical::to_html(lexical),Some(member.status)));
    // Derived from the rendered HTML rather than from the Lexical nodes: a
    // body written as a markdown card has no text nodes to walk, and the
    // rendered form is what the reader actually gets either way.
    let plaintext = ghost_mail::newsletter::plaintext(&body_html);

    let site_ctx = ghost_mail::newsletter::SiteContext {
        title: title.clone(),
        url: site_url.clone(),
        icon_url: state
            .settings
            .get_str("icon")
            .filter(|i| !i.is_empty())
            .map(|i| ghost_core::url::absolute(&site_url, &ghost_core::url::base_asset(&i))),
        locale: state.settings.get_str("locale").unwrap_or_else(|| "en".into()),
        direction: "ltr".into(),
        accent_color: state.settings.get_str("accent_color").unwrap_or_default(),
    };

    // An automation email is not a post: it has no page to read online, no
    // byline and no feature image. `email_only` is what suppresses the
    // "view in browser" and share affordances that would link nowhere.
    let post_ctx = ghost_mail::newsletter::PostContext {
        id: String::new(),
        title: subject.to_string(),
        url: site_url.clone(),
        html: body_html,
        plaintext,
        published_at: ghost_core::dates::short_date_in_zone(
            &ghost_core::dates::now_db(),
            &timezone,
        ),
        authors: title.clone(),
        email_only: true,
        ..Default::default()
    };

    let rendered = ghost_mail::newsletter::NewsletterEmail {
        site: &site_ctx,
        newsletter: &newsletter,
        post: &post_ctx,
        latest_posts: &[],
        // The comment CTA points at a post. There isn't one.
        comments_enabled: false,
    };
    let html = rendered
        .render()
        .map_err(|e| format!("could not render the automation email: {}", e.message()))?;
    let html=match revision {Some((revision,send_id))=>crate::automation_tracking::instrument_for_send(state,revision,member.uuid,send_id,&html).await.map_err(|e|e.message())?,None=>html};
    let text = ghost_mail::newsletter::plaintext(&html);

    // The unsubscribe link lands on the updates-and-announcements preference
    // rather than a newsletter, because that is the switch that governs
    // these. Same HMAC the members API verifies.
    let key = crate::members::newsletter_key(state, member.uuid);
    let unsubscribe_url = format!(
        "{site_url}/unsubscribe/?uuid={}&key={key}&updatesAndAnnouncements=1",
        member.uuid
    );
    let recipient = ghost_mail::newsletter::Recipient {
        uuid: member.uuid.to_string(),
        email: member.email.to_string(),
        name: member.name.map(str::to_string).filter(|n| !n.trim().is_empty()),
        status: member.status.to_string(),
        created_at: String::new(),
        unsubscribe_url: unsubscribe_url.clone(),
        manage_account_url: format!("{site_url}/#/portal/account"),
        key,
    };
    let html = ghost_mail::newsletter::personalize(&html, &recipient);
    let text = ghost_mail::newsletter::personalize(&text, &recipient);

    let mut cfg = ghost_mail::MailConfig::from_settings(&state.settings, &title, &site_url);
    if let Some(name)=state.settings.get_str("automated_email_sender_name").filter(|s|!s.is_empty()){cfg.from_name=name;}
    if let Some(email)=state.settings.get_str("automated_email_sender_email").filter(|s|!s.is_empty()){cfg.from_address=email;}
    if let Some(reply)=state.settings.get_str("automated_email_sender_reply_to").filter(|s|!s.is_empty()){cfg.reply_to=Some(reply);}
    if let Some(id)=design_id {
        if let Ok(Some(raw))=sqlx::query_scalar::<_,String>("SELECT settings FROM email_design_settings WHERE id=?").bind(id).fetch_optional(&state.db).await{
            if let Ok(values)=serde_json::from_str::<serde_json::Value>(&raw){
                if let Some(v)=values["sender_name"].as_str().filter(|s|!s.is_empty()){cfg.from_name=v.into();}
                if let Some(v)=values["sender_email"].as_str().filter(|s|!s.is_empty()){cfg.from_address=v.into();}
                if let Some(v)=values["sender_reply_to"].as_str().filter(|s|!s.is_empty()){cfg.reply_to=Some(v.into());}
            }
        }
    }
    let msg = ghost_mail::Message {
        to: member.email.to_string(),
        subject: subject.to_string(),
        html,
        text,
    };

    // Mailgun when it is configured, since that is what the newsletter uses
    // and an automation email is the same kind of mail to the same people.
    if let Some(mut mg) = ghost_mail::MailgunConfig::from_settings(&state.settings, &cfg) {
        mg.from_address=cfg.from_address.clone();mg.from_name=cfg.from_name.clone();
        return ghost_mail::send_mailgun_with_reply(&mg, &msg,cfg.reply_to.as_deref()).await.map(|_| ()).map_err(|e| e.message());
    }
    ghost_mail::send(&cfg, &msg).await.map_err(|e| e.message())
}

/// The recipient fields [`send_automation_email`] needs, borrowed.
///
/// A small struct rather than four arguments so the preview path and the
/// poller cannot pass them in different orders.
pub struct MemberSnapshotLike<'a> {
    pub uuid: &'a str,
    pub email: &'a str,
    pub name: Option<&'a str>,
    pub status: &'a str,
}

impl MemberSnapshot {
    fn borrowed(&self) -> MemberSnapshotLike<'_> {
        MemberSnapshotLike {
            uuid: &self.uuid,
            email: &self.email,
            name: self.name.as_deref(),
            status: &self.status,
        }
    }
}

/// Convenience wrapper so the poller can hand over its owned snapshot.
async fn send_automation_email_for(
    state: &AppState,
    member: &MemberSnapshot,
    subject: &str,
    lexical: &str,
    revision_id:&str,
    send_id:&str,
) -> Result<(), String> {
    let design_id:Option<String>=sqlx::query_scalar("SELECT email_design_setting_id FROM automation_action_revisions WHERE id=?").bind(revision_id).fetch_one(&state.db).await.map_err(|e|e.to_string())?;
    send_automation_email_tracked(state, &member.borrowed(), subject, lexical,design_id.as_deref(),Some((revision_id,send_id))).await
}

// ---------------------------------------------------------------------------
// the signup trigger
// ---------------------------------------------------------------------------

/// Start any automation this member now qualifies for.
///
/// Called after a signup and after a move to paid. Failures are logged rather
/// than surfaced: a member whose signup succeeded must not see an error
/// because an automation could not start, and the next poll will not
/// retroactively enrol them — which is the correct trade, since a missed
/// enrolment is recoverable by hand and a failed signup is not.
pub async fn trigger_for_member(state: &AppState, member_id: &str) {
    if !enabled(state) {
        return;
    }
    let Some(member) = load_member(state, member_id).await else {
        return;
    };
    match repo::trigger(
        &state.db,
        &member.id,
        &member.email,
        &member.status,
        &member.tier_ids,
    )
    .await
    {
        Ok(0) => {}
        Ok(n) => {
            tracing::info!("automations: started {n} automation(s) for member {member_id}");
            // The first step of a chain that opens with an email is due now.
            request_poll();
        }
        Err(e) => tracing::warn!(
            "automations: could not start automations for member {member_id}: {}",
            e.message()
        ),
    }
}

/// An automation is only worth polling for if one exists and is active.
pub async fn any_active(state: &AppState) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM automations WHERE status = 'active'")
        .fetch_one(&state.db)
        .await
        .map(|n| n > 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(status: &str, preference: Option<bool>, newsletters: i64, tiers: &[&str]) -> MemberSnapshot {
        MemberSnapshot {
            id: "m1".into(),
            uuid: "u1".into(),
            email: "a@b.test".into(),
            name: None,
            status: status.into(),
            updates_preference: preference,
            newsletter_count: newsletters,
            tier_ids: tiers.iter().map(|t| t.to_string()).collect(),
        }
    }

    #[test]
    fn an_explicit_updates_preference_wins_over_newsletters() {
        // explicitly on, even with no newsletters
        assert!(member("free", Some(true), 0, &[]).wants_updates());
        // explicitly off, even while subscribed to newsletters
        assert!(!member("free", Some(false), 3, &[]).wants_updates());
    }

    #[test]
    fn an_unset_preference_falls_back_to_newsletter_subscriptions() {
        assert!(member("free", None, 1, &[]).wants_updates());
        // unsubscribed from everything counts as opted out
        assert!(!member("free", None, 0, &[]).wants_updates());
    }

    #[test]
    fn eligibility_is_rechecked_against_the_trigger_scope() {
        use rules::TriggerScope::*;
        // a free automation stops for someone who upgraded
        assert!(still_qualifies(&member("free", None, 1, &[]), Free, &[]));
        assert!(!still_qualifies(&member("paid", None, 1, &[]), Free, &[]));
        // a paid automation stops for someone who cancelled
        assert!(still_qualifies(&member("paid", None, 1, &[]), AllPaid, &[]));
        assert!(!still_qualifies(&member("free", None, 1, &[]), AllPaid, &[]));
        // comped and gift still count as paid
        assert!(still_qualifies(&member("comped", None, 1, &[]), AllPaid, &[]));
        assert!(still_qualifies(&member("gift", None, 1, &[]), AllPaid, &[]));
    }

    #[test]
    fn a_tier_scoped_automation_also_checks_the_tier() {
        use rules::TriggerScope::SelectedPaid;
        let tiers = vec!["t1".to_string(), "t2".to_string()];
        assert!(still_qualifies(&member("paid", None, 1, &["t2"]), SelectedPaid, &tiers));
        // still paid, but moved to a tier the automation does not cover
        assert!(!still_qualifies(&member("paid", None, 1, &["t9"]), SelectedPaid, &tiers));
        assert!(!still_qualifies(&member("paid", None, 1, &[]), SelectedPaid, &tiers));
    }

    #[test]
    fn a_poll_request_is_consumed_once() {
        POLL_REQUESTED.store(false, Ordering::Relaxed);
        request_poll();
        assert!(POLL_REQUESTED.swap(false, Ordering::Relaxed));
        assert!(!POLL_REQUESTED.swap(false, Ordering::Relaxed));
    }
}

pub async fn preview_email(state:&AppState,subject:&str,lexical:&str,design:Option<&str>)->Result<String,String>{
    let newsletter=ghost_db::repos::newsletters::default_newsletter(&state.db).await.map_err(|e|e.message())?.ok_or("Create a newsletter first")?;
    let newsletter=crate::automation_designs::apply(state,newsletter,design).await.map_err(|e|e.message())?;
    let newsletter=ghost_db::repos::newsletters::Newsletter{show_post_title_section:false,show_feature_image:false,show_latest_posts:false,show_share_button:false,feedback_enabled:false,show_comment_cta:false,..newsletter};
    let site=ghost_mail::newsletter::SiteContext{title:state.settings.site_name(),url:state.site_url(),icon_url:state.settings.get_str("icon"),locale:state.settings.get_str("locale").unwrap_or_else(||"en".into()),direction:"ltr".into(),accent_color:state.settings.get_str("accent_color").unwrap_or_default()};
    let post=ghost_mail::newsletter::PostContext{title:subject.into(),html:ghost_core::url::absolutize_html(&state.site_url(),&ghost_render::lexical::to_html(lexical)),email_only:true,..Default::default()};
    ghost_mail::newsletter::NewsletterEmail{site:&site,newsletter:&newsletter,post:&post,latest_posts:&[],comments_enabled:false}.render().map_err(|e|e.message())
}
