package metrics

import (
	"math"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"
)

// maxVRAMBytes 与 maxVRAMMiB 对应：单卡显存上限 16 TiB，超过视为脏数据。
const maxVRAMBytes = uint64(maxVRAMMiB) * 1024 * 1024

// gpuDevice 是从 /sys/class/drm 发现的一块显卡（Intel / AMD）。
// 发现结果低频缓存，指标采样只复用路径，避免每轮重复扫描文件系统。
type gpuDevice struct {
	// id 是稳定标识：PCI 地址（0000:01:00.0）。DRM 卡号会随热插拔变化，不能当身份。
	id     string
	card   string // /sys/class/drm/cardN
	vendor uint64 // 0x8086 / 0x1002
	index  int    // DRM 卡号，作为对外 Index（与 sysfs 一一对应）
	kind   string // igpu / dgpu / unknown
	name   string
}

// discover 扫描 /sys/class/drm/cardN，30s 缓存一次；同时顺带探测是否存在 NVIDIA 物理卡
// （只读 PCI vendor，不触碰驱动，也不执行任何外部命令）。
func (s *gpuSampler) discover(now time.Time) []gpuDevice {
	if !s.devAt.IsZero() && now.Sub(s.devAt) < drmRescanInterval {
		return s.devs
	}
	s.devAt = now
	s.nvPresent = s.detectNvidiaPresent()
	if s.sys == "" {
		s.devs = nil
		return nil
	}

	paths, _ := filepath.Glob(filepath.Join(s.sys, "class", "drm", "card*"))
	devs := make([]gpuDevice, 0, len(paths))
	for _, card := range paths {
		base := filepath.Base(card)
		rest, ok := strings.CutPrefix(base, "card")
		if !ok {
			continue
		}
		idx, err := strconv.Atoi(rest) // card0-DP-1 这类连接器节点会被跳过
		if err != nil || idx < 0 {
			continue
		}
		vendor, ok := readHex(filepath.Join(card, "device", "vendor"))
		if !ok || (vendor != 0x8086 && vendor != 0x1002) {
			continue // NVIDIA 走 nvidia-smi；其它厂商不采集
		}
		kind := detectGPUType(card)
		devs = append(devs, gpuDevice{
			id:     pciAddress(card),
			card:   card,
			vendor: vendor,
			index:  idx,
			kind:   kind,
			name:   drmGPUName(vendor, kind),
		})
	}
	sort.Slice(devs, func(i, j int) bool { return devs[i].index < devs[j].index })
	s.devs = devs
	return devs
}

// detectNvidiaPresent 判断系统里是否有 NVIDIA 物理卡（读 PCI vendor，0x10DE）。
// 只有存在实体卡时，才把「驱动未加载」当作需要提示的降级状态。
func (s *gpuSampler) detectNvidiaPresent() bool {
	if s.sys == "" {
		return false
	}
	paths, _ := filepath.Glob(filepath.Join(s.sys, "bus", "pci", "devices", "*", "vendor"))
	for _, p := range paths {
		if v, ok := readHex(p); ok && v == 0x10DE {
			return true
		}
	}
	return false
}

// pciAddress 取设备稳定标识：优先解析 device 符号链接得到 PCI 地址，失败退回卡名。
// device 不是符号链接时（假 sysfs、或只把 /sys/class/drm 做了绑定挂载把符号链接弄断），
// 解析结果会是 "device" 本身——那样所有卡都会拿到同一个 ID，多卡 RC6 基线会串台，必须退卡名。
func pciAddress(card string) string {
	if resolved, err := filepath.EvalSymlinks(filepath.Join(card, "device")); err == nil {
		base := filepath.Base(resolved)
		if base != "" && base != "." && base != string(filepath.Separator) && base != "device" {
			return base
		}
	}
	return filepath.Base(card)
}

// detectGPUType 判断核显/独显：boot_vga=1 为主显示设备（通常是核显），0 为独显。
//
// 这是唯一不需要 PCI ID 数据库的判据。例外情况（例如无核显的机器上 Intel Arc
// 独显会拿到 boot_vga=1）无法可靠区分，此时按 igpu 展示并在文档中标注为已知局限；
// 缺少该文件时一律返回 unknown，绝不猜。
func detectGPUType(card string) string {
	b, err := os.ReadFile(filepath.Join(card, "device", "boot_vga"))
	if err != nil {
		return gpuTypeUnknown
	}
	switch strings.TrimSpace(string(b)) {
	case "1":
		return gpuTypeIGPU
	case "0":
		return gpuTypeDGPU
	default:
		return gpuTypeUnknown
	}
}

// drmGPUName 生成稳定的展示名（不探测具体型号，避免依赖会过期的 PCI ID 库）。
func drmGPUName(vendor uint64, kind string) string {
	prefix := "Intel"
	if vendor == 0x1002 {
		prefix = "AMD"
	}
	switch kind {
	case gpuTypeIGPU:
		return prefix + " 核显"
	case gpuTypeDGPU:
		return prefix + " 独显"
	default:
		return prefix + " 显卡"
	}
}

// vendorKey 厂商标识（供小屏品牌底图选择）。
func vendorKey(vendor uint64) string {
	if vendor == 0x1002 {
		return "amd"
	}
	return "intel"
}

// sampleDRMDevice 采集单块 Intel/AMD 显卡；卡路径消失时返回 false（视为已拔出）。
func (s *gpuSampler) sampleDRMDevice(d gpuDevice, now time.Time) (GPU, bool) {
	if _, err := os.Stat(d.card); err != nil {
		return GPU{}, false
	}
	g := GPU{
		Index:  d.index,
		Name:   d.name,
		Vendor: vendorKey(d.vendor),
		Type:   d.kind,
		Dev:    d.id,
	}
	g.Utilization = s.drmUtil(d, now)
	g.Temp = hwmonTemp(d.card)
	if d.vendor == 0x1002 {
		// 仅 amdgpu 暴露独立显存统计；核显共享内存没有可靠接口，保持 nil（显示 N/A）。
		g.MemUsed, g.MemTotal = amdVRAM(d.card)
	}
	g.Status = itemStatus(g)
	return g, true
}

// drmUtil 使用率：优先驱动直接暴露的 gpu_busy_percent（amdgpu / 新 i915）；
// Intel 老驱动没有该文件时，用 RC6 驻留时间反推（结果标注为估算）。
func (s *gpuSampler) drmUtil(d gpuDevice, now time.Time) *int {
	if v, ok := readInt(filepath.Join(d.card, "device", "gpu_busy_percent")); ok && v >= 0 && v <= 100 {
		return &v
	}
	if d.vendor != 0x8086 {
		return nil // 非 Intel：没有 RC6 之类的通用替代接口，直接标 N/A
	}
	return s.intelRC6Util(d, now)
}

// intelRC6Util 用 rc6_residency_ms 的增量反推 GPU 活跃率：
// 使用率 = 100 × (1 − Δrc6 / Δt)。RC6 是空闲低功耗态，其以外时间都算活跃。
//
// 安全处理：首次采样只建立基线（返回 nil）；计数器回退、采样间隔异常（<=0 或
// > rc6MaxGap）都丢弃本次结果并重建基线；rc6_enable=0（未启用统计）时返回 nil。
// 该指标是**估算值**，与驱动直接暴露的 gpu_busy_percent 不同源。
func (s *gpuSampler) intelRC6Util(d gpuDevice, now time.Time) *int {
	gt := filepath.Join(d.card, "gt", "gt0")
	cur, ok := readUint(filepath.Join(gt, "rc6_residency_ms"))
	if !ok {
		delete(s.igPrev, d.id)
		return nil
	}
	if en, ok := readUint(filepath.Join(gt, "rc6_enable")); !ok || en == 0 {
		delete(s.igPrev, d.id)
		return nil
	}
	prev, had := s.igPrev[d.id]
	s.igPrev[d.id] = rc6Sample{v: cur, at: now}
	if !had || cur < prev.v || now.Before(prev.at) {
		return nil // 首次 / 计数器回退 / 时钟异常：只重建基线
	}
	dt := now.Sub(prev.at).Seconds()
	if dt <= 0 || dt > rc6MaxGap.Seconds() {
		return nil // 采样间隔异常：旧基线不可用
	}
	busy := 100 * (1 - (float64(cur-prev.v)/1000)/dt)
	if math.IsNaN(busy) || math.IsInf(busy, 0) {
		return nil
	}
	v := int(math.Round(math.Max(0, math.Min(100, busy))))
	return &v
}

// amdVRAM 读 amdgpu 的显存统计（字节）。接口不存在时返回 nil；
// 已用大于总量视为脏数据，丢弃已用值，避免出现 >100% 的假占用。
func amdVRAM(card string) (*uint64, *uint64) {
	used, okUsed := readUint(filepath.Join(card, "device", "mem_info_vram_used"))
	total, okTotal := readUint(filepath.Join(card, "device", "mem_info_vram_total"))
	var mu, mt *uint64
	if okUsed && used <= maxVRAMBytes {
		v := used
		mu = &v
	}
	if okTotal && total > 0 && total <= maxVRAMBytes {
		v := total
		mt = &v
	}
	if mu != nil && mt != nil && *mu > *mt {
		mu = nil
	}
	return mu, mt
}

// hwmonTemp 读显卡温度：hwmon 的 temp1_input（毫摄氏度），范围 1~150℃，
// 越界或文件缺失返回 nil（其它指标继续采集）。
func hwmonTemp(card string) *int {
	files, _ := filepath.Glob(filepath.Join(card, "device", "hwmon", "hwmon*", "temp1_input"))
	for _, f := range files {
		if v, ok := readInt(f); ok && v > 0 && v <= 150000 {
			t := v / 1000
			return &t
		}
	}
	return nil
}
