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

## ChatGPT 订阅与第三方 API 混合使用

可以保留官方 Plus/Pro 的 ChatGPT 登录，同时配置一个或多个第三方服务商。每个服务商使用自己的 endpoint 和 API key；切换服务商不会注销订阅账户，也不会覆盖其登录凭据。目前第三方接口需要兼容 **OpenAI Responses API**（`/responses`）；仅提供 Chat Completions 的接口不能直接使用。

在现有配置中加入服务商定义，例如：

```toml
# 主代理默认使用订阅模型。
model_provider = "openai"
model = "gpt-5.4"

[model_providers.third_party]
name = "My API service"
base_url = "https://your-api.example/v1"
wire_api = "responses"
env_key = "THIRD_PARTY_API_KEY"
requires_openai_auth = false
# 服务商不支持 Responses WebSocket 时使用 HTTP。
supports_websockets = false
```

将 `THIRD_PARTY_API_KEY` 设置在运行 Codex 的进程环境中；使用后台服务器时，该变量需要存在于**服务器进程**的环境中。上述域名和模型名是示例，应替换为服务商实际提供的值。已有 ChatGPT 登录时无需重新登录；首次使用订阅模型时先通过 `codex-auto-continue login` 登录 ChatGPT。

### 在同一线程切换服务商

TUI 支持 `provider::model` 形式的模型选择：

```text
/model third_party::vendor/custom-model
/model openai::gpt-5.4
```

模型选择作用于该线程后续启动的回合，保留同一线程的聊天历史，不会改变已经发出的模型请求。没有前缀的模型名沿用当前服务商。app-server 客户端也可以在 `turn/start` 或 `thread/settings/update` 的 `model` 字段中传入同样的选择器。

前缀是配置中的服务商 ID（上例中的 `third_party`），传给 API 的模型名只有后面的 `vendor/custom-model`。切换时会隔离服务商的连接和路由状态；订阅认证不会用作第三方 API 的认证。

### 主代理与子代理使用不同服务商

可以在请求子代理时显式指定 `model = "third_party::vendor/custom-model"`，或指定 `model = "openai::gpt-5.4"`。新建无历史继承的子代理可使用如下参数：

```json
{
  "task_name": "review",
  "message": "Review the changes",
  "fork_turns": "none",
  "model": "third_party::vendor/custom-model"
}
```

也可以配置默认子代理模型：

```toml
[agents]
default_subagent_model = "third_party::vendor/custom-model"
```

角色配置文件可以选择父线程已配置的服务商，例如 `agents/api-reviewer.toml`：

```toml
model_provider = "third_party"
model = "vendor/custom-model"
developer_instructions = "Review the changes and report concrete issues."
```

在主配置中注册该角色：

```toml
[agents.api_reviewer]
description = "Review changes using the third-party API"
config_file = "agents/api-reviewer.toml"
```

反过来，主代理使用第三方服务时，可将默认子代理模型设为 `openai::gpt-5.4`，或在子代理角色文件中设置 `model_provider = "openai"`。子代理的服务商选择不会改变父线程的选择；权限和其他父线程约束仍然保留。角色文件只能选择已经配置的服务商，不能借此重定义 endpoint 或凭据。受管理环境中的服务商限制仍然生效。

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

多服务商集成测试（使用本地模拟 API，不使用真实订阅或 API 额度）：

```bash
CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4 \
  cargo test --locked -p codex-app-server --test all provider_switch
```

测试覆盖退避边界、刷新时间解析、无客户端续跑、重连恢复倒计时、取消等待、Ctrl+B 退出、版本比较、同线程跨服务商切换，以及主子代理混用服务商时的请求和认证隔离。它们不代替真实额度耗尽场景的端到端验证。

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

## 管理后台服务器

以下命令使用安装后的自定义程序名 `codex-auto-continue`；如果尚未安装，可在 `codex-rs` 目录中改用 `./target/release/codex`，开发构建则使用 `./target/debug/codex`。请使用与原服务器相同的系统用户和 `CODEX_HOME`。

### 停止、启动与重启

```bash
codex-auto-continue app-server daemon stop
codex-auto-continue app-server daemon start
```

只需重启当前后台服务版本时，可直接执行：

```bash
codex-auto-continue app-server daemon restart
```

停止或重启会中断服务器上正在执行的工作，并结束内存中的自动等待倒计时；该倒计时不会跨服务器重启恢复。Ctrl+B 退出等待中的 TUI 不会停止后台服务器。

### 将后台服务器切换到新构建

后台服务器使用单独保存的程序副本。替换前端二进制或执行 `restart` 不会自动将该副本替换为新构建。更新本 fork 并重新构建后，在 `codex-rs` 目录执行：

```bash
./target/release/codex app-server daemon update --from-cli --yes
./target/release/codex app-server daemon start
./target/release/codex app-server daemon version
```

开发构建请将 `release` 替换为 `debug`。也可以用已安装的新版本 `codex-auto-continue` 执行这些子命令。

`update --from-cli --yes` 会复制并固定为当前 CLI 的程序包；如果后台服务器正在运行，会自动重启。随后 `start` 确保服务已启动，`version` 输出本地 CLI、已安装后台服务及运行中服务的版本信息。首次使用、尚未安装后台服务时，先执行一次 `app-server daemon start`。

自定义版本应保留 `--from-cli`；不带该参数的 `daemon update` 会切回官方生产版本更新。上述操作管理后台程序及其状态，不修改现有 `config.toml` 或登录凭据。

## 许可证

本项目沿用 [Apache-2.0 License](LICENSE)。更多实现行为见 [auto-continue 文档](docs/auto-continue.md)。
