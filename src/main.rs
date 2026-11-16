// Stays running (started at logon, or by COM on a click): shows a Windows toast for each file
// main.js drops in the spool folder. A click reaches us straight through COM (no process launch),
// and we bring Herdr to the front and send the toast's line to herdr-app.sock.
#![windows_subsystem = "windows"]

use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use windows::core::{h, implement, Interface, Ref, BOOL, GUID, HSTRING, IUnknown, PCWSTR, Result};
use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, MAX_PATH};
use windows::Win32::Storage::FileSystem::{
    FindFirstChangeNotificationW, FindNextChangeNotification, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
};
use windows::Win32::System::Com::{
    CoInitializeEx, CoRegisterClassObject, IClassFactory, IClassFactory_Impl, CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED,
    REGCLS_MULTIPLEUSE,
};
use windows::Win32::System::Threading::{
    CreateMutexW, OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Notifications::{INotificationActivationCallback, INotificationActivationCallback_Impl, NOTIFICATION_USER_INPUT_DATA};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindow, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow, GW_OWNER,
    SW_RESTORE,
};

// Must match install.ps1.
const AUMID: &str = "Herdr.Notify";
const CLSID: GUID = GUID::from_u128(0xF284B6D9_7D6B_4119_9170_D8A75506F3FA);

fn local() -> PathBuf {
    PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join("herdr-win-toast")
}

fn log(msg: &str) {
    let t = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_millis();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(local().join("daemon.log")) {
        let _ = writeln!(f, "{t} {msg}");
    }
}

#[implement(INotificationActivationCallback)]
struct Activator;

impl INotificationActivationCallback_Impl for Activator_Impl {
    fn Activate(&self, _aumid: &PCWSTR, args: &PCWSTR, _d: *const NOTIFICATION_USER_INPUT_DATA, _n: u32) -> Result<()> {
        let line = unsafe { args.to_string() }.unwrap_or_default();
        focus_herdr();
        if !line.is_empty() {
            match send(&line) {
                Ok(()) => log(&format!("click ok {line}")),
                Err(e) => log(&format!("click failed {line}: {e}")),
            }
        }
        Ok(())
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, _outer: Ref<IUnknown>, riid: *const GUID, ppv: *mut *mut core::ffi::c_void) -> Result<()> {
        let unk: IUnknown = Activator.into();
        unsafe { unk.query(riid, ppv).ok() }
    }
    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

fn is_herdr(hwnd: HWND) -> bool {
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let mut buf = [0u16; MAX_PATH as usize];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(p);
        ok && String::from_utf16_lossy(&buf[..len as usize]).to_lowercase().ends_with("\\herdr-app.exe")
    }
}

extern "system" fn find_herdr(hwnd: HWND, out: LPARAM) -> BOOL {
    unsafe {
        if IsWindowVisible(hwnd).as_bool() && GetWindow(hwnd, GW_OWNER).is_err() && is_herdr(hwnd) {
            *(out.0 as *mut HWND) = hwnd;
            return false.into();
        }
    }
    true.into()
}

fn focus_herdr() {
    let mut hwnd = HWND::default();
    unsafe {
        let _ = EnumWindows(Some(find_herdr), LPARAM(&mut hwnd as *mut HWND as isize));
        if hwnd.is_invalid() {
            return;
        }
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        // The toast click grants us foreground rights, so this is honored without tricks.
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Sends one request line to herdr-app.sock and returns its one-line reply.
fn request(line: &str) -> std::io::Result<String> {
    let app = std::env::var("HERDR_APP_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default()).join(".config\\herdr-app"));
    let mut s = uds_windows::UnixStream::connect(app.join("herdr-app.sock"))?;
    s.write_all(format!("{line}\n").as_bytes())?;
    s.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut out = Vec::new();
    let mut buf = [0u8; 65536];
    while !out.ends_with(b"\n") {
        let n = s.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

fn send(line: &str) -> std::io::Result<()> {
    request(line).map(drop) // waiting for the reply makes sure the command landed
}

/// The agent's name in install.ps1's AppIDs and png files, if the app is a known agent.
fn agent_of(app: &str) -> Option<&'static str> {
    let app = app.to_lowercase();
    [("claude", "Claude"), ("codex", "Codex"), ("antigravity", "Antigravity"), ("agy", "Antigravity")]
        .iter()
        .find(|(k, _)| app.contains(k))
        .map(|(_, v)| *v)
}

/// ("space：X　tab：Y", app) for the terminal the toast's line targets, from herdr-app's ls.
/// Fallback for when main.js couldn't find the agent itself.
fn describe(line: &str) -> Option<(String, String)> {
    let target = serde_json::from_str::<serde_json::Value>(line).ok()?["args"]["terminal"].as_str()?.to_string();
    let ls: serde_json::Value = serde_json::from_str(&request(r#"{"ls":{}}"#).ok()?).ok()?;
    for space in ls["spaces"].as_array()? {
        for tab in space["tabs"].as_array().into_iter().flatten() {
            for t in tab["terminals"].as_array().into_iter().flatten() {
                if t["terminal"].as_str() == Some(&target) {
                    let mut text =
                        format!("space：{}　tab：{}", space["name"].as_str().unwrap_or("?"), tab["name"].as_str().unwrap_or("?"));
                    if let Some(name) = t["name"].as_str() {
                        text += &format!("　{name}");
                    }
                    return Some((text, t["app"].as_str().unwrap_or("").to_string()));
                }
            }
        }
    }
    None
}

fn same_dir(a: &str, b: &str) -> bool {
    let norm = |p: &str| p.replace('/', "\\").trim_end_matches('\\').to_lowercase();
    norm(a) == norm(b)
}

/// The last thing the user typed into the newest Codex session started in cwd.
fn codex_prompt(cwd: &str) -> Option<String> {
    let root = PathBuf::from(std::env::var("USERPROFILE").ok()?).join(".codex").join("sessions");
    let newest = |dir: &PathBuf, n: usize| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
        v.sort();
        v.into_iter().rev().take(n).collect()
    };
    // sessions/YYYY/MM/DD/rollout-*.jsonl; a few newest day folders cover sessions still open from earlier days.
    let mut files: Vec<PathBuf> = Vec::new();
    for y in newest(&root, 2) {
        for m in newest(&y, 2) {
            for d in newest(&m, 3) {
                files.extend(newest(&d, 1000));
            }
        }
    }
    let mtime = |p: &PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    files.sort_by_key(|p| std::cmp::Reverse(mtime(p)));
    for f in files.iter().take(20) {
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        let mut lines = text.lines().map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap_or_default());
        let meta = lines.next().unwrap_or_default();
        if !same_dir(meta["payload"]["cwd"].as_str().unwrap_or(""), cwd) {
            continue;
        }
        let mut last = None;
        for j in lines {
            let p = &j["payload"];
            if j["type"] == "response_item" && p["type"] == "message" && p["role"] == "user" {
                for c in p["content"].as_array().into_iter().flatten() {
                    let t = c["text"].as_str().unwrap_or("").trim();
                    // Codex also logs AGENTS.md and <environment_context> as user messages.
                    if !t.is_empty() && !t.starts_with('<') && !t.starts_with("# AGENTS.md") {
                        last = Some(t.to_string());
                    }
                }
            }
        }
        return last;
    }
    None
}

fn one_line(s: &str, max: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() > max { s.chars().take(max - 1).collect::<String>() + "…" } else { s }
}

struct Toast {
    aumid: String,
    logo: Option<PathBuf>,
    headline: String,
    detail: String,
    footer: String,
    line: String,
}

fn show(t: &Toast) -> Result<()> {
    let xml = XmlDocument::new()?;
    xml.LoadXml(h!("<toast><visual><binding template=\"ToastGeneric\"/></visual></toast>"))?;
    let binding = xml.GetElementsByTagName(h!("binding"))?.Item(0)?;
    for (text, attribution) in [(&t.headline, false), (&t.detail, false), (&t.footer, true)] {
        if text.is_empty() {
            continue;
        }
        let el = xml.CreateElement(h!("text"))?;
        el.SetInnerText(&HSTRING::from(text.as_str()))?;
        if attribution {
            el.SetAttribute(h!("placement"), h!("attribution"))?;
        }
        binding.AppendChild(&el)?;
    }
    if let Some(logo) = &t.logo {
        let img = xml.CreateElement(h!("image"))?;
        img.SetAttribute(h!("placement"), h!("appLogoOverride"))?;
        img.SetAttribute(h!("src"), &HSTRING::from(format!("file:///{}", logo.display()).replace('\\', "/")))?;
        binding.AppendChild(&img)?;
    }
    if !t.line.is_empty() {
        xml.DocumentElement()?.SetAttribute(h!("launch"), &HSTRING::from(t.line.as_str()))?;
    }
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(t.aumid.as_str()))?
        .Show(&ToastNotification::CreateToastNotification(&xml)?)
}

/// Herdr's title is "<Agent> finished" / "<Agent> needs you"; the other fields come from main.js.
/// Layout (mockup/toast-mockup.html): "status · space", task summary, "tab X · branch", logo with a status badge.
fn build(n: &serde_json::Value, seen: &mut HashSet<String>) -> Toast {
    let s = |k: &str| n[k].as_str().unwrap_or("").to_string();
    let title = s("title");
    let term = serde_json::from_str::<serde_json::Value>(&s("line")).ok()
        .and_then(|l| l["args"]["terminal"].as_str().map(String::from)).unwrap_or_default();
    let first = !term.is_empty() && seen.insert(term);
    let (place, app) = if !s("app").is_empty() {
        (String::new(), s("app"))
    } else {
        describe(&s("line")).unwrap_or_else(|| (s("body"), String::new()))
    };
    let agent = agent_of(&app);
    // Claude Code puts its task summary in the terminal title, after a spinner glyph.
    let term_title = s("term_title");
    let term_title = term_title.trim_start_matches(|c: char| !c.is_alphanumeric()).trim();
    let cwd = s("cwd").replace('/', "\\");
    let folder = cwd.trim_end_matches('\\').rsplit('\\').next().unwrap_or("").to_string();
    // Agents that don't set a title leave the shell's: the app, the folder or an exe path.
    let real_title = !term_title.is_empty()
        && !term_title.eq_ignore_ascii_case(&app)
        && !term_title.eq_ignore_ascii_case("Claude Code") // Claude's title before it has a task
        && !term_title.eq_ignore_ascii_case(&folder)
        && !term_title.contains('\\')
        && !s("line").contains(&format!("\"{term_title}\"")); // agy's title is its terminal id (t1b)
    let summary = if agent == Some("Codex") { codex_prompt(&cwd) } else { None }
        .or_else(|| real_title.then(|| term_title.to_string()))
        .map(|x| one_line(&x, 80));
    // Herdr also says "finished" when an agent starts up and goes idle: no task yet, first toast for the terminal.
    // shortcut: "first" only lasts one daemon run, so a real finish right after a daemon restart can read as init; fine unless it bites.
    let (state, status) = if title.ends_with("needs you") {
        ("wait", "等你回覆".to_string())
    } else if title.ends_with("finished") && first && summary.is_none() {
        ("init", "初始化完成".to_string())
    } else if title.ends_with("finished") {
        ("ok", "已完成".to_string())
    } else {
        ("", title)
    };
    let join = |v: [String; 2]| v.into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
    let tab = s("tab");
    let detail = summary.unwrap_or_else(|| if state == "init" { "可以開始下指令".to_string() } else { place });
    Toast {
        aumid: agent.map_or(AUMID.to_string(), |a| format!("{AUMID}.{a}")),
        logo: agent.and_then(|a| {
            let a = a.to_lowercase();
            [format!("{a}-{state}.png"), format!("{a}.png")].into_iter().map(|f| local().join(f)).find(|p| p.exists())
        }),
        headline: join([status, s("space")]),
        detail,
        footer: join([if tab.is_empty() { tab } else { format!("tab {tab}") }, s("branch")]),
        line: s("line"),
    }
}

fn drain(spool: &PathBuf, seen: &mut HashSet<String>) {
    let Ok(dir) = std::fs::read_dir(spool) else { return };
    for f in dir.flatten() {
        let path = f.path();
        if path.extension().map_or(true, |e| e != "json") {
            continue;
        }
        let parsed = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
        let Some(n) = parsed else {
            // Half written: the next change event retries it; give up on it after 10 s.
            let old = f.metadata().and_then(|m| m.modified()).map_or(false, |m| m.elapsed().unwrap_or_default() > Duration::from_secs(10));
            if old {
                let _ = std::fs::remove_file(&path);
            }
            continue;
        };
        let _ = std::fs::remove_file(&path);
        let t = build(&n, seen);
        match show(&t) {
            Ok(()) => log(&format!("shown {} | {}", t.headline, t.detail)),
            Err(e) => log(&format!("toast failed: {e}")),
        }
    }
}

fn main() -> Result<()> {
    unsafe {
        let _mutex = CreateMutexW(None, true, h!("herdr-win-toast-rs"))?;
        if windows::Win32::Foundation::GetLastError() == windows::Win32::Foundation::ERROR_ALREADY_EXISTS {
            return Ok(()); // already running
        }
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let factory: IUnknown = Factory.into();
        CoRegisterClassObject(&CLSID, &factory, CLSCTX_LOCAL_SERVER, REGCLS_MULTIPLEUSE)?;
    }
    let spool = local().join("spool");
    let _ = std::fs::create_dir_all(&spool);
    // Skip notifications queued while we were down.
    if let Ok(dir) = std::fs::read_dir(&spool) {
        dir.flatten().for_each(|f| drop(std::fs::remove_file(f.path())));
    }
    let watch = unsafe {
        FindFirstChangeNotificationW(&HSTRING::from(spool.as_os_str()), false, FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE)?
    };
    let mut seen = HashSet::new();
    loop {
        drain(&spool, &mut seen);
        unsafe {
            WaitForSingleObject(watch, 1000); // the timeout retries files that were half written
            let _ = FindNextChangeNotification(watch);
        }
    }
}
