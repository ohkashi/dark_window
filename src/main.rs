#![windows_subsystem = "windows"]
#![allow(dead_code)]

use std::ffi::c_void;
use std::{fs::File, path::Path};
use std::{mem, ptr};
use std::sync::atomic::{ AtomicBool, Ordering };

use log::*;
use simplelog::*;
use time::macros::format_description;
use encoding_rs::EUC_KR;

use windows::{
    core::*,
    Win32::Foundation::*, Win32::System::Registry::*,
    Win32::Graphics::Dwm::*, Win32::Graphics::Gdi::*, Win32::UI::HiDpi::*, Win32::UI::Controls::*,
    Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress, LoadLibraryA},
    Win32::UI::WindowsAndMessaging::*,
};

static IS_DARK_MODE: AtomicBool = AtomicBool::new(false);
static IS_FIRST_PAINT: AtomicBool = AtomicBool::new(true);

fn check_dark_mode() {
    unsafe {
        let mut key = HKEY::default();
        let _ = RegOpenKeyExA(HKEY_CURRENT_USER, s!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"), Some(0), KEY_READ, &mut key);
        let mut len = std::mem::size_of::<u32>() as u32;
        let mut buffer = vec![0u8; len as usize];
        let _ = RegQueryValueExA(key, s!("AppsUseLightTheme"), None, None, Some(buffer.as_mut_ptr() as _), Some(&mut len));
        let _ = RegCloseKey(key);
        if buffer[0] == 0 {
            IS_DARK_MODE.store(true, Ordering::Relaxed)
        } else {
            IS_DARK_MODE.store(false, Ordering::Relaxed)
        }
    }
}

fn enable_dark_mode(hwnd: HWND, enable: bool) {
    let value:u32 = enable as u32;
    unsafe {
        DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &value as *const u32 as *const _, std::mem::size_of::<u32>() as u32).unwrap();
    }
}

pub const DWMWA_SYSTEMBACKDROP_TYPE: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(38i32);

#[allow(non_camel_case_types)]
type DWM_SYSTEMBACKDROP_TYPE = u32;

const DWMSBT_AUTO: DWM_SYSTEMBACKDROP_TYPE = 0;
const DWMSBT_NONE: DWM_SYSTEMBACKDROP_TYPE = 1;
const DWMSBT_MAINWINDOW: DWM_SYSTEMBACKDROP_TYPE = 2;
const DWMSBT_TRANSIENTWINDOW: DWM_SYSTEMBACKDROP_TYPE = 3;
const DWMSBT_TABBEDWINDOW: DWM_SYSTEMBACKDROP_TYPE = 4;

fn set_backdrop_type(hwnd: HWND, backdrop: DWM_SYSTEMBACKDROP_TYPE) -> bool {
    let value:u32 = backdrop as u32;
    let res = unsafe {
        DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &value as *const u32 as *const _, std::mem::size_of::<u32>() as u32)
    };
    return match res {
        Err(_) => { error!("DWMWA_SYSTEMBACKDROP_TYPE invalid parameter"); false },
        Ok(_) => true,
    };
}

type FnSetWindowCompositionAttribute = unsafe extern "system" fn(HWND, *mut WINDOWCOMPOSITIONATTRIBDATA) -> BOOL;

#[allow(clippy::upper_case_acronyms)]
type WINDOWCOMPOSITIONATTRIB = u32;
const WCA_ACCENT_POLICY: WINDOWCOMPOSITIONATTRIB = 19;
const WCA_USEDARKMODECOLORS: WINDOWCOMPOSITIONATTRIB = 26;

#[allow(non_snake_case)]
#[allow(clippy::upper_case_acronyms)]
#[repr(C)]
struct WINDOWCOMPOSITIONATTRIBDATA {
    Attrib: WINDOWCOMPOSITIONATTRIB,
    pvData: *mut c_void,
    cbData: usize,
}

#[allow(non_snake_case)]
#[allow(clippy::upper_case_acronyms)]
#[allow(non_camel_case_types)]
#[repr(C)]
struct ACCENT_POLICY {
	AccentState: ACCENT_STATE,
	AccentFlags: u32,
	GradientColor: u32,
	AnimationId: u32
}

#[allow(non_camel_case_types)]
type ACCENT_STATE = u32;
#[allow(dead_code)]
const ACCENT_DISABLED: ACCENT_STATE = 0;
const ACCENT_ENABLE_GRADIENT: ACCENT_STATE = 1;
const ACCENT_ENABLE_TRANSPARENTGRADIENT: ACCENT_STATE = 2;
const ACCENT_ENABLE_BLURBEHIND: ACCENT_STATE = 3;
const ACCENT_ENABLE_ACRYLICBLURBEHIND: ACCENT_STATE = 4;
const ACCENT_INVALID_STATE: ACCENT_STATE = 5;

fn enable_blur_behind(hwnd: HWND) -> bool {
    let bb = DWM_BLURBEHIND {
        dwFlags: DWM_BB_ENABLE,
        fEnable: true.into(),
        hRgnBlur: HRGN(0 as *mut c_void),
        fTransitionOnMaximized: false.into(),
    };

    unsafe { DwmEnableBlurBehindWindow(hwnd, &bb).unwrap(); };
    true
}

fn set_window_blur(hwnd: HWND, accent_state: ACCENT_STATE) -> bool {
    unsafe {
        let dll_handle = LoadLibraryA(s!("user32.dll"));
        if dll_handle.is_err() {
            println!("Failed to load DLL: {}", dll_handle.err().unwrap());
            return false;
        }
        let function = GetProcAddress(dll_handle.unwrap(), s!("SetWindowCompositionAttribute"));
        if function.is_none() {
            println!("SetWindowCompositionAttribute entry point not found!");
            return false;
        }
        let mut policy = ACCENT_POLICY {
            AccentState: accent_state,
            AccentFlags: 0,
            GradientColor: (0x40 << 24) | (0x2f2f2f & 0xFFFFFF),
            AnimationId: 0
        };

        let mut data = WINDOWCOMPOSITIONATTRIBDATA {
            Attrib: WCA_ACCENT_POLICY,
            pvData: &mut policy as *mut _ as _,
            cbData: std::mem::size_of_val(&policy) as _,
        };

        let set_wnd_composition_attr: FnSetWindowCompositionAttribute = mem::transmute(function);
        return set_wnd_composition_attr(hwnd, &mut data).as_bool();
    };
}

fn main() -> Result<()> {
    unsafe {
        let instance = GetModuleHandleA(None).unwrap();
        debug_assert!(instance.0 != ptr::null_mut());

        let exe_path = std::env::current_exe().unwrap();
        let exename = exe_path.file_name().unwrap().to_str().unwrap();
        let fname = Path::new(exename).file_stem().unwrap();
        let log_fname = Path::new(fname).with_extension("log");

        let config = ConfigBuilder::new()
            .set_time_format_custom(format_description!("[year]-[month]-[day] [hour]:[minute]:[second]"))
            .set_time_offset_to_local() // Or set a fixed offset
            .unwrap() // Handle potential error if local offset can't be determined
            .build();
        let config2 = ConfigBuilder::new()
            .set_time_format_custom(format_description!("[year]-[month]-[day] [hour]:[minute]:[second]"))
            .set_time_offset_to_local() // Or set a fixed offset
            .unwrap() // Handle potential error if local offset can't be determined
            .build();

        CombinedLogger::init(vec![
            //#[cfg(feature = "termcolor")]
            TermLogger::new(
                LevelFilter::Warn,
                config,
                TerminalMode::Mixed,
                ColorChoice::Auto,
            ),
            //#[cfg(not(feature = "termcolor"))]
            SimpleLogger::new(LevelFilter::Warn, Config::default()),
            WriteLogger::new(
                LevelFilter::Trace,
                config2,
                File::create(log_fname).unwrap(),
            ),
        ]).unwrap();

        SetProcessDpiAwareness(PROCESS_PER_MONITOR_DPI_AWARE).unwrap();
        check_dark_mode();

        let class_name = fname.to_string_lossy() + "\0";
        let window_class = PCSTR::from_raw(class_name.as_ptr());

        let wc = WNDCLASSA {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance.into(),
            lpszClassName: window_class,

            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            ..Default::default()
        };

        let atom = RegisterClassA(&wc);
        debug_assert!(atom != 0);

        let dpi: i32 = GetDpiForSystem() as i32;
        let cx = 320 * dpi / 96;
        let cy = (240 * dpi / 96) + GetSystemMetrics(SM_CYCAPTION);
        let cx_scrn = GetSystemMetrics(SM_CXFULLSCREEN);
        let cy_scrn = GetSystemMetrics(SM_CYFULLSCREEN);
        let x = (cx_scrn - cx) / 2;
        let y = (cy_scrn - cy) / 2;
        println!("dpi = {}, cx = {}, cy = {}", dpi, cx, cy);

        let _hwnd = CreateWindowExA(
            WINDOW_EX_STYLE::default(),
            window_class,
            s!("Dark Window"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            x,
            y,
            cx,
            cy,
            None,
            None,
            Some(instance.into()),
            None,
        );

        let mut message = MSG::default();
        while GetMessageA(&mut message, None, 0, 0).into() {
            DispatchMessageA(&message);
        }

        Ok(())
    }
}

extern "system" fn wndproc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match message {
            WM_CREATE => {
                enable_dark_mode(window, IS_DARK_MODE.load(Ordering::Relaxed));
                if !set_backdrop_type(window, DWMSBT_TRANSIENTWINDOW) {
                    let margins = MARGINS {
                        cxLeftWidth: -1,
                        cxRightWidth: -1,
                        cyTopHeight: -1,
                        cyBottomHeight: -1
                    };
                    DwmExtendFrameIntoClientArea(window, &margins).unwrap();
                    set_window_blur(window, ACCENT_ENABLE_BLURBEHIND);
                } else {
                    enable_blur_behind(window);
                }
                LRESULT(0)
            }
            WM_ERASEBKGND => {
                let hdc = HDC(wparam.0 as _);
                let mut rect = RECT::default();
                let _ = GetClientRect(window, &mut rect);
                let clr_text: COLORREF;
                let clr_bk: COLORREF;
                match IS_DARK_MODE.load(Ordering::Relaxed) {
                    false => {
                        clr_bk = COLORREF(0x00ffffff);
                        clr_text = COLORREF(0);
                    },
                    true => {
                        clr_bk = COLORREF(0x004f061f);
                        clr_text = COLORREF(0x00ffffff);
                    },
                };
                let brush = CreateSolidBrush(clr_bk);
                FillRect(hdc, &rect as *const RECT, brush);
                let _ = DeleteObject(brush.into());

                SetBkColor(hdc, clr_bk);
                SetTextColor(hdc, clr_text);
                let mut rc_text = RECT::from(rect);
                rc_text.top = rect.bottom / 3 + 16;
                let text = String::from("Hello world!\n안녕 세상!");
                let (ansi, _, _) = EUC_KR.encode(&text);
                DrawTextA(hdc, ansi.into_owned().as_mut(), &mut rc_text, DT_CENTER | DT_NOPREFIX);
                if IS_FIRST_PAINT.load(Ordering::Relaxed) {
                    IS_FIRST_PAINT.store(false, Ordering::Relaxed);
                    debug!("first paint.");
                    let _ = RedrawWindow(Some(window), None, None, RDW_FRAME | RDW_INVALIDATE);
                    let _ = InvalidateRect(Some(window), None, true);
                }
                LRESULT(0)
            }
            /*WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(window, &mut ps);
                EndPaint(window, &ps);
                LRESULT(0)
            }*/
            WM_DESTROY => {
                debug!("WM_DESTROY");
                PostQuitMessage(0);
                LRESULT(0)
            }
            WM_SETTINGCHANGE => {
                debug!("WM_SETTINGCHANGE");
                check_dark_mode();
                enable_dark_mode(window, IS_DARK_MODE.load(Ordering::Relaxed));
                let _ = InvalidateRect(Some(window), None, true);
                LRESULT(0)
            }
            _ => DefWindowProcA(window, message, wparam, lparam),
        }
    }
}
