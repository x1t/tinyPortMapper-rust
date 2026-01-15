# tinyPortMapper Rust 版本 Makefile
#
# 对齐 C++ 版本的交叉编译能力
# 支持: Linux native, musl 静态链接, MinGW, macOS, OpenWRT targets

NAME = tinymapper
VERSION = 0.1.0
RUST_NAME = tinyportmapper

# Rust target triples
TARGET_X86_64 = x86_64-unknown-linux-gnu
TARGET_X86_64_MUSL = x86_64-unknown-linux-musl
TARGET_AARCH64 = aarch64-unknown-linux-gnu
TARGET_AARCH64_MUSL = aarch64-unknown-linux-musl
TARGET_ARMV7 = armv7-unknown-linux-gnueabihf
TARGET_MIPS = mips-unknown-linux-gnu
TARGET_MIPS64 = mips64-unknown-linux-gnuabi64
TARGET_MIPS_LE = mipsel-unknown-linux-gnu
TARGET_I386 = i686-unknown-linux-gnu
TARGET_X86 = i486-unknown-linux-gnu

# MinGW and macOS targets
TARGET_X86_64_WINDOWS = x86_64-pc-windows-gnu
TARGET_I686_WINDOWS = i686-pc-windows-gnu
TARGET_AARCH64_WINDOWS = aarch64-pc-windows-gnu
TARGET_X86_64_MACOS = x86_64-apple-darwin
TARGET_AARCH64_MACOS = aarch64-apple-darwin

# Release targets (对齐 C++ 版本)
RELEASE_TARGETS = amd64 arm mips24kc_be mips24kc_le x86

.PHONY: all clean release musl release-musl help

# 默认目标 - 本地构建
all: git_version
	cargo build --release

# Git version tracking
GIT_VERSION := $(shell git rev-parse HEAD 2>/dev/null || echo "unknown")
BUILD_DATE := $(shell date +%Y-%m-%d)
BUILD_TIME := $(shell date +%H:%M:%S)

git_version:
	@mkdir -p src/.generated
	@echo 'pub const GIT_VERSION: &str = "$(GIT_VERSION)";' > src/.generated/git_version.rs
	@echo 'pub const BUILD_DATE: &str = "$(BUILD_DATE)";' >> src/.generated/git_version.rs
	@echo 'pub const BUILD_TIME: &str = "$(BUILD_TIME)";' >> src/.generated/git_version.rs
	@echo "Git version: $(GIT_VERSION)"

# Debug build
debug: git_version
	cargo build

# Fast build (no optimization)
fast: git_version
	cargo build --profile fast

# musl 静态链接构建 (推荐用于 Alpine Linux)
musl: git_version
	cargo build --release --target $(TARGET_X86_64_MUSL)
	@echo "Build completed: target/release/$(TARGET_X86_64_MUSL)/$(RUST_NAME)"

musl-aarch64: git_version
	cargo build --release --target $(TARGET_AARCH64_MUSL)
	@echo "Build completed: target/release/$(TARGET_AARCH64_MUSL)/$(RUST_NAME)"

# Native musl builds
release-musl: git_version
	@for target in $(TARGET_X86_64_MUSL) $(TARGET_AARCH64_MUSL); do \
		echo "Building $$target..."; \
		cargo build --release --target $$target; \
	done

# 完整 Release 构建 (对齐 C++ 版本的 make release)
release: git_version
	cargo build --release --target $(TARGET_X86_64)
	cp target/release/$(RUST_NAME) $(NAME)_native
	@echo "Built: $(NAME)_native"

# OpenWRT targets (对齐 C++ 版本)
arm: git_version
	cargo build --release --target $(TARGET_ARMV7)
	cp target/release/$(TARGET_ARMV7)/$(RUST_NAME) $(NAME)_$@

amd64: git_version
	cargo build --release --target $(TARGET_X86_64)
	cp target/release/$(TARGET_X86_64)/$(RUST_NAME) $(NAME)_$@

mips24kc_be: git_version
	cargo build --release --target $(TARGET_MIPS)
	cp target/release/$(TARGET_MIPS)/$(RUST_NAME) $(NAME)_$@

mips24kc_le: git_version
	cargo build --release --target $(TARGET_MIPS_LE)
	cp target/release/$(TARGET_MIPS_LE)/$(RUST_NAME) $(NAME)_$@

x86: git_version
	cargo build --release --target $(TARGET_I386)
	cp target/release/$(TARGET_I386)/$(RUST_NAME) $(NAME)_$@

# 完整 release 包 (对齐 C++ 版本)
release-full: git_version arm amd64 mips24kc_be mips24kc_le x86
	@echo "Creating release package..."
	rm -f $(NAME)_binaries.tar.gz
	tar -zcvf $(NAME)_binaries.tar.gz \
		$(NAME)_arm \
		$(NAME)_amd64 \
		$(NAME)_mips24kc_be \
		$(NAME)_mips24kc_le \
		$(NAME)_x86 \
		version.txt
	@echo "Release package created: $(NAME)_binaries.tar.gz"

# MinGW 交叉编译 (Windows)
mingw: git_version
	cargo build --release --target $(TARGET_X86_64_WINDOWS)
	cp target/release/$(TARGET_X86_64_WINDOWS)/$(RUST_NAME).exe $(NAME).exe
	@echo "Windows build completed: $(NAME).exe"

mingw32: git_version
	cargo build --release --target $(TARGET_I686_WINDOWS)
	cp target/release/$(TARGET_I686_WINDOWS)/$(RUST_NAME).exe $(NAME)_i386.exe

# macOS 交叉编译
macos: git_version
	cargo build --release --target $(TARGET_X86_64_MACOS)
	cp target/release/$(TARGET_X86_64_MACOS)/$(RUST_NAME) $(NAME)_mac
	@echo "macOS build completed: $(NAME)_mac"

macos-aarch64: git_version
	cargo build --release --target $(TARGET_AARCH64_MACOS)
	cp target/release/$(TARGET_AARCH64_MACOS)/$(RUST_NAME) $(NAME)_mac_arm64

# FreeBSD (使用 native build，标记为 freebsd)
freebsd: git_version
	cargo build --release --target $(TARGET_X86_64)
	cp target/release/$(RUST_NAME) $(NAME)_freebsd
	@echo "FreeBSD build completed: $(NAME)_freebsd"

# 完整发布包 (包含所有平台)
release-all: git_version release-full mingw macos
	@echo "Creating full release package..."
	tar -zcvf $(NAME)_all_platforms.tar.gz \
		$(NAME)_binaries.tar.gz \
		$(NAME).exe \
		$(NAME)_mac \
		version.txt
	@echo "Full release package created: $(NAME)_all_platforms.tar.gz"

# 清理
clean:
	cargo clean
	rm -f $(NAME)_* $(RUST_NAME).exe version.txt
	rm -rf src/.generated

# 清理所有构建产物
distclean: clean
	rm -rf target

# 运行测试
test: git_version
	cargo test --release

# 运行测试 (详细输出)
test-verbose: git_version
	cargo test --release -- --nocapture

# 检查代码
check: git_version
	cargo check
	cargo check --target $(TARGET_X86_64_MUSL)
	cargo check --target $(TARGET_AARCH64_MUSL)

# 代码格式化
fmt:
	cargo fmt --all

# 代码检查 (clippy)
clippy: git_version
	cargo clippy
	cargo clippy --target $(TARGET_X86_64_MUSL)

# 查看依赖
deps:
	cargo tree --depth 3

# 查看二进制大小
size:
	@ls -lh target/release/$(RUST_NAME) 2>/dev/null || echo "Build first with 'make'"
	@ls -lh target/release/$(TARGET_X86_64_MUSL)/$(RUST_NAME) 2>/dev/null || echo "Musl build not found"

# 打印帮助信息
help:
	@echo ""
	@echo "tinyPortMapper Rust 版本 Makefile"
	@echo "=================================="
	@echo ""
	@echo "基本构建:"
	@echo "  make              - 本地 Release 构建"
	@echo "  make debug        - 本地 Debug 构建"
	@echo "  make fast         - 快速构建 (无优化)"
	@echo ""
	@echo "musl 静态链接:"
	@echo "  make musl         - x86_64 musl 静态链接"
	@echo "  make musl-aarch64 - aarch64 musl 静态链接"
	@echo ""
	@echo "OpenWRT 目标 (对齐 C++):"
	@echo "  make arm          - ARM build"
	@echo "  make amd64        - x86_64 build"
	@echo "  make mips24kc_be  - MIPS big-endian"
	@echo "  make mips24kc_le  - MIPS little-endian"
	@echo "  make x86          - x86 build"
	@echo ""
	@echo "跨平台构建:"
	@echo "  make mingw        - Windows x86_64"
	@echo "  make mingw32      - Windows i386"
	@echo "  make macos        - macOS x86_64"
	@echo "  make macos-aarch64 - macOS ARM64"
	@echo "  make freebsd      - FreeBSD"
	@echo ""
	@echo "发布包:"
	@echo "  make release      - 本地 release"
	@echo "  make release-full - OpenWRT 全平台 release"
	@echo "  make release-all  - 全平台发布包 (Linux + Win + macOS)"
	@echo ""
	@echo "工具命令:"
	@echo "  make test         - 运行测试"
	@echo "  make check        - 代码检查"
	@echo "  make clippy       - 代码质量检查"
	@echo "  make fmt          - 代码格式化"
	@echo "  make size         - 查看二进制大小"
	@echo "  make help         - 显示此帮助信息"
	@echo ""
