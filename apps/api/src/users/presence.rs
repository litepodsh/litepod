use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// How long after their last action someone still counts as online.
const ONLINE_WINDOW: Duration = Duration::from_secs(5 * 60);

#[derive(Clone)]
pub struct RecentAction {
    pub action: String,
    pub target: Option<String>,
    at: Instant,
}

// shortcut: process memory, so a restart empties it until each person's next action; move it to
// the ephemeral store if the API ever runs on more than one instance.
static ACTIONS: LazyLock<Mutex<HashMap<String, RecentAction>>> = LazyLock::new(Default::default);

fn actions() -> MutexGuard<'static, HashMap<String, RecentAction>> {
    ACTIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Only what someone actually does lands here (deployments, restarts, sign-ins), so an open tab
/// and its polling never count as being online.
pub fn on_audit(user_id: &str, action: &str, target: Option<&str>) {
    // An empty id is a non-user actor (SCIM).
    if user_id.is_empty() {
        return;
    }
    if action == "auth.logout" {
        actions().remove(user_id);
        return;
    }
    // Anyone can fail a sign-in on someone's account; that says nothing about its owner.
    if action.starts_with("auth.") && action != "auth.login" {
        return;
    }
    actions().insert(
        user_id.to_owned(),
        RecentAction {
            action: action.to_owned(),
            target: target.map(str::to_owned),
            at: Instant::now(),
        },
    );
}

/// What they last did, while it is recent enough to call them online.
pub fn status(user_id: &str) -> Option<RecentAction> {
    status_at(user_id, Instant::now())
}

fn status_at(user_id: &str, now: Instant) -> Option<RecentAction> {
    actions()
        .get(user_id)
        .filter(|recent| now.duration_since(recent.at) <= ONLINE_WINDOW)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_action_puts_someone_online_until_the_window_passes() {
        on_audit("presence-action", "deployment.triggered", Some("web"));
        let recent = status("presence-action").expect("online");
        assert_eq!(recent.action, "deployment.triggered");
        assert_eq!(recent.target.as_deref(), Some("web"));

        let later = Instant::now() + ONLINE_WINDOW + Duration::from_secs(1);
        assert!(status_at("presence-action", later).is_none());
    }

    #[test]
    fn signing_out_goes_offline_and_a_failed_sign_in_does_not_come_online() {
        on_audit("presence-logout", "auth.login", None);
        assert!(status("presence-logout").is_some());
        on_audit("presence-logout", "auth.logout", None);
        assert!(status("presence-logout").is_none());

        on_audit("presence-victim", "auth.login_failed", None);
        assert!(status("presence-victim").is_none());

        on_audit("", "user.created", None);
        assert!(status("").is_none());
    }
}
