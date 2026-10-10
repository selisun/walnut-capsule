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
}

type CPU struct {
	Usage float64  `json:"usage"` // 0~100
	Temp  *float64 `json:"temp"`  // 摄氏度，无法读取时为 null
}

type Memory struct {
	Usage float64 `json:"usage"` // 0~100
	Used  uint64  `json:"used"`  // 字节
	Total uint64  `json:"total"` // 字节
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
	Index       int    `json:"index"`
	Name        string `json:"name"`
	Temp        *int   `json:"temp"`
	Utilization *int   `json:"utilization"` // 0~100
	MemUsed     uint64 `json:"memory_used"`
	MemTotal    uint64 `json:"memory_total"`
}

type NetIf struct {
	Name      string `json:"interface"`
	RxBPS     uint64 `json:"rx_bytes_per_sec"`
	TxBPS     uint64 `json:"tx_bytes_per_sec"`
	SpeedMbps int    `json:"speed_mbps"` // 协商速率（Mbps），0 表示未知（网卡 down 或读不到）
}
