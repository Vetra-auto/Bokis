use leptos::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub user_id: String,
    pub username: String,
    pub display_name: String,
    pub roles: Vec<String>,
    pub company_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FiscalYearInfo {
    pub id: String,
    pub name: String,
    pub start_date: String,
    pub end_date: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppPhase {
    Loading,
    Setup,
    Login,
    Authenticated,
}

#[derive(Debug, Clone, Copy)]
pub struct AppContext {
    pub current_user: RwSignal<Option<UserInfo>>,
    pub active_fiscal_year: RwSignal<Option<FiscalYearInfo>>,
    pub toast_message: RwSignal<Option<String>>,
    pub phase: RwSignal<AppPhase>,
}

impl AppContext {
    pub fn new() -> Self {
        Self {
            current_user: RwSignal::new(None),
            active_fiscal_year: RwSignal::new(None),
            toast_message: RwSignal::new(None),
            phase: RwSignal::new(AppPhase::Loading),
        }
    }

    pub fn is_logged_in(&self) -> bool {
        self.current_user.get().is_some()
    }

    pub fn has_role(&self, role: &str) -> bool {
        self.current_user.get()
            .map(|u| u.roles.iter().any(|r| r == role))
            .unwrap_or(false)
    }

    pub fn can_write(&self) -> bool {
        self.has_role("admin") || self.has_role("bokforare")
    }
}
