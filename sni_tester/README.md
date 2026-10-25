# SNI Tester — High-Performance Dual-Pool Censorship-Resistant Probe Engine
[English](#english) | [فارسی (Persian)](#فارسی-persian) | [Русский (Russian)](#русский-russian)

---

<a name="english"></a>
## English

High-performance concurrent SNI testing and Post-Quantum (PQ) camouflage domain management engine, tailored for anti-censorship systems (REALITY / XHTTP).

### Key Features
- **Decoupled Two-Phase Pipeline**:
  - **Phase 1 (`probe-survival`)**: Lightweight survival check running inside censored regions (China, Iran, Russia) to detect GFW / TSP RST injection and active filtering without triggering ISP scrutiny.
  - **Phase 2 (`build-pools`)**: Deep inspection on overseas VPS to evaluate TLS 1.3, Post-Quantum key exchange (`X25519MLKEM768`), ALPN H2/H3, cert-chain length limits (3500–5500B), and exclude CDNs/Cloudflare.
- **Post-Quantum Dual-Pool Classification**: Separates domains into `pq_domains` (ML-KEM / Kyber hybrid) and `classic_domains` (standard X25519) per country.
- **Global Outbound Proxy Support (`-p socks5://...`)**: Routes all DNS lookups and TLS probing traffic through SOCKS5 to hide local residential broadband IPs.
- **Hardware Interface Isolation (`-iface`)**: Binds traffic to a secondary wireless interface (e.g. `wlp6s0`) using `SO_BINDTODEVICE` to prevent saturated test traffic from interrupting local browsing or SSH sessions.
- **Fast In-Place Upgrade (`-upgrade-dir`)**: Scans existing `.pb` country resources in parallel to refresh and prune dead nodes.

### Quick Start

#### Phase 1: Local Survival Probe (Run inside censored regions / local network)
Fast and lightweight — checks DNS resolution, filters Cloudflare, and tests whether TCP/TLS 443 is blocked or reset:
```bash
go build -o sni_tester ./cmd/sni_tester/
./sni_tester probe-survival -f top-1m.csv -o survived_candidates.txt -w 50 -p socks5://127.0.0.1:10808
```

#### Phase 2: VPS Dual-Pool Builder (Run on your overseas VPS)
Takes `survived_candidates.txt` and performs deep inspection with zero risk from local censorship:
```bash
./sni_tester build-pools -f survived_candidates.txt -out-dir rust/aegis/src/resources/sni -w 50
```

#### Upgrading Existing Country `.pb` Files Directly
```bash
./sni_tester -upgrade-dir rust/aegis/src/resources/sni -w 50 -p socks5://127.0.0.1:10808
```

### CLI Options

| Command / Flag | Description | Example |
| :--- | :--- | :--- |
| `probe-survival` | Phase 1: Lightweight survival probe for censored regions | `./sni_tester probe-survival -f list.txt -o survived.txt` |
| `build-pools` | Phase 2: Deep inspection and dual-pool `.pb` generator | `./sni_tester build-pools -f survived.txt` |
| `-p` | Global outbound SOCKS5 proxy URL (TLS + DNS + GeoDB) | `-p socks5://127.0.0.1:1080` |
| `-iface` | Bind outbound traffic to network interface or IP | `-iface wlp6s0` |
| `-upgrade-dir` | In-place batch upgrade of existing `.pb` files | `-upgrade-dir rust/aegis/src/resources/sni` |
| `-upgrade-pb` | Upgrade a single `.pb` file | `-upgrade-pb rust/aegis/src/resources/sni/US.pb` |
| `-timeout` | Connection & handshake timeout in seconds | `-timeout 3` |
| `-w` | Concurrency worker count | `-w 50` |
| `-f` | Input TXT/CSV domain list | `-f domains.txt` |
| `-out-dir` | Output directory for `.pb` files | `-out-dir ./output` |

---

<a name="فارسی-persian"></a>
## فارسی (Persian)

موتور قدرتمند و پرسرعت برای تست SNI و مدیریت دامنه‌های استتاری پساکوانتومی (Post-Quantum)، طراحی‌شده برای مقابله با فیلترینگ شدید (سیستم‌های REALITY و XHTTP).

### ویژگی‌های کلیدی
- **ساختار دومرحله‌ای مجزا (Two-Phase Pipeline)**:
  - **مرحله اول (`probe-survival`)**: تست سبک و سریع بقا در داخل مناطق تحت سانسور (ایران، چین، روسیه) برای تشخیص فیلترینگ فعال و بسته‌های RST تزریقی فیلترچی (بدون بارگذاری دیتابیس‌های سنگین و بدون جلب توجه سیستم‌های نظارتی ISP).
  - **مرحله دوم (`build-pools`)**: اعتبارسنجی عمیق روی سرور مجازی (VPS) خارج از کشور برای بررسی TLS 1.3، الگوریتم‌های پساکوانتومی (`X25519MLKEM768`)، استانداردهای ALPN H2/H3، طول زنجیره گواهی (۳۵۰۰ تا ۵۵۰۰ بایت) و حذف CDNهای شناسایی‌پذیر مانند Cloudflare.
- **دسته‌بندی در دو مخزن پساکوانتومی و کلاسیک**: تفکیک دامنه‌ها به دو مخزن `pq_domains` (مبتنی بر ML-KEM/Kyber) و `classic_domains` (الگوریتم X25519) برای هر کشور.
- **پشتیبانی کامل از پروکسی سراسری خروجی (`-p socks5://...`)**: هدایت کلیه پرس‌وجوهای DNS و ترافیک بررسی TLS از طریق پروکسی SOCKS5 جهت پنهان‌سازی کامل IP اینترنت خانگی.
- **جداسازی رابط شبکه با (`-iface`)**: امکان قفل کردن ترافیک تست روی یک کارت شبکه بی‌سیم مجزا (مانند مودم 4G روی `wlp6s0`) تا سرعت مرور وب و اتصال SSH اصلی دچار اختلال نشود.
- **بروزرسانی مستقیم فایل‌های باینری موجود (`-upgrade-dir`)**: بررسی مجدد فایل‌های `.pb` موجود در ۱۷۲ کشور و حذف لینک‌های از کار افتاده.

### راهنمای سریع

#### مرحله ۱: بررسی بقا در داخل شبکه فیلترشده (ایران / کلاینت محلی)
بسیار سریع و سبک — حل نام دامنه‌ها، حذف آی‌پی‌های کلودفلر، و اطمینان از مسدود نبودن اتصال به پورت 443:
```bash
go build -o sni_tester ./cmd/sni_tester/
./sni_tester probe-survival -f top-1m.csv -o survived_candidates.txt -w 50 -p socks5://127.0.0.1:10808
```

#### مرحله ۲: ساخت مخزن دامنه‌ها در سرور خارج (VPS)
فایل `survived_candidates.txt` را دریافت کرده و اعتبارسنجی دقیق را بدون خطر مسدودسازی انجام می‌دهد:
```bash
./sni_tester build-pools -f survived_candidates.txt -out-dir rust/aegis/src/resources/sni -w 50
```

#### بروزرسانی مستقیم فایل‌های `.pb` موجود
```bash
./sni_tester -upgrade-dir rust/aegis/src/resources/sni -w 50 -p socks5://127.0.0.1:10808
```

### راهنمای پارامترهای خط فرمان

| دستور / سوئیچ | توضیحات | مثال |
| :--- | :--- | :--- |
| `probe-survival` | مرحله ۱: بررسی سبک بقای دامنه در مناطق تحت فیلترینگ | `./sni_tester probe-survival -f list.txt -o survived.txt` |
| `build-pools` | مرحله ۲: بررسی عمیق و ایجاد فایل‌های Protobuf دو مخزنه | `./sni_tester build-pools -f survived.txt` |
| `-p` | نشانی پروکسی SOCKS5 خروجی برای کلیه ترافیک TLS و DNS | `-p socks5://127.0.0.1:1080` |
| `-iface` | اتصال ترافیک خروجی به کارت شبکه یا آی‌پی خاص | `-iface wlp6s0` |
| `-upgrade-dir` | ارتقای دسته‌ای فایل‌های دایرکتوری به فرمت دو مخزنه | `-upgrade-dir rust/aegis/src/resources/sni` |
| `-upgrade-pb` | ارتقای یک فایل اختصاصی `.pb` | `-upgrade-pb rust/aegis/src/resources/sni/US.pb` |
| `-timeout` | مهلت برقراری ارتباط به ثانیه | `-timeout 3` |
| `-w` | تعداد رشته‌های همزمان (Worker) | `-w 50` |
| `-f` | مسیر فایل ورودی شامل فهرست دامنه‌ها | `-f domains.txt` |
| `-out-dir` | مسیر ذخیره فایل‌های نهایی `.pb` | `-out-dir ./output` |

---

<a name="русский-russian"></a>
## Русский (Russian)

Высокопроизводительный движок параллельного тестирования SNI и управления маскировочными доменами с поддержкой постквантовой криптографии (Post-Quantum), разработанный для обхода блокировок (REALITY и XHTTP в условиях ТСПУ / РКН).

### Ключевые возможности
- **Двухэтапная раздельная архитектура (Two-Phase Pipeline)**:
  - **Этап 1 (`probe-survival`)**: Легковесная быстрая проверка выживаемости внутри цензурируемых сетей (Россия, Иран, Китай) для выявления сбросов соединений (TCP RST) и блокировок ТСПУ без нагрузки и без привлечения внимания систем аудита провайдеров.
  - **Этап 2 (`build-pools`)**: Глубокий анализ на зарубежном VPS для проверки TLS 1.3, постквантового обмена ключами (`X25519MLKEM768`), протоколов ALPN H2/H3, длины цепочки сертификатов (3500–5500 байт) и отсеивания Cloudflare/сторонних CDN.
- **Классификация на два пула (Dual-Pool)**: Разделение доменов на `pq_domains` (гибриды ML-KEM/Kyber) и `classic_domains` (стандартный X25519) по каждой стране.
- **Поддержка глобального исходящего прокси (`-p socks5://...`)**: Перенаправление всех DNS-запросов (DoH/DoT) и TLS-проб через SOCKS5 для полной маскировки реального IP-адреса домашнего провайдера.
- **Изоляция сетевого интерфейса (`-iface`)**: Привязка всего трафика к отдельной сетевой карте (например, 4G-модем на `wlp6s0`) через `SO_BINDTODEVICE`, предотвращающая просадку домашнего интернета и SSH.
- **Пакетное обновление существующих баз (`-upgrade-dir`)**: Быстрая параллельная перепроверка существующих файлов `.pb` по 172 странам с удалением неработающих доменов.

### Быстрый старт

#### Этап 1: Проверка выживаемости (Запуск из РФ / локальной сети)
Максимально быстро и незаметно — резолвит адреса, отфильтровывает Cloudflare и проверяет доступность порта 443 без сброса TCP:
```bash
go build -o sni_tester ./cmd/sni_tester/
./sni_tester probe-survival -f top-1m.csv -o survived_candidates.txt -w 50 -p socks5://127.0.0.1:10808
```

#### Этап 2: Построение пулов на VPS (Запуск на зарубежном сервере)
Принимает `survived_candidates.txt` и производит полный анализ сертификатов и протоколов без ограничений цензуры:
```bash
./sni_tester build-pools -f survived_candidates.txt -out-dir rust/aegis/src/resources/sni -w 50
```

#### Прямое обновление существующих файлов `.pb`
```bash
./sni_tester -upgrade-dir rust/aegis/src/resources/sni -w 50 -p socks5://127.0.0.1:10808
```

### Параметры командной строки

| Команда / Флаг | Описание | Пример |
| :--- | :--- | :--- |
| `probe-survival` | Этап 1: Быстрая проверка выживаемости в зоне цензуры | `./sni_tester probe-survival -f list.txt -o survived.txt` |
| `build-pools` | Этап 2: Глубокая инспекция и генерация двухпуловых `.pb` | `./sni_tester build-pools -f survived.txt` |
| `-p` | URL исходящего SOCKS5-прокси для TLS, DNS и загрузки баз | `-p socks5://127.0.0.1:1080` |
| `-iface` | Привязка исходящего трафика к интерфейсу или IP | `-iface wlp6s0` |
| `-upgrade-dir` | Пакетное обновление директории с файлами `.pb` | `-upgrade-dir rust/aegis/src/resources/sni` |
| `-upgrade-pb` | Обновление одного выбранного файла `.pb` | `-upgrade-pb rust/aegis/src/resources/sni/US.pb` |
| `-timeout` | Таймаут соединения и рукопожатия в секундах | `-timeout 3` |
| `-w` | Количество параллельных воркеров | `-w 50` |
| `-f` | Путь к входному TXT/CSV файлу с доменами | `-f domains.txt` |
| `-out-dir` | Директория для сохранения готовых `.pb` файлов | `-out-dir ./output` |

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
