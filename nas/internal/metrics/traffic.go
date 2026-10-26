package metrics

import (
	"encoding/json"
	"log"
	"os"
	"path/filepath"
	"sync"
	"time"
)

const (
	// trafficSaveEvery 是落盘间隔：累计值变化很慢，没必要每秒写盘。
	trafficSaveEvery = 60 * time.Second
	// trafficKeepDays 是保留的天数：窗口最大 30 日，多留 1 天容错。
	trafficKeepDays = 31
)

// trafficState 是 traffic.json 的结构。days 是「日期 → {收, 发}」的按天累计，
// 只保留最近 31 天；今天 / 昨日 / 7 日 / 30 日都从这张表算出来。
type trafficState struct {
	Iface   string               `json:"iface"`
	Days    map[string][2]uint64 `json:"days"`
	RxTotal uint64               `json:"rx_total"`
	TxTotal uint64               `json:"tx_total"`
}

// trafficStore 按天累计目标网卡的收发字节数，供「今日 / 昨日 / 7 日 / 30 日」四档展示。
//
// 口径：每秒取 /proc/net/dev 里该网卡的累计计数（自开机起），用两轮差值累加到当天。
// 计数回退（重启、网卡重挂、切换监控网卡）时只重置基线，不把负值算进统计。
// 有状态目录时每 60s 落盘一次，跨重启保留；没有状态目录（本地调试）只在内存里累计。
type trafficStore struct {
	path string

	mu       sync.Mutex
	iface    string
	days     map[string][2]uint64
	rxTotal  uint64
	txTotal  uint64
	haveLast bool
	lastRx   uint64
	lastTx   uint64
	lastSave time.Time
}

func newTrafficStore(stateDir string) *trafficStore {
	t := &trafficStore{days: map[string][2]uint64{}}
	if stateDir != "" {
		t.path = filepath.Join(stateDir, "traffic.json")
	}
	t.load()
	return t
}

func dayStr(now time.Time) string { return now.Format("2006-01-02") }

// load 读取上次落盘的按天累计；超过保留期的日期直接丢弃。
func (t *trafficStore) load() {
	if t.path == "" {
		return
	}
	b, err := os.ReadFile(t.path)
	if err != nil {
		return // 首次运行没有文件，正常
	}
	// 边界检查：31 天流量正常 <2KB，限 64KB 防御异常增长（日志轮转会在 4MB 限制）
	if len(b) > 65536 {
		log.Printf("traffic.json 过大 (%d 字节)，按新文件处理", len(b))
		return
	}
	var st trafficState
	if err := json.Unmarshal(b, &st); err != nil {
		log.Printf("解析 traffic.json 失败（按新文件处理）：%v", err)
		return
	}
	t.iface = st.Iface
	t.rxTotal, t.txTotal = st.RxTotal, st.TxTotal
	for k, v := range st.Days {
		t.days[k] = v
	}
	t.prune(time.Now())
}

// prune 丢掉保留期之外的日期，避免文件无限增长。调用方需持锁（或尚未并发）。
func (t *trafficStore) prune(now time.Time) {
	oldest := dayStr(now.AddDate(0, 0, -trafficKeepDays+1))
	for k := range t.days {
		if k < oldest {
			delete(t.days, k)
		}
	}
}

// add 用网卡的累计计数推进当天统计；计数回退或网卡变化时只重置基线。
func (t *trafficStore) add(iface string, rx, tx uint64) {
	if iface == "" {
		return
	}
	now := time.Now()
	t.mu.Lock()
	defer t.mu.Unlock()

	if t.iface != iface || !t.haveLast {
		// 首次运行、或用户换了监控网卡：只记基线
		first := t.iface != iface
		t.iface = iface
		t.haveLast = true
		t.lastRx, t.lastTx = rx, tx
		if first {
			t.saveLocked(now, false)
		}
		return
	}
	var drx, dtx uint64
	if rx >= t.lastRx {
		drx = rx - t.lastRx
	}
	if tx >= t.lastTx {
		dtx = tx - t.lastTx
	}
	t.lastRx, t.lastTx = rx, tx
	if drx == 0 && dtx == 0 {
		t.saveLocked(now, true)
		return
	}
	key := dayStr(now)
	cur := t.days[key]
	cur[0] += drx
	cur[1] += dtx
	t.days[key] = cur
	t.rxTotal += drx
	t.txTotal += dtx
	t.saveLocked(now, true)
}

// saveLocked 按间隔落盘（throttled=false 时立即写）。调用方已持锁。
func (t *trafficStore) saveLocked(now time.Time, throttled bool) {
	if t.path == "" {
		return
	}
	if throttled && now.Sub(t.lastSave) < trafficSaveEvery {
		return
	}
	t.lastSave = now
	t.prune(now)
	st := trafficState{
		Iface:   t.iface,
		Days:    t.days,
		RxTotal: t.rxTotal,
		TxTotal: t.txTotal,
	}
	b, err := json.Marshal(st)
	if err != nil {
		return
	}
	// 先写临时文件再改名：进程被杀时不会留下半截 JSON
	tmp := t.path + ".tmp"
	if err := os.WriteFile(tmp, b, 0o644); err != nil {
		log.Printf("写 traffic.json 失败：%v", err)
		return
	}
	if err := os.Rename(tmp, t.path); err != nil {
		log.Printf("替换 traffic.json 失败：%v", err)
	}
}

// snapshot 计算四档累计值（今日 / 昨日 / 最近 7 日 / 最近 30 日，都含当天），收发分开。
// 同时给出按天序列（旧 → 新，固定 30 项）供客户端画 30 日柱状图。
func (t *trafficStore) snapshot() *Traffic {
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.iface == "" {
		return nil
	}
	now := time.Now()
	var todayRx, todayTx, yRx, yTx, d7Rx, d7Tx, d30Rx, d30Tx uint64
	daysRx := make([]uint64, 30)
	daysTx := make([]uint64, 30)
	dates := make([]string, 30)
	for i := 0; i < 30; i++ {
		day := now.AddDate(0, 0, -i)
		v := t.days[dayStr(day)]
		// 下标 0 = 29 天前，29 = 今天：客户端按从左到右的时间顺序直接画
		daysRx[29-i] = v[0]
		daysTx[29-i] = v[1]
		dates[29-i] = day.Format("01-02")
		switch i {
		case 0:
			todayRx, todayTx = v[0], v[1]
		case 1:
			yRx, yTx = v[0], v[1]
		}
		if i < 7 {
			d7Rx += v[0]
			d7Tx += v[1]
		}
		d30Rx += v[0]
		d30Tx += v[1]
	}
	return &Traffic{
		Iface:       t.iface,
		RxToday:     todayRx,
		TxToday:     todayTx,
		RxYesterday: yRx,
		TxYesterday: yTx,
		RxDays7:     d7Rx,
		TxDays7:     d7Tx,
		RxDays30:    d30Rx,
		TxDays30:    d30Tx,
		DaysRx:      daysRx,
		DaysTx:      daysTx,
		Dates:       dates,
	}
}
