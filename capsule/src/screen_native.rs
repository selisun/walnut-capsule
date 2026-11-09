//! B-10 P3：native 引擎的小屏进程托管。
//!
//! 职责（全部与旧 PowerShell 宿主隔离，旧逻辑一行未改）：
//! 1. 由胶囊设置生成小屏 `config.toml`（方向/刷新间隔/组件显隐）；
//! 2. 用 `CreateProcessW` + `CREATE_NO_WINDOW` 拉起同目录的 `hetao-screen.exe`；
//! 3. 每 5 s 巡检，意外退出时重新拉起（连续失败退避 10 s）；
//! 4. 退出时结束子进程并清理 pid 文件，避免残留。
//!
//! `hetao-screen.exe` 缺失或拉起失败时返回 `false`，由调用方回退 PowerShell。

use std::ffi::c_void;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, HANDLE};
use windows::Win32::System::Threading::{
    CreateProcessW, GetCurrentProcessId, GetExitCodeProcess, OpenProcess,
    QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject, CREATE_NO_WINDOW,
    PROCESS_INFORMATION, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    STARTUPINFOW,
};

use crate::config::{ScreenConfig, SCREEN_ENGINE_NATIVE};

const EXE_NAME: &str = "hetao-screen.exe";
const TICK: Duration = Duration::from_secs(5);
const RETRY_AFTER_FAIL: Duration = Duration::from_secs(10);
/// GetExitCodeProcess 的「仍在运行」哨兵值（windows crate 未导出）。
const STILL_ACTIVE: u32 = 259;

struct Child {
    handle: isize,
    pid: u32,
}

impl Drop for Child {
    fn drop(&mut self) {
        // 进程句柄必须显式关闭：看门狗每次重启都会替换 Child，不关会持续泄漏内核句柄
        if self.handle != 0 {
            unsafe {
                let _ = CloseHandle(HANDLE(self.handle as *mut c_void));
            }
            self.handle = 0;
        }
    }
}

static CHILD: Mutex<Option<Child>> = Mutex::new(None);
static STOPPING: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);
/// 启动代号：start/stop 自增，旧看门狗发现自己过期就退出。
static GEN: AtomicU32 = AtomicU32::new(0);

/// 启动诊断：记录引擎选择与 exe 定位结果（无凭据），便于现场判断为何回退。
pub fn log_startup(cfg: &ScreenConfig) {
    let exe = exe_path();
    let exists = exe.as_ref().map(|p| p.exists()).unwrap_or(false);
    log(&format!(
        "startup: enabled={} engine={} exe={} exists={exists}",
        cfg.enabled,
        cfg.engine,
        exe.map(|p| p.display().to_string())
            .unwrap_or_else(|| "<none>".to_string()),
    ));
}

/// 是否处于托管状态（设置界面用它切换「开启 / 关闭 / 重连」按钮可用性）。
pub fn is_running() -> bool {
    STARTED.load(Ordering::SeqCst)
}

/// 启动托管。返回 `false` 表示无法拉起小屏进程（exe 缺失或创建失败）。
pub fn start(cfg: &ScreenConfig) -> bool {
    if cfg.engine != SCREEN_ENGINE_NATIVE {
        return false;
    }
    if STARTED.swap(true, Ordering::SeqCst) {
        return true;
    }
    STOPPING.store(false, Ordering::SeqCst);

    let Some(exe) = exe_path() else {
        log("无法定位自身目录，小屏未启动");
        STARTED.store(false, Ordering::SeqCst);
        return false;
    };
    if !exe.exists() {
        log(&format!(
            "未找到 {}，小屏未启动（安装器需一并分发）",
            exe.display()
        ));
        STARTED.store(false, Ordering::SeqCst);
        return false;
    }
    // 配置生成为 best-effort：写不进去（受限环境/权限异常）时小屏仍可凭内置默认值运行，
    // 不能因为一个非关键写失败就不启动。
    if let Err(e) = write_config(cfg) {
        log(&format!(
            "生成小屏配置失败：{e}（继续启动，小屏使用内置默认配置）"
        ));
    }

    // 上一次异常退出可能留下同名进程，先按映像路径校验后收掉
    if let Some(pid) = read_pid() {
        terminate_if_ours(pid);
        remove_pid();
    }

    if !spawn(&exe) {
        STARTED.store(false, Ordering::SeqCst);
        return false;
    }

    let gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    match thread::Builder::new()
        .name("screen-native".into())
        .spawn(move || watchdog(exe, gen))
    {
        Ok(_) => true,
        Err(e) => {
            log(&format!("看门狗线程启动失败：{e}"));
            stop();
            false
        }
    }
}

/// 结束托管：停看门狗并结束子进程。可重复调用。
pub fn stop() {
    if !STARTED.swap(false, Ordering::SeqCst) {
        STOPPING.store(true, Ordering::SeqCst);
        GEN.fetch_add(1, Ordering::SeqCst);
        let child = lock().take();
        if let Some(child) = child {
            terminate(child);
        }
        return;
    }
    STOPPING.store(true, Ordering::SeqCst);
    GEN.fetch_add(1, Ordering::SeqCst);
    let child = lock().take();
    if let Some(child) = child {
        let pid = child.pid;
        terminate(child);
        log(&format!("已结束 native 小屏进程，PID={pid}"));
        remove_pid();
    }
    if let Some(pid) = read_pid() {
        terminate_if_ours(pid);
        remove_pid();
    }
}

fn terminate(child: Child) {
    let handle = HANDLE(child.handle as *mut c_void);
    let mut code = 0u32;
    let alive = unsafe { GetExitCodeProcess(handle, &mut code).is_ok() && code == STILL_ACTIVE };
    if alive {
        unsafe {
            let _ = TerminateProcess(handle, 0);
        }
    }
    unsafe {
        // 必须等进程真正退出再返回：旧进程若还在往串口写帧尾，
        // 紧接着拉起的新进程会同时写帧，两路字节交错 → 设备帧错位（切换主题花屏）。
        let _ = WaitForSingleObject(handle, 3_000);
    }
    // 句柄由 Child::drop 关闭，避免与手动 CloseHandle 重复
}

fn pid_alive(pid: u32) -> bool {
    match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(handle) => {
            let mut code = 0u32;
            let alive =
                unsafe { GetExitCodeProcess(handle, &mut code).is_ok() && code == STILL_ACTIVE };
            unsafe {
                let _ = CloseHandle(handle);
            }
            alive
        }
        // 打不开不等于死了：只有「进程不存在」才判 dead。
        // 无权限/时序等原因一律保守视为存活，避免重复拉起导致两个屏端抢串口。
        Err(_) => {
            let code = unsafe { GetLastError() };
            code != ERROR_INVALID_PARAMETER
        }
    }
}

/// 仅当该 PID 的映像路径与我们的 `hetao-screen.exe` 一致时才终止，避免误杀。
fn terminate_if_ours(pid: u32) {
    let Some(expected) = exe_path() else {
        return;
    };
    let Ok(handle) = (unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE,
            false,
            pid,
        )
    }) else {
        return;
    };
    let mut buffer = vec![0u16; 1024];
    let mut len = buffer.len() as u32;
    let ok = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR::from_raw(buffer.as_mut_ptr()),
            &mut len,
        )
        .is_ok()
    };
    let image = ok.then(|| String::from_utf16_lossy(&buffer[..len as usize]));
    if let Some(image) = image {
        let same = image.eq_ignore_ascii_case(&expected.to_string_lossy());
        if same {
            unsafe {
                let _ = TerminateProcess(handle, 0);
            }
            log(&format!("已结束残留 native 小屏进程，PID={pid}"));
        }
    }
    unsafe {
        let _ = CloseHandle(handle);
    }
}

fn watchdog(exe: PathBuf, gen: u32) {
    let mut last_fail: Option<Instant> = None;
    loop {
        thread::sleep(TICK);
        if STOPPING.load(Ordering::SeqCst) || GEN.load(Ordering::SeqCst) != gen {
            return;
        }
        let dead = match lock().as_ref() {
            Some(child) => !pid_alive(child.pid),
            None => true,
        };
        if !dead {
            continue;
        }
        if STOPPING.load(Ordering::SeqCst) {
            return;
        }
        if let Some(t) = last_fail {
            if t.elapsed() < RETRY_AFTER_FAIL {
                continue;
            }
        }
        log("native 小屏进程已退出，重新拉起");
        if spawn(&exe) {
            last_fail = None;
        } else {
            last_fail = Some(Instant::now());
        }
    }
}

/// 子进程命令行。`--parent-pid` 让子进程在我们被强杀时自行退出，
/// 否则残留进程会一直占用串口，并锁住 hetao-screen.exe 导致覆盖安装失败。
fn child_command(exe: &Path, config: &Path, parent_pid: u32) -> String {
    format!(
        "\"{}\" --config \"{}\" --parent-pid {parent_pid}",
        exe.display(),
        config.display()
    )
}

fn spawn(exe: &Path) -> bool {
    if STOPPING.load(Ordering::SeqCst) {
        return false;
    }
    // 覆盖前先结束旧进程并关闭句柄：否则每次重启泄漏一个内核句柄，
    // 极端情况下旧屏端还活着，两个进程会同时写串口造成帧错位。
    if let Some(old) = lock().take() {
        terminate(old);
    }
    let parent_pid = unsafe { GetCurrentProcessId() };
    let mut cmd: Vec<u16> = child_command(exe, &config_path(), parent_pid)
        .encode_utf16()
        .collect();
    cmd.push(0);

    let si = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut pi = PROCESS_INFORMATION::default();
    let launched = unsafe {
        CreateProcessW(
            None,
            PWSTR(cmd.as_mut_ptr()),
            None,
            None,
            false,
            CREATE_NO_WINDOW,
            None,
            None,
            &si,
            &mut pi,
        )
    };
    if let Err(e) = launched {
        log(&format!("拉起 native 小屏失败：{e}"));
        return false;
    }
    unsafe {
        let _ = CloseHandle(pi.hThread);
    }
    if STOPPING.load(Ordering::SeqCst) {
        unsafe {
            let _ = TerminateProcess(pi.hProcess, 0);
            let _ = CloseHandle(pi.hProcess);
        }
        return false;
    }
    let handle = pi.hProcess.0 as isize;
    *lock() = Some(Child {
        handle,
        pid: pi.dwProcessId,
    });
    write_pid(pi.dwProcessId);
    log(&format!("已拉起 native 小屏进程，PID={}", pi.dwProcessId));
    true
}

// ---------------------------------------------------------------------------
// 配置生成
// ---------------------------------------------------------------------------

/// 只重写配置文件，不拉起/重启进程。
/// 亮度、刷新间隔走热更新：小屏运行期自己重读这两个值，避免重启导致屏幕初始化闪烁。
pub fn rewrite_config(cfg: &ScreenConfig) {
    if let Err(e) = write_config(cfg) {
        log(&format!("生成小屏配置失败：{e}（小屏继续使用旧值）"));
    }
}

fn write_config(cfg: &ScreenConfig) -> Result<(), String> {
    let path = config_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let text = render_config(cfg);
    fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// 配置由胶囊设置生成，用户无需/不应手改小屏配置文件。
fn render_config(cfg: &ScreenConfig) -> String {
    let (orientation, flip) = match cfg.orientation.as_str() {
        "portrait-flip" => ("portrait", "flipped"),
        "landscape" => ("landscape", "normal"),
        "landscape-flip" => ("landscape", "flipped"),
        _ => ("portrait", "normal"),
    };
    let mut visible = Vec::new();
    if cfg.cpu {
        visible.push("cpu");
    }
    if cfg.memory {
        visible.push("memory");
    }
    if cfg.gpu {
        visible.push("gpu");
    }
    if cfg.network {
        visible.push("network");
    }
    if cfg.disk {
        visible.push("disk");
    }
    if cfg.storage {
        visible.push("storage");
    }
    if cfg.temp {
        visible.push("temp");
    }
    if cfg.clock {
        visible.push("clock");
    }
    let list = visible
        .iter()
        .map(|v| format!("\"{v}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let theme = native_theme_id(&cfg.theme);
    format!(
        "# 由核桃胶囊自动生成（B-10 native 引擎），请勿手改；改胶囊设置即可。\n\
         version = 1\n\
         \n\
         [display]\n\
         theme = \"{theme}\"\n\
         orientation = \"{orientation}\"\n\
         flip = \"{flip}\"\n\
         refresh_secs = {interval}\n\
         brightness = {brightness}\n\
         \n\
         [components]\n\
         visible = [{list}]\n\
         order = [{list}]\n\
         \n\
         [integration]\n\
         mode = \"ipc\"\n\
         pipe = \"hetao-screen\"\n\
         stale_ms = 3000\n\
         client_exe = \"hetao-capsule.exe\"\n",
        interval = cfg.interval.clamp(1, 10),
        brightness = cfg.brightness.clamp(10, 100),
    )
}

/// 胶囊主题名 → 小屏主题 id。旧名（white/official 等）按既有迁移规则归一；
/// 不在内置列表里的 id（如用户自定义的 `pulsecore`）原样透传，
/// 否则每次重启都会把自定义主题写回 dark。
fn native_theme_id(theme: &str) -> String {
    match theme {
        "light" | "white" => "light".to_string(),
        "bignum-light" | "official" => "bignum-light".to_string(),
        "bignum-dark" => "bignum-dark".to_string(),
        "dark" | "" => "dark".to_string(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// 路径与日志
// ---------------------------------------------------------------------------

fn appdata_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("hetao-capsule")
}

fn config_path() -> PathBuf {
    appdata_dir().join("screen").join("config.toml")
}

fn pid_file() -> PathBuf {
    appdata_dir().join("screen").join("native.pid")
}

/// screen-app 写出的设备状态文件（与 config.toml 同目录）。
fn status_path() -> PathBuf {
    appdata_dir().join("screen").join("status.json")
}

/// 读取小屏进程上报的设备状态；超过 10s 未更新视为未知（进程可能已退出）。
pub fn device_connected() -> Option<bool> {
    let text = fs::read_to_string(status_path()).ok()?;
    parse_device_status(&text, now_ms())
}

#[derive(serde::Deserialize)]
struct DeviceStatus {
    connected: bool,
    updated_ms: u64,
}

fn parse_device_status(text: &str, now_ms: u64) -> Option<bool> {
    let st: DeviceStatus = serde_json::from_str(text).ok()?;
    if now_ms.saturating_sub(st.updated_ms) > 10_000 {
        return None;
    }
    Some(st.connected)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn exe_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    Some(dir.join(EXE_NAME))
}

fn read_pid() -> Option<u32> {
    let text = fs::read_to_string(pid_file()).ok()?;
    text.trim().parse().ok()
}

fn write_pid(pid: u32) {
    let path = pid_file();
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(path, pid.to_string());
}

fn remove_pid() {
    let _ = fs::remove_file(pid_file());
}

fn log(msg: &str) {
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let line = format!("{}.{:03} {}", t.as_secs(), t.subsec_millis(), msg);
    let mut targets = vec![appdata_dir().join("screen_native.log")];
    // 兜底：AppData 不可写时（受限环境/权限异常）落到 exe 同目录，便于现场取证。
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            targets.push(dir.join("screen_native.log"));
        }
    }
    for path in targets {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| writeln!(f, "{line}"));
    }
}

fn lock() -> MutexGuard<'static, Option<Child>> {
    match CHILD.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(orientation: &str, interval: u32) -> ScreenConfig {
        ScreenConfig {
            orientation: orientation.to_string(),
            interval,
            cpu: true,
            memory: true,
            gpu: false,
            network: true,
            disk: false,
            ..Default::default()
        }
    }

    #[test]
    fn orientation_mapping_covers_four_modes() {
        let p = render_config(&cfg("portrait", 1));
        assert!(p.contains("orientation = \"portrait\""));
        assert!(p.contains("flip = \"normal\""));
        let pf = render_config(&cfg("portrait-flip", 2));
        assert!(pf.contains("orientation = \"portrait\""));
        assert!(pf.contains("flip = \"flipped\""));
        let l = render_config(&cfg("landscape", 3));
        assert!(l.contains("orientation = \"landscape\""));
        assert!(l.contains("flip = \"normal\""));
        let lf = render_config(&cfg("landscape-flip", 5));
        assert!(lf.contains("orientation = \"landscape\""));
        assert!(lf.contains("flip = \"flipped\""));
    }

    #[test]
    fn visible_components_follow_switches() {
        let text = render_config(&cfg("portrait", 1));
        assert!(text.contains("visible = [\"cpu\", \"memory\", \"network\"]"));
        assert!(!text.contains("\"gpu\""));
        assert!(!text.contains("\"disk\""));
        assert!(!text.contains("\"storage\""), "storage 默认关闭");
    }

    #[test]
    fn storage_card_can_be_enabled() {
        let mut c = cfg("portrait", 1);
        c.storage = true;
        let text = render_config(&c);
        assert!(text.contains("\"storage\""));
        assert!(text.contains("visible = [\"cpu\", \"memory\", \"network\", \"storage\"]"));
    }

    #[test]
    fn temp_and_clock_cards_append_in_order() {
        let mut c = cfg("portrait", 1);
        c.temp = true;
        c.clock = true;
        let text = render_config(&c);
        assert!(text.contains("visible = [\"cpu\", \"memory\", \"network\", \"temp\", \"clock\"]"));
    }

    #[test]
    fn child_command_passes_parent_pid() {
        let cmd = child_command(
            Path::new(r"C:\app\hetao-screen.exe"),
            Path::new(r"C:\cfg\config.toml"),
            4321,
        );
        assert_eq!(
            cmd,
            r#""C:\app\hetao-screen.exe" --config "C:\cfg\config.toml" --parent-pid 4321"#
        );
    }

    #[test]
    fn theme_name_is_mapped_for_native_engine() {
        for (input, expected) in [
            ("dark", "dark"),
            ("light", "light"),
            ("white", "light"),
            ("bignum-light", "bignum-light"),
            ("official", "bignum-light"),
            ("bignum-dark", "bignum-dark"),
            // 自定义主题（用户 themes/ 目录里的）原样透传，重启不会写回 dark
            ("pulsecore", "pulsecore"),
            ("unknown", "unknown"),
        ] {
            let mut c = cfg("portrait", 1);
            c.theme = input.to_string();
            let text = render_config(&c);
            assert!(
                text.contains(&format!("theme = \"{expected}\"")),
                "输入 {input} 应映射为 {expected}"
            );
        }
    }

    #[test]
    fn interval_is_clamped_and_written() {
        let text = render_config(&cfg("portrait", 1));
        assert!(text.contains("refresh_secs = 1"));
        let text = render_config(&cfg("portrait", 99));
        assert!(text.contains("refresh_secs = 10"));
    }

    #[test]
    fn device_status_requires_fresh_timestamp() {
        let now = 1_000_000u64;
        assert_eq!(
            parse_device_status(r#"{"connected":false,"updated_ms":999500}"#, now),
            Some(false)
        );
        assert_eq!(
            parse_device_status(r#"{"connected":true,"updated_ms":995000}"#, now),
            Some(true)
        );
        // 超过 10s 未更新：视为未知，避免进程退出后仍显示“已连接”
        assert_eq!(
            parse_device_status(r#"{"connected":true,"updated_ms":980000}"#, now),
            None
        );
        assert_eq!(parse_device_status("not json", now), None);
    }

    #[test]
    fn brightness_is_clamped_and_written() {
        let mut c = cfg("portrait", 1);
        c.brightness = 60;
        assert!(render_config(&c).contains("brightness = 60"));
        c.brightness = 0;
        assert!(render_config(&c).contains("brightness = 10"));
        c.brightness = 200;
        assert!(render_config(&c).contains("brightness = 100"));
    }

    #[test]
    fn unknown_orientation_falls_back_to_portrait_normal() {
        let text = render_config(&cfg("weird", 1));
        assert!(text.contains("orientation = \"portrait\""));
        assert!(text.contains("flip = \"normal\""));
    }

    #[test]
    fn generated_config_has_no_credentials() {
        let text = render_config(&cfg("portrait", 1));
        for bad in ["token", "Token", "Authorization", "http://", "https://"] {
            assert!(!text.contains(bad), "生成配置不得包含 {bad}");
        }
    }
}
