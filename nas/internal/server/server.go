package server

import (
	"crypto/subtle"
	"encoding/json"
	"log"
	"net/http"
	"strings"
	"sync"
	"time"

	"github.com/gorilla/websocket"

	"hetao-nas/internal/metrics"
)

type Server struct {
	col      *metrics.Collector
	tok      *TokenStore
	upgrader websocket.Upgrader
}

func New(col *metrics.Collector, tok *TokenStore) *Server {
	return &Server{
		col: col,
		tok: tok,
		upgrader: websocket.Upgrader{
			ReadBufferSize:  1024,
			WriteBufferSize: 4096,
			CheckOrigin:     func(*http.Request) bool { return true }, // 局域网工具
		},
	}
}

func (s *Server) Handler() http.Handler {
	mux := http.NewServeMux()
	// 对外 API 一律要求配对令牌：不允许任何免鉴权访问（含健康检查）。
	mux.HandleFunc("/health", s.auth(s.handleHealth))
	mux.HandleFunc("/metrics", s.auth(s.handleMetrics))
	mux.HandleFunc("/ws", s.handleWS)
	return mux
}

// auth 要求配对令牌：Authorization: Bearer <token> 或 ?token=<token>。
// 令牌仅在局域网链路使用，常量时间比较避免时序侧信道。
func (s *Server) auth(next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		if !s.validToken(r) {
			w.Header().Set("WWW-Authenticate", `Bearer realm="hetao-nas"`)
			http.Error(w, "unauthorized", http.StatusUnauthorized)
			return
		}
		next(w, r)
	}
}

func (s *Server) validToken(r *http.Request) bool {
	token := s.tok.Get()
	if token == "" {
		return false // 未配置令牌一律拒绝（正常不会发生，main 保证令牌非空）
	}
	provided := r.URL.Query().Get("token")
	if h := r.Header.Get("Authorization"); strings.HasPrefix(h, "Bearer ") {
		provided = strings.TrimPrefix(h, "Bearer ")
	}
	return subtle.ConstantTimeCompare([]byte(provided), []byte(token)) == 1
}

func (s *Server) handleHealth(w http.ResponseWriter, _ *http.Request) {
	writeJSON(w, map[string]any{"status": "ok", "timestamp": time.Now().UnixMilli()})
}

func (s *Server) handleMetrics(w http.ResponseWriter, _ *http.Request) {
	writeJSON(w, s.col.Snapshot())
}

func writeJSON(w http.ResponseWriter, v any) {
	w.Header().Set("Content-Type", "application/json")
	if err := json.NewEncoder(w).Encode(v); err != nil {
		log.Printf("write json: %v", err)
	}
}

// ---- WebSocket 订阅推送 ----

type subscribeMsg struct {
	Type     string   `json:"type"`
	Metrics  []string `json:"metrics"`
	Interval int      `json:"interval"` // 毫秒
}

// subscription 单连接订阅状态，readPump 写、writePump 读。
type subscription struct {
	mu       sync.Mutex
	prefixes map[string]bool // nil = 全量
	interval time.Duration
	changed  chan struct{}
}

func newSubscription() *subscription {
	return &subscription{interval: time.Second, changed: make(chan struct{}, 1)}
}

func (sub *subscription) update(m subscribeMsg) {
	sub.mu.Lock()
	if len(m.Metrics) == 0 {
		sub.prefixes = nil
	} else {
		sub.prefixes = map[string]bool{}
		for _, k := range m.Metrics {
			if p, ok := metricPrefix(k); ok {
				sub.prefixes[p] = true
			}
		}
	}
	if m.Interval >= 200 && m.Interval <= 60000 {
		sub.interval = time.Duration(m.Interval) * time.Millisecond
	}
	sub.mu.Unlock()
	select {
	case sub.changed <- struct{}{}:
	default:
	}
}

func (sub *subscription) get() (map[string]bool, time.Duration) {
	sub.mu.Lock()
	defer sub.mu.Unlock()
	return sub.prefixes, sub.interval
}

// metricPrefix 将 "cpu.usage" 这类键映射到快照分区。
func metricPrefix(key string) (string, bool) {
	head, _, _ := strings.Cut(key, ".")
	switch head {
	case "cpu", "gpu", "mem", "disk", "net":
		return head, true
	default:
		return "", false
	}
}

func (s *Server) handleWS(w http.ResponseWriter, r *http.Request) {
	// WS 子协议难以携带标准鉴权头，统一用 ?token=<配对令牌>；
	// 校验失败直接 401，不升级、不暴露任何内部状态。
	// 注意：令牌为空时也必须拒绝（否则空 token 与空明文相等会误判为通过）。
	token := s.tok.Get()
	if token == "" || subtle.ConstantTimeCompare([]byte(r.URL.Query().Get("token")), []byte(token)) != 1 {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	conn, err := s.upgrader.Upgrade(w, r, nil)
	if err != nil {
		log.Printf("ws upgrade: %v", err)
		return
	}
	defer conn.Close()

	sub := newSubscription()
	done := make(chan struct{})

	go s.wsWritePump(conn, sub, done)
	s.wsReadPump(conn, sub, done)
}

func (s *Server) wsReadPump(conn *websocket.Conn, sub *subscription, done chan<- struct{}) {
	defer close(done)
	conn.SetReadLimit(4096)
	for {
		var m subscribeMsg
		if err := conn.ReadJSON(&m); err != nil {
			return // 对端关闭或协议错误
		}
		if m.Type == "subscribe" {
			sub.update(m)
		}
	}
}

func (s *Server) wsWritePump(conn *websocket.Conn, sub *subscription, done <-chan struct{}) {
	_, interval := sub.get()
	t := time.NewTicker(interval)
	defer t.Stop()
	// 令牌重置时立即断开本连接：WS 只在握手时校验令牌，不主动踢会一直收推送。
	// 客户端随后重连会拿到 401，进入“令牌错误”并停止重连。
	tokenChanged := s.tok.Changed()
	for {
		select {
		case <-done:
			return
		case <-tokenChanged:
			log.Printf("ws: 令牌已重置，断开旧会话")
			_ = conn.WriteControl(websocket.CloseMessage,
				websocket.FormatCloseMessage(websocket.ClosePolicyViolation, "token rotated"),
				time.Now().Add(time.Second))
			_ = conn.Close() // 让阻塞在 ReadJSON 的 readPump 立即返回
			return
		case <-sub.changed:
			_, interval = sub.get()
			t.Reset(interval)
		case <-t.C:
			prefixes, _ := sub.get()
			snap := s.col.Snapshot()
			payload := buildPayload(&snap, prefixes)
			conn.SetWriteDeadline(time.Now().Add(5 * time.Second))
			if err := conn.WriteJSON(payload); err != nil {
				return
			}
		}
	}
}

// buildPayload 按订阅前缀裁剪快照；prefixes 为 nil 时返回全量。
func buildPayload(snap *metrics.Snapshot, prefixes map[string]bool) map[string]any {
	out := map[string]any{"timestamp": snap.Timestamp, "hostname": snap.Hostname}
	full := prefixes == nil
	if full || prefixes["cpu"] {
		out["cpu"] = snap.CPU
	}
	if full || prefixes["gpu"] {
		out["gpus"] = snap.GPUs
	}
	if full || prefixes["mem"] {
		out["memory"] = snap.Memory
	}
	if full || prefixes["disk"] {
		out["disks"] = snap.Disks
		out["volumes"] = snap.Volumes
	}
	if full || prefixes["net"] {
		out["network"] = snap.Network
		out["traffic"] = snap.Traffic
	}
	return out
}
