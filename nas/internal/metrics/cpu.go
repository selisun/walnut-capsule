package metrics

import (
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
)

// cpuSampler 读取 /proc/stat 与 /sys/class/thermal，差值计算使用率。
type cpuSampler struct {
	proc string
	sys  string

	mu      sync.RWMutex
	usage   float64
	temp    *float64
	prevAll uint64
	prevIdl uint64
	hasPrev bool
}

func newCPUSampler(proc, sys string) *cpuSampler {
	return &cpuSampler{proc: proc, sys: sys}
}

func (s *cpuSampler) snapshot() (float64, *float64) {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return s.usage, s.temp
}

func (s *cpuSampler) sample() {
	s.sampleUsage()
	s.sampleTemp()
}

func (s *cpuSampler) sampleUsage() {
	b, err := os.ReadFile(filepath.Join(s.proc, "stat"))
	if err != nil {
		return
	}
	for _, line := range strings.Split(string(b), "\n") {
		if !strings.HasPrefix(line, "cpu ") {
			continue
		}
		f := strings.Fields(line)
		if len(f) < 5 {
			return
		}
		var all, idl uint64
		for i, v := range f[1:] {
			n, err := strconv.ParseUint(v, 10, 64)
			if err != nil {
				return
			}
			all += n
			if i == 3 || i == 4 { // idle + iowait
				idl += n
			}
		}
		s.mu.Lock()
		if s.hasPrev && all > s.prevAll {
			busy := float64((all - s.prevAll) - (idl - s.prevIdl))
			s.usage = busy / float64(all-s.prevAll) * 100
		}
		s.prevAll, s.prevIdl, s.hasPrev = all, idl, true
		s.mu.Unlock()
		return
	}
}

// 温度来源优先级：常见 CPU 传感器命名 > 第一个热区。
var cpuThermalTypes = []string{"x86_pkg_temp", "k10temp", "cpu_thermal", "coretemp", "cpu-thermal", "soc_thermal"}

func (s *cpuSampler) sampleTemp() {
	zones, err := filepath.Glob(filepath.Join(s.sys, "class/thermal/thermal_zone*"))
	if err != nil || len(zones) == 0 {
		return
	}
	readZone := func(z string) (string, float64, bool) {
		tb, err1 := os.ReadFile(filepath.Join(z, "type"))
		vb, err2 := os.ReadFile(filepath.Join(z, "temp"))
		if err1 != nil || err2 != nil {
			return "", 0, false
		}
		v, err := strconv.ParseFloat(strings.TrimSpace(string(vb)), 64)
		if err != nil {
			return "", 0, false
		}
		return strings.TrimSpace(string(tb)), v / 1000.0, true
	}
	var first float64
	haveFirst := false
	for _, z := range zones {
		typ, val, ok := readZone(z)
		if !ok {
			continue
		}
		if !haveFirst {
			first, haveFirst = val, true
		}
		for _, want := range cpuThermalTypes {
			if typ == want {
				s.mu.Lock()
				s.temp = &val
				s.mu.Unlock()
				return
			}
		}
	}
	if haveFirst {
		s.mu.Lock()
		s.temp = &first
		s.mu.Unlock()
	}
}
