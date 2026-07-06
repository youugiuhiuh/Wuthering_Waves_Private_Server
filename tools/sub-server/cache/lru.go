package cache

import (
	"container/list"
	"sync"
)

type entry struct {
	key   string
	value interface{}
}

type LRU struct {
	mu    sync.Mutex
	max   int
	ll    *list.List
	items map[string]*list.Element
}

func New(max int) *LRU {
	return &LRU{
		max:   max,
		ll:    list.New(),
		items: make(map[string]*list.Element),
	}
}

func (c *LRU) Get(key string) (interface{}, bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if elem, ok := c.items[key]; ok {
		c.ll.MoveToFront(elem)
		return elem.Value.(*entry).value, true
	}
	return nil, false
}

func (c *LRU) Set(key string, value interface{}) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if elem, ok := c.items[key]; ok {
		c.ll.MoveToFront(elem)
		elem.Value.(*entry).value = value
		return
	}
	if c.ll.Len() >= c.max {
		oldest := c.ll.Back()
		if oldest != nil {
			c.ll.Remove(oldest)
			delete(c.items, oldest.Value.(*entry).key)
		}
	}
	elem := c.ll.PushFront(&entry{key, value})
	c.items[key] = elem
}
