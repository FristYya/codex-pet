use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountStatus {
    Checking,
    Unavailable,
    LoggedOut,
    LoggingIn,
    LoggedIn,
    LoginFailed,
    Cancelled,
}

pub struct AccountMachine {
    status: AccountStatus,
    active_login_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginNotification {
    Completed { login_id: String },
    Failed { login_id: String },
    Cancelled { login_id: String },
    AccountUpdated,
}

impl LoginNotification {
    pub fn from_server_event(method: &str, params: &Value) -> Option<Self> {
        match method {
            "account/updated" => Some(Self::AccountUpdated),
            "account/login/completed" => {
                let login_id = params.get("loginId")?.as_str()?.to_owned();
                match params.get("success")?.as_bool()? {
                    true => Some(Self::Completed { login_id }),
                    false => Some(Self::Failed { login_id }),
                }
            }
            "account/login/failed" => Some(Self::Failed {
                login_id: params.get("loginId")?.as_str()?.to_owned(),
            }),
            "account/login/cancelled" => Some(Self::Cancelled {
                login_id: params.get("loginId")?.as_str()?.to_owned(),
            }),
            _ => None,
        }
    }

    pub fn login_id(&self) -> Option<&str> {
        match self {
            Self::Completed { login_id }
            | Self::Failed { login_id }
            | Self::Cancelled { login_id } => Some(login_id),
            Self::AccountUpdated => None,
        }
    }
}

impl Default for AccountMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl AccountMachine {
    pub fn new() -> Self {
        Self {
            status: AccountStatus::Checking,
            active_login_id: None,
        }
    }

    pub fn status(&self) -> AccountStatus {
        self.status
    }
    pub fn active_login_id(&self) -> Option<&str> {
        self.active_login_id.as_deref()
    }

    pub fn reconcile(&mut self, logged_in: bool) {
        self.active_login_id = None;
        self.status = if logged_in {
            AccountStatus::LoggedIn
        } else {
            AccountStatus::LoggedOut
        };
    }

    pub fn begin(&mut self, login_id: String) {
        self.active_login_id = Some(login_id);
        self.status = AccountStatus::LoggingIn;
    }

    pub fn complete(&mut self, login_id: &str, success: bool) -> bool {
        if self.active_login_id.as_deref() != Some(login_id) {
            return false;
        }
        self.active_login_id = None;
        self.status = if success {
            AccountStatus::Checking
        } else {
            AccountStatus::LoginFailed
        };
        true
    }

    pub fn cancel(&mut self, login_id: &str) -> bool {
        if self.active_login_id.as_deref() != Some(login_id) {
            return false;
        }
        self.active_login_id = None;
        self.status = AccountStatus::Cancelled;
        true
    }

    pub fn fail(&mut self) {
        self.active_login_id = None;
        self.status = AccountStatus::LoginFailed;
    }

    pub fn unavailable(&mut self) {
        self.active_login_id = None;
        self.status = AccountStatus::Unavailable;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn startup_restores_the_account_state_from_account_read() {
        let mut state = AccountMachine::new();
        state.reconcile(true);
        assert_eq!(state.status(), AccountStatus::LoggedIn);

        state.reconcile(false);
        assert_eq!(state.status(), AccountStatus::LoggedOut);
    }

    #[test]
    fn completion_only_applies_to_the_active_login() {
        let mut state = AccountMachine::new();
        state.begin("current".into());

        assert!(!state.complete("old", true));
        assert_eq!(state.status(), AccountStatus::LoggingIn);
        assert!(state.complete("current", true));
        assert_eq!(state.status(), AccountStatus::Checking);
    }

    #[test]
    fn login_notifications_require_the_active_login_id_and_ignore_duplicates() {
        let notification = LoginNotification::from_server_event(
            "account/login/completed",
            &json!({"loginId": "current", "success": true}),
        )
        .unwrap();
        assert_eq!(
            notification,
            LoginNotification::Completed {
                login_id: "current".into()
            }
        );

        let mut state = AccountMachine::new();
        state.begin("current".into());
        assert!(state.complete(notification.login_id().unwrap(), true));
        assert!(!state.complete(notification.login_id().unwrap(), true));
        assert!(!state.complete("stale", true));
    }

    #[test]
    fn failed_completion_does_not_count_as_success() {
        assert_eq!(
            LoginNotification::from_server_event(
                "account/login/completed",
                &json!({"loginId": "current", "success": false}),
            ),
            Some(LoginNotification::Failed {
                login_id: "current".into()
            })
        );
    }

    #[test]
    fn account_updated_has_no_login_id_and_requests_an_authoritative_read() {
        assert_eq!(
            LoginNotification::from_server_event(
                "account/updated",
                &json!({"authMode": "chatgpt"}),
            ),
            Some(LoginNotification::AccountUpdated)
        );
    }

    #[test]
    fn failed_and_cancelled_login_return_to_retryable_terminal_states() {
        let mut state = AccountMachine::new();
        state.begin("failed".into());
        assert!(state.complete("failed", false));
        assert_eq!(state.status(), AccountStatus::LoginFailed);

        state.begin("cancelled".into());
        assert!(state.cancel("cancelled"));
        assert_eq!(state.status(), AccountStatus::Cancelled);
    }
}
