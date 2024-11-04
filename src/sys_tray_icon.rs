use dirs::home_dir;
use tray_icon::{menu::Menu, menu::MenuEvent, menu::MenuItem, Icon, TrayIcon, TrayIconBuilder};
use windows::Win32::Foundation::LPARAM;
use windows::Win32::Foundation::WPARAM;
use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;
use windows::Win32::UI::WindowsAndMessaging::WM_CLOSE;

use crate::border_config::Config;
use crate::restart_borders;

pub fn create_tray_icon(main_thread: u32) -> Result<TrayIcon, tray_icon::Error> {
    let icon = match Icon::from_resource(1, Some((64, 64))) {
        Ok(icon) => icon,
        Err(_) => {
            println!("error getting icon!");
            std::process::exit(1)
        }
    };

    let tray_menu = Menu::new();
    let _ = tray_menu.append(&MenuItem::with_id("0", "Show Config", true, None));
    let _ = tray_menu.append(&MenuItem::with_id("1", "Reload Borders", true, None));
    let _ = tray_menu.append(&MenuItem::with_id("2", "Close", true, None));

    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_tooltip("tacky-borders")
        .with_icon(icon)
        .build();

    MenuEvent::set_event_handler(Some(move |event: MenuEvent| match event.id.0.as_str() {
        "0" => {
            let home_dir = home_dir().expect("can't find home path");
            let config_dir = home_dir.join(".config").join("tacky-borders");
            let _ = open::that(config_dir);
        }
        "1" => {
            Config::reload_config();
            restart_borders();
        }
        "2" => {
            let result = unsafe { PostThreadMessageW(main_thread, WM_CLOSE, WPARAM(0), LPARAM(0)) };
            println!("Sending WM_CLOSE to main thread: {:?}", result);
        }
        _ => {}
    }));

    tray_icon
}
