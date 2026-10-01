package main

import (
	"flag"
	"log"
	"net/http"
	"time"

	"hetao-nas/internal/metrics"
	"hetao-nas/internal/server"
)

func main() {
	addr := flag.String("addr", ":8765", "listen address")
	flag.Parse()

	col := metrics.NewCollector()
	col.Run()

	srv := &http.Server{
		Addr:              *addr,
		Handler:           server.New(col).Handler(),
		ReadHeaderTimeout: 5 * time.Second,
	}
	log.Printf("hetao-nas listening on %s", *addr)
	log.Fatal(srv.ListenAndServe())
}
