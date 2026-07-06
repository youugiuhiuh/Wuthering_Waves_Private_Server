package middleware

import (
	"net/http"
	"sync"

	"github.com/go-chi/chi/v5"
	"golang.org/x/time/rate"
)

type RateLimiter struct {
	mu       sync.RWMutex
	visitors map[string]*rate.Limiter
	rps      rate.Limit
	burst    int
}

func NewRateLimiter(rps int) *RateLimiter {
	burst := rps * 2
	if burst < 1 {
		burst = 1
	}
	return &RateLimiter{
		visitors: make(map[string]*rate.Limiter),
		rps:      rate.Limit(rps) / 60.0,
		burst:    burst,
	}
}

func (rl *RateLimiter) getLimiter(key string) *rate.Limiter {
	rl.mu.RLock()
	limiter, ok := rl.visitors[key]
	rl.mu.RUnlock()
	if ok {
		return limiter
	}
	limiter = rate.NewLimiter(rl.rps, rl.burst)
	rl.mu.Lock()
	rl.visitors[key] = limiter
	rl.mu.Unlock()
	return limiter
}

func RateLimit(rps int) func(http.Handler) http.Handler {
	rl := NewRateLimiter(rps)
	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			token := chi.URLParam(r, "token")
			if token == "" {
				token = r.RemoteAddr
			}
			if !rl.getLimiter(token).Allow() {
				http.Error(w, "rate limit exceeded", http.StatusTooManyRequests)
				return
			}
			next.ServeHTTP(w, r)
		})
	}
}
