//go:build linux

package server

import (
	"context"
	"net"
	"syscall"
)

type peerUIDKey struct{}

// ConnContext 把 unix socket 对端 UID 放进请求 context。
// 仅对 *net.UnixConn 生效；LAN TCP 连接不设置该值。
func ConnContext(ctx context.Context, c net.Conn) context.Context {
	uc, ok := c.(*net.UnixConn)
	if !ok {
		return ctx
	}
	raw, err := uc.SyscallConn()
	if err != nil {
		return ctx
	}
	var cred *syscall.Ucred
	var credErr error
	if err := raw.Control(func(fd uintptr) {
		cred, credErr = syscall.GetsockoptUcred(int(fd), syscall.SOL_SOCKET, syscall.SO_PEERCRED)
	}); err != nil || credErr != nil || cred == nil {
		return ctx
	}
	return context.WithValue(ctx, peerUIDKey{}, uint32(cred.Uid))
}

// PeerUID 返回 unix socket 对端 UID；非 unix socket 或读取失败时 ok=false。
func PeerUID(ctx context.Context) (uint32, bool) {
	v, ok := ctx.Value(peerUIDKey{}).(uint32)
	return v, ok
}
