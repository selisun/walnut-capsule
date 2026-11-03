//! 展开面板：小面板（320 宽）的构建、卡片组装与绘制，以及超大卡片共用的卡片/标题绘制。

use super::*;

impl CapsuleView {
    pub(super) fn render_expanded(
        &mut self,
        px: &mut Pixmap,
        frame: &Frame,
        font: Option<&Font>,
        p: f32,
        vis_w: u32,
    ) -> bool {
        let ph = panel_height(frame.has_data, frame.snap, frame.cards);
        // 固定到任务栏且任务栏在下方时，标题行放面板底部（贴任务栏一侧）
        let header_bottom = frame.pinned && frame.expand_up;
        // 数据版本/连接状态/面板高度/标题行位置/卡片开关/主题变化时，把整层构建派发到后台线程；
        // 动画帧键不变，不重复派发。面板光栅化（2~4ms）完全离开 UI 关键路径。
        // 卡片开关必须进键：开关变了但总高度恰好相同时（如关 CPU 同时开显卡）也要重建。
        // 主题也必须进键：暗色下文字取白色，切主题要重画文字。
        let key = (
            frame.seq,
            frame.conn.clone(),
            ph,
            header_bottom,
            frame.cards,
            frame.dark,
            false,
            PANEL_W,
        );
        let up_to_date = self.current.as_ref().is_some_and(|c| {
            c.seq == frame.seq
                && c.conn == *frame.conn
                && c.ph == ph
                && !c.big
                && c.w == PANEL_W
                && c.header_bottom == header_bottom
                && c.cards == frame.cards
                && c.dark == frame.dark
        });
        if !up_to_date && self.sent_key.as_ref() != Some(&key) {
            let _ = self.req_tx.send(PanelJob::Build(BuildReq {
                seq: frame.seq,
                conn: frame.conn.clone(),
                ph,
                big: false,
                w: PANEL_W,
                has_data: frame.has_data,
                dark: frame.dark,
                header_bottom,
                cards: frame.cards,
                snap: frame.arc.clone(),
            }));
            self.sent_key = Some(key);
            // 稳态数据帧：本帧不再重画旧面板（约 3ms 后 WM_APP_PANEL 会带着
            // 新面板强制上屏），省掉一次纯浪费的 1MB blit + ULW 提交。
            // 动画进行中不能跳，每一帧都要随进度上屏。
            if p >= 0.999 {
                return false;
            }
        }

        // 面板/顶栏合成走逐行 memcpy 快路径（见 blit_rows），不用 tiny-skia 混合管线

        // 取回新面板放在帧尾（见函数末）：避免在构建线程刚写完 1MB 像素、
        // 缓存尚热在其核心时本帧立即冷读（实测会造成 3~5ms blit 尖峰）。

        // 展开方向：固定到任务栏时窗口本身就是面板（底边贴任务栏，标题行贴任务栏一侧）；
        // 竖排时竖条与面板并排（面板顶边对齐竖条顶边，面板从竖条一侧推出）；
        // 其余未固定情况：顶栏贴窗口上沿、面板在其下方。
        let up = frame.expand_up;
        let pinned = frame.pinned;
        // 固定到任务栏时窗口本身就是面板，竖排设置不参与（收起态由任务栏三环承担）
        let vertical = frame.vertical && !frame.pinned;
        let vis_h = self.row_fade.len() as u32;
        let bar_y = 0.0f32;
        let y0 = if pinned || vertical {
            0i32
        } else {
            (CAPSULE_H + GAP) as i32
        };
        // 面板 blit 锚点：
        // - 固定态窗口已是全尺寸，面板只画“当前动画高度”那一段（向上展开时贴窗口底、
        //   源取面板最下面一段），其余留透明，靠内容生长成动画；
        // - 竖排窗口同样一次到位，面板靠“横向已推出宽度”裁切生长；
        // - 未固定横排：面板整幅贴在顶栏下方。
        let (panel_dx, panel_dy, panel_sx, panel_sy, panel_sw, panel_h) = if vertical {
            let grown = if p >= 0.999 {
                PANEL_W
            } else {
                (ease_motion(p) * PANEL_W as f32)
                    .round()
                    .clamp(0.0, PANEL_W as f32) as u32
            };
            if frame.panel_left {
                // 胶囊贴屏幕右缘：面板在竖条左侧，从贴竖条的一侧（面板右缘）向左推出
                let dx = PANEL_W - grown;
                (dx as i32, 0, dx, 0, grown, ph)
            } else {
                // 胶囊贴屏幕左缘：面板在竖条右侧，从面板左缘向右推出
                (PILL_V_W as i32, 0, 0, 0, grown, ph)
            }
        } else if pinned {
            let h = if p >= 0.999 {
                ph
            } else {
                let r = ease_motion(p);
                ((RINGS_H as f32) + (ph as f32 - RINGS_H as f32) * r)
                    .round()
                    .max(1.0) as u32
            };
            let h = h.min(ph);
            let (dy, sy) = if up {
                (vis_h as i32 - h as i32, ph - h)
            } else {
                (0, 0)
            };
            (0, dy, 0, sy, vis_w, h)
        } else {
            // 未固定横排：窗口向左加宽（右缘不动），面板内容右缘锚定、整体左移宽度差
            (
                vis_w as i32 - PANEL_W as i32,
                y0,
                PANEL_W - vis_w,
                0,
                vis_w,
                ph,
            )
        };

        // 面板区域随展开进度淡入。固定态/竖排面板与竖条并排，只对面板整块做淡入；
        // 未固定横排时顶栏保持不透明，只淡入其下方的面板区域。
        if p < 0.999 {
            // 内容在动画前段就基本不透明（约 45% 处到位），后段靠位移收尾，
            // 避免整段都在半透明里"发灰"。
            let a = ease_out_quad((p / 0.45).clamp(0.0, 1.0));
            let av = ((a * 255.0).round() as u8).max(if p > 0.001 { 1 } else { 0 });
            if pinned || up {
                for v in self.row_fade.iter_mut().take(ph as usize) {
                    *v = av;
                }
            } else if !vertical {
                for v in self.row_fade.iter_mut().skip(CAPSULE_H as usize) {
                    *v = av;
                }
            }
        }

        // 顶栏（横排，展开时与面板同宽）/ 竖条（竖排，贴在面板旁边）。
        // 圆角条按亮色调色板缓存为 320×52 像素层，动画中整层 blit
        // （超出可视宽的右段不会被 StretchDIBits 上屏）。
        // 固定在任务栏时顶栏不画底（透明融入任务栏）；竖排时窗口本身就是竖条，不画顶栏。
        let bar_w = vis_w;
        if vertical {
            let bx = if frame.panel_left {
                PANEL_W as f32
            } else {
                0.0
            };
            self.render_rings_v(px, frame, font, bx, bar_y);
        } else if !frame.pinned {
            if self.bar.is_none() {
                let mut bpm = Pixmap::new(PANEL_W, CAPSULE_H).unwrap();
                fill_round(
                    &mut bpm,
                    0.0,
                    0.0,
                    PANEL_W as f32,
                    CAPSULE_H as f32,
                    14.0,
                    BG,
                );
                self.bar = Some(bpm);
            }
            blit_rows(
                px,
                self.bar.as_ref().unwrap().as_ref(),
                0,
                bar_y as i32,
                0,
                vis_w,
                CAPSULE_H,
            );
        }

        if let Some(font) = font {
            // 固定到深色任务栏时顶栏文字改用浅色；暗色主题下次要文字也统一提白
            let light_on_dark = frame.pinned && !frame.taskbar_light;
            let (fg, dim) = if light_on_dark {
                (
                    color(PIN_FG.0, PIN_FG.1, PIN_FG.2, PIN_FG.3),
                    color(PIN_FG_DIM.0, PIN_FG_DIM.1, PIN_FG_DIM.2, PIN_FG_DIM.3),
                )
            } else {
                let d = text_c(frame.dark, FG_DIM);
                (color(FG.0, FG.1, FG.2, FG.3), color(d.0, d.1, d.2, d.3))
            };

            // 未固定横排：名称/IP 在顶栏（顶栏在面板区域之外，不会被面板 blit 覆盖）；
            // 固定到任务栏与竖排时改画在面板标题行（见帧尾），状态灯/状态/齿轮两者都在帧尾画
            if !frame.pinned && !vertical {
                let name = frame
                    .snap
                    .hostname
                    .as_deref()
                    .map(str::trim)
                    .filter(|h| !h.is_empty());
                let hbaseline =
                    bar_y + CAPSULE_H as f32 / 2.0 + font.ascent(FONT_SIZE_HEADER) * 0.36;
                let avail = (bar_w as f32 - PANEL_PAD * 2.0).max(20.0);
                draw_title(
                    font,
                    px,
                    14.0,
                    hbaseline,
                    avail,
                    (FONT_SIZE_HEADER, FONT_SIZE),
                    (fg, dim),
                    name,
                    frame.nas_host.trim(),
                );
            }
        }

        // 每帧只 blit 一次已就位的面板。展开首帧/数据刚到达的瞬间新面板可能尚未
        // 构建完，此时该区域逐行 alpha≈0（淡入刚开始），无上屏可见空窗。
        if let Some(c) = &self.current {
            blit_rect(
                px,
                c.pm.as_ref(),
                panel_dx,
                panel_dy,
                panel_sx,
                panel_sy,
                panel_sw,
                panel_h,
            );
        }

        // 帧尾才取回新面板（本帧主体 blit 的是上一帧已就位的面板）：
        // 构建线程 post 完消息即阻塞在 recv，此刻已不占访存带宽；
        // 若拿到新面板，就在同位置补一次 blit 即时上屏（与冷写并发已消除），
        // 数据显示不额外延迟一个采样周期。旧缓冲归还复用，稳态零大内存分配。
        let mut swapped = false;
        while let Ok(panel) = self.panel_rx.try_recv() {
            if let Some(old) = self.current.replace(panel) {
                let _ = self.req_tx.send(PanelJob::Recycle(old.pm));
            }
            swapped = true;
        }
        if swapped {
            if let Some(c) = &self.current {
                blit_rect(
                    px,
                    c.pm.as_ref(),
                    panel_dx,
                    panel_dy,
                    panel_sx,
                    panel_sy,
                    panel_sw,
                    panel_h,
                );
            }
        }

        // 标题行：状态灯 + 连接状态 + 设置齿轮（固定到任务栏/竖排时左侧还有 NAS 名称/IP）。
        // 固定态标题行贴任务栏一侧（任务栏在下方时贴面板底）；竖排与未固定横排都在面板顶部。
        // 必须在面板像素 blit 之后再画——面板底含这一行，先画会被盖掉。
        if let Some(font) = font {
            let baseline = if pinned {
                if up {
                    vis_h as f32 - PANEL_PAD - font.ascent(FONT_SIZE_SMALL)
                } else {
                    PANEL_PAD + font.ascent(FONT_SIZE_SMALL)
                }
            } else {
                y0 as f32 + PANEL_PAD + font.ascent(FONT_SIZE_SMALL)
            };
            // 竖排时面板在窗口里的横向偏移（竖条占另一侧），标题行随之平移
            let px0 = if vertical && !frame.panel_left {
                PILL_V_W as f32
            } else {
                0.0
            };
            let cy = baseline - font.ascent(FONT_SIZE_SMALL) * 0.34;
            // 齿轮贴面板右内边距；状态文字右对齐到齿轮左侧
            let gx = px0 + PANEL_W as f32 - PANEL_PAD - GEAR_SIZE;
            let gy = cy - GEAR_SIZE / 2.0;
            self.gear_pos = (gx, gy);
            let (text, sc, dc) = conn_badge(frame.conn, frame.dark);
            // 暗色下文字统一为白（状态灯仍用状态色）
            let tc = if frame.dark {
                color(FG.0, FG.1, FG.2, FG.3)
            } else {
                sc
            };
            let tw = font.measure(FONT_SIZE_SMALL, &text);
            let status_right = gx - 8.0;
            font.draw(px, status_right - tw, baseline, FONT_SIZE_SMALL, &text, tc);
            // 状态灯：连接状态色实心圆点，排在状态文字之前
            let dot_r = 3.0;
            let dot_cx = status_right - tw - 9.0 - dot_r;
            fill_dot(px, dot_cx, cy, dot_r, dc);
            if frame.pinned || vertical {
                // 固定到任务栏/竖排：名称/IP 排在状态灯之前（面板浅色底，用主题深色文字）
                let name = frame
                    .snap
                    .hostname
                    .as_deref()
                    .map(str::trim)
                    .filter(|h| !h.is_empty());
                let avail = (dot_cx - dot_r - 10.0 - px0 - PANEL_PAD).max(60.0);
                let d = text_c(frame.dark, FG_DIM);
                draw_title(
                    font,
                    px,
                    px0 + PANEL_PAD,
                    baseline,
                    avail,
                    (FONT_SIZE_HEADER, FONT_SIZE_SMALL),
                    (color(FG.0, FG.1, FG.2, FG.3), color(d.0, d.1, d.2, d.3)),
                    name,
                    frame.nas_host.trim(),
                );
            }
            if let Some(g) = self.gear.as_ref() {
                px.draw_pixmap(
                    gx as i32,
                    gy.round() as i32,
                    g.as_ref(),
                    &PixmapPaint::default(),
                    Transform::identity(),
                    None,
                );
            }
        }
        true
    }
}

/// 组装设备卡片：CPU/内存/GPU 仪表、网卡心电图、硬盘读写、存储状态。
/// 心电图历史由后台构建线程持有（UI 线程不再触碰）。
/// `cards` 为用户的显示开关；与 card_heights 用同一份开关，保证高度与内容对齐。
fn build_cards(
    s: &Snapshot,
    net_hist: &HashMap<String, VecDeque<(u64, u64)>>,
    disk_hist: &VecDeque<(u64, u64)>,
    cards: CardVis,
) -> Vec<Card> {
    let mut cards_out = Vec::new();
    let disks = s.disks.as_deref().unwrap_or(&[]);
    let volumes = s.volumes.as_deref().unwrap_or(&[]);
    let has_storage = !disks.is_empty() || !volumes.is_empty();

    // 顺序跟随设置里的勾选先后（最近勾选的排最后）
    for idx in cards.ordered() {
        match idx {
            // CPU：使用率 / 温度各自可读才画环；两项都读不到时整张卡不显示
            0 => {
                if let Some(c) = &s.cpu {
                    let usage = c.usage.map(|v| v as f32);
                    let temp = c.temp.map(|v| v as f32);
                    let mut rings = Vec::new();
                    if usage.is_some() {
                        rings.push(pct_ring(usage));
                    }
                    if temp.is_some() {
                        rings.push(temp_ring(temp));
                    }
                    if !rings.is_empty() {
                        cards_out.push(Card::dual_gauge("CPU", "", rings, CPU_CARD_H));
                    }
                }
            }
            // 内存
            1 => {
                if let Some(m) = &s.memory {
                    let usage = m.usage.map(|v| v as f32);
                    let subtitle = match (m.used, m.total) {
                        (Some(u), Some(t)) => format!("{} / {}", fmt::bytes(u), fmt::bytes(t)),
                        _ => String::new(),
                    };
                    if usage.is_some() || !subtitle.is_empty() {
                        cards_out.push(Card::big_gauge("内存", subtitle, usage));
                    }
                }
            }
            // 显卡：使用率环 + 温度环（有独显看独显，没有则看核显）。
            // 卡片左侧分两行：左上角厂商（Intel / AMD），下面一行具体型号（UHD 630）。
            2 => {
                let gpus = s.gpus.as_deref().unwrap_or(&[]);
                // 单卡时不带 `GPU0 · ` 前缀：左侧栏只有 ~110px 宽，前缀会吃掉近一半
                let multi = gpus.len() > 1;
                for g in gpus {
                    let (vendor, model) = split_gpu_name(&g.name);
                    let vram = match (g.memory_used, g.memory_total) {
                        (Some(mu), Some(mt)) => {
                            format!("显存 {}/{}", fmt::bytes(mu), fmt::bytes(mt))
                        }
                        _ => String::new(),
                    };
                    // 型号放第二行；有独显显存就并到同一行，型号在前——真放不下时截断的是显存，不丢型号
                    let detail = match (model.is_empty(), vram.is_empty()) {
                        (false, false) => format!("{model} · {vram}"),
                        (false, true) => model,
                        (true, _) => vram,
                    };
                    let title = if multi {
                        format!("GPU{} · {}", g.index, vendor)
                    } else {
                        vendor
                    };
                    let util = g.utilization.map(|v| v as f32);
                    let temp = g.temp.map(|v| v as f32);
                    // 使用率 / 温度读不到的环不画（标注一起省掉）；名称本身可读，卡片照常显示
                    let mut rings = Vec::new();
                    if util.is_some() {
                        rings.push(pct_ring(util));
                    }
                    if temp.is_some() {
                        rings.push(temp_ring(temp));
                    }
                    cards_out.push(Card::dual_gauge(title, detail, rings, DUAL_GAUGE_H));
                }
            }
            // 网卡心电图：每个接口一张图（蓝收/橙发），标题带上网卡协商速率（读不到则只显示"网络"）
            3 => {
                for n in s.network.as_deref().unwrap_or(&[]) {
                    let pts = net_hist.get(&n.name).map_or_else(Vec::new, hist_points);
                    // 读不到的收发速度整段不显示（连"收 / 发"字样一起省掉）
                    let d1 = n
                        .rx_bytes_per_sec
                        .map_or_else(String::new, |v| format!("收 {}", fmt::rate(v)));
                    let d2 = n
                        .tx_bytes_per_sec
                        .map_or_else(String::new, |v| format!("发 {}", fmt::rate(v)));
                    let speed = fmt::link_speed(n.speed_mbps.unwrap_or(0));
                    // 标题只留"网络"，链路协商速率作为右上角小字（避免标题串过长过重）
                    cards_out.push(Card::chart(
                        "网络".to_string(),
                        speed,
                        pts,
                        d1,
                        d2,
                        ACCENT,
                        TX_LINE,
                    ));
                }
            }
            // 硬盘读写：所有盘汇总的读（蓝）/写（绿）双曲线心电图
            4 => {
                if has_storage {
                    let (dr, dw) = disk_io_sum(s);
                    let pts = hist_points(disk_hist);
                    let d1 = format!("读 {}", fmt::rate(dr));
                    let d2 = format!("写 {}", fmt::rate(dw));
                    cards_out.push(Card::chart(
                        "硬盘读写".to_string(),
                        String::new(),
                        pts,
                        d1,
                        d2,
                        READ_BLUE,
                        LEVEL_GREEN,
                    ));
                }
            }
            // 存储状态：容量读不到的行整行不显示（连卷名 / 盘名一起省掉）
            5 => {
                let vols: Vec<&crate::data::Volume> =
                    volumes.iter().filter(|v| vol_row_ok(v)).collect();
                let ds: Vec<&Disk> = disks.iter().filter(|d| disk_row_ok(d)).collect();
                if !vols.is_empty() {
                    // 方案一：卷为主，默认只显示卷行；点开的卷列出底层盘（standby 灰显）
                    let rows = vols
                        .iter()
                        .map(|vol| VolRow {
                            name: vol.name.clone(),
                            cap: format!(
                                "{} / {}",
                                fmt::bytes(vol.used_bytes.unwrap_or(0)),
                                fmt::bytes(vol.total_bytes.unwrap_or(0))
                            ),
                            ratio: ratio_of(vol.used_bytes, vol.total_bytes),
                        })
                        .collect();
                    cards_out.push(Card::volumes(
                        "存储状态",
                        format!("{} 卷", vols.len()),
                        rows,
                    ));
                } else if !ds.is_empty() {
                    // 兜底：旧服务端/无卷数据时，盘直接平铺，保证物理盘可见
                    let rows = ds
                        .iter()
                        .map(|d| {
                            let name = d.device.trim_start_matches("/dev/").to_string();
                            let (ratio, cap) = disk_capacity_text(d);
                            BarRow { name, ratio, cap }
                        })
                        .collect();
                    cards_out.push(Card::bars("存储状态", format!("{} 块盘", ds.len()), rows));
                }
            }
            _ => {}
        }
    }

    cards_out
}

/// CPU 卡可显示：使用率 / 温度至少一项读得到（都读不到时连卡名一起不显示）。
fn cpu_card_ok(s: &Snapshot) -> bool {
    s.cpu
        .as_ref()
        .is_some_and(|c| c.usage.is_some() || c.temp.is_some())
}

/// 内存卡可显示：使用率或「已用 / 总量」读得到。
fn mem_card_ok(s: &Snapshot) -> bool {
    s.memory
        .as_ref()
        .is_some_and(|m| m.usage.is_some() || (m.used.is_some() && m.total.is_some()))
}

/// 卷行可显示：已用与总量都读得到（读不到的整行不显示，连卷名一起省掉）。
fn vol_row_ok(v: &crate::data::Volume) -> bool {
    v.total_bytes.is_some_and(|t| t > 0) && v.used_bytes.is_some()
}

/// 盘行可显示：同卷行。
fn disk_row_ok(d: &Disk) -> bool {
    d.total_bytes.is_some_and(|t| t > 0) && d.used_bytes.is_some()
}

/// 存储卡可显示的行数与行高：卷行 42 / 盘行 48（读不到容量的行整行不显示）。
fn storage_rows_geom(s: &Snapshot) -> (usize, f32) {
    let vols = s.volumes.as_deref().unwrap_or(&[]);
    let nv = vols.iter().filter(|v| vol_row_ok(v)).count();
    if nv > 0 {
        return (nv, VOL_ROW_H);
    }
    let nd = s
        .disks
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter(|d| disk_row_ok(d))
        .count();
    (nd, BAR_ROW_H)
}

/// 追加网卡样本并清理已消失的接口（后台线程持有心电图历史）。
fn update_net_hist(hist: &mut HashMap<String, VecDeque<(u64, u64)>>, s: &Snapshot) {
    let Some(nets) = &s.network else { return };
    for n in nets {
        let e = hist.entry(n.name.clone()).or_default();
        e.push_back((
            n.rx_bytes_per_sec.unwrap_or(0),
            n.tx_bytes_per_sec.unwrap_or(0),
        ));
        while e.len() > NET_HIST {
            e.pop_front();
        }
    }
    hist.retain(|k, _| nets.iter().any(|n| &n.name == k));
}

/// 追加硬盘总读/总写样本（所有盘汇总），用于读写心电图。
fn update_disk_hist(hist: &mut VecDeque<(u64, u64)>, s: &Snapshot) {
    let (dr, dw) = disk_io_sum(s);
    hist.push_back((dr, dw));
    while hist.len() > NET_HIST {
        hist.pop_front();
    }
}

/// 追加 CPU（使用率、温度）、显卡（使用率、温度）与内存使用率样本，供大卡全宽曲线。
/// 缺数据的指标记 -1，画图时跳过该点。
fn update_gauge_hist(
    cpu: &mut VecDeque<(f32, f32)>,
    gpu: &mut VecDeque<(f32, f32)>,
    mem: &mut VecDeque<f32>,
    s: &Snapshot,
) {
    let usage = s
        .cpu
        .as_ref()
        .and_then(|c| c.usage)
        .map_or(-1.0, |v| v as f32);
    let temp = s
        .cpu
        .as_ref()
        .and_then(|c| c.temp)
        .map_or(-1.0, |v| v as f32);
    cpu.push_back((usage, temp));
    while cpu.len() > NET_HIST {
        cpu.pop_front();
    }
    // 显卡只取第一张（与网络/硬盘卡同样只显示首项），温度缺失同样记 -1
    let g = s.gpus.as_deref().and_then(|g| g.first());
    let gu = g.and_then(|g| g.utilization).map_or(-1.0, |v| v as f32);
    let gt = g.and_then(|g| g.temp).map_or(-1.0, |v| v as f32);
    gpu.push_back((gu, gt));
    while gpu.len() > NET_HIST {
        gpu.pop_front();
    }
    let mu = s
        .memory
        .as_ref()
        .and_then(|m| m.usage)
        .map_or(-1.0, |v| v as f32);
    mem.push_back(mu);
    while mem.len() > NET_HIST {
        mem.pop_front();
    }
}

/// 展开面板总高度（仅由设备数量与卡片开关决定）。
pub(super) fn panel_height(has_data: bool, snap: &Snapshot, cards: CardVis) -> u32 {
    if !has_data {
        return (PANEL_PAD * 2.0 + HEADER_H) as u32;
    }
    let hs = card_heights(snap, cards);
    let sum: f32 = hs.iter().sum();
    if hs.is_empty() {
        // 卡片被全部关掉（理论上设置界面会拦，这里兜底）：只留标题行高度
        return (PANEL_PAD * 2.0 + HEADER_H) as u32;
    }
    (PANEL_PAD * 2.0 + HEADER_H + sum + CARD_GAP * (hs.len() as f32 - 1.0)) as u32
}

/// 展开面板高度：固定到任务栏时窗口本身就是面板（底边贴任务栏，无顶栏）；
/// 竖排时与竖条并排、面板顶边对齐竖条顶边（同样无顶栏）；未固定横排 = 顶栏 + 间隙 + 面板。
pub(super) fn expanded_h(frame: &Frame) -> u32 {
    let ph = panel_height(frame.has_data, frame.snap, frame.cards);
    if frame.pinned || frame.vertical {
        ph
    } else {
        CAPSULE_H + GAP + ph
    }
}

/// 卡片区起始 y：标题行在底部时卡片从顶部内边距开始，否则让出顶部标题行。
fn card_top_y(header_bottom: bool) -> f32 {
    if header_bottom {
        PANEL_PAD
    } else {
        PANEL_PAD + HEADER_H
    }
}

/// 后台面板构建线程：把每秒一次的整层光栅化（2~4ms）移出 UI 帧关键路径。
/// 持有独立的字体实例、心电图历史与可复用 clip mask；
/// 请求通道关闭（UI 退出）时 recv 出错，线程自然结束。
pub(super) struct PanelBuilder {
    font: Option<Font>,
    net_hist: HashMap<String, VecDeque<(u64, u64)>>,
    disk_hist: VecDeque<(u64, u64)>,
    /// CPU 历史：(使用率%, 温度°C)，温度缺省记 -1；大卡曲线用
    cpu_hist: VecDeque<(f32, f32)>,
    /// 显卡历史：(使用率%, 温度°C)，缺省记 -1
    gpu_hist: VecDeque<(f32, f32)>,
    /// 内存使用率历史（%）
    mem_hist: VecDeque<f32>,
    /// 本次构建得到的柱状图几何（随面板一起交给 UI 线程）
    bars_pending: Option<BarsGeom>,
    last_seq: u64,
    clip: Option<Mask>,
    clip_w: u32,
    clip_h: u32,
    /// UI 归还的面板像素缓冲（尺寸匹配时复用，稳态零大分配）
    spare: Vec<Pixmap>,
    rx: mpsc::Receiver<PanelJob>,
    tx: mpsc::Sender<BuiltPanel>,
    hwnd: Arc<AtomicIsize>,
}

impl PanelBuilder {
    pub(super) fn spawn(
        hwnd: Arc<AtomicIsize>,
    ) -> (mpsc::Sender<PanelJob>, mpsc::Receiver<BuiltPanel>) {
        let (req_tx, req_rx) = mpsc::channel();
        let (out_tx, out_rx) = mpsc::channel();
        thread::Builder::new()
            .name("panel".into())
            .spawn(move || {
                let mut b = PanelBuilder {
                    font: Font::load(),
                    net_hist: HashMap::new(),
                    disk_hist: VecDeque::new(),
                    cpu_hist: VecDeque::new(),
                    gpu_hist: VecDeque::new(),
                    mem_hist: VecDeque::new(),
                    bars_pending: None,
                    last_seq: 0,
                    clip: None,
                    clip_w: 0,
                    clip_h: 0,
                    spare: Vec::new(),
                    rx: req_rx,
                    tx: out_tx,
                    hwnd,
                };
                // 低于 UI 线程优先级：面板构建是 1Hz 离屏活，晚几毫秒无妨，
                // 但不能抢占动画/数据帧造成 6ms 以上的调度尖峰
                unsafe {
                    let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
                }
                // 启动空闲期预热字形冷光栅化，展开首帧不卡
                if let Some(f) = &b.font {
                    f.preheat();
                }
                b.run();
            })
            .expect("spawn panel thread");
        (req_tx, out_rx)
    }

    fn run(&mut self) {
        while let Ok(first) = self.rx.recv() {
            // 排空积压：Advance 按序推进历史；多个 Build 只构建最新一份。
            // 1Hz 推送下通常没有积压，循环只在动画/切换瞬间生效。
            let mut pending: Option<BuildReq> = None;
            self.handle(first, &mut pending);
            while let Ok(job) = self.rx.try_recv() {
                self.handle(job, &mut pending);
            }
            let Some(req) = pending else { continue };
            let panel = self.build(&req);
            if self.tx.send(panel).is_err() {
                break; // UI 端已释放接收端
            }
            // 构建可能晚于触发帧，主动唤醒 UI 取结果重绘
            let h = self.hwnd.load(Ordering::Acquire);
            if h != 0 {
                unsafe {
                    let _ = PostMessageW(HWND(h as *mut _), WM_APP_PANEL, WPARAM(0), LPARAM(0));
                }
            }
        }
    }

    fn handle(&mut self, job: PanelJob, pending: &mut Option<BuildReq>) {
        match job {
            PanelJob::Advance {
                seq,
                has_data,
                snap,
            } => {
                if has_data && seq != self.last_seq {
                    self.last_seq = seq;
                    update_net_hist(&mut self.net_hist, &snap);
                    update_disk_hist(&mut self.disk_hist, &snap);
                    update_gauge_hist(
                        &mut self.cpu_hist,
                        &mut self.gpu_hist,
                        &mut self.mem_hist,
                        &snap,
                    );
                }
            }
            PanelJob::Build(req) => *pending = Some(req),
            PanelJob::Recycle(pm) => self.spare.push(pm),
        }
    }

    fn build(&mut self, req: &BuildReq) -> BuiltPanel {
        // 心电图历史只按数据版本推进一次
        if req.has_data && req.seq != self.last_seq {
            self.last_seq = req.seq;
            update_net_hist(&mut self.net_hist, &req.snap);
            update_disk_hist(&mut self.disk_hist, &req.snap);
            update_gauge_hist(
                &mut self.cpu_hist,
                &mut self.gpu_hist,
                &mut self.mem_hist,
                &req.snap,
            );
        }

        let (w, ph) = (req.w, req.ph);
        self.bars_pending = None;
        // 优先复用 UI 归还的同尺寸缓冲；复用前整幅清零（圆角外域必须保持透明）
        let mut pm = match self
            .spare
            .iter()
            .position(|p| p.width() == w && p.height() == ph)
        {
            Some(i) => {
                let mut pm = self.spare.swap_remove(i);
                pm.data_mut().fill(0);
                pm
            }
            None => Pixmap::new(w, ph).unwrap(),
        };
        if self.clip.as_ref().is_none() || self.clip_h != ph || self.clip_w != w {
            self.clip = Mask::new(w, ph);
            self.clip_w = w;
            self.clip_h = ph;
        }

        if req.big {
            self.paint_big(&mut pm, req);
        } else {
            let heights = card_heights(&req.snap, req.cards);
            // 背景与卡片底/描边：clip mask 借用限制在本块内
            {
                let clip = self.clip.as_mut().unwrap();
                fill_round_opaque(&mut pm, clip, 0.0, 0.0, w as f32, ph as f32, 14.0, BG);

                let mut cy = card_top_y(req.header_bottom);
                for &h in &heights {
                    fill_round_opaque(&mut pm, clip, PANEL_PAD, cy, CARD_W, h, 10.0, CARD_BG);
                    stroke_card(&mut pm, PANEL_PAD, cy, CARD_W, h);
                    cy += h + CARD_GAP;
                }
            }

            if let Some(font) = &self.font {
                // 标题行的连接状态灯/文字与设置齿轮由 render_expanded 在面板 blit 之后绘制
                if req.has_data {
                    let cards = build_cards(&req.snap, &self.net_hist, &self.disk_hist, req.cards);
                    let mut cy = card_top_y(req.header_bottom);
                    for card in &cards {
                        draw_card(&mut pm, font, PANEL_PAD, cy, CARD_W, card, req.dark);
                        cy += card.h + CARD_GAP;
                    }
                }
            }
        }

        BuiltPanel {
            seq: req.seq,
            conn: req.conn.clone(),
            w,
            ph,
            big: req.big,
            dark: req.dark,
            header_bottom: req.header_bottom,
            cards: req.cards,
            bars: self.bars_pending,
            pm,
        }
    }

    /// 超大卡片整层：不透明圆角底 + 两列卡片（复用 292 宽的小面板卡片）。
    /// 标题行由 UI 线程在 blit 之后绘制（齿轮图标在视图侧）。
    fn paint_big(&mut self, pm: &mut Pixmap, req: &BuildReq) {
        let (w, h) = (req.w as f32, req.ph as f32);
        {
            let clip = self.clip.as_mut().unwrap();
            fill_round_opaque(pm, clip, 0.0, 0.0, w, h, BIG_R, BIG_BG);
        }
        if !req.has_data {
            return;
        }
        let Some(font) = &self.font else { return };
        let s = &req.snap;
        let dark = req.dark;
        let d = text_c(dark, FG_DIM);
        let dim = color(d.0, d.1, d.2, d.3);
        let fg = color(FG.0, FG.1, FG.2, FG.3);
        let x0 = BIG_PAD;
        let w = BIG_INNER_W;
        // 每块图表各自一张卡片：内容再内缩一层内边距
        let ix = x0 + BIG_CARD_PAD;
        let iw = w - BIG_CARD_PAD * 2.0;
        // 左右两栏：左栏实时曲线 + 实时读数，右栏统计（列表 / 柱状图）
        const LEFT_W: f32 = 344.0;
        let cx = ix + 34.0;
        let cw = LEFT_W - 34.0;
        // 通栏卡片（显卡 / 硬盘读写）的曲线宽度：占满整卡，不再留右栏
        let cw_full = iw - 34.0;
        let div_x = ix + LEFT_W + 6.0;
        let rxo = div_x + 10.0;
        let rw = ix + iw - rxo;
        let cy_off = 36.0;
        // 显示哪些块完全跟随设置窗的「显示卡片」开关，顺序与设置窗一致
        let cards = req.cards;
        let cpu = s.cpu.as_ref();
        let gpu = s.gpus.as_deref().and_then(|g| g.first());
        let net = s.network.as_deref().and_then(|n| n.first());
        let vols = s.volumes.as_deref().unwrap_or(&[]);
        let disks = s.disks.as_deref().unwrap_or(&[]);
        // 显示哪些块：开关打开 **且** 该项读得到（读不到的块整块不显示，连标题一起省掉）
        let show_cpu = cards.cpu && cpu_card_ok(s);
        let show_mem = cards.memory && mem_card_ok(s);
        let show_gpu = cards.gpu && gpu.is_some();
        let show_net = cards.network && net.is_some();
        let show_disk = cards.disk_io && !(disks.is_empty() && vols.is_empty());
        // 存储块与小面板同源：优先卷行，无卷时退回物理盘；容量读不到的行不计入
        let store_rows = storage_rows_geom(s).0;
        let show_store = cards.storage && store_rows > 0;

        // 面板与卡片高度都固定：始终按 6 个卡位算，勾选少时留白、不缩面板；
        // 只有工作区太矮、6 个 152 的卡位放不下时才等比压缩。
        const STORE_HEAD: f32 = 42.0;
        const STORE_ROW: f32 = 28.0;
        let slots = crate::config::CARD_COUNT as f32;
        let avail_h = h - BIG_PAD * 2.0 - BIG_HEADER_H;
        let fixed_h = BIG_CARD_H * slots + BIG_BLOCK_GAP * (slots - 1.0);
        let bh = if fixed_h <= avail_h {
            BIG_CARD_H
        } else {
            ((avail_h - BIG_BLOCK_GAP * (slots - 1.0)) / slots).max(110.0)
        };
        // 存储卡最多 4 行卷，行数随卡片高度自适应
        let shown_rows = (((bh - STORE_HEAD) / STORE_ROW).floor() as usize)
            .clamp(1, 4)
            .min(store_rows);
        let ch = (bh - cy_off - 58.0).max(24.0);
        // 块顺序跟随设置里的勾选先后：先按顺序给每块算好 y（块之间不重叠，
        // 绘制先后不影响结果），未显示的块不占位。
        let visible = [
            show_cpu, show_mem, show_gpu, show_net, show_disk, show_store,
        ];
        let mut ypos = [0.0f32; 6];
        let mut rank = 0usize;
        for idx in cards.ordered() {
            if visible[idx] {
                ypos[idx] = BIG_PAD + BIG_HEADER_H + rank as f32 * (bh + BIG_BLOCK_GAP);
                rank += 1;
            }
        }

        // ---- CPU：使用率面积 + 温度折线 ----
        if show_cpu {
            let y = ypos[0];
            let usage = cpu.and_then(|c| c.usage).map(|v| v as f32);
            let temp = cpu.and_then(|c| c.temp).map(|v| v as f32);
            draw_big_card(pm, self.clip.as_mut().unwrap(), x0, y, w, bh);
            // 右上说明只列真的画得出的指标
            let cpu_note = if temp.is_some() {
                "使用率 · 温度"
            } else {
                "使用率"
            };
            draw_big_title(pm, font, ix, iw, y, "CPU", cpu_note, fg, dim);
            draw_big_grid(
                pm,
                font,
                cx,
                y + cy_off,
                cw,
                ch,
                &percent_grid_labels(),
                dim,
            );
            draw_big_series(
                pm,
                cx,
                y + cy_off,
                cw,
                ch,
                &hist_ratio_pts(&self.cpu_hist, false),
                ACCENT,
                true,
            );
            draw_big_series(
                pm,
                cx,
                y + cy_off,
                cw,
                ch,
                &hist_ratio_pts(&self.cpu_hist, true),
                WARN,
                false,
            );
            let sy = y + bh - 48.0;
            // 读数行：读不到的指标整段不画（连"利用率 / 温度"标签一起省掉），依次左排
            let mut sx = ix;
            if let Some(v) = usage {
                let (txt, col) = (format!("{v:.0}%"), usage_color(v));
                draw_big_stat(pm, font, sx, sy, col, "利用率", &txt, 22.0, dark, dim);
                sx += 174.0;
            }
            if let Some(v) = temp {
                let (txt, col) = (format!("{v:.0}°C"), temp_color(v));
                draw_big_stat(pm, font, sx, sy, col, "温度", &txt, 22.0, dark, dim);
            }
            // 右栏：平均负载（/proc/loadavg 三个时间窗口）；读不到时整块不显示
            let load = match (
                cpu.and_then(|c| c.load1),
                cpu.and_then(|c| c.load5),
                cpu.and_then(|c| c.load15),
            ) {
                (Some(a), Some(b), Some(c)) => {
                    let mut rows = vec![
                        ("1 分钟".to_string(), format!("{a:.2}")),
                        ("5 分钟".to_string(), format!("{b:.2}")),
                        ("15 分钟".to_string(), format!("{c:.2}")),
                    ];
                    // 30 分钟档是服务端自算的（对 load1 采样取均值），覆盖满 30 分钟才有值
                    if let Some(d) = cpu.and_then(|c| c.load30) {
                        rows.push(("30 分钟".to_string(), format!("{d:.2}")));
                    }
                    Some(rows)
                }
                _ => None,
            };
            if let Some(load_rows) = load {
                draw_big_divider(pm, div_x, y + 32.0, bh - 46.0);
                draw_big_kv_grid(pm, font, rxo, y + 30.0, rw, bh - 40.0, &load_rows, fg, dim);
            }
        }

        // ---- 内存：使用率面积 ----
        if show_mem {
            let y = ypos[1];
            let mem = s.memory.as_ref();
            let mem_usage = mem.and_then(|m| m.usage).map(|v| v as f32);
            let mem_note = match (mem.and_then(|m| m.used), mem.and_then(|m| m.total)) {
                (Some(u), Some(t)) => {
                    format!("{} / {}", fmt::bytes_unit(u), fmt::bytes_unit(t))
                }
                _ => String::new(),
            };
            draw_big_card(pm, self.clip.as_mut().unwrap(), x0, y, w, bh);
            draw_big_title(pm, font, ix, iw, y, "内存", &mem_note, fg, dim);
            draw_big_grid(
                pm,
                font,
                cx,
                y + cy_off,
                cw,
                ch,
                &percent_grid_labels(),
                dim,
            );
            draw_big_series(
                pm,
                cx,
                y + cy_off,
                cw,
                ch,
                &mem_ratio_pts(&self.mem_hist),
                ACCENT,
                true,
            );
            let sy = y + bh - 48.0;
            // 一位小数：整数百分比在空闲机器上会长时间停在同一格，看起来像"卡住了"
            if let Some(v) = mem_usage {
                let (txt, col) = (format!("{v:.1}%"), usage_color(v));
                draw_big_stat(pm, font, ix, sy, col, "使用率", &txt, 22.0, dark, dim);
            }
            // 右栏：内存明细小卡片（总计 / 已用 / 缓存 / 可用，三者相加等于总计）；
            // 读不到的项整行不显示，一行都读不到时整块不显示
            let mem_rows: Vec<(String, String)> = match mem {
                Some(m) => [
                    ("总计", m.total),
                    ("已用", m.used),
                    ("缓存", m.cache),
                    ("可用", m.free),
                ]
                .into_iter()
                .filter_map(|(k, v)| v.map(|b| (k.to_string(), fmt::bytes_unit(b))))
                .collect(),
                None => Vec::new(),
            };
            if !mem_rows.is_empty() {
                draw_big_divider(pm, div_x, y + 32.0, bh - 46.0);
                draw_big_kv_grid(pm, font, rxo, y + 30.0, rw, bh - 40.0, &mem_rows, fg, dim);
            }
        }

        // ---- 显卡：使用率面积 + 温度折线（与小面板一样是双指标）----
        if show_gpu {
            if let Some(g) = gpu {
                let y = ypos[2];
                let gu = g.utilization.map(|v| v as f32);
                let gt = g.temp.map(|v| v as f32);
                draw_big_card(pm, self.clip.as_mut().unwrap(), x0, y, w, bh);
                draw_big_title(pm, font, ix, iw, y, "显卡", &g.name, fg, dim);
                draw_big_grid(
                    pm,
                    font,
                    cx,
                    y + cy_off,
                    cw_full,
                    ch,
                    &percent_grid_labels(),
                    dim,
                );
                draw_big_series(
                    pm,
                    cx,
                    y + cy_off,
                    cw_full,
                    ch,
                    &hist_ratio_pts(&self.gpu_hist, false),
                    ACCENT,
                    true,
                );
                draw_big_series(
                    pm,
                    cx,
                    y + cy_off,
                    cw_full,
                    ch,
                    &hist_ratio_pts(&self.gpu_hist, true),
                    WARN,
                    false,
                );
                let sy = y + bh - 48.0;
                // 读数行：读不到的指标整段不画（标签一起省掉），依次左排
                let mut sx = ix;
                if let Some(v) = gu {
                    let (txt, col) = (format!("{v:.0}%"), usage_color(v));
                    draw_big_stat(pm, font, sx, sy, col, "利用率", &txt, 22.0, dark, dim);
                    sx += 174.0;
                }
                if let Some(v) = gt {
                    let (txt, col) = (format!("{v:.0}°C"), temp_color(v));
                    draw_big_stat(pm, font, sx, sy, col, "温度", &txt, 22.0, dark, dim);
                    sx += 174.0;
                }
                // 通栏：第三列显示显存（已用 / 总量 + 占用百分比）；读不到时整段不显示
                if let (Some(mu), Some(mt)) = (g.memory_used, g.memory_total) {
                    if mt > 0 {
                        let note = format!("显存 {:.0}%", mu as f64 / mt as f64 * 100.0);
                        let txt = format!("{} / {}", fmt::bytes_unit(mu), fmt::bytes_unit(mt));
                        draw_big_stat(pm, font, sx, sy, FG_DIM, &note, &txt, 16.0, dark, dim);
                    }
                }
            }
        }

        // ---- 网络：接收 / 发送双线 ----
        if show_net {
            let y = ypos[3];
            let iface = net.map_or("--", |n| n.name.as_str());
            let speed = net.map_or(String::new(), |n| {
                fmt::link_speed(n.speed_mbps.unwrap_or(0))
            });
            let net_note = if speed.is_empty() {
                iface.to_string()
            } else {
                format!("{iface} · {speed}")
            };
            draw_big_card(pm, self.clip.as_mut().unwrap(), x0, y, w, bh);
            draw_big_title(pm, font, ix, iw, y, "网络", &net_note, fg, dim);
            let q = self.net_hist.get(iface);
            let (net_max, net_pts) =
                q.map_or((0.0f64, Vec::new()), |q| (hist_max(q), hist_points(q)));
            let net_labels = rate_grid_labels(net_max);
            draw_big_grid(pm, font, cx, y + cy_off, cw, ch, &net_labels, dim);
            let rx_pts: Vec<(f32, f32)> = net_pts.iter().map(|p| (p.0, p.1)).collect();
            let tx_pts: Vec<(f32, f32)> = net_pts.iter().map(|p| (p.0, p.2)).collect();
            draw_big_series(pm, cx, y + cy_off, cw, ch, &rx_pts, ACCENT, true);
            draw_big_series(pm, cx, y + cy_off, cw, ch, &tx_pts, TX_LINE, false);
            let sy = y + bh - 48.0;
            // 读数行：读不到的方向整段不画（标签一起省掉），依次左排
            let mut sx = ix;
            if let Some(v) = net.and_then(|n| n.rx_bytes_per_sec) {
                draw_big_stat(
                    pm,
                    font,
                    sx,
                    sy,
                    ACCENT,
                    "接收速度",
                    &fmt::rate_unit(v),
                    22.0,
                    dark,
                    dim,
                );
                sx += 174.0;
            }
            if let Some(v) = net.and_then(|n| n.tx_bytes_per_sec) {
                draw_big_stat(
                    pm,
                    font,
                    sx,
                    sy,
                    TX_LINE,
                    "发送速度",
                    &fmt::rate_unit(v),
                    22.0,
                    dark,
                    dim,
                );
            }
            // 右栏：近 30 日堆叠柱状图（下蓝＝下行 / 上橙＝上行）
            // 下面的信息栏由 UI 线程叠加绘制（悬停时换成单日数据），这里只画图并记几何。
            // 服务端没给流量统计时整块不显示，连分隔线一起省掉。
            if let Some(tf) = s.traffic.as_ref() {
                draw_big_divider(pm, div_x, y + 32.0, bh - 46.0);
                font.draw(pm, rxo, y + 30.0, 9.5, "近 30 日流量（下蓝 / 上橙）", dim);
                let bars_y = y + 40.0;
                let line_y = y + bh - 12.0;
                let bars_h = (line_y - 18.0) - bars_y;
                let count = tf.days_rx.len().min(tf.days_tx.len()).min(30);
                if bars_h >= 22.0 && count > 0 {
                    draw_big_bars(pm, rxo, bars_y, rw, bars_h, &tf.days_rx, &tf.days_tx);
                    self.bars_pending = Some(BarsGeom {
                        x: rxo,
                        y: bars_y,
                        w: rw,
                        h: bars_h,
                        line_y,
                        count,
                    });
                }
            }
        }

        // ---- 硬盘读写：读 / 写双线 ----
        if show_disk {
            let y = ypos[4];
            let (dr, dw) = disk_io_sum(s);
            draw_big_card(pm, self.clip.as_mut().unwrap(), x0, y, w, bh);
            draw_big_title(pm, font, ix, iw, y, "硬盘读写", "读 / 写", fg, dim);
            let disk_max = hist_max(&self.disk_hist);
            let disk_labels = rate_grid_labels(disk_max);
            draw_big_grid(pm, font, cx, y + cy_off, cw_full, ch, &disk_labels, dim);
            let disk_pts = hist_points(&self.disk_hist);
            let read_pts: Vec<(f32, f32)> = disk_pts.iter().map(|p| (p.0, p.1)).collect();
            let write_pts: Vec<(f32, f32)> = disk_pts.iter().map(|p| (p.0, p.2)).collect();
            draw_big_series(pm, cx, y + cy_off, cw_full, ch, &read_pts, READ_BLUE, true);
            draw_big_series(
                pm,
                cx,
                y + cy_off,
                cw_full,
                ch,
                &write_pts,
                LEVEL_GREEN,
                false,
            );
            let sy = y + bh - 48.0;
            draw_big_stat(
                pm,
                font,
                ix,
                sy,
                READ_BLUE,
                "读取速度",
                &fmt::rate_unit(dr),
                22.0,
                dark,
                dim,
            );
            draw_big_stat(
                pm,
                font,
                ix + 174.0,
                sy,
                LEVEL_GREEN,
                "写入速度",
                &fmt::rate_unit(dw),
                22.0,
                dark,
                dim,
            );
            // 通栏：第三列显示磁盘总容量（单盘明细在存储卡里）；容量读不到时整段不显示
            let (du, dt): (u64, u64) = {
                let u: u64 = disks.iter().filter_map(|d| d.used_bytes).sum();
                let t: u64 = disks.iter().filter_map(|d| d.total_bytes).sum();
                (u, t)
            };
            if dt > 0 {
                let cap_txt = format!("{} / {}", fmt::bytes_unit(du), fmt::bytes_unit(dt));
                draw_big_stat(
                    pm,
                    font,
                    ix + 348.0,
                    sy,
                    FG_DIM,
                    "总容量",
                    &cap_txt,
                    16.0,
                    dark,
                    dim,
                );
            }
        }

        // ---- 存储状态：卷用量进度条（通栏，无卷时退回物理盘）----
        if show_store {
            let y = ypos[5];
            // 容量读不到的行整行不显示（连卷名 / 盘名一起省掉）
            let (rows, used, total): (Vec<(String, String, Option<f32>)>, u64, u64) = {
                let vrows: Vec<&crate::data::Volume> =
                    vols.iter().filter(|v| vol_row_ok(v)).collect();
                if !vrows.is_empty() {
                    let rows = vrows
                        .iter()
                        .map(|v| {
                            (
                                v.name.clone(),
                                capacity_text_unit(v.used_bytes, v.total_bytes),
                                ratio_of(v.used_bytes, v.total_bytes),
                            )
                        })
                        .collect();
                    let u = vrows.iter().filter_map(|v| v.used_bytes).sum();
                    let t = vrows.iter().filter_map(|v| v.total_bytes).sum();
                    (rows, u, t)
                } else {
                    let drows: Vec<&Disk> = disks.iter().filter(|d| disk_row_ok(d)).collect();
                    let rows = drows
                        .iter()
                        .map(|d| {
                            let name = d
                                .device
                                .rsplit('/')
                                .next()
                                .unwrap_or(d.device.as_str())
                                .to_string();
                            (
                                name,
                                capacity_text_unit(d.used_bytes, d.total_bytes),
                                ratio_of(d.used_bytes, d.total_bytes),
                            )
                        })
                        .collect();
                    let u = drows.iter().filter_map(|d| d.used_bytes).sum();
                    let t = drows.iter().filter_map(|d| d.total_bytes).sum();
                    (rows, u, t)
                }
            };
            // 通栏：已用 / 可用 / 使用率直接放在标题行右侧
            let note = if total > 0 {
                format!(
                    "已用 {} · 可用 {} · {:.1}%",
                    fmt::bytes_unit(used),
                    fmt::bytes_unit(total.saturating_sub(used)),
                    used as f64 / total as f64 * 100.0
                )
            } else {
                String::new()
            };
            draw_big_card(pm, self.clip.as_mut().unwrap(), x0, y, w, bh);
            draw_big_title(pm, font, ix, iw, y, "存储", &note, fg, dim);
            // 卷用量条通栏
            let bar_w = iw;
            for (i, (name, cap, ratio)) in rows.iter().take(shown_rows).enumerate() {
                let ry = y + 36.0 + i as f32 * STORE_ROW;
                let base = ry + 13.0;
                let right = ix + bar_w;
                let cap_w = font.measure(10.5, cap);
                font.draw(pm, right - cap_w, base, 10.5, cap, dim);
                let nm = fit_text(font, 10.5, name, (right - cap_w - 12.0 - ix).max(40.0));
                font.draw(pm, ix, base, 10.5, &nm, fg);
                const PB: f32 = 5.0;
                fill_round(pm, ix, ry + 18.0, bar_w, PB, PB / 2.0, TRACK);
                if let Some(p) = ratio {
                    let pc = usage_color(*p);
                    let fw = (bar_w * (p / 100.0).clamp(0.0, 1.0)).max(PB);
                    fill_round(pm, ix, ry + 18.0, fw, PB, PB / 2.0, pc);
                }
            }
        }
    }
}

/// 绘制一张卡片正文（背景与描边由 panel_bg 缓存统一提供），按正文类型分发。
/// `dark` 只影响**文字取色**（暗色下所有文字走中性色，随后被 apply_dark 提白），
/// 环/条/曲线等图形仍按亮色的彩色画。
fn draw_card(px: &mut Pixmap, font: &Font, x: f32, y: f32, w: f32, card: &Card, dark: bool) {
    let h = card.h;

    let fg = color(FG.0, FG.1, FG.2, FG.3);
    // 次要文字：亮色用灰，暗色取主前景色（apply_dark 把深中性色提白）。
    // 直接用 FG_DIM 的话，它的抗锯齿中间覆盖率像素会落在提白阈值下方、被反相成中灰，
    // 整排小字看着是"灰的"而不是"白的"。
    let dc = text_c(dark, FG_DIM);
    let dim = color(dc.0, dc.1, dc.2, dc.3);

    match &card.body {
        Body::DualGauge {
            title,
            subtitle,
            rings,
        } => {
            // 右：数据环（数值在环心偏上、标注在环内下方）；环数随可读指标变化，右对齐
            const RING_R: f32 = 33.0;
            const RING_FS: f32 = GAUGE_VALUE_FS;
            const CAP_FS: f32 = GAUGE_CAP_FS;
            const RING_GAP: f32 = 82.0;
            let cy = y + h / 2.0;
            let cx_t = x + w - CARD_PAD - RING_R - 1.0;
            let n_ring = rings.len();
            let cx_u = cx_t - RING_GAP * n_ring.saturating_sub(1) as f32;
            // 名称可用宽度：到最左环边缘为止
            let avail = (cx_u - RING_R - 4.5 - (x + CARD_PAD)).max(60.0);

            // 左：名称（有小字明细时名称在上、明细在下；否则整体垂直居中）
            // 名称可用宽度只有 ~110px：先试一行；放不下按空格折两行（显卡名 `Intel UHD 630`
            // 就是这种情况，折行比缩字号清楚）；两行仍放不下才降一档字号，最后才截断。
            // 卡片标题一律**粗体**（与 NAS 名同级）；宽度按粗体测量，避免截断。
            const TITLE_MIN_FS: f32 = FONT_SIZE;
            let one_line = font.measure_bold(FONT_SIZE_TITLE, title) <= avail;
            let two_lines = if one_line {
                None
            } else {
                wrap_title_bold(font, FONT_SIZE_TITLE, title, avail, 2)
            };
            if let Some(lines) = two_lines {
                // 两行标题（+可选副行）作为一整块在卡片里垂直居中
                const LH: f32 = 17.0;
                const SUB_H: f32 = 15.0;
                let sub_h = if subtitle.is_empty() { 0.0 } else { SUB_H };
                let top = cy - (lines.len() as f32 * LH + sub_h) / 2.0;
                for (i, ln) in lines.iter().enumerate() {
                    let bl = top + LH / 2.0 + i as f32 * LH + font.ascent(FONT_SIZE_TITLE) / 2.0;
                    font.draw_bold(px, x + CARD_PAD, bl, FONT_SIZE_TITLE, ln, fg);
                }
                if !subtitle.is_empty() {
                    let sub = fit_text(font, FONT_SIZE_SMALL, subtitle, avail);
                    let bl = top
                        + lines.len() as f32 * LH
                        + SUB_H / 2.0
                        + font.ascent(FONT_SIZE_SMALL) / 2.0;
                    font.draw(px, x + CARD_PAD, bl, FONT_SIZE_SMALL, &sub, dim);
                }
            } else {
                let (title, tfs) = if one_line {
                    (title.clone(), FONT_SIZE_TITLE)
                } else if font.measure_bold(TITLE_MIN_FS, title) <= avail {
                    (title.clone(), TITLE_MIN_FS)
                } else {
                    (
                        fit_text_bold(font, TITLE_MIN_FS, title, avail),
                        TITLE_MIN_FS,
                    )
                };
                if subtitle.is_empty() {
                    font.draw_bold(
                        px,
                        x + CARD_PAD,
                        cy + font.ascent(tfs) / 2.0,
                        tfs,
                        &title,
                        fg,
                    );
                } else {
                    let sub = fit_text(font, FONT_SIZE_SMALL, subtitle, avail);
                    font.draw_bold(
                        px,
                        x + CARD_PAD,
                        y + 40.0 + font.ascent(tfs),
                        tfs,
                        &title,
                        fg,
                    );
                    font.draw(px, x + CARD_PAD, y + 74.0, FONT_SIZE_SMALL, &sub, dim);
                }
            }

            for (i, spec) in rings.iter().enumerate() {
                let cx = cx_t - RING_GAP * (n_ring - 1 - i) as f32;
                draw_ring(px, cx, cy, RING_R, 9.0, spec.ratio, spec.color);
                let vc = if spec.ratio.is_some() {
                    let c = text_c(dark, spec.color);
                    color(c.0, c.1, c.2, c.3)
                } else {
                    fg
                };
                let tw = font.measure(RING_FS, &spec.center);
                font.draw(px, cx - tw / 2.0, cy - 1.0, RING_FS, &spec.center, vc);
                let cw = font.measure(CAP_FS, &spec.cap);
                font.draw(px, cx - cw / 2.0, cy + 13.0, CAP_FS, &spec.cap, dim);
            }
        }
        Body::BigGauge {
            title,
            subtitle,
            pct,
        } => {
            // 左：名称 + 小字明细，垂直居中（标题粗体，同 DualGauge）
            let title = fit_text_bold(font, FONT_SIZE_TITLE, title, 150.0);
            if subtitle.is_empty() {
                font.draw_bold(
                    px,
                    x + CARD_PAD,
                    y + h / 2.0 + font.ascent(FONT_SIZE_TITLE) / 2.0,
                    FONT_SIZE_TITLE,
                    &title,
                    fg,
                );
            } else {
                let sub = fit_text(font, FONT_SIZE_SMALL, subtitle, 150.0);
                font.draw_bold(
                    px,
                    x + CARD_PAD,
                    y + 40.0 + font.ascent(FONT_SIZE_TITLE),
                    FONT_SIZE_TITLE,
                    &title,
                    fg,
                );
                font.draw(px, x + CARD_PAD, y + 74.0, FONT_SIZE_SMALL, &sub, dim);
            }
            // 右：单个大圆环（使用率读不到时整环不画，连「使用率」一起省掉）
            if let Some(pct) = *pct {
                const RING_R: f32 = 36.0;
                const RING_FS: f32 = GAUGE_VALUE_FS;
                let cy = y + h / 2.0;
                let cx = x + w - CARD_PAD - RING_R - 2.0;
                let rc = usage_color(pct);
                draw_ring(px, cx, cy, RING_R, 8.0, Some(pct), rc);
                let ctext = format!("{:.0}%", pct.round());
                let tw = font.measure(RING_FS, &ctext);
                let c = text_c(dark, rc);
                font.draw(
                    px,
                    cx - tw / 2.0,
                    cy + font.ascent(RING_FS) / 2.0,
                    RING_FS,
                    &ctext,
                    color(c.0, c.1, c.2, c.3),
                );
            }
        }
        Body::Bars { title, note, rows } => {
            font.draw_bold(px, x + CARD_PAD, y + 24.0, FONT_SIZE_TITLE, title, fg);
            if !note.is_empty() {
                let nw = font.measure(FONT_SIZE_SMALL, note);
                font.draw(
                    px,
                    x + w - CARD_PAD - nw,
                    y + 24.0 - NOTE_LIFT,
                    FONT_SIZE_SMALL,
                    note,
                    dim,
                );
            }
            for (i, row) in rows.iter().enumerate() {
                let ry = y + 28.0 + i as f32 * BAR_ROW_H;
                // 信息行：仅盘名（不显示温度与电源状态）
                let ib = ry + 17.0;
                font.draw(px, x + CARD_PAD, ib, FONT_SIZE_SMALL, &row.name, fg);
                // 用量横条
                let bx = x + CARD_PAD;
                let by = ry + 24.0;
                const BH: f32 = 10.0;
                let bw = w - CARD_PAD * 2.0;
                fill_round(px, bx, by, bw, BH, BH / 2.0, TRACK);
                if let Some(p) = row.ratio {
                    let pc = usage_color(p);
                    let fw = (bw * (p / 100.0).clamp(0.0, 1.0)).max(BH);
                    fill_round(px, bx, by, fw, BH, BH / 2.0, pc);
                }
                // 容量明细
                font.draw(px, x + CARD_PAD, ry + 48.0, FONT_SIZE_SMALL, &row.cap, dim);
            }
        }
        Body::Volumes { title, note, rows } => {
            font.draw_bold(px, x + CARD_PAD, y + 24.0, FONT_SIZE_TITLE, title, fg);
            if !note.is_empty() {
                let nw = font.measure(FONT_SIZE_SMALL, note);
                font.draw(
                    px,
                    x + w - CARD_PAD - nw,
                    y + 24.0 - NOTE_LIFT,
                    FONT_SIZE_SMALL,
                    note,
                    dim,
                );
            }
            let mut ry = y + 28.0;
            for row in rows {
                // 卷行：上半 卷名(左) + 容量文本(右)；下半 用量进度条
                let text_baseline = ry + 16.0;
                let right = x + w - CARD_PAD;
                let cap_w = font.measure(FONT_SIZE_SMALL, &row.cap);
                font.draw(
                    px,
                    right - cap_w,
                    text_baseline,
                    FONT_SIZE_SMALL,
                    &row.cap,
                    dim,
                );
                // 卷名超长时按剩余宽度截断，避免与容量文本重叠
                let name_max = right - cap_w - 8.0 - (x + CARD_PAD);
                let vname = fit_text(font, FONT_SIZE_SMALL, &row.name, name_max.max(20.0));
                font.draw(px, x + CARD_PAD, text_baseline, FONT_SIZE_SMALL, &vname, fg);
                // 用量进度条
                const PB_H: f32 = 6.0;
                let bx = x + CARD_PAD;
                let by = ry + 24.0;
                let bw = w - CARD_PAD * 2.0;
                fill_round(px, bx, by, bw, PB_H, PB_H / 2.0, TRACK);
                if let Some(p) = row.ratio {
                    let pc = usage_color(p);
                    let fw = (bw * (p / 100.0).clamp(0.0, 1.0)).max(PB_H);
                    fill_round(px, bx, by, fw, PB_H, PB_H / 2.0, pc);
                }
                ry += VOL_ROW_H;
            }
        }
        Body::Chart {
            title,
            note,
            pts,
            detail1,
            detail2,
            c1,
            c2,
        } => {
            font.draw_bold(px, x + CARD_PAD, y + 24.0, FONT_SIZE_TITLE, title, fg);
            // 右上角小字补充（如网络卡的链路速率）：与"存储状态 · 1 卷"同一套排版，
            // 不并进标题串，否则标题又长又重，比其它卡片标题"大"一截。
            if !note.is_empty() {
                let nw = font.measure(FONT_SIZE_SMALL, note);
                font.draw(
                    px,
                    x + w - CARD_PAD - nw,
                    y + 24.0 - NOTE_LIFT,
                    FONT_SIZE_SMALL,
                    note,
                    dim,
                );
            }
            if !pts.is_empty() {
                draw_chart(
                    px,
                    x + CARD_PAD,
                    y + 32.0,
                    w - CARD_PAD * 2.0,
                    h - 62.0,
                    pts,
                    *c1,
                    *c2,
                    dark,
                );
            }
            // 底部明细：左段用 c1 色，右段用 c2 色——**两个主题都跟线条同色**。
            // 暗色下不再改成白：彩色像素的反算映射会把半覆盖的笔画落成同色暗调，
            // 小字照样是彩色的（以前统一改白是因为压灰会把彩色小字花掉）。
            let by = y + h - 8.0;
            let c1c = color(c1.0, c1.1, c1.2, c1.3);
            let c2c = color(c2.0, c2.1, c2.2, c2.3);
            if !detail1.is_empty() {
                font.draw(px, x + CARD_PAD, by, FONT_SIZE_SMALL, detail1, c1c);
            }
            if !detail2.is_empty() {
                let w2 = font.measure(FONT_SIZE_SMALL, detail2);
                font.draw(px, x + w - CARD_PAD - w2, by, FONT_SIZE_SMALL, detail2, c2c);
            }
        }
    }
}

/// 画「NAS 名称（粗体）+ IP」标题：名称超宽按 avail 截断；缺名称只显 IP，都没有则 “NAS” 兜底。
/// 未固定时用在顶栏，固定到任务栏/竖排时用在面板标题行（排在“已连接”之前）；
/// 名称字号两处统一取 FONT_SIZE_HEADER，IP 取 FONT_SIZE_SMALL。
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_title(
    font: &Font,
    px: &mut Pixmap,
    x: f32,
    baseline: f32,
    avail: f32,
    sizes: (f32, f32),
    colors: (Color, Color),
    name: Option<&str>,
    ip: &str,
) {
    let (name_size, ip_size) = sizes;
    let (fg, dim) = colors;
    let gap = 8.0;
    match (name, ip.is_empty()) {
        (Some(n), false) => {
            let ip_w = font.measure(ip_size, ip);
            let name_max = (avail - ip_w - gap).max(30.0);
            let nlabel = fit_text_bold(font, name_size, n, name_max);
            let n_end = font.draw_bold(px, x, baseline, name_size, &nlabel, fg);
            font.draw(px, n_end + gap, baseline, ip_size, ip, dim);
        }
        (Some(n), true) => {
            let nlabel = fit_text_bold(font, name_size, n, avail);
            font.draw_bold(px, x, baseline, name_size, &nlabel, fg);
        }
        (None, false) => {
            let ilabel = fit_text(font, ip_size, ip, avail);
            font.draw(px, x, baseline, ip_size, &ilabel, dim);
        }
        (None, true) => {
            font.draw(px, x, baseline, ip_size, "NAS", dim);
        }
    }
}

/// 各卡片高度（与运行时数据无关，仅由设备数量与卡片开关决定）。
/// 顺序与 build_cards 完全一致（跟随设置里的勾选先后）。
/// 与 build_cards 必须用同一份开关，否则窗口高度与卡片内容会错位。
fn card_heights(s: &Snapshot, cards: CardVis) -> Vec<f32> {
    let mut v = Vec::new();
    let ng = s.gpus.as_deref().map_or(0, |g| g.len());
    let nn = s.network.as_deref().map_or(0, |n| n.len());
    let nd = s.disks.as_deref().map_or(0, |d| d.len());
    let nv = s.volumes.as_deref().map_or(0, |v| v.len());
    // 存储卡只按「容量读得到」的行数与行高算
    let (store_rows, store_row_h) = storage_rows_geom(s);

    for idx in cards.ordered() {
        match idx {
            0 => {
                if cpu_card_ok(s) {
                    v.push(CPU_CARD_H);
                }
            }
            1 => {
                if mem_card_ok(s) {
                    v.push(DUAL_GAUGE_H);
                }
            }
            // 显卡：每卡一张
            2 => v.extend(std::iter::repeat_n(DUAL_GAUGE_H, ng)),
            // 网卡心电图：每接口一张
            3 => v.extend(std::iter::repeat_n(CHART_H, nn)),
            4 => {
                if nd + nv > 0 {
                    v.push(CHART_H);
                }
            }
            // 存储卡（卷行优先，无卷时盘平铺）
            5 => {
                if store_rows > 0 {
                    v.push(BAR_BASE + store_rows as f32 * store_row_h);
                }
            }
            _ => {}
        }
    }
    v
}
