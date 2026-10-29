# Codex Auto Continue

基于 [OpenAI Codex](https://github.com/openai/codex) 的自定义版本，由本地后台服务器等待额度恢复或暂时性模型错误，并自动继续被中断的任务；TUI 负责显示状态和交互。当前基于官方 `0.165.0`，CLI 和 TUI 显示版本为 `0.165.0+auto_continue`。

## 相对官方版本的新增特性

### 达到 5 小时额度后自动等待

- 优先读取已耗尽的 300 分钟额度窗口中的刷新时间戳。
- 没有可用时间戳时，尝试解析错误消息中的本地时间 `try again at …`。
- 无法确定刷新时间时，不启动自动等待。

### Model capability 错误随机指数退避

识别包含 `model capability` 或 `model_capability` 的终止性错误消息，按以下规则重试：

1. 首次等待从 `0～0.5 秒` 中随机选择。
2. 等待结束后的 64 秒内再次发生该错误，随机区间上限翻倍：`0～1 秒`、`0～2 秒`、`0～4 秒`，依此类推。
3. 如果某次实际等待超过 64 秒，且等待结束后的 64 秒内再次发生该错误，则停止自动重试。
4. 再次发生错误时，距上次等待结束已超过 64 秒，则重置为初始随机区间。

随机区间包含下限，不包含上限。

### 倒计时、取消与自动继续

等待期间，空白输入框中显示实时更新的英文状态，例如：

```text
5h usage limit reached. Automatically continuing in 1 h 23 m 45 s. Type to cancel.
```

- 普通按键或粘贴会取消服务器上的等待、移除状态提示，并正常处理用户输入。
- 等待时按 **Ctrl+B** 只退出 TUI，不发送取消或关闭线程请求；后台服务器继续等待并自动续跑。
- TUI 收到等待状态时，如果输入框已有草稿，会请求取消服务器等待，避免干扰正在编辑的消息。
- 等待期间 TUI 不会自动发送排队消息；切换会话只清除本地显示，不取消原会话的后台等待。
- 没有 TUI 连接时，服务器仍持有计时器并保留等待中的线程。重新连接该线程后，TUI 会显示服务器原有截止时间的倒计时，不会重新计时或重复续跑。
- 等待结束后自动发送 `Continue`；如果被中断的回合正在追求 goal，则执行 `/goal resume`。

重试逻辑运行在 app-server 中，不依赖 TUI 存活，也不需要外部重试脚本。后台模式要求本地 background server 正在运行且也使用本 fork 的新构建；仅更新前端不能给旧服务器增加此功能。停止服务器会结束内存中的等待，当前不跨服务器重启持久化计时器。非交互式 `codex exec` 不走 app-server 的等待流程。

### 自定义版本号与更新提示

版本号使用当前官方版本号加 `+auto_continue` 后缀。发现官方新版本时仍显示通知，但通知不提供安装命令或直接更新操作；更新这个 fork 后重新构建即可。

更新检查沿用现有 `check_for_update_on_startup` 设置。普通 debug 构建默认不启用官方更新检查；下方提供 release 构建和保留更新检查的开发构建命令。

## 从源码构建

以下命令面向 Linux；已在 Linux x86_64 环境完成开发构建和针对性测试。

### 准备环境

需要 Git、通过 rustup 管理的 Rust 工具链、C/C++ 编译工具以及 OpenSSL 开发文件。Ubuntu/Debian 可安装：

```bash
sudo apt-get update
sudo apt-get install -y git curl build-essential pkg-config libssl-dev
```

如果尚未安装 Rust：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
```

克隆本 fork 并进入 Rust 工作区：

```bash
git clone --branch codex-auto-continue https://github.com/DeswV/codex-auto-continue.git
cd codex-auto-continue/codex-rs
rustup show
```

工作区的 `rust-toolchain.toml` 固定工具链版本（当前为 `1.99.1`）；rustup 会根据该文件选择和安装工具链。首次构建需要下载依赖，耗时和磁盘占用较大，请预留足够空间。

### Release 构建

```bash
cargo build --locked --release -p codex-cli --bin codex
./target/release/codex --version
```

产物为 `codex-rs/target/release/codex`，当前应输出：

```text
codex-cli 0.165.0+auto_continue
```

### 开发构建

以下命令关闭调试符号和增量缓存，并让 TUI 保留正式构建中的更新检查逻辑。所有设置仅作用于本次构建命令，不会修改 Codex 配置：

```bash
CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4 \
  cargo build --locked -p codex-cli --bin codex \
  --config 'profile.dev.package.codex-tui.debug-assertions=false'

./target/debug/codex --version
```

### 运行针对性测试

在 `codex-rs` 目录执行：

```bash
CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4 \
  cargo test --locked -p codex-tui --lib auto_retry

CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4 \
  cargo test --locked -p codex-tui --lib update_versions
```

后台测试：

```bash
CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4 \
  cargo test --locked -p codex-app-server --lib auto_retry

CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4 \
  cargo test --locked -p codex-app-server --test all auto_retry
```

测试覆盖退避边界、刷新时间解析、无客户端续跑、重连恢复倒计时、取消等待、Ctrl+B 退出以及版本比较。它们不代替真实额度耗尽场景的端到端验证。

## 安装编译产物

在 `codex-rs` 目录执行，使用独立名称保留现有官方版：

```bash
mkdir -p ~/.local/bin
install -m 755 target/release/codex ~/.local/bin/codex-auto-continue
# 可选：去除符号以缩小文件
strip ~/.local/bin/codex-auto-continue

~/.local/bin/codex-auto-continue --version
~/.local/bin/codex-auto-continue
```

如果使用开发构建，将源路径替换为 `target/debug/codex`。如果 `~/.local/bin` 已在 PATH 中，可直接运行 `codex-auto-continue`。

安装仅复制二进制，不修改现有 Codex 配置。程序启动后沿用现有配置与登录状态；新机器需自行登录。

复制到其他机器时，需要匹配操作系统、CPU 架构和运行库。当前 Linux x86_64 构建依赖 glibc 和 OpenSSL 3，可使用 `ldd` 检查目标机器上的缺失依赖；它不能直接运行在 macOS、Windows 或 ARM64 上，这些平台需要另行构建。

## 许可证

本项目沿用 [Apache-2.0 License](LICENSE)。更多实现行为见 [auto-continue 文档](docs/auto-continue.md)。
