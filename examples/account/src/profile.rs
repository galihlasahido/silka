//! The "Profile" tab: `text_field`, `text_area`, and `avatar` — the parts of
//! the form catalogue every settings screen already exercises elsewhere in
//! this repository, gathered on one page rather than proved for the first
//! time.

use silka_core::tree::CrossAlign;
use silka_core::view::{column, row, View};
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{
    avatar, button_variant, card_header, card_padded, field, form, spacer, text, text_area,
    text_field, ButtonVariant,
};

use crate::data;
use crate::state::AccountState;

/// The a11y name of the avatar row's "Change photo" button.
pub const CHANGE_PHOTO: &str = "Change photo";

/// The tab's content — no overlays: every control here draws inline.
pub fn section(t: &Theme, state: AccountState) -> View {
    column([identity_row(t, state), fields(t, state)])
        .spacing(t.space(5.0))
        .cross(CrossAlign::Stretch)
        .into()
}

/// The avatar, the name it stands for, and the (inert — there is no file
/// picker in this example) button to replace it.
fn identity_row(t: &Theme, state: AccountState) -> View {
    let name = state.name.get();
    card_padded([row([
        View::from(avatar(if name.trim().is_empty() { "?" } else { &name }).lg()),
        View::from(
            column([
                View::from(
                    text(if name.trim().is_empty() {
                        "Your name".to_string()
                    } else {
                        name
                    })
                    .size(t.typography.headline.size)
                    .weight(FontWeight::SEMIBOLD)
                    .color(t.color.label)
                    .single_line(),
                ),
                View::from(
                    text(data::SEED_EMAIL)
                        .size(t.typography.footnote.size)
                        .color(t.color.secondary_label)
                        .single_line(),
                ),
            ])
            .spacing(t.space(0.5))
            .cross(CrossAlign::Start),
        ),
        View::from(spacer()),
        View::from(button_variant(CHANGE_PHOTO, ButtonVariant::Secondary)),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)])
    .into()
}

/// The actual form: name, email, website, bio.
fn fields(_t: &Theme, state: AccountState) -> View {
    let email_value = state.email.get();
    let error = data::validate_email(&email_value);
    state.email_error.set_if_changed(error);

    let bio_value = state.bio.get();
    const BIO_MAX: usize = 280;

    card_padded([
        View::from(card_header("Personal information")),
        form([
            field(
                "Full name",
                text_field(state.name.get())
                    .label("Full name")
                    .placeholder("Full name")
                    .on_change(move |s| state.name.set(s.to_string())),
            )
            .required(true),
            field(
                "Email",
                text_field(email_value)
                    .label("Email")
                    .placeholder("you@example.com")
                    .on_change(move |s| state.email.set(s.to_string())),
            )
            .required(true)
            .error(error.map(str::to_string)),
            field(
                "Website",
                text_field(state.website.get())
                    .label("Website")
                    .placeholder("https://")
                    .on_change(move |s| state.website.set(s.to_string())),
            )
            .help("Optional — shown on your public profile."),
            field(
                "Bio",
                text_area(bio_value.clone())
                    .label("Bio")
                    .placeholder("A few words about you…")
                    .auto_grow(3, 6)
                    .on_change(move |s| {
                        let s = s.to_string();
                        if s.chars().count() <= BIO_MAX {
                            state.bio.set(s);
                        }
                    }),
            )
            .help(format!(
                "{}/{BIO_MAX} characters",
                bio_value.chars().count()
            )),
        ])
        .into(),
    ])
    .into()
}

#[cfg(test)]
mod tests {
    use silka_core::signals::Runtime;

    use super::*;

    #[test]
    fn an_invalid_email_is_flagged_and_recorded_for_the_save_button() {
        let rt = Runtime::new();
        let state = crate::state::AccountState::seed(&rt);
        silka_core::view::with_theme(Theme::default(), || {
            state.email.set("not-an-email".to_string());
            let _ = fields(&Theme::default(), state);
        });
        assert_eq!(state.email_error.get(), Some("Missing the @"));
    }

    #[test]
    fn a_valid_email_clears_the_error() {
        let rt = Runtime::new();
        let state = crate::state::AccountState::seed(&rt);
        silka_core::view::with_theme(Theme::default(), || {
            state.email.set("dian@example.com".to_string());
            let _ = fields(&Theme::default(), state);
        });
        assert_eq!(state.email_error.get(), None);
    }
}
