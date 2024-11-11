use windows::{
    core::*, Win32::Foundation::*, Win32::Graphics::Direct2D::Common::*, Win32::Graphics::Dwm::*,
    Win32::UI::HiDpi::*, Win32::UI::WindowsAndMessaging::*,
};

use regex::Regex;
use std::ptr;
use std::thread;

use crate::border_config::MatchKind;
use crate::border_config::MatchStrategy;
use crate::border_config::WindowRule;
use crate::border_config::CONFIG;
use crate::window_border;
use crate::SendHWND;
use crate::__ImageBase;
use crate::BORDERS;
use crate::INITIAL_WINDOWS;

// I need these because Rust doesn't allow expressions for a match pattern
pub const WM_APP_LOCATIONCHANGE: u32 = WM_APP;
pub const WM_APP_REORDER: u32 = WM_APP + 1;
pub const WM_APP_SHOWUNCLOAKED: u32 = WM_APP + 2;
pub const WM_APP_HIDECLOAKED: u32 = WM_APP + 3;
pub const WM_APP_MINIMIZESTART: u32 = WM_APP + 4;
pub const WM_APP_MINIMIZEEND: u32 = WM_APP + 5;

// TODO THE CODE IS STILL A MESS

pub fn has_filtered_style(hwnd: HWND) -> bool {
    let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) as u32 };
    let ex_style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 };

    style & WS_CHILD.0 != 0
        || ex_style & WS_EX_TOOLWINDOW.0 != 0
        || ex_style & WS_EX_NOACTIVATE.0 != 0
}

// Getting the window title sometimes takes unexpectedly long (over 1ms), but it should be fine.
pub fn get_window_title(hwnd: HWND) -> String {
    let mut title_arr: [u16; 256] = [0; 256];

    if unsafe { GetWindowTextW(hwnd, &mut title_arr) } == 0 {
        println!("error getting window title!");
    }

    let title_binding = String::from_utf16_lossy(&title_arr);
    return title_binding.split_once("\0").unwrap().0.to_string();
}

pub fn get_window_class(hwnd: HWND) -> String {
    let mut class_arr: [u16; 256] = [0; 256];

    if unsafe { GetClassNameW(hwnd, &mut class_arr) } == 0 {
        println!("error getting class name!");
    }

    let class_binding = String::from_utf16_lossy(&class_arr);
    return class_binding.split_once("\0").unwrap().0.to_string();
}

pub fn get_window_rule(hwnd: HWND) -> WindowRule {
    let title = get_window_title(hwnd);
    let class = get_window_class(hwnd);

    let config = CONFIG.lock().unwrap();

    for rule in config.window_rules.iter() {
        let name = match rule.kind {
            Some(MatchKind::Title) => &title,
            Some(MatchKind::Class) => &class,
            None => {
                println!("Expected 'match' for window rule but None found!");
                continue;
            }
        };

        let Some(pattern) = &rule.pattern else {
            println!("Expected `pattern` for window rule but None found!");
            continue;
        };

        if match rule.strategy {
            Some(MatchStrategy::Equals) | None => name.to_lowercase().eq(&pattern.to_lowercase()),
            Some(MatchStrategy::Contains) => name.to_lowercase().contains(&pattern.to_lowercase()),
            Some(MatchStrategy::Regex) => Regex::new(pattern).unwrap().captures(name).is_some(),
        } {
            return rule.clone();
        }
    }

    drop(config);
    WindowRule::default()
}

pub fn is_window_visible(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd).as_bool() }
}

pub fn is_rect_visible(rect: &RECT) -> bool {
    rect.top >= 0 || rect.left >= 0 || rect.bottom >= 0 || rect.right >= 0
}

pub fn are_rects_same_size(rect1: &RECT, rect2: &RECT) -> bool {
    rect1.right - rect1.left == rect2.right - rect2.left
        && rect1.bottom - rect1.top == rect2.bottom - rect2.top
}

pub fn is_cloaked(hwnd: HWND) -> bool {
    let mut is_cloaked = FALSE;
    let result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            ptr::addr_of_mut!(is_cloaked) as _,
            size_of::<BOOL>() as u32,
        )
    };
    if result.is_err() {
        println!("error getting is_cloaked");
        return true;
    }
    is_cloaked.as_bool()
}

pub fn is_active_window(hwnd: HWND) -> bool {
    unsafe { GetForegroundWindow() == hwnd }
}

// If the tracking window does not have a window edge or is maximized, then there should be no
// border.
pub fn has_native_border(hwnd: HWND) -> bool {
    unsafe {
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;

        ex_style & WS_EX_WINDOWEDGE.0 != 0 && style & WS_MAXIMIZE.0 == 0
    }
}

pub fn get_show_cmd(hwnd: HWND) -> u32 {
    let mut wp: WINDOWPLACEMENT = WINDOWPLACEMENT::default();
    let result = unsafe { GetWindowPlacement(hwnd, ptr::addr_of_mut!(wp)) };
    if result.is_err() {
        println!("error getting window_placement!");
        return 0;
    }
    wp.showCmd
}

pub fn create_border_for_window(tracking_window: HWND) -> Result<()> {
    println!("in create_border_for_window for: {:?}", tracking_window);
    let window = SendHWND(tracking_window);

    let _ = thread::spawn(move || {
        let window_sent = window;

        let window_rule = get_window_rule(window_sent.0);

        if window_rule.enabled == Some(false) {
            println!("border is disabled for this window, exiting!");
            return;
        }

        let config = CONFIG.lock().unwrap();

        // TODO holy this is ugly
        let config_width = window_rule
            .border_width
            .unwrap_or(config.global.border_width);
        let config_offset = window_rule
            .border_offset
            .unwrap_or(config.global.border_offset);
        let config_radius = window_rule
            .border_radius
            .unwrap_or(config.global.border_radius);
        let config_active = window_rule
            .active_color
            .unwrap_or(config.global.active_color.clone());
        let config_inactive = window_rule
            .inactive_color
            .unwrap_or(config.global.inactive_color.clone());

        //let border_colors = convert_config_colors(config_active, config_inactive);
        let active_color = config_active.convert_to_color(true);
        let inactive_color = config_inactive.convert_to_color(false);

        let border_radius = convert_config_radius(config_width, config_radius, window_sent.0);

        let window_isize = window_sent.0 .0 as isize;

        let init_delay = if INITIAL_WINDOWS.lock().unwrap().contains(&window_isize) {
            0
        } else {
            window_rule
                .init_delay
                .unwrap_or(config.global.init_delay.unwrap_or(250))
        };

        let unminimize_delay = window_rule
            .unminimize_delay
            .unwrap_or(config.global.unminimize_delay.unwrap_or(200));

        let mut border = window_border::WindowBorder {
            tracking_window: window_sent.0,
            border_width: config_width,
            border_offset: config_offset,
            border_radius,
            active_color,
            inactive_color,
            unminimize_delay,
            ..Default::default()
        };

        drop(config);

        let mut borders_hashmap = BORDERS.lock().unwrap();

        // Check to see if the key already exists in the hashmap. I don't think this should ever
        // return true, but it's just in case.
        if borders_hashmap.contains_key(&window_isize) {
            drop(borders_hashmap);
            return;
        }

        let hinstance: HINSTANCE = unsafe { std::mem::transmute(&__ImageBase) };
        let _ = border.create_border_window(hinstance);
        borders_hashmap.insert(window_isize, border.border_window.0 as isize);

        // Drop these values (to save some RAM?) before calling init and entering a message loop
        drop(borders_hashmap);
        let _ = window_sent;
        let _ = window_rule;
        let _ = config_width;
        let _ = config_offset;
        let _ = config_radius;
        let _ = config_active;
        let _ = config_inactive;
        //let _ = border_colors;
        let _ = active_color;
        let _ = inactive_color;
        let _ = window_isize;
        let _ = hinstance;

        let _ = border.init(init_delay);

        drop(border);
    });

    Ok(())
}

// DEPRECATED
pub fn convert_config_colors(
    config_active: String,
    config_inactive: String,
) -> (D2D1_COLOR_F, D2D1_COLOR_F) {
    let mut accent_red: f32 = 0.0;
    let mut accent_green: f32 = 0.0;
    let mut accent_blue: f32 = 0.0;
    let mut accent_avg: f32 = 0.0;

    if config_active == "accent" || config_inactive == "accent" {
        // Get the Windows accent color
        let mut pcr_colorization: u32 = 0;
        let mut pf_opaqueblend: BOOL = FALSE;
        let result = unsafe { DwmGetColorizationColor(&mut pcr_colorization, &mut pf_opaqueblend) };
        if result.is_err() {
            println!("Error getting Windows accent color!");
        }
        accent_red = ((pcr_colorization & 0x00FF0000) >> 16) as f32 / 255.0;
        accent_green = ((pcr_colorization & 0x0000FF00) >> 8) as f32 / 255.0;
        accent_blue = (pcr_colorization & 0x000000FF) as f32 / 255.0;
        accent_avg = (accent_red + accent_green + accent_blue) / 3.0;
    }

    let active_color = if config_active == "accent" {
        D2D1_COLOR_F {
            r: accent_red,
            g: accent_green,
            b: accent_blue,
            a: 1.0,
        }
    } else {
        get_color_from_hex(config_active.as_str())
    };

    let inactive_color = if config_inactive == "accent" {
        D2D1_COLOR_F {
            r: accent_avg / 1.5 + accent_red / 10.0,
            g: accent_avg / 1.5 + accent_green / 10.0,
            b: accent_avg / 1.5 + accent_blue / 10.0,
            a: 1.0,
        }
    } else {
        get_color_from_hex(config_inactive.as_str())
    };

    (active_color, inactive_color)
}

pub fn convert_config_radius(config_width: i32, config_radius: f32, tracking_window: HWND) -> f32 {
    let mut corner_preference = DWM_WINDOW_CORNER_PREFERENCE::default();
    let dpi = unsafe { GetDpiForWindow(tracking_window) } as f32;

    // -1.0 means to use default Windows corner preference. I might want to use an enum to allow
    // for something like border_radius == "system" instead TODO
    if config_radius == -1.0 {
        let result = unsafe {
            DwmGetWindowAttribute(
                tracking_window,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                ptr::addr_of_mut!(corner_preference) as _,
                size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
            )
        };
        if result.is_err() {
            println!("Error getting window corner preference!");
        }
        match corner_preference {
            DWMWCP_DEFAULT => {
                return 8.0 * dpi / 96.0 + (config_width as f32) / 2.0;
            }
            DWMWCP_DONOTROUND => {
                return 0.0;
            }
            DWMWCP_ROUND => {
                return 8.0 * dpi / 96.0 + (config_width as f32) / 2.0;
            }
            DWMWCP_ROUNDSMALL => {
                return 4.0 * dpi / 96.0 + (config_width as f32) / 2.0;
            }
            _ => {}
        }
    }

    config_radius * dpi / 96.0
}

pub fn destroy_border_for_window(tracking_window: HWND) -> Result<()> {
    let window = SendHWND(tracking_window);

    let _ = thread::spawn(move || {
        let window_sent = window;
        let mut borders_hashmap = BORDERS.lock().unwrap();
        let window_isize = window_sent.0 .0 as isize;
        let Some(border_isize) = borders_hashmap.get(&window_isize) else {
            drop(borders_hashmap);
            return;
        };

        let border_window: HWND = HWND(*border_isize as _);
        unsafe {
            let _ = PostMessageW(border_window, WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        borders_hashmap.remove(&window_isize);

        drop(borders_hashmap);
    });

    Ok(())
}

pub fn get_border_from_window(hwnd: HWND) -> Option<HWND> {
    let borders = BORDERS.lock().unwrap();
    let hwnd_isize = hwnd.0 as isize;
    let Some(border_isize) = borders.get(&hwnd_isize) else {
        drop(borders);
        return None;
    };

    let border_window: HWND = HWND(*border_isize as _);
    drop(borders);
    Some(border_window)
}

// Return true if the border exists in the border hashmap. Otherwise, create a new border and
// return false.
pub fn show_border_for_window(hwnd: HWND) -> bool {
    let border_window = get_border_from_window(hwnd);
    if let Some(hwnd) = border_window {
        unsafe {
            let _ = PostMessageW(hwnd, WM_APP_SHOWUNCLOAKED, WPARAM(0), LPARAM(0));
        }
        true
    } else {
        if is_window_visible(hwnd) && !is_cloaked(hwnd) && !has_filtered_style(hwnd) {
            let _ = create_border_for_window(hwnd);
        }
        false
    }
}

pub fn hide_border_for_window(hwnd: HWND) -> bool {
    let window = SendHWND(hwnd);

    let _ = thread::spawn(move || {
        let window_sent = window;
        let border_option = get_border_from_window(window_sent.0);
        if let Some(border_window) = border_option {
            unsafe {
                let _ = PostMessageW(border_window, WM_APP_HIDECLOAKED, WPARAM(0), LPARAM(0));
            }
        }
    });
    true
}

pub fn get_color_from_hex(hex: &str) -> D2D1_COLOR_F {
    if hex.len() != 7 && hex.len() != 9 && hex.len() != 4 && hex.len() != 5 || !hex.starts_with('#')
    {
        println!("Invalid hex color format: {}", hex);
        return D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        };
    }
    // Expand shorthand hex formats (#RGB or #RGBA to #RRGGBB or #RRGGBBAA)
    let expanded_hex = match hex.len() {
        4 => format!(
            "#{}{}{}{}{}{}",
            &hex[1..2],
            &hex[1..2],
            &hex[2..3],
            &hex[2..3],
            &hex[3..4],
            &hex[3..4]
        ),
        5 => format!(
            "#{}{}{}{}{}{}{}{}",
            &hex[1..2],
            &hex[1..2],
            &hex[2..3],
            &hex[2..3],
            &hex[3..4],
            &hex[3..4],
            &hex[4..5],
            &hex[4..5]
        ),
        _ => hex.to_string(),
    };

    // Convert each color component to f32 between 0.0 and 1.0, handling errors
    let parse_component = |s: &str| -> f32 {
        match u8::from_str_radix(s, 16) {
            Ok(val) => val as f32 / 255.0,
            Err(_) => {
                println!("Error: Invalid component '{}' in hex: {}", s, expanded_hex);
                0.0
            }
        }
    };

    // Parse RGB values
    let r = parse_component(&expanded_hex[1..3]);
    let g = parse_component(&expanded_hex[3..5]);
    let b = parse_component(&expanded_hex[5..7]);

    // Parse alpha value if present
    let a = if expanded_hex.len() == 9 {
        parse_component(&expanded_hex[7..9])
    } else {
        1.0
    };

    D2D1_COLOR_F { r, g, b, a }
}
pub fn get_color_from_rgba(rgba: &str) -> D2D1_COLOR_F {
    let rgba = rgba
        .trim_start_matches("rgb(")
        .trim_start_matches("rgba(")
        .trim_end_matches(')');
    let components: Vec<&str> = rgba.split(',').map(|s| s.trim()).collect();
    // Check for correct number of components
    if components.len() == 3 || components.len() == 4 {
        // Parse red, green, and blue values
        let red: f32 = components[0].parse::<u32>().unwrap_or(0) as f32 / 255.0;
        let green: f32 = components[1].parse::<u32>().unwrap_or(0) as f32 / 255.0;
        let blue: f32 = components[2].parse::<u32>().unwrap_or(0) as f32 / 255.0;
        let alpha: f32 = if components.len() == 4 {
            components[3].parse::<f32>().unwrap_or(1.0).clamp(0.0, 1.0)
        } else {
            1.0
        };
        return D2D1_COLOR_F {
            r: red,
            g: green,
            b: blue,
            a: alpha, // Default alpha value for rgb()
        };
    }
    // Return a default color if parsing fails
    D2D1_COLOR_F {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    }
}
