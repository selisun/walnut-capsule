//! 小屏脚本宿主：把 `tools/nas-screen/screen.ps1` 编进 exe，后台无窗口托管。
//!
//! 启动时把脚本落到 `%APPDATA%\hetao-capsule\screen.ps1`（内容有变化才覆盖），
//! 用 `CreateProcessW` + `CREATE_NO_WINDOW` 拉起 PowerShell，桌面上不会出现控制台窗口。
//! 后台线程每 5 秒巡检一次子进程，意外退出时重新拉起（连续失败退避 10 秒）；
//! 客户端退出时 `stop()` 结束子进程，避免残留 PowerShell。
//!
//! 脚本自身有单实例互斥，用户手动起的实例在跑时，宿主起的实例会自行退出，无需额外协调。

use std::ffi::c_void;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows::Win32::System::Threading::{
    CreateProcessW, GetExitCodeProcess, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW,
    TerminateProcess, CREATE_NO_WINDOW, PROCESS_INFORMATION, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, STARTUPINFOW,
};

const SCRIPT: &str = include_str!("../../tools/nas-screen/screen.ps1");

/// 内置主题：每套主题一个 ps1，启动时落到 `data_dir\themes\`，由 screen.ps1 按名字加载。
const THEMES: [(&str, &str); 4] = [
    (
        "dark",
        include_str!("../../tools/nas-screen/themes/dark.ps1"),
    ),
    (
        "bignum-light",
        include_str!("../../tools/nas-screen/themes/bignum-light.ps1"),
    ),
    (
        "light",
        include_str!("../../tools/nas-screen/themes/light.ps1"),
    ),
    (
        "bignum-dark",
        include_str!("../../tools/nas-screen/themes/bignum-dark.ps1"),
    ),
];
const POWERSHELL: &str = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";

/// GetExitCodeProcess 的"仍在运行"哨兵值（windows crate 未导出该常量）。
const STILL_ACTIVE: u32 = 259;
/// 巡检间隔。
const TICK: Duration = Duration::from_secs(5);
/// 连续失败（起不来或刚起就退）时的重试间隔。
const RETRY_AFTER_FAIL: Duration = Duration::from_secs(10);

struct Child {
    /// 进程句柄，保持打开用于查存活 / 结束进程
    handle: isize,
    pid: u32,
    started: Instant,
}

static CHILD: Mutex<Option<Child>> = Mutex::new(None);
static STOPPING: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);
/// 启动代号：每次 start / stop 自增，老的看门狗线程发现自己过期就退出，
/// 避免「关掉再开启」时两个看门狗同时拉起进程。
static GEN: AtomicU32 = AtomicU32::new(0);

/// 启动宿主（幂等）。失败只写日志，不影响客户端主流程。
pub fn start() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    STOPPING.store(false, Ordering::SeqCst);

    let script = data_dir().join("screen.ps1");
    match write_script(&script) {
        Ok(true) => log("已更新小屏脚本"),
        Ok(false) => {}
        Err(e) => log(&format!("写入小屏脚本失败：{e}")),
    }
    write_themes();

    let _ = spawn(&script);

    let path = script.clone();
    let gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    match thread::Builder::new()
        .name("screen-host".into())
        .spawn(move || watchdog(path, gen))
    {
        Ok(_) => {}
        Err(e) => log(&format!("看门狗线程启动失败：{e}")),
    }
}

/// 结束宿主：置停止标志并结束子进程。之后可以再次 start()。
pub fn stop() {
    STOPPING.store(true, Ordering::SeqCst);
    GEN.fetch_add(1, Ordering::SeqCst);
    // 先取走子进程句柄再释放锁：后面的 PID 校验要拉起 PowerShell 查命令行，
    // 不能长时间占着锁，避免设置页/看门狗被阻塞。
    let child = lock().take();
    if let Some(child) = child {
        let handle = HANDLE(child.handle as *mut c_void);
        let mut code = 0u32;
        let alive =
            unsafe { GetExitCodeProcess(handle, &mut code).is_ok() && code == STILL_ACTIVE };
        let terminated = !alive || unsafe { TerminateProcess(handle, 0).is_ok() };
        unsafe {
            let _ = CloseHandle(handle);
        }
        if terminated {
            log(&format!("已结束后台小屏脚本，PID={}", child.pid));
            cleanup_identity_files(child.pid);
        } else {
            log(&format!(
                "结束后台小屏脚本失败，PID={}，保留身份文件",
                child.pid
            ));
        }
    } else if let Some(pid) = read_pid() {
        // 升级/异常退出后残留的脚本（不是本进程拉起的，拿不到子进程句柄）：
        // 身份校验和终止必须使用同一个进程句柄，校验通过后直接终止该句柄，
        // 避免验证完成后 PID 被复用导致误杀新进程。
        match open_verified_instance(pid) {
            Some(handle) => {
                let mut code = 0u32;
                let alive = unsafe {
                    GetExitCodeProcess(handle, &mut code).is_ok() && code == STILL_ACTIVE
                };
                let terminated = !alive || unsafe { TerminateProcess(handle, 0).is_ok() };
                unsafe {
                    let _ = CloseHandle(handle);
                }
                if terminated {
                    log(&format!("已结束后台残留小屏脚本，PID={pid}"));
                    cleanup_identity_files(pid);
                } else {
                    log(&format!("结束残留小屏脚本失败，PID={pid}，保留身份文件"));
                }
            }
            None => {
                // 无法验证身份（WMI 超时、命令行不匹配或伪造）：
                // 只有确认进程已经不存在时才清理过期文件，否则保留以便下次重试。
                if process_alive(pid) {
                    log(&format!(
                        "跳过结束 PID={pid}：未能验证为小屏脚本实例，保留身份文件"
                    ));
                } else {
                    log(&format!("PID={pid} 已不存在，清理过期身份文件"));
                    cleanup_identity_files(pid);
                }
            }
        }
    } else if heartbeat_fresh() {
        // 老版本脚本没有可验证的身份文件：宁可放弃终止，也不误杀无关进程
        log("检测到小屏心跳但缺少可验证的 PID，跳过终止");
    }
    STARTED.store(false, Ordering::SeqCst);
}

/// 小屏宿主是否在跑（设置页用来切换「开启 / 关闭」按钮的可用态）。
/// 除了本进程拉起的子进程，还要认「pid 文件里仍然活着的脚本」——
/// 覆盖软件升级后旧脚本没被结束、界面却显示已连接的情况。
pub fn is_running() -> bool {
    if lock().is_some() {
        return true;
    }
    match read_pid() {
        Some(pid) => pid_is_our_script(pid),
        None => false,
    }
}

/// 脚本运行时写的 pid 文件（客户端据此接管残留实例）。
fn pid_file() -> PathBuf {
    data_dir().join("screen.pid")
}

/// 脚本写的进程身份文件：`pid|创建时间(FILETIME)|可执行路径`。
/// 客户端只信这份由脚本自己写下的信息，避免外部伪造 PID 导致误杀。
fn pid_owner_file() -> PathBuf {
    data_dir().join("screen.pid.owner")
}

fn read_pid() -> Option<u32> {
    read_pid_at(&pid_file())
}

fn read_pid_at(path: &Path) -> Option<u32> {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
}

struct PidOwner {
    pid: u32,
    created: u64,
    exe: String,
}

fn read_pid_owner() -> Option<PidOwner> {
    parse_owner_file(&pid_owner_file())
}

fn parse_owner_file(path: &Path) -> Option<PidOwner> {
    let text = fs::read_to_string(path).ok()?;
    // PowerShell 5.1 的 Set-Content -Encoding UTF8 会写入 BOM，
    // 而 str::trim() 不会移除 U+FEFF，解析前先剥离。
    parse_owner(text.trim_start_matches('\u{feff}'))
}

/// 解析 `pid|创建时间|可执行路径`；缺字段或为空一律视为不可信。
fn parse_owner(raw: &str) -> Option<PidOwner> {
    let mut parts = raw.trim().splitn(3, '|');
    let pid = parts.next()?.trim().parse::<u32>().ok()?;
    let created = parts.next()?.trim().parse::<u64>().ok()?;
    let exe = parts.next()?.trim().to_string();
    if pid == 0 || created == 0 || exe.is_empty() {
        return None;
    }
    Some(PidOwner { pid, created, exe })
}

/// 通过已打开的句柄读取映像路径与创建时间。
fn process_identity_handle(h: HANDLE) -> Option<(String, u64)> {
    unsafe {
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let image =
            QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
                .ok()
                .map(|_| String::from_utf16_lossy(&buf[..len as usize]));
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let times_ok =
            GetProcessTimes(h, &mut created, &mut exited, &mut kernel, &mut user).is_ok();
        let image = image?;
        if !times_ok {
            return None;
        }
        let ticks = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
        Some((image, ticks))
    }
}

/// 读取目标进程的完整映像路径与创建时间；进程不存在或权限不足返回 None。
fn process_identity(pid: u32) -> Option<(String, u64)> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let identity = process_identity_handle(h);
        let _ = CloseHandle(h);
        identity
    }
}

/// 进程是否仍然存活（用于清理过期的身份文件）。
fn process_alive(pid: u32) -> bool {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut code = 0u32;
        let alive = GetExitCodeProcess(h, &mut code).is_ok() && code == STILL_ACTIVE;
        let _ = CloseHandle(h);
        alive
    }
}

/// 一次取得查询 + 终止权限：验证和终止必须使用同一个句柄。
fn open_process_for_kill(pid: u32) -> Option<HANDLE> {
    unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE,
            false,
            pid,
        )
        .ok()
    }
}

fn same_path(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// 句柄读到的身份是否与脚本声明一致，且映像必须是 powershell.exe。
fn identity_matches(owner: &PidOwner, image: &str, created: u64) -> bool {
    created == owner.created && same_path(image, &owner.exe) && same_path(image, POWERSHELL)
}

/// 轻量身份校验：PID、创建时间、映像路径都对得上，才认为 pid 文件可信。
/// 用于「是否在运行」的判断，不启动额外进程。
fn pid_is_our_script(pid: u32) -> bool {
    if pid <= 4 || pid == std::process::id() {
        return false;
    }
    let Some(owner) = read_pid_owner() else {
        return false;
    };
    if owner.pid != pid {
        return false;
    }
    let Some((image, created)) = process_identity(pid) else {
        return false;
    };
    created == owner.created && same_path(&image, &owner.exe) && same_path(&image, POWERSHELL)
}

/// 打开进程并完成全部身份校验，返回**已经过验证的原始句柄**。
/// 调用方只能终止该句柄指向的进程，禁止再用 PID 重新打开，
/// 这样即使验证期间 PID 被系统复用，也不会影响新进程。
fn open_verified_instance(pid: u32) -> Option<HANDLE> {
    if pid <= 4 || pid == std::process::id() {
        return None;
    }
    let owner = read_pid_owner()?;
    if owner.pid != pid {
        return None;
    }
    // 查询 + 终止权限一次取得；从这里开始所有校验都针对同一个句柄
    let h = open_process_for_kill(pid)?;
    let verified = match process_identity_handle(h) {
        // 命令行查询期间句柄保持打开，校验完成后直接终止该句柄
        Some((image, created)) => {
            identity_matches(&owner, &image, created)
                && command_line_targets_script(pid, &script_path())
        }
        None => false,
    };
    if !verified {
        unsafe {
            let _ = CloseHandle(h);
        }
        return None;
    }
    Some(h)
}

/// 由 PowerShell 读取目标进程命令行并做精确比对；路径通过环境变量传入，
/// 避免拼接命令行带来的转义/编码问题。3 秒内没结果按「无法确认」处理。
fn command_line_targets_script(pid: u32, script: &Path) -> bool {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    let target = script.display().to_string();
    let quoted = format!("-File \"{target}\"");
    let bare = format!("-File {target}");
    let cmd = format!(
        "$p=Get-CimInstance Win32_Process -Filter \"ProcessId={pid}\" -ErrorAction SilentlyContinue; \
         if($p -and $p.CommandLine -and ($p.CommandLine.IndexOf($env:HETAO_EXPECT_Q,[System.StringComparison]::OrdinalIgnoreCase) -ge 0 -or $p.CommandLine.IndexOf($env:HETAO_EXPECT_B,[System.StringComparison]::OrdinalIgnoreCase) -ge 0)){{'1'}}else{{'0'}}"
    );
    let Ok(mut child) = Command::new(POWERSHELL)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            cmd.as_str(),
        ])
        .env("HETAO_EXPECT_Q", quoted)
        .env("HETAO_EXPECT_B", bare)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .spawn()
    else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(40)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
    let mut out = String::new();
    if let Some(mut so) = child.stdout.take() {
        let _ = so.read_to_string(&mut out);
    }
    out.trim() == "1"
}

fn script_path() -> PathBuf {
    data_dir().join("screen.ps1")
}

/// 清理与已确认退出/终止的 PID 一致的身份文件。
/// pid 与 owner 必须同时指向该 PID 才一起删除；任一不一致时保持原样，
/// 避免留下"只有 pid 没有 owner"或反之的半套身份。
fn cleanup_identity_files(pid: u32) {
    cleanup_identity_files_at(&pid_file(), &pid_owner_file(), pid);
}

fn cleanup_identity_files_at(pid_path: &Path, owner_path: &Path, pid: u32) {
    if read_pid_at(pid_path) != Some(pid) {
        return;
    }
    if !owner_path.exists() {
        // 旧版本只写 pid 文件：直接清掉，不会留下半套身份
        let _ = fs::remove_file(pid_path);
        return;
    }
    if parse_owner_file(owner_path).map(|o| o.pid) != Some(pid) {
        return;
    }
    let _ = fs::remove_file(pid_path);
    let _ = fs::remove_file(owner_path);
}

/// 心跳文件 60 秒内更新过：说明有脚本实例在跑（可能是老版本留下的孤儿进程）。
fn heartbeat_fresh() -> bool {
    let Ok(raw) = fs::read_to_string(data_dir().join("screen.status.json")) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    let ts = v.get("ts").and_then(|x| x.as_i64()).unwrap_or(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    now.saturating_sub(ts) <= 60
}

/// 通知脚本重试一次串口（端口被占用后脚本会停下来等这条指令）。
pub fn request_reconnect() {
    let p = data_dir().join("screen.ctl");
    let seq = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    if let Err(e) = fs::write(&p, seq.to_string()) {
        log(&format!("写入重连指令失败：{e}"));
    }
}

fn watchdog(script: PathBuf, gen: u32) {
    let mut failures = 0u32;
    loop {
        // 分段休眠：客户端退出时不必等满一个巡检周期
        if !sleep_interruptible(TICK) || GEN.load(Ordering::SeqCst) != gen {
            return;
        }
        match poll() {
            Poll::Alive => {}
            Poll::Missing => {
                if failures > 0 && !sleep_interruptible(RETRY_AFTER_FAIL) {
                    return;
                }
                failures = if spawn(&script) { 0 } else { failures + 1 };
            }
            Poll::Exited { code, lived } => {
                log(&format!(
                    "后台小屏脚本已退出，退出码={code}，运行 {}s",
                    lived.as_secs()
                ));
                // 刚起就退视为失败，连续失败才退避，避免高频重启
                if lived < RETRY_AFTER_FAIL {
                    failures += 1;
                } else {
                    failures = 0;
                }
                if failures > 0 && !sleep_interruptible(RETRY_AFTER_FAIL) {
                    return;
                }
                failures = if spawn(&script) { 0 } else { failures + 1 };
            }
        }
    }
}

enum Poll {
    Alive,
    /// 没有子进程记录（启动失败等），需要重新拉起
    Missing,
    Exited {
        code: u32,
        lived: Duration,
    },
}

fn poll() -> Poll {
    let mut guard = lock();
    if guard.is_none() {
        drop(guard);
        // 任务模式（或用户手动起的实例）：没有子进程句柄，按校验通过的 pid 判断存活
        if let Some(pid) = read_pid() {
            if pid_is_our_script(pid) {
                return Poll::Alive;
            }
        }
        return Poll::Missing;
    }
    let handle = guard.as_ref().map(|c| c.handle).unwrap_or(0);
    let mut code = 0u32;
    let code = match unsafe { GetExitCodeProcess(HANDLE(handle as *mut c_void), &mut code) } {
        Ok(()) => code,
        Err(e) => {
            log(&format!("查询小屏进程状态失败：{e}"));
            u32::MAX
        }
    };
    if code == STILL_ACTIVE {
        return Poll::Alive;
    }
    let Some(child) = guard.take() else {
        return Poll::Missing;
    };
    unsafe {
        let _ = CloseHandle(HANDLE(handle as *mut c_void));
    }
    Poll::Exited {
        code,
        lived: child.started.elapsed(),
    }
}

/// 拉起 PowerShell 跑脚本。返回是否成功。
fn spawn(script: &Path) -> bool {
    if STOPPING.load(Ordering::SeqCst) {
        return false;
    }
    let mut cmd: Vec<u16> = format!(
        "\"{POWERSHELL}\" -NoProfile -ExecutionPolicy Bypass -File \"{}\"",
        script.display()
    )
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
        log(&format!("启动后台小屏脚本失败：{e}"));
        return false;
    }
    unsafe {
        let _ = CloseHandle(pi.hThread);
    }
    // 与 stop() 竞争：刚拉起就赶上退出，直接收掉，别留活着的孤儿进程
    if STOPPING.load(Ordering::SeqCst) {
        unsafe {
            let _ = TerminateProcess(pi.hProcess, 0);
            let _ = CloseHandle(pi.hProcess);
        }
        return false;
    }
    log(&format!("已启动后台小屏脚本，PID={}", pi.dwProcessId));

    let mut guard = lock();
    if let Some(old) = guard.take() {
        let h = HANDLE(old.handle as *mut c_void);
        unsafe {
            let _ = TerminateProcess(h, 0);
            let _ = CloseHandle(h);
        }
    }
    *guard = Some(Child {
        handle: pi.hProcess.0 as isize,
        pid: pi.dwProcessId,
        started: Instant::now(),
    });
    true
}

/// 休眠指定时长；期间收到停止请求则立刻返回 false。
fn sleep_interruptible(d: Duration) -> bool {
    const STEP: Duration = Duration::from_millis(250);
    let mut left = d;
    while left > Duration::ZERO {
        if STOPPING.load(Ordering::SeqCst) {
            return false;
        }
        let s = left.min(STEP);
        thread::sleep(s);
        left -= s;
    }
    !STOPPING.load(Ordering::SeqCst)
}

fn lock() -> MutexGuard<'static, Option<Child>> {
    CHILD.lock().unwrap_or_else(|e| e.into_inner())
}

fn data_dir() -> PathBuf {
    let dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("hetao-capsule");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// 写脚本到本地；内容一致时不重复写。返回是否真的覆盖了。
fn write_script(path: &Path) -> std::io::Result<bool> {
    write_text(path, SCRIPT)
}

/// 把内置主题写到 `data_dir\themes\`（内容一致时跳过）。
fn write_themes() {
    let dir = data_dir().join("themes");
    if let Err(e) = fs::create_dir_all(&dir) {
        log(&format!("创建主题目录失败：{e}"));
        return;
    }
    for (name, body) in THEMES.iter() {
        let p = dir.join(format!("{name}.ps1"));
        match write_text(&p, body) {
            Ok(true) => log(&format!("已更新主题脚本 {name}.ps1")),
            Ok(false) => {}
            Err(e) => log(&format!("写入主题脚本 {name}.ps1 失败：{e}")),
        }
    }
}

/// 内容一致时不重复写，返回是否真的覆盖了。
fn write_text(path: &Path, body: &str) -> std::io::Result<bool> {
    if let Ok(existing) = fs::read_to_string(path) {
        if existing == body {
            return Ok(false);
        }
    }
    fs::write(path, body)?;
    Ok(true)
}

/// 只记关键事件：启动成功/失败、退出、重启、结束。
fn log(msg: &str) {
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(data_dir().join("screen_host.log"))
        .and_then(|mut f| {
            writeln!(
                f,
                "{}.{:03} {}",
                t.as_secs() % 100000,
                t.subsec_millis(),
                msg
            )
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_parsing_requires_all_fields() {
        assert!(parse_owner("").is_none());
        assert!(parse_owner("123").is_none());
        assert!(parse_owner("123|456").is_none());
        assert!(parse_owner("0|456|C:\\x.exe").is_none());
        assert!(parse_owner("123|0|C:\\x.exe").is_none());
        assert!(parse_owner("abc|456|C:\\x.exe").is_none());
        assert!(parse_owner("123|456|").is_none());

        let o = parse_owner(
            "123|456|C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\r\n",
        )
        .expect("完整字段应解析成功");
        assert_eq!(o.pid, 123);
        assert_eq!(o.created, 456);
        assert!(same_path(&o.exe, POWERSHELL));
    }

    #[test]
    fn path_compare_is_case_insensitive() {
        assert!(same_path(POWERSHELL, &POWERSHELL.to_uppercase()));
        assert!(!same_path(POWERSHELL, r"C:\Windows\System32\cmd.exe"));
    }

    #[test]
    fn pid_owner_mismatch_is_rejected() {
        // 伪造文件：PID 与 owner 记录不一致时不得通过校验
        let o =
            parse_owner("123|456|C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe")
                .unwrap();
        assert_ne!(o.pid, 124);
    }

    #[test]
    fn command_line_check_accepts_only_our_script() {
        use std::os::windows::process::CommandExt;
        use std::process::Command;

        let dir = std::env::temp_dir().join("hetao-h2-rust-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let decoy = dir.join("screen.ps1");
        let other = dir.join("other.ps1");
        fs::write(&decoy, "Start-Sleep -Seconds 20").unwrap();
        fs::write(&other, "Start-Sleep -Seconds 20").unwrap();

        let spawn = |p: &Path| {
            Command::new(POWERSHELL)
                .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
                .arg(p)
                .creation_flags(0x0800_0000)
                .spawn()
                .expect("拉起测试用 PowerShell")
        };
        let mut ours = spawn(&decoy);
        let mut foreign = spawn(&other);
        thread::sleep(Duration::from_millis(1200));

        let ours_ok = command_line_targets_script(ours.id(), &decoy);
        let foreign_ok = command_line_targets_script(foreign.id(), &decoy);

        let _ = ours.kill();
        let _ = ours.wait();
        let _ = foreign.kill();
        let _ = foreign.wait();
        let _ = fs::remove_dir_all(&dir);

        assert!(ours_ok, "应识别出命令行指向本脚本的实例");
        assert!(!foreign_ok, "不得把其他 PowerShell 判为本脚本实例");
    }

    #[test]
    fn verified_handle_terminates_original_process() {
        use std::os::windows::process::CommandExt;
        use std::process::Command;

        let dir = std::env::temp_dir().join("hetao-h2-handle-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let decoy = dir.join("screen.ps1");
        fs::write(&decoy, "Start-Sleep -Seconds 20").unwrap();

        let mut child = Command::new(POWERSHELL)
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&decoy)
            .creation_flags(0x0800_0000)
            .spawn()
            .expect("拉起测试用 PowerShell");
        thread::sleep(Duration::from_millis(1200));

        // 与 stop() 一致：查询 + 终止权限一次取得，身份校验后再终止同一个句柄
        let handle = open_process_for_kill(child.id()).expect("应能打开进程句柄");
        let identity = process_identity_handle(handle).expect("应能通过同一句柄读取身份");
        assert!(same_path(&identity.0, POWERSHELL));
        assert!(identity.1 > 0);

        let terminated = unsafe { TerminateProcess(handle, 0).is_ok() };
        unsafe {
            let _ = CloseHandle(handle);
        }
        assert!(terminated, "应能通过验证过的原始句柄终止进程");
        assert!(child.wait().is_ok(), "测试进程应被回收");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleanup_removes_only_matching_identity_files() {
        let dir = std::env::temp_dir().join("hetao-h2-cleanup-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let pid_path = dir.join("screen.pid");
        let owner_path = dir.join("screen.pid.owner");
        let owner = |pid: u32| {
            format!("{pid}|456|C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe")
        };

        // pid 与 owner 都指向同一 PID：一起清理
        fs::write(&pid_path, "123").unwrap();
        fs::write(&owner_path, owner(123)).unwrap();
        cleanup_identity_files_at(&pid_path, &owner_path, 123);
        assert!(!pid_path.exists() && !owner_path.exists());

        // owner 指向其他实例：保持原样，避免留下半套身份
        fs::write(&pid_path, "123").unwrap();
        fs::write(&owner_path, owner(456)).unwrap();
        cleanup_identity_files_at(&pid_path, &owner_path, 123);
        assert!(pid_path.exists() && owner_path.exists());

        // 旧版本只有 pid 文件：可以清理
        fs::write(&pid_path, "123").unwrap();
        let _ = fs::remove_file(&owner_path);
        cleanup_identity_files_at(&pid_path, &owner_path, 123);
        assert!(!pid_path.exists());

        // pid 文件指向其他进程：不得清理
        fs::write(&pid_path, "999").unwrap();
        fs::write(&owner_path, owner(999)).unwrap();
        cleanup_identity_files_at(&pid_path, &owner_path, 123);
        assert!(pid_path.exists() && owner_path.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn identity_mismatch_is_rejected() {
        let owner =
            parse_owner("123|456|C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe")
                .unwrap();
        assert!(identity_matches(&owner, POWERSHELL, 456));
        // 创建时间不一致（PID 重用场景）
        assert!(!identity_matches(&owner, POWERSHELL, 457));
        // 映像路径不一致
        assert!(!identity_matches(
            &owner,
            r"C:\Windows\System32\cmd.exe",
            456
        ));
    }

    #[test]
    fn open_verified_rejects_invalid_pid() {
        assert!(open_verified_instance(0).is_none());
        assert!(open_verified_instance(4).is_none());
        assert!(open_verified_instance(std::process::id()).is_none());
    }

    #[test]
    fn terminated_original_handle_fails_safely() {
        use std::os::windows::process::CommandExt;
        use std::process::Command;

        let dir = std::env::temp_dir().join("hetao-h2-stale-handle-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let decoy = dir.join("screen.ps1");
        fs::write(&decoy, "Start-Sleep -Seconds 20").unwrap();

        let mut child = Command::new(POWERSHELL)
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&decoy)
            .creation_flags(0x0800_0000)
            .spawn()
            .expect("拉起测试用 PowerShell");
        thread::sleep(Duration::from_millis(1200));

        let handle = open_process_for_kill(child.id()).expect("应能打开进程句柄");
        assert!(process_identity_handle(handle).is_some());

        // 原进程在验证后退出：对旧句柄终止必须失败，且不会影响别的进程
        let _ = child.kill();
        let _ = child.wait();
        let terminated = unsafe { TerminateProcess(handle, 0).is_ok() };
        unsafe {
            let _ = CloseHandle(handle);
        }
        assert!(!terminated, "原进程已退出时，旧句柄不应终止任何进程");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn owner_file_parses_with_and_without_bom() {
        let dir = std::env::temp_dir().join("hetao-owner-bom-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("screen.pid.owner");
        let body = "123|456|C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe";

        // 不带 BOM：正常解析
        fs::write(&path, body.as_bytes()).unwrap();
        let plain = parse_owner_file(&path).expect("无 BOM 应解析成功");
        assert_eq!(plain.pid, 123);
        assert_eq!(plain.created, 456);
        assert_eq!(
            plain.exe,
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
        );

        // 真实写入 EF BB BF 字节（PowerShell 5.1 Set-Content -Encoding UTF8 的实际输出）
        let mut bytes = vec![0xEFu8, 0xBB, 0xBF];
        bytes.extend_from_slice(body.as_bytes());
        fs::write(&path, &bytes).unwrap();
        assert_eq!(
            &fs::read(&path).unwrap()[..3],
            &[0xEF, 0xBB, 0xBF],
            "测试文件必须包含真实 BOM 字节"
        );
        let bom = parse_owner_file(&path).expect("带 BOM 应解析成功");
        assert_eq!(bom.pid, 123);
        assert_eq!(bom.created, 456);
        assert_eq!(
            bom.exe,
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
        );

        // BOM + 前后空白与换行
        let mut bytes = vec![0xEFu8, 0xBB, 0xBF];
        bytes.extend_from_slice(b"\r\n  123|456|C:\\Windows\\System32\\cmd.exe  \r\n");
        fs::write(&path, &bytes).unwrap();
        let padded = parse_owner_file(&path).expect("BOM 与空白应被忽略");
        assert_eq!(padded.pid, 123);
        assert_eq!(padded.created, 456);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn owner_file_rejects_invalid_fields() {
        let dir = std::env::temp_dir().join("hetao-owner-invalid-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("screen.pid.owner");
        let write_bom = |body: &str| {
            let mut bytes = vec![0xEFu8, 0xBB, 0xBF];
            bytes.extend_from_slice(body.as_bytes());
            fs::write(&path, &bytes).unwrap();
        };

        // PID 非数字
        write_bom("abc|456|C:\\x.exe");
        assert!(parse_owner_file(&path).is_none());
        // FILETIME 格式错误
        write_bom("123|abc|C:\\x.exe");
        assert!(parse_owner_file(&path).is_none());
        // 可执行路径缺失
        write_bom("123|456|");
        assert!(parse_owner_file(&path).is_none());
        write_bom("123|456");
        assert!(parse_owner_file(&path).is_none());
        // 0 值不可信
        write_bom("0|456|C:\\x.exe");
        assert!(parse_owner_file(&path).is_none());
        write_bom("123|0|C:\\x.exe");
        assert!(parse_owner_file(&path).is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn owner_file_parses_real_powershell_utf8_output() {
        use std::os::windows::process::CommandExt;
        use std::process::Command;

        let dir = std::env::temp_dir().join("hetao-owner-powershell-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("screen.pid.owner");
        let content = "123|456|C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe";

        // 用项目实际使用的 PowerShell 5.1 写文件，复现真实的 UTF-8 BOM 输出
        let cmd = format!(
            "Set-Content -LiteralPath '{}' -Value '{}' -Encoding UTF8",
            path.display(),
            content
        );
        let status = Command::new(POWERSHELL)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                cmd.as_str(),
            ])
            .creation_flags(0x0800_0000)
            .status()
            .expect("运行 PowerShell 写入测试文件");
        assert!(status.success());

        let raw = fs::read(&path).unwrap();
        assert_eq!(
            &raw[..3],
            &[0xEF, 0xBB, 0xBF],
            "PowerShell 5.1 应写出 UTF-8 BOM"
        );
        let owner = parse_owner_file(&path).expect("PowerShell 实际生成的文件应解析成功");
        assert_eq!(owner.pid, 123);
        assert_eq!(owner.created, 456);

        let _ = fs::remove_dir_all(&dir);
    }
}
