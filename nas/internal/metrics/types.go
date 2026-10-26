package metrics

// Snapshot 全量采样结果，/metrics 与 /ws 推送共用此结构。
// 缺失数据以 null 表示（指针字段），不报错。
type Snapshot struct {
	Timestamp int64    `json:"timestamp"`
	Hostname  string   `json:"hostname"` // NAS 主机名（宿主机 UTS 名）
	CPU       CPU      `json:"cpu"`
	Memory    Memory   `json:"memory"`
	Disks     []Disk   `json:"disks"`
	Volumes   []Volume `json:"volumes"` // 逻辑存储空间（fnOS /volN、群晖 /volumeN、QNAP /share/…）
	GPUs      []GPU    `json:"gpus"`
	Network   []NetIf  `json:"network"`
	// Traffic 是目标网卡的累计流量（今日 / 昨日 / 7 日 / 30 日，收发分开），读不到时为 null
	Traffic *Traffic `json:"traffic,omitempty"`
}

// Traffic 目标网卡的累计流量（字节）：今日 / 昨日 / 最近 7 日 / 最近 30 日，均含当天，收发分开。
type Traffic struct {
	Iface       string `json:"iface"`
	RxToday     uint64 `json:"rx_today"`
	TxToday     uint64 `json:"tx_today"`
	RxYesterday uint64 `json:"rx_yesterday"`
	TxYesterday uint64 `json:"tx_yesterday"`
	RxDays7     uint64 `json:"rx_days7"`
	TxDays7     uint64 `json:"tx_days7"`
	RxDays30    uint64 `json:"rx_days30"`
	TxDays30    uint64 `json:"tx_days30"`
	// DaysRx / DaysTx 是最近 30 天的按天序列（旧 → 新，固定 30 项），供柱状图
	DaysRx []uint64 `json:"days_rx"`
	DaysTx []uint64 `json:"days_tx"`
	// Dates 与上面两条序列一一对应，格式 "10-25"（不含年份），供悬停提示显示日期
	Dates []string `json:"dates"`
}

type CPU struct {
	Usage float64  `json:"usage"` // 0~100
	Temp  *float64 `json:"temp"`  // 摄氏度，无法读取时为 null
	// 平均负载（/proc/loadavg 的 1 / 5 / 15 分钟值），无法读取时为 null
	Load1  *float64 `json:"load1,omitempty"`
	Load5  *float64 `json:"load5,omitempty"`
	Load15 *float64 `json:"load15,omitempty"`
	// Load30 是本应用自算的 30 分钟平均负载（对 load1 采样取均值）：
	// 内核只提供 1 / 5 / 15 分钟三档；采样覆盖满 30 分钟后才有值，否则为 null
	Load30 *float64 `json:"load30,omitempty"`
}

type Memory struct {
	Usage float64 `json:"usage"` // 0~100（已用 / 总计，已用不含缓存）
	Used  uint64  `json:"used"`  // 字节：已用（不含缓存）
	Total uint64  `json:"total"` // 字节
	Cache uint64  `json:"cache"` // 字节：Buffers + Cached
	Free  uint64  `json:"free"`  // 字节：可用（MemFree）
}

type Disk struct {
	Device     string  `json:"device"` // /dev/sda
	Status     string  `json:"status"` // active / standby / unknown
	Temp       *int    `json:"temp"`   // 休眠时为 null（绝不主动唤醒）
	ReadBPS    uint64  `json:"read_bytes_per_sec"`
	WriteBPS   uint64  `json:"write_bytes_per_sec"`
	UsedBytes  *uint64 `json:"used_bytes"`  // 已用容量（字节），无法读取为 null
	TotalBytes *uint64 `json:"total_bytes"` // 总容量（字节），无法读取为 null
}

// Volume 逻辑存储空间：LVM/btrfs/RAID 等卷管理器上的数据卷，
// 无法归属到单块物理盘（fnOS trim 卷、群晖 volume、QNAP CACHEDEV 等）。
type Volume struct {
	Name       string   `json:"name"`        // 卷显示名（vol5 / CACHEDEV1_DATA）
	Fs         string   `json:"fs"`          // 文件系统（btrfs / ext4 / xfs）
	UsedBytes  uint64   `json:"used_bytes"`  // 已用字节
	TotalBytes uint64   `json:"total_bytes"` // 总字节
	Devices    []string `json:"devices"`     // 底层物理盘（/dev/sdb 等），多盘 RAID 全列；查不到为空
}

type GPU struct {
	Index       int     `json:"index"`
	Name        string  `json:"name"`
	Temp        *int    `json:"temp"`
	Utilization *int    `json:"utilization"`           // 0~100
	MemUsed     *uint64 `json:"memory_used,omitempty"` // 核显共享内存，无独立显存时为 null
	MemTotal    *uint64 `json:"memory_total,omitempty"`
}

type NetIf struct {
	Name      string `json:"interface"`
	RxBPS     uint64 `json:"rx_bytes_per_sec"`
	TxBPS     uint64 `json:"tx_bytes_per_sec"`
	SpeedMbps int    `json:"speed_mbps"` // 协商速率（Mbps），0 表示未知（网卡 down 或读不到）
}
