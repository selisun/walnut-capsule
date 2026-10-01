package metrics

import (
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"
)

// netSampler 读取 /proc/net/dev，差值计算上下行速率。
// 跳过 lo 与容器/虚拟网卡。
type netSampler struct {
	proc string

	mu     sync.RWMutex
	ifaces []NetIf
	prev   map[string]netCounter
	prevAt time.Time
}

type netCounter struct{ rx, tx uint64 }

func newNetSampler(proc string) *netSampler {
	return &netSampler{proc: proc, prev: map[string]netCounter{}}
}

func (s *netSampler) snapshot() []NetIf {
	s.mu.RLock()
	defer s.mu.RUnlock()
	out := make([]NetIf, len(s.ifaces))
	copy(out, s.ifaces)
	return out
}

func skipIface(name string) bool {
	for _, p := range []string{"lo", "docker", "veth", "br-", "virbr", "cni", "flannel", "kube", "tun", "tap", "zt"} {
		if strings.HasPrefix(name, p) {
			return true
		}
	}
	return false
}

func (s *netSampler) sample() {
	b, err := os.ReadFile(filepath.Join(s.proc, "net/dev"))
	if err != nil {
		return
	}
	now := time.Now()
	cur := map[string]netCounter{}
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
		cur[name] = netCounter{rx: rx, tx: tx}
	}

	s.mu.Lock()
	defer s.mu.Unlock()
	out := make([]NetIf, 0, len(cur))
	if !s.prevAt.IsZero() {
		dt := now.Sub(s.prevAt).Seconds()
		if dt > 0 {
			for name, c := range cur {
				p, ok := s.prev[name]
				if !ok || c.rx < p.rx || c.tx < p.tx {
					out = append(out, NetIf{Name: name})
					continue
				}
				out = append(out, NetIf{
					Name:  name,
					RxBPS: uint64(float64(c.rx-p.rx) / dt),
					TxBPS: uint64(float64(c.tx-p.tx) / dt),
				})
			}
		}
	} else {
		for name := range cur {
			out = append(out, NetIf{Name: name})
		}
	}
	s.ifaces = out
	s.prev = cur
	s.prevAt = now
}
