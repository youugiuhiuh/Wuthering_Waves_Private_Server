package pkg

import (
	"sync"
	"testing"
)

func TestLRUPutGet(t *testing.T) {
	c := NewLRU[string, int](3)
	c.Set("a", 1)
	v, ok := c.Get("a")
	if !ok || v != 1 {
		t.Fatalf("expected (1,true) got (%v,%v)", v, ok)
	}
}

func TestLRUMiss(t *testing.T) {
	c := NewLRU[string, int](3)
	_, ok := c.Get("missing")
	if ok {
		t.Fatal("expected false for missing key")
	}
}

func TestLRUEvict(t *testing.T) {
	c := NewLRU[string, int](2)
	c.Set("a", 1)
	c.Set("b", 2)
	c.Set("c", 3) // should evict "a"
	_, ok := c.Get("a")
	if ok {
		t.Fatal("expected 'a' to be evicted")
	}
	v, ok := c.Get("b")
	if !ok || v != 2 {
		t.Fatal("expected b=2")
	}
	v, ok = c.Get("c")
	if !ok || v != 3 {
		t.Fatal("expected c=3")
	}
}

func TestLRUUpdatePreservesOrder(t *testing.T) {
	c := NewLRU[string, int](2)
	c.Set("a", 1)
	c.Set("b", 2)
	c.Set("a", 10) // move "a" to front
	c.Set("c", 3)  // evicts "b", not "a"
	_, ok := c.Get("b")
	if ok {
		t.Fatal("expected 'b' to be evicted")
	}
	v, ok := c.Get("a")
	if !ok || v != 10 {
		t.Fatal("expected a=10")
	}
}

func TestLRUConcurrent(t *testing.T) {
	c := NewLRU[int, int](1000)
	var wg sync.WaitGroup
	for i := 0; i < 100; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for j := 0; j < 100; j++ {
				c.Set(j, j)
				c.Get(j)
			}
		}()
	}
	wg.Wait()
}
