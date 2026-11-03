//! 超大卡片（贴屏幕侧边滑出的 660 宽面板）：滑动渲染入口、区块绘制辅助与历史折线换算。

use super::*;

impl CapsuleView {
    /// 超大卡片：窗口几何一次到位，内容从贴边一侧滑入（只改 blit 源/目标偏移）。
    /// 标题行固定在大卡顶部，不随滑入平移。
    pub(super) fn render_big(
        &mut self,
        px: &mut Pixmap,
        frame: &Frame,
        font: Option<&Font>,
        p: f32,
        vis_w: u32,
        vis_h: u32,
    ) -> bool {
        let (bw, bh, left) = self.big_geom;
        let bw = bw.max(PANEL_W);
        let bh = bh.max(320);

        let key = (
            frame.seq,
            frame.conn.clone(),
            bh,
            false,
            frame.cards,
            frame.dark,
            true,
            bw,
        );
        let up_to_date = self.current.as_ref().is_some_and(|c| {
            c.seq == frame.seq
                && c.conn == *frame.conn
                && c.ph == bh
                && c.big
                && c.w == bw
                && c.cards == frame.cards
                && c.dark == frame.dark
        });
        if !up_to_date && self.sent_key.as_ref() != Some(&key) {
            let _ = self.req_tx.send(PanelJob::Build(BuildReq {
                seq: frame.seq,
                conn: frame.conn.clone(),
                ph: bh,
                big: true,
                w: bw,
                has_data: frame.has_data,
                dark: frame.dark,
                header_bottom: false,
                cards: frame.cards,
                snap: frame.arc.clone(),
            }));
            self.sent_key = Some(key);
            // 稳态且动画已结束：等后台构建完成（WM_APP_PANEL）再整帧上屏
            if p >= 0.999 {
                return false;
            }
        }

        // 滑入：贴右侧时内容从右缘向左推入，贴左侧时从左缘向右推入
        let ease = if p >= 0.999 { 1.0 } else { ease_motion(p) };
        let shift = ((1.0 - ease) * bw as f32).round() as u32;
        let (dx, sx, sw) = if left {
            (0, shift, bw - shift)
        } else {
            (shift as i32, 0, bw - shift)
        };
        let bh = bh.min(vis_h.max(1));
        let sw = sw.min(vis_w.max(1));
        if sw > 0 {
            if let Some(c) = &self.current {
                blit_rect(px, c.pm.as_ref(), dx, 0, sx, 0, sw, bh);
            }
        }
        let mut swapped = false;
        while let Ok(panel) = self.panel_rx.try_recv() {
            if let Some(old) = self.current.replace(panel) {
                let _ = self.req_tx.send(PanelJob::Recycle(old.pm));
            }
            swapped = true;
        }
        if swapped && sw > 0 {
            if let Some(c) = &self.current {
                blit_rect(px, c.pm.as_ref(), dx, 0, sx, 0, sw, bh);
            }
        }

        if let Some(font) = font {
            let row_cy = BIG_PAD + BIG_HEADER_H / 2.0;
            let baseline = row_cy + font.ascent(FONT_SIZE_SMALL) * 0.36;
            let gx = bw as f32 - BIG_PAD - GEAR_SIZE;
            let gy = row_cy - GEAR_SIZE / 2.0;
            self.gear_pos = (gx, gy);

            let (text, sc, dc) = conn_badge(frame.conn, frame.dark);
            let tc = if frame.dark {
                color(FG.0, FG.1, FG.2, FG.3)
            } else {
                sc
            };
            let tw = font.measure(FONT_SIZE_SMALL, &text);
            let status_right = gx - 12.0;
            font.draw(px, status_right - tw, baseline, FONT_SIZE_SMALL, &text, tc);
            let dot_r = 3.0;
            let dot_cx = status_right - tw - 9.0 - dot_r;
            fill_dot(px, dot_cx, row_cy, dot_r, dc);

            let name = frame
                .snap
                .hostname
                .as_deref()
                .map(str::trim)
                .filter(|h| !h.is_empty());
            let avail = (dot_cx - dot_r - 12.0 - BIG_PAD).max(80.0);
            let d = text_c(frame.dark, FG_DIM);
            draw_title(
                font,
                px,
                BIG_PAD,
                baseline,
                avail,
                (FONT_SIZE_HEADER, FONT_SIZE_SMALL),
                (color(FG.0, FG.1, FG.2, FG.3), color(d.0, d.1, d.2, d.3)),
                name,
                frame.nas_host.trim(),
            );

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

            // 网络卡柱图信息栏：未悬停显示「今日 / 30 日」合计，悬停某根柱子显示该日数据
            if let Some(geom) = self.current.as_ref().and_then(|c| c.bars) {
                if let Some(tf) = frame.snap.traffic.as_ref() {
                    let idx = self.hover_bar.filter(|i| *i < geom.count);
                    let d = text_c(frame.dark, FG_DIM);
                    let dc = color(d.0, d.1, d.2, d.3);
                    if let Some(i) = idx {
                        // 悬停柱高亮：整列铺一层浅色底（不遮相邻柱），再描一圈强调色边框
                        const GAP: f32 = 1.5;
                        let bw = ((geom.w - GAP * (geom.count as f32 - 1.0)) / geom.count as f32)
                            .max(1.2);
                        let bx = geom.x + i as f32 * (bw + GAP);
                        let mut wash = Paint::default();
                        wash.set_color_rgba8(ACCENT.0, ACCENT.1, ACCENT.2, 26);
                        if let Some(r) =
                            Rect::from_xywh(bx - 1.0, geom.y - 2.0, bw + 2.0, geom.h + 2.0)
                        {
                            px.fill_rect(r, &wash, Transform::identity(), None);
                        }
                        let outline =
                            rounded_rect(bx - 1.5, geom.y - 2.0, bw + 3.0, geom.h + 3.0, 1.5);
                        let mut paint = Paint::default();
                        paint.set_color_rgba8(ACCENT.0, ACCENT.1, ACCENT.2, 255);
                        paint.anti_alias = true;
                        let stroke = Stroke {
                            width: 1.5,
                            ..Default::default()
                        };
                        px.stroke_path(&outline, &paint, &stroke, Transform::identity(), None);
                    }
                    let text = match idx {
                        Some(i) => {
                            let date = tf
                                .dates
                                .get(i)
                                .map(|s| fmt_date_cn(s))
                                .filter(|s| !s.is_empty())
                                .unwrap_or_else(|| format!("第 {} 天", i + 1));
                            let rx = tf.days_rx.get(i).copied().unwrap_or(0);
                            let tx = tf.days_tx.get(i).copied().unwrap_or(0);
                            format!(
                                "{date}  下行 {} · 上行 {}",
                                fmt::bytes_unit(rx),
                                fmt::bytes_unit(tx)
                            )
                        }
                        None => format!(
                            "今日 {} / {} · 30 日 {} / {}",
                            fmt::bytes_unit(tf.rx_today),
                            fmt::bytes_unit(tf.tx_today),
                            fmt::bytes_unit(tf.rx_days30),
                            fmt::bytes_unit(tf.tx_days30)
                        ),
                    };
                    font.draw(px, geom.x, geom.line_y, 9.5, &text, dc);
                }
            }
        }
        true
    }
}

/// 大卡左右两栏的竖分隔线（从标题行下方到卡片底部）。
pub(super) fn draw_big_divider(pm: &mut Pixmap, x: f32, y: f32, h: f32) {
    fill_round(pm, x, y, 1.0, h, 0.5, CARD_LINE);
}

/// 大卡右栏的 2×2 小卡片网格：每张小卡一项「竖排标签 + 大号数值」
/// （CPU：平均负载 1 / 5 / 15 / 30 分钟，内存：总计 / 已用 / 缓存 / 可用）。
/// 标签竖排只占一条窄列，横向空间留给数值，数值因此能放大到 17px。
/// 读不到的项不会传进来，只剩几项就排几项，空位不留灰底。
pub(super) fn draw_big_kv_grid(
    pm: &mut Pixmap,
    font: &Font,
    x: f32,
    y: f32,
    w: f32,
    max_h: f32,
    items: &[(String, String)],
    fg: Color,
    dim: Color,
) {
    const GAP: f32 = 12.0;
    const COLS: usize = 2;
    const PAD: f32 = 10.0;
    // 竖排标签：一字一行（空格丢弃），列宽 12、行距 12
    const LBL_FS: f32 = 10.0;
    const LBL_LH: f32 = 12.0;
    const LBL_COL: f32 = 12.0;
    // 数值：标签列右侧，字号放大
    const VAL_FS: f32 = 17.0;
    if items.is_empty() {
        return;
    }
    let rows = (items.len() + COLS - 1) / COLS;
    let cw = (w - GAP * (COLS as f32 - 1.0)) / COLS as f32;
    let ch = ((max_h - GAP * (rows as f32 - 1.0)) / rows as f32).min(58.0);
    if ch < 34.0 || cw < 60.0 {
        return;
    }
    for (i, (label, value)) in items.iter().enumerate() {
        let cx = x + (i % COLS) as f32 * (cw + GAP);
        let cy = y + (i / COLS) as f32 * (ch + GAP);
        fill_round(pm, cx, cy, cw, ch, 10.0, BIG_SUB_BG);
        // 竖排标签：整块在卡片内垂直居中
        let chars: Vec<char> = label.chars().filter(|c| !c.is_whitespace()).collect();
        let block_top = cy + (ch - chars.len() as f32 * LBL_LH) / 2.0;
        for (k, c) in chars.iter().enumerate() {
            let s = c.to_string();
            let tw = font.measure(LBL_FS, &s);
            font.draw(
                pm,
                cx + PAD + (LBL_COL - tw) / 2.0,
                block_top + k as f32 * LBL_LH + LBL_LH / 2.0 + font.ascent(LBL_FS) / 2.0,
                LBL_FS,
                &s,
                dim,
            );
        }
        // 数值：标签列右侧、垂直居中
        let vx = cx + PAD + LBL_COL + 4.0;
        let vw = (cx + cw - PAD - vx).max(20.0);
        let v = fit_text_bold(font, VAL_FS, value, vw);
        font.draw_bold(
            pm,
            vx,
            cy + ch / 2.0 + font.ascent(VAL_FS) / 2.0,
            VAL_FS,
            &v,
            fg,
        );
    }
}

/// 大卡右栏的 30 日堆叠柱状图：下半蓝色为下行、上半橙色为上行，按当前最大值归一。
pub(super) fn draw_big_bars(
    pm: &mut Pixmap,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    rx: &[u64],
    tx: &[u64],
) {
    let n = rx.len().min(tx.len()).min(30);
    if n < 2 || h < 18.0 {
        return;
    }
    let mut max = 0u64;
    for i in 0..n {
        max = max.max(rx[i].saturating_add(tx[i]));
    }
    if max == 0 {
        return;
    }
    const GAP: f32 = 1.5;
    let bw = ((w - GAP * (n as f32 - 1.0)) / n as f32).max(1.2);
    let base = y + h;
    for i in 0..n {
        let total = rx[i].saturating_add(tx[i]) as f32;
        let filled = h * (total / max as f32);
        let tx_ratio = if total > 0.0 {
            tx[i] as f32 / total
        } else {
            0.0
        };
        let h_tx = filled * tx_ratio;
        let h_rx = filled - h_tx;
        let bx = x + i as f32 * (bw + GAP);
        if h_rx > 0.4 {
            // 下行：蓝
            fill_round(pm, bx, base - h_rx, bw, h_rx, 0.5, ACCENT);
        }
        if h_tx > 0.4 {
            // 上行：橙
            fill_round(pm, bx, base - h_rx - h_tx, bw, h_tx, 0.5, TX_LINE);
        }
    }
}

/// 超大卡片面板总高：**恒定按 6 个卡位算**（与实际勾选几张、读得到几张无关），
/// 面板大小因此不会随内张小卡片增减而变化，用不满的卡位就留白。
pub(crate) fn big_panel_height() -> u32 {
    let slots = crate::config::CARD_COUNT as f32;
    let h = BIG_PAD * 2.0 + BIG_HEADER_H + slots * BIG_CARD_H + (slots - 1.0) * BIG_BLOCK_GAP;
    h.ceil() as u32
}

/// 大卡内一块图表的卡片底 + 1px 描边。
pub(super) fn draw_big_card(pm: &mut Pixmap, clip: &mut Mask, x: f32, y: f32, w: f32, h: f32) {
    fill_round_opaque(pm, clip, x, y, w, h, 10.0, BIG_CARD_BG);
    stroke_card(pm, x, y, w, h);
}

/// 大卡区块标题：左上标题（粗体）、右上补充说明。
pub(super) fn draw_big_title(
    pm: &mut Pixmap,
    font: &Font,
    x: f32,
    w: f32,
    y: f32,
    title: &str,
    note: &str,
    fg: Color,
    dim: Color,
) {
    font.draw_bold(pm, x, y + 15.0, FONT_SIZE_TITLE + 0.5, title, fg);
    if !note.is_empty() {
        let nw = font.measure(FONT_SIZE_SMALL, note);
        font.draw(pm, x + w - nw, y + 15.0, FONT_SIZE_SMALL, note, dim);
    }
}

/// 大卡图表网格：5 条横线 + 左侧 3 个刻度（上 / 中 / 下）。
pub(super) fn draw_big_grid(
    pm: &mut Pixmap,
    font: &Font,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    labels: &[String; 3],
    dim: Color,
) {
    for i in 0..=4 {
        let gy = y + h * i as f32 / 4.0;
        fill_round(pm, x, gy, w, 1.0, 0.0, (231, 235, 242, 255));
    }
    for (i, lab) in labels.iter().enumerate() {
        if lab.is_empty() {
            continue;
        }
        let ly = y + h * [0.0f32, 0.5, 1.0][i];
        let lw = font.measure(9.0, lab);
        let bl = match i {
            0 => ly + 8.0,
            1 => ly + 3.0,
            _ => ly - 2.0,
        };
        font.draw(pm, x - 6.0 - lw, bl, 9.0, lab, dim);
    }
}

/// 大卡曲线：面积填充（可选）+ 折线。`pts` 为 (x 0..1, y 0..1，1=底)。
pub(super) fn draw_big_series(
    pm: &mut Pixmap,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    pts: &[(f32, f32)],
    c: (u8, u8, u8, u8),
    fill: bool,
) {
    if pts.len() < 2 {
        return;
    }
    let px_at = |p: &(f32, f32)| (x + p.0.clamp(0.0, 1.0) * w, y + p.1.clamp(0.0, 1.0) * h);
    if fill {
        let mut pb = PathBuilder::new();
        let (fx, fy) = px_at(&pts[0]);
        pb.move_to(fx, y + h);
        pb.line_to(fx, fy);
        for p in &pts[1..] {
            let (vx, vy) = px_at(p);
            pb.line_to(vx, vy);
        }
        let (lx, _) = px_at(&pts[pts.len() - 1]);
        pb.line_to(lx, y + h);
        pb.close();
        if let Some(path) = pb.finish() {
            let mut paint = Paint::default();
            paint.set_color_rgba8(c.0, c.1, c.2, (c.3 / 7).max(10));
            paint.anti_alias = true;
            pm.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }
    let mut pb = PathBuilder::new();
    let (fx, fy) = px_at(&pts[0]);
    pb.move_to(fx, fy);
    for p in &pts[1..] {
        let (vx, vy) = px_at(p);
        pb.line_to(vx, vy);
    }
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color_rgba8(c.0, c.1, c.2, c.3);
        paint.anti_alias = true;
        let stroke = Stroke {
            width: 1.6,
            ..Default::default()
        };
        pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

/// 大卡底部统计：竖色条 + 小标签 + 大号数值（暗色主题下数值随 apply_dark 提白）。
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_big_stat(
    pm: &mut Pixmap,
    font: &Font,
    x: f32,
    y: f32,
    c: (u8, u8, u8, u8),
    label: &str,
    value: &str,
    value_fs: f32,
    dark: bool,
    dim: Color,
) {
    fill_round(pm, x, y + 2.0, 2.5, 34.0, 1.25, c);
    font.draw(pm, x + 10.0, y + 14.0, FONT_SIZE_SMALL - 0.5, label, dim);
    let vc = text_c(dark, c);
    font.draw_bold(
        pm,
        x + 10.0,
        y + 38.0,
        value_fs,
        value,
        color(vc.0, vc.1, vc.2, vc.3),
    );
}

/// CPU 历史 -> 归一化折线（`temp` 选择温度序列；温度缺省点跳过）。
pub(super) fn hist_ratio_pts(hist: &VecDeque<(f32, f32)>, temp: bool) -> Vec<(f32, f32)> {
    let n = hist.len();
    if n < 2 {
        return Vec::new();
    }
    hist.iter()
        .enumerate()
        .filter_map(|(i, &(u, t))| {
            let v = if temp { t } else { u };
            if v < 0.0 {
                None
            } else {
                Some((i as f32 / (n - 1) as f32, 1.0 - (v / 100.0).clamp(0.0, 1.0)))
            }
        })
        .collect()
}

/// 内存使用率历史 -> 归一化折线。
pub(super) fn mem_ratio_pts(hist: &VecDeque<f32>) -> Vec<(f32, f32)> {
    let n = hist.len();
    if n < 2 {
        return Vec::new();
    }
    hist.iter()
        .enumerate()
        .filter_map(|(i, &v)| {
            if v < 0.0 {
                None
            } else {
                Some((i as f32 / (n - 1) as f32, 1.0 - (v / 100.0).clamp(0.0, 1.0)))
            }
        })
        .collect()
}

/// 速率历史的最大值（与 hist_points 一致，含 1.2 头部余量）。
pub(super) fn hist_max(q: &VecDeque<(u64, u64)>) -> f64 {
    q.iter()
        .fold(1.0f64, |m, e| m.max(e.0 as f64).max(e.1 as f64))
        * 1.2
}

/// 百分比图表刻度（0-100）。
pub(super) fn percent_grid_labels() -> [String; 3] {
    ["100".to_string(), "50".to_string(), "0".to_string()]
}

/// 速率图表的三个刻度：上 / 中 / 0。
pub(super) fn rate_grid_labels(max: f64) -> [String; 3] {
    [
        fmt::rate(max as u64),
        fmt::rate((max / 2.0) as u64),
        "0".to_string(),
    ]
}
