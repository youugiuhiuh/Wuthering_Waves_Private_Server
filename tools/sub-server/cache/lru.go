package cache

import (
	"container/list"
	"sync"
	"time"
)

type entry struct {
	key       string
	value     interface{}
	createdAt time.Time
}

type LRU struct {
	mu    sync.Mutex
	max   int
	ttl   time.Duration
	ll    *list.List
	items map[string]*list.Element
}

func New(max int) *LRU {
	return &LRU{
		max:   max,
		ttl:   60 * time.Second,
		ll:    list.New(),
		items: make(map[string]*list.Element),
	}
}

func NewWithTTL(max int, ttl time.Duration) *LRU {
	return &LRU{
		max:   max,
		ttl:   ttl,
		ll:    list.New(),
		items: make(map[string]*list.Element),
	}
}

func (c *LRU) Get(key string) (interface{}, bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if elem, ok := c.items[key]; ok {
		e := elem.Value.(*entry)
		if c.ttl > 0 && time.Since(e.createdAt) > c.ttl {
			c.ll.Remove(elem)
			delete(c.items, key)
			return nil, false
		}
		c.ll.MoveToFront(elem)
		return e.value, true
	}
	return nil, false
}

func (c *LRU) Set(key string, value interface{}) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if elem, ok := c.items[key]; ok {
		c.ll.MoveToFront(elem)
		elem.Value.(*entry).value = value
		elem.Value.(*entry).createdAt = time.Now()
		return
	}
	if c.ll.Len() >= c.max {
		oldest := c.ll.Back()
		if oldest != nil {
			c.ll.Remove(oldest)
			delete(c.items, oldest.Value.(*entry).key)
		}
	}
	elem := c.ll.PushFront(&entry{key, value, time.Now()})
	c.items[key] = elem
}
