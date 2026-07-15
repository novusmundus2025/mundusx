#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("mundusx-tray is available on Windows only");
}

#[cfg(windows)]
mod windows_tray {
    use std::{
        ffi::OsStr,
        os::windows::{ffi::OsStrExt, process::CommandExt},
        path::PathBuf,
        process::Command,
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Shell::{
                Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
                NOTIFYICONDATAW,
            },
            WindowsAndMessaging::{
                AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
                DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, LoadImageW,
                MessageBoxW, PostQuitMessage, RegisterClassW, SetForegroundWindow, TrackPopupMenu,
                TranslateMessage, CREATESTRUCTW, CW_USEDEFAULT, IDI_APPLICATION, IMAGE_ICON,
                LR_DEFAULTSIZE, LR_LOADFROMFILE, MB_ICONERROR, MB_ICONINFORMATION, MB_OK,
                MF_SEPARATOR, MF_STRING, MSG, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON,
                WM_APP, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_LBUTTONDBLCLK, WM_RBUTTONUP,
                WNDCLASSW, WS_OVERLAPPED,
            },
        },
    };

    const CREATE_NO_WINDOW: u32 = 0x08000000;
    const TRAY_MESSAGE: u32 = WM_APP + 1;
    const MENU_STATUS: usize = 1001;
    const MENU_SETUP: usize = 1002;
    const MENU_ONBOARDING: usize = 1003;
    const MENU_CAP: usize = 1004;
    const MENU_CREDITS: usize = 1005;
    const MENU_MODEL_LIST: usize = 1006;
    const MENU_MODEL_USE: usize = 1007;
    const MENU_MODEL_ADD: usize = 1008;
    const MENU_DOCTOR: usize = 1009;
    const MENU_LOGS: usize = 1010;
    const MENU_START: usize = 1011;
    const MENU_PAUSE: usize = 1012;
    const MENU_RESUME: usize = 1013;
    const MENU_DISCONNECT: usize = 1014;
    const MENU_EXIT: usize = 1015;

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

    fn tray_icon_path() -> Option<PathBuf> {
        let exe_icon = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|parent| parent.join("mundusx.ico")));
        if exe_icon.as_ref().is_some_and(|path| path.is_file()) {
            return exe_icon;
        }

        let dev_icon = std::env::current_dir().ok().map(|cwd| {
            cwd.join("apps")
                .join("tray")
                .join("assets")
                .join("mundusx.ico")
        });
        if dev_icon.as_ref().is_some_and(|path| path.is_file()) {
            return dev_icon;
        }

        None
    }

    fn run_cli(args: &[&str]) {
        let _ = Command::new(cli_path())
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }

    fn show_message(title: &str, body: &str, error: bool) {
        let title = wide(title);
        let body = wide(body);
        unsafe {
            MessageBoxW(
                ptr::null_mut(),
                body.as_ptr(),
                title.as_ptr(),
                MB_OK
                    | if error {
                        MB_ICONERROR
                    } else {
                        MB_ICONINFORMATION
                    },
            );
        }
    }

    fn short_text(value: &str) -> String {
        const LIMIT: usize = 6000;
        if value.chars().count() <= LIMIT {
            return value.to_string();
        }

        let mut clipped: String = value.chars().take(LIMIT).collect();
        clipped.push_str("\n\n... output clipped. Use `opengpu logs` for full details.");
        clipped
    }

    fn cli_output(args: &[&str]) -> Result<String, String> {
        let output = Command::new(cli_path())
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|error| format!("failed to run opengpu {}: {error}", args.join(" ")))?;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let mut body = String::new();
        if !stdout.is_empty() {
            body.push_str(&stdout);
        }
        if !stderr.is_empty() {
            if !body.is_empty() {
                body.push_str("\n\n");
            }
            body.push_str(&stderr);
        }
        if body.is_empty() {
            body = format!("opengpu {} completed.", args.join(" "));
        }

        if output.status.success() {
            Ok(short_text(&body))
        } else {
            Err(short_text(&body))
        }
    }

    fn run_cli_app_output(title: &str, args: &[&str]) {
        match cli_output(args) {
            Ok(body) => show_message(title, &body, false),
            Err(body) => show_message(title, &body, true),
        }
    }

    fn powershell_quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }

    fn powershell_cli_command(args: &[&str]) -> String {
        let mut command = format!("& {}", powershell_quote(&cli_path().display().to_string()));
        for arg in args {
            command.push(' ');
            command.push_str(&powershell_quote(arg));
        }
        command
    }

    fn run_cli_window(args: &[&str]) {
        let command = format!(
            "{}; Write-Host ''; Write-Host 'Press Enter to close this window.'; Read-Host",
            powershell_cli_command(args)
        );
        let _ = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-NoExit",
                "-Command",
                &command,
            ])
            .spawn();
    }

    fn open_status_window() {
        run_cli_app_output("MundusX Status", &["status"]);
    }

    unsafe fn append_menu_item(menu: *mut core::ffi::c_void, id: usize, label: &str) {
        let label = wide(label);
        AppendMenuW(menu, MF_STRING, id, label.as_ptr());
    }

    unsafe fn append_separator(menu: *mut core::ffi::c_void) {
        AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
    }

    unsafe fn show_menu(hwnd: HWND) {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        append_menu_item(menu, MENU_STATUS, "Status");
        append_menu_item(menu, MENU_CREDITS, "Credits and earnings");
        append_separator(menu);
        append_menu_item(menu, MENU_SETUP, "Guided setup");
        append_menu_item(menu, MENU_ONBOARDING, "Onboarding checklist");
        append_menu_item(menu, MENU_CAP, "Contribution cap");
        append_separator(menu);
        append_menu_item(menu, MENU_MODEL_LIST, "Model cache");
        append_menu_item(menu, MENU_MODEL_USE, "Choose active model");
        append_menu_item(menu, MENU_MODEL_ADD, "Download another model");
        append_separator(menu);
        append_menu_item(menu, MENU_DOCTOR, "Diagnostics");
        append_menu_item(menu, MENU_LOGS, "Logs");
        append_separator(menu);
        append_menu_item(menu, MENU_START, "Start in background");
        append_menu_item(menu, MENU_PAUSE, "Pause contribution");
        append_menu_item(menu, MENU_RESUME, "Resume contribution");
        append_menu_item(menu, MENU_DISCONNECT, "Disconnect and cool GPU");
        append_separator(menu);
        append_menu_item(menu, MENU_EXIT, "Exit app");

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
                    WM_LBUTTONDBLCLK => open_status_window(),
                    _ => {}
                }
                0
            }
            WM_COMMAND => {
                match wparam & 0xffff {
                    MENU_STATUS => open_status_window(),
                    MENU_SETUP => run_cli_window(&["install"]),
                    MENU_ONBOARDING => run_cli_app_output("MundusX Onboarding", &["onboarding"]),
                    MENU_CAP => run_cli_window(&["cap"]),
                    MENU_CREDITS => run_cli_app_output("MundusX Credits", &["credits"]),
                    MENU_MODEL_LIST => {
                        run_cli_app_output("MundusX Model Cache", &["model", "list"])
                    }
                    MENU_MODEL_USE => run_cli_window(&["model", "use"]),
                    MENU_MODEL_ADD => run_cli_window(&["model", "add"]),
                    MENU_DOCTOR => run_cli_app_output("MundusX Diagnostics", &["doctor"]),
                    MENU_LOGS => run_cli_app_output("MundusX Logs", &["logs"]),
                    MENU_START => run_cli_app_output("MundusX Start", &["start", "--background"]),
                    MENU_PAUSE => run_cli(&["pause"]),
                    MENU_RESUME => run_cli_app_output("MundusX Resume", &["resume"]),
                    MENU_DISCONNECT => run_cli_app_output("MundusX Disconnect", &["exit"]),
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
        data.hIcon = tray_icon_path()
            .and_then(|path| {
                let path = wide(&path.display().to_string());
                let icon = LoadImageW(
                    ptr::null_mut(),
                    path.as_ptr(),
                    IMAGE_ICON,
                    0,
                    0,
                    LR_LOADFROMFILE | LR_DEFAULTSIZE,
                );
                if icon.is_null() {
                    None
                } else {
                    Some(icon)
                }
            })
            .unwrap_or_else(|| LoadIconW(ptr::null_mut(), IDI_APPLICATION));
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

    #[cfg(test)]
    mod tests {
        use super::powershell_cli_command;

        #[test]
        fn powershell_command_keeps_multi_arg_cli_commands() {
            let command = powershell_cli_command(&["model", "use"]);

            assert!(command.contains("'model' 'use'"));
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
