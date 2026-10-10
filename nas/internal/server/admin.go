package server

import (
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"hetao-nas/internal/metrics"
)

// AdminDeps 管理页依赖。管理页只挂在飞牛统一网关的 unix socket 上，
// 由网关校验飞牛登录态，因此自身不再做令牌鉴权（局域网 TCP 端口不暴露本页面）。
type AdminDeps struct {
	Col      *metrics.Collector
	Tok      *TokenStore
	Version  string
	Hostname string
	LogPath  string
	StateDir string // 持久化网卡选择（iface 文件）
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
	mux.HandleFunc("/api/iface", a.handleIface)
	mux.HandleFunc("/api/token/reset", a.handleTokenReset)
	mux.HandleFunc("/api/logs", a.handleLogs)
	mux.HandleFunc("/api/icon", a.handleIcon)
	mux.HandleFunc("/api/download", a.handleDownload)
	return stripPrefix(prefix, mux)
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
	_, _ = w.Write([]byte(indexHTML))
}

func (a *admin) handleState(w http.ResponseWriter, _ *http.Request) {
	writeJSON(w, map[string]any{
		"hostname":         a.Hostname,
		"version":          a.Version,
		"uptime_sec":       int64(time.Since(a.Start).Seconds()),
		"token":            a.Tok.Get(),
		"iface":            a.Col.NetIface(),          // 手动指定（空=自动）
		"iface_effective":  a.Col.NetIfaceEffective(), // 实际生效
		"ifaces":           a.Col.NetIfaces(),
		"service_port":     8765,
		"client_version":   a.ClientVersion,
		"client_size":      a.ClientSize,
		"client_sha256":    a.ClientSHA256,
		"client_available": a.ClientSize > 0,
	})
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
	w.Header().Set("Cache-Control", "public, max-age=86400")
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
