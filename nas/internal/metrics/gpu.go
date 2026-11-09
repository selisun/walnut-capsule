package metrics

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"log"
	"os"
	"os/exec"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"
)

// gpuSampler 采集所有可用显卡（多卡并列，不再「独显优先、核显兜底」）：
//   - Intel / AMD：只读 /sys/class/drm（核显、独显都走这条，零外部进程）；
//   - NVIDIA：nvidia-smi，调用前做驱动/设备节点前置检查，失败退避，绝不触发驱动加载。
//
// 资源约束：设备发现低频（30s）缓存，指标采样跟随外层 3s 调度；任何一路失败
// 都不影响其它显卡，也不向外抛错。
type gpuSampler struct {
	sys string
	// nvidia 相关路径与执行函数可注入，便于用假 sysfs 做单测。
	nvPath    string
	nvModDir  string
	nvDevNode string
	nvRun     func(ctx context.Context, path string) ([]byte, error)

	// sampleMu 串行化整轮采样：RC6 基线、发现缓存、NVIDIA 退避只在采样线程内读写。
	// 用 TryLock：上一轮还没结束（例如 nvidia-smi 卡住）时直接跳过本轮，不排队堆积。
	sampleMu sync.Mutex

	// 以下字段只在持有 sampleMu 时访问。
	devs      []gpuDevice
	devAt     time.Time
	nvPresent bool
	igPrev    map[string]rc6Sample
	nvFails   int
	nvNextTry time.Time
	nvState   string
	nvReason  string

	// mu 只保护对外可见的数据；采样时绝不在持锁状态下执行外部命令。
	mu     sync.RWMutex
	gpus   []GPU
	status string
	reason string
}

type rc6Sample struct {
	v  uint64
	at time.Time
}

const (
	// NVIDIA 驱动的两个判据（两者都要满足才会调用 nvidia-smi）：
	//   - /sys/module/nvidia：内核模块是否已加载；
	//   - /dev/nvidiactl：模块初始化后由 udev 创建的控制设备。
	nvidiaModDir  = "/sys/module/nvidia"
	nvidiaDevNode = "/dev/nvidiactl"

	// nvidia-smi 单次执行上限与输出上限（异常输出不得把内存撑爆）。
	nvTimeout   = 3 * time.Second
	nvMaxOutput = 256 * 1024

	// 设备发现缓存：/sys/class/drm 低频扫描，指标采样复用缓存路径。
	drmRescanInterval = 30 * time.Second

	// RC6 反推使用率的时间窗上限；超过视为基线失效（休眠唤醒、采样被阻塞）。
	rc6MaxGap = 10 * time.Second

	// 失败退避：60s → 300s → 600s → 1200s → 1800s 封顶。
	nvBackoffMin = 60 * time.Second
	nvBackoffMax = 30 * time.Minute

	// 单块卡的显存上限（MiB）：16 TiB，超过视为脏数据，避免字节换算溢出。
	maxVRAMMiB = 1 << 24
)

// 管理页展示的显卡采集状态（尽量短，表格一列放得下）。
const (
	gpuStatusPending = "检测中"
	gpuStatusOK      = "正常"
	gpuStatusPartial = "部分可用"
	gpuStatusNoTool  = "无驱动工具"
	gpuStatusNoDrv   = "驱动未加载"
	gpuStatusNoNode  = "设备未就绪"
	gpuStatusFailed  = "采集异常"
	gpuStatusNone    = "未检测到"
)

// 显卡形态与单卡状态（写进 GPU.Type / GPU.Status，供前端区分展示）。
const (
	gpuTypeIGPU    = "igpu"
	gpuTypeDGPU    = "dgpu"
	gpuTypeUnknown = "unknown"

	gpuItemOK      = "ok"
	gpuItemPartial = "partial"
	gpuItemError   = "error"
	// gpuItemUnavailable：认出了实体卡（PCI 上有），但驱动/工具不可用，一个指标都读不到。
	// 与 error 的区别：error 是「驱动在、但读到的值不可用」，unavailable 是「根本没驱动」。
	gpuItemUnavailable = "unavailable"
)

// nvidia-smi 超时哨兵错误（用于把「超时」和「执行失败」分类记录）。
var errNvTimeout = errors.New("nvidia-smi 执行超时")

func newGPUSampler(sys string) *gpuSampler {
	path, _ := exec.LookPath("nvidia-smi")
	return &gpuSampler{
		sys:       sys,
		nvPath:    path,
		nvModDir:  nvidiaModDir,
		nvDevNode: nvidiaDevNode,
		igPrev:    map[string]rc6Sample{},
	}
}

func (s *gpuSampler) snapshot() []GPU {
	s.mu.RLock()
	defer s.mu.RUnlock()
	out := make([]GPU, len(s.gpus))
	copy(out, s.gpus)
	return out
}

// Status 返回显卡采集状态短文案（管理页展示）。
func (s *gpuSampler) Status() string {
	s.mu.RLock()
	defer s.mu.RUnlock()
	if s.status == "" {
		return gpuStatusPending
	}
	return s.status
}

// sample 采集一轮：DRM（Intel/AMD）+ NVIDIA，多卡并列输出。
// 任一路失败只影响该路，不影响其它显卡。
func (s *gpuSampler) sample() {
	if !s.sampleMu.TryLock() {
		return // 上一轮尚未结束：跳过本轮，避免采样任务堆积
	}
	defer s.sampleMu.Unlock()

	now := time.Now()
	devs := s.discover(now)
	seenIntel := make(map[string]bool, len(devs))

	var out []GPU
	missing := false
	for _, d := range devs {
		if d.vendor == 0x8086 {
			seenIntel[d.id] = true
		}
		g, ok := s.sampleDRMDevice(d, now)
		if !ok {
			missing = true // 设备刚被拔掉：下一轮强制重新发现
			continue
		}
		out = append(out, g)
	}
	// 清掉已消失设备的 RC6 基线，避免设备编号复用后继承旧基线。
	for k := range s.igPrev {
		if !seenIntel[k] {
			delete(s.igPrev, k)
		}
	}
	if missing {
		s.devAt = time.Time{}
	}

	nvGPUs := s.sampleNvidia(now)
	out = append(out, nvGPUs...)
	// 认出了 NVIDIA 实体卡，但驱动/工具不可用：补一条「只有品牌、没有指标」的占位卡。
	// 小屏官方皮肤靠厂商选品牌底图，缺了这条就会按 CPU 猜成核显品牌（如 AMD 平台显示 AMD 卡图）；
	// 指标一律留空，客户端按既有约定显示 N/A，绝不伪造 0。
	if s.nvPresent && len(nvGPUs) == 0 && s.nvState != "" && s.nvState != gpuStatusOK {
		out = append(out, GPU{
			Index:  0,
			Name:   "NVIDIA 显卡",
			Vendor: "nvidia",
			Type:   gpuTypeDGPU,
			Dev:    "nvidia:0",
			Status: gpuItemUnavailable,
		})
	}
	sort.SliceStable(out, func(i, j int) bool {
		if out[i].Index != out[j].Index {
			return out[i].Index < out[j].Index
		}
		return out[i].Vendor < out[j].Vendor
	})

	status, reason := s.aggregate(out)
	s.publish(out, status, reason)
}

// publish 原子替换对外数据，并只在状态变化时打一条日志（避免每轮刷屏）。
func (s *gpuSampler) publish(gpus []GPU, status, reason string) {
	s.mu.Lock()
	changed := s.status != status || s.reason != reason
	s.gpus = gpus
	s.status = status
	s.reason = reason
	s.mu.Unlock()
	if !changed {
		return
	}
	switch {
	case reason != "":
		log.Printf("GPU 采集状态：%s（%s）", status, reason)
	case len(gpus) == 0:
		log.Printf("GPU 采集状态：%s", status)
	default:
		if status == gpuStatusOK {
			log.Printf("GPU 采集状态：%s（共 %d 块，全部正常）", status, len(gpus))
		} else {
			log.Printf("GPU 采集状态：%s（共 %d 块）", status, len(gpus))
		}
	}
}

// aggregate 汇总总体状态：只要有一块卡可用就不算整体故障。
func (s *gpuSampler) aggregate(gpus []GPU) (string, string) {
	if len(gpus) == 0 {
		switch s.nvState {
		case gpuStatusFailed:
			return gpuStatusFailed, s.nvReason
		case gpuStatusNoDrv:
			if s.nvPresent {
				return gpuStatusNoDrv, s.nvReason
			}
		case gpuStatusNoNode:
			return gpuStatusNoNode, s.nvReason
		case gpuStatusNoTool:
			// 有实体卡但 PATH 里没有 nvidia-smi：同样要明确提示，不能笼统报「未检测到」。
			if s.nvPresent {
				return gpuStatusNoTool, s.nvReason
			}
		}
		return gpuStatusNone, ""
	}

	okCount, partial, failed, unavailable := 0, 0, 0, 0
	for _, g := range gpus {
		switch g.Status {
		case gpuItemOK:
			okCount++
		case gpuItemPartial:
			partial++
		case gpuItemUnavailable:
			unavailable++
		default:
			failed++
		}
	}
	// NVIDIA 有实体卡但采集不可用（含未装工具）：即便核显正常，也应明确是「部分可用」，
	// 不能把「有一块卡没采到」隐藏成整体正常。
	nvDegraded := s.nvPresent && s.nvState != "" && s.nvState != gpuStatusOK

	switch {
	case failed > 0 && okCount+partial == 0:
		return gpuStatusFailed, "所有 GPU 指标均不可读"
	case okCount == 0 && partial == 0 && unavailable > 0:
		// 只认出卡、一个指标都读不到：如实回报驱动/工具状态（驱动未加载 / 无驱动工具 …）。
		if s.nvState != "" {
			return s.nvState, s.nvReason
		}
		return gpuStatusFailed, "GPU 指标不可读"
	case partial > 0 || failed > 0 || unavailable > 0 || nvDegraded:
		return gpuStatusPartial, ""
	default:
		return gpuStatusOK, ""
	}
}

// ---------------- NVIDIA ----------------

// sampleNvidia 经 nvidia-smi 采集独显；未安装/驱动未加载/节点缺失/退避中一律跳过。
//
// 关键：先确认驱动真的加载了再调用 nvidia-smi。设备节点缺失时调用 nvidia-smi，
// 它会连带触发 nvidia-modprobe → modprobe，把上百 MB 的 nvidia.ko 读进来尝试
// insmod；在驱动被安全策略拒绝的机器上（老笔记本转 NAS 很常见）这每次要烧掉
// 几百毫秒内核态 CPU，且 nvidia-smi 进程只活零点几秒，按秒采样的进程列表根本
// 抓不到——表现为「查不到进程的持续 20~30% CPU 占用」。
func (s *gpuSampler) sampleNvidia(now time.Time) []GPU {
	if s.nvPath == "" {
		s.nvSet(gpuStatusNoTool, "未检测到 nvidia-smi（未安装 NVIDIA 驱动工具）")
		return nil
	}
	if _, err := os.Stat(s.nvModDir); err != nil {
		// 驱动没加载：只 stat，绝不调用 nvidia-smi / modprobe。
		s.nvSet(gpuStatusNoDrv, "NVIDIA 驱动未加载（缺少 "+s.nvModDir+"）")
		return nil
	}
	if _, err := os.Stat(s.nvDevNode); err != nil {
		// 模块在但设备节点没出来：可能是 udev 未创建或权限受限，同样不调用。
		s.nvSet(gpuStatusNoNode, "NVIDIA 驱动已加载，但缺少 "+s.nvDevNode+"（udev 未创建或权限不足）")
		return nil
	}
	// 驱动从「不可用」恢复为「可用」时清空退避，允许立刻尝试一次。
	if s.nvState == gpuStatusNoDrv || s.nvState == gpuStatusNoNode {
		s.nvFails = 0
		s.nvNextTry = time.Time{}
	}
	if now.Before(s.nvNextTry) {
		return nil // 退避中：不启动进程，状态保持上一次的失败原因
	}

	out, err := s.runNvidia()
	if err != nil {
		s.nvFails++
		s.nvNextTry = now.Add(nvBackoff(s.nvFails))
		s.nvSet(gpuStatusFailed, classifyNvError(err))
		return nil
	}
	gpus := parseNvidiaCSV(out)
	s.nvFails = 0
	s.nvNextTry = time.Time{}
	s.nvSet(gpuStatusOK, "")
	return gpus
}

// runNvidia 参数化调用（无 shell），带超时与输出上限。
func (s *gpuSampler) runNvidia() ([]byte, error) {
	ctx, cancel := context.WithTimeout(context.Background(), nvTimeout)
	defer cancel()
	if s.nvRun != nil {
		return s.nvRun(ctx, s.nvPath)
	}
	cmd := exec.CommandContext(ctx, s.nvPath,
		"--query-gpu=index,name,temperature.gpu,utilization.gpu,memory.used,memory.total",
		"--format=csv,noheader,nounits")
	var stdout limitedBuffer
	stdout.max = nvMaxOutput
	cmd.Stdout = &stdout
	var stderr limitedBuffer
	stderr.max = 4 * 1024
	cmd.Stderr = &stderr
	err := cmd.Run()
	if ctx.Err() == context.DeadlineExceeded {
		return nil, errNvTimeout
	}
	if err != nil {
		if msg := strings.TrimSpace(stderr.String()); msg != "" {
			return nil, fmt.Errorf("%w：%s", err, truncate(msg, 200))
		}
		return nil, err
	}
	return stdout.Bytes(), nil
}

// nvSet 记录 NVIDIA 采集状态；原因变化时才写日志（由 publish 统一打印总体状态）。
func (s *gpuSampler) nvSet(state, reason string) {
	s.nvState = state
	s.nvReason = reason
}

// nvBackoff 失败退避：60s → 300s → 600s → 1200s → 1800s 封顶。
func nvBackoff(fails int) time.Duration {
	if fails <= 1 {
		return nvBackoffMin
	}
	if fails == 2 {
		return 300 * time.Second
	}
	d := 300 * time.Second
	for i := 3; i <= fails; i++ {
		if d >= nvBackoffMax {
			return nvBackoffMax
		}
		d *= 2
	}
	if d > nvBackoffMax {
		return nvBackoffMax
	}
	return d
}

// classifyNvError 把失败原因分类，便于日志定位（超时 / 权限 / 退出码 / 其它）。
func classifyNvError(err error) string {
	switch {
	case errors.Is(err, errNvTimeout):
		return "nvidia-smi 执行超时（" + nvTimeout.String() + "）"
	case errors.Is(err, os.ErrPermission):
		return "nvidia-smi 权限不足"
	}
	var ee *exec.ExitError
	if errors.As(err, &ee) {
		return fmt.Sprintf("nvidia-smi 退出码 %d", ee.ExitCode())
	}
	return "nvidia-smi 执行失败：" + truncate(err.Error(), 160)
}

// parseNvidiaCSV 解析 `--format=csv,noheader,nounits` 输出；脏行直接跳过。
func parseNvidiaCSV(raw []byte) []GPU {
	text := strings.TrimSpace(string(raw))
	if text == "" {
		return nil
	}
	var out []GPU
	for _, line := range strings.Split(text, "\n") {
		f := strings.Split(line, ",")
		if len(f) < 6 {
			continue
		}
		for i := range f {
			f[i] = strings.TrimSpace(f[i])
		}
		idx, err := strconv.Atoi(f[0])
		if err != nil || idx < 0 {
			continue
		}
		g := GPU{
			Index:       idx,
			Name:        sanitizeGPUName(f[1], "NVIDIA GPU"),
			Vendor:      "nvidia",
			Type:        gpuTypeDGPU,
			Dev:         fmt.Sprintf("nvidia:%d", idx),
			Temp:        parseTempC(f[2]),
			Utilization: parsePercent(f[3]),
			MemUsed:     parseMiB(f[4]),
			MemTotal:    parseMiB(f[5]),
		}
		// 异常数据：显存已用大于总量时不展示总量，避免出现 >100% 的假数据。
		if g.MemUsed != nil && g.MemTotal != nil && *g.MemUsed > *g.MemTotal {
			g.MemTotal = nil
		}
		g.Status = itemStatus(g)
		out = append(out, g)
	}
	return out
}

// ---------------- 通用工具 ----------------

// itemStatus 单卡状态：有使用率算正常；只有温度/显存算部分可用；都没有算失败。
func itemStatus(g GPU) string {
	switch {
	case g.Utilization != nil:
		return gpuItemOK
	case g.Temp != nil || g.MemTotal != nil:
		return gpuItemPartial
	default:
		return gpuItemError
	}
}

// limitedBuffer 只保留前 max 字节，多余输出静默丢弃（防止异常输出撑爆内存）。
type limitedBuffer struct {
	buf bytes.Buffer
	max int
}

func (b *limitedBuffer) Write(p []byte) (int, error) {
	n := len(p)
	if room := b.max - b.buf.Len(); room > 0 {
		if len(p) > room {
			p = p[:room]
		}
		b.buf.Write(p)
	}
	return n, nil
}

func (b *limitedBuffer) Bytes() []byte { return b.buf.Bytes() }

func (b *limitedBuffer) String() string { return b.buf.String() }

// truncate 截断日志文本，避免超长错误串刷屏。
func truncate(s string, max int) string {
	if len(s) <= max {
		return s
	}
	return s[:max] + "…"
}

// sanitizeGPUName 清理驱动返回的名称：控制字符转空格、折叠空白、限长。
func sanitizeGPUName(name, fallback string) string {
	s := strings.Map(func(r rune) rune {
		if r < 0x20 || r == 0x7f {
			return ' '
		}
		return r
	}, name)
	s = strings.Join(strings.Fields(s), " ")
	if s == "" {
		return fallback
	}
	return truncate(s, 64)
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

// parsePercent 使用率：只接受 0~100，其它一律视为不可用（不伪造数据）。
func parsePercent(s string) *int {
	v, err := strconv.Atoi(strings.TrimSpace(s))
	if err != nil || v < 0 || v > 100 {
		return nil
	}
	return &v
}

// parseTempC 温度：只接受 -20~150℃，越界视为不可用。
func parseTempC(s string) *int {
	v, err := strconv.Atoi(strings.TrimSpace(s))
	if err != nil || v < -20 || v > 150 {
		return nil
	}
	return &v
}

// parseMiB MiB → 字节；解析失败、超上限（16 TiB）返回 nil。
func parseMiB(s string) *uint64 {
	v, err := strconv.ParseUint(strings.TrimSpace(s), 10, 64)
	if err != nil || v > maxVRAMMiB {
		return nil
	}
	b := v * 1024 * 1024
	return &b
}
