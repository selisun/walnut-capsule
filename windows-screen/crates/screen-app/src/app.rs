//! 小屏进程生命周期与模块装配。第一阶段不打开设备、不连接 IPC。

use std::path::{Path, PathBuf};

use screen_adapter_api::{DeviceAdapter, DeviceInfo};
use screen_adapter_turzx_cdc::TurzxCdcAdapter;
use screen_core::{
    Frame, FrameBuffer, LayoutTree, Orientation, PixelFormat, ScreenProfile, Snapshot, ThemeConfig,
};
use screen_data::{DataError, DataState, SnapshotGate};
use screen_device::DeviceManager;
use screen_ipc::{ConnectionState, Message, MessageKind};
use screen_layout::{build, TemplateKind};
use screen_protocol::rgba_to_rgb565le;
use screen_render::{
    bind_with_clock, create_text_renderer, local_clock, render_legacy, render_notice_bg,
    render_with_bg, render_with_metrics_bg, BackgroundImage, FontSpec, LegacyData,
};
use screen_theme::{load_dir, load_str, ThemeStore};
use screen_transport_cdc::CdcTransport;

use crate::config::{Config, ConfigError};
use crate::orientation::{Direction, DirectionError};
use crate::runtime::RunOptions;
use crate::session::{DeviceSession, SessionState};

pub const BUILTIN_DEFAULT_THEME: &str = include_str!("../../../assets/themes/default/theme.json");
pub const BUILTIN_DARK_THEME: &str = include_str!("../../../assets/themes/dark/theme.json");
pub const BUILTIN_LIGHT_THEME: &str = include_str!("../../../assets/themes/light/theme.json");
pub const BUILTIN_BIGNUM_DARK_THEME: &str =
    include_str!("../../../assets/themes/bignum-dark/theme.json");
pub const BUILTIN_BIGNUM_LIGHT_THEME: &str =
    include_str!("../../../assets/themes/bignum-light/theme.json");

/// 内置主题（首项为默认回退）。
pub const BUILTIN_THEMES: [&str; 5] = [
    BUILTIN_DEFAULT_THEME,
    BUILTIN_DARK_THEME,
    BUILTIN_LIGHT_THEME,
    BUILTIN_BIGNUM_DARK_THEME,
    BUILTIN_BIGNUM_LIGHT_THEME,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppError {
    Config(String),
    Theme(String),
    Io(String),
    State(String),
    Data(String),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Config(msg) => write!(f, "配置错误：{msg}"),
            AppError::Theme(msg) => write!(f, "主题错误：{msg}"),
            AppError::Io(msg) => write!(f, "IO 错误：{msg}"),
            AppError::State(msg) => write!(f, "状态错误：{msg}"),
            AppError::Data(msg) => write!(f, "数据错误：{msg}"),
        }
    }
}

impl std::error::Error for AppError {}

impl From<ConfigError> for AppError {
    fn from(value: ConfigError) -> Self {
        AppError::Config(value.to_string())
    }
}

impl From<DirectionError> for AppError {
    fn from(value: DirectionError) -> Self {
        AppError::Config(value.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppState {
    WaitingDevice,
    Running,
    Shutdown,
}

pub struct ScreenApp {
    config: Config,
    config_path: Option<PathBuf>,
    config_used_defaults: bool,
    themes: ThemeStore,
    current_theme: ThemeConfig,
    background_image: Option<BackgroundImage>,
    devices: DeviceManager,
    gate: SnapshotGate,
    ipc_state: ConnectionState,
    state: AppState,
    session: Option<DeviceSession<dyn DeviceAdapter>>,
    snapshot: Option<Snapshot>,
}

impl ScreenApp {
    pub fn init(config_path: Option<&Path>) -> Result<Self, AppError> {
        let resolved = config_path
            .map(Path::to_path_buf)
            .or_else(default_config_path);
        let (config, used_defaults) = match &resolved {
            Some(path) => Config::load_or_default(path)?,
            None => (Config::default(), true),
        };

        let theme = load_str(BUILTIN_DEFAULT_THEME).map_err(|e| AppError::Theme(e.to_string()))?;
        let mut themes = ThemeStore::new();
        for json in BUILTIN_THEMES {
            let builtin = load_str(json).map_err(|e| AppError::Theme(e.to_string()))?;
            themes
                .register(builtin, true)
                .map_err(|e| AppError::Theme(e.to_string()))?;
        }

        // 用户主题目录：与 config.toml 同级的 themes/（可选；坏文件只跳过并记录）
        if let Some(dir) = resolved
            .as_ref()
            .and_then(|path| path.parent())
            .map(|dir| dir.join("themes"))
        {
            let (loaded, errors) = load_dir(&dir);
            for user_theme in loaded {
                if let Err(err) = themes.register(user_theme, false) {
                    eprintln!("用户主题注册失败：{err}");
                }
            }
            for (path, err) in errors {
                eprintln!("用户主题已跳过 {}：{err}", path.display());
            }
        }

        let current_theme = if config.display.theme.is_empty() || config.display.theme == "default"
        {
            theme
        } else {
            match themes.switch(&config.display.theme) {
                Ok(selected) => selected,
                Err(err) => {
                    eprintln!("主题切换失败（{err}），回退默认主题");
                    theme
                }
            }
        };

        let period_ms = (config.display.refresh_secs as i64).saturating_mul(1_000);
        let gate = SnapshotGate::new(config.integration.stale_ms, period_ms)
            .map_err(|e| AppError::Config(e.to_string()))?;

        let background_image = load_background_image(
            &current_theme,
            resolved.as_deref(),
            themes.is_builtin(&current_theme.id),
        );

        Ok(Self {
            config,
            config_path: resolved,
            config_used_defaults: used_defaults,
            themes,
            current_theme,
            background_image,
            devices: DeviceManager::single_device(),
            gate,
            ipc_state: ConnectionState::WaitingClient,
            state: AppState::WaitingDevice,
            session: None,
            snapshot: None,
        })
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// 实际使用的配置文件路径（未找到默认路径时为 `None`）。
    pub fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }

    pub fn theme(&self) -> &ThemeConfig {
        &self.current_theme
    }

    pub fn theme_store(&self) -> &ThemeStore {
        &self.themes
    }

    /// 当前主题与用户配置合成的字号缩放。
    pub fn font_scale(&self) -> f32 {
        effective_font_scale(&self.current_theme, self.config.theme.font_scale)
    }

    pub fn devices(&self) -> &DeviceManager {
        &self.devices
    }

    pub fn data_state(&self) -> DataState {
        self.gate.state()
    }

    pub fn ipc_state(&self) -> ConnectionState {
        self.ipc_state
    }

    pub fn state(&self) -> AppState {
        self.state
    }

    pub fn snapshot(&self) -> Option<&Snapshot> {
        self.snapshot.as_ref()
    }

    pub fn layout(&self) -> Option<LayoutTree> {
        None
    }

    /// 模拟收到客户端快照（第二阶段由 IPC 调用）。本阶段仅用于状态机测试。
    pub fn receive_snapshot(
        &mut self,
        snapshot: &Snapshot,
        now_ms: i64,
    ) -> Result<DataState, AppError> {
        self.gate
            .accept(snapshot, now_ms)
            .map_err(|e: DataError| AppError::Data(e.to_string()))?;
        self.snapshot = Some(snapshot.clone());
        self.ipc_state = ConnectionState::Connected;
        self.state = AppState::Running;
        Ok(self.gate.state())
    }

    pub fn tick(&mut self, now_ms: i64) -> DataState {
        let state = self.gate.tick(now_ms);
        if state == DataState::WaitingClient {
            self.ipc_state = ConnectionState::WaitingClient;
        }
        state
    }

    /// 处理一条 IPC 消息：Hello 重置序列基线，Snapshot 进入 gate，Bye 标记断开。
    pub fn handle_ipc_message(
        &mut self,
        message: &Message,
        now_ms: i64,
    ) -> Result<DataState, AppError> {
        match message.header.kind {
            MessageKind::Hello => {
                self.gate.reset_for_new_client();
                self.snapshot = None;
                self.ipc_state = ConnectionState::Connected;
                self.state = AppState::Running;
                Ok(self.gate.state())
            }
            MessageKind::Snapshot => {
                let snapshot: Snapshot = message
                    .decode_payload()
                    .map_err(|e| AppError::Data(e.to_string()))?;
                self.receive_snapshot(&snapshot, now_ms)
            }
            MessageKind::Bye => {
                self.handle_client_disconnected();
                Ok(self.gate.state())
            }
            other => Err(AppError::Data(format!("不支持的消息类型：{other:?}"))),
        }
    }

    /// 管道断开/客户端退出：清掉旧快照，绝不把旧值继续当实时数据。
    pub fn handle_client_disconnected(&mut self) {
        self.gate.mark_client_disconnected();
        self.snapshot = None;
        self.ipc_state = ConnectionState::WaitingClient;
    }

    /// 无设备时的渲染自检：使用指定 Profile 生成布局并渲染空卡片（不含任何指标数据）。
    pub fn render_probe(&self, profile: &ScreenProfile) -> Result<FrameBuffer, AppError> {
        let template = match screen_layout::TemplateKind::from_override(
            &self.config.display.layout_template,
        ) {
            Some(t) => t,
            None => TemplateKind::for_profile(profile),
        };
        let layout = build(
            profile,
            &self.config.components.visible,
            template,
            self.font_scale(),
        )
        .map_err(|e| AppError::State(e.to_string()))?;
        render_with_bg(
            profile,
            &self.current_theme,
            &layout,
            None,
            self.background_image.as_ref(),
        )
        .map_err(|e| AppError::State(e.to_string()))
    }

    /// 解析当前配置的方向（orientation + flip），以物理 Profile 的原生方向为 auto 默认。
    pub fn direction(&self, physical: &ScreenProfile) -> Result<Direction, AppError> {
        Direction::parse(
            &self.config.display.orientation,
            &self.config.display.flip,
            physical.orientation,
        )
        .map_err(AppError::from)
    }

    /// 按当前方向渲染逻辑画布，再旋转到物理 Profile 尺寸。
    pub fn oriented_frame(&self, physical: &ScreenProfile) -> Result<FrameBuffer, AppError> {
        let direction = self.direction(physical)?;
        let plan = direction.plan(physical, Some(&self.config.display.layout_template))?;
        let layout = build(
            &plan.logical,
            &self.config.components.visible,
            plan.template,
            self.font_scale(),
        )
        .map_err(|e| AppError::State(e.to_string()))?;
        let logical = render_with_bg(
            &plan.logical,
            &self.current_theme,
            &layout,
            None,
            self.background_image.as_ref(),
        )
        .map_err(|e| AppError::State(e.to_string()))?;
        direction
            .oriented_frame(physical, &logical)
            .map_err(AppError::from)
    }

    /// 生成完整物理帧（RGB565LE，不含 6 字节头；头由传输/适配器生成）。
    pub fn refresh_frame(&self, physical: &ScreenProfile) -> Result<Frame, AppError> {
        let logical = self.oriented_frame(physical)?;
        let pixels = rgba_to_rgb565le(logical.pixels(), logical.width(), logical.height())
            .map_err(|e| AppError::State(e.to_string()))?;
        Frame::new(
            logical.width(),
            logical.height(),
            PixelFormat::Rgb565Le,
            pixels,
            Some(8192),
        )
        .map_err(|e| AppError::State(e.to_string()))
    }

    /// 用已接受的快照渲染真实指标，再旋转/RGB565 为完整物理帧。
    /// 缺失指标不画卡；`usage_pct=None` 显示 `--`（由 `screen-render::metrics` 保证）。
    pub fn render_with_data(&self, physical: &ScreenProfile) -> Result<Frame, AppError> {
        let snapshot = self
            .snapshot
            .as_ref()
            .ok_or_else(|| AppError::Data("无客户端快照，不能渲染指标".into()))?;
        let direction = self.direction(physical)?;
        let plan = direction.plan(physical, Some(&self.config.display.layout_template))?;

        // 旧版四主题：走专用版式（顶栏 + 圆环 / 双列），与旧 screen.ps1 对齐
        if self.current_theme.render.is_some() {
            let data = legacy_data(snapshot);
            let visible = &self.config.components.visible;
            let set = create_text_renderer(&FontSpec {
                family: self.current_theme.typography.family.clone(),
                size_px: 16.0,
                weight: theme_weight(&self.current_theme),
            });
            let logical = match set {
                Ok(mut renderer) => render_legacy(
                    &plan.logical,
                    &self.current_theme,
                    &data,
                    visible,
                    Some(renderer.as_mut()),
                ),
                Err(_) => render_legacy(&plan.logical, &self.current_theme, &data, visible, None),
            }
            .map_err(|e| AppError::State(e.to_string()))?;
            return finish_frame(direction, physical, &logical);
        }

        let bound = bind_with_clock(
            &snapshot.metrics,
            &self.config.components.visible,
            local_clock(),
        );
        let tree = if bound.is_empty() {
            LayoutTree {
                profile_id: plan.logical.id.clone(),
                template: plan.template.name().to_string(),
                font_px: 14.0,
                gap: 8.0,
                items: Vec::new(),
            }
        } else {
            let kinds: Vec<_> = bound.iter().map(|(kind, _)| *kind).collect();
            build(&plan.logical, &kinds, plan.template, self.font_scale())
                .map_err(|e| AppError::State(e.to_string()))?
        };

        let logical = match create_text_renderer(&FontSpec {
            family: self.current_theme.typography.family.clone(),
            size_px: tree.font_px,
            weight: theme_weight(&self.current_theme),
        }) {
            Ok(mut renderer) => render_with_metrics_bg(
                &plan.logical,
                &self.current_theme,
                &tree,
                &bound,
                Some(renderer.as_mut()),
                self.background_image.as_ref(),
            ),
            Err(_) => render_with_metrics_bg(
                &plan.logical,
                &self.current_theme,
                &tree,
                &bound,
                None,
                self.background_image.as_ref(),
            ),
        }
        .map_err(|e| AppError::State(e.to_string()))?;
        finish_frame(direction, physical, &logical)
    }

    /// 生成占位物理帧（等待客户端/数据过期/版本不兼容），不显示任何旧数值。
    pub fn render_placeholder(
        &self,
        physical: &ScreenProfile,
        notice: &str,
    ) -> Result<Frame, AppError> {
        let direction = self.direction(physical)?;
        let plan = direction.plan(physical, Some(&self.config.display.layout_template))?;
        let logical = match create_text_renderer(&FontSpec {
            family: self.current_theme.typography.family.clone(),
            size_px: (plan.logical.short_edge as f32 * 0.08).max(14.0),
            weight: theme_weight(&self.current_theme),
        }) {
            Ok(mut renderer) => render_notice_bg(
                &plan.logical,
                &self.current_theme,
                notice,
                Some(renderer.as_mut()),
                self.background_image.as_ref(),
            ),
            Err(_) => render_notice_bg(
                &plan.logical,
                &self.current_theme,
                notice,
                None,
                self.background_image.as_ref(),
            ),
        }
        .map_err(|e| AppError::State(e.to_string()))?;
        let logical = direction
            .oriented_frame(physical, &logical)
            .map_err(AppError::from)?;
        let pixels = rgba_to_rgb565le(logical.pixels(), logical.width(), logical.height())
            .map_err(|e| AppError::State(e.to_string()))?;
        Frame::new(
            logical.width(),
            logical.height(),
            PixelFormat::Rgb565Le,
            pixels,
            Some(8192),
        )
        .map_err(|e| AppError::State(e.to_string()))
    }

    /// 切换屏幕方向并持久化；不改设备、不初始化、不发送命令。
    pub fn set_orientation(&mut self, orientation: &str) -> Result<(), AppError> {
        self.update_direction(orientation, &self.config.display.flip.clone())
    }

    /// 切换方向对调并持久化；180° 不交换宽高。
    pub fn set_flip(&mut self, flip: &str) -> Result<(), AppError> {
        self.update_direction(&self.config.display.orientation.clone(), flip)
    }

    /// 一次性切换 orientation+flip，并立即重新布局/渲染/旋转，返回完整物理帧。
    /// 只做帧生成，不打开设备、不初始化、不发送命令；发送/失败由上层 session 处理。
    pub fn switch_direction(
        &mut self,
        orientation: &str,
        flip: &str,
        physical: &ScreenProfile,
    ) -> Result<Frame, AppError> {
        self.update_direction(orientation, flip)?;
        self.refresh_frame(physical)
    }

    /// 当前 Session 状态；未接入设备时为 None。
    pub fn session_state(&self) -> Option<SessionState> {
        self.session.as_ref().map(|session| session.state())
    }

    /// 接入一个已建立或待建立的 Session；同一 ScreenApp 只允许一个。
    pub fn attach_session(
        &mut self,
        session: DeviceSession<dyn DeviceAdapter>,
    ) -> Result<(), AppError> {
        if self.session.is_some() {
            return Err(AppError::State("设备 Session 已存在，拒绝重复持有".into()));
        }
        self.session = Some(session);
        Ok(())
    }

    /// 生产路径：创建 3.5 CDC 适配器，打开并初始化一次。
    pub fn connect_device_session(&mut self, device: &DeviceInfo) -> Result<(), AppError> {
        if self.session.is_some() {
            return Err(AppError::State("设备 Session 已存在，拒绝重复打开".into()));
        }
        let adapter: Box<dyn DeviceAdapter> = Box::new(TurzxCdcAdapter::new(CdcTransport::new()));
        let session =
            DeviceSession::connect(adapter, device).map_err(|e| AppError::State(e.to_string()))?;
        self.session = Some(session);
        Ok(())
    }

    /// 断开后重连：关闭旧句柄、重新打开/初始化一次；失败不自动重试。
    pub fn reconnect_device_session(&mut self, device: &DeviceInfo) -> Result<(), AppError> {
        if let Some(session) = self.session.as_mut() {
            session
                .reconnect(device)
                .map_err(|e| AppError::State(e.to_string()))
        } else {
            self.connect_device_session(device)
        }
    }

    pub fn close_device_session(&mut self) {
        if let Some(mut session) = self.session.take() {
            session.close();
        }
    }

    /// 重新渲染、旋转、RGB565 转换后发送完整物理帧；不初始化。
    pub fn refresh_device(&mut self, physical: &ScreenProfile) -> Result<(), AppError> {
        if self.session.is_none() {
            return Err(AppError::State("未连接设备 Session".into()));
        }
        let frame = self.refresh_frame(physical)?;
        self.session
            .as_mut()
            .ok_or_else(|| AppError::State("未连接设备 Session".into()))?
            .send_frame(&frame)
            .map_err(|e| AppError::State(e.to_string()))
    }

    /// 方向切换 + 完整帧刷新；切换不重新初始化设备。
    pub fn switch_direction_and_refresh(
        &mut self,
        orientation: &str,
        flip: &str,
        physical: &ScreenProfile,
    ) -> Result<(), AppError> {
        if self.session.is_none() {
            return Err(AppError::State("未连接设备 Session".into()));
        }
        self.update_direction(orientation, flip)?;
        self.refresh_device(physical)
    }

    fn update_direction(&mut self, orientation: &str, flip: &str) -> Result<(), AppError> {
        // 3.5 阶段以 Portrait 为设备原生方向；其他设备仍由 Direction 纯函数显式传入。
        Direction::parse(orientation, flip, Orientation::Portrait)?;
        let mut next = self.config.clone();
        next.display.orientation = orientation.to_string();
        next.display.flip = flip.to_string();
        next.validate()?;
        if let Some(path) = &self.config_path {
            next.save(path)?;
        }
        self.config = next;
        Ok(())
    }

    pub fn status_lines(&self) -> Vec<String> {
        let config_line = match (&self.config_path, self.config_used_defaults) {
            (Some(path), false) => format!("config: {}", path.display()),
            (Some(path), true) => format!("config: {} (缺失，使用默认值)", path.display()),
            (None, _) => "config: 内置默认值（未找到 %APPDATA%)".to_string(),
        };
        vec![
            format!("hetao-screen {}", env!("CARGO_PKG_VERSION")),
            config_line,
            format!(
                "theme: {} ({})",
                self.current_theme.id, self.current_theme.name
            ),
            "platform: Windows 10/11 only; device: single 3.5 CDC (no COM fallback)".to_string(),
            format!(
                "data: {:?} (不连接 NAS；等待主客户端快照)",
                self.gate.state()
            ),
            format!("device state: {:?}", self.state),
        ]
    }

    /// 生产常驻主循环（无限运行）；`--run-seconds` 使用 `run_with_options`。
    pub fn run(&mut self) -> Result<(), AppError> {
        self.run_with_options(&RunOptions::default())
    }

    pub fn run_with_options(&mut self, options: &RunOptions) -> Result<(), AppError> {
        for line in self.status_lines() {
            println!("{line}");
        }
        let result = crate::runtime::run_production(self, options.clone());
        self.shutdown();
        result.map(|_| ())
    }

    pub fn shutdown(&mut self) {
        self.close_device_session();
        self.devices.clear();
        self.ipc_state = ConnectionState::Disconnected;
        self.state = AppState::Shutdown;
    }
}

/// 逻辑帧 → 按方向旋转 → RGB565LE 物理帧。
fn finish_frame(
    direction: Direction,
    physical: &ScreenProfile,
    logical: &FrameBuffer,
) -> Result<Frame, AppError> {
    let logical = direction
        .oriented_frame(physical, logical)
        .map_err(AppError::from)?;
    let pixels = rgba_to_rgb565le(logical.pixels(), logical.width(), logical.height())
        .map_err(|e| AppError::State(e.to_string()))?;
    Frame::new(
        logical.width(),
        logical.height(),
        PixelFormat::Rgb565Le,
        pixels,
        Some(8192),
    )
    .map_err(|e| AppError::State(e.to_string()))
}

/// 从快照提取旧版版式所需的数据（缺失一律留 None）。
fn legacy_data(snapshot: &Snapshot) -> LegacyData {
    let m = &snapshot.metrics;
    LegacyData {
        host: snapshot.nas.name.clone(),
        addr: snapshot.nas.addr.clone(),
        online: snapshot.nas.online.unwrap_or(false),
        cpu: m.cpu.as_ref().and_then(|c| c.usage_pct),
        cpu_temp: m.cpu.as_ref().and_then(|c| c.temp_c),
        mem: m.memory.as_ref().and_then(|v| v.usage_pct),
        mem_used: m.memory.as_ref().and_then(|v| v.used_bytes),
        mem_total: m.memory.as_ref().and_then(|v| v.total_bytes),
        gpu: m.gpu.as_ref().and_then(|v| v.usage_pct),
        gpu_temp: m.gpu.as_ref().and_then(|v| v.temp_c),
        up: m.network.as_ref().and_then(|v| v.up_bps),
        down: m.network.as_ref().and_then(|v| v.down_bps),
        read: m.disk.as_ref().and_then(|v| v.read_bps),
        write: m.disk.as_ref().and_then(|v| v.write_bps),
        pc_cpu: snapshot.pc.cpu_pct,
        pc_mem: snapshot.pc.mem_pct,
        pc_gpu: snapshot.pc.gpu_pct,
        pc_gpu_temp: snapshot.pc.gpu_temp,
    }
}

/// 主题字号 = 主题 base_px/14 × 主题 scale × 用户 font_scale，限幅到布局允许区间。
fn effective_font_scale(theme: &ThemeConfig, user_scale: f32) -> f32 {
    let ratio = theme.typography.base_px / 14.0;
    let combined = ratio * theme.typography.scale * user_scale;
    if combined.is_finite() {
        combined.clamp(0.5, 3.0)
    } else {
        1.0
    }
}

/// 主题字重：优先 `value`，其次 `regular`，最后取第一个声明的字重；默认 400。
fn theme_weight(theme: &ThemeConfig) -> u16 {
    theme
        .typography
        .weights
        .get("value")
        .or_else(|| theme.typography.weights.get("regular"))
        .or_else(|| theme.typography.weights.values().next())
        .copied()
        .unwrap_or(400)
        .clamp(100, 900)
}

/// 加载主题背景图。内置主题不携带图片资源；用户主题从 `<config 目录>/themes/` 读取 PNG。
fn load_background_image(
    theme: &ThemeConfig,
    config_path: Option<&Path>,
    is_builtin: bool,
) -> Option<BackgroundImage> {
    let rel = theme.background.image.as_deref()?;
    if is_builtin {
        eprintln!("内置主题不支持图片背景，已忽略：{rel}");
        return None;
    }
    let Some(path) = config_path
        .and_then(|p| p.parent())
        .map(|dir| dir.join("themes").join(rel))
    else {
        return None;
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("背景图读取失败 {}：{err}", path.display());
            return None;
        }
    };
    match BackgroundImage::from_png(&bytes) {
        Ok(image) => Some(image),
        Err(err) => {
            eprintln!("背景图已跳过 {}：{err}", path.display());
            None
        }
    }
}

pub fn default_config_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|base| {
        PathBuf::from(base)
            .join("hetao-capsule")
            .join("screen")
            .join("config.toml")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::test_support::{MockAdapter, MockCounters};
    use screen_core::{CpuMetrics, Orientation, ScreenProfile, Snapshot};
    use screen_protocol::frame_header;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    fn missing_config() -> PathBuf {
        let path = std::env::temp_dir().join("hetao-screen-test-missing.toml");
        let _ = std::fs::remove_file(&path);
        path
    }

    fn temp_config(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("hetao-screen-{name}-{}.toml", std::process::id()))
    }

    fn physical_profile() -> ScreenProfile {
        ScreenProfile::new("3.5-physical", 320, 480, Orientation::Portrait, true).expect("profile")
    }

    const CUSTOM_THEME: &str = r##"{
        "schema":"hetao.theme/1","id":"custom","name":"用户主题","version":"1.0.0",
        "palette":{"bg":"#000000","card":"#111111","text":"#FFFFFF","dim":"#888888","accent":"#00FF00","warn":"#FFB020","danger":"#FF5A5F"}
    }"##;

    fn theme_test_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hetao-screen-theme-{}-{}-{tag}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(dir.join("themes")).expect("mkdir themes");
        dir
    }

    #[test]
    fn builtin_legacy_themes_are_registered() {
        let app = ScreenApp::init(Some(&temp_config("builtin-themes"))).expect("app");
        for id in ["default", "dark", "light", "bignum-dark", "bignum-light"] {
            assert!(
                app.theme_store().ids().any(|v| v == id),
                "缺少内置主题 {id}"
            );
        }
        assert_eq!(app.theme().id, "default");
    }

    #[test]
    fn legacy_theme_can_be_selected_and_scales_font() {
        let path = temp_config("legacy-theme");
        std::fs::write(&path, "version = 1\n[display]\ntheme = \"bignum-light\"\n")
            .expect("write config");
        let app = ScreenApp::init(Some(&path)).expect("app");
        assert_eq!(app.theme().id, "bignum-light");
        assert_eq!(
            app.theme().palette.bg,
            screen_core::Color::rgb(255, 255, 255)
        );
        // 18/14 × 1.1 ≈ 1.414
        assert!(
            (app.font_scale() - 1.414).abs() < 0.02,
            "{}",
            app.font_scale()
        );
    }

    #[test]
    fn user_theme_directory_is_loaded_and_selected() {
        let dir = theme_test_dir("pick");
        let config_path = dir.join("config.toml");
        std::fs::write(&config_path, "version = 1\n[display]\ntheme = \"custom\"\n")
            .expect("write config");
        std::fs::write(dir.join("themes").join("custom.json"), CUSTOM_THEME).expect("write theme");

        let app = ScreenApp::init(Some(&config_path)).expect("app");
        assert_eq!(app.theme().id, "custom");
        assert!(app.theme_store().ids().any(|id| id == "custom"));
        assert!(app.theme_store().is_builtin("default"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_theme_falls_back_to_default() {
        let dir = theme_test_dir("missing");
        let config_path = dir.join("config.toml");
        std::fs::write(
            &config_path,
            "version = 1\n[display]\ntheme = \"not-exist\"\n",
        )
        .expect("write config");

        let app = ScreenApp::init(Some(&config_path)).expect("app");
        assert_eq!(app.theme().id, "default", "未知主题回退默认而不是报错");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn init_without_config_uses_defaults() {
        let path = missing_config();
        let app = ScreenApp::init(Some(&path)).expect("app");
        assert_eq!(app.state(), AppState::WaitingDevice);
        assert_eq!(app.devices().active_count(), 0);
        assert_eq!(app.layout(), None);
        assert!(app.theme().id == "default");
    }

    #[test]
    fn snapshot_updates_state_and_turns_stale() {
        let path = missing_config();
        let mut app = ScreenApp::init(Some(&path)).expect("app");
        let snap = Snapshot::new(1, 10_000);
        assert_eq!(app.receive_snapshot(&snap, 10_000), Ok(DataState::Ready));
        assert_eq!(app.ipc_state(), ConnectionState::Connected);
        assert_eq!(app.tick(20_000), DataState::Stale);
    }

    #[test]
    fn render_probe_matches_profile() {
        let path = missing_config();
        let app = ScreenApp::init(Some(&path)).expect("app");
        let profile =
            ScreenProfile::new("3.5-l", 480, 320, Orientation::Landscape, true).expect("profile");
        let frame = app.render_probe(&profile).expect("render");
        assert_eq!((frame.width(), frame.height()), (480, 320));
        assert_eq!(frame.pixels().len(), 480 * 320 * 4);
    }

    fn pixel_565(frame: &Frame, x: u32, y: u32) -> u16 {
        let idx = ((y * frame.width + x) * 2) as usize;
        u16::from_le_bytes([frame.pixels[idx], frame.pixels[idx + 1]])
    }

    fn bg_565() -> u16 {
        // #101418 → RGB565 0x10A3
        0x10A3
    }

    fn card_565() -> u16 {
        // #1B2028 → RGB565 0x1905
        0x1905
    }

    #[test]
    fn accepted_snapshot_is_retained() {
        let path = missing_config();
        let mut app = ScreenApp::init(Some(&path)).expect("app");
        let snap = Snapshot::new(1, 10_000);
        app.receive_snapshot(&snap, 10_000).expect("accept");
        assert_eq!(app.snapshot(), Some(&snap));
    }

    #[test]
    fn ipc_snapshot_message_marks_ready_and_stores_snapshot() {
        let path = missing_config();
        let mut app = ScreenApp::init(Some(&path)).expect("app");
        let snap = Snapshot::new(3, 10_000);
        let message = Message::with_payload(MessageKind::Snapshot, 3, &snap).expect("ipc message");
        assert_eq!(
            app.handle_ipc_message(&message, 10_000),
            Ok(DataState::Ready)
        );
        assert_eq!(app.snapshot(), Some(&snap));
        assert_eq!(app.ipc_state(), ConnectionState::Connected);
    }

    #[test]
    fn duplicate_snapshot_message_is_rejected() {
        let path = missing_config();
        let mut app = ScreenApp::init(Some(&path)).expect("app");
        let snap = Snapshot::new(5, 10_000);
        let message = Message::with_payload(MessageKind::Snapshot, 5, &snap).expect("ipc message");
        app.handle_ipc_message(&message, 10_000).expect("first");
        assert!(app.handle_ipc_message(&message, 10_100).is_err());
    }

    #[test]
    fn hello_resets_sequence_baseline() {
        let path = missing_config();
        let mut app = ScreenApp::init(Some(&path)).expect("app");
        app.receive_snapshot(&Snapshot::new(9, 10_000), 10_000)
            .expect("old client");
        let hello = Message::new(MessageKind::Hello, 0, Vec::new()).expect("hello");
        app.handle_ipc_message(&hello, 10_000).expect("hello");
        assert!(app.snapshot().is_none());

        let snap = Snapshot::new(1, 10_000);
        let message = Message::with_payload(MessageKind::Snapshot, 1, &snap).expect("snapshot");
        assert_eq!(
            app.handle_ipc_message(&message, 10_000),
            Ok(DataState::Ready)
        );
    }

    #[test]
    fn bye_clears_snapshot_and_returns_to_waiting() {
        let path = missing_config();
        let mut app = ScreenApp::init(Some(&path)).expect("app");
        app.receive_snapshot(&Snapshot::new(1, 10_000), 10_000)
            .expect("accept");
        assert!(app.snapshot().is_some());

        let bye = Message::new(MessageKind::Bye, 1, Vec::new()).expect("bye");
        app.handle_ipc_message(&bye, 10_000).expect("bye");
        assert!(app.snapshot().is_none());
        assert_eq!(app.ipc_state(), ConnectionState::WaitingClient);
        assert!(app.render_with_data(&physical_profile()).is_err());
    }

    #[test]
    fn render_with_data_without_snapshot_is_error() {
        let path = missing_config();
        let app = ScreenApp::init(Some(&path)).expect("app");
        assert!(app.render_with_data(&physical_profile()).is_err());
    }

    #[test]
    fn placeholder_frame_is_320x480_rgb565() {
        let path = missing_config();
        let app = ScreenApp::init(Some(&path)).expect("app");
        let frame = app
            .render_placeholder(&physical_profile(), "等待客户端")
            .expect("placeholder");
        assert_eq!((frame.width, frame.height), (320, 480));
        assert_eq!(frame.pixels.len(), 307_200);
        assert_eq!(frame.pixel_format, PixelFormat::Rgb565Le);
    }

    #[cfg(windows)]
    #[test]
    fn placeholder_draws_notice_text() {
        let path = missing_config();
        let app = ScreenApp::init(Some(&path)).expect("app");
        let frame = app
            .render_placeholder(&physical_profile(), "等待客户端")
            .expect("placeholder");
        let non_bg = frame
            .pixels
            .as_chunks::<2>()
            .0
            .iter()
            .filter(|px| u16::from_le_bytes(**px) != bg_565())
            .count();
        assert!(non_bg > 0, "占位画面应绘制提示文字");
    }

    #[test]
    fn empty_metrics_render_only_background() {
        let path = missing_config();
        let mut app = ScreenApp::init(Some(&path)).expect("app");
        app.receive_snapshot(&Snapshot::new(1, 10_000), 10_000)
            .expect("accept");
        let frame = app
            .render_with_data(&physical_profile())
            .expect("render with data");
        assert_eq!((frame.width, frame.height), (320, 480));
        assert_eq!(frame.pixels.len(), 307_200);
        assert!(
            frame
                .pixels
                .as_chunks::<2>()
                .0
                .iter()
                .all(|px| u16::from_le_bytes(*px) == bg_565()),
            "空指标不得画出任何卡片"
        );
    }

    #[test]
    fn cpu_only_metrics_leave_second_cell_empty() {
        let path = missing_config();
        let mut app = ScreenApp::init(Some(&path)).expect("app");
        let mut snap = Snapshot::new(1, 10_000);
        snap.metrics.cpu = Some(CpuMetrics {
            usage_pct: Some(42.0),
            temp_c: Some(45.0),
            ..CpuMetrics::default()
        });
        app.receive_snapshot(&snap, 10_000).expect("accept");
        let frame = app
            .render_with_data(&physical_profile())
            .expect("render with data");

        assert_eq!(pixel_565(&frame, 20, 300), card_565());
        assert_eq!(
            pixel_565(&frame, 200, 300),
            bg_565(),
            "只开 CPU 时右侧不应出现空白卡片"
        );
    }

    #[test]
    fn refresh_frame_four_modes_are_320x480_with_e1a_header() {
        let path = temp_config("four-modes");
        let _ = std::fs::remove_file(&path);
        let mut app = ScreenApp::init(Some(&path)).expect("app");
        let physical = physical_profile();
        let header = frame_header(320, 480).expect("header");
        assert_eq!(header, [0, 0, 0x04, 0xFD, 0xDF, 0xC5]);

        for (orientation, flip) in [
            ("portrait", "normal"),
            ("portrait", "flipped"),
            ("landscape", "normal"),
            ("landscape", "flipped"),
        ] {
            let frame = app
                .switch_direction(orientation, flip, &physical)
                .expect("switch+refresh");
            assert_eq!((frame.width, frame.height), (320, 480));
            assert_eq!(frame.pixels.len(), 307_200);
            assert_eq!(frame.pixel_format, PixelFormat::Rgb565Le);
            assert_eq!(frame.chunk_hint, Some(8192));
            assert_eq!(
                frame_header(frame.width, frame.height).expect("header"),
                header
            );
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn direction_settings_persist_and_reload() {
        let path = temp_config("direction-persist");
        let _ = std::fs::remove_file(&path);
        let mut app = ScreenApp::init(Some(&path)).expect("app");
        app.set_orientation("landscape").expect("orientation");
        app.set_flip("flipped").expect("flip");

        let saved = Config::load(&path).expect("load saved");
        assert_eq!(saved.display.orientation, "landscape");
        assert_eq!(saved.display.flip, "flipped");

        let app2 = ScreenApp::init(Some(&path)).expect("reload app");
        assert_eq!(app2.config().display.orientation, "landscape");
        assert_eq!(app2.config().display.flip, "flipped");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn invalid_direction_is_rejected_without_change() {
        let path = temp_config("invalid-direction");
        let _ = std::fs::remove_file(&path);
        let mut app = ScreenApp::init(Some(&path)).expect("app");
        let before = app.config().clone();

        assert!(app.set_flip("mirror").is_err());
        assert!(app.set_orientation("sideways").is_err());
        assert_eq!(app.config(), &before);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn refresh_failure_is_not_reported_as_success() {
        let path = temp_config("refresh-failure");
        let _ = std::fs::remove_file(&path);
        let mut app = ScreenApp::init(Some(&path)).expect("app");
        app.config.display.flip = "mirror".to_string();
        let physical = physical_profile();
        assert!(app.refresh_frame(&physical).is_err());
        assert!(app.oriented_frame(&physical).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn simulated_session_reinitializes_only_once_across_direction_switches() {
        struct TestSession {
            init_calls: u32,
            frames: u32,
            initialized: bool,
        }
        impl TestSession {
            fn refresh(
                &mut self,
                app: &ScreenApp,
                physical: &ScreenProfile,
            ) -> Result<(), AppError> {
                if !self.initialized {
                    self.initialized = true;
                    self.init_calls += 1;
                }
                app.refresh_frame(physical)?;
                self.frames += 1;
                Ok(())
            }
        }

        let path = temp_config("session-init-once");
        let _ = std::fs::remove_file(&path);
        let mut app = ScreenApp::init(Some(&path)).expect("app");
        let physical = physical_profile();
        let mut session = TestSession {
            init_calls: 0,
            frames: 0,
            initialized: false,
        };
        for (orientation, flip) in [
            ("portrait", "normal"),
            ("portrait", "flipped"),
            ("landscape", "normal"),
            ("landscape", "flipped"),
        ] {
            app.set_orientation(orientation).expect("orientation");
            app.set_flip(flip).expect("flip");
            session.refresh(&app, &physical).expect("refresh");
        }
        assert_eq!(session.init_calls, 1);
        assert_eq!(session.frames, 4);
        assert_eq!(app.devices().active_count(), 0);
        let _ = std::fs::remove_file(&path);
    }

    fn mock_session() -> (DeviceSession<dyn DeviceAdapter>, Arc<MockCounters>) {
        let mock = MockAdapter::new();
        let counts = mock.counters.clone();
        let adapter: Box<dyn DeviceAdapter> = Box::new(mock);
        let info = DeviceInfo {
            key: "mock".into(),
            model_id: "mock".into(),
            stable_id: Some("mock".into()),
            transport: screen_core::TransportKind::Cdc,
            open_path: Some("mock-path".into()),
        };
        let session = DeviceSession::connect(adapter, &info).expect("mock session");
        (session, counts)
    }

    #[test]
    fn device_session_refreshes_four_modes_without_reinitializing() {
        let path = temp_config("device-session-modes");
        let _ = std::fs::remove_file(&path);
        let mut app = ScreenApp::init(Some(&path)).expect("app");
        let (session, counts) = mock_session();
        app.attach_session(session).expect("attach");
        let physical = physical_profile();

        for (orientation, flip) in [
            ("portrait", "normal"),
            ("portrait", "flipped"),
            ("landscape", "normal"),
            ("landscape", "flipped"),
        ] {
            app.switch_direction_and_refresh(orientation, flip, &physical)
                .expect("switch+refresh");
        }

        assert_eq!(counts.open_calls.load(Ordering::SeqCst), 1);
        assert_eq!(counts.init_calls.load(Ordering::SeqCst), 1);
        assert_eq!(counts.send_calls.load(Ordering::SeqCst), 4);
        let frames = counts.frames.lock().expect("frames");
        assert_eq!(frames.len(), 4);
        for frame in frames.iter() {
            assert_eq!((frame.width, frame.height), (320, 480));
            assert_eq!(frame.pixels.len(), 307_200);
            assert_eq!(frame.pixel_format, PixelFormat::Rgb565Le);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn refresh_without_session_is_an_error_not_success() {
        let path = temp_config("no-session");
        let _ = std::fs::remove_file(&path);
        let mut app = ScreenApp::init(Some(&path)).expect("app");
        let physical = physical_profile();
        assert!(app.refresh_device(&physical).is_err());
        assert!(app
            .switch_direction_and_refresh("landscape", "normal", &physical)
            .is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn closed_session_cannot_send() {
        let path = temp_config("closed-session");
        let _ = std::fs::remove_file(&path);
        let mut app = ScreenApp::init(Some(&path)).expect("app");
        let (session, counts) = mock_session();
        app.attach_session(session).expect("attach");
        app.close_device_session();
        assert!(app.session_state().is_none());
        let physical = physical_profile();
        assert!(app.refresh_device(&physical).is_err());
        assert_eq!(counts.send_calls.load(Ordering::SeqCst), 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn send_failure_marks_faulted_and_reconnect_restores() {
        let path = temp_config("send-failure-reconnect");
        let _ = std::fs::remove_file(&path);
        let mut app = ScreenApp::init(Some(&path)).expect("app");
        let mock = MockAdapter::new();
        let counts = mock.counters.clone();
        let fail_send = mock.fail_send.clone();
        fail_send.store(true, Ordering::SeqCst);
        let adapter: Box<dyn DeviceAdapter> = Box::new(mock);
        let info = DeviceInfo {
            key: "mock".into(),
            model_id: "mock".into(),
            stable_id: Some("mock".into()),
            transport: screen_core::TransportKind::Cdc,
            open_path: Some("mock-path".into()),
        };
        let session = DeviceSession::connect(adapter, &info).expect("session");
        app.attach_session(session).expect("attach");
        let physical = physical_profile();

        assert!(app
            .switch_direction_and_refresh("portrait", "normal", &physical)
            .is_err());
        assert_eq!(app.session_state(), Some(SessionState::Faulted));

        fail_send.store(false, Ordering::SeqCst);
        app.reconnect_device_session(&info).expect("reconnect");
        assert_eq!(app.session_state(), Some(SessionState::Ready));
        app.switch_direction_and_refresh("landscape", "normal", &physical)
            .expect("refresh after reconnect");
        assert_eq!(counts.open_calls.load(Ordering::SeqCst), 2);
        assert_eq!(counts.init_calls.load(Ordering::SeqCst), 2);
        assert_eq!(counts.send_calls.load(Ordering::SeqCst), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn second_session_is_rejected() {
        let path = temp_config("second-session");
        let _ = std::fs::remove_file(&path);
        let mut app = ScreenApp::init(Some(&path)).expect("app");
        let (first, _) = mock_session();
        app.attach_session(first).expect("first");
        let (second, _) = mock_session();
        assert!(app.attach_session(second).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn shutdown_closes_device_session() {
        let path = temp_config("shutdown-session");
        let _ = std::fs::remove_file(&path);
        let mut app = ScreenApp::init(Some(&path)).expect("app");
        let (session, counts) = mock_session();
        app.attach_session(session).expect("attach");
        app.shutdown();
        assert!(app.session_state().is_none());
        assert!(counts.close_calls.load(Ordering::SeqCst) >= 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn shutdown_releases_state() {
        let path = missing_config();
        let mut app = ScreenApp::init(Some(&path)).expect("app");
        app.shutdown();
        assert_eq!(app.state(), AppState::Shutdown);
        assert_eq!(app.ipc_state(), ConnectionState::Disconnected);
        assert_eq!(app.devices().active_count(), 0);
    }

    #[test]
    fn invalid_config_file_is_reported() {
        let path = std::env::temp_dir().join("hetao-screen-invalid-config.toml");
        std::fs::write(
            &path,
            "version = 99\n[integration]\nmode = \"standalone\"\n",
        )
        .expect("write");
        let err = match ScreenApp::init(Some(&path)) {
            Ok(_) => panic!("非法配置不应初始化成功"),
            Err(err) => err,
        };
        assert!(matches!(err, AppError::Config(_)));
        let _ = std::fs::remove_file(&path);
    }
}
