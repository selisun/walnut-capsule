package metrics

import (
	"context"
	"encoding/json"
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"
)

// 铁律：绝不主动唤醒休眠硬盘。
// - hdparm -C 仅查询电源状态，不唤醒；
// - standby 盘只报状态，跳过 SMART；
// - SMART 前再加 smartctl -n standby 双保险（期间进入休眠也不唤醒）；
// - SMART 间隔 ≥ smartInterval。

const (
	smartInterval = 30 * time.Minute
	cmdTimeout    = 15 * time.Second
	smartTimeout  = 30 * time.Second
	sectorSize    = 512
)

// 整盘设备名：sda / nvme0n1 / mmcblk0 / vda 等，排除分区与虚拟设备。
var wholeDiskRe = regexp.MustCompile(`^(sd[a-z]+|hd[a-z]+|vd[a-z]+|xvd[a-z]+|nvme[0-9]+n[0-9]+|mmcblk[0-9]+)$`)

// 数据卷宿主路径（已去前导 /）：fnOS volN、群晖 volumeN、QNAP share/*、通用 mnt/*。
var dataVolRe = regexp.MustCompile(`^(vol(ume)?[0-9]+|share/.+|mnt/.+)$`)

type diskState struct {
	name       string
	status     string // active / standby / sleeping / idle / unknown
	temp       *int
	readBPS    uint64
	writeBPS   uint64
	usedBytes  *uint64 // 整盘已用容量（字节），无法读取为 nil
	totalBytes *uint64 // 整盘总容量（字节），无法读取为 nil
	prevRead   uint64
	prevWrite  uint64
	prevAt     time.Time
	lastSMART  time.Time
}

type diskSampler struct {
	proc string
	sys  string // 宿主机 sysfs 在容器内的路径（如 /host/sys），用于 NVMe 温度
	root string // 宿主机根目录在容器内的路径（如 /host/root），空则按本机直跑处理

	mu      sync.RWMutex
	disks   map[string]*diskState
	order   []string // 稳定输出顺序
	volumes []Volume // 逻辑存储空间（fnOS /volN 等）

	// statusMu 串行化 sampleStatus：启动 goroutine 与 45s 定时器可能并发，
	// 并发跑 SMART 会击穿节流、重复 fork smartctl。
	statusMu      sync.Mutex
	smartDisabled bool // smartctl 权限/设备失败后永久关闭，避免每 45 秒重试
}

func newDiskSampler(proc, sys, root string) *diskSampler {
	return &diskSampler{proc: proc, sys: sys, root: root, disks: map[string]*diskState{}}
}

func (s *diskSampler) snapshot() []Disk {
	s.mu.RLock()
	defer s.mu.RUnlock()
	out := make([]Disk, 0, len(s.order))
	for _, name := range s.order {
		d := s.disks[name]
		out = append(out, Disk{
			Device:     "/dev/" + d.name,
			Status:     d.status,
			Temp:       d.temp,
			ReadBPS:    d.readBPS,
			WriteBPS:   d.writeBPS,
			UsedBytes:  d.usedBytes,
			TotalBytes: d.totalBytes,
		})
	}
	return out
}

// snapshotVolumes 返回逻辑存储空间容量（调用方持有自己的引用即可，量很小）。
func (s *diskSampler) snapshotVolumes() []Volume {
	s.mu.RLock()
	defer s.mu.RUnlock()
	out := make([]Volume, len(s.volumes))
	copy(out, s.volumes)
	return out
}

// sampleRates 每秒调用：读 /proc/diskstats 差值算速率，同时发现新盘。
// 只读 /proc，对休眠盘无影响。
func (s *diskSampler) sampleRates() {
	b, err := os.ReadFile(filepath.Join(s.proc, "diskstats"))
	if err != nil {
		return
	}
	now := time.Now()
	s.mu.Lock()
	defer s.mu.Unlock()
	for _, line := range strings.Split(string(b), "\n") {
		f := strings.Fields(line)
		if len(f) < 10 || !wholeDiskRe.MatchString(f[2]) {
			continue
		}
		name := f[2]
		rd, err1 := strconv.ParseUint(f[5], 10, 64) // sectors read
		wr, err2 := strconv.ParseUint(f[9], 10, 64) // sectors written
		if err1 != nil || err2 != nil {
			continue
		}
		d, ok := s.disks[name]
		if !ok {
			d = &diskState{name: name, status: "unknown"}
			s.disks[name] = d
			s.order = append(s.order, name)
		}
		if !d.prevAt.IsZero() {
			dt := now.Sub(d.prevAt).Seconds()
			if dt > 0 && rd >= d.prevRead && wr >= d.prevWrite {
				d.readBPS = uint64(float64((rd-d.prevRead)*sectorSize) / dt)
				d.writeBPS = uint64(float64((wr-d.prevWrite)*sectorSize) / dt)
			}
		}
		d.prevRead, d.prevWrite, d.prevAt = rd, wr, now
	}
}

// sampleStatus 低频调用（30~60s）：hdparm -C 查询电源状态；
// 只有明确 active 且距上次 SMART ≥30 分钟才执行 smartctl。
// 铁律：standby / sleeping / idle / unknown 一律不碰 SMART。
func (s *diskSampler) sampleStatus() {
	// 启动时的 sampleStatus goroutine 与 45s 定时器可能并发；串行化避免双跑。
	s.statusMu.Lock()
	defer s.statusMu.Unlock()

	s.mu.RLock()
	names := make([]string, len(s.order))
	copy(names, s.order)
	s.mu.RUnlock()

	for _, name := range names {
		s.mu.RLock()
		d, exists := s.disks[name]
		if !exists {
			s.mu.RUnlock()
			continue
		}
		lastSMART := d.lastSMART
		cachedTemp := d.temp
		s.mu.RUnlock()

		// NVMe：绝不调 hdparm（不支持）。温度优先读 sysfs hwmon（纯文本、零 IO）；
		// 读不到再按 30 分钟节流兜底 smartctl -n standby。
		if strings.HasPrefix(name, "nvme") {
			temp := readNvmeTempSysfs(s.sys, name)
			if temp == nil && !s.smartDisabled && time.Since(lastSMART) >= smartInterval {
				var ok bool
				temp, ok = readSMARTTemp(name)
				s.mu.Lock()
				if d, exists := s.disks[name]; exists {
					d.lastSMART = time.Now()
				}
				s.mu.Unlock()
				if !ok {
					s.disableSMART("nvme", name)
				}
				if temp == nil {
					temp = cachedTemp // 保留上次温度，避免休眠/暂不可读时闪烁
				}
			}
			status := "unknown"
			if temp != nil {
				status = "active" // 能读到温度即控制器在线
			}
			s.mu.Lock()
			if d, ok := s.disks[name]; ok {
				d.status = status
				d.temp = temp
			}
			s.mu.Unlock()
			continue
		}

		status := probePowerState(name)

		var temp *int
		switch {
		case status == "standby", status == "sleeping", s.smartDisabled:
			// 休眠/完全关闭/已禁用 SMART：只保留上次温度，绝不碰盘。
			temp = cachedTemp
		case status == "active":
			if time.Since(lastSMART) >= smartInterval {
				var ok bool
				temp, ok = readSMARTTemp(name)
				s.mu.Lock()
				if d, exists := s.disks[name]; exists {
					d.lastSMART = time.Now()
				}
				s.mu.Unlock()
				if !ok {
					s.disableSMART("sata", name)
				}
				if temp == nil {
					temp = cachedTemp // 保留上次温度，避免休眠/暂不可读时闪烁
				}
			} else {
				temp = cachedTemp // 保留上次温度
			}
		default:
			// idle / unknown：宁可没温度，也不冒险让 smartctl 碰盘。
			temp = cachedTemp
		}

		s.mu.Lock()
		if d, ok := s.disks[name]; ok {
			d.status = status
			d.temp = temp
		}
		s.mu.Unlock()
	}
}

// disableSMART 在一次权限/设备失败后永久关闭 SMART 采集路径，避免每 45 秒重试。
func (s *diskSampler) disableSMART(kind, name string) {
	if s.smartDisabled {
		return
	}
	s.smartDisabled = true
	log.Printf("smartctl 不可用（%s %s，权限或设备不支持），已关闭 SMART 采集", kind, name)
}

// readNvmeTempSysfs 读 NVMe 控制器温度：
// /sys/class/nvme/nvme0/hwmon1/temp1_input（毫摄氏度）。
// 注意：/sys/class/nvme/nvme0 是指向 devices 的符号链接，filepath.Glob
// 对含 symlink 的中间路径不可靠，因此改为读取目录项逐一匹配 hwmonN。
// sysfs 是内核导出的只读值，不产生设备 IO，绝不唤醒盘。
func readNvmeTempSysfs(sys, name string) *int {
	// /proc/diskstats 里的设备名是 nvme0n1（命名空间），
	// 但 sysfs 温度节点挂在控制器 nvme0 下，需提取控制器名。
	ctrl := name
	if strings.HasPrefix(name, "nvme") {
		i := 4 // "nvme" 长度
		for i < len(name) && name[i] >= '0' && name[i] <= '9' {
			i++
		}
		if i > 4 {
			ctrl = name[:i] // nvme0n1 -> nvme0
		}
	}
	dir := filepath.Join(sys, "class", "nvme", ctrl)
	entries, err := os.ReadDir(dir)
	if err != nil {
		log.Printf("nvme sysfs read dir %s: %v", dir, err)
		return nil
	}
	for _, e := range entries {
		if !strings.HasPrefix(e.Name(), "hwmon") {
			continue
		}
		b, err := os.ReadFile(filepath.Join(dir, e.Name(), "temp1_input"))
		if err != nil {
			continue
		}
		v, err := strconv.Atoi(strings.TrimSpace(string(b)))
		if err != nil || v <= 0 {
			continue
		}
		t := v / 1000 // 毫摄氏度 -> 摄氏度
		return &t
	}
	return nil
}

// sysBlockTotal 读整盘物理总容量：/sys/block/<name>/size（扇区数）×512。
// 纯 sysfs 文本读取，不触碰设备，休眠盘也可用（sdb/sdc 未挂载时的兜底）。
func sysBlockTotal(sys, name string) *uint64 {
	path := filepath.Join(sys, "block", name, "size")
	b, err := os.ReadFile(path)
	if err != nil {
		log.Printf("sysfs size read %s: %v", path, err)
		return nil
	}
	sectors, err := strconv.ParseUint(strings.TrimSpace(string(b)), 10, 64)
	if err != nil || sectors == 0 {
		log.Printf("sysfs size parse %s: %v", path, err)
		return nil
	}
	t := sectors * sectorSize
	return &t
}

// probePowerState 执行 hdparm -C（只查询，不唤醒盘）。
func probePowerState(name string) string {
	ctx, cancel := context.WithTimeout(context.Background(), cmdTimeout)
	defer cancel()
	out, err := exec.CommandContext(ctx, "hdparm", "-C", "/dev/"+name).CombinedOutput()
	if err != nil {
		// hdparm 不可用或不支持（如部分 USB）：状态未知。按铁律不碰 SMART。
		return "unknown"
	}
	low := strings.ToLower(string(out))
	switch {
	case strings.Contains(low, "standby"):
		return "standby"
	case strings.Contains(low, "sleeping"):
		return "sleeping"
	case strings.Contains(low, "active"):
		return "active"
	default:
		return "unknown"
	}
}

type smartJSON struct {
	Temperature *struct {
		Current *int `json:"current"`
	} `json:"temperature"`
	PowerMode *string `json:"power_mode"`
}

// readSMARTTemp 读取盘温。-n standby：若盘恰好在探测后进入休眠，直接退出不唤醒。
// 第二个返回值表示 SMART 路径是否可用；空输出/无法解析说明权限或设备不支持，
// 调用方据此一次性关闭后续采集，避免每 45 秒重试并刷日志。
func readSMARTTemp(name string) (*int, bool) {
	ctx, cancel := context.WithTimeout(context.Background(), smartTimeout)
	defer cancel()
	// smartctl 退出码非零也常带有效 JSON，忽略 err 直接解析 stdout。
	out, err := exec.CommandContext(ctx, "smartctl", "-n", "standby", "-a", "-j", "/dev/"+name).Output()
	if len(out) == 0 {
		// 退出码 2 = 盘在 standby，smartctl 按 -n standby 主动跳过，不是权限问题。
		if ee, ok := err.(*exec.ExitError); ok && ee.ExitCode() == 2 {
			return nil, true
		}
		return nil, false // 打不开设备 / smartctl 不存在
	}
	// 边界检查：SMART JSON 正常 5-15KB，限 256KB 防御异常输出
	if len(out) > 262144 {
		log.Printf("smartctl %s 输出过大 (%d 字节)，跳过", name, len(out))
		return nil, true
	}
	var sj smartJSON
	if err := json.Unmarshal(out, &sj); err != nil {
		log.Printf("smartctl parse %s: %v", name, err)
		return nil, false
	}
	if sj.PowerMode != nil && strings.Contains(strings.ToUpper(*sj.PowerMode), "STANDBY") {
		return nil, true // 竞态保护：盘已休眠
	}
	if sj.Temperature != nil && sj.Temperature.Current != nil {
		v := *sj.Temperature.Current
		return &v, true
	}
	return nil, true
}

// sampleCapacity 低频调用：解析挂载表 statfs 容量。只读 VFS 超级块缓存，不产生磁盘 IO。
//
// 容器内 bind 挂的是宿主 procfs，但 /proc/mounts 按当前 mount namespace 解析，
// 因此读到的挂载点是容器视角路径（如 /host/root/vol5），可直接 statfs；
// 去掉 root 前缀后得到宿主路径（/vol5），用于识别数据卷与归属物理盘。
func (s *diskSampler) sampleCapacity() {
	b, err := os.ReadFile(filepath.Join(s.proc, "mounts"))
	if err != nil {
		return
	}

	type agg struct {
		used, total uint64
		devs        map[string]bool // 同设备 bind 挂载去重
	}
	totals := map[string]*agg{}
	volByDev := map[string]int{}
	var vols []Volume

	for _, line := range strings.Split(string(b), "\n") {
		f := strings.Fields(line)
		if len(f) < 3 || !strings.HasPrefix(f[0], "/dev/") {
			continue
		}
		dev, mp, fstype := f[0], unescapeMount(f[1]), f[2]
		used, total, ok := fsUsedTotal(mp)
		if !ok || total == 0 {
			continue // 路径不可达或零块设备（伪挂载/特殊根），丢弃
		}
		hostPath := s.toHostPath(mp)

		// ① 可归属整盘的分区直挂（ext4/xfs 等传统布局），按整盘汇总。
		if isDataFS(fstype) && isHostDataPath(hostPath) {
			if name, ok := wholeDiskOf(dev); ok {
				a := totals[name]
				if a == nil {
					a = &agg{devs: map[string]bool{}}
					totals[name] = a
				}
				if !a.devs[dev] {
					a.devs[dev] = true
					a.used += used
					a.total += total
				}
			}
			// ② 卷管理器上的逻辑卷（fnOS trim_*、群晖、QNAP CACHEDEV 等），
			//    无法归属单盘，按卷呈现；同设备多 bind 只取一次。
			if dataVolRe.MatchString(strings.TrimPrefix(filepath.Clean(hostPath), "/")) {
				if _, seen := volByDev[dev]; !seen {
					volByDev[dev] = len(vols)
					devs := backingDisks(s.sys, dev)
					vols = append(vols, Volume{
						Name:       volumeName(hostPath),
						Fs:         fstype,
						UsedBytes:  used,
						TotalBytes: total,
						Devices:    devs,
					})
				}
			}
		}
	}
	sort.Slice(vols, func(i, j int) bool { return vols[i].Name < vols[j].Name })

	s.mu.Lock()
	defer s.mu.Unlock()
	s.volumes = vols
	for name, a := range totals {
		d, ok := s.disks[name]
		if !ok || len(a.devs) == 0 {
			continue
		}
		u, t := a.used, a.total
		d.usedBytes, d.totalBytes = &u, &t
	}
	// 未挂载文件系统的整盘（NVMe 数据卷、休眠未挂载盘）：挂载表给不出容量，
	// 用 sysfs 块设备 size 兜底总容量，已用量保持 null。
	for _, name := range s.order {
		d := s.disks[name]
		if d.totalBytes == nil {
			d.totalBytes = sysBlockTotal(s.sys, name)
		}
	}
}

// toHostPath 把容器视角挂载点还原成宿主路径：/host/root/vol5 -> /vol5；
// 非容器直跑（root 为空）时挂载点即宿主路径，原样返回。
func (s *diskSampler) toHostPath(mp string) string {
	if s.root == "" {
		return mp
	}
	if mp == s.root {
		return "/"
	}
	if p := strings.TrimPrefix(mp, s.root); strings.HasPrefix(p, "/") {
		return p
	}
	return mp
}

// isHostDataPath 排除容器系统路径的 bind 挂载（/etc/hostname 等）。
func isHostDataPath(hostPath string) bool {
	switch filepath.Clean(hostPath) {
	case "/etc", "/proc", "/sys", "/dev", "/run":
		return false
	}
	return !strings.HasPrefix(hostPath, "/etc/")
}

func isDataFS(fstype string) bool {
	switch fstype {
	case "ext2", "ext3", "ext4", "btrfs", "xfs", "f2fs":
		return true
	}
	return false
}

// volumeName 取宿主路径的末段作为卷显示名：/vol5 -> vol5，
// /share/CACHEDEV1_DATA -> CACHEDEV1_DATA。
func volumeName(hostPath string) string {
	return filepath.Base(filepath.Clean(hostPath))
}

// wholeDiskOf 把挂载设备（含分区）归并到整盘名：sda1->sda、nvme0n1p1->nvme0n1。
func wholeDiskOf(dev string) (string, bool) {
	base := strings.TrimPrefix(dev, "/dev/")
	if wholeDiskRe.MatchString(base) {
		return base, true
	}
	stripped := strings.TrimSuffix(strings.TrimRight(base, "0123456789"), "p")
	if wholeDiskRe.MatchString(stripped) {
		return stripped, true
	}
	return "", false
}

// backingDisks 解析卷的底层物理盘列表。
//
// 通用流程（不假设固定层级，兼容 dm/md/LVM/bcache 等任意中间层）：
//  1. 把挂载源设备名解析为 sysfs 中的块设备名（dm-N、mdN 等）：
//     - 已在 /sys/block 下的直接用；
//     - /dev/mapper/<name> 先解 symlink 到 dm-N，再按 dm/name 兜底扫描；
//  2. 从该块设备出发递归 /sys/block/<dev>/slaves；
//  3. 叶子是整盘（sdX/nvmeXnY/mmcblkX）→ 收录；
//     叶子是分区（sdX1/nvmeXnYp1/mmcblkXp1）→ 去分区号归并整盘。
//
// 纯读 sysfs，零 IO，不触碰设备，休眠盘安全；多盘 RAID 全列。
func backingDisks(sys, dev string) []string {
	base := resolveBlockName(sys, dev)
	if base == "" {
		return nil
	}
	found := map[string]bool{}
	resolveSlaves(sys, base, found, 0)
	// btrfs 多盘卷：挂载源只指向其中一块盘，通过 /sys/fs/btrfs/<uuid>/devices
	// 补全该卷所在 btrfs 文件系统的所有成员盘（纯 sysfs，零 IO）。
	btrfsMembers(sys, base, found)
	names := make([]string, 0, len(found))
	for n := range found {
		names = append(names, "/dev/"+n)
	}
	sort.Strings(names)
	return names
}

// resolveBlockName 把 /proc/mounts 里的设备名解析为 /sys/block 下的块设备名。
// 处理 /dev/sda1、/dev/dm-3、/dev/md127、/dev/mapper/<name>、/dev/trim_vol 等。
func resolveBlockName(sys, dev string) string {
	base := strings.TrimPrefix(dev, "/dev/")
	// 已是整盘
	if wholeDiskRe.MatchString(base) {
		return base
	}
	// /dev/mapper/<name>：优先解 symlink（→ ../dm-N），失败再按 dm/name 扫描
	if strings.HasPrefix(base, "mapper/") {
		name := strings.TrimPrefix(base, "mapper/")
		if target, err := os.Readlink(filepath.Join("/dev", "mapper", name)); err == nil {
			if t := filepath.Base(target); strings.HasPrefix(t, "dm-") {
				return t
			}
		}
		base = name
	}
	// 直接命名的块设备（dm-N、mdN、loopN 等）在 sysfs 下存在即可用
	if _, err := os.Stat(filepath.Join(sys, "block", base)); err == nil {
		return base
	}
	// 兜底：扫描所有 dm-N，用 dm/name 匹配
	if entries, err := os.ReadDir(filepath.Join(sys, "block")); err == nil {
		for _, e := range entries {
			if !strings.HasPrefix(e.Name(), "dm-") {
				continue
			}
			b, err := os.ReadFile(filepath.Join(sys, "block", e.Name(), "dm", "name"))
			if err == nil && strings.TrimSpace(string(b)) == base {
				return e.Name()
			}
		}
	}
	// 最后兜底：按分区名归并整盘
	if w, ok := wholeDiskOf(base); ok {
		return w
	}
	return ""
}

// btrfsMembers 扫描所有已挂载 btrfs 文件系统的成员盘，若 base（或其归并整盘）
// 是某 btrfs 文件系统的成员，则把该文件系统所有成员盘加入 out。
// 纯读 /sys/fs/btrfs，不触碰设备，休眠盘安全。
func btrfsMembers(sys, base string, out map[string]bool) {
	root := filepath.Join(sys, "fs", "btrfs")
	fsUUIDs, err := os.ReadDir(root)
	if err != nil {
		return
	}
	want := map[string]bool{base: true}
	if w, ok := wholeDiskOf(base); ok {
		want[w] = true
	}
	for _, fs := range fsUUIDs {
		devDir := filepath.Join(root, fs.Name(), "devices")
		devs, err := os.ReadDir(devDir)
		if err != nil || len(devs) == 0 {
			continue
		}
		// 成员设备原始名（可能是整盘 sda、分区 sda3 或 dm-N），保留原名用于匹配
		raw := make([]string, 0, len(devs))
		match := false
		for _, d := range devs {
			n := d.Name()
			raw = append(raw, n)
			// base（或其归并整盘）命中该成员 → 该 btrfs 就是目标文件系统
			if want[n] {
				match = true
			}
			if w, ok := wholeDiskOf(n); ok && want[w] {
				match = true
			}
		}
		if match {
			// 对每个成员设备递归 slaves 解析到物理整盘：
			// dm-0 → sda3 → sda；多盘 RAID 各盘全部列出。
			for _, n := range raw {
				resolveSlaves(sys, n, out, 0)
			}
			return
		}
	}
}

// resolveSlaves 沿 /sys/block/<name>/slaves 递归到物理整盘；叶子分区归并整盘。
func resolveSlaves(sys, name string, out map[string]bool, depth int) {
	if depth > 8 {
		return
	}
	if wholeDiskRe.MatchString(name) {
		out[name] = true
		return
	}
	slaves, err := os.ReadDir(filepath.Join(sys, "block", name, "slaves"))
	if err != nil || len(slaves) == 0 {
		// 叶子：分区则归并整盘，否则忽略
		if w, ok := wholeDiskOf(name); ok {
			out[w] = true
		}
		return
	}
	for _, sl := range slaves {
		resolveSlaves(sys, sl.Name(), out, depth+1)
	}
}

// unescapeMount 还原 /proc/mounts 八进制转义（空格 \040 等）。
func unescapeMount(p string) string {
	if !strings.Contains(p, `\`) {
		return p
	}
	var b strings.Builder
	for i := 0; i < len(p); i++ {
		if p[i] == '\\' && i+3 < len(p) {
			if v, err := strconv.ParseUint(p[i+1:i+4], 8, 32); err == nil {
				b.WriteRune(rune(v))
				i += 3
				continue
			}
		}
		b.WriteByte(p[i])
	}
	return b.String()
}
