package pkg

import (
	"context"
	"encoding/binary"
	"encoding/json"
	"fmt"
	"strings"
	"sync"
	"time"

	"github.com/dgraph-io/badger/v4"
	"github.com/dgraph-io/badger/v4/options"
)

const keyPrefixFailedStr = "failed:"

func keyPrefixFailed() []byte {
	return []byte(keyPrefixFailedStr)
}

func keyPrefixBlockedCountry() []byte {
	return []byte("blocked:country:")
}

func keyPrefixBlockedASN() []byte {
	return []byte("blocked:asn:")
}

func keyPrefixASN() []byte {
	return []byte("asn:")
}

func strKeyPrefixBlockedCountry() string {
	return "blocked:country:"
}

func strKeyPrefixBlockedASN() string {
	return "blocked:asn:"
}

func strKeyPrefixASN() string {
	return "asn:"
}

type StorageManager struct {
	db      *badger.DB
	ttlDays int
}

func NewStorageManager(dbDir string, ttlDays int) (*StorageManager, error) {
	db, err := badger.Open(badger.DefaultOptions(dbDir).
		WithSyncWrites(true).
		WithMemTableSize(64 << 20).
		WithValueLogFileSize(256 << 20).
		WithCompression(options.ZSTD).
		WithNumVersionsToKeep(1).
		WithCompactL0OnClose(true))
	if err != nil {
		return nil, err
	}
	return &StorageManager{db: db, ttlDays: ttlDays}, nil
}

func (s *StorageManager) Close() error {
	s.db.RunValueLogGC(0.5)
	return s.db.Close()
}

func (s *StorageManager) DB() *badger.DB {
	return s.db
}

func (s *StorageManager) IsFailedRecently(domain string, now int64) bool {
	key := append(keyPrefixFailed(), domain...)
	var lastFail int64
	err := s.db.View(func(txn *badger.Txn) error {
		item, err := txn.Get(key)
		if err != nil {
			return err
		}
		val, err := item.ValueCopy(nil)
		if err != nil || len(val) != 8 {
			return err
		}
		lastFail = int64(binary.LittleEndian.Uint64(val))
		return nil
	})
	if err != nil {
		return false
	}
	ttlSec := int64(s.ttlDays * 24 * 3600)
	return (now - lastFail) < ttlSec
}

func (s *StorageManager) CleanAndCountFailure(now int64, ttlSec int64) (int, int) {
	active := 0
	_ = s.db.View(func(txn *badger.Txn) error {
		iter := txn.NewIterator(badger.DefaultIteratorOptions)
		defer iter.Close()
		prefix := keyPrefixFailed()
		for iter.Seek(prefix); iter.ValidForPrefix(prefix); iter.Next() {
			active++
		}
		return nil
	})
	return active, 0
}

func (s *StorageManager) AppendFailureHistory(domains []string) {
	if len(domains) == 0 {
		return
	}
	now := time.Now().Unix()
	ttl := time.Duration(s.ttlDays) * 24 * time.Hour

	wb := s.db.NewWriteBatch()
	defer wb.Cancel()

	for _, d := range domains {
		key := append(keyPrefixFailed(), d...)
		buf := make([]byte, 8)
		binary.LittleEndian.PutUint64(buf, uint64(now))
		wb.SetEntry(&badger.Entry{
			Key:       key,
			Value:     buf,
			ExpiresAt: uint64(now) + uint64(ttl.Seconds()),
		})
	}
	wb.Flush()

	lastKey := append(keyPrefixFailed(), domains[len(domains)-1]...)
	err := s.db.View(func(txn *badger.Txn) error {
		_, err := txn.Get(lastKey)
		return err
	})
	if err != nil {
		fmt.Printf("[WriteVerify] Warning: failed to verify write for %s: %v\n", domains[len(domains)-1], err)
	}
}

func (s *StorageManager) LoadSuccessHistory() map[string]struct{} {
	m := make(map[string]struct{})
	_ = s.db.View(func(txn *badger.Txn) error {
		iter := txn.NewIterator(badger.IteratorOptions{PrefetchValues: false})
		defer iter.Close()
		prefix := keyPrefixSuccess()
		for iter.Seek(prefix); iter.ValidForPrefix(prefix); iter.Next() {
			key := string(iter.Item().Key())
			domain := strings.TrimPrefix(key, strKeyPrefixSuccess())
			m[domain] = struct{}{}
		}
		return nil
	})
	return m
}

func (s *StorageManager) LoadBlockedHistory() map[string]struct{} {
	m := make(map[string]struct{})
	_ = s.db.View(func(txn *badger.Txn) error {
		iter := txn.NewIterator(badger.IteratorOptions{PrefetchValues: false})
		defer iter.Close()

		prefixCountry := keyPrefixBlockedCountry()
		for iter.Seek(prefixCountry); iter.ValidForPrefix(prefixCountry); iter.Next() {
			key := string(iter.Item().Key())
			domain := strings.TrimPrefix(key, strKeyPrefixBlockedCountry())
			m[domain] = struct{}{}
		}

		prefixASN := keyPrefixBlockedASN()
		for iter.Seek(prefixASN); iter.ValidForPrefix(prefixASN); iter.Next() {
			key := string(iter.Item().Key())
			domain := strings.TrimPrefix(key, strKeyPrefixBlockedASN())
			m[domain] = struct{}{}
		}
		return nil
	})
	return m
}

func (s *StorageManager) LoadASNBlocklist() *sync.Map {
	var asnBlocklist sync.Map
	now := time.Now().Unix()
	for asn, org := range SeedBlockedASNs {
		asnBlocklist.Store(asn, ASNInfo{Org: org, Country: "SEED", AddedAt: now})
	}
	_ = s.db.View(func(txn *badger.Txn) error {
		iter := txn.NewIterator(badger.DefaultIteratorOptions)
		defer iter.Close()
		prefix := keyPrefixASN()
		for iter.Seek(prefix); iter.ValidForPrefix(prefix); iter.Next() {
			item := iter.Item()
			key := string(item.Key())
			asnStr := strings.TrimPrefix(key, strKeyPrefixASN())
			var asn uint32
			fmt.Sscanf(asnStr, "%d", &asn)
			val, _ := item.ValueCopy(nil)
			var info ASNInfo
			if err := json.Unmarshal(val, &info); err == nil {
				asnBlocklist.Store(asn, info)
			}
		}
		return nil
	})
	return &asnBlocklist
}

func (s *StorageManager) AddASNToBlocklist(asn uint32, org, country string) {
	info := ASNInfo{
		Org:     org,
		Country: country,
		AddedAt: time.Now().Unix(),
	}
	data, _ := json.Marshal(info)
	key := append(keyPrefixASN(), fmt.Sprintf("%d", asn)...)

	ttl := time.Duration(s.ttlDays) * 24 * time.Hour
	now := time.Now().Unix()

	_ = s.db.Update(func(txn *badger.Txn) error {
		return txn.SetEntry(&badger.Entry{
			Key:       key,
			Value:     data,
			ExpiresAt: uint64(now) + uint64(ttl.Seconds()),
		})
	})
}

func (s *StorageManager) AddBlockedDomain(domain, reason, code string) {
	info := BlockedInfo{
		Domain:   domain,
		Reason:   reason,
		Code:     code,
		TestedAt: time.Now().Unix(),
	}
	data, _ := json.Marshal(info)

	var key []byte
	if reason == "COUNTRY" {
		key = append(keyPrefixBlockedCountry(), domain...)
	} else if reason == "ASN" {
		key = append(keyPrefixBlockedASN(), domain...)
	} else {
		key = append(keyPrefixBlockedCountry(), domain...)
	}

	ttl := time.Duration(s.ttlDays) * 24 * time.Hour
	now := time.Now().Unix()

	_ = s.db.Update(func(txn *badger.Txn) error {
		return txn.SetEntry(&badger.Entry{
			Key:       key,
			Value:     data,
			ExpiresAt: uint64(now) + uint64(ttl.Seconds()),
		})
	})
}

func (s *StorageManager) ClearAll() error {
	if s.db == nil {
		return nil
	}
	err := s.db.DropAll()
	if err != nil {
		return err
	}
	s.db.RunValueLogGC(0.5)
	return nil
}

func (s *StorageManager) GetASNBlocklistCount(blocklist *sync.Map) int {
	count := 0
	blocklist.Range(func(key, value interface{}) bool {
		count++
		return true
	})
	return count
}

func (s *StorageManager) SaveSuccess(domain, country string) error {
	now := time.Now().Unix()
	info := SuccessInfo{
		Domain:   domain,
		Country:  country,
		TestedAt: now,
	}
	data, _ := json.Marshal(info)
	key := append(keyPrefixSuccess(), domain...)
	ttl := time.Duration(s.ttlDays) * 24 * time.Hour
	return s.db.Update(func(txn *badger.Txn) error {
		return txn.SetEntry(&badger.Entry{
			Key:       key,
			Value:     data,
			ExpiresAt: uint64(now) + uint64(ttl.Seconds()),
		})
	})
}

func (s *StorageManager) StartGC(ctx context.Context) {
	ticker := time.NewTicker(15 * time.Minute)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			s.db.RunValueLogGC(0.3)
		}
	}
}

func (s *StorageManager) VerifyIntegrity() int {
	var corruptCount int
	s.db.View(func(txn *badger.Txn) error {
		iter := txn.NewIterator(badger.DefaultIteratorOptions)
		defer iter.Close()
		count := 0
		for iter.Seek([]byte("failed:")); count < 100; iter.Next() {
			if !iter.Valid() {
				break
			}
			_, err := iter.Item().ValueCopy(nil)
			if err != nil {
				corruptCount++
			}
			count++
		}
		return nil
	})
	return corruptCount
}
