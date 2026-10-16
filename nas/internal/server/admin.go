package server

import (
	"encoding/json"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"hetao-nas/internal/metrics"
)

// AdminDeps 管理页依赖。管理页只挂在飞牛统一网关的 unix socket 上，
// 登录态由网关校验；管理类接口再由 requireAdmin 校验网关转发的管理员身份，
// 只读接口不回显任何密钥（局域网 TCP 端口不暴露本页面）。
type AdminDeps struct {
	Col      *metrics.Collector
	Tok      *TokenStore
	Version  string
	Hostname string
	LogPath  string
	StateDir string // 持久化网卡选择与手动协商速率（iface / netspeed 文件）
	IconPath string // 管理页品牌图标（app/ui/images/icon_64.png）
	// 电脑端安装器信息（随 fpk 一起下发，供管理页下载）
	SetupPath     string // 安装器文件路径（app/ui/download/hetao-capsule-setup.exe）
	ClientVersion string // 安装器版本号
	ClientSize    int64  // 安装器字节数（0 表示缺失）
	ClientSHA256  string // 安装器 SHA256 十六进制
	Start         time.Time
}

type admin struct{ AdminDeps }

// NewAdmin 返回管理页 handler；prefix 为网关前缀（如 /app/hetaocapsule），
// 网关转发时保留完整前缀，这里统一剥离后交给内部路由。
func NewAdmin(prefix string, d AdminDeps) http.Handler {
	a := &admin{d}
	mux := http.NewServeMux()
	mux.HandleFunc("/", a.handleIndex)
	mux.HandleFunc("/api/state", a.handleState)
	mux.HandleFunc("/api/usage", a.handleUsage)
	// 管理类接口要求飞牛网关注入的管理员身份（X-Trim-Isadmin）。
	// 网关只负责登录态与业务权限的区分交给应用（见文档「管理接口需要管理员身份」）。
	mux.HandleFunc("/api/iface", requireAdmin(a.handleIface))
	mux.HandleFunc("/api/netspeed", requireAdmin(a.handleNetSpeed))
	mux.HandleFunc("/api/token", requireAdmin(a.handleTokenPlain))
	mux.HandleFunc("/api/token/reset", requireAdmin(a.handleTokenReset))
	mux.HandleFunc("/api/logs", requireAdmin(a.handleLogs))
	mux.HandleFunc("/api/icon", a.handleIcon)
	mux.HandleFunc("/api/download", a.handleDownload)
	return stripPrefix(prefix, mux)
}

// requireAdmin 校验网关转发的管理员身份；非管理员一律 403。
// 只读展示类接口（/、/api/state、/api/usage、图标与安装包）不在此列，且都不含密钥。
func requireAdmin(next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		if strings.EqualFold(strings.TrimSpace(r.Header.Get("X-Trim-Isadmin")), "true") {
			next(w, r)
			return
		}
		log.Printf("admin required denied: path=%s isadmin=%q uid=%q",
			r.URL.Path, r.Header.Get("X-Trim-Isadmin"), r.Header.Get("X-Trim-Userid"))
		http.Error(w, "forbidden: admin required", http.StatusForbidden)
	}
}

// MaskToken 只保留令牌前 4 位，其余打码。用于日志与管理页默认展示，避免明文密钥外泄。
func MaskToken(v string) string {
	if v == "" {
		return ""
	}
	if len(v) <= 4 {
		return "••••"
	}
	return v[:4] + "••••••••"
}

func stripPrefix(prefix string, h http.Handler) http.Handler {
	if prefix == "" {
		return h
	}
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if strings.HasPrefix(r.URL.Path, prefix) {
			p := strings.TrimPrefix(r.URL.Path, prefix)
			if p == "" {
				p = "/"
			}
			r.URL.Path = p
		}
		h.ServeHTTP(w, r)
	})
}

func (a *admin) handleIndex(w http.ResponseWriter, _ *http.Request) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	html := indexHTML
	// 品牌图标 URL 带上版本参数：图标自身可长期缓存，升级后 URL 变化即可自动取新图
	// （否则浏览器会按 max-age 继续用旧图标，表现为“换了图标但管理页没变”）
	if v := iconVersion(a.Version); v != "" {
		html = strings.Replace(html, `src="api/icon"`, `src="api/icon?v=`+v+`"`, 1)
	}
	_, _ = w.Write([]byte(html))
}

// iconVersion 只保留 URL 安全字符，避免把环境变量值直接拼进 HTML/URL。
func iconVersion(v string) string {
	return strings.Map(func(r rune) rune {
		switch {
		case r >= '0' && r <= '9', r >= 'A' && r <= 'Z', r >= 'a' && r <= 'z',
			r == '.', r == '-', r == '_':
			return r
		default:
			return -1
		}
	}, v)
}

func (a *admin) handleState(w http.ResponseWriter, _ *http.Request) {
	writeJSON(w, map[string]any{
		"hostname":         a.Hostname,
		"version":          a.Version,
		"uptime_sec":       int64(time.Since(a.Start).Seconds()),
		"token_masked":     MaskToken(a.Tok.Get()),    // 完整令牌只经管理员接口 /api/token 下发
		"iface":            a.Col.NetIface(),          // 手动指定（空=自动）
		"iface_effective":  a.Col.NetIfaceEffective(), // 实际生效
		"ifaces":           a.Col.NetIfaces(),
		"net_speeds":       a.Col.NetSpeeds(),         // 手动填写的协商速率（网卡名 → Mbps）
		"speed_effective":  a.Col.NetSpeedEffective(), // 生效网卡的协商速率（0=未知）
		"service_port":     8765,
		"client_version":   a.ClientVersion,
		"client_size":      a.ClientSize,
		"client_sha256":    a.ClientSHA256,
		"client_available": a.ClientSize > 0,
	})
}

// handleTokenPlain 返回完整配对令牌，供管理页「显示/复制」（仅管理员可访问）。
func (a *admin) handleTokenPlain(w http.ResponseWriter, _ *http.Request) {
	writeJSON(w, map[string]any{"token": a.Tok.Get()})
}

// handleUsage 返回本应用进程自身的 CPU 与内存占用，供管理页顶栏展示。
func (a *admin) handleUsage(w http.ResponseWriter, _ *http.Request) {
	cpu, mem := metrics.SelfUsage()
	writeJSON(w, map[string]any{"cpu_percent": cpu, "mem_bytes": mem})
}

type ifaceReq struct {
	Name string `json:"name"`
}

func (a *admin) handleIface(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	var req ifaceReq
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, "bad json", http.StatusBadRequest)
		return
	}
	name := strings.TrimSpace(req.Name)
	// 仅接受已发现的网卡名（或空串=自动），避免写入任意值。
	if name != "" {
		ok := false
		for _, n := range a.Col.NetIfaces() {
			if n == name {
				ok = true
				break
			}
		}
		if !ok {
			http.Error(w, "unknown interface", http.StatusBadRequest)
			return
		}
	}
	a.Col.SetNetIface(name)
	if a.StateDir != "" {
		_ = os.WriteFile(filepath.Join(a.StateDir, "iface"), []byte(name), 0600)
	}
	writeJSON(w, map[string]any{"ok": true, "iface": name})
}

// maxSpeedMbps 手动协商速率上限（400 Gbps），防止写入明显不合理的值。
const maxSpeedMbps = 400_000

type netSpeedReq struct {
	Name string `json:"name"`
	Mbps int    `json:"mbps"`
}

// handleNetSpeed 手动设置某块网卡的协商速率（Mbps）。WiFi 网卡不导出
// /sys/class/net/*/speed，无法自动识别，需管理员按实际链路填写；
// 客户端据此计算网络环占比。mbps=0 表示清除该网卡的手动值、恢复自动读取。
func (a *admin) handleNetSpeed(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	var req netSpeedReq
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, "bad json", http.StatusBadRequest)
		return
	}
	name := strings.TrimSpace(req.Name)
	if name == "" {
		http.Error(w, "interface required", http.StatusBadRequest)
		return
	}
	// 仅接受已发现的网卡名，避免把任意键写进配置文件。
	ok := false
	for _, n := range a.Col.NetIfaces() {
		if n == name {
			ok = true
			break
		}
	}
	if !ok {
		http.Error(w, "unknown interface", http.StatusBadRequest)
		return
	}
	if req.Mbps < 0 || req.Mbps > maxSpeedMbps {
		http.Error(w, "invalid speed", http.StatusBadRequest)
		return
	}
	a.Col.SetNetSpeed(name, req.Mbps)
	if a.StateDir != "" {
		if b, err := json.Marshal(a.Col.NetSpeeds()); err == nil {
			_ = os.WriteFile(filepath.Join(a.StateDir, "netspeed"), b, 0600)
		}
	}
	writeJSON(w, map[string]any{"ok": true, "name": name, "mbps": req.Mbps})
}

func (a *admin) handleTokenReset(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	tok, err := a.Tok.Reset()
	if err != nil {
		http.Error(w, "reset failed: "+err.Error(), http.StatusInternalServerError)
		return
	}
	writeJSON(w, map[string]any{"ok": true, "token": tok})
}

func (a *admin) handleLogs(w http.ResponseWriter, r *http.Request) {
	n := 200
	if v := r.URL.Query().Get("lines"); v != "" {
		if p, err := strconv.Atoi(v); err == nil && p > 0 && p <= 2000 {
			n = p
		}
	}
	w.Header().Set("Content-Type", "text/plain; charset=utf-8")
	if a.LogPath == "" {
		_, _ = w.Write([]byte("(未配置日志路径)"))
		return
	}
	b, err := os.ReadFile(a.LogPath)
	if err != nil {
		_, _ = w.Write([]byte("(读取日志失败: " + err.Error() + ")"))
		return
	}
	lines := strings.Split(strings.TrimRight(string(b), "\n"), "\n")
	if len(lines) > n {
		lines = lines[len(lines)-n:]
	}
	_, _ = w.Write([]byte(strings.Join(lines, "\n")))
}

// handleIcon 返回管理页品牌图标（64x64 PNG）。
func (a *admin) handleIcon(w http.ResponseWriter, _ *http.Request) {
	b, err := os.ReadFile(a.IconPath)
	if err != nil {
		http.NotFound(w, nil)
		return
	}
	w.Header().Set("Content-Type", "image/png")
	// URL 带版本参数（见 handleIndex），同一版本内可长期缓存
	w.Header().Set("Cache-Control", "public, max-age=31536000, immutable")
	_, _ = w.Write(b)
}

// handleDownload 下发电脑端安装器（浏览器直接下载）。
func (a *admin) handleDownload(w http.ResponseWriter, r *http.Request) {
	if a.SetupPath == "" {
		http.NotFound(w, r)
		return
	}
	f, err := os.Open(a.SetupPath)
	if err != nil {
		http.NotFound(w, r)
		return
	}
	defer f.Close()
	st, err := f.Stat()
	if err != nil {
		http.Error(w, "stat failed", http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "application/octet-stream")
	w.Header().Set("Content-Disposition", `attachment; filename="hetao-capsule-setup.exe"`)
	http.ServeContent(w, r, "hetao-capsule-setup.exe", st.ModTime(), f)
}
