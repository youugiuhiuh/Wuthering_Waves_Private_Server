package pkg

import (
	"container/list"
	"sync"
)

type entry[K comparable, V any] struct {
	key   K
	value V
}

type LRU[K comparable, V any] struct {
	mu      sync.Mutex
	maxSize int
	items   map[K]*list.Element
	order   *list.List
}

func NewLRU[K comparable, V any](maxSize int) *LRU[K, V] {
	return &LRU[K, V]{
		maxSize: maxSize,
		items:   make(map[K]*list.Element),
		order:   list.New(),
	}
}

func (c *LRU[K, V]) Get(key K) (V, bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	elem, ok := c.items[key]
	if !ok {
		var zero V
		return zero, false
	}
	c.order.MoveToFront(elem)
	return elem.Value.(*entry[K, V]).value, true
}

func (c *LRU[K, V]) Set(key K, val V) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if elem, ok := c.items[key]; ok {
		c.order.MoveToFront(elem)
		elem.Value.(*entry[K, V]).value = val
		return
	}
	elem := c.order.PushFront(&entry[K, V]{key: key, value: val})
	c.items[key] = elem
	if c.order.Len() > c.maxSize {
		tail := c.order.Back()
		if tail != nil {
			c.order.Remove(tail)
			delete(c.items, tail.Value.(*entry[K, V]).key)
		}
	}
}
