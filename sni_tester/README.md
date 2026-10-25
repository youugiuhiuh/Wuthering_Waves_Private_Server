# SNI Tester — High-Performance Three-Phase Censorship-Resistant Probe Engine
[English](#english) | [فارسی (Persian)](#فارسی-persian) | [Русский (Russian)](#русский-russian)

---

<a name="english"></a>
## English

High-performance concurrent SNI testing and Post-Quantum (PQ) camouflage domain management engine, tailored for anti-censorship systems (REALITY / XHTTP).

### The Three-Phase Decoupled Architecture
To achieve absolute safety and lightning-fast execution, testing is strictly decoupled into three specialized phases:

```text
  Phase 1: resolve-map (Overseas VPS)
  Input: top-1m.csv
  Output: resolved_targets.csv (domain,ip mapping)
  - Uses pure overseas Anycast DNS (Cloudflare, Google, Quad9, OpenDNS). Zero China DNS.
  - Filters Cloudflare IP ranges in advance. 100% free from GFW/censor interference.
           │
           ▼
  Phase 2: probe-survival (Inside China / Iran / Russia)
  Input: resolved_targets.csv
  Output: survived_candidates.csv
  - ZERO DNS QUERIES! Connects directly to pre-resolved IPs.
  - Minimal byte-level Chrome TLS ClientHello handshake.
  - Built-in 150ms post-handshake probe to capture delayed GFW TCP RST injection.
  - Leaves zero scanning or crawling footprint. Zero domestic ISP audit risk.
           │
           ▼
  Phase 3: build-pools (Overseas User VPS)
  Input: survived_candidates.csv
  Output: rust/aegis/src/resources/sni/*.pb
  - Reuses pre-resolved IPs without repeating DNS.
  - Evaluates TLS 1.3, Post-Quantum key exchange (X25519MLKEM768), H2 ALPN, and HTTP CDN headers.
  - Filters bloated cert chains (3500–5500B) and strictly discards CN/HK/MO/IR/RU/KP domains.
  - Incrementally merges into existing .pb country pools without losing historical assets.
```

### Quick Start (Three-Phase Workflow)

#### 1. Phase 1: Pre-Resolve (Run on Overseas VPS)
```bash
go build -o sni_tester ./cmd/sni_tester/
./sni_tester resolve-map -f top-1m.csv -o resolved_targets.csv -w 100
```

#### 2. Phase 2: Survival Probe (Run Inside Censored Region / Local Network)
```bash
./sni_tester probe-survival -f resolved_targets.csv -o survived_candidates.csv -w 50 -p socks5://127.0.0.1:10808
```

#### 3. Phase 3: Dual-Pool Builder (Run on User Overseas VPS)
```bash
./sni_tester build-pools -f survived_candidates.csv -out-dir rust/aegis/src/resources/sni -w 50
```

### CLI Subcommands & Options

| Command / Flag | Purpose | Example |
| :--- | :--- | :--- |
| `resolve-map` | Phase 1: Overseas concurrent DNS pre-resolver (`domain,ip`) | `./sni_tester resolve-map -f top-1m.csv -o resolved.csv` |
| `probe-survival` | Phase 2: Zero-DNS survival probe for censored regions | `./sni_tester probe-survival -f resolved.csv -o survived.csv` |
| `build-pools` | Phase 3: VPS deep inspector and dual-pool `.pb` generator | `./sni_tester build-pools -f survived.csv` |
| `-p` | Global outbound SOCKS5 proxy URL (TLS + DNS + GeoDB) | `-p socks5://127.0.0.1:1080` |
| `-iface` | Bind outbound traffic to network interface or IP | `-iface wlp6s0` |
| `-upgrade-dir` | In-place batch upgrade of existing `.pb` files | `-upgrade-dir rust/aegis/src/resources/sni` |
| `-upgrade-pb` | Upgrade a single `.pb` file | `-upgrade-pb rust/aegis/src/resources/sni/US.pb` |
| `-timeout` | Connection & handshake timeout in seconds | `-timeout 3` |
| `-w` | Concurrency worker count | `-w 50` |

---

<a name="فارسی-persian"></a>
## فارسی (Persian)

موتور قدرتمند و پرسرعت برای تست SNI و مدیریت دامنه‌های استتاری پساکوانتومی (Post-Quantum)، طراحی‌شده برای مقابله با فیلترینگ شدید (سیستم‌های REALITY و XHTTP).

### معماری تفکیک‌شده سه‌مرحله‌ای (Three-Phase Architecture)
برای دستیابی به نهایت امنیت و سرعت خارق‌العاده، فرآیند تست به سه فاز مستقل تقسیم شده است:

```text
  مرحله ۱: resolve-map (روی سرور خارج از کشور)
  ورودی: top-1m.csv
  خروجی: resolved_targets.csv (نگاشت دامنه به آی‌پی)
  - استفاده اختصاصی از Anycast DNSهای خارجی (Cloudflare، Google، Quad9، OpenDNS).
  - حذف پیشاپیش رنج‌های کلودفلر و دور از هرگونه اختلال یا آلودگی فیلترینگ.
           │
           ▼
  مرحله ۲: probe-survival (داخل ایران / شبکه فیلترشده)
  ورودی: resolved_targets.csv
  خروجی: survived_candidates.csv
  - بدون هیچ‌گونه درخواست DNS محلی! اتصال مستقیم به آی‌پی‌های آماده.
  - ارسال کمترین بایت ممکن با اثر انگشت استاندارد Chrome TLS ClientHello.
  - تاخیر اختصاصی ۱۵۰ میلی‌ثانیه‌ای برای شکار بسته‌های تزریقی TCP RST فیلترچی.
  - بدون هیچ ردپای اسکن یا بار اضافه بر اینترنت خانگی.
           │
           ▼
  مرحله ۳: build-pools (روی سرور خارج کاربر)
  ورودی: survived_candidates.csv
  خروجی: rust/aegis/src/resources/sni/*.pb
  - بررسی دقیق TLS 1.3، کلیدهای پساکوانتومی (X25519MLKEM768)، طول زنجیره گواهی (۳۵۰۰ تا ۵۵۰۰ بایت).
  - حذف خودکار دامنه‌های تحت کنترل سانسور (CN, IR, RU, KP).
  - ادغام افزایشی با دیتابیس‌های موجود بدون پاک شدن دامنه‌های قبلی.
```

### راهنمای اجرای سه‌مرحله‌ای

#### ۱. مرحله اول: پیش‌حل نام دامنه‌ها (روی سرور خارجی)
```bash
go build -o sni_tester ./cmd/sni_tester/
./sni_tester resolve-map -f top-1m.csv -o resolved_targets.csv -w 100
```

#### ۲. مرحله دوم: بررسی بقا در داخل کشور (ایران / بدون DNS محلی)
```bash
./sni_tester probe-survival -f resolved_targets.csv -o survived_candidates.csv -w 50 -p socks5://127.0.0.1:10808
```

#### ۳. مرحله سوم: ساخت مخزن دامنه‌ها (روی سرور خارجی کاربر)
```bash
./sni_tester build-pools -f survived_candidates.csv -out-dir rust/aegis/src/resources/sni -w 50
```

---

<a name="русский-russian"></a>
## Русский (Russian)

Высокопроизводительный движок параллельного тестирования SNI и управления маскировочными доменами с поддержкой постквантовой криптографии (Post-Quantum), разработанный для обхода блокировок (REALITY и XHTTP в условиях ТСПУ / РКН).

### Трехэтапная раздельная архитектура (Three-Phase Architecture)
Для обеспечения абсолютной безопасности и максимальной скорости процесс разделен на три независимых этапа:

```text
  Этап 1: resolve-map (Зарубежный сервер)
  Вход: top-1m.csv
  Выход: resolved_targets.csv (связка домен,IP)
  - Использует исключительно зарубежные Anycast DNS (Cloudflare, Google, Quad9, OpenDNS).
  - Заранее отсекает Cloudflare IP, полностью исключая DNS-спуфинг цензуры.
           │
           ▼
  Этап 2: probe-survival (Внутри РФ / цензурируемой сети)
  Вход: resolved_targets.csv
  Выход: survived_candidates.csv
  - ПОЛНОЕ ОТСУТСТВИЕ ЛОКАЛЬНЫХ DNS-ЗАПРОСОВ! Прямое подключение к готовым IP.
  - Минимальный ClientHello со слепком Google Chrome без утечек трафика.
  - Встроенная задержка 150 мс для детекции отложенных TCP RST пакетов от ТСПУ.
  - Нулевой риск привлечения внимания провайдера.
           │
           ▼
  Этап 3: build-pools (Зарубежный VPS пользователя)
  Вход: survived_candidates.csv
  Выход: rust/aegis/src/resources/sni/*.pb
  - Глубокая валидация TLS 1.3, постквантовых ключей (X25519MLKEM768), длины сертификатов (3500–5500 байт).
  - Автоматическая фильтрация подконтрольных цензуре доменов (CN, IR, RU, KP).
  - Инкрементальное объединение с существующими базами 172 стран без потери старых доменов.
```

### Порядок работы (Три шага)

#### 1. Этап 1: Предварительный резолв (На зарубежном VPS)
```bash
go build -o sni_tester ./cmd/sni_tester/
./sni_tester resolve-map -f top-1m.csv -o resolved_targets.csv -w 100
```

#### 2. Этап 2: Проверка выживаемости (Внутри РФ / локально)
```bash
./sni_tester probe-survival -f resolved_targets.csv -o survived_candidates.csv -w 50 -p socks5://127.0.0.1:10808
```

#### 3. Этап 3: Генерация пулов (На зарубежном VPS пользователя)
```bash
./sni_tester build-pools -f survived_candidates.csv -out-dir rust/aegis/src/resources/sni -w 50
```

---

## Protobuf Definition (`proto/sni.proto`)

```protobuf
syntax = "proto3";
package sni;

option go_package = "sni_tester/proto";

// DomainList stores validated domain names classified into PQ and Classic pools
message DomainList {
    repeated string pq_domains = 1;       // Post-Quantum hybrid keys (X25519MLKEM768)
    repeated string classic_domains = 2;  // Classic TLS 1.3 keys (X25519)
}
```
