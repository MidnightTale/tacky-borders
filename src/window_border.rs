// TODO Add result handling. There's so many let _ =
use crate::utils::*;
use std::ptr;
use std::sync::LazyLock;
use std::sync::OnceLock;
use std::thread;
use windows::{
    core::*, Foundation::Numerics::*, Win32::Foundation::*, Win32::Graphics::Direct2D::Common::*,
    Win32::Graphics::Direct2D::*, Win32::Graphics::Dwm::*, Win32::Graphics::Dxgi::Common::*,
    Win32::Graphics::Gdi::*, Win32::UI::WindowsAndMessaging::*,
};

pub static RENDER_FACTORY: LazyLock<ID2D1Factory> = unsafe {
    LazyLock::new(|| {
        D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_MULTI_THREADED, None)
            .expect("creating RENDER_FACTORY failed")
    })
};

#[derive(Debug, Default)]
pub struct WindowBorder {
    pub border_window: HWND,
    pub tracking_window: HWND,
    pub window_rect: RECT,
    pub border_size: i32,
    pub border_offset: i32,
    pub border_radius: f32,
    pub brush_properties: D2D1_BRUSH_PROPERTIES,
    pub render_target: OnceLock<ID2D1HwndRenderTarget>,
    pub rounded_rect: D2D1_ROUNDED_RECT,
    pub active_color: D2D1_COLOR_F,
    pub inactive_color: D2D1_COLOR_F,
    pub current_color: D2D1_COLOR_F,
    // Delay border visbility when tracking window is in unminimize animation
    pub unminimize_delay: u64,
    // This is to pause the border from doing anything when it doesn't need to
    pub pause: bool,
}

impl WindowBorder {
    pub fn create_border_window(&mut self, hinstance: HINSTANCE) -> Result<()> {
        unsafe {
            self.border_window = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
                w!("tacky-border"),
                w!("tacky-border"),
                WS_POPUP | WS_DISABLED,
                0,
                0,
                0,
                0,
                None,
                None,
                hinstance,
                Some(ptr::addr_of!(*self) as _),
            )?;
        }

        Ok(())
    }

    pub fn init(&mut self, init_delay: u64) -> Result<()> {
        // Delay the border while the tracking window is in its creation animation
        thread::sleep(std::time::Duration::from_millis(init_delay));

        unsafe {
            // Make the window border transparent
            let pos: i32 = -GetSystemMetrics(SM_CXVIRTUALSCREEN) - 8;
            let hrgn = CreateRectRgn(pos, 0, pos + 1, 1);
            let mut bh: DWM_BLURBEHIND = Default::default();
            if !hrgn.is_invalid() {
                bh = DWM_BLURBEHIND {
                    dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION,
                    fEnable: TRUE,
                    hRgnBlur: hrgn,
                    fTransitionOnMaximized: FALSE,
                };
            }

            let _ = DwmEnableBlurBehindWindow(self.border_window, &bh);
            if SetLayeredWindowAttributes(self.border_window, COLORREF(0x00000000), 0, LWA_COLORKEY)
                .is_err()
            {
                println!("Error Setting Layered Window Attributes!");
            }
            if SetLayeredWindowAttributes(self.border_window, COLORREF(0x00000000), 255, LWA_ALPHA)
                .is_err()
            {
                println!("Error Setting Layered Window Attributes!");
            }

            let _ = self.create_render_targets();
            if has_native_border(self.tracking_window) {
                let _ = self.update_position(Some(SWP_SHOWWINDOW));
                let _ = self.render();

                // Sometimes, it doesn't show the window at first, so we wait 5ms and update it.
                // This is very hacky and needs to be looked into. It may be related to the issue
                // detailed in the wnd_proc. TODO
                thread::sleep(std::time::Duration::from_millis(5));
                let _ = self.update_position(Some(SWP_SHOWWINDOW));
                let _ = self.render();
            }

            let mut message = MSG::default();
            while GetMessageW(&mut message, HWND::default(), 0, 0).into() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
                thread::sleep(std::time::Duration::from_millis(1));
            }
        }

        Ok(())
    }

    pub fn create_render_targets(&mut self) -> Result<()> {
        let render_target_properties = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_UNKNOWN,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            ..Default::default()
        };
        let hwnd_render_target_properties = D2D1_HWND_RENDER_TARGET_PROPERTIES {
            hwnd: self.border_window,
            pixelSize: Default::default(),
            presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY,
        };
        self.brush_properties = D2D1_BRUSH_PROPERTIES {
            opacity: 1.0,
            transform: Matrix3x2::identity(),
        };

        self.rounded_rect = D2D1_ROUNDED_RECT {
            rect: Default::default(),
            radiusX: self.border_radius,
            radiusY: self.border_radius,
        };

        // Initialize the actual border color assuming it is in focus
        self.current_color = self.active_color;

        unsafe {
            let factory = &*RENDER_FACTORY;
            let _ = self.render_target.set(
                factory
                    .CreateHwndRenderTarget(
                        &render_target_properties,
                        &hwnd_render_target_properties,
                    )
                    .expect("creating self.render_target failed"),
            );
            let render_target = self.render_target.get().unwrap();
            render_target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        }

        let _ = self.update_color();
        let _ = self.update_window_rect();
        let _ = self.update_position(None);

        Ok(())
    }

    pub fn update_window_rect(&mut self) -> Result<()> {
        let result = unsafe {
            DwmGetWindowAttribute(
                self.tracking_window,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                ptr::addr_of_mut!(self.window_rect) as _,
                size_of::<RECT>() as u32,
            )
        };
        if result.is_err() {
            println!("Error getting frame rect!");
            unsafe {
                let _ = ShowWindow(self.border_window, SW_HIDE);
            }
        }

        self.window_rect.top -= self.border_size;
        self.window_rect.left -= self.border_size;
        self.window_rect.right += self.border_size;
        self.window_rect.bottom += self.border_size;

        Ok(())
    }

    pub fn update_position(&mut self, c_flags: Option<SET_WINDOW_POS_FLAGS>) -> Result<()> {
        unsafe {
            // Place the window border above the tracking window
            let hwnd_above_tracking = GetWindow(self.tracking_window, GW_HWNDPREV);
            let mut u_flags =
                SWP_NOSENDCHANGING | SWP_NOACTIVATE | SWP_NOREDRAW | c_flags.unwrap_or_default();

            // If hwnd_above_tracking is the window border itself, we have what we want and there's
            //  no need to change the z-order (plus it results in an error if we try it).
            // If hwnd_above_tracking returns an error, it's likely that tracking_window is already
            //  the highest in z-order, so we use HWND_TOP to place the window border above.
            if hwnd_above_tracking == Ok(self.border_window) {
                u_flags |= SWP_NOZORDER;
            }

            let result = SetWindowPos(
                self.border_window,
                hwnd_above_tracking.unwrap_or(HWND_TOP),
                self.window_rect.left,
                self.window_rect.top,
                self.window_rect.right - self.window_rect.left,
                self.window_rect.bottom - self.window_rect.top,
                u_flags,
            );
            if result.is_err() {
                println!("Error setting window pos!");
                let _ = ShowWindow(self.border_window, SW_HIDE);
            }
        }
        Ok(())
    }

    pub fn update_color(&mut self) -> Result<()> {
        if is_active_window(self.tracking_window) {
            self.current_color = self.active_color;
        } else {
            self.current_color = self.inactive_color;
        }
        Ok(())
    }

    pub fn render(&mut self) -> Result<()> {
        // Get the render target
        let render_target_option = self.render_target.get();
        if render_target_option.is_none() {
            return Ok(());
        }
        let render_target = render_target_option.unwrap();

        let pixel_size = D2D_SIZE_U {
            width: (self.window_rect.right - self.window_rect.left) as u32,
            height: (self.window_rect.bottom - self.window_rect.top) as u32,
        };

        self.rounded_rect.rect = D2D_RECT_F {
            left: (self.border_size / 2 - self.border_offset) as f32,
            top: (self.border_size / 2 - self.border_offset) as f32,
            right: (self.window_rect.right - self.window_rect.left - self.border_size / 2
                + self.border_offset) as f32,
            bottom: (self.window_rect.bottom - self.window_rect.top - self.border_size / 2
                + self.border_offset) as f32,
        };

        unsafe {
            let _ = render_target.Resize(ptr::addr_of!(pixel_size));

            let brush = render_target
                .CreateSolidColorBrush(&self.current_color, Some(&self.brush_properties))?;

            render_target.BeginDraw();
            render_target.Clear(None);
            render_target.DrawRoundedRectangle(
                &self.rounded_rect,
                &brush,
                self.border_size as f32,
                None,
            );
            let _ = render_target.EndDraw(None, None);
        }
        Ok(())
    }

    // When CreateWindowExW is called, we can optionally pass a value to its LPARAM field which will
    // get sent to the window process on creation. In our code, we've passed a pointer to the
    // WindowBorder structure during the window creation process, and here we are getting that pointer
    // and attaching it to the window using SetWindowLongPtrW.
    pub unsafe extern "system" fn s_wnd_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let mut border_pointer: *mut WindowBorder = GetWindowLongPtrW(window, GWLP_USERDATA) as _;

        if border_pointer.is_null() && message == WM_CREATE {
            let create_struct: *mut CREATESTRUCTW = lparam.0 as *mut _;
            border_pointer = (*create_struct).lpCreateParams as *mut _;
            SetWindowLongPtrW(window, GWLP_USERDATA, border_pointer as _);
        }
        match !border_pointer.is_null() {
            true => Self::wnd_proc(&mut *border_pointer, window, message, wparam, lparam),
            false => DefWindowProcW(window, message, wparam, lparam),
        }
    }

    pub unsafe fn wnd_proc(
        &mut self,
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            // EVENT_OBJECT_LOCATIONCHANGE
            WM_APP_0 => {
                if self.pause {
                    return LRESULT(0);
                } else if !has_native_border(self.tracking_window) {
                    let _ = self.update_position(Some(SWP_HIDEWINDOW));
                    return LRESULT(0);
                }

                let flags = if !is_window_visible(self.border_window) {
                    Some(SWP_SHOWWINDOW)
                } else {
                    None
                };

                let old_rect = self.window_rect;
                let _ = self.update_window_rect();
                let _ = self.update_position(flags);

                // TODO When a window is minimized, all four of these points go way below 0, and for some
                // reason, render() will sometimes render at this minimized size, even when the
                // render_target size and rounded_rect.rect are changed correctly after an
                // update_window_rect call. So, this is a temporary solution but it should absolutely be
                // looked further into.
                if self.window_rect.top <= 0
                    && self.window_rect.left <= 0
                    && self.window_rect.right <= 0
                    && self.window_rect.bottom <= 0
                {
                    self.window_rect = old_rect;
                } else if get_rect_width(self.window_rect) != get_rect_width(old_rect)
                    || get_rect_height(self.window_rect) != get_rect_height(old_rect)
                {
                    // Only re-render the border when its size changes
                    let _ = self.render();
                }
            }
            // EVENT_OBJECT_REORDER
            WM_APP_1 => {
                if self.pause {
                    return LRESULT(0);
                }

                let _ = self.update_color();
                let _ = self.update_position(None);
                let _ = self.render();
            }
            // EVENT_OBJECT_SHOW / EVENT_OBJECT_UNCLOAKED
            WM_APP_2 => {
                if has_native_border(self.tracking_window) {
                    let _ = self.update_color();
                    let _ = self.update_window_rect();
                    let _ = self.update_position(Some(SWP_SHOWWINDOW));
                    let _ = self.render();
                }
                self.pause = false;
            }
            // EVENT_OBJECT_HIDE / EVENT_OBJECT_CLOAKED
            WM_APP_3 => {
                let _ = self.update_position(Some(SWP_HIDEWINDOW));
                self.pause = true;
            }
            // EVENT_OBJECT_MINIMIZESTART
            WM_APP_4 => {
                let _ = self.update_position(Some(SWP_HIDEWINDOW));
                self.pause = true;
            }
            // EVENT_SYSTEM_MINIMIZEEND
            // When a window is about to be unminimized, hide the border and let the thread sleep
            // for 200ms to wait for the window animation to finish, then show the border.
            WM_APP_5 => {
                thread::sleep(std::time::Duration::from_millis(self.unminimize_delay));

                if has_native_border(self.tracking_window) {
                    let _ = self.update_window_rect();
                    let _ = self.update_position(Some(SWP_SHOWWINDOW));
                    let _ = self.render();
                }
                self.pause = false;
            }
            // TODO i have not tested if this actually works yet
            WM_PAINT => {
                println!("in wm_paint");

                // Same as what LGUG2Z has in komorebi. Should stop the WM_PAINT messages.
                let _ = BeginPaint(window, &mut PAINTSTRUCT::default());
                let _ = EndPaint(window, &PAINTSTRUCT::default());
            }
            WM_DESTROY => {
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                PostQuitMessage(0);
            }
            // Ignore these window position messages
            WM_WINDOWPOSCHANGING => {}
            WM_WINDOWPOSCHANGED => {}
            _ => {
                return DefWindowProcW(window, message, wparam, lparam);
            }
        }
        LRESULT(0)
    }
}
