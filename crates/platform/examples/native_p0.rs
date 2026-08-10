//! Native integration P0 end to end (INTEGRASI-NATIVE §1–§2).
//!
//! One window that carries everything the milestone covers: a real menubar with
//! the standard macOS Edit menu, native file and message dialogs, the system
//! clipboard, a tray icon with its own menu, a custom transparent titlebar with
//! repositioned traffic lights, and a vibrancy material behind the window.
//!
//! ```sh
//! cargo run -p silka-platform --example native_p0
//! ```
//!
//! What to look at while it runs:
//!
//! - ⌘C / ⌘V in any native text field of the app work **because** the Edit menu
//!   is there — remove `menubar()`'s Edit menu and they stop, which is the
//!   whole argument for the default.
//! - The window background is translucent unless System Settings →
//!   Accessibility → Display → Reduce transparency is on, in which case the
//!   opaque theme token is used instead and nothing else changes.

use silka_core::input::KeyCode;
use silka_platform::menu::{cmd, cmd_shift, item, menu, menubar, MenuRole};
use silka_platform::{
    clipboard, file_dialog, message, tray, window, Dirty, Material, MessageAnswer, MessageButtons,
    MessageLevel, RgbaImage, TitlebarStyle,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The menubar: standard App + Edit + Window from `menubar()`, plus one
    // application menu of our own. `File` lands before `Window`, where the HIG
    // wants it.
    let bar = menubar("Silka Native").menu(
        menu("File")
            .item(item("file.open", "Open…").shortcut(cmd(KeyCode::Character('o'))))
            .item(item("file.save_as", "Save As…").shortcut(cmd_shift(KeyCode::Character('s'))))
            .separator()
            .item(item("edit.copy_time", "Copy Timestamp"))
            .separator()
            .role(MenuRole::CloseWindow),
    );
    assert!(bar.has_standard_edit_menu());
    assert!(bar.duplicate_ids().is_empty());

    // A tray icon needs pixels; a real application ships a template PNG.
    let ikon = RgbaImage::solid(18, 18, [255, 255, 255, 255])?;
    let tray_icon = tray("utama").tooltip("Silka Native").icon(ikon).menu(
        menu("Silka")
            .item(item("tray.show", "Tampilkan"))
            .separator()
            .role(MenuRole::Quit),
    );

    window("Silka Native")
        .size(900.0, 600.0)
        .titlebar(TitlebarStyle::Transparent)
        .material(Material::Sidebar)
        .traffic_light_inset(20.0, 24.0)
        .menubar(bar)
        .tray(tray_icon)
        .on_menu(|a| {
            match a.id().as_str() {
                "file.open" => {
                    if let Some(path) = file_dialog()
                        .title("Buka dokumen")
                        .filter("Teks", &[".txt", "md"])
                        .pick_file()
                    {
                        println!("dibuka: {}", path.display());
                    }
                }
                "file.save_as" => {
                    if let Some(path) = file_dialog().file_name("catatan.md").save_file() {
                        println!("disimpan: {}", path.display());
                    }
                }
                "edit.copy_time" => match clipboard() {
                    Ok(mut papan) => {
                        let teks = format!("{:?}", std::time::SystemTime::now());
                        if let Err(e) = papan.set_text(teks) {
                            eprintln!("clipboard gagal: {e}");
                        }
                    }
                    Err(e) => eprintln!("clipboard tidak tersedia: {e}"),
                },
                lain => println!("menu belum ditangani: {lain}"),
            }
            // Nothing on screen changed, so nothing is redrawn: opening a
            // dialog is not a reason to burn a frame (§3.5).
            Dirty::NONE
        })
        .on_tray(|a| {
            println!("tray: {a:?}");
            Dirty::NONE
        })
        .on_quit(|ctx| {
            let jawab = message("Keluar?")
                .body("Perubahan yang belum disimpan akan hilang.")
                .level(MessageLevel::Warning)
                .buttons(MessageButtons::YesNo)
                .ask();
            if jawab != MessageAnswer::Yes {
                ctx.cancel();
            }
        })
        .run()?;
    Ok(())
}
