# 构建并把产物放到 crate 根目录的 ./sni_probe
#
# 为什么需要这一步：cargo 的产物固定落在 <target-dir>/release/sni_probe，
# 想让 `./sni_probe` 直接可执行就得拷一份出来。

BIN  := sni_probe
MODE ?= release

ifeq ($(MODE),debug)
  PROFILE := debug
  CARGO_FLAGS :=
else
  PROFILE := release
  CARGO_FLAGS := --release
endif

.PHONY: all build test lint fmt clean

all: build

build:
	cargo build $(CARGO_FLAGS)
	cp -f build/$(PROFILE)/$(BIN) ./$(BIN)
	@echo "=> ./$(BIN)  ($(PROFILE))"

test:
	cargo test $(CARGO_FLAGS)

lint:
	cargo clippy $(CARGO_FLAGS) --all-targets

fmt:
	cargo fmt --check

clean:
	cargo clean
	rm -f ./$(BIN)
