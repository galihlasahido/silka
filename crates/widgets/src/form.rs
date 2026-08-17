//! `label()`, `field()` and `form()` — the Tier 2 form layout (`KOMPONEN.md`:
//! "grid label-kanan/kontrol-kiri ala macOS Settings; validasi + pesan error").
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_widgets::{field, form, text_field};
//! # let rt = Runtime::new();
//! let email = rt.signal(String::new());
//!
//! // Validation is a value, not a mechanism: the application decides what is
//! // wrong and the field is simply told.
//! let complaint = (!email.get().contains('@')).then(|| String::from("Not a valid address"));
//!
//! form([
//!     field("Email", text_field(email.get()).label("Email"))
//!         .required(true)
//!         .error(complaint),
//! ]);
//! ```
//!
//! ## What a form actually is
//!
//! Two columns and a shared width. That sounds trivial and is exactly the part
//! every application re-invents slightly differently, ending up with labels
//! that do not line up between two sections of the same window. So the width is
//! computed **once, for the whole form**, by measuring every label through the
//! same text engine that will draw them ([`Form::label_width`]) — not guessed
//! at by the caller, and not left to a grid that would let each row size its own
//! first column.
//!
//! The rest is the vocabulary a real form needs and a `row` does not have:
//!
//! - a **required** marker that is part of the label rather than typed into it;
//! - a **help** line under the control, in the secondary colour;
//! - an **error** line that replaces the help line, in the destructive colour;
//! - one [`AccessRole::Group`] per field, so a screen reader hears the question
//!   before the answer.
//!
//! ## The label column
//!
//! Ends at the reading edge by default (`MainAlign::End`) — the macOS Settings
//! shape, and the one that keeps the eye on the boundary between question and
//! answer. `.label_align(MainAlign::Start)` gives the web/shadcn shape instead;
//! both mirror in an RTL document without a single coordinate changing, because
//! [`MainAlign`] is reading-relative (§9.8).
//!
//! Each label sits in a band at least [`MIN_HIT_TARGET`] tall and is centred in
//! it, which is what makes it line up with a text field, a select or a stepper —
//! all of which are that tall — instead of floating above them.
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! A form is a **layout**, not a control, and the lines of the Definition of
//! Done that concern controls are answered by saying so rather than pretending:
//!
//! | Line | How it is met |
//! |---|---|
//! | Both presets | every value flows through [`FormStyle::from_theme`]; no literal in this module |
//! | Dark mode | the same answer — tokens move with the appearance |
//! | Interactive states on springs | none exist: a label cannot be hovered, pressed or focused. The **controls inside** keep every state they always had |
//! | Keyboard + focus ring | the form adds no Tab stop of its own; Tab walks the controls in source order |
//! | AccessKit node | [`AccessRole::Group`] per field carrying the label, [`AccessRole::Label`] for the label and for the message |
//! | Hit target ≥ 44pt | not applicable to the layout; the label band is that tall so the controls line up with it |
//! | Reduced motion | nothing here moves |
//!
//! ## Acknowledged debt: there is no `describedby`
//!
//! An error message is drawn under its control and announced as text next to
//! it, because [`silka_core::access::AccessNode`] has no relation that would
//! attach it *to* the control the way ARIA's `aria-describedby` and
//! `aria-invalid` do. A screen reader therefore reads the message when it
//! reaches it, not when it reaches the field. Recorded here rather than hidden:
//! the fix is a relation in the a11y vocabulary, not a workaround in this file.

use silka_core::access::AccessRole;
use silka_core::input::FocusPolicy;
use silka_core::signals::Key;
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use silka_core::view::{column, constrained, expanded, interactive, row, View};
use silka_paint::Color;
use silka_text::{FontWeight, TextConstraints, TextStyle};
use silka_theme::{SpaceToken, Theme};

use crate::button::MIN_HIT_TARGET;
use crate::fonts::Fonts;
use crate::text::text_in;

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

/// Every paint value of a form, **already resolved** from theme tokens.
///
/// ```
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::FormStyle;
///
/// let light = FormStyle::from_theme(&Theme::cupertino(Appearance::Light));
/// let dark = FormStyle::from_theme(&Theme::cupertino(Appearance::Dark));
///
/// // An error is a different colour from a hint, and both move with the
/// // appearance — a message that survives dark mode is a hard-coded message.
/// assert_ne!(light.error, light.help);
/// assert_ne!(light.label, dark.label);
///
/// // The label column has a floor and a ceiling: without the ceiling one long
/// // question squeezes every answer in the form.
/// assert!(light.min_label_width < light.max_label_width);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FormStyle {
    /// Text size of a label.
    pub label_size: f32,
    /// Line height factor of a label.
    pub label_line_height: f32,
    /// Weight of a label.
    pub label_weight: FontWeight,
    /// Text size of a help or error line.
    pub message_size: f32,
    /// Line height factor of a message.
    pub message_line_height: f32,
    /// Gap between the label column and the control column.
    pub label_gap: f32,
    /// Gap between two fields.
    pub field_gap: f32,
    /// Gap between a control and its message.
    pub message_gap: f32,
    /// Gap between a label and its required marker.
    pub marker_gap: f32,
    /// The label column never gets narrower than this.
    pub min_label_width: f32,
    /// …nor wider, whatever a long question asks for.
    pub max_label_width: f32,
    /// Minimum height of a label's band, so labels line up with the controls.
    pub label_band: f32,

    /// Colour of a label.
    pub label: Color,
    /// Colour of a label whose field is disabled.
    pub disabled_label: Color,
    /// Colour of the required marker.
    pub marker: Color,
    /// Colour of a help line.
    pub help: Color,
    /// Colour of an error line.
    pub error: Color,
}

impl FormStyle {
    /// The defaults taken from `theme`.
    pub fn from_theme(theme: &Theme) -> Self {
        let c = &theme.color;
        Self {
            label_size: theme.typography.body_size,
            label_line_height: theme.typography.body_line_height,
            label_weight: FontWeight::MEDIUM,
            message_size: theme.typography.footnote.size,
            message_line_height: theme.typography.footnote.line_height,
            label_gap: theme.space(4.0),
            field_gap: theme.space(4.0),
            message_gap: theme.space(1.0),
            marker_gap: theme.space(0.5),
            min_label_width: theme.space(16.0),
            max_label_width: theme.space(48.0),
            label_band: MIN_HIT_TARGET,

            label: c.label,
            disabled_label: c.disabled_label,
            // The marker is destructive-coloured rather than accent-coloured:
            // "you must" is a warning, not a highlight.
            marker: c.destructive,
            help: c.secondary_label,
            error: c.destructive,
        }
    }

    /// The text style a label is measured and drawn with.
    ///
    /// One function, used by **both** the measuring in [`Form::label_width`]
    /// and the drawing in [`FormLabel`] — a column width computed from a
    /// different style than the one that ends up on screen is a column that is
    /// always slightly wrong.
    pub fn label_style(&self) -> TextStyle {
        TextStyle::new()
            .size(self.label_size)
            .line_height(self.label_line_height)
            .weight(self.label_weight)
            .single_line()
    }
}

// ---------------------------------------------------------------------------
// Label
// ---------------------------------------------------------------------------

/// Dart-style form label builder (§2.5).
///
/// It is a label in the a11y sense too — [`AccessRole::Label`], the role static
/// text carries — which is what distinguishes it from a bare [`crate::text()`]
/// that happens to sit next to a control.
#[derive(Debug, Clone)]
pub struct FormLabel {
    fonts: Fonts,
    style: FormStyle,
    text: String,
    required: bool,
    disabled: bool,
    align: MainAlign,
    /// Whether the label announces itself as a name of its own.
    ///
    /// True for a label written on its own; **false** for the one [`field`]
    /// puts in its label column, because the group around that field already
    /// carries the same words. Announcing them twice is how a screen reader
    /// ends up saying "Email, group, Email".
    announce: bool,
    key: Option<Key>,
}

/// The text that names a control — the `label` component (`KOMPONEN.md`
/// Tier 2).
///
/// ```
/// use silka_widgets::label;
///
/// let name = label("Full name").required(true);
/// # let _ = name;
/// ```
///
/// A label written here is **drawn**; the name a screen reader announces for
/// the control still comes from the control itself (see the module docs on the
/// missing `describedby` relation). [`field`] does both at once.
///
/// Use [`label_in`] outside a build pass.
pub fn label(text: impl Into<String>) -> FormLabel {
    label_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        text,
    )
}

/// [`label`] with the text engine and the theme passed explicitly.
pub fn label_in(fonts: &Fonts, theme: &Theme, text: impl Into<String>) -> FormLabel {
    FormLabel {
        fonts: fonts.clone(),
        style: FormStyle::from_theme(theme),
        text: text.into(),
        required: false,
        disabled: false,
        align: MainAlign::Start,
        announce: true,
        key: None,
    }
}

impl FormLabel {
    /// Whether the label carries a name of its own in the a11y tree.
    ///
    /// On by default. [`field`] turns it **off** for the label in its column:
    /// the group around the field already says those words, and a name that is
    /// announced twice is worse than one that is announced once.
    pub fn announce(mut self, announce: bool) -> Self {
        self.announce = announce;
        self
    }

    /// Mark the field as required: an asterisk in the destructive colour.
    ///
    /// A marker rather than a word typed into the label, so that a form can be
    /// translated without every string growing a punctuation convention.
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Dim the label because its control is unusable.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Which edge the label is packed against — reading-relative (§9.8).
    pub fn align(mut self, align: MainAlign) -> Self {
        self.align = align;
        self
    }

    /// Custom paint values.
    pub fn style(mut self, style: FormStyle) -> Self {
        self.style = style;
        self
    }

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The text drawn.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The width this label needs, measured through the engine that will draw
    /// it.
    pub fn measured_width(&self) -> f32 {
        measure(&self.fonts, &self.style, &self.text)
    }
}

impl From<FormLabel> for View {
    fn from(l: FormLabel) -> View {
        let s = l.style;
        let color = if l.disabled {
            s.disabled_label
        } else {
            s.label
        };
        let mut parts: Vec<View> = Vec::with_capacity(2);
        parts.push(
            text_in(&l.fonts, &l.text)
                .size(s.label_size)
                .line_height(s.label_line_height)
                .weight(s.label_weight)
                .color(color)
                .role(if l.announce {
                    AccessRole::Label
                } else {
                    AccessRole::Container
                })
                .into(),
        );
        if l.required {
            parts.push(
                text_in(&l.fonts, "*")
                    .size(s.label_size)
                    .line_height(s.label_line_height)
                    .weight(s.label_weight)
                    .color(s.marker)
                    // The asterisk is not a word: announcing "asterisk" after
                    // every required field is noise, and the requirement is
                    // carried by the control instead.
                    .role(AccessRole::Container)
                    .into(),
            );
        }
        let mut b = row(parts)
            .spacing(s.marker_gap)
            .main(l.align)
            .cross(CrossAlign::Center);
        if let Some(key) = l.key {
            b = b.key(key);
        }
        b.into()
    }
}

// ---------------------------------------------------------------------------
// Field
// ---------------------------------------------------------------------------

/// One row of a [`form`]: a label, a control, and at most one message.
///
/// Not `Clone`, because a [`View`] is not: a field owns the control it was
/// given, which is exactly what stops the same control being mounted twice.
pub struct FormField {
    label: String,
    control: View,
    error: Option<String>,
    help: Option<String>,
    required: bool,
    disabled: bool,
    key: Option<Key>,
}

/// One labelled control.
///
/// ```
/// use silka_widgets::{field, text_field};
///
/// let email = field("Email", text_field("").label("Email"))
///     .required(true)
///     .help("We only use it for receipts.");
/// # let _ = email;
/// ```
///
/// The control keeps its own a11y name: pass the same words to its `.label(…)`.
/// The field's label is what a **sighted** user reads, and until the a11y
/// vocabulary grows a `describedby` relation the two have to be given
/// separately (see the module docs).
pub fn field(label: impl Into<String>, control: impl Into<View>) -> FormField {
    FormField {
        label: label.into(),
        control: control.into(),
        error: None,
        help: None,
        required: false,
        disabled: false,
        key: None,
    }
}

impl FormField {
    /// The validation message, in the destructive colour.
    ///
    /// It **replaces** the help line while it is present: two messages under
    /// one control is one message too many, and the one that matters is the one
    /// saying what went wrong.
    ///
    /// Takes an `Option` on purpose — validation state is normally
    /// `Option<String>` in the application, and unwrapping it at the call site
    /// is how a form grows an `if` around every row.
    pub fn error(mut self, error: Option<String>) -> Self {
        self.error = error.filter(|e| !e.is_empty());
        self
    }

    /// The validation message from a plain string.
    pub fn error_text(self, error: impl Into<String>) -> Self {
        self.error(Some(error.into()))
    }

    /// A hint under the control, in the secondary colour.
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Mark the field required: an asterisk beside the label.
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Dim the **label** because the control is unusable.
    ///
    /// It does not disable the control: that is the control's own property, and
    /// a layout quietly overriding it is how a form ends up with a greyed-out
    /// label above a perfectly editable field.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The label text.
    pub fn label_text(&self) -> &str {
        &self.label
    }

    /// The message that will actually be drawn, and whether it is an error.
    ///
    /// ```
    /// use silka_widgets::field;
    /// use silka_core::view::fixed;
    ///
    /// let plain = field("Email", fixed(10.0, 10.0)).help("Optional");
    /// assert_eq!(plain.message(), Some(("Optional", false)));
    ///
    /// // An error replaces the hint rather than joining it.
    /// let bad = field("Email", fixed(10.0, 10.0))
    ///     .help("Optional")
    ///     .error_text("Not a valid address");
    /// assert_eq!(bad.message(), Some(("Not a valid address", true)));
    ///
    /// // An empty error is not an error: validation state that says "nothing
    /// // wrong" must not draw an empty red line.
    /// let ok = field("Email", fixed(10.0, 10.0)).error(Some(String::new()));
    /// assert_eq!(ok.message(), None);
    /// ```
    pub fn message(&self) -> Option<(&str, bool)> {
        match (&self.error, &self.help) {
            (Some(e), _) => Some((e.as_str(), true)),
            (None, Some(h)) => Some((h.as_str(), false)),
            _ => None,
        }
    }

    /// True when this field is currently failing validation.
    pub fn has_error(&self) -> bool {
        self.error.is_some()
    }

    /// Assemble the row, given the shared column width.
    fn into_view(
        self,
        fonts: &Fonts,
        style: FormStyle,
        label_align: MainAlign,
        width: f32,
    ) -> View {
        // The message is resolved **before** the control is moved out, so the
        // "an error replaces the hint" rule stays in one place
        // ([`FormField::message`]) instead of being restated here.
        let message = self
            .message()
            .map(|(text, is_error)| (text.to_string(), is_error));
        let FormField {
            label,
            control,
            required,
            disabled,
            key,
            ..
        } = self;

        let label_cell = constrained(
            BoxConstraints::new(width, width, style.label_band, f32::INFINITY),
            row([View::from(
                label_in_style(fonts, style, &label)
                    .required(required)
                    .disabled(disabled)
                    .align(label_align),
            )])
            .main(label_align)
            .cross(CrossAlign::Center),
        );

        let mut right: Vec<View> = Vec::with_capacity(2);
        right.push(control);
        if let Some((text, is_error)) = message {
            right.push(
                text_in(fonts, text)
                    .size(style.message_size)
                    .line_height(style.message_line_height)
                    .color(if is_error { style.error } else { style.help })
                    .role(AccessRole::Label)
                    .into(),
            );
        }

        let body = column(right)
            .spacing(style.message_gap)
            .cross(CrossAlign::Stretch);

        let line = row([View::from(label_cell), View::from(expanded(body))])
            .spacing(style.label_gap)
            // Start, not Center: a field with a message under it is taller than
            // its label, and centring the label against the whole column would
            // slide it away from the control it belongs to.
            .cross(CrossAlign::Start);

        // One group per field, so a screen reader hears the question before the
        // answer. It is deliberately **not** focusable: a form must not add a
        // Tab stop between the controls.
        let mut group = interactive(line)
            .role(AccessRole::Group)
            .label(label)
            .focusable(false);
        if disabled {
            group = group.disabled(true);
        }
        if let Some(key) = key {
            group = group.key(key);
        }
        group.into()
    }
}

impl core::fmt::Debug for FormField {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FormField")
            .field("label", &self.label)
            .field("error", &self.error)
            .field("help", &self.help)
            .field("required", &self.required)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Form
// ---------------------------------------------------------------------------

/// Dart-style form builder (§2.5).
pub struct Form {
    fonts: Fonts,
    theme: Theme,
    style: FormStyle,
    fields: Vec<FormField>,
    label_align: MainAlign,
    label_width: Option<f32>,
    key: Option<Key>,
}

/// A column of labelled controls sharing one label column — the `form` layout
/// (`KOMPONEN.md` Tier 2).
///
/// ```
/// use silka_widgets::{field, form, text_field};
///
/// let settings = form([
///     field("Full name", text_field("Ada").label("Full name")).required(true),
///     field("Email", text_field("").label("Email"))
///         .error_text("Not a valid address"),
/// ]);
/// # let _ = settings;
/// ```
///
/// Use [`form_in`] outside a build pass.
pub fn form(fields: impl IntoIterator<Item = FormField>) -> Form {
    form_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        fields,
    )
}

/// [`form`] with the text engine and the theme passed explicitly.
pub fn form_in(fonts: &Fonts, theme: &Theme, fields: impl IntoIterator<Item = FormField>) -> Form {
    Form {
        fonts: fonts.clone(),
        theme: *theme,
        style: FormStyle::from_theme(theme),
        fields: fields.into_iter().collect(),
        // The macOS Settings shape: questions end at the boundary, answers
        // start there.
        label_align: MainAlign::End,
        label_width: None,
        key: None,
    }
}

impl Form {
    /// Which edge the labels are packed against — [`MainAlign::End`] by default
    /// (macOS Settings), [`MainAlign::Start`] for the web/shadcn shape.
    pub fn label_align(mut self, align: MainAlign) -> Self {
        self.label_align = align;
        self
    }

    /// Pin the label column to an exact width instead of measuring.
    ///
    /// For two forms on the same page that must line up with each other: they
    /// have different labels, so measuring would give them different columns.
    pub fn label_width(mut self, width: f32) -> Self {
        self.label_width = (width.is_finite() && width >= 0.0).then_some(width);
        self
    }

    /// The gap between two fields, named by a spacing token (§2.6).
    pub fn spacing(mut self, token: SpaceToken) -> Self {
        self.style.field_gap = self.theme.space_of(token);
        self
    }

    /// The gap between the label column and the control column, named by a
    /// spacing token (§2.6).
    pub fn label_gap(mut self, token: SpaceToken) -> Self {
        self.style.label_gap = self.theme.space_of(token);
        self
    }

    /// Custom paint values.
    pub fn style(mut self, style: FormStyle) -> Self {
        self.style = style;
        self
    }

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The fields, in order.
    pub fn fields(&self) -> &[FormField] {
        &self.fields
    }

    /// True while any field is failing validation — what a submit button asks.
    ///
    /// ```
    /// use silka_core::view::fixed;
    /// use silka_widgets::{field, form_in, Fonts};
    /// use silka_theme::{Appearance, Theme};
    ///
    /// let fonts = Fonts::bundled_only();
    /// let theme = Theme::cupertino(Appearance::Dark);
    ///
    /// let clean = form_in(&fonts, &theme, [field("Email", fixed(10.0, 10.0))]);
    /// assert!(clean.is_valid());
    ///
    /// let dirty = form_in(
    ///     &fonts,
    ///     &theme,
    ///     [field("Email", fixed(10.0, 10.0)).error_text("Not a valid address")],
    /// );
    /// assert!(!dirty.is_valid());
    /// ```
    pub fn is_valid(&self) -> bool {
        !self.fields.iter().any(FormField::has_error)
    }

    /// The width the label column will take.
    ///
    /// Measured through the **same** engine and the same text style that will
    /// draw the labels, then clamped between [`FormStyle::min_label_width`] and
    /// [`FormStyle::max_label_width`]. A pinned [`Form::label_width`] wins.
    pub fn label_width_value(&self) -> f32 {
        if let Some(w) = self.label_width {
            return w;
        }
        let widest = self
            .fields
            .iter()
            .map(|f| measure(&self.fonts, &self.style, &f.label))
            .fold(0.0f32, f32::max);
        // The required marker lives in the same column, so it is part of the
        // width — otherwise every required label is the one that wraps.
        let marker = if self.fields.iter().any(|f| f.required) {
            measure(&self.fonts, &self.style, "*") + self.style.marker_gap
        } else {
            0.0
        };
        (widest + marker).clamp(self.style.min_label_width, self.style.max_label_width)
    }

    /// The paint values that will be used — for the gallery and token tests.
    pub fn resolved_style(&self) -> FormStyle {
        self.style
    }
}

impl core::fmt::Debug for Form {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Form")
            .field("fields", &self.fields.len())
            .field("label_align", &self.label_align)
            .finish()
    }
}

impl From<Form> for View {
    fn from(f: Form) -> View {
        let width = f.label_width_value();
        let style = f.style;
        let align = f.label_align;
        let fonts = f.fonts.clone();
        let rows: Vec<View> = f
            .fields
            .into_iter()
            .map(|field| field.into_view(&fonts, style, align, width))
            .collect();
        let mut b = column(rows)
            .spacing(style.field_gap)
            .cross(CrossAlign::Stretch);
        if let Some(key) = f.key {
            b = b.key(key);
        }
        b.into()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A label built directly from an already-resolved style, for [`field`]'s own
/// column: silent in the a11y tree, because its group already says the words.
fn label_in_style(fonts: &Fonts, style: FormStyle, text: &str) -> FormLabel {
    FormLabel {
        fonts: fonts.clone(),
        style,
        text: text.to_string(),
        required: false,
        disabled: false,
        align: MainAlign::Start,
        announce: false,
        key: None,
    }
}

/// The width `text` takes in the label style, in logical points.
fn measure(fonts: &Fonts, style: &FormStyle, text: &str) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let s = style.label_style();
    fonts.with(|engine| {
        engine
            .measure(text, &s, TextConstraints::UNBOUNDED)
            .content_size
            .width
    })
}

/// The focus policy a form's group wrapper uses — never a Tab stop.
///
/// Exposed so a test can state the rule rather than restate the number.
pub const FIELD_FOCUS: FocusPolicy = FocusPolicy::NONE;

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::tree::{RenderTree, TextDirection};
    use silka_core::view::{fixed, reconcile};
    use silka_paint::Size;
    use silka_theme::{Appearance, Preset};

    const BOX: Size = Size::new(600.0, 400.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    fn laid_out(view: impl Into<View>) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::loose(BOX));
        tree
    }

    fn control() -> impl Into<View> {
        fixed(200.0, MIN_HIT_TARGET)
    }

    #[test]
    fn the_label_column_is_measured_once_for_the_whole_form() {
        let f = form_in(
            &fonts(),
            &theme(),
            [
                field("Name", control()),
                field("A considerably longer question", control()),
            ],
        );
        let wide = f.label_width_value();

        let narrow = form_in(&fonts(), &theme(), [field("Name", control())]).label_width_value();
        assert!(
            wide > narrow,
            "the longest label is what sets the column, not the first one"
        );
    }

    #[test]
    fn the_label_column_has_a_floor_and_a_ceiling() {
        let s = FormStyle::from_theme(&theme());
        let tiny = form_in(&fonts(), &theme(), [field("A", control())]).label_width_value();
        assert_eq!(tiny, s.min_label_width);

        let huge = form_in(
            &fonts(),
            &theme(),
            [field(
                "A question so long that letting it size the column would leave \
                 no room at all for the answer beside it",
                control(),
            )],
        )
        .label_width_value();
        assert_eq!(huge, s.max_label_width);
    }

    #[test]
    fn a_pinned_width_wins_over_measuring() {
        let f = form_in(&fonts(), &theme(), [field("Name", control())]).label_width(123.0);
        assert_eq!(f.label_width_value(), 123.0);
    }

    #[test]
    fn a_required_field_reserves_room_for_its_marker() {
        // Long enough to clear the floor, short enough to stay under the
        // ceiling — otherwise the clamp would hide the difference.
        let question = "Preferred contact";
        let plain = form_in(&fonts(), &theme(), [field(question, control())]).label_width_value();
        let marked = form_in(
            &fonts(),
            &theme(),
            [field(question, control()).required(true)],
        )
        .label_width_value();
        assert!(
            marked >= plain,
            "the asterisk lives in the label column, so it is part of its width"
        );
    }

    #[test]
    fn an_error_replaces_the_hint_and_an_empty_one_is_not_an_error() {
        let both = field("Email", control())
            .help("Optional")
            .error_text("Not a valid address");
        assert_eq!(both.message(), Some(("Not a valid address", true)));
        assert!(both.has_error());

        let empty = field("Email", control())
            .help("Optional")
            .error(Some(String::new()));
        assert_eq!(empty.message(), Some(("Optional", false)));
        assert!(!empty.has_error());

        let none = field("Email", control()).error(None);
        assert_eq!(none.message(), None);
    }

    #[test]
    fn a_form_knows_whether_it_can_be_submitted() {
        let clean = form_in(&fonts(), &theme(), [field("Email", control())]);
        assert!(clean.is_valid());

        let dirty = form_in(
            &fonts(),
            &theme(),
            [field("Email", control()).error_text("Not a valid address")],
        );
        assert!(!dirty.is_valid());
    }

    #[test]
    fn every_field_is_a_group_carrying_its_question() {
        let tree = laid_out(form_in(
            &fonts(),
            &theme(),
            [field("Email", control()), field("Country", control())],
        ));
        let a11y = tree.access_tree(None);
        for question in ["Email", "Country"] {
            let e = a11y
                .find_label(question)
                .unwrap_or_else(|| panic!("{}", a11y.dump()));
            assert_eq!(e.node.role, AccessRole::Group, "{question}");
        }
    }

    #[test]
    fn a_form_adds_no_tab_stop_of_its_own() {
        let tree = laid_out(form_in(&fonts(), &theme(), [field("Email", control())]));
        let focusable = {
            let mut count = 0usize;
            let mut stack = vec![tree.root()];
            while let Some(id) = stack.pop() {
                if tree
                    .render(id)
                    .map(|r| r.focus_policy().focusable)
                    .unwrap_or(false)
                {
                    count += 1;
                }
                stack.extend(tree.children(id).iter().copied());
            }
            count
        };
        assert_eq!(
            focusable, 0,
            "the control here is a plain box; a form must not invent a Tab stop"
        );
        assert!(!FIELD_FOCUS.focusable);
    }

    #[test]
    fn a_field_announces_its_question_exactly_once() {
        let tree = laid_out(form_in(&fonts(), &theme(), [field("Email", control())]));
        let a11y = tree.access_tree(None);
        assert_eq!(
            a11y.dump().matches("Email").count(),
            1,
            "the group and its label column would otherwise both say it:\n{}",
            a11y.dump()
        );
    }

    #[test]
    fn the_error_line_is_announced_as_text() {
        let tree = laid_out(form_in(
            &fonts(),
            &theme(),
            [field("Email", control()).error_text("Not a valid address")],
        ));
        let a11y = tree.access_tree(None);
        let e = a11y
            .find_label("Not a valid address")
            .unwrap_or_else(|| panic!("{}", a11y.dump()));
        assert_eq!(e.node.role, AccessRole::Label);
    }

    #[test]
    fn every_colour_moves_with_the_preset_and_the_appearance() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let light = FormStyle::from_theme(&Theme::new(preset, Appearance::Light));
            let dark = FormStyle::from_theme(&Theme::new(preset, Appearance::Dark));
            assert_ne!(light.label, dark.label, "{preset:?}");
            assert_ne!(light.error, light.help, "{preset:?}");
        }
    }

    #[test]
    fn the_column_mirrors_in_an_rtl_document() {
        let mut tree = RenderTree::new();
        reconcile(
            &mut tree,
            form_in(&fonts(), &theme(), [field("Email", control())]),
        );
        tree.set_direction(TextDirection::Rtl);
        tree.layout(BoxConstraints::loose(BOX));
        // Nothing to assert about coordinates that `row` does not already
        // guarantee; what this test protects is that an RTL layout pass runs at
        // all without the form computing a single x of its own.
        assert!(tree.size(tree.children(tree.root())[0]).width > 0.0);
    }

    #[test]
    fn rebuilding_an_identical_form_costs_nothing() {
        let t = theme();
        let f = fonts();
        let build = || form_in(&f, &t, [field("Email", control())]);
        let mut tree = RenderTree::new();
        reconcile(&mut tree, build());
        tree.layout(BoxConstraints::loose(BOX));
        assert!(reconcile(&mut tree, build()).is_noop());
    }

    #[test]
    fn a_label_measures_itself_with_the_style_it_is_drawn_in() {
        let l = label_in(&fonts(), &theme(), "Full name");
        assert!(l.measured_width() > 0.0);
        assert_eq!(l.text(), "Full name");

        let empty = label_in(&fonts(), &theme(), "");
        assert_eq!(empty.measured_width(), 0.0);
    }
}
