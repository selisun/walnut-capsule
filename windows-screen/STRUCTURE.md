# 核桃胶囊小屏系统 — 工程结构说明（第二阶段 CDC）

> 状态：第一阶段骨架 + 第二阶段 CDC 适配（0.1.0）。已实现 3.5 英寸真机 CDC 通信路径；**不含主题接入/IPC/安装器**。
> 对应设计文档：`../docs/windows-screen/01`、`03`、`06`、`07`、`08`、`09`、`10`、`12`、`13`、`14`、`15`、`16`。

## 1. 位置与形态

- 位置：仓库根目录 `windows-screen/`（与 `capsule/`、`nas/`、`tools/` 平级）。
- 形态：独立 Rust workspace，**不依赖 `capsule` crate**（capsule 是冻结的二进制工程，无法作为库依赖，也不应被改写）。
- 复用方式：只复用**已批准的依赖版本**（tiny-skia 0.11 / serde 1 / serde_json 1 / toml 0.8）与 `rust-lld` 链接配置；不复制、不 import 客户端源码。
- 冻结区隔离：新 workspace 自带 `Cargo.toml` / `Cargo.lock` / `target/`，不参与 `capsule` 的构建；`capsule/`、`nas/`、`tools/nas-screen/`、安装器在本次任务中零改动。

## 2. 目录树

```text
windows-screen/
├── Cargo.toml                 # workspace（members = crates/*）
├── Cargo.lock                 # 独立锁文件
├── .cargo/config.toml         # rust-lld + self-contained（与客户端一致）
├── .gitignore
├── STRUCTURE.md               # 本文件
├── assets/
│   └── themes/default/theme.json   # 内置默认主题（编译期嵌入）
└── crates/
    ├── screen-core/           # 领域模型（无 IO）
    │   └── src/{lib,error,capabilities,snapshot,device_state,theme_model,layout_model,frame}.rs
    ├── screen-ipc/            # IPC 协议结构（无 Windows 实现）
    │   └── src/{lib,error,header,message,transport}.rs
    ├── screen-data/           # 快照接收 + 新鲜度状态机
    │   └── src/{lib,error,state,gate}.rs
    ├── screen-device/         # 设备/传输/适配器抽象 + 单设备生命周期
    │   └── src/{lib,error,manager}.rs
    ├── screen-adapter-api/    # 传输/适配器 trait 与错误（第二阶段新增）
    │   └── src/{lib,error,types}.rs
    ├── screen-protocol/       # 帧头/RGB565/命令/分块纯函数（第二阶段新增）
    │   └── src/{lib,error,header,pixels,command,chunk}.rs
    ├── screen-transport-cdc/  # SetupAPI 枚举 + CreateFile/DCB/读写（第二阶段新增）
    │   └── src/{lib,port,enumerate}.rs
    ├── screen-adapters/
    │   └── turzx-cdc/         # 3.5 英寸型号适配（第二阶段新增）
    │       └── src/lib.rs
    ├── screen-layout/         # 布局模板与网格计算
    │   └── src/{lib,error,template,engine}.rs
    ├── screen-theme/          # JSON 主题加载/校验/注册
    │   └── src/{lib,error,loader,store}.rs
    ├── screen-render/         # tiny-skia 画布/变换/帧输出
    │   └── src/{lib,error,canvas,text,transform,pipeline}.rs
    └── screen-app/            # 独立进程入口（lib + bin）
        ├── src/{lib,main,app,config,cli}.rs
        └── tests/smoke.rs     # 跨 crate 冒烟测试
```

## 3. 模块职责

| crate | 职责 | 明确不做 |
|---|---|---|
| screen-core | 能力 `Cap<T>`、设备状态、快照模型、主题/布局数据、帧缓冲、错误 | 设备 IO、NAS、Token、渲染 |
| screen-ipc | 22 字节消息头、固定消息类型、长度/版本/序列号校验、连接与对端身份抽象 | Windows 管道实现、凭据传递 |
| screen-data | 接收快照、序列号单调、时间戳校验、过期阈值、`Ready/Stale/WaitingClient/VersionMismatch` | 连 NAS、凭据、采集 |
| screen-device | `DeviceManager` 单设备生命周期与状态机 | 适配器/传输抽象、真实 IO |
| screen-adapter-api | `Transport`/`DeviceAdapter` trait、`PortCandidate`/`DeviceInfo`、`AdapterError` | 设备 IO、协议字节 |
| screen-protocol | 6 字节候选帧头、RGB565LE、命令白名单（`0x6D`/`0x6E`）、8192 分块 | 任何 IO、型号判断 |
| screen-transport-cdc | SetupAPI 枚举 COMPORT、接口路径打开、DCB/超时、同步读写、错误映射 | 帧语义、重试策略 |
| screen-adapters/turzx-cdc | 3.5 基线能力、初始化顺序、帧发送编排 | 主题/布局/NAS |
| screen-layout | 模板选择、网格/缩放/字体尺寸、边界与重叠校验 | 判断型号、绝对像素覆盖 |
| screen-theme | JSON Schema v1 校验、颜色/范围/路径/资源类型、注册与切换 | 执行脚本、加载可执行文件 |
| screen-render | tiny-skia 画布、矩形绘制、旋转/镜像/反色、帧缓冲 | 设备通信、RGB565 协议、假数据 |
| screen-app | 配置加载/校验、模块装配、生命周期、CLI、退出释放 | 在主客户端进程内跑渲染循环 |

> `08` 已批准修正：`screen-adapter-api` 独立并依赖 `screen-core`。本阶段仍未创建 `screen-config`/`screen-diag`/`screen-render` 的字体与图片实现、`screen-ipc` 的 Windows 管道。

## 4. 依赖方向

```text
screen-app ─→ screen-core
     ├─→ screen-ipc      （无内部依赖）
     ├─→ screen-data ─→ screen-core
     ├─→ screen-device ─→ screen-adapter-api ─→ screen-core
     ├─→ screen-layout ─→ screen-core
     ├─→ screen-theme  ─→ screen-core
     ├─→ screen-render ─→ screen-core
     ├─→ screen-protocol（仅 std）
     ├─→ screen-transport-cdc ─→ screen-adapter-api + windows
     └─→ screen-adapters/turzx-cdc ─→ adapter-api/protocol/transport-cdc/core
```

- 不存在环：核心 crate 均为叶子或只指向 `screen-core`；`screen-app` 只作为终点。
- `screen-ipc` 不依赖 `screen-core`（payload 为字节 + serde 泛型工具），避免协议层被领域模型绑死。
- `screen-render` 的正式依赖只有 `screen-core` + `tiny-skia`；测试中才使用 `screen-layout` 构造布局树。

## 5. 核心接口

### 5.1 IPC（`screen-ipc`）

- `Header`：`magic "HTSP"` + major/minor + kind + flags + seq(u64) + payload_len(u32)，共 22 字节小端。
- `Message`：`to_bytes/from_bytes` 校验长度、上限 256KB；`MessageKind` 仅 6 种固定类型。
- `message::encode/decode_payload<T>` 基于 serde_json。
- `Version::is_compatible_with`：主版本相同才兼容。
- `PeerVerifier`/`IpcClient`/`IpcServer`/`PeerInfo`：仅抽象；Windows 管道与 PID/SID/映像校验留到第二阶段。

### 5.2 数据快照（`screen-core` + `screen-data`）

- `Snapshot { seq, ts_ms, nas, metrics }`；所有指标 `Option`，缺失不显示。
- `SnapshotGate::accept` 拒绝重复/倒退序列号、明显未来时间戳；`tick` 判定 `Ready/Stale/WaitingClient`；`mark_client_disconnected`、`reset_for_new_client`、`mark_version_mismatch`。
- 阈值：`max(3s, 3×推送周期, 配置值)`。

### 5.3 设备（`screen-core` + `screen-device`）

- `Cap<T>::Known/Unknown/Unsupported`；`DeviceCapabilities::frame_spec()` 任关键能力未知即返回 `MissingCapability`。
- `Transport`（枚举/打开/写/读/关闭）与 `DeviceAdapter`（open/init/send_frame/brightness/health）分离。
- `DeviceManager`：默认单设备；`attach/set_state/detach`；状态转移受 `core::can_transition` 约束。

### 5.4 布局（`screen-layout`）

- `TemplateKind`：`portrait-2col` / `landscape-4col` / `wide-6col` / `square-2x2` / `generic-4col`。
- `build(profile, visible, template, font_scale)`：按短边缩放、网格计算、最小可读尺寸检查，随后调用 `LayoutTree::validate`。
- 非 3.5 尺寸没有写死；测试覆盖 480×320、320×480、1920×462、480×480、100×80。

### 5.5 主题（`screen-theme`）

- `load_str`：2MB 上限、`deny_unknown_fields`、Schema 必须 `hetao.theme/1`。
- 校验：颜色格式（反序列化时）、字体范围、背景 dim、组件重复、样式引用、模板名、资源路径（拒绝绝对/`..`/盘符/反斜杠）、扩展名白名单、sha256 必填。
- `ThemeStore`：内置只读、切换、删除保护。

### 5.6 渲染（`screen-render`）

- `Canvas`：tiny-skia Pixmap，`clear/fill_rect/into_frame`。
- `TextRenderer` trait + `UnavailableTextRenderer`（明确返回未实现，不伪造文字）。
- `render(profile, theme, layout, text)`：背景 + 卡片 + 顶部强调条；不绘制任何指标值。
- `rotate90` / `mirror_horizontal` / `invert`：纯像素实现，测试覆盖。

## 6. 依赖清单（实际引入）

| 依赖 | 版本（锁定） | 使用 crate | 审批状态（`13`） |
|---|---|---|---|
| `serde` | 1.0.229 | core / ipc / data / theme / app | **已追认（2026-10-31）** |
| `serde_json` | 1.0.154 | ipc / theme / app | **已追认（2026-10-31）** |
| `toml` | 0.8.23 | app | **已追认（2026-10-31）** |
| `tiny-skia` | 0.11.4 | render | **已追认（2026-10-31）** |
| `windows` | 0.58.0（阶段 A 七项 feature） | transport-cdc | 版本有条件批准；A 已批准；B/C 暂缓 |

未引入（阶段 B/C）：`Win32_Graphics_Gdi`/`Win32_Graphics_Imaging`/`Win32_System_Com`（文本/图片），`Win32_System_Pipes`/`Win32_Security_Authorization`/`Win32_System_Threading`/`Win32_Security_Cryptography`/`Win32_Security_WinTrust`（IPC/安全）。`windows` 已按阶段 A 七项 feature 引入（`Security`/`System_IO` 仅编译门控）。另未引入 `serialport`、`image`、`zip`、`sha2`、`tungstenite`、`resvg`、tokio。

## 7. 构建与测试

```powershell
# 在本目录
cargo check --workspace --all-targets
cargo test --workspace
cargo run -q -p screen-app -- --version
cargo run -q -p screen-app -- --check-theme assets/themes/default/theme.json
cargo run -q -p screen-app -- --print-config
cargo run -q -p screen-app
# 真机（3.5，已授权范围）
cargo run -q -p screen-app -- --list-devices
cargo run -q -p screen-app -- --device-check
cargo run -q -p screen-app -- --send-frame bars --repeat 2
cargo run -q -p screen-app -- --device-session 30
```

> `.cargo/config.toml` 使用 `rust-lld`：本机无 MinGW gcc/MSVC link.exe；直接使用默认 gcc 链接器会因缺对象文件失败。

## 8. 当前边界（第二阶段 CDC）

- 已实现：3.5 英寸 COMPORT 枚举/实例 ID 匹配、打开、DCB 配置、初始化 `0x6D`/`0x6E`（0 值）、480×320 帧（RGB565LE、6 字节候选帧头、8192 分块）、关闭释放。
- 未实现：Windows 命名管道、对端身份校验、文本渲染、图片解码、配置迁移、CM 热插拔通知（恢复采用有界轮询）。
- 未验证：画面显示（需目视）、拔插恢复（需人工）、波特率回读异常、其他尺寸、官方协议参数。
- 不产生安装包/FPK/分发 EXE；`target/` 产物仅用于测试。
