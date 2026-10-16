package metrics

import (
	"fmt"
	"log"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"sync"
)

// cpuSampler 读取 /proc/stat 计算使用率；温度优先读 /sys/class/hwmon（AMD k10temp 只注册 hwmon），
// 回退 /sys/class/thermal。
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

// CPU 温度来源优先级：
//  1. hwmon 中已知的 CPU 传感器驱动——AMD 平台（如 Ryzen 5825U 的 k10temp）的 CPU 温度
//     只注册 hwmon，不创建 thermal_zone，只看热区会读不到（表现为客户端一直显示 --°）；
//  2. thermal_zone 中已知的 CPU 类型；
//  3. 第一个可读热区（兜底，保持原有行为）。
var cpuHwmonDrivers = []string{"k10temp", "zenpower", "coretemp", "cpu_thermal", "soc_thermal", "cpu-thermal"}

// hwmon 通道按 tempN_label 小写包含匹配，越靠前越优先：
// AMD 的 Tctl/Tdie、Intel 的 Package id 0，其次多 CCD 的 Tccd1/Tccd2。
var cpuTempLabels = []string{"tctl", "tdie", "package id 0", "package", "tccd"}

// thermal_zone 的 type 白名单。不含 k10temp——它是 hwmon 驱动名，不是热区类型。
var cpuThermalTypes = []string{"x86_pkg_temp", "cpu_thermal", "coretemp", "cpu-thermal", "soc_thermal"}

// 未命中任何已知标签的通道排在已知标签之后
const tempRankUnknown = 100

func (s *cpuSampler) setTemp(v float64) {
	s.mu.Lock()
	s.temp = &v
	s.mu.Unlock()
}

func (s *cpuSampler) sampleTemp() {
	if v, ok := s.tempFromHwmon(); ok {
		s.setTemp(v)
		return
	}
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
		if slices.Contains(cpuThermalTypes, typ) {
			s.setTemp(val)
			return
		}
	}
	if haveFirst {
		s.setTemp(first)
	}
}

// tempFromHwmon 在 /sys/class/hwmon 下按驱动名找 CPU 传感器（AMD k10temp、Intel coretemp 等）。
func (s *cpuSampler) tempFromHwmon() (float64, bool) {
	dirs, _ := filepath.Glob(filepath.Join(s.sys, "class/hwmon/hwmon*"))
	for _, dir := range dirs {
		nb, err := os.ReadFile(filepath.Join(dir, "name"))
		if err != nil {
			continue
		}
		if !slices.Contains(cpuHwmonDrivers, strings.TrimSpace(string(nb))) {
			continue
		}
		if v, ok := pickHwmonTemp(dir); ok {
			return v, true
		}
	}
	return 0, false
}

// pickHwmonTemp 在该 hwmon 目录下挑一个 CPU 温度通道：
// 命中已知 label 的优先，其次通道号小的（temp1 优先），并过滤 0 与超范围读数。
func pickHwmonTemp(dir string) (float64, bool) {
	inputs, _ := filepath.Glob(filepath.Join(dir, "temp*_input"))
	bestRank, bestIdx := tempRankUnknown+1, 0
	bestVal, found := 0.0, false
	for _, in := range inputs {
		num := strings.TrimSuffix(strings.TrimPrefix(filepath.Base(in), "temp"), "_input")
		idx, err := strconv.Atoi(num)
		if err != nil {
			continue
		}
		v, ok := readMilliC(in)
		if !ok {
			continue
		}
		rank := hwmonLabelRank(dir, num)
		if !found || rank < bestRank || (rank == bestRank && idx < bestIdx) {
			bestRank, bestIdx, bestVal, found = rank, idx, v, true
		}
	}
	return bestVal, found
}

// hwmonLabelRank 读 tempN_label 判断优先级；无 label 或无法识别时返回 tempRankUnknown。
func hwmonLabelRank(dir, num string) int {
	lb, err := os.ReadFile(filepath.Join(dir, "temp"+num+"_label"))
	if err != nil {
		return tempRankUnknown
	}
	l := strings.ToLower(strings.TrimSpace(string(lb)))
	for i, want := range cpuTempLabels {
		if strings.Contains(l, want) {
			return i
		}
	}
	return tempRankUnknown
}

// readMilliC 读毫摄氏度并转摄氏度；过滤 0 与明显异常的读数（部分机型传感器会返回 0）。
func readMilliC(p string) (float64, bool) {
	b, err := os.ReadFile(p)
	if err != nil {
		return 0, false
	}
	v, err := strconv.ParseFloat(strings.TrimSpace(string(b)), 64)
	if err != nil {
		return 0, false
	}
	c := v / 1000.0
	if c <= 0 || c > 130 {
		return 0, false
	}
	return c, true
}

// logTempSources 启动时打印一次温度源探测结果。
// 目的：用户报「CPU 温度不显示」时，日志里就能看到这台机器的 hwmon / 热区实际长什么样，
// 不必再去用户机器上跑命令（本应用无法远程取证）。
func (s *cpuSampler) logTempSources() {
	hw := make([]string, 0, 4)
	dirs, _ := filepath.Glob(filepath.Join(s.sys, "class/hwmon/hwmon*"))
	for _, dir := range dirs {
		nb, err := os.ReadFile(filepath.Join(dir, "name"))
		if err != nil {
			continue
		}
		parts := []string{strings.TrimSpace(string(nb))}
		inputs, _ := filepath.Glob(filepath.Join(dir, "temp*_input"))
		for _, in := range inputs {
			num := strings.TrimSuffix(strings.TrimPrefix(filepath.Base(in), "temp"), "_input")
			v, ok := readMilliC(in)
			if !ok {
				continue
			}
			label := "temp" + num
			if lb, err := os.ReadFile(filepath.Join(dir, "temp"+num+"_label")); err == nil {
				label = strings.TrimSpace(string(lb))
			}
			parts = append(parts, fmt.Sprintf("%s=%.1f", label, v))
		}
		hw = append(hw, filepath.Base(dir)+":"+strings.Join(parts, ","))
	}

	zs := make([]string, 0, 4)
	zones, _ := filepath.Glob(filepath.Join(s.sys, "class/thermal/thermal_zone*"))
	for _, z := range zones {
		tb, e1 := os.ReadFile(filepath.Join(z, "type"))
		vb, e2 := os.ReadFile(filepath.Join(z, "temp"))
		if e1 != nil || e2 != nil {
			continue
		}
		v, err := strconv.ParseFloat(strings.TrimSpace(string(vb)), 64)
		if err != nil {
			continue
		}
		zs = append(zs, fmt.Sprintf("%s=%.1f", strings.TrimSpace(string(tb)), v/1000.0))
	}

	s.sampleTemp()
	_, t := s.snapshot()
	res := "未找到（客户端将显示 --°）"
	if t != nil {
		res = fmt.Sprintf("%.1f°C", *t)
	}
	log.Printf("CPU 温度探测：hwmon[%s] thermal_zone[%s] 结果=%s",
		strings.Join(hw, " "), strings.Join(zs, " "), res)
}
