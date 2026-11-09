//go:build !linux

package server

import (
	"context"
	"net"
)

// ConnContext 在非 Linux 构建下不做对端 UID 解析（本地开发/交叉编译用）。
func ConnContext(ctx context.Context, c net.Conn) context.Context { return ctx }

func PeerUID(ctx context.Context) (uint32, bool) { return 0, false }
