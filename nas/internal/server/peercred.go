package server

import (
	"context"
	"log"
	"os"
	"strconv"
	"strings"
	"sync"
)

var (
	deniedPeerUIDs  sync.Map // uid -> struct{}，拒绝日志每个 UID 只记录一次，避免被打爆
	allowedUIDsOnce sync.Once
	allowedUIDsMap  map[uint32]bool
)

// defaultPeerUIDs 是未配置 ADMIN_SOCKET_ALLOW_UIDS 时允许的 UID：
// 0 = 服务自身（root），65534 = 网关 worker（nobody）。
// 这样默认就挡掉本机其它用户的进程直连 socket 读取/重置令牌。
var defaultPeerUIDs = map[uint32]bool{0: true, 65534: true}

// peerAllowed 校验管理 socket 的对端 UID。
// 配置了 ADMIN_SOCKET_ALLOW_UIDS 时严格按白名单；未配置时只允许 defaultPeerUIDs。
// 特殊部署可用 ADMIN_SOCKET_ALLOW_ALL=1 恢复「全部放行」（仅用于排障）。
func peerAllowed(ctx context.Context) bool {
	uid, ok := PeerUID(ctx)
	if !ok {
		// 非 Linux / 拿不到凭据时无法校验，保持原有行为不阻断
		return true
	}
	allowed := allowedUIDs()
	if len(allowed) == 0 {
		if allowAllPeers() {
			return true
		}
		if defaultPeerUIDs[uid] {
			return true
		}
		if _, loaded := deniedPeerUIDs.LoadOrStore(uid, struct{}{}); !loaded {
			log.Printf("admin socket peer denied: uid=%d (set ADMIN_SOCKET_ALLOW_UIDS to allow)", uid)
		}
		return false
	}
	if allowed[uid] {
		return true
	}
	if _, loaded := deniedPeerUIDs.LoadOrStore(uid, struct{}{}); !loaded {
		log.Printf("admin socket peer denied: uid=%d", uid)
	}
	return false
}

func allowAllPeers() bool {
	v := strings.TrimSpace(os.Getenv("ADMIN_SOCKET_ALLOW_ALL"))
	return v == "1" || strings.EqualFold(v, "true")
}

func allowedUIDs() map[uint32]bool {
	allowedUIDsOnce.Do(func() {
		allowedUIDsMap = map[uint32]bool{}
		for _, s := range strings.Split(os.Getenv("ADMIN_SOCKET_ALLOW_UIDS"), ",") {
			n, err := strconv.ParseUint(strings.TrimSpace(s), 10, 32)
			if err == nil {
				allowedUIDsMap[uint32(n)] = true
			}
		}
	})
	return allowedUIDsMap
}
