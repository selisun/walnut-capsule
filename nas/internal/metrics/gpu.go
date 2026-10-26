package metrics

import (
	"context"
	"log"
	"math"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"
)

// gpuSampler 采集显卡占用，独显优先、核显兜底（两者不同时展示）：
//   - 独显：nvidia-smi（2~5s 间隔，由外层调度）；未安装、驱动未加载或执行失败时跳过；
//   - 核显：/sys/class/drm 下的 Intel(i915) / AMD(amdgpu)，无需额外工具，
//     仅在检测不到独显时才作为数据源。
type gpuSampler struct {
	path string
	sys  string

	// 采集状态：短文案给管理页展示，reason 给日志（只在变化时打一条）。
	status string
	reason string
	// 连续失败次数与退避到期时间：nvidia-smi 执行失败后按 60s → 300s 退避，
	// 不再每个采样周期都重试。
	nvFails   int
	nvNextTry time.Time

	// 核显 RC6 基线：i915 不导出 gpu_busy_percent，用 rc6_residency_ms
	// 的增量反推活跃度，需要保留上一次的读数与时间。
	igPrev   map[string]uint64
	igPrevAt map[string]time.Time

	mu   sync.RWMutex
	gpus []GPU
}

// NVIDIA 驱动的两个判据（两者都要满足才会调用 nvidia-smi）：
//   - /sys/module/nvidia：内核模块是否已加载；
//   - /dev/nvidiactl：模块初始化后由 udev 创建的控制设备。
const (
	nvidiaModDir  = "/sys/module/nvidia"
	nvidiaDevNode = "/dev/nvidiactl"
)

// 管理页展示的显卡采集状态（尽量短，表格一列放得下）。
const (
	gpuStatusPending = "检测中"
	gpuStatusOK      = "正常"
	gpuStatusNoTool  = "无驱动工具"
	gpuStatusNoDrv   = "驱动未加载"
	gpuStatusNoNode  = "设备未就绪"
	gpuStatusFailed  = "采集异常"
)

func newGPUSampler(sys string) *gpuSampler {
	path, _ := exec.LookPath("nvidia-smi")
	return &gpuSampler{
		path:     path,
		sys:      sys,
		igPrev:   map[string]uint64{},
		igPrevAt: map[string]time.Time{},
	}
}

func (s *gpuSampler) snapshot() []GPU {
	s.mu.RLock()
	defer s.mu.RUnlock()
	out := make([]GPU, len(s.gpus))
	copy(out, s.gpus)
	return out
}

// sample 采集显卡：有独显只报独显，没有独显才回退到核显（两者不并列显示）。
// 核显每轮都采样（即便本轮不展示），以保持 RC6 基线——独显拔除/驱动异常时
// 下一轮即可直接给出核显占用，而不必再等一个采样周期。
func (s *gpuSampler) sample() {
	ig := s.sampleIGPU()
	gpus := s.sampleNvidia()
	if len(gpus) == 0 {
		gpus = ig
	}
	s.mu.Lock()
	s.gpus = gpus
	s.mu.Unlock()
}

// sampleNvidia 经 nvidia-smi 采集独显信息；未安装、驱动未加载或执行失败返回 nil。
//
// 关键：先确认驱动真的加载了再调用 nvidia-smi。设备节点缺失时调用 nvidia-smi，
// 它会连带触发 nvidia-modprobe → modprobe，把上百 MB 的 nvidia.ko 读进来尝试
// insmod；在驱动被安全策略拒绝的机器上（老笔记本转 NAS 很常见）这每次要烧掉
// 几百毫秒内核态 CPU，且 nvidia-smi 进程只活零点几秒，按秒采样的进程列表根本
// 抓不到——表现为「查不到进程的持续 20~30% CPU 占用」。
func (s *gpuSampler) sampleNvidia() []GPU {
	if s.path == "" {
		s.setState(gpuStatusNoTool, "未检测到 nvidia-smi（未安装 NVIDIA 驱动工具）")
		return nil
	}
	if _, err := os.Stat(nvidiaModDir); err != nil {
		s.setState(gpuStatusNoDrv, "NVIDIA 驱动未加载（缺少 "+nvidiaModDir+"）")
		return nil
	}
	if _, err := os.Stat(nvidiaDevNode); err != nil {
		s.setState(gpuStatusNoNode, "NVIDIA 驱动已加载，但缺少 "+nvidiaDevNode+"（udev 未创建或权限受限）")
		return nil
	}
	if time.Now().Before(s.nvNextTry) {
		return nil // 上次执行失败，退避中（状态保持「采集异常」）
	}
	ctx, cancel := context.WithTimeout(context.Background(), 8*time.Second)
	defer cancel()
	out, err := exec.CommandContext(ctx, s.path,
		"--query-gpu=index,name,temperature.gpu,utilization.gpu,memory.used,memory.total",
		"--format=csv,noheader,nounits").Output()
	if err != nil {
		s.backoff()
		s.setState(gpuStatusFailed, "nvidia-smi 执行失败："+err.Error()+"，已退避")
		return nil
	}
	s.nvFails = 0
	s.nvNextTry = time.Time{}
	s.setState(gpuStatusOK, "")
	var gpus []GPU
	for _, line := range strings.Split(strings.TrimSpace(string(out)), "\n") {
		f := strings.Split(line, ",")
		if len(f) < 6 {
			continue
		}
		for i := range f {
			f[i] = strings.TrimSpace(f[i])
		}
		idx, err := strconv.Atoi(f[0])
		if err != nil {
			continue
		}
		gpus = append(gpus, GPU{
			Index:       idx,
			Name:        f[1],
			Temp:        parseIntOrNil(f[2]),
			Utilization: parseIntOrNil(f[3]),
			MemUsed:     parseMiBPtr(f[4]),
			MemTotal:    parseMiBPtr(f[5]),
		})
	}
	return gpus
}

// Status 返回显卡采集状态短文案（管理页展示，说明为什么没有显卡数据）。
func (s *gpuSampler) Status() string {
	s.mu.RLock()
	defer s.mu.RUnlock()
	if s.status == "" {
		return gpuStatusPending
	}
	return s.status
}

// setState 更新采集状态；reason 非空表示不可用/异常，仅在原因变化时记一条日志，
// 避免老机器上日志刷屏。
func (s *gpuSampler) setState(short, reason string) {
	s.mu.Lock()
	changed := s.status != short || s.reason != reason
	s.status, s.reason = short, reason
	s.mu.Unlock()
	if !changed {
		return
	}
	if reason == "" {
		log.Printf("NVIDIA 驱动可用，开始通过 nvidia-smi 采集独显")
		return
	}
	log.Printf("%s；已跳过独显采集，避免反复触发 nvidia-modprobe 加载驱动", reason)
}

// backoff 为执行失败的 nvidia-smi 设置退避：首次 60s，之后 300s。
func (s *gpuSampler) backoff() {
	s.nvFails++
	d := 60 * time.Second
	if s.nvFails >= 2 {
		d = 300 * time.Second
	}
	s.nvNextTry = time.Now().Add(d)
}

// sampleIGPU 扫描 /sys/class/drm/card*，采集核显使用率与温度，编号从 0 起。
// 取不到使用率的卡直接跳过，避免出现一块只有温度的“假”占用。
func (s *gpuSampler) sampleIGPU() []GPU {
	if s.sys == "" {
		return nil
	}
	cards, _ := filepath.Glob(filepath.Join(s.sys, "class", "drm", "card[0-9]*"))
	now := time.Now()
	var out []GPU
	for _, card := range cards {
		vendor, ok := readHex(filepath.Join(card, "device", "vendor"))
		if !ok || (vendor != 0x8086 && vendor != 0x1002) {
			continue // 只认 Intel / AMD 核显
		}
		util := s.igpuUtil(card, filepath.Base(card), now)
		if util == nil {
			continue
		}
		out = append(out, GPU{
			Index:       len(out),
			Name:        igpuName(vendor),
			Temp:        hwmonTemp(card),
			Utilization: util,
		})
	}
	return out
}

// igpuUtil 返回核显使用率（0~100）：
//   - amdgpu 直接读 device/gpu_busy_percent；
//   - i915 无该文件，用 RC6 驻留时间反推——RC6 是空闲低功耗态，
//     其以外的时间都算活跃：使用率 = 100 × (1 − Δrc6 / Δt)。
//
// 首次采样只建立基线，返回 nil；rc6_enable 为 0（未启用统计）时同样返回 nil。
func (s *gpuSampler) igpuUtil(card, id string, now time.Time) *int {
	if v, ok := readInt(filepath.Join(card, "device", "gpu_busy_percent")); ok && v >= 0 && v <= 100 {
		return &v
	}
	gt := filepath.Join(card, "gt", "gt0")
	cur, ok := readUint(filepath.Join(gt, "rc6_residency_ms"))
	if !ok {
		return nil
	}
	if en, ok := readUint(filepath.Join(gt, "rc6_enable")); !ok || en == 0 {
		return nil
	}
	prev, had := s.igPrev[id]
	prevAt := s.igPrevAt[id]
	s.igPrev[id] = cur
	s.igPrevAt[id] = now
	if !had || cur < prev {
		return nil
	}
	dt := now.Sub(prevAt).Seconds()
	if dt <= 0 {
		return nil
	}
	busy := 100 * (1 - (float64(cur-prev)/1000)/dt)
	v := int(math.Round(math.Max(0, math.Min(100, busy))))
	return &v
}

// igpuName 核显显示名（区分厂商，便于多显卡时辨认）。
// 不探测具体型号：型号要靠 PCI ID 去查系统 pci.ids，而那个库的版本随发行版走，
// 老库查不到新核显、同号设备还可能跨厂商撞名，取到的"型号"未必可信——不如稳定的通用名。
func igpuName(vendor uint64) string {
	if vendor == 0x1002 {
		return "AMD 核显"
	}
	return "Intel 核显"
}

// hwmonTemp 读核显温度：hwmon 的 temp1_input（毫摄氏度）。
func hwmonTemp(card string) *int {
	files, _ := filepath.Glob(filepath.Join(card, "device", "hwmon", "hwmon*", "temp1_input"))
	for _, f := range files {
		if v, ok := readInt(f); ok && v > 0 && v < 200000 {
			t := v / 1000
			return &t
		}
	}
	return nil
}

func readUint(path string) (uint64, bool) {
	b, err := os.ReadFile(path)
	if err != nil {
		return 0, false
	}
	v, err := strconv.ParseUint(strings.TrimSpace(string(b)), 10, 64)
	if err != nil {
		return 0, false
	}
	return v, true
}

func readInt(path string) (int, bool) {
	b, err := os.ReadFile(path)
	if err != nil {
		return 0, false
	}
	v, err := strconv.Atoi(strings.TrimSpace(string(b)))
	if err != nil {
		return 0, false
	}
	return v, true
}

// readHex 读十六进制属性（如 PCI vendor "0x8086"）。
func readHex(path string) (uint64, bool) {
	b, err := os.ReadFile(path)
	if err != nil {
		return 0, false
	}
	s := strings.TrimPrefix(strings.TrimSpace(string(b)), "0x")
	v, err := strconv.ParseUint(s, 16, 32)
	if err != nil {
		return 0, false
	}
	return v, true
}

func parseIntOrNil(s string) *int {
	v, err := strconv.Atoi(s)
	if err != nil {
		return nil
	}
	return &v
}

// parseMiBPtr MiB → 字节；解析失败返回 nil（JSON 里省略该字段）。
func parseMiBPtr(s string) *uint64 {
	v, err := strconv.ParseUint(s, 10, 64)
	if err != nil {
		return nil
	}
	b := v * 1024 * 1024
	return &b
}
