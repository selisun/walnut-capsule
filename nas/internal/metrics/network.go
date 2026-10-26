package metrics

import (
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"
)

// netSampler 读取 /proc/net/dev，差值计算上下行速率。
// 只返回主力物理网卡：
//   - 若设了 NET_IFACE 环境变量，用指定网卡；
//   - 否则排除 lo / veth* / br-* / docker0 / ovs-system 等虚拟网卡，
//     取累计收发流量最大的那块物理网卡。
type netSampler struct {
	proc string
	sys  string

	// target 为手动指定的网卡名（空串表示自动选择）。
	// 可由管理页在运行时改写，用独立锁保护。
	targetMu sync.RWMutex
	target   string

	// speeds 为手动指定的网卡协商速率（Mbps），键为网卡名。
	// WiFi 网卡不导出 /sys/class/net/*/speed，无法自动识别，需由管理页手动填写，
	// 客户端据此计算网络环占比。手动值优先于 sysfs。
	speedMu sync.RWMutex
	speeds  map[string]int

	mu     sync.RWMutex
	ifaces []NetIf
	prev   map[string]netCounter
	prevAt time.Time
	seen   map[string]netCounter // 所有见过的非虚拟网卡累计流量（用于自动选主力）

	// traffic 记录目标网卡的今日 / 本月 / 总量累计（可跨重启持久化）
	traffic *trafficStore
}

type netCounter struct{ rx, tx uint64 }

func newNetSampler(proc, sys string, traffic *trafficStore) *netSampler {
	return &netSampler{
		proc:    proc,
		sys:     sys,
		target:  getenvOr("NET_IFACE", ""),
		speeds:  map[string]int{},
		prev:    map[string]netCounter{},
		seen:    map[string]netCounter{},
		traffic: traffic,
	}
}

// linkSpeedMbps 读取网卡协商速率（Mbps）：/sys/class/net/<iface>/speed。
// 网卡 down、虚拟设备或驱动未导出时读到负值/错误，一律返回 0（未知），
// 客户端据此退化为不画进度弧（只留轨道）。
func linkSpeedMbps(sys, name string) int {
	if sys == "" || name == "" {
		return 0
	}
	b, err := os.ReadFile(filepath.Join(sys, "class", "net", name, "speed"))
	if err != nil {
		return 0
	}
	v, err := strconv.Atoi(strings.TrimSpace(string(b)))
	if err != nil || v <= 0 {
		return 0
	}
	return v
}

// speedOf 返回网卡的协商速率（Mbps）：手动设置优先，其次读 sysfs，都取不到为 0。
func (s *netSampler) speedOf(name string) int {
	s.speedMu.RLock()
	v := s.speeds[name]
	s.speedMu.RUnlock()
	if v > 0 {
		return v
	}
	return linkSpeedMbps(s.sys, name)
}

// setSpeed 手动设置网卡协商速率（Mbps）；mbps<=0 表示清除手动值、恢复自动读取。
func (s *netSampler) setSpeed(name string, mbps int) {
	if name == "" {
		return
	}
	s.speedMu.Lock()
	defer s.speedMu.Unlock()
	if mbps <= 0 {
		delete(s.speeds, name)
		return
	}
	s.speeds[name] = mbps
}

// speedsSnapshot 返回手动设置的协商速率表副本（网卡名 → Mbps）。
func (s *netSampler) speedsSnapshot() map[string]int {
	s.speedMu.RLock()
	defer s.speedMu.RUnlock()
	out := make(map[string]int, len(s.speeds))
	for k, v := range s.speeds {
		out[k] = v
	}
	return out
}

func (s *netSampler) snapshot() []NetIf {
	s.mu.RLock()
	defer s.mu.RUnlock()
	out := make([]NetIf, len(s.ifaces))
	copy(out, s.ifaces)
	return out
}

func skipIface(name string) bool {
	for _, p := range []string{"lo", "docker", "veth", "br-", "virbr", "cni", "flannel", "kube", "tun", "tap", "zt", "ovs"} {
		if strings.HasPrefix(name, p) {
			return true
		}
	}
	return false
}

// defaultRouteIface 读 /proc/net/route，返回默认路由（Destination=00000000）
// 所在网卡。多条默认路由时取 Metric 最小者，平手优先有线（非 wl*）。
// 读不到返回空串，交由调用方按流量兜底。
func defaultRouteIface(proc string) string {
	b, err := os.ReadFile(filepath.Join(proc, "net", "route"))
	if err != nil {
		return ""
	}
	var best string
	var bestMetric uint64
	have := false
	for n, line := range strings.Split(string(b), "\n") {
		if n == 0 { // 跳过表头
			continue
		}
		f := strings.Fields(line)
		if len(f) < 7 || f[1] != "00000000" {
			continue
		}
		if skipIface(f[0]) {
			continue
		}
		metric, err := strconv.ParseUint(f[6], 10, 64)
		if err != nil {
			continue
		}
		if !have || metric < bestMetric ||
			(metric == bestMetric && strings.HasPrefix(best, "wl") && !strings.HasPrefix(f[0], "wl")) {
			have = true
			best = f[0]
			bestMetric = metric
		}
	}
	return best
}

func (s *netSampler) sample() {
	b, err := os.ReadFile(filepath.Join(s.proc, "net/dev"))
	if err != nil {
		return
	}
	now := time.Now()
	cur := map[string]netCounter{}
	seen := map[string]netCounter{} // 本次采样的累计流量快照
	for _, line := range strings.Split(string(b), "\n") {
		i := strings.IndexByte(line, ':')
		if i < 0 {
			continue
		}
		name := strings.TrimSpace(line[:i])
		if skipIface(name) {
			continue
		}
		f := strings.Fields(line[i+1:])
		if len(f) < 9 {
			continue
		}
		rx, err1 := strconv.ParseUint(f[0], 10, 64)
		tx, err2 := strconv.ParseUint(f[8], 10, 64)
		if err1 != nil || err2 != nil {
			continue
		}
		cnt := netCounter{rx: rx, tx: tx}
		cur[name] = cnt
		seen[name] = cnt // 填本地副本，稍后一次性持锁更新 s.seen
	}

	// 选择目标网卡：手动指定 > 默认路由网卡 > 累计流量最大
	s.targetMu.RLock()
	target := s.target
	s.targetMu.RUnlock()
	if target == "" {
		target = defaultRouteIface(s.proc)
	}
	if target == "" {
		// 拿写锁，同时更新 s.seen 并读取用于选择主力网卡
		s.mu.Lock()
		for name, c := range seen {
			s.seen[name] = c
		}
		var best string
		var bestTotal uint64
		for name, c := range s.seen {
			total := c.rx + c.tx
			// 平手时优先有线（非无线网卡）
			if total > bestTotal || (total == bestTotal && strings.HasPrefix(best, "wl") && !strings.HasPrefix(name, "wl")) {
				bestTotal = total
				best = name
			}
		}
		s.mu.Unlock()
		target = best
	} else {
		// 已选定目标网卡，只需更新 s.seen
		s.mu.Lock()
		for name, c := range seen {
			s.seen[name] = c
		}
		s.mu.Unlock()
	}

	// 累计流量：拿该网卡自开机起的计数推进「今日 / 本月 / 总量」
	if c, ok := cur[target]; ok && s.traffic != nil {
		s.traffic.add(target, c.rx, c.tx)
	}

	s.mu.Lock()
	defer s.mu.Unlock()
	// 协商速率：供客户端计算网络环百分比（读不到则为 0，客户端只画轨道）
	speed := s.speedOf(target)
	var out []NetIf
	if c, ok := cur[target]; ok {
		if !s.prevAt.IsZero() {
			dt := now.Sub(s.prevAt).Seconds()
			if dt > 0 {
				p, ok := s.prev[target]
				if ok && c.rx >= p.rx && c.tx >= p.tx {
					out = append(out, NetIf{
						Name:      target,
						RxBPS:     uint64(float64(c.rx-p.rx) / dt),
						TxBPS:     uint64(float64(c.tx-p.tx) / dt),
						SpeedMbps: speed,
					})
				} else {
					out = append(out, NetIf{Name: target, SpeedMbps: speed})
				}
			}
		} else {
			out = append(out, NetIf{Name: target, SpeedMbps: speed})
		}
	}
	s.ifaces = out
	s.prev = cur
	s.prevAt = now
}

// setTarget 运行时切换手动指定网卡（空串恢复自动选择）。
func (s *netSampler) setTarget(name string) {
	s.targetMu.Lock()
	s.target = name
	s.targetMu.Unlock()
}

// currentTarget 返回当前手动指定的网卡名，空串表示自动。
func (s *netSampler) currentTarget() string {
	s.targetMu.RLock()
	defer s.targetMu.RUnlock()
	return s.target
}

// availableIfaces 返回扫描到的所有非虚拟网卡名（升序），供管理页下拉选择。
func (s *netSampler) availableIfaces() []string {
	s.mu.RLock()
	defer s.mu.RUnlock()
	out := make([]string, 0, len(s.seen))
	for name := range s.seen {
		out = append(out, name)
	}
	sort.Strings(out)
	return out
}

// effectiveTarget 返回当前实际生效的网卡名（含自动选择结果）。
func (s *netSampler) effectiveTarget() string {
	if t := s.currentTarget(); t != "" {
		return t
	}
	s.mu.RLock()
	defer s.mu.RUnlock()
	if len(s.ifaces) > 0 {
		return s.ifaces[0].Name
	}
	return ""
}

// effectiveSpeed 返回当前生效网卡的协商速率（Mbps，手动值优先）；
// 无生效网卡或无法确定时为 0，供管理页回显。
func (s *netSampler) effectiveSpeed() int {
	name := s.effectiveTarget()
	if name == "" {
		return 0
	}
	return s.speedOf(name)
}
