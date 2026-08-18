//! The top bar: the page title, the appearance toggle, notifications, and the
//! user menu.
//!
//! The user menu is the piece worth pointing at. It is
//! [`silka_widgets::menu()`], which rides the overlay system
//! ([`silka_widgets::overlay()`]): the panel flips above the chip when the window
//! is short, flips to the other side at the right edge, closes on an outside
//! click and on Esc, and animates in and out on a spring — and **not one
//! coordinate of that is computed here**. The application's whole contribution
//! is a list of rows and a trigger.

use silka_core::signals::Signal;
use silka_core::tree::CrossAlign;
use silka_core::view::{column, row, View};
use silka_text::FontWeight;
use silka_theme::{Appearance, Theme};
use silka_widgets::menu::{item, menu, separator, MenuEntry, MenuState};
use silka_widgets::overlay::OverlayBuilder;
use silka_widgets::{divider, spacer, text, IconName};

use crate::kit;
use crate::nav::{Page, USER_EMAIL, USER_NAME};

/// The a11y name of the appearance button when the application is light.
pub const TO_DARK: &str = "Switch to dark mode";
/// …and when it is dark.
pub const TO_LIGHT: &str = "Switch to light mode";
/// The a11y name of the notifications button.
pub const NOTIFICATIONS: &str = "Notifications, 3 unread";
/// The a11y name of the user menu.
pub const USER_MENU: &str = "Account menu";

/// The last thing the account menu was asked to do.
///
/// A newtype rather than a bare `&'static str` because
/// [`Env`](silka_core::app::Env) is keyed by type: a second `Signal<&str>`
/// anywhere in the application would collide with this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LastAccountAction(pub &'static str);

/// The rows of the user menu.
///
/// The first row is the identity block — disabled, because it is information
/// rather than a command, and a screen reader should say so.
pub fn account_entries() -> Vec<MenuEntry> {
    vec![
        item("account.identity", format!("{USER_NAME} · {USER_EMAIL}"))
            .enabled(false)
            .into(),
        separator(),
        item("account.profile", "Profile").into(),
        item("account.security", "Security").into(),
        separator(),
        item("account.logout", "Logout").into(),
    ]
}

/// The top bar, plus the overlay panels its menu needs.
///
/// The panels are handed back rather than mounted here: they belong in the
/// application's single [`silka_widgets::overlay_layer`], because *that* order
/// is the stacking order for the whole window.
pub struct TopBar {
    /// The bar itself.
    pub view: View,
    /// The menu panels, outermost first.
    pub overlays: Vec<OverlayBuilder>,
}

/// Build the top bar.
pub fn top_bar(
    t: &Theme,
    page: Page,
    menu_state: Signal<MenuState>,
    last_action: Signal<LastAccountAction>,
    on_toggle_appearance: impl Fn() + 'static,
) -> TopBar {
    let dark = t.appearance == Appearance::Dark;
    // The symbol says where pressing goes, not where the application is — and
    // it is a real icon now, so the a11y name is the sentence beside it rather
    // than whatever a screen reader makes of a text glyph.
    let (symbol, label) = if dark {
        (IconName::Sun, TO_LIGHT)
    } else {
        (IconName::Moon, TO_DARK)
    };

    let account = menu(account_entries())
        .label(USER_MENU)
        .key("account-menu")
        .chip(true)
        .bind(menu_state)
        .on_activate(move |id| {
            // Static ids so the tests can assert what was chosen without
            // leaking a `String` into application state.
            let chosen = match id {
                "account.profile" => "profile",
                "account.security" => "security",
                "account.logout" => "logout",
                _ => "none",
            };
            last_action.set(LastAccountAction(chosen));
        });

    let chip = row([
        kit::avatar(t, USER_NAME, t.space(7.0)),
        text(USER_NAME)
            .size(t.typography.callout.size)
            .weight(FontWeight::MEDIUM)
            .color(t.color.label)
            .single_line()
            .into(),
    ])
    .spacing(t.space(2.0))
    .cross(CrossAlign::Center);

    let view = row([
        text(page.short_title())
            .size(t.typography.title3.size)
            .weight(FontWeight::SEMIBOLD)
            .tracking(t.typography.title3.tracking)
            .color(t.color.label)
            .single_line()
            .into(),
        // The spacer that pushes the controls to the far side — the Tier 0
        // component, so the layout engine owns the gap.
        View::from(spacer()),
        kit::icon_button(t, symbol, label, on_toggle_appearance),
        kit::icon_button(t, IconName::Bell, NOTIFICATIONS, || {}),
        account.trigger_with(chip),
    ])
    .spacing(t.space(2.0))
    .cross(CrossAlign::Center)
    .px_6()
    .py_2()
    .bg(silka_theme::ColorToken::Surface);

    TopBar {
        view: column([View::from(view), divider().into()])
            .cross(CrossAlign::Stretch)
            .into(),
        overlays: account.overlays(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_account_menu_has_identity_profile_security_and_logout() {
        let entries = account_entries();
        let labels: Vec<String> = entries
            .iter()
            .filter_map(|e| e.item().map(|i| i.label().to_string()))
            .collect();
        assert!(labels[0].contains(USER_NAME) && labels[0].contains(USER_EMAIL));
        assert!(labels.iter().any(|l| l == "Profile"));
        assert!(labels.iter().any(|l| l == "Security"));
        assert!(labels.iter().any(|l| l == "Logout"));
    }

    #[test]
    fn the_identity_row_is_not_a_command() {
        let entries = account_entries();
        let identity = entries[0].item().expect("the first row is an item");
        assert!(
            !identity.is_enabled(),
            "the identity block must not be activatable — it is information"
        );
    }

    #[test]
    fn the_appearance_button_names_where_it_goes_not_where_it_is() {
        // The a11y name of a toggle has to say what pressing it does; "Dark
        // mode" alone leaves a screen reader user guessing.
        assert!(TO_DARK.contains("dark"));
        assert!(TO_LIGHT.contains("light"));
        assert_ne!(TO_DARK, TO_LIGHT);
    }
}
