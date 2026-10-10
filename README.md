<div align="center">
  <h1>核桃胶囊 · Walnut Capsule</h1>
  <p>把 NAS 的运行状态，变成桌面右上角一颗胶囊。</p>
  <p>
    <img alt="license" src="https://img.shields.io/badge/license-MIT-blue">
    <img alt="platform" src="https://img.shields.io/badge/platform-Windows%20%7C%20fnOS-lightgrey">
    <img alt="version" src="https://img.shields.io/badge/version-0.1.13-green">
  </p>
</div>

---

## 这是什么

核桃胶囊是一款轻量 NAS 状态监控工具，由三部分组成：

| 组成 | 说明 |
|---|---|
| **桌面客户端** | Windows 悬浮胶囊，实时显示 NAS 的 CPU、内存、硬盘读写、网络速率、存储用量 |
| **NAS 采集服务** | Go 单二进制（≤15MB 内存），采集指标并经 WebSocket 推送，附带管理页 |
| **飞牛应用包** | 符合飞牛 fnOS 规范的 `.fpk`，可在应用中心安装/升级，非容器、非 root 运行 |

数据只在局域网内传输：不监听公网、不上传云端、不收集任何用户数据。

## 功能特性

**桌面客户端**

- 收起态：一颗 276×32 的悬浮胶囊，两行三列观察 CPU / 内存 / 硬盘读写 / 网络上下行
- 展开态：320 宽面板 —— CPU 双环（使用率 + 温度）、内存单环、存储用量横条、硬盘读写心电图、网卡心电图
- 固定到任务栏：三个迷你圆环常驻任务栏空段，展开面板水平居中弹出，可拖动换位
- 亚克力磨砂背景 + 窗口圆角，动效曲线对齐 Fluent（展开 300ms / 收起 220ms）
- 系统托盘、开机自启（可选）、窗口与 exe 图标
- 断线自动重连，状态灯实时反映连接状况（绿=已连接 / 蓝=连接中 / 橙=重连 / 红=令牌错误）
- `hetao-capsule.exe --fake` 可本地模拟数据，无需 NAS 即可预览

**NAS 采集服务**

- CPU 使用率与温度、内存、GPU（如有）、物理盘速率/电源状态/温度、卷与底层物理盘、网卡速率
- WebSocket 实时推送 + HTTP `/metrics`，全部接口需配对令牌鉴权
- 内置管理页：复制配对令牌、选择监控网卡、查看运行日志、下载 Windows 客户端、查看应用自身 CPU/内存占用
- 管理页只挂在飞牛统一网关的 unix socket 上（复用平台登录态），不在局域网端口暴露
- **绝不主动唤醒休眠硬盘**：只用 `hdparm -C` 查电源状态，SMART 前加 `smartctl -n standby` 双保险，间隔 ≥30 分钟

## 下载

从 [Releases](../../releases) 获取：

| 文件 | 用途 |
|---|---|
| `hetaocapsule-0.1.13.fpk` | 飞牛 fnOS 应用包，在应用中心「手动安装」上传 |
| `hetao-capsule-setup-0.1.13.exe` | Windows 电脑客户端安装程序（免管理员，单文件，无运行库依赖） |

## 安装与使用

1. 在飞牛应用中心安装 `hetaocapsule-*.fpk`，桌面会出现「核桃胶囊」图标
2. 打开管理页，复制**配对令牌**
3. 运行 Windows 客户端安装程序，填入 NAS 的 IP / 端口（默认 `8765`）与配对令牌，点「测试连接」
4. 连接成功后胶囊浮在屏幕右上角；单击展开、按住拖动、右键（托盘）进入设置

> 系统要求：Windows 10/11 x64；飞牛 fnOS 1.1.3100 及以上。

## 性能红线

客户端为纯 CPU 软渲染，对资源占用有硬性约束，也是本项目的设计前提：

| 指标 | 目标 |
|---|---|
| 客户端内存 | ≤ 30MB |
| 客户端空闲 CPU | ≈ 0% |
| 客户端单帧耗时 | ≤ 6ms |
| NAS 端内存 | ≤ 15MB |

为守住这条线，客户端**不使用**任何 GUI 框架、GPU 加速或 WebView（无 eframe/egui、无 OpenGL/Vulkan/D3D、无 Tauri/Electron），渲染链路固定为：

```
Rust  →  tiny-skia（CPU 软渲染，RGBA）
      →  通道交换为 BGRA
      →  Win32 DIB 段
      →  StretchDIBits / UpdateLayeredWindow 上屏
```

配合「稳态数据帧不上屏」「面板合成 memcpy 快路径」「`timeBeginPeriod(1)` + UI 线程 ABOVE_NORMAL」等手段，实测空闲 CPU ≈ 0%、内存 15~22MB。

## 仓库结构

```
capsule/        Windows 客户端（Rust）
  src/ui/       全部界面渲染：药丸 / 三环 / 展开面板 / 动画
  installer/    Inno Setup 打包脚本
nas/            NAS 采集服务（Go）
  internal/metrics/   各类指标采集
  internal/server/    WebSocket、/metrics、管理页、令牌
hetaocapsule/   飞牛 fpk 应用包源码（manifest、cmd、wizard、config）
HANDOFF.md      交接文档：版本策略、铁律、形态规格、构建打包流程、已知陷阱
```

## 从源码构建

```powershell
# 客户端
cd capsule; cargo build --release
& "C:\Program Files (x86)\Inno Setup 6\ISCC.exe" installer\hetao-capsule.iss

# NAS 二进制（交叉编译到 Linux）
cd nas
$env:GOOS="linux"; $env:GOARCH="amd64"; $env:CGO_ENABLED="0"
go build -trimpath -ldflags "-s -w" -o ../hetaocapsule/app/hetao-nas .

# fpk 必须在 NAS 上用官方 fnpack 打包（详见 HANDOFF.md 第七节）
fnpack build -d /tmp/hetaocapsule
```

## 许可

本项目基于 [MIT License](LICENSE) 开源，Copyright (c) 2026 蓝胖纸 (selisun)。
