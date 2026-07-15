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
        ptr, thread,
    };
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
        Graphics::Gdi::{CreateSolidBrush, DeleteObject, SetBkColor, SetTextColor},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Shell::{
                Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
                NOTIFYICONDATAW,
            },
            WindowsAndMessaging::{
                AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
                DestroyWindow, DispatchMessageW, GetClientRect, GetCursorPos, GetMessageW,
                GetWindowLongPtrW, LoadIconW, LoadImageW, MoveWindow, PostMessageW,
                PostQuitMessage, RegisterClassW, SetForegroundWindow, SetWindowLongPtrW,
                SetWindowTextW, ShowWindow, TrackPopupMenu, TranslateMessage, CREATESTRUCTW,
                CW_USEDEFAULT, ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY, GWLP_USERDATA,
                IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTSIZE, LR_LOADFROMFILE, MF_SEPARATOR,
                MF_STRING, MSG, SW_RESTORE, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON,
                WM_APP, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_CTLCOLORBTN, WM_CTLCOLOREDIT,
                WM_CTLCOLORSTATIC, WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_RBUTTONUP,
                WM_SIZE, WNDCLASSW, WS_BORDER, WS_CHILD, WS_OVERLAPPED, WS_OVERLAPPEDWINDOW,
                WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
            },
        },
    };

    const CREATE_NO_WINDOW: u32 = 0x08000000;
    const TRAY_MESSAGE: u32 = WM_APP + 1;
    const DASHBOARD_RESULT_MESSAGE: u32 = WM_APP + 2;
    static mut DASHBOARD_HWND: HWND = ptr::null_mut();
    const OUTPUT_CLOSE: usize = 2001;
    const DASH_REFRESH: usize = 3001;
    const DASH_CREDITS: usize = 3002;
    const DASH_MODELS: usize = 3003;
    const DASH_DOCTOR: usize = 3004;
    const DASH_LOGS: usize = 3005;
    const DASH_START: usize = 3006;
    const DASH_PAUSE: usize = 3007;
    const DASH_RESUME: usize = 3008;
    const DASH_DISCONNECT: usize = 3009;
    const DASH_SETUP: usize = 3010;
    const DASH_CAP: usize = 3011;
    const DASH_MODEL_USE: usize = 3012;
    const DASH_MODEL_ADD: usize = 3013;
    const DASH_CLOSE: usize = 3014;
    const COLOR_BACKGROUND: u32 = 0x00faf9f7;
    const COLOR_PANEL: u32 = 0x00ffffff;
    const COLOR_TEXT: u32 = 0x00201714;
    const COLOR_SUCCESS: u32 = 0x004aa316;
    const COLOR_ERROR: u32 = 0x001c1cb9;
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

    struct OutputWindowState {
        title: Vec<u16>,
        status: Vec<u16>,
        body: Vec<u16>,
        error: bool,
        title_hwnd: HWND,
        status_hwnd: HWND,
        body_hwnd: HWND,
        close_hwnd: HWND,
        background_brush: *mut core::ffi::c_void,
        panel_brush: *mut core::ffi::c_void,
    }

    impl OutputWindowState {
        fn new(title: &str, body: &str, error: bool) -> Self {
            let normalized = body.replace('\n', "\r\n");
            Self {
                title: wide(title),
                status: wide(if error {
                    "Action failed"
                } else {
                    "Action completed"
                }),
                body: wide(&normalized),
                error,
                title_hwnd: ptr::null_mut(),
                status_hwnd: ptr::null_mut(),
                body_hwnd: ptr::null_mut(),
                close_hwnd: ptr::null_mut(),
                background_brush: unsafe { CreateSolidBrush(COLOR_BACKGROUND) },
                panel_brush: unsafe { CreateSolidBrush(COLOR_PANEL) },
            }
        }
    }

    fn window_state(hwnd: HWND) -> Option<&'static mut OutputWindowState> {
        let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut OutputWindowState };
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { &mut *ptr })
        }
    }

    unsafe fn layout_output_window(hwnd: HWND, state: &OutputWindowState) {
        let mut rect = std::mem::zeroed();
        GetClientRect(hwnd, &mut rect);
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        let margin = 24;
        MoveWindow(state.title_hwnd, margin, 18, width - margin * 2, 28, 1);
        MoveWindow(state.status_hwnd, margin, 52, width - margin * 2, 24, 1);
        MoveWindow(
            state.body_hwnd,
            margin,
            88,
            width - margin * 2,
            height - 150,
            1,
        );
        MoveWindow(
            state.close_hwnd,
            width - margin - 110,
            height - 44,
            110,
            30,
            1,
        );
    }

    unsafe extern "system" fn output_window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_CREATE => {
                let create = lparam as *const CREATESTRUCTW;
                if create.is_null() {
                    return -1;
                }
                let state_ptr = (*create).lpCreateParams as *mut OutputWindowState;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);
                let state = &mut *state_ptr;
                state.title_hwnd = CreateWindowExW(
                    0,
                    wide("STATIC").as_ptr(),
                    state.title.as_ptr(),
                    WS_CHILD | WS_VISIBLE,
                    0,
                    0,
                    0,
                    0,
                    hwnd,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null(),
                );
                state.status_hwnd = CreateWindowExW(
                    0,
                    wide("STATIC").as_ptr(),
                    state.status.as_ptr(),
                    WS_CHILD | WS_VISIBLE,
                    0,
                    0,
                    0,
                    0,
                    hwnd,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null(),
                );
                state.body_hwnd = CreateWindowExW(
                    0,
                    wide("EDIT").as_ptr(),
                    state.body.as_ptr(),
                    WS_CHILD
                        | WS_VISIBLE
                        | WS_BORDER
                        | WS_VSCROLL
                        | ES_MULTILINE as u32
                        | ES_AUTOVSCROLL as u32
                        | ES_READONLY as u32,
                    0,
                    0,
                    0,
                    0,
                    hwnd,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null(),
                );
                state.close_hwnd = CreateWindowExW(
                    0,
                    wide("BUTTON").as_ptr(),
                    wide("Close").as_ptr(),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                    0,
                    0,
                    0,
                    0,
                    hwnd,
                    OUTPUT_CLOSE as _,
                    ptr::null_mut(),
                    ptr::null(),
                );
                layout_output_window(hwnd, state);
                0
            }
            WM_SIZE => {
                if let Some(state) = window_state(hwnd) {
                    layout_output_window(hwnd, state);
                }
                0
            }
            WM_COMMAND => {
                if wparam & 0xffff == OUTPUT_CLOSE {
                    DestroyWindow(hwnd);
                }
                0
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORBTN => {
                if let Some(state) = window_state(hwnd) {
                    let hdc = wparam as _;
                    SetBkColor(hdc, COLOR_PANEL);
                    SetTextColor(
                        hdc,
                        if message == WM_CTLCOLORSTATIC && state.error {
                            COLOR_ERROR
                        } else if message == WM_CTLCOLORSTATIC {
                            COLOR_SUCCESS
                        } else {
                            COLOR_TEXT
                        },
                    );
                    return state.panel_brush as isize;
                }
                0
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut OutputWindowState;
                if !state_ptr.is_null() {
                    let state = Box::from_raw(state_ptr);
                    DeleteObject(state.background_brush);
                    DeleteObject(state.panel_brush);
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
                0
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }

    fn show_output_window(title: &str, body: &str, error: bool) {
        unsafe {
            let instance = GetModuleHandleW(ptr::null());
            if instance.is_null() {
                return;
            }
            let class_name = wide("MundusXOutputWindow");
            let window_class = WNDCLASSW {
                lpfnWndProc: Some(output_window_proc),
                hInstance: instance,
                lpszClassName: class_name.as_ptr(),
                hbrBackground: CreateSolidBrush(COLOR_BACKGROUND),
                ..std::mem::zeroed()
            };
            RegisterClassW(&window_class);

            let state = Box::new(OutputWindowState::new(title, body, error));
            let state_ptr = Box::into_raw(state);
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                wide("MundusX Contributor").as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                760,
                520,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                state_ptr as *const _,
            );
            if hwnd.is_null() {
                let state = Box::from_raw(state_ptr);
                DeleteObject(state.background_brush);
                DeleteObject(state.panel_brush);
            }
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
            Ok(body) => show_output_window(title, &body, false),
            Err(body) => show_output_window(title, &body, true),
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

    struct DashboardState {
        logo_hwnd: HWND,
        sidebar_status_hwnd: HWND,
        sidebar_node_hwnd: HWND,
        title_hwnd: HWND,
        subtitle_hwnd: HWND,
        status_hwnd: HWND,
        metric_hwnds: Vec<HWND>,
        section_hwnds: Vec<HWND>,
        output_hwnd: HWND,
        buttons: Vec<HWND>,
        background_brush: *mut core::ffi::c_void,
        panel_brush: *mut core::ffi::c_void,
    }

    impl DashboardState {
        fn new() -> Self {
            Self {
                logo_hwnd: ptr::null_mut(),
                sidebar_status_hwnd: ptr::null_mut(),
                sidebar_node_hwnd: ptr::null_mut(),
                title_hwnd: ptr::null_mut(),
                subtitle_hwnd: ptr::null_mut(),
                status_hwnd: ptr::null_mut(),
                metric_hwnds: Vec::new(),
                section_hwnds: Vec::new(),
                output_hwnd: ptr::null_mut(),
                buttons: Vec::new(),
                background_brush: unsafe { CreateSolidBrush(COLOR_BACKGROUND) },
                panel_brush: unsafe { CreateSolidBrush(COLOR_PANEL) },
            }
        }
    }

    struct DashboardCommandResult {
        status: String,
        body: String,
    }

    fn dashboard_state(hwnd: HWND) -> Option<&'static mut DashboardState> {
        let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DashboardState };
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { &mut *ptr })
        }
    }

    unsafe fn create_dashboard_button(hwnd: HWND, id: usize, label: &str, buttons: &mut Vec<HWND>) {
        let button = CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide(label).as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            0,
            0,
            0,
            0,
            hwnd,
            id as _,
            ptr::null_mut(),
            ptr::null(),
        );
        buttons.push(button);
    }

    unsafe fn create_dashboard_static(
        hwnd: HWND,
        label: &str,
        bordered: bool,
        controls: &mut Vec<HWND>,
    ) {
        let style = if bordered {
            WS_CHILD | WS_VISIBLE | WS_BORDER
        } else {
            WS_CHILD | WS_VISIBLE
        };
        let control = CreateWindowExW(
            0,
            wide("STATIC").as_ptr(),
            wide(label).as_ptr(),
            style,
            0,
            0,
            0,
            0,
            hwnd,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null(),
        );
        controls.push(control);
    }

    unsafe fn layout_dashboard_window(hwnd: HWND, state: &DashboardState) {
        let mut rect = std::mem::zeroed();
        GetClientRect(hwnd, &mut rect);
        let width = (rect.right - rect.left).max(1040);
        let height = (rect.bottom - rect.top).max(680);
        let sidebar_width = 214;
        let main_x = sidebar_width + 28;
        let main_width = width - main_x - 28;
        let gap = 12;
        let card_width = ((main_width - gap * 3) / 4).max(150);

        MoveWindow(state.logo_hwnd, 30, 28, sidebar_width - 58, 42, 1);
        MoveWindow(
            state.sidebar_status_hwnd,
            22,
            472,
            sidebar_width - 44,
            84,
            1,
        );
        MoveWindow(
            state.sidebar_node_hwnd,
            22,
            height - 126,
            sidebar_width - 44,
            86,
            1,
        );
        MoveWindow(state.title_hwnd, main_x, 24, main_width - 190, 28, 1);
        MoveWindow(state.subtitle_hwnd, main_x, 56, main_width - 190, 22, 1);
        MoveWindow(state.status_hwnd, width - 170, 24, 138, 30, 1);

        for (index, button) in state.buttons.iter().enumerate() {
            if index < 8 {
                MoveWindow(
                    *button,
                    22,
                    92 + index as i32 * 38,
                    sidebar_width - 44,
                    30,
                    1,
                );
            } else if index == 8 {
                MoveWindow(*button, width - 170, 62, 138, 30, 1);
            } else {
                let local = index as i32 - 9;
                let col = local % 3;
                let row = local / 3;
                MoveWindow(*button, main_x + col * 118, 374 + row * 38, 106, 30, 1);
            }
        }

        for (index, card) in state.metric_hwnds.iter().enumerate() {
            MoveWindow(
                *card,
                main_x + index as i32 * (card_width + gap),
                102,
                card_width,
                96,
                1,
            );
        }

        let left_w = ((main_width * 62) / 100).max(430);
        let right_x = main_x + left_w + 16;
        let right_w = main_width - left_w - 16;
        if state.section_hwnds.len() >= 4 {
            MoveWindow(state.section_hwnds[0], main_x, 216, left_w, 142, 1);
            MoveWindow(state.section_hwnds[1], right_x, 216, right_w, 142, 1);
            MoveWindow(state.section_hwnds[2], main_x, 472, left_w, 108, 1);
            MoveWindow(state.section_hwnds[3], right_x, 374, right_w, 206, 1);
        }

        MoveWindow(state.output_hwnd, main_x, 596, left_w, height - 622, 1);
    }

    unsafe fn set_dashboard_text(state: &DashboardState, status: &str, body: &str) {
        let normalized = body.replace('\n', "\r\n");
        SetWindowTextW(state.status_hwnd, wide(status).as_ptr());
        SetWindowTextW(state.output_hwnd, wide(&normalized).as_ptr());

        if !body.trim().is_empty() {
            let lower = body.to_ascii_lowercase();
            let connected = if lower.contains("connected: yes") {
                "Connected"
            } else if lower.contains("readyforjobs: no") || lower.contains("connected: no") {
                "Standby"
            } else {
                "Checking"
            };
            if !state.metric_hwnds.is_empty() {
                SetWindowTextW(
                    state.metric_hwnds[0],
                    wide(&format!(
                        "Status\r\n\r\n{connected}\r\nAll systems reviewed by control plane"
                    ))
                    .as_ptr(),
                );
            }
            if state.metric_hwnds.len() > 2 {
                let contribution = body
                    .lines()
                    .find_map(|line| line.strip_prefix("contributionPercent:"))
                    .map(str::trim)
                    .unwrap_or("--");
                SetWindowTextW(
                    state.metric_hwnds[2],
                    wide(&format!(
                        "Contribution\r\n\r\n{contribution}\r\nAutomatic routing budget"
                    ))
                    .as_ptr(),
                );
            }
            if state.section_hwnds.len() > 1 {
                let model = body
                    .lines()
                    .find_map(|line| line.strip_prefix("activeModel:"))
                    .map(str::trim)
                    .unwrap_or("Not selected");
                let backend = body
                    .lines()
                    .find_map(|line| line.strip_prefix("detectedBackend:"))
                    .map(str::trim)
                    .unwrap_or("--");
                SetWindowTextW(
                    state.section_hwnds[1],
                    wide(&format!(
                        "Active Model\r\n\r\n{model}\r\n\r\nPerformance: live probe\r\nBackend: {backend}"
                    ))
                    .as_ptr(),
                );
            }
        }
    }

    unsafe fn queue_dashboard_cli(hwnd: HWND, state: &DashboardState, status: &str, args: &[&str]) {
        set_dashboard_text(
            state,
            status,
            &format!("Running `opengpu {}`...", args.join(" ")),
        );
        let hwnd_value = hwnd as isize;
        let args = args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
        thread::spawn(move || {
            let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
            let result = match cli_output(&refs) {
                Ok(body) => DashboardCommandResult {
                    status: "Action completed".to_string(),
                    body,
                },
                Err(body) => DashboardCommandResult {
                    status: "Action failed".to_string(),
                    body,
                },
            };
            let result_ptr = Box::into_raw(Box::new(result));
            let posted = unsafe {
                PostMessageW(
                    hwnd_value as HWND,
                    DASHBOARD_RESULT_MESSAGE,
                    0,
                    result_ptr as LPARAM,
                )
            };
            if posted == 0 {
                unsafe {
                    drop(Box::from_raw(result_ptr));
                }
            }
        });
    }

    unsafe fn queue_dashboard_start(hwnd: HWND, state: &DashboardState) {
        set_dashboard_text(
            state,
            "Starting node",
            "Start requested. The dashboard will refresh status in a few seconds.",
        );
        let hwnd_value = hwnd as isize;
        thread::spawn(move || {
            let start_result = Command::new(cli_path())
                .args(["start", "--background"])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();

            let result = match start_result {
                Ok(_) => {
                    thread::sleep(std::time::Duration::from_secs(3));
                    match cli_output(&["status"]) {
                        Ok(body) => DashboardCommandResult {
                            status: "Start requested".to_string(),
                            body,
                        },
                        Err(body) => DashboardCommandResult {
                            status: "Start requested; status failed".to_string(),
                            body,
                        },
                    }
                }
                Err(error) => DashboardCommandResult {
                    status: "Action failed".to_string(),
                    body: format!("failed to launch `opengpu start --background`: {error}"),
                },
            };

            let result_ptr = Box::into_raw(Box::new(result));
            let posted = unsafe {
                PostMessageW(
                    hwnd_value as HWND,
                    DASHBOARD_RESULT_MESSAGE,
                    0,
                    result_ptr as LPARAM,
                )
            };
            if posted == 0 {
                unsafe {
                    drop(Box::from_raw(result_ptr));
                }
            }
        });
    }

    unsafe fn open_dashboard_shell(state: &DashboardState, status: &str, args: &[&str]) {
        run_cli_window(args);
        set_dashboard_text(
            state,
            status,
            &format!(
                "Opened guided terminal workflow for `opengpu {}`.\r\n\r\nInteractive selector screens still run in a terminal until the installer wizard is moved fully into this app window.",
                args.join(" ")
            ),
        );
    }

    unsafe extern "system" fn dashboard_window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_CREATE => {
                let create = lparam as *const CREATESTRUCTW;
                if create.is_null() {
                    return -1;
                }
                let state_ptr = (*create).lpCreateParams as *mut DashboardState;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);
                let state = &mut *state_ptr;

                state.logo_hwnd = CreateWindowExW(
                    0,
                    wide("STATIC").as_ptr(),
                    wide("MUNDUSX").as_ptr(),
                    WS_CHILD | WS_VISIBLE,
                    0,
                    0,
                    0,
                    0,
                    hwnd,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null(),
                );
                state.sidebar_status_hwnd = CreateWindowExW(
                    0,
                    wide("STATIC").as_ptr(),
                    wide(
                        "Connected\r\n\r\nStuttgart Control Plane\r\nLatency: checking\r\n\r\nNode ID\r\nloading",
                    )
                    .as_ptr(),
                    WS_CHILD | WS_VISIBLE | WS_BORDER,
                    0,
                    0,
                    0,
                    0,
                    hwnd,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null(),
                );
                state.sidebar_node_hwnd = CreateWindowExW(
                    0,
                    wide("STATIC").as_ptr(),
                    wide("Contributor\r\n\r\nMundusX Node\r\nv1.0.0").as_ptr(),
                    WS_CHILD | WS_VISIBLE | WS_BORDER,
                    0,
                    0,
                    0,
                    0,
                    hwnd,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null(),
                );
                state.title_hwnd = CreateWindowExW(
                    0,
                    wide("STATIC").as_ptr(),
                    wide("Good morning, Contributor").as_ptr(),
                    WS_CHILD | WS_VISIBLE,
                    0,
                    0,
                    0,
                    0,
                    hwnd,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null(),
                );
                state.subtitle_hwnd = CreateWindowExW(
                    0,
                    wide("STATIC").as_ptr(),
                    wide("Your node is contributing to the MundusX network.").as_ptr(),
                    WS_CHILD | WS_VISIBLE,
                    0,
                    0,
                    0,
                    0,
                    hwnd,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null(),
                );
                state.status_hwnd = CreateWindowExW(
                    0,
                    wide("STATIC").as_ptr(),
                    wide("Ready").as_ptr(),
                    WS_CHILD | WS_VISIBLE | WS_BORDER,
                    0,
                    0,
                    0,
                    0,
                    hwnd,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null(),
                );
                state.output_hwnd = CreateWindowExW(
                    0,
                    wide("EDIT").as_ptr(),
                    wide("Recent logs and command output will appear here.").as_ptr(),
                    WS_CHILD
                        | WS_VISIBLE
                        | WS_BORDER
                        | WS_VSCROLL
                        | ES_MULTILINE as u32
                        | ES_AUTOVSCROLL as u32
                        | ES_READONLY as u32,
                    0,
                    0,
                    0,
                    0,
                    hwnd,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null(),
                );

                create_dashboard_static(
                    hwnd,
                    "Status\r\n\r\nChecking\r\nWaiting for node status",
                    true,
                    &mut state.metric_hwnds,
                );
                create_dashboard_static(
                    hwnd,
                    "Today's Credits\r\n\r\n--\r\nRefresh credits to load",
                    true,
                    &mut state.metric_hwnds,
                );
                create_dashboard_static(
                    hwnd,
                    "Contribution\r\n\r\n--\r\nAutomatic routing budget",
                    true,
                    &mut state.metric_hwnds,
                );
                create_dashboard_static(
                    hwnd,
                    "Uptime\r\n\r\n--\r\nRefresh status to load",
                    true,
                    &mut state.metric_hwnds,
                );
                create_dashboard_static(
                    hwnd,
                    "Contribution Control\r\n\r\nRunning\r\nContribution is active\r\n\r\nLevel: --\r\nUse Contribution to edit the cap.",
                    true,
                    &mut state.section_hwnds,
                );
                create_dashboard_static(
                    hwnd,
                    "Active Model\r\n\r\nLoading model state\r\n\r\nPerformance: checking\r\nBackend: --",
                    true,
                    &mut state.section_hwnds,
                );
                create_dashboard_static(
                    hwnd,
                    "System Overview\r\n\r\nCPU: --     GPU: --     Memory: --     Temperature: --\r\nHardware metrics will be wired from the node agent next.",
                    true,
                    &mut state.section_hwnds,
                );
                create_dashboard_static(
                    hwnd,
                    "Earnings\r\n\r\nToday: -- credits\r\nThis week: -- credits\r\nThis month: -- credits\r\n\r\nOpen Credits for ledger details.",
                    true,
                    &mut state.section_hwnds,
                );

                create_dashboard_button(hwnd, DASH_REFRESH, "Dashboard", &mut state.buttons);
                create_dashboard_button(hwnd, DASH_MODELS, "Models", &mut state.buttons);
                create_dashboard_button(hwnd, DASH_CAP, "Contribution", &mut state.buttons);
                create_dashboard_button(hwnd, DASH_CREDITS, "Credits", &mut state.buttons);
                create_dashboard_button(hwnd, DASH_DOCTOR, "Identity", &mut state.buttons);
                create_dashboard_button(hwnd, DASH_LOGS, "Logs", &mut state.buttons);
                create_dashboard_button(hwnd, DASH_SETUP, "Settings", &mut state.buttons);
                create_dashboard_button(hwnd, DASH_CLOSE, "Help", &mut state.buttons);
                create_dashboard_button(hwnd, DASH_PAUSE, "Pause Contribution", &mut state.buttons);
                create_dashboard_button(hwnd, DASH_START, "Start", &mut state.buttons);
                create_dashboard_button(hwnd, DASH_DISCONNECT, "Stop", &mut state.buttons);
                create_dashboard_button(hwnd, DASH_RESUME, "Resume", &mut state.buttons);
                create_dashboard_button(hwnd, DASH_MODEL_USE, "Choose model", &mut state.buttons);
                create_dashboard_button(hwnd, DASH_MODEL_ADD, "Add model", &mut state.buttons);

                layout_dashboard_window(hwnd, state);
                queue_dashboard_cli(hwnd, state, "Loading status", &["status"]);
                0
            }
            DASHBOARD_RESULT_MESSAGE => {
                if let Some(state) = dashboard_state(hwnd) {
                    let result_ptr = lparam as *mut DashboardCommandResult;
                    if !result_ptr.is_null() {
                        let result = Box::from_raw(result_ptr);
                        set_dashboard_text(state, &result.status, &result.body);
                    }
                }
                0
            }
            WM_SIZE => {
                if let Some(state) = dashboard_state(hwnd) {
                    layout_dashboard_window(hwnd, state);
                }
                0
            }
            WM_COMMAND => {
                if let Some(state) = dashboard_state(hwnd) {
                    match wparam & 0xffff {
                        DASH_REFRESH => {
                            queue_dashboard_cli(hwnd, state, "Loading status", &["status"])
                        }
                        DASH_CREDITS => {
                            queue_dashboard_cli(hwnd, state, "Loading credits", &["credits"])
                        }
                        DASH_MODELS => {
                            queue_dashboard_cli(hwnd, state, "Loading models", &["model", "list"])
                        }
                        DASH_DOCTOR => {
                            queue_dashboard_cli(hwnd, state, "Running diagnostics", &["doctor"])
                        }
                        DASH_LOGS => queue_dashboard_cli(hwnd, state, "Loading logs", &["logs"]),
                        DASH_START => queue_dashboard_start(hwnd, state),
                        DASH_PAUSE => queue_dashboard_cli(hwnd, state, "Pausing node", &["pause"]),
                        DASH_RESUME => {
                            queue_dashboard_cli(hwnd, state, "Resuming node", &["resume"])
                        }
                        DASH_DISCONNECT => {
                            queue_dashboard_cli(hwnd, state, "Disconnecting node", &["exit"])
                        }
                        DASH_SETUP => {
                            open_dashboard_shell(state, "Setup wizard opened", &["install"])
                        }
                        DASH_CAP => {
                            open_dashboard_shell(state, "Contribution wizard opened", &["cap"])
                        }
                        DASH_MODEL_USE => {
                            open_dashboard_shell(state, "Model selector opened", &["model", "use"])
                        }
                        DASH_MODEL_ADD => open_dashboard_shell(
                            state,
                            "Model downloader opened",
                            &["model", "add"],
                        ),
                        DASH_CLOSE => {
                            DestroyWindow(hwnd);
                        }
                        _ => {}
                    }
                }
                0
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORBTN => {
                if let Some(state) = dashboard_state(hwnd) {
                    let hdc = wparam as _;
                    SetBkColor(hdc, COLOR_PANEL);
                    SetTextColor(hdc, COLOR_TEXT);
                    return state.panel_brush as isize;
                }
                0
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DashboardState;
                if !state_ptr.is_null() {
                    let state = Box::from_raw(state_ptr);
                    DeleteObject(state.background_brush);
                    DeleteObject(state.panel_brush);
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
                DASHBOARD_HWND = ptr::null_mut();
                0
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }

    fn open_dashboard_window() {
        unsafe {
            if !DASHBOARD_HWND.is_null() {
                ShowWindow(DASHBOARD_HWND, SW_RESTORE);
                SetForegroundWindow(DASHBOARD_HWND);
                return;
            }

            let instance = GetModuleHandleW(ptr::null());
            if instance.is_null() {
                return;
            }
            let class_name = wide("MundusXDashboardWindow");
            let window_class = WNDCLASSW {
                lpfnWndProc: Some(dashboard_window_proc),
                hInstance: instance,
                lpszClassName: class_name.as_ptr(),
                hbrBackground: CreateSolidBrush(COLOR_BACKGROUND),
                ..std::mem::zeroed()
            };
            RegisterClassW(&window_class);

            let state = Box::new(DashboardState::new());
            let state_ptr = Box::into_raw(state);
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                wide("MundusX Contributor Control").as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                980,
                680,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                state_ptr as *const _,
            );
            if hwnd.is_null() {
                let state = Box::from_raw(state_ptr);
                DeleteObject(state.background_brush);
                DeleteObject(state.panel_brush);
            } else {
                DASHBOARD_HWND = hwnd;
                SetForegroundWindow(hwnd);
            }
        }
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
                    WM_LBUTTONUP | WM_LBUTTONDBLCLK => open_dashboard_window(),
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
