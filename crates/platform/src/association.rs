//! File associations and deep links (INTEGRASI-NATIVE §5).
//!
//! "Double-click a `.silka` file and my application opens it" and "click a
//! `silka://` link in a browser and my application handles it" are the same
//! feature seen from two sides, and both of them are **packaging**, not
//! runtime: the OS learns about an application from its `Info.plist`, its
//! registry entries or its `.desktop` file, long before the application runs.
//!
//! A UI framework cannot register an association at runtime — and a framework
//! that pretended to would produce applications that work when launched from a
//! terminal and not when installed. What it *can* do, and what this module
//! does, is generate those three declarations from **one** description, so they
//! cannot drift apart:
//!
//! ```
//! use silka_platform::association::{app, association, url_scheme};
//!
//! let editor = app("com.example.editor", "Editor")
//!     .associate(association("silka", "Silka document").uti("com.example.silka"))
//!     .url_scheme(url_scheme("silka").description("Silka link"));
//!
//! // One description, three declarations — none of them written by hand.
//! assert!(editor.info_plist().contains("CFBundleDocumentTypes"));
//! assert!(editor.registry_script().contains("HKEY_CURRENT_USER"));
//! assert!(editor.desktop_entry().contains("MimeType="));
//! ```
//!
//! ## The other half: what arrives at runtime
//!
//! Once the OS knows, it launches the application with either a path or a URL
//! in `argv` — and on macOS, with an Apple Event to an application that is
//! **already running**. Both ends up as one question: "what was I asked to
//! open?" [`launch_request`] answers it from `argv`, and [`DeepLink`] takes the
//! URL apart without an extra dependency.
//!
//! ```
//! use silka_platform::association::{launch_request, LaunchRequest};
//!
//! let args = ["editor", "silka://open?file=notes.md"];
//! match launch_request(args.iter().copied(), &["silka"]) {
//!     Some(LaunchRequest::Url(link)) => {
//!         assert_eq!(link.action(), "open");
//!         assert_eq!(link.query("file").as_deref(), Some("notes.md"));
//!     }
//!     other => panic!("expected a deep link, got {other:?}"),
//! }
//! ```

use std::path::PathBuf;

/// The token a generated `.reg` script carries where the install directory
/// belongs.
///
/// A registry command has to be an **absolute** path, and nothing running at
/// build time knows where the application will be installed. Emitting a
/// placeholder the installer substitutes is honest; emitting a relative path
/// would produce a `.reg` file that installs cleanly and opens nothing.
///
/// ```
/// use silka_platform::association::{app, association, INSTALL_DIR_PLACEHOLDER};
///
/// let reg = app("com.example.editor", "Editor")
///     .associate(association("silka", "Silka document"))
///     .registry_script();
/// assert!(reg.contains(INSTALL_DIR_PLACEHOLDER));
/// ```
pub const INSTALL_DIR_PLACEHOLDER: &str = "<INSTALLDIR>";

// ---------------------------------------------------------------------------
// Description
// ---------------------------------------------------------------------------

/// Whether the application edits the file type or only shows it.
///
/// The OS uses it to decide which application "owns" a document: an editor is
/// offered as the default, a viewer is offered under "Open With".
///
/// ```
/// use silka_platform::association::AssociationRole;
///
/// // Claiming to edit what you can only display is how an application ends up
/// // as the default handler for files it cannot save.
/// assert_eq!(AssociationRole::default(), AssociationRole::Editor);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AssociationRole {
    /// The application can open **and save** this type.
    #[default]
    Editor,
    /// The application can only display it.
    Viewer,
}

impl AssociationRole {
    /// The `CFBundleTypeRole` value macOS expects.
    pub const fn plist_role(self) -> &'static str {
        match self {
            AssociationRole::Editor => "Editor",
            AssociationRole::Viewer => "Viewer",
        }
    }
}

/// One file type the application handles.
///
/// ```
/// use silka_platform::association::association;
///
/// let doc = association("silka", "Silka document")
///     .uti("com.example.silka")
///     .mime("application/x-silka")
///     .extension("silkaz");
///
/// // Extensions are normalised: a leading dot and a capital letter are the
/// // two ways every hand-written manifest gets this wrong.
/// assert_eq!(doc.extensions(), ["silka", "silkaz"]);
/// assert_eq!(association(".SILKA", "x").extensions(), ["silka"]);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAssociation {
    extensions: Vec<String>,
    description: String,
    uti: Option<String>,
    mime: Option<String>,
    role: AssociationRole,
}

/// Describe a file type the application handles.
///
/// `description` is what the user reads in Finder's "Kind" column and in the
/// Windows "Open With" dialog, so it is a noun phrase ("Silka document"), not a
/// sentence.
pub fn association(extension: impl AsRef<str>, description: impl Into<String>) -> FileAssociation {
    FileAssociation {
        extensions: normalize_extension(extension.as_ref())
            .into_iter()
            .collect(),
        description: description.into(),
        uti: None,
        mime: None,
        role: AssociationRole::Editor,
    }
}

/// An extension without its dot, lowercased; `None` when there is nothing left.
///
/// Both mistakes this fixes are silent: `.silka` registered with its dot
/// matches a file called `x..silka`, and `SILKA` never matches anything on a
/// case-sensitive filesystem.
///
/// ```
/// use silka_platform::association::normalize_extension;
///
/// assert_eq!(normalize_extension(".Silka").as_deref(), Some("silka"));
/// assert_eq!(normalize_extension("silka").as_deref(), Some("silka"));
/// assert_eq!(normalize_extension("  ."), None);
/// ```
pub fn normalize_extension(extension: &str) -> Option<String> {
    let trimmed = extension.trim().trim_start_matches('.').trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

impl FileAssociation {
    /// Add another extension for the same type.
    pub fn extension(mut self, extension: impl AsRef<str>) -> Self {
        if let Some(e) = normalize_extension(extension.as_ref()) {
            if !self.extensions.contains(&e) {
                self.extensions.push(e);
            }
        }
        self
    }

    /// The macOS Uniform Type Identifier (`com.example.silka`).
    pub fn uti(mut self, uti: impl Into<String>) -> Self {
        self.uti = Some(uti.into());
        self
    }

    /// The MIME type, which is what Linux keys on.
    pub fn mime(mut self, mime: impl Into<String>) -> Self {
        self.mime = Some(mime.into());
        self
    }

    /// Whether the application edits the type or only views it.
    pub fn role(mut self, role: AssociationRole) -> Self {
        self.role = role;
        self
    }

    /// The extensions, normalised, in order.
    pub fn extensions(&self) -> &[String] {
        &self.extensions
    }

    /// The human-readable name of the type.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The UTI, when one was given.
    pub fn uti_name(&self) -> Option<&str> {
        self.uti.as_deref()
    }

    /// The MIME type — the one given, or a `application/x-<ext>` fallback.
    ///
    /// A fallback rather than nothing, because a `.desktop` file with no
    /// `MimeType=` registers no association at all, which looks exactly like a
    /// working installation until someone double-clicks a file.
    ///
    /// ```
    /// use silka_platform::association::association;
    ///
    /// assert_eq!(association("silka", "x").mime_type(), "application/x-silka");
    /// assert_eq!(association("silka", "x").mime("text/silka").mime_type(), "text/silka");
    /// ```
    pub fn mime_type(&self) -> String {
        match &self.mime {
            Some(m) => m.clone(),
            None => match self.extensions.first() {
                Some(e) => format!("application/x-{e}"),
                None => "application/octet-stream".to_string(),
            },
        }
    }
}

/// A URL scheme the application answers to.
///
/// ```
/// use silka_platform::association::url_scheme;
///
/// // The scheme is stored without its separator, and lowercased: `Silka://`
/// // and `silka` are the same scheme, and only one of them matches.
/// assert_eq!(url_scheme("Silka://").name(), "silka");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlScheme {
    name: String,
    description: Option<String>,
}

/// Describe a URL scheme.
pub fn url_scheme(name: impl AsRef<str>) -> UrlScheme {
    UrlScheme {
        name: normalize_scheme(name.as_ref()),
        description: None,
    }
}

/// A scheme name without `://`, lowercased.
///
/// ```
/// use silka_platform::association::normalize_scheme;
///
/// assert_eq!(normalize_scheme("Silka://"), "silka");
/// assert_eq!(normalize_scheme("silka:"), "silka");
/// ```
pub fn normalize_scheme(name: &str) -> String {
    name.trim()
        .trim_end_matches('/')
        .trim_end_matches(':')
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

impl UrlScheme {
    /// What the user reads when the OS asks which application to use.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// The scheme, without `://`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The description, when one was given.
    pub fn label(&self) -> Option<&str> {
        self.description.as_deref()
    }
}

/// The application these declarations describe.
///
/// ```
/// use silka_platform::association::{app, association};
///
/// let editor = app("com.example.editor", "Editor")
///     .executable("editor")
///     .associate(association("silka", "Silka document"));
///
/// assert_eq!(editor.bundle_id(), "com.example.editor");
/// assert_eq!(editor.associations().len(), 1);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppIdentity {
    bundle_id: String,
    name: String,
    executable: Option<String>,
    associations: Vec<FileAssociation>,
    schemes: Vec<UrlScheme>,
}

/// Describe an application for the purpose of these declarations.
///
/// The bundle identifier is reverse-DNS on macOS, the ProgID root on Windows,
/// and the `.desktop` file's basename on Linux — one name, three jobs, which is
/// exactly why it is one field.
pub fn app(bundle_id: impl Into<String>, name: impl Into<String>) -> AppIdentity {
    AppIdentity {
        bundle_id: bundle_id.into(),
        name: name.into(),
        executable: None,
        associations: Vec::new(),
        schemes: Vec::new(),
    }
}

impl AppIdentity {
    /// The executable's file name, when it differs from the application name.
    pub fn executable(mut self, executable: impl Into<String>) -> Self {
        self.executable = Some(executable.into());
        self
    }

    /// Handle a file type.
    pub fn associate(mut self, association: FileAssociation) -> Self {
        self.associations.push(association);
        self
    }

    /// Answer a URL scheme.
    pub fn url_scheme(mut self, scheme: UrlScheme) -> Self {
        self.schemes.push(scheme);
        self
    }

    /// The bundle identifier.
    pub fn bundle_id(&self) -> &str {
        &self.bundle_id
    }

    /// The display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The executable's file name — the application name when none was set.
    pub fn executable_name(&self) -> &str {
        self.executable.as_deref().unwrap_or(&self.name)
    }

    /// The file types.
    pub fn associations(&self) -> &[FileAssociation] {
        &self.associations
    }

    /// The URL schemes.
    pub fn url_schemes(&self) -> &[UrlScheme] {
        &self.schemes
    }

    /// The `Info.plist` keys macOS needs, as a plist fragment.
    ///
    /// A **fragment**, not a whole file: it is meant to be pasted into (or
    /// merged with) the `Info.plist` a bundler already writes, and generating a
    /// whole file here would fight with every packaging tool.
    ///
    /// ```
    /// use silka_platform::association::{app, association, url_scheme};
    ///
    /// let plist = app("com.example.editor", "Editor")
    ///     .associate(association("silka", "Silka document").uti("com.example.silka"))
    ///     .url_scheme(url_scheme("silka"))
    ///     .info_plist();
    ///
    /// assert!(plist.contains("<key>CFBundleDocumentTypes</key>"));
    /// assert!(plist.contains("com.example.silka"));
    /// assert!(plist.contains("<key>CFBundleURLSchemes</key>"));
    /// ```
    pub fn info_plist(&self) -> String {
        let mut out = String::new();
        if !self.associations.is_empty() {
            out.push_str("<key>CFBundleDocumentTypes</key>\n<array>\n");
            for a in &self.associations {
                out.push_str("  <dict>\n");
                out.push_str(&format!(
                    "    <key>CFBundleTypeName</key>\n    <string>{}</string>\n",
                    xml_escape(&a.description)
                ));
                out.push_str(&format!(
                    "    <key>CFBundleTypeRole</key>\n    <string>{}</string>\n",
                    a.role.plist_role()
                ));
                out.push_str("    <key>CFBundleTypeExtensions</key>\n    <array>\n");
                for e in &a.extensions {
                    out.push_str(&format!("      <string>{}</string>\n", xml_escape(e)));
                }
                out.push_str("    </array>\n");
                if let Some(uti) = &a.uti {
                    out.push_str("    <key>LSItemContentTypes</key>\n    <array>\n");
                    out.push_str(&format!("      <string>{}</string>\n", xml_escape(uti)));
                    out.push_str("    </array>\n");
                }
                out.push_str("  </dict>\n");
            }
            out.push_str("</array>\n");
        }
        if !self.schemes.is_empty() {
            out.push_str("<key>CFBundleURLTypes</key>\n<array>\n");
            for s in &self.schemes {
                out.push_str("  <dict>\n");
                out.push_str(&format!(
                    "    <key>CFBundleURLName</key>\n    <string>{}</string>\n",
                    xml_escape(s.label().unwrap_or(&self.bundle_id))
                ));
                out.push_str("    <key>CFBundleURLSchemes</key>\n    <array>\n");
                out.push_str(&format!("      <string>{}</string>\n", xml_escape(&s.name)));
                out.push_str("    </array>\n");
                out.push_str("  </dict>\n");
            }
            out.push_str("</array>\n");
        }
        out
    }

    /// The Windows registry entries, as a `.reg` script.
    ///
    /// Written under `HKEY_CURRENT_USER\Software\Classes` rather than
    /// `HKEY_CLASSES_ROOT`: a per-user association needs no administrator, and
    /// an installer that demands elevation to claim a file extension is one
    /// users cancel.
    ///
    /// ```
    /// use silka_platform::association::{app, association};
    ///
    /// let reg = app("com.example.editor", "Editor")
    ///     .associate(association("silka", "Silka document"))
    ///     .registry_script();
    ///
    /// assert!(reg.starts_with("Windows Registry Editor Version 5.00"));
    /// assert!(reg.contains(r"Software\Classes\.silka"));
    /// // The command is quoted and takes the dropped file as its argument.
    /// assert!(reg.contains(r#"\"%1\""#));
    /// // The install directory is not knowable here; the installer fills it in.
    /// assert!(reg.contains(silka_platform::association::INSTALL_DIR_PLACEHOLDER));
    /// ```
    pub fn registry_script(&self) -> String {
        let prog_root = self.bundle_id.replace(' ', "");
        let mut out = String::from("Windows Registry Editor Version 5.00\r\n\r\n");
        for a in &self.associations {
            let Some(first) = a.extensions.first() else {
                continue;
            };
            let prog_id = format!("{prog_root}.{first}");
            for e in &a.extensions {
                out.push_str(&format!(
                    "[HKEY_CURRENT_USER\\Software\\Classes\\.{e}]\r\n@=\"{prog_id}\"\r\n\r\n"
                ));
            }
            out.push_str(&format!(
                "[HKEY_CURRENT_USER\\Software\\Classes\\{prog_id}]\r\n@=\"{}\"\r\n\r\n",
                reg_escape(&a.description)
            ));
            out.push_str(&format!(
                "[HKEY_CURRENT_USER\\Software\\Classes\\{prog_id}\\shell\\open\\command]\r\n@=\"\\\"{INSTALL_DIR_PLACEHOLDER}\\\\{}.exe\\\" \\\"%1\\\"\"\r\n\r\n",
                reg_escape(self.executable_name())
            ));
        }
        for s in &self.schemes {
            let key = format!("HKEY_CURRENT_USER\\Software\\Classes\\{}", s.name);
            out.push_str(&format!(
                "[{key}]\r\n@=\"URL:{}\"\r\n\"URL Protocol\"=\"\"\r\n\r\n",
                reg_escape(s.label().unwrap_or(&self.name))
            ));
            out.push_str(&format!(
                "[{key}\\shell\\open\\command]\r\n@=\"\\\"{INSTALL_DIR_PLACEHOLDER}\\\\{}.exe\\\" \\\"%1\\\"\"\r\n\r\n",
                reg_escape(self.executable_name())
            ));
        }
        out
    }

    /// The freedesktop `.desktop` entry.
    ///
    /// `%U` rather than `%f`: it makes one entry handle both a dropped file and
    /// a `silka://` link, which is the whole point of doing these two features
    /// together.
    ///
    /// ```
    /// use silka_platform::association::{app, association, url_scheme};
    ///
    /// let entry = app("com.example.editor", "Editor")
    ///     .associate(association("silka", "Silka document").mime("application/x-silka"))
    ///     .url_scheme(url_scheme("silka"))
    ///     .desktop_entry();
    ///
    /// assert!(entry.starts_with("[Desktop Entry]"));
    /// assert!(entry.contains("Exec=Editor %U"));
    /// // Both the file type and the scheme end up in one MimeType line.
    /// assert!(entry.contains("application/x-silka"));
    /// assert!(entry.contains("x-scheme-handler/silka"));
    /// ```
    pub fn desktop_entry(&self) -> String {
        let mut mimes: Vec<String> = self.associations.iter().map(|a| a.mime_type()).collect();
        mimes.extend(
            self.schemes
                .iter()
                .map(|s| format!("x-scheme-handler/{}", s.name)),
        );
        let mut out = String::from("[Desktop Entry]\n");
        out.push_str("Type=Application\n");
        out.push_str(&format!("Name={}\n", desktop_escape(&self.name)));
        out.push_str(&format!(
            "Exec={} %U\n",
            desktop_escape(self.executable_name())
        ));
        out.push_str("Terminal=false\n");
        out.push_str(&format!("Icon={}\n", desktop_escape(&self.bundle_id)));
        // A trailing semicolon is required by the spec for a list, and its
        // absence is the classic reason a `.desktop` file "does nothing".
        out.push_str(&format!("MimeType={};\n", mimes.join(";")));
        out
    }
}

/// Escape the five characters XML cannot carry literally.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

/// Escape a string for a `.reg` value.
fn reg_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Strip what a `.desktop` value may not contain.
fn desktop_escape(s: &str) -> String {
    s.replace(['\n', '\r', ';'], " ")
}

// ---------------------------------------------------------------------------
// Deep links
// ---------------------------------------------------------------------------

/// A `myapp://…` URL, taken apart.
///
/// Not a general URL parser and not trying to be one: a deep link is a command
/// with arguments, so what matters is the **action** and the **query**, and
/// getting percent-decoding right so that `?file=my%20notes.md` is a filename
/// with a space rather than one with a `%20` in it.
///
/// ```
/// use silka_platform::association::DeepLink;
///
/// let link = DeepLink::parse("silka://open/project?file=my%20notes.md&line=42").unwrap();
/// assert_eq!(link.scheme(), "silka");
/// assert_eq!(link.action(), "open");
/// assert_eq!(link.path(), ["open", "project"]);
/// assert_eq!(link.query("file").as_deref(), Some("my notes.md"));
/// assert_eq!(link.query("line").as_deref(), Some("42"));
/// assert_eq!(link.query("missing"), None);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeepLink {
    scheme: String,
    segments: Vec<String>,
    query: Vec<(String, String)>,
}

impl DeepLink {
    /// Take a URL apart, or `None` when it is not one.
    ///
    /// ```
    /// use silka_platform::association::DeepLink;
    ///
    /// // A path is not a URL, and must not be mistaken for one — that is how
    /// // a file called `open` ends up executing an action called `open`.
    /// assert!(DeepLink::parse("/tmp/notes.md").is_none());
    /// assert!(DeepLink::parse("silka://").is_some());
    /// ```
    pub fn parse(url: &str) -> Option<Self> {
        let url = url.trim();
        let (scheme, rest) = url.split_once("://")?;
        let scheme = normalize_scheme(scheme);
        if scheme.is_empty() || !scheme.chars().all(is_scheme_char) {
            return None;
        }
        let (path, query) = match rest.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (rest, None),
        };
        // A fragment is not part of a deep link's meaning; dropping it keeps
        // `silka://open?file=a#frag` from producing a file called "a#frag".
        let path = path.split('#').next().unwrap_or(path);
        let segments = path
            .split('/')
            .filter(|s| !s.is_empty())
            .filter_map(percent_decode)
            .collect();
        let query = query
            .map(|q| {
                q.split('&')
                    .filter(|p| !p.is_empty())
                    .filter_map(|pair| {
                        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                        Some((percent_decode(k)?, percent_decode(v)?))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(DeepLink {
            scheme,
            segments,
            query,
        })
    }

    /// The scheme, without `://`.
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// The first path segment — what the link asks the application to do.
    ///
    /// An empty string for a bare `silka://`, which is "just come to the
    /// front" and is a perfectly ordinary link.
    pub fn action(&self) -> &str {
        self.segments.first().map(String::as_str).unwrap_or("")
    }

    /// Every path segment, percent-decoded.
    pub fn path(&self) -> &[String] {
        &self.segments
    }

    /// One query value, percent-decoded.
    pub fn query(&self, key: &str) -> Option<String> {
        self.query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    }

    /// Every query pair, in order.
    pub fn queries(&self) -> &[(String, String)] {
        &self.query
    }
}

fn is_scheme_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')
}

/// Percent-decode a URL component; `None` when the bytes are not UTF-8.
///
/// `None` rather than a lossy string: a filename that silently gained a `�`
/// does not exist on disk, and failing here is how the caller finds out.
///
/// ```
/// use silka_platform::association::percent_decode;
///
/// assert_eq!(percent_decode("my%20notes.md").as_deref(), Some("my notes.md"));
/// assert_eq!(percent_decode("caf%C3%A9").as_deref(), Some("café"));
/// // A stray `%` is kept as itself rather than eating the next two characters.
/// assert_eq!(percent_decode("100%").as_deref(), Some("100%"));
/// assert_eq!(percent_decode("%FF"), None);
/// ```
pub fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2])) {
                out.push(hi << 4 | lo);
                i += 3;
                continue;
            }
        }
        // `+` means a space in a query string, and browsers do produce it.
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8(out).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// What the OS asked the application to open.
///
/// ```
/// use silka_platform::association::{launch_request, LaunchRequest};
///
/// // A plain launch asks for nothing.
/// assert!(launch_request(["editor"], &["silka"]).is_none());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchRequest {
    /// A file, by path.
    File(PathBuf),
    /// A deep link.
    Url(DeepLink),
}

/// The first thing in `argv` that is a file or a link the application claims.
///
/// `schemes` is the list the application registered; anything else is left
/// alone, because a URL for somebody else's scheme in our `argv` is somebody
/// else's business.
///
/// The first argument is skipped — it is the executable's own path, and the
/// classic way to write this function wrongly is to open it.
///
/// ```
/// use silka_platform::association::{launch_request, LaunchRequest};
///
/// // A file path.
/// assert_eq!(
///     launch_request(["editor", "/tmp/notes.md"], &["silka"]),
///     Some(LaunchRequest::File("/tmp/notes.md".into()))
/// );
///
/// // Flags are not documents.
/// assert!(launch_request(["editor", "--verbose"], &["silka"]).is_none());
///
/// // Somebody else's scheme is somebody else's business.
/// assert!(launch_request(["editor", "mailto:a@example.com"], &["silka"]).is_none());
/// ```
pub fn launch_request<S: AsRef<str>>(
    args: impl IntoIterator<Item = S>,
    schemes: &[&str],
) -> Option<LaunchRequest> {
    for arg in args.into_iter().skip(1) {
        let arg = arg.as_ref();
        if arg.starts_with('-') {
            continue;
        }
        if let Some(link) = DeepLink::parse(arg) {
            if schemes.iter().any(|s| normalize_scheme(s) == link.scheme) {
                return Some(LaunchRequest::Url(link));
            }
            continue;
        }
        if looks_like_url(arg) {
            // A URL for a scheme we do not own.
            continue;
        }
        return Some(LaunchRequest::File(PathBuf::from(arg)));
    }
    None
}

/// Whether an argument reads as a URL rather than a path.
///
/// The one case this has to get right is `C:\Users\a.md`, which has a colon in
/// it and is a path: a scheme is at least two characters, so a single drive
/// letter never qualifies.
///
/// ```
/// use silka_platform::association::looks_like_url;
///
/// assert!(looks_like_url("mailto:a@example.com"));
/// assert!(looks_like_url("https://example.com"));
/// assert!(!looks_like_url(r"C:\Users\a.md"));
/// assert!(!looks_like_url("/tmp/a.md"));
/// ```
pub fn looks_like_url(arg: &str) -> bool {
    let Some((scheme, _)) = arg.split_once(':') else {
        return false;
    };
    scheme.len() >= 2
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme.chars().all(is_scheme_char)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ekstensi_dinormalkan_titik_dan_huruf_besarnya() {
        // Both mistakes are silent: a dot registers `x..silka`, and an
        // uppercase extension matches nothing on a case-sensitive filesystem.
        assert_eq!(normalize_extension(".Silka").as_deref(), Some("silka"));
        assert_eq!(normalize_extension("SILKA").as_deref(), Some("silka"));
        assert_eq!(normalize_extension(" . "), None);
        assert_eq!(normalize_extension(""), None);
    }

    #[test]
    fn ekstensi_ganda_tidak_didaftarkan_dua_kali() {
        let a = association("silka", "Dokumen")
            .extension(".SILKA")
            .extension("silkaz");
        assert_eq!(a.extensions(), ["silka", "silkaz"]);
    }

    #[test]
    fn skema_dinormalkan_pemisahnya() {
        assert_eq!(normalize_scheme("Silka://"), "silka");
        assert_eq!(normalize_scheme("silka:"), "silka");
        assert_eq!(normalize_scheme("silka"), "silka");
    }

    #[test]
    fn mime_punya_cadangan_supaya_desktop_entry_tidak_kosong() {
        // A `.desktop` file with an empty MimeType= registers nothing, and
        // looks exactly like a working install until somebody double-clicks.
        assert_eq!(association("silka", "x").mime_type(), "application/x-silka");
        assert_eq!(
            association("silka", "x").mime("text/silka").mime_type(),
            "text/silka"
        );
    }

    #[test]
    fn plist_menyebut_semua_ekstensi_dan_utinya() {
        let plist = app("com.example.editor", "Editor")
            .associate(
                association("silka", "Dokumen Silka")
                    .extension("silkaz")
                    .uti("com.example.silka"),
            )
            .info_plist();
        assert!(plist.contains("<string>silka</string>"));
        assert!(plist.contains("<string>silkaz</string>"));
        assert!(plist.contains("com.example.silka"));
        assert!(plist.contains("<string>Editor</string>") || plist.contains("Dokumen Silka"));
    }

    #[test]
    fn plist_menyandikan_karakter_xml() {
        // A description containing `&` produces an unparseable plist, and the
        // failure appears at install time on somebody else's machine.
        let plist = app("com.example.editor", "Editor")
            .associate(association("silka", "Notes & drafts"))
            .info_plist();
        assert!(plist.contains("Notes &amp; drafts"));
        assert!(!plist.contains("Notes & drafts"));
    }

    #[test]
    fn plist_kosong_kalau_tidak_ada_yang_didaftarkan() {
        assert!(app("com.example.editor", "Editor").info_plist().is_empty());
    }

    #[test]
    fn registry_menulis_per_pengguna_bukan_per_mesin() {
        // An installer that demands elevation to claim an extension is one
        // users cancel.
        let reg = app("com.example.editor", "Editor")
            .associate(association("silka", "Dokumen"))
            .registry_script();
        assert!(reg.contains("HKEY_CURRENT_USER"));
        assert!(!reg.contains("HKEY_LOCAL_MACHINE"));
        assert!(reg.contains("\\Software\\Classes\\.silka"));
    }

    #[test]
    fn registry_mendaftarkan_url_protocol_untuk_skema() {
        let reg = app("com.example.editor", "Editor")
            .url_scheme(url_scheme("silka"))
            .registry_script();
        // Without this exact empty value Windows ignores the whole key.
        assert!(reg.contains("\"URL Protocol\"=\"\""));
    }

    #[test]
    fn desktop_entry_menggabungkan_berkas_dan_skema_dalam_satu_mimetype() {
        let entry = app("com.example.editor", "Editor")
            .executable("editor")
            .associate(association("silka", "Dokumen").mime("application/x-silka"))
            .url_scheme(url_scheme("silka"))
            .desktop_entry();
        assert!(entry.contains("Exec=editor %U"));
        assert!(entry.contains("application/x-silka"));
        assert!(entry.contains("x-scheme-handler/silka"));
        // The trailing semicolon is required by the spec.
        assert!(entry.contains("x-scheme-handler/silka;\n"));
    }

    #[test]
    fn desktop_entry_membuang_karakter_yang_merusak_formatnya() {
        let entry = app("com.example.editor", "Edit;or\nX").desktop_entry();
        assert!(entry.contains("Name=Edit or X"));
    }

    #[test]
    fn deep_link_dipecah_jadi_aksi_dan_kueri() {
        let link = DeepLink::parse("silka://open/project?file=my%20notes.md&line=42").unwrap();
        assert_eq!(link.scheme(), "silka");
        assert_eq!(link.action(), "open");
        assert_eq!(link.path(), ["open", "project"]);
        assert_eq!(link.query("file").as_deref(), Some("my notes.md"));
        assert_eq!(link.query("line").as_deref(), Some("42"));
        assert_eq!(link.queries().len(), 2);
    }

    #[test]
    fn deep_link_kosong_tetap_sah() {
        // `silka://` means "come to the front", which is an ordinary link.
        let link = DeepLink::parse("silka://").unwrap();
        assert_eq!(link.action(), "");
        assert!(link.path().is_empty());
    }

    #[test]
    fn fragment_tidak_ikut_ke_dalam_nilai() {
        let link = DeepLink::parse("silka://open#section").unwrap();
        assert_eq!(link.action(), "open");
    }

    #[test]
    fn bukan_url_bukan_deep_link() {
        // A file called `open` must not execute an action called `open`.
        assert!(DeepLink::parse("/tmp/notes.md").is_none());
        assert!(DeepLink::parse("notes.md").is_none());
        assert!(DeepLink::parse("://x").is_none());
    }

    #[test]
    fn persen_didekode_per_bita_utf8() {
        assert_eq!(percent_decode("caf%C3%A9").as_deref(), Some("café"));
        assert_eq!(percent_decode("a+b").as_deref(), Some("a b"));
        assert_eq!(percent_decode("100%").as_deref(), Some("100%"));
        assert_eq!(percent_decode("%zz").as_deref(), Some("%zz"));
        // Invalid UTF-8 is `None` rather than a filename with a replacement
        // character that does not exist on disk.
        assert_eq!(percent_decode("%FF"), None);
    }

    #[test]
    fn argumen_pertama_tidak_pernah_dibuka() {
        // The classic way to write this wrongly: opening your own executable.
        assert!(launch_request(["/usr/bin/editor"], &["silka"]).is_none());
        assert_eq!(
            launch_request(["/usr/bin/editor", "/tmp/a.md"], &["silka"]),
            Some(LaunchRequest::File("/tmp/a.md".into()))
        );
    }

    #[test]
    fn bendera_bukan_dokumen() {
        assert!(launch_request(["editor", "--verbose", "-q"], &["silka"]).is_none());
        // …but a real document after the flags still counts.
        assert_eq!(
            launch_request(["editor", "--verbose", "/tmp/a.md"], &["silka"]),
            Some(LaunchRequest::File("/tmp/a.md".into()))
        );
    }

    #[test]
    fn skema_orang_lain_dilewati() {
        assert!(launch_request(["editor", "mailto:a@example.com"], &["silka"]).is_none());
        assert!(launch_request(["editor", "https://example.com"], &["silka"]).is_none());
        match launch_request(["editor", "silka://open"], &["silka"]) {
            Some(LaunchRequest::Url(l)) => assert_eq!(l.action(), "open"),
            other => panic!("harusnya deep link, dapat {other:?}"),
        }
    }

    #[test]
    fn peran_bawaan_adalah_editor() {
        assert_eq!(AssociationRole::default(), AssociationRole::Editor);
        assert_eq!(AssociationRole::Viewer.plist_role(), "Viewer");
    }
}
