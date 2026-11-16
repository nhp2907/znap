//! znap installer: one setup window, per-user install (no admin).
//! Registers in Apps & features; `uninstall.exe` / `/uninstall` removes it.
#![windows_subsystem = "windows"]

use std::{
    env,
    ffi::{c_void, OsString},
    fs,
    mem::{size_of, transmute, zeroed},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::Command,
    ptr::{null, null_mut},
    thread::sleep,
    time::Duration,
};

use windows_sys::{
    core::{GUID, HRESULT},
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{
            ApplicationInstallationAndServicing::{ActivateActCtx, CreateActCtxW, ACTCTXW},
            Com::*,
            LibraryLoader::{GetModuleHandleW, LoadLibraryW},
            Registry::*,
        },
        UI::{
            HiDpi::{GetDpiForSystem, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_SYSTEM_AWARE},
            Input::KeyboardAndMouse::{EnableWindow, SetFocus},
            Shell::{SHGetKnownFolderPath, FOLDERID_Desktop, FOLDERID_Programs},
            WindowsAndMessaging::*,
        },
    },
};

const ZNAP: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../target/release/znap.exe"));
const VERSION: &str = env!("CARGO_PKG_VERSION");
const UNINST_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\znap";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const LNK: &str = "znap.lnk";
const NO_WINDOW: u32 = 0x0800_0000;
const BST_CHECKED: usize = 1;

const ID_DIR: i32 = 100;
const ID_BROWSE: i32 = 101;
const ID_AUTOSTART: i32 = 102;
const ID_DESKTOP: i32 = 103;
const ID_STARTMENU: i32 = 104;
const ID_LAUNCH: i32 = 105;

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
<dependency><dependentAssembly><assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/></dependentAssembly></dependency>
</assembly>"#;

fn main() {
    let me = env::current_exe().unwrap_or_default();
    let uninstall_mode = env::args().skip(1).any(|a| a.eq_ignore_ascii_case("/uninstall"))
        || me.file_stem().is_some_and(|s| s.eq_ignore_ascii_case("uninstall"));
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_SYSTEM_AWARE);
        CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32);
        visual_styles();
        if uninstall_mode {
            if msg(null_mut(), "Remove znap from this PC?", MB_YESNO | MB_ICONQUESTION) == IDYES {
                uninstall();
                msg(null_mut(), "znap was removed.", MB_ICONINFORMATION);
            }
        } else {
            run_ui();
        }
    }
}

// ---------- install / uninstall ----------

struct Opts {
    dir: PathBuf,
    autostart: bool,
    desktop: bool,
    startmenu: bool,
    launch: bool,
}

fn install(o: &Opts) -> Result<(), String> {
    kill_znap();
    fs::create_dir_all(&o.dir).map_err(|e| format!("Cannot create {}: {e}", o.dir.display()))?;
    let exe = o.dir.join("znap.exe");
    write_retry(&exe, ZNAP)?;
    let un = o.dir.join("uninstall.exe");
    let me = env::current_exe().map_err(|e| e.to_string())?;
    if !same_file(&me, &un) {
        fs::copy(&me, &un).map_err(|e| format!("Cannot write {}: {e}", un.display()))?;
    }
    let exe_s = exe.to_string_lossy();

    if o.autostart {
        reg_set_sz(RUN_KEY, "znap", &format!("\"{exe_s}\""))?;
    } else {
        reg_del_value(RUN_KEY, "znap");
    }
    for (on, folder) in [(o.desktop, &FOLDERID_Desktop), (o.startmenu, &FOLDERID_Programs)] {
        let Some(lnk) = known_folder(folder).map(|p| p.join(LNK)) else { continue };
        if on {
            unsafe { shortcut(&lnk, &exe)? };
        } else {
            let _ = fs::remove_file(lnk);
        }
    }

    // Apps & features entry.
    let k = UNINST_KEY;
    reg_set_sz(k, "DisplayName", "znap")?;
    reg_set_sz(k, "DisplayVersion", VERSION)?;
    reg_set_sz(k, "Publisher", "znap")?;
    reg_set_sz(k, "DisplayIcon", &exe_s)?;
    reg_set_sz(k, "InstallLocation", &o.dir.to_string_lossy())?;
    reg_set_sz(k, "UninstallString", &format!("\"{}\" /uninstall", un.display()))?;
    reg_set_dword(k, "NoModify", 1)?;
    reg_set_dword(k, "NoRepair", 1)?;
    reg_set_dword(k, "EstimatedSize", (ZNAP.len() / 1024) as u32)?;

    if o.launch {
        let _ = Command::new(&exe).current_dir(&o.dir).spawn();
    }
    Ok(())
}

fn uninstall() {
    let dir = reg_get_sz(UNINST_KEY, "InstallLocation")
        .map(PathBuf::from)
        .or_else(|| env::current_exe().ok()?.parent().map(Path::to_path_buf));
    kill_znap();
    reg_del_value(RUN_KEY, "znap");
    for f in [&FOLDERID_Desktop, &FOLDERID_Programs] {
        if let Some(p) = known_folder(f) {
            let _ = fs::remove_file(p.join(LNK));
        }
    }
    unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, w(UNINST_KEY).as_ptr()) };
    let Some(dir) = dir else { return };
    let _ = fs::remove_file(dir.join("znap.exe"));
    // This exe may be dir\uninstall.exe: delete it + the (now empty) folder after we exit.
    // Non-recursive rmdir: never removes anything we did not put there.
    let cmd = env::var_os("ComSpec").map(PathBuf::from).unwrap_or_else(|| sys32("cmd.exe"));
    let _ = Command::new(cmd)
        .raw_arg(format!(
            "/d /s /c \"\"{}\" -n 3 127.0.0.1 >nul & del /f /q \"{}\" & rmdir \"{}\"\"",
            sys32("ping.exe").display(),
            dir.join("uninstall.exe").display(),
            dir.display()
        ))
        .current_dir(env::temp_dir())
        .creation_flags(NO_WINDOW)
        .spawn();
}

fn kill_znap() {
    let _ = Command::new(sys32("taskkill.exe"))
        .args(["/f", "/im", "znap.exe"])
        .creation_flags(NO_WINDOW)
        .status();
}

/// znap may need a moment to exit after taskkill.
fn write_retry(p: &Path, data: &[u8]) -> Result<(), String> {
    let mut last = String::new();
    for _ in 0..15 {
        match fs::write(p, data) {
            Ok(()) => return Ok(()),
            Err(e) => last = e.to_string(),
        }
        sleep(Duration::from_millis(200));
    }
    Err(format!("Cannot write {}: {last}", p.display()))
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn sys32(name: &str) -> PathBuf {
    let root = env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    PathBuf::from(root).join("System32").join(name)
}

fn default_dir() -> PathBuf {
    reg_get_sz(UNINST_KEY, "InstallLocation").map(PathBuf::from).unwrap_or_else(|| {
        let base = env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(env::temp_dir);
        base.join("Programs").join("znap")
    })
}

// ---------- win32 helpers ----------

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn wp(p: &Path) -> Vec<u16> {
    p.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn msg(h: HWND, text: &str, flags: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    unsafe { MessageBoxW(h, w(text).as_ptr(), w("znap Setup").as_ptr(), flags) }
}

fn reg_set(key: &str, name: &str, kind: REG_VALUE_TYPE, data: &[u8]) -> Result<(), String> {
    unsafe {
        let mut h: HKEY = null_mut();
        let r = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            w(key).as_ptr(),
            0,
            null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            null(),
            &mut h,
            null_mut(),
        );
        if r != 0 {
            return Err(format!("Registry {key}: error {r}"));
        }
        let r = RegSetValueExW(h, w(name).as_ptr(), 0, kind, data.as_ptr(), data.len() as u32);
        RegCloseKey(h);
        if r != 0 {
            return Err(format!("Registry {key}\\{name}: error {r}"));
        }
    }
    Ok(())
}

fn reg_set_sz(key: &str, name: &str, val: &str) -> Result<(), String> {
    let v = w(val);
    let bytes = unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2) };
    reg_set(key, name, REG_SZ, bytes)
}

fn reg_set_dword(key: &str, name: &str, val: u32) -> Result<(), String> {
    reg_set(key, name, REG_DWORD, &val.to_le_bytes())
}

fn reg_del_value(key: &str, name: &str) {
    unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, w(key).as_ptr(), w(name).as_ptr()) };
}

fn reg_get_sz(key: &str, name: &str) -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w(key).as_ptr(),
            w(name).as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            buf.as_mut_ptr() as *mut c_void,
            &mut size,
        )
    };
    if r != 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
    Some(String::from_utf16_lossy(&buf[..len])).filter(|s| !s.is_empty())
}

fn known_folder(id: &GUID) -> Option<PathBuf> {
    unsafe {
        let mut p = null_mut();
        if SHGetKnownFolderPath(id, 0, null_mut(), &mut p) < 0 {
            return None;
        }
        let len = (0..).take_while(|&i| *p.add(i) != 0).count();
        let s = OsString::from_wide(std::slice::from_raw_parts(p, len));
        CoTaskMemFree(p as *const c_void);
        Some(s.into())
    }
}

/// Create a .lnk via IShellLinkW / IPersistFile (raw vtables; windows-sys has no COM wrappers).
unsafe fn shortcut(lnk: &Path, target: &Path) -> Result<(), String> {
    const CLSID_SHELL_LINK: GUID = GUID::from_u128(0x00021401_0000_0000_c000_000000000046);
    const IID_ISHELL_LINK_W: GUID = GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
    const IID_IPERSIST_FILE: GUID = GUID::from_u128(0x0000010b_0000_0000_c000_000000000046);
    type SetStr = unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT;
    type Qi = unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;
    type Release = unsafe extern "system" fn(*mut c_void) -> u32;
    type Save = unsafe extern "system" fn(*mut c_void, *const u16, BOOL) -> HRESULT;
    let vfn = |obj: *mut c_void, i: usize| *(*(obj as *const *const usize)).add(i);

    let mut sl = null_mut();
    let hr = CoCreateInstance(&CLSID_SHELL_LINK, null_mut(), CLSCTX_INPROC_SERVER, &IID_ISHELL_LINK_W, &mut sl);
    if hr < 0 {
        return Err(format!("Shortcut {}: 0x{hr:08x}", lnk.display()));
    }
    // IShellLinkW: 7 SetDescription, 9 SetWorkingDirectory, 20 SetPath.
    transmute::<usize, SetStr>(vfn(sl, 20))(sl, wp(target).as_ptr());
    if let Some(d) = target.parent() {
        transmute::<usize, SetStr>(vfn(sl, 9))(sl, wp(d).as_ptr());
    }
    transmute::<usize, SetStr>(vfn(sl, 7))(sl, w("Screenshot + annotate (Ctrl+Alt+S)").as_ptr());
    let mut pf = null_mut();
    let mut hr = transmute::<usize, Qi>(vfn(sl, 0))(sl, &IID_IPERSIST_FILE, &mut pf);
    if hr >= 0 {
        // IPersistFile: 6 Save.
        hr = transmute::<usize, Save>(vfn(pf, 6))(pf, wp(lnk).as_ptr(), 1);
        transmute::<usize, Release>(vfn(pf, 2))(pf);
    }
    transmute::<usize, Release>(vfn(sl, 2))(sl);
    if hr < 0 {
        return Err(format!("Shortcut {}: 0x{hr:08x}", lnk.display()));
    }
    Ok(())
}

/// Themed controls without a manifest resource: activate a temp manifest.
unsafe fn visual_styles() {
    let path = env::temp_dir().join("znap-setup.manifest");
    if fs::write(&path, MANIFEST).is_err() {
        return;
    }
    let p = wp(&path);
    let mut ctx: ACTCTXW = zeroed();
    ctx.cbSize = size_of::<ACTCTXW>() as u32;
    ctx.lpSource = p.as_ptr();
    let h = CreateActCtxW(&ctx);
    if h != INVALID_HANDLE_VALUE {
        let mut cookie = 0usize;
        ActivateActCtx(h, &mut cookie);
        LoadLibraryW(w("comctl32.dll").as_ptr());
    }
}

// ---------- UI ----------

unsafe fn run_ui() {
    let hinst = GetModuleHandleW(null());
    let cls = w("znap_setup");
    let wc = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: 0,
        lpfnWndProc: Some(wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinst,
        hIcon: LoadIconW(null_mut(), IDI_APPLICATION),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        hbrBackground: GetSysColorBrush(COLOR_WINDOW),
        lpszMenuName: null(),
        lpszClassName: cls.as_ptr(),
        hIconSm: null_mut(),
    };
    RegisterClassExW(&wc);

    let s = GetDpiForSystem() as f32 / 96.0;
    let px = |v: i32| (v as f32 * s).round() as i32;
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
    let mut rc = RECT { left: 0, top: 0, right: px(460), bottom: px(290) };
    AdjustWindowRectEx(&mut rc, style, 0, 0);
    let (ww, wh) = (rc.right - rc.left, rc.bottom - rc.top);
    let x = (GetSystemMetrics(SM_CXSCREEN) - ww) / 2;
    let y = (GetSystemMetrics(SM_CYSCREEN) - wh) / 2;
    let title = w(&format!("znap {VERSION} Setup"));
    let hwnd = CreateWindowExW(0, cls.as_ptr(), title.as_ptr(), style, x, y, ww, wh, null_mut(), null_mut(), hinst, null());

    let mut ncm: NONCLIENTMETRICSW = zeroed();
    ncm.cbSize = size_of::<NONCLIENTMETRICSW>() as u32;
    SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, ncm.cbSize, &mut ncm as *mut _ as *mut c_void, 0);
    let font = CreateFontIndirectW(&ncm.lfMessageFont);
    let mut big = ncm.lfMessageFont;
    big.lfHeight = (big.lfHeight as f32 * 1.6) as i32;
    big.lfWeight = 600;
    let big_font = CreateFontIndirectW(&big);

    let ctl = |ex: u32, class: &str, text: &str, st: u32, id: i32, r: [i32; 4]| {
        let h = CreateWindowExW(
            ex,
            w(class).as_ptr(),
            w(text).as_ptr(),
            WS_CHILD | WS_VISIBLE | st,
            px(r[0]),
            px(r[1]),
            px(r[2]),
            px(r[3]),
            hwnd,
            id as usize as HMENU,
            hinst,
            null(),
        );
        SendMessageW(h, WM_SETFONT, font as usize, 1);
        h
    };
    let tab = WS_TABSTOP;
    let title = ctl(0, "STATIC", "Install znap", 0, 0, [20, 14, 420, 32]);
    SendMessageW(title, WM_SETFONT, big_font as usize, 1);
    ctl(0, "STATIC", "Screenshot + annotate tool. Press Ctrl+Alt+S to capture.", 0, 0, [20, 48, 420, 20]);
    ctl(0, "STATIC", "Install folder:", 0, 0, [20, 82, 420, 20]);
    ctl(WS_EX_CLIENTEDGE, "EDIT", &default_dir().to_string_lossy(), tab | ES_AUTOHSCROLL as u32, ID_DIR, [20, 102, 330, 24]);
    ctl(0, "BUTTON", "Browse...", tab, ID_BROWSE, [358, 101, 82, 26]);
    let checks = [
        (ID_AUTOSTART, "Start znap when Windows starts"),
        (ID_DESKTOP, "Create a desktop shortcut"),
        (ID_STARTMENU, "Add to Start menu"),
        (ID_LAUNCH, "Open znap after install"),
    ];
    for (i, (id, label)) in checks.into_iter().enumerate() {
        let h = ctl(0, "BUTTON", label, tab | BS_AUTOCHECKBOX as u32, id, [20, 140 + i as i32 * 26, 420, 22]);
        SendMessageW(h, BM_SETCHECK, BST_CHECKED, 0);
    }
    let install = ctl(0, "BUTTON", "Install", tab | BS_DEFPUSHBUTTON as u32, IDOK, [262, 246, 86, 28]);
    ctl(0, "BUTTON", "Cancel", tab, IDCANCEL, [356, 246, 86, 28]);

    ShowWindow(hwnd, SW_SHOW);
    SetFocus(install);
    let mut m: MSG = zeroed();
    while GetMessageW(&mut m, null_mut(), 0, 0) > 0 {
        if IsDialogMessageW(hwnd, &m) == 0 {
            TranslateMessage(&m);
            DispatchMessageW(&m);
        }
    }
}

unsafe fn get_text(h: HWND) -> String {
    let mut buf = vec![0u16; GetWindowTextLengthW(h) as usize + 1];
    let n = GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32);
    String::from_utf16_lossy(&buf[..n as usize])
}

unsafe fn on_install(h: HWND) {
    let dir = get_text(GetDlgItem(h, ID_DIR));
    let dir = dir.trim();
    if dir.is_empty() {
        msg(h, "Please choose an install folder.", MB_ICONWARNING);
        return;
    }
    let checked = |id| SendMessageW(GetDlgItem(h, id), BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
    let o = Opts {
        dir: PathBuf::from(dir),
        autostart: checked(ID_AUTOSTART),
        desktop: checked(ID_DESKTOP),
        startmenu: checked(ID_STARTMENU),
        launch: checked(ID_LAUNCH),
    };
    EnableWindow(h, 0);
    let res = install(&o);
    EnableWindow(h, 1);
    match res {
        Ok(()) => {
            msg(h, "znap is installed.\n\nPress Ctrl+Alt+S to take a screenshot.", MB_ICONINFORMATION);
            DestroyWindow(h);
        }
        Err(e) => {
            msg(h, &format!("Install failed:\n{e}"), MB_ICONERROR);
        }
    }
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp_: WPARAM, lp: LPARAM) -> LRESULT {
    match m {
        WM_COMMAND => {
            match (wp_ & 0xffff) as i32 {
                IDOK => on_install(h),
                IDCANCEL => {
                    DestroyWindow(h);
                }
                ID_BROWSE => {
                    let cur = PathBuf::from(get_text(GetDlgItem(h, ID_DIR)));
                    let start = cur.parent().filter(|p| p.exists()).unwrap_or(&cur).to_path_buf();
                    if let Some(p) = rfd::FileDialog::new().set_directory(start).pick_folder() {
                        let named = p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("znap"));
                        let p = if named { p } else { p.join("znap") };
                        SetWindowTextW(GetDlgItem(h, ID_DIR), wp(&p).as_ptr());
                    }
                }
                _ => {}
            }
            0
        }
        WM_CTLCOLORSTATIC => {
            SetBkMode(wp_ as HDC, TRANSPARENT as _);
            GetSysColorBrush(COLOR_WINDOW) as LRESULT
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(h, m, wp_, lp),
    }
}
