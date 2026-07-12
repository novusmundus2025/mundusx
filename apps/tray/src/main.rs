#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("mundusx-tray is available on Windows only");
}

#[cfg(windows)]
mod windows_tray {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt, path::PathBuf, process::Command, ptr};
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Shell::{Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW},
            WindowsAndMessaging::{
                AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
                DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW,
                PostQuitMessage, RegisterClassW, SetForegroundWindow, TrackPopupMenu,
                TranslateMessage, CREATESTRUCTW, CW_USEDEFAULT, IDI_APPLICATION, MF_SEPARATOR,
                MF_STRING, MSG, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON, WM_APP,
                WM_COMMAND, WM_CREATE, WM_DESTROY, WM_LBUTTONDBLCLK, WM_RBUTTONUP, WNDCLASSW,
                WS_OVERLAPPED,
            },
        },
    };

    const TRAY_MESSAGE: u32 = WM_APP + 1;
    const MENU_OPEN: usize = 1001;
    const MENU_PAUSE: usize = 1002;
    const MENU_RESUME: usize = 1003;
    const MENU_EXIT: usize = 1004;

    fn wide(value: &str) -> Vec<u16> {
        OsStr::new(value).encode_wide().chain(Some(0)).collect()
    }

    fn cli_path() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|parent| parent.join("opengpu.exe")))
            .filter(|path| path.is_file())
            .unwrap_or_else(|| PathBuf::from("opengpu.exe"))
    }

    fn run_cli(argument: &str) {
        let _ = Command::new(cli_path()).arg(argument).spawn();
    }

    fn open_dashboard() {
        let _ = Command::new(cli_path()).arg("status").spawn();
    }

    unsafe fn show_menu(hwnd: HWND) {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        let open = wide("Open MundusX status");
        let pause = wide("Pause contribution");
        let resume = wide("Resume contribution");
        let exit = wide("Exit tray");
        AppendMenuW(menu, MF_STRING, MENU_OPEN, open.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
        AppendMenuW(menu, MF_STRING, MENU_PAUSE, pause.as_ptr());
        AppendMenuW(menu, MF_STRING, MENU_RESUME, resume.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
        AppendMenuW(menu, MF_STRING, MENU_EXIT, exit.as_ptr());

        let mut point = POINT { x: 0, y: 0 };
        GetCursorPos(&mut point);
        SetForegroundWindow(hwnd);
        TrackPopupMenu(
            menu,
            TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            hwnd,
            ptr::null(),
        );
        DestroyMenu(menu);
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_CREATE => {
                let create = lparam as *const CREATESTRUCTW;
                if !create.is_null() {
                    let _ = (*create).lpCreateParams;
                }
                0
            }
            TRAY_MESSAGE => {
                match lparam as u32 {
                    WM_RBUTTONUP => show_menu(hwnd),
                    WM_LBUTTONDBLCLK => open_dashboard(),
                    _ => {}
                }
                0
            }
            WM_COMMAND => {
                match wparam & 0xffff {
                    MENU_OPEN => open_dashboard(),
                    MENU_PAUSE => run_cli("pause"),
                    MENU_RESUME => run_cli("resume"),
                    MENU_EXIT => {
                        DestroyWindow(hwnd);
                    }
                    _ => {}
                }
                0
            }
            WM_DESTROY => {
                remove_icon(hwnd);
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }

    unsafe fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
        let mut data: NOTIFYICONDATAW = std::mem::zeroed();
        data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = 1;
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = TRAY_MESSAGE;
        data.hIcon = LoadIconW(ptr::null_mut(), IDI_APPLICATION);
        let tip = wide("MundusX contributor");
        let count = tip.len().min(data.szTip.len());
        data.szTip[..count].copy_from_slice(&tip[..count]);
        data
    }

    unsafe fn remove_icon(hwnd: HWND) {
        let mut data = icon_data(hwnd);
        Shell_NotifyIconW(NIM_DELETE, &mut data);
    }

    pub fn run() -> Result<(), String> {
        unsafe {
            let instance = GetModuleHandleW(ptr::null());
            if instance.is_null() {
                return Err("failed to locate the MundusX tray module".to_string());
            }
            let class_name = wide("MundusXContributorTrayWindow");
            let window_class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: class_name.as_ptr(),
                ..std::mem::zeroed()
            };
            if RegisterClassW(&window_class) == 0 {
                return Err("failed to register the MundusX tray window".to_string());
            }
            let title = wide("MundusX");
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPED,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null(),
            );
            if hwnd.is_null() {
                return Err("failed to create the MundusX tray window".to_string());
            }
            let mut data = icon_data(hwnd);
            if Shell_NotifyIconW(NIM_ADD, &mut data) == 0 {
                DestroyWindow(hwnd);
                return Err("failed to add the MundusX tray icon".to_string());
            }

            let mut message: MSG = std::mem::zeroed();
            while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            Ok(())
        }
    }
}

#[cfg(windows)]
fn main() {
    if let Err(error) = windows_tray::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
