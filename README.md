# Codex Auto Continue

基于 [OpenAI Codex](https://github.com/openai/codex) 的自定义版本。当前基于官方 `0.165.0`，CLI 和 TUI 显示版本为 `0.165.0+auto_continue.0.1.0`。

版本号采用 `<官方版本>+auto_continue.<自定义版本>`，自定义版本从 `0.1.0` 开始独立递增。

## 新增功能

- **5 小时额度自动恢复**：达到限额后等待额度刷新，再额外等待 1 秒，自动继续任务。
- **Model capability 错误自动重试**：使用随机指数退避；持续失败时停止重试。
- **后台等待**：关闭 TUI 后仍可等待并自动继续；重新连接时显示原有倒计时。
- **倒计时与取消**：等待时输入框显示英文倒计时；普通按键或粘贴可取消等待。输入框为空时，按 **Ctrl+C** 退出 TUI，后台继续等待。
- **自动恢复 goal**：普通任务自动发送 `Continue`；因错误中断的 goal 自动恢复追求。
- **订阅与第三方 API 共存**：同一线程可切换服务商，主代理与子代理也可以使用不同服务商。
- **自定义版本更新提示**：发现官方更新时提示，不提供直接更新选项；请更新本 fork 后重新构建。

后台功能需要运行本版本的后台服务器，升级方法见下文。停止或重启服务器后，原有等待倒计时不会恢复。`codex exec` 不支持上述自动等待功能。

## ChatGPT 订阅与第三方 API 混合使用

保留 Plus/Pro 的 ChatGPT 登录，在现有配置中添加第三方服务商。第三方接口需要兼容 **OpenAI Responses API**；仅提供 Chat Completions 的服务暂不支持。

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

将示例 endpoint 和模型名替换为服务商实际提供的值，并在**后台服务器进程**的环境中设置 `THIRD_PARTY_API_KEY`。已有 ChatGPT 登录时无需重新登录；首次使用订阅模型时执行 `codex-auto-continue login`。

### 在同一线程切换服务商

```text
/model third_party::vendor/custom-model
/model openai::gpt-5.4
```

`::` 前面是配置中的服务商 ID，后面是模型名；省略前缀时沿用当前服务商。切换保留聊天历史，对后续回合生效。

### 主代理与子代理使用不同服务商

请求子代理时，可显式指定服务商和模型：

```json
{
  "task_name": "review",
  "message": "Review the changes",
  "fork_turns": "none",
  "model": "third_party::vendor/custom-model"
}
```

也可以设置默认子代理模型：

```toml
[agents]
default_subagent_model = "third_party::vendor/custom-model"
```

或者创建角色文件 `agents/api-reviewer.toml`：

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

主代理使用第三方服务时，可将子代理模型改为 `openai::gpt-5.4`，或在角色文件中设置 `model_provider = "openai"`。服务商需要事先在主配置中定义。

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
codex-cli 0.165.0+auto_continue.0.1.0
```

### 开发构建

开发时可使用以下构建命令：

```bash
CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4 \
  cargo build --locked -p codex-cli --bin codex \
  --config 'profile.dev.package.codex-tui.debug-assertions=false'

./target/debug/codex --version
```

## 安装编译产物

在 `codex-rs` 目录执行：

```bash
mkdir -p ~/.local/bin
install -m 755 target/release/codex ~/.local/bin/codex-auto-continue
# 可选：去除符号以缩小文件
strip ~/.local/bin/codex-auto-continue
```

使用开发构建时，将安装命令中的 `target/release/codex` 替换为 `target/debug/codex`。

将 `~/.local/bin/` 加入 `PATH`，然后你就可以像使用 `codex` 那样使用 `codex-auto-continue` 命令。

也可以在你的 `~/.bashrc` 或 `~/.zshrc` 中添加一句：

```
alias codex="~/.local/bin/codex-auto-continue"
```

那么使用 `codex` 命令就是直接使用 `codex-auto-continue`。

## 管理后台服务器

以下命令使用安装后的 `codex-auto-continue`。尚未安装时，可在 `codex-rs` 目录中改用 `./target/release/codex`；开发构建使用 `./target/debug/codex`。请使用与原服务器相同的系统用户和 `CODEX_HOME`。

### 停止、启动与重启

```bash
codex-auto-continue app-server daemon stop
codex-auto-continue app-server daemon start
# 重启当前后台服务版本
codex-auto-continue app-server daemon restart
```

停止或重启会中断正在执行的任务，并结束当前自动等待。等待时仅想退出 TUI，请在输入框为空时按 Ctrl+C。

### 将后台服务器切换到新构建

安装新构建后，执行以下命令更新后台服务；仅替换前端或执行 `restart` 不会更新后台服务版本。

```bash
codex-auto-continue app-server daemon update --from-cli --yes
codex-auto-continue app-server daemon start
codex-auto-continue app-server daemon version
```

第一条命令会更新后台服务，并在其正在运行时自动重启。首次使用、尚未安装后台服务时，先执行一次 `app-server daemon start`。

请保留 `--from-cli`，以使用当前自定义构建；不带该参数会切回官方生产版本更新。上述操作不修改 `config.toml` 或登录凭据。

## 开发文档与许可证

实现细节、代码位置和测试命令见 [docs/auto-continue.md](docs/auto-continue.md)。本项目沿用 [Apache-2.0 License](LICENSE)。
