package metrics

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"sync"
	"testing"
	"time"
)

// writeFake 在伪 sysfs 根下写文件（自动建父目录）。
func writeFake(t *testing.T, root, rel, content string) {
	t.Helper()
	p := filepath.Join(root, filepath.FromSlash(rel))
	if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(p, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

func newTestSampler(sys string) *gpuSampler {
	return &gpuSampler{sys: sys, igPrev: map[string]rc6Sample{}}
}

// 典型的 Intel 核显 + AMD 独显 + NVIDIA 独显：三块必须同时出现。
func TestSampleCollectsAllGPUClasses(t *testing.T) {
	sys := t.TempDir()
	// Intel 核显（i915）：只有 RC6，无 gpu_busy_percent
	writeFake(t, sys, "class/drm/card0/device/vendor", "0x8086\n")
	writeFake(t, sys, "class/drm/card0/device/boot_vga", "1\n")
	writeFake(t, sys, "class/drm/card0/gt/gt0/rc6_enable", "1\n")
	writeFake(t, sys, "class/drm/card0/gt/gt0/rc6_residency_ms", "1000\n")
	writeFake(t, sys, "class/drm/card0/device/hwmon/hwmon0/temp1_input", "45000\n")
	// AMD 独显（amdgpu）：直接使用率 + 显存 + 温度
	writeFake(t, sys, "class/drm/card1/device/vendor", "0x1002\n")
	writeFake(t, sys, "class/drm/card1/device/boot_vga", "0\n")
	writeFake(t, sys, "class/drm/card1/device/gpu_busy_percent", "37\n")
	writeFake(t, sys, "class/drm/card1/device/mem_info_vram_used", "1073741824\n")
	writeFake(t, sys, "class/drm/card1/device/mem_info_vram_total", "8589934592\n")
	writeFake(t, sys, "class/drm/card1/device/hwmon/hwmon1/temp1_input", "61000\n")
	// 系统里存在 NVIDIA 实体卡
	writeFake(t, sys, "bus/pci/devices/dev-0000-01-00.0/vendor", "0x10DE\n")

	dir := t.TempDir()
	modDir := filepath.Join(dir, "nvidia")
	nodePath := filepath.Join(dir, "nvidiactl")
	if err := os.MkdirAll(modDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(nodePath, []byte{}, 0o644); err != nil {
		t.Fatal(err)
	}

	s := newTestSampler(sys)
	s.nvPath = "/bin/true"
	s.nvModDir = modDir
	s.nvDevNode = nodePath
	s.nvRun = func(context.Context, string) ([]byte, error) {
		return []byte("0, NVIDIA GeForce RTX 3060, 55, 12, 2048, 12288\n"), nil
	}

	s.sample()
	gpus := s.snapshot()
	if len(gpus) != 3 {
		t.Fatalf("应同时采集 3 块 GPU，实际 %d：%+v", len(gpus), gpus)
	}
	byVendor := map[string]GPU{}
	for _, g := range gpus {
		byVendor[g.Vendor] = g
	}
	intel, ok := byVendor["intel"]
	if !ok || intel.Type != gpuTypeIGPU || intel.Name != "Intel 核显" {
		t.Fatalf("Intel 核显识别异常：%+v", intel)
	}
	if intel.Temp == nil || *intel.Temp != 45 {
		t.Fatalf("Intel 温度异常：%+v", intel.Temp)
	}
	if intel.Utilization != nil {
		t.Fatalf("RC6 首次采样只建基线，不应给使用率：%+v", intel.Utilization)
	}
	if intel.Status != gpuItemPartial {
		t.Fatalf("只有温度时应为 partial，实际 %q", intel.Status)
	}

	amd, ok := byVendor["amd"]
	if !ok || amd.Type != gpuTypeDGPU || amd.Name != "AMD 独显" {
		t.Fatalf("AMD 独显识别异常：%+v", amd)
	}
	if amd.Utilization == nil || *amd.Utilization != 37 {
		t.Fatalf("AMD 使用率异常：%+v", amd.Utilization)
	}
	if amd.MemTotal == nil || *amd.MemTotal != 8*1024*1024*1024 {
		t.Fatalf("AMD 显存异常：%+v", amd.MemTotal)
	}
	if amd.Status != gpuItemOK {
		t.Fatalf("AMD 状态应为 ok，实际 %q", amd.Status)
	}

	nv, ok := byVendor["nvidia"]
	if !ok || nv.Type != gpuTypeDGPU || nv.Dev != "nvidia:0" {
		t.Fatalf("NVIDIA 识别异常：%+v", nv)
	}
	if s.Status() != gpuStatusPartial {
		t.Fatalf("存在只有部分指标的卡时应为「部分可用」，实际 %q", s.Status())
	}
}

// AMD 核显 / AMD 独显 / 无法判断 三种情况必须正确区分，不能一律叫核显。
func TestAMDTypeClassification(t *testing.T) {
	sys := t.TempDir()
	writeFake(t, sys, "class/drm/card0/device/vendor", "0x1002\n")
	writeFake(t, sys, "class/drm/card0/device/boot_vga", "1\n")
	writeFake(t, sys, "class/drm/card0/device/gpu_busy_percent", "10\n")
	writeFake(t, sys, "class/drm/card1/device/vendor", "0x1002\n")
	writeFake(t, sys, "class/drm/card1/device/boot_vga", "0\n")
	writeFake(t, sys, "class/drm/card1/device/gpu_busy_percent", "20\n")
	writeFake(t, sys, "class/drm/card2/device/vendor", "0x1002\n")
	writeFake(t, sys, "class/drm/card2/device/gpu_busy_percent", "30\n")

	s := newTestSampler(sys)
	s.sample()
	gpus := s.snapshot()
	if len(gpus) != 3 {
		t.Fatalf("应识别 3 块 AMD 卡，实际 %d", len(gpus))
	}
	want := []struct {
		kind string
		name string
	}{
		{gpuTypeIGPU, "AMD 核显"},
		{gpuTypeDGPU, "AMD 独显"},
		{gpuTypeUnknown, "AMD 显卡"},
	}
	for i, w := range want {
		if gpus[i].Type != w.kind || gpus[i].Name != w.name {
			t.Fatalf("第 %d 块卡分类异常：%+v，期望 %+v", i, gpus[i], w)
		}
	}
}

// NVIDIA 执行失败不得影响 Intel / AMD 的采集。
func TestNvidiaFailureDoesNotBreakOtherGPUs(t *testing.T) {
	sys := t.TempDir()
	writeFake(t, sys, "class/drm/card0/device/vendor", "0x1002\n")
	writeFake(t, sys, "class/drm/card0/device/gpu_busy_percent", "42\n")
	writeFake(t, sys, "bus/pci/devices/dev-0000-01-00.0/vendor", "0x10DE\n")

	dir := t.TempDir()
	modDir := filepath.Join(dir, "nvidia")
	nodePath := filepath.Join(dir, "nvidiactl")
	if err := os.MkdirAll(modDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(nodePath, []byte{}, 0o644); err != nil {
		t.Fatal(err)
	}

	s := newTestSampler(sys)
	s.nvPath = "/bin/false"
	s.nvModDir = modDir
	s.nvDevNode = nodePath
	s.nvRun = func(context.Context, string) ([]byte, error) {
		return nil, errors.New("driver exploded")
	}
	s.sample()

	gpus := s.snapshot()
	// AMD 卡照常采集；NVIDIA 有实体卡但没有可用指标，补一条只有品牌的占位卡
	if len(gpus) != 2 || gpus[0].Vendor != "amd" {
		t.Fatalf("NVIDIA 失败不应影响 AMD：%+v", gpus)
	}
	if gpus[1].Vendor != "nvidia" || gpus[1].Status != gpuItemUnavailable {
		t.Fatalf("NVIDIA 应保留品牌占位卡：%+v", gpus[1])
	}
	if s.Status() != gpuStatusPartial {
		t.Fatalf("有 NVIDIA 实体卡但采集失败时总体应为「部分可用」，实际 %q", s.Status())
	}
}

// NVIDIA 驱动未加载：只提示，不执行命令，其它卡照常采集。
func TestNvidiaDriverMissingKeepsOthers(t *testing.T) {
	sys := t.TempDir()
	writeFake(t, sys, "class/drm/card0/device/vendor", "0x8086\n")
	writeFake(t, sys, "class/drm/card0/device/gpu_busy_percent", "5\n")
	writeFake(t, sys, "bus/pci/devices/dev-0000-01-00.0/vendor", "0x10DE\n")

	called := false
	s := newTestSampler(sys)
	s.nvPath = "/bin/true"
	s.nvModDir = filepath.Join(t.TempDir(), "no-such-module")
	s.nvDevNode = filepath.Join(t.TempDir(), "no-such-node")
	s.nvRun = func(context.Context, string) ([]byte, error) {
		called = true
		return nil, nil
	}
	s.sample()

	if called {
		t.Fatal("驱动未加载时不允许执行 nvidia-smi")
	}
	gpus := s.snapshot()
	// 核显照常采集；NVIDIA 驱动未加载时仍要报出品牌，客户端才能选对应底图
	if len(gpus) != 2 || gpus[0].Vendor != "intel" {
		t.Fatalf("核显应照常采集：%+v", gpus)
	}
	if gpus[1].Vendor != "nvidia" || gpus[1].Status != gpuItemUnavailable {
		t.Fatalf("NVIDIA 应保留品牌占位卡：%+v", gpus[1])
	}
}

// 没有任何可用设备时给出「未检测到」，而不是「采集异常」。
func TestNoGPUReportsNone(t *testing.T) {
	s := newTestSampler(t.TempDir())
	s.sample()
	if got := s.Status(); got != gpuStatusNone {
		t.Fatalf("无设备时状态应为 %q，实际 %q", gpuStatusNone, got)
	}
	if len(s.snapshot()) != 0 {
		t.Fatalf("无设备时不应有数据：%+v", s.snapshot())
	}
}

// RC6 反推：基线、正常增量、计数器回退、间隔异常、统计未启用。
func TestIntelRC6Util(t *testing.T) {
	sys := t.TempDir()
	writeFake(t, sys, "class/drm/card0/device/vendor", "0x8086\n")
	writeFake(t, sys, "class/drm/card0/gt/gt0/rc6_enable", "1\n")
	writeFake(t, sys, "class/drm/card0/gt/gt0/rc6_residency_ms", "1000\n")

	s := newTestSampler(sys)
	d := gpuDevice{id: "0000:00:02.0", card: filepath.Join(sys, "class/drm/card0"), vendor: 0x8086}
	now := time.Now()

	if got := s.intelRC6Util(d, now); got != nil {
		t.Fatalf("首次采样只建基线，应返回 nil，实际 %v", *got)
	}
	// 2 秒内 RC6 只增加 500ms → 使用率 75%
	s.igPrev[d.id] = rc6Sample{v: 1000, at: now.Add(-2 * time.Second)}
	writeFake(t, sys, "class/drm/card0/gt/gt0/rc6_residency_ms", "1500\n")
	got := s.intelRC6Util(d, now)
	if got == nil || *got != 75 {
		t.Fatalf("RC6 估算应为 75，实际 %v", got)
	}

	// 计数器回退：只重建基线，不给数据
	s.igPrev[d.id] = rc6Sample{v: 5000, at: now.Add(-2 * time.Second)}
	writeFake(t, sys, "class/drm/card0/gt/gt0/rc6_residency_ms", "1000\n")
	if got := s.intelRC6Util(d, now); got != nil {
		t.Fatalf("计数器回退时应返回 nil，实际 %v", *got)
	}
	if s.igPrev[d.id].v != 1000 {
		t.Fatalf("计数器回退后应重建基线，实际 %+v", s.igPrev[d.id])
	}

	// 间隔过长（休眠唤醒/采样被阻塞）：旧基线作废
	s.igPrev[d.id] = rc6Sample{v: 1000, at: now.Add(-(rc6MaxGap + time.Second))}
	writeFake(t, sys, "class/drm/card0/gt/gt0/rc6_residency_ms", "1500\n")
	if got := s.intelRC6Util(d, now); got != nil {
		t.Fatalf("间隔过长时应返回 nil，实际 %v", *got)
	}

	// 统计未启用：不产出使用率，并清掉基线
	writeFake(t, sys, "class/drm/card0/gt/gt0/rc6_enable", "0\n")
	s.igPrev[d.id] = rc6Sample{v: 1000, at: now.Add(-2 * time.Second)}
	if got := s.intelRC6Util(d, now); got != nil {
		t.Fatalf("rc6_enable=0 时应返回 nil，实际 %v", *got)
	}
	if _, exists := s.igPrev[d.id]; exists {
		t.Fatal("统计关闭时应清理基线")
	}
}

// 设备被拔出后必须从结果里消失，不能长期显示已不存在的卡。
func TestRemovedDeviceDisappears(t *testing.T) {
	sys := t.TempDir()
	writeFake(t, sys, "class/drm/card0/device/vendor", "0x1002\n")
	writeFake(t, sys, "class/drm/card0/device/gpu_busy_percent", "10\n")

	s := newTestSampler(sys)
	s.sample()
	if len(s.snapshot()) != 1 {
		t.Fatalf("初始应识别到 1 块卡：%+v", s.snapshot())
	}

	if err := os.RemoveAll(filepath.Join(sys, "class/drm/card0")); err != nil {
		t.Fatal(err)
	}
	s.devAt = time.Time{} // 强制重新发现（正常路径由采样里检测到缺失自动触发）
	s.sample()
	if len(s.snapshot()) != 0 {
		t.Fatalf("设备拔出后不应残留数据：%+v", s.snapshot())
	}
}

// 显存接口给脏数据（超大值 / 已用 > 总量）时不能算出假占用。
func TestAMDVDirtyVRAMIsRejected(t *testing.T) {
	sys := t.TempDir()
	card := filepath.Join(sys, "card0")
	writeFake(t, sys, "card0/device/mem_info_vram_used", "999999999999999999\n")
	writeFake(t, sys, "card0/device/mem_info_vram_total", "8589934592\n")
	used, total := amdVRAM(card)
	if used != nil {
		t.Fatalf("超大已用值应丢弃，实际 %v", *used)
	}
	if total == nil || *total != 8589934592 {
		t.Fatalf("总量应保留，实际 %+v", total)
	}

	writeFake(t, sys, "card0/device/mem_info_vram_used", "8589934592\n")
	writeFake(t, sys, "card0/device/mem_info_vram_total", "1073741824\n")
	used, total = amdVRAM(card)
	if used != nil || total == nil {
		t.Fatalf("已用 > 总量时应丢弃已用、保留总量，实际 used=%v total=%v", used, total)
	}
}

// 并发采样不得产生数据竞争（配合 go test -race），也不得阻塞或 panic。
func TestConcurrentSampleIsSafe(t *testing.T) {
	sys := t.TempDir()
	writeFake(t, sys, "class/drm/card0/device/vendor", "0x1002\n")
	writeFake(t, sys, "class/drm/card0/device/gpu_busy_percent", "55\n")
	writeFake(t, sys, "class/drm/card0/device/hwmon/hwmon0/temp1_input", "55000\n")

	s := newTestSampler(sys)
	var wg sync.WaitGroup
	for i := 0; i < 8; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for j := 0; j < 20; j++ {
				s.sample()
				_ = s.snapshot()
				_ = s.Status()
			}
		}()
	}
	wg.Wait()

	gpus := s.snapshot()
	if len(gpus) != 1 {
		t.Fatalf("并发采样后数据异常：%+v", gpus)
	}
	if gpus[0].Utilization == nil || *gpus[0].Utilization != 55 {
		t.Fatalf("并发采样后使用率异常：%+v", gpus[0].Utilization)
	}
}

// 半成品单卡（无使用率、无温度、无显存）标记为 error，且不伪造任何数值。
func TestUnreadableGPUIsMarkedError(t *testing.T) {
	sys := t.TempDir()
	writeFake(t, sys, "class/drm/card0/device/vendor", "0x1002\n")
	s := newTestSampler(sys)
	s.sample()
	gpus := s.snapshot()
	if len(gpus) != 1 {
		t.Fatalf("应识别到 1 块卡：%+v", gpus)
	}
	g := gpus[0]
	if g.Status != gpuItemError {
		t.Fatalf("全部指标不可读时应为 error，实际 %q", g.Status)
	}
	if g.Utilization != nil || g.Temp != nil || g.MemTotal != nil {
		t.Fatalf("不可读的指标必须为 nil：%+v", g)
	}
	if s.Status() != gpuStatusFailed {
		t.Fatalf("只有失败卡时总体应为「采集异常」，实际 %q", s.Status())
	}
}

// 50 块卡的发现与采样不应引入明显额外开销（只做一次 glob + 每卡少量小文件读）。
func TestManyGPUsStayLinear(t *testing.T) {
	sys := t.TempDir()
	for i := 0; i < 50; i++ {
		card := "class/drm/card" + strconv.Itoa(i)
		writeFake(t, sys, card+"/device/vendor", "0x1002\n")
		writeFake(t, sys, card+"/device/gpu_busy_percent", "7\n")
	}
	s := newTestSampler(sys)
	// 耗时上限只用于拦住 O(n²) 级别的退化：目标平台 Linux 给 500ms（正常值个位数毫秒）；
	// Windows 本机跑测试时 NTFS 目录 + EvalSymlinks 比 sysfs 慢一个量级，会偶发越过 500ms，
	// 那不是性能退化，放宽到 3s 只保留「不爆炸」的兜底含义。
	limit := 500 * time.Millisecond
	if runtime.GOOS == "windows" {
		limit = 3 * time.Second
	}
	start := time.Now()
	s.sample()
	if elapsed := time.Since(start); elapsed > limit {
		t.Fatalf("50 块卡采样耗时异常：%v（上限 %v）", elapsed, limit)
	}
	if len(s.snapshot()) != 50 {
		t.Fatalf("应采集 50 块卡，实际 %d", len(s.snapshot()))
	}
}
