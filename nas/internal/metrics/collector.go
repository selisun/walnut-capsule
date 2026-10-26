package metrics

import (
	"log"
	"os"
	"strings"
	"time"
)

const (
	fastInterval       = 1 * time.Second  // cpu/内存/盘速率/网络
	gpuInterval        = 3 * time.Second  // nvidia-smi（规格 2~5s）
	diskStatusInterval = 45 * time.Second // hdparm -C（规格 30~60s）
)

// Collector 调度各采样器并聚合快照。
// 路径经 HOST_PROC / HOST_SYS 环境变量注入，容器内挂宿主机 /proc、/sys。
type Collector struct {
	cpu      *cpuSampler
	mem      *memSampler
	disk     *diskSampler
	gpu      *gpuSampler
	net      *netSampler
	traffic  *trafficStore
	hostname string
}

func NewCollector(stateDir string) *Collector {
	proc := getenvOr("HOST_PROC", "/proc")
	sys := getenvOr("HOST_SYS", "/sys")
	root := getenvOr("HOST_ROOT", "") // 宿主机根在容器内的挂载点（/host/root），用于还原宿主路径
	traffic := newTrafficStore(stateDir)
	c := &Collector{
		cpu:      newCPUSampler(proc, sys),
		mem:      newMemSampler(proc),
		disk:     newDiskSampler(proc, sys, root),
		gpu:      newGPUSampler(sys),
		net:      newNetSampler(proc, sys, traffic),
		traffic:  traffic,
		hostname: readHostname(proc),
	}
	// 启动时打一次温度源探测结果，便于用户报「CPU 温度不显示」时凭日志定位
	c.cpu.logTempSources()
	return c
}

// readHostname 取宿主机名：优先读挂载进来的宿主机 procfs
// （容器内 /host/proc/sys/kernel/hostname），回退当前 UTS 名
// （compose 配置 uts: host 后，UTS 名就是宿主机名）。
// 主机名运行期不变，启动时读一次即可。
func readHostname(proc string) string {
	if b, err := os.ReadFile(proc + "/sys/kernel/hostname"); err == nil {
		if h := strings.TrimSpace(string(b)); h != "" {
			return h
		}
	} else {
		log.Printf("read %s/sys/kernel/hostname: %v（改用 UTS 主机名）", proc, err)
	}
	if h, err := os.Hostname(); err == nil {
		h = strings.TrimSpace(h)
		if h != "" {
			return h
		}
	}
	return ""
}

// Run 启动后台采样循环（不阻塞）。
func (c *Collector) Run() {
	log.Printf("host: %q", c.hostname)
	// 启动即采一轮，避免接口返回空数据。
	c.cpu.sample()
	c.mem.sample()
	c.disk.sampleRates()
	c.net.sample()
	c.gpu.sample()
	go c.disk.sampleStatus() // 启动时也按铁律探测一轮
	c.disk.sampleCapacity()  // 启动即解析挂载表采一轮容量

	go loop(fastInterval, func() {
		c.cpu.sample()
		c.mem.sample()
		c.disk.sampleRates()
		c.net.sample()
	})
	go loop(gpuInterval, c.gpu.sample)
	go loop(diskStatusInterval, func() {
		c.disk.sampleStatus()
		c.disk.sampleCapacity()
	})
}

// Snapshot 聚合当前各采样器的最新值。
func (c *Collector) Snapshot() Snapshot {
	usage, temp := c.cpu.snapshot()
	l1, l5, l15, hasLoad, l30 := c.cpu.loadSnapshot()
	memUsage, memUsed, memTotal, memCache, memFree := c.mem.snapshot()
	cpu := CPU{Usage: usage, Temp: temp}
	if hasLoad {
		cpu.Load1, cpu.Load5, cpu.Load15 = &l1, &l5, &l15
		cpu.Load30 = l30
	}
	return Snapshot{
		Timestamp: time.Now().UnixMilli(),
		Hostname:  c.hostname,
		CPU:       cpu,
		Memory: Memory{
			Usage: memUsage,
			Used:  memUsed,
			Total: memTotal,
			Cache: memCache,
			Free:  memFree,
		},
		Disks:   c.disk.snapshot(),
		Volumes: c.disk.snapshotVolumes(),
		GPUs:    c.gpu.snapshot(),
		Network: c.net.snapshot(),
		Traffic: c.traffic.snapshot(),
	}
}

// Hostname 返回宿主机名（启动时读取一次）。
func (c *Collector) Hostname() string { return c.hostname }

// GPUStatus 返回显卡采集状态短文案（管理页展示，说明为什么没有显卡数据）。
func (c *Collector) GPUStatus() string { return c.gpu.Status() }

// SetNetIface 运行时切换手动指定网卡（空串恢复自动选择）。
func (c *Collector) SetNetIface(name string) { c.net.setTarget(name) }

// NetIfaces 返回可用网卡名列表（供管理页下拉）。
func (c *Collector) NetIfaces() []string { return c.net.availableIfaces() }

// NetIface 返回当前手动指定的网卡（空串为自动）。
func (c *Collector) NetIface() string { return c.net.currentTarget() }

// NetIfaceEffective 返回当前实际生效的网卡（含自动选择结果）。
func (c *Collector) NetIfaceEffective() string { return c.net.effectiveTarget() }

// SetNetSpeed 手动设置网卡协商速率（Mbps）；mbps<=0 表示清除手动值、恢复自动读取。
// WiFi 网卡不导出 sysfs speed，需由管理页手动填写。
func (c *Collector) SetNetSpeed(name string, mbps int) { c.net.setSpeed(name, mbps) }

// NetSpeeds 返回手动设置的协商速率表（网卡名 → Mbps）。
func (c *Collector) NetSpeeds() map[string]int { return c.net.speedsSnapshot() }

// NetSpeedEffective 返回当前生效网卡的协商速率（Mbps，手动值优先）。
func (c *Collector) NetSpeedEffective() int { return c.net.effectiveSpeed() }

func loop(d time.Duration, f func()) {
	t := time.NewTicker(d)
	defer t.Stop()
	for range t.C {
		f()
	}
}

func getenvOr(key, def string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return def
}
