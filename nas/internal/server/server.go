package server

import (
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
	upgrader websocket.Upgrader
}

func New(col *metrics.Collector) *Server {
	return &Server{
		col: col,
		upgrader: websocket.Upgrader{
			ReadBufferSize:  1024,
			WriteBufferSize: 4096,
			CheckOrigin:     func(*http.Request) bool { return true }, // 局域网工具
		},
	}
}

func (s *Server) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("/health", s.handleHealth)
	mux.HandleFunc("/metrics", s.handleMetrics)
	mux.HandleFunc("/ws", s.handleWS)
	return mux
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
	for {
		select {
		case <-done:
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
	out := map[string]any{"timestamp": snap.Timestamp}
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
	}
	if full || prefixes["net"] {
		out["network"] = snap.Network
	}
	return out
}
