# Auto Continue 实现与开发说明

用户功能、配置、构建、安装和后台服务操作见 [README](../README.md)。本文记录实现规则、代码入口及验证方式。以下路径均相对仓库根目录。

## 服务端自动恢复

自动恢复状态保存在每个线程的 `AutoRetry` 中，由 app-server 的线程监听器驱动。TUI 不持有可以提交请求的重试计时器。

- [app-server/src/auto_retry.rs](../codex-rs/app-server/src/auto_retry.rs)：错误识别、刷新时间解析、退避策略、截止时间、取消和续跑。
- [app-server/src/thread_state.rs](../codex-rs/app-server/src/thread_state.rs)：线程的重试状态及查找。
- [request_processors/thread_lifecycle.rs](../codex-rs/app-server/src/request_processors/thread_lifecycle.rs)：监听事件及计时器，保留等待中的线程，处理重新订阅。没有客户端连接时也继续运行。
- [request_processors/turn_processor.rs](../codex-rs/app-server/src/request_processors/turn_processor.rs)：新回合和中断请求对等待的取消。
- [request_processors/thread_goal_processor.rs](../codex-rs/app-server/src/request_processors/thread_goal_processor.rs)：goal 操作与等待的协调。

### 5 小时额度

只对影响回合终止状态的错误启动自动恢复。识别包含 `usage limit`、`5h` 的消息及结构化 `UsageLimitExceeded` 错误。

优先使用额度快照中已耗尽的 300 分钟窗口的 `resets_at`，包括 primary 或 secondary 窗口；额度 ID 为 `codex` 或缺省时接受该快照。服务商发生变化时清除旧的额度状态。没有快照时间时，回退解析消息中的本地时间 `try again at …`。无法解析时不等待。

等待时长按毫秒计算：`max(刷新时间 - 当前时间, 0) + 1 秒`。使用单调时钟驱动等待，同时保存 Unix 毫秒截止时间供客户端显示。额外 1 秒用于避免在额度刷新边界过早发送续跑请求；刷新时间已经过去时仍至少等待 1 秒。

### Model capability 退避

终止性错误消息中包含 `model capability` 或 `model_capability` 时，使用 full jitter：

1. 首次等待在 `[0, 0.5)` 秒中随机选择。
2. 等待结束后 64 秒内再次发生同类错误，采样区间上限翻倍，依次为 1、2、4 秒等。
3. 如果上次实际等待超过 64 秒，且其结束后 64 秒内再次发生错误，放弃重试。
4. 距上次等待结束超过 64 秒才再次发生错误，重置上限为 0.5 秒。

区间包含下限，不包含上限。取消等待会清除相应退避状态。

### 续跑、goal 和取消

收到回合终止事件后，计时器才允许提交续跑。提交使用 `continue_turn_if_idle` 并校验失败回合 ID，避免续跑覆盖后来的用户回合。清理中的 `NotIdle` 状态短暂重试 admission。

普通续跑使用内部上下文消息 `Continue`，不构成新的用户授权。如果错误前正在追求 goal，则通过 GoalService 执行与 `/goal resume` 等价的操作。记录 goal 对应回合 ID，以兼容 goal 状态事件与错误事件的到达顺序；不会重新激活已完成或预算受限的 goal。

普通按键、粘贴、已有草稿、新回合和有关线程设置或 goal 操作可以取消等待。TUI 通过现有 `turn/interrupt` 请求指向失败回合；即使该回合已不活跃，服务器也识别并取消等待。服务端和前端记录已取消回合，避免迟到状态恢复同一次等待。

计时器只存在于服务器内存中，不跨服务器重启恢复。后台模式要求服务器也使用本 fork。嵌入式服务器随前端进程退出，不能提供关闭 TUI 后的等待。`codex exec` 不使用该 app-server 恢复流程。

## TUI 状态和退出

- [协议 notification.rs](../codex-rs/app-server-protocol/src/protocol/v2/notification.rs)：`thread/autoRetry` 通知包含线程 ID、失败回合 ID、Unix 毫秒截止时间及英文原因；截止时间为空表示清除等待。
- [chatwidget/auto_retry.rs](../codex-rs/tui/src/chatwidget/auto_retry.rs)：显示倒计时、恢复占位文字、请求取消、处理等待中的 Ctrl+C。
- [chatwidget/protocol.rs](../codex-rs/tui/src/chatwidget/protocol.rs)：分发服务端通知。
- [app/input.rs](../codex-rs/tui/src/app/input.rs) 与 [chatwidget/interaction.rs](../codex-rs/tui/src/chatwidget/interaction.rs)：在普通输入取消逻辑之前处理等待中空输入框的 Ctrl+C，包括按键组合路由入口。
- [app/event_dispatch.rs](../codex-rs/tui/src/app/event_dispatch.rs)：将取消事件发送给服务端。
- [chatwidget/session_flow.rs](../codex-rs/tui/src/chatwidget/session_flow.rs)：切换线程时清理本地等待显示。

服务器会重新发布等待状态。重新连接取得原有截止时间，不重新计时。TUI 约每 100 毫秒请求重绘，英文占位文字显示剩余时分秒；等待期间不自动提交排队消息。切换线程只清除本地显示，不取消原线程等待。

等待且输入框为空时，Ctrl+C 发送 `ExitMode::Immediate`，不发送取消等待、线程中断或服务器关闭请求。这样后台继续等待。Ctrl+B 没有自定义退出逻辑，按普通输入处理。已有草稿时接到等待通知，会取消等待并保留草稿。

## 服务商与模型选择

`provider::model` 选择器先解析配置中的服务商 ID，再将不带前缀的模型名传给 API。无前缀时沿用当前服务商。支持 TUI `/model`，以及 app-server `turn/start`、`thread/settings/update` 的 `model` 字段。切换作用于后续回合，保留当前线程历史。

- [config/model_selection.rs](../codex-rs/core/src/config/model_selection.rs)：选择器解析、注册服务商查找及受管理服务商约束。
- [session/environment.rs](../codex-rs/core/src/session/environment.rs)：线程设置更新及运行时服务商切换。
- [core/src/client.rs](../codex-rs/core/src/client.rs)：`ModelClient::for_provider` 保留线程元数据，为新服务商建立独立认证、连接、WebSocket、路由和工具缓存状态。
- [chatwidget/model_command.rs](../codex-rs/tui/src/chatwidget/model_command.rs)：解析 `/model`，包括没有模型目录的自定义服务商。
- [chatwidget/session_model_selection.rs](../codex-rs/tui/src/chatwidget/session_model_selection.rs)：TUI 会话模型选择。
- [model-provider/src/provider.rs](../codex-rs/model-provider/src/provider.rs)：认证和第一方 attestation 路径隔离。

运行时继续使用原会话的认证管理器；第三方服务商使用自己的 `env_key` 或配置的认证方式，不覆盖 ChatGPT 登录。变化后的服务商不复用不匹配的预热连接，压缩等请求也使用当前服务商。

第三方目前需要 Responses API。没有增加 Chat Completions 适配。切换服务商时清除前端过期的额度显示。

### 子代理

- [agent/child_config.rs](../codex-rs/core/src/agent/child_config.rs)：子代理模型与服务商选择。
- [agent/role.rs](../codex-rs/core/src/agent/role.rs)：角色配置选择已注册服务商，保留权限等父线程约束。
- [tools/handlers/multi_agents_spec.rs](../codex-rs/core/src/tools/handlers/multi_agents_spec.rs)：子代理工具的模型参数说明。

新建无历史继承的子代理可以显式指定 `provider::model`，`agents.default_subagent_model` 也支持此形式。角色配置可指定已注册的 `model_provider` 或限定模型名；不能借此重定义 endpoint、凭据或放宽权限。子代理服务商选择不改变父线程。订阅主代理与 API 子代理，以及反向组合均有模拟 HTTP 集成测试。

## 版本与更新

[cli/src/main.rs](../codex-rs/cli/src/main.rs) 的 CLI 版本添加 `+auto_continue.0.1.0`；TUI 使用同样的显示后缀。版本格式为 `<官方版本>+auto_continue.<自定义版本>`，当前为 `0.165.0+auto_continue.0.1.0`。自定义版本按 SemVer 独立递增；合并官方更新时不重置自定义版本。[tui/src/update_versions.rs](../codex-rs/tui/src/update_versions.rs) 的版本比较忽略构建后缀，[tui/src/update_action.rs](../codex-rs/tui/src/update_action.rs) 不提供直接安装操作。

更新检查沿用 `check_for_update_on_startup`。普通 debug 构建默认关闭官方更新检查；README 开发构建命令通过针对 TUI 的 `debug-assertions=false` 保留更新检查，并关闭调试符号和增量缓存来降低构建磁盘占用。这些构建环境变量不修改 Codex 配置。独立 `codex-app-server` 的版本可能只显示官方基础版本，不附带 CLI 后缀。

## 后台服务器程序副本

- [app-server-daemon/src/prepare_install.rs](../codex-rs/app-server-daemon/src/prepare_install.rs)：安装或替换后台服务程序包。
- [app-server-daemon/src/lib.rs](../codex-rs/app-server-daemon/src/lib.rs)：start、stop、restart 及运行中版本查询。

后台服务使用独立保存的程序包；普通 restart 沿用已有副本。`daemon update --from-cli --yes` 将当前 CLI 程序包固定为服务端版本，并在服务运行时重启。不带 `--from-cli` 的更新会返回官方生产更新渠道。管理服务使用相同用户与 `CODEX_HOME`；后台进程需要自己的 API key 环境变量。

## 源码构建的完整程序包

`daemon update --from-cli` 不接受孤立的 Cargo 二进制。安装上下文从实际可执行文件的路径识别包布局；`bin/codex` 上一级必须有 `codex-package.json`，并包含 code-mode host、ripgrep 及当前平台的沙箱资源。符号链接可以保留原包路径的识别，直接复制单个二进制则会丢失包上下文。

- [scripts/build_auto_continue_package.py](../scripts/build_auto_continue_package.py)：本 fork 的打包入口，读取 Rust 中的显示版本，核对预编译 CLI 的版本，并选择本机 Rust target；复用官方完整包构建器。
- [scripts/codex_package/layout.py](../scripts/codex_package/layout.py)：包目录、资源复制及清单生成。
- [scripts/codex_package/cargo.py](../scripts/codex_package/cargo.py)：缺少预编译输入时构建辅助程序。本 fork 打包入口默认使用本机 target 和原有 Cargo 输出目录；显式传入 `--target` 时使用官方跨目标构建方式。构建 code-mode host 时，通过官方 `v8.py` 获取并校验匹配的 Codex V8 sandbox 制品；直接编译 host 而不指定这些制品时，denoland 默认下载源可能返回 404。
- [install-context/src/lib.rs](../codex-rs/install-context/src/lib.rs)：从实际可执行文件路径识别完整包。
- [app-server-daemon/src/prepare_install.rs](../codex-rs/app-server-daemon/src/prepare_install.rs)：完整性、平台和可执行文件摘要校验；复制后固定版本，运行中的服务完成替换后重启。

CLI 显示仍使用用户指定的 `0.165.0+auto_continue.0.1.0`。下划线不符合严格 SemVer，因此机器读取的包清单版本写为 `0.165.0+auto-continue.0.1.0`。这是包元数据规范化，不改变 CLI/TUI 显示。带构建元数据的本地包不会被当作官方稳定包自动跟随更新。

这里纠正此前仅复制二进制的安装说明：该方式可以启动前端，但无法提供后台服务安装所需的完整包，也会缺失辅助程序。完整包应整体保留；`--from-cli` 必须通过包内的 CLI 或指向它的符号链接执行。

## 验证

在 `codex-rs` 目录执行。下面的环境变量仅影响本次测试，不修改用户配置：

```bash
export CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=4
cargo fmt --all --check
cargo test --locked -p codex-tui --lib auto_retry
RUST_MIN_STACK=16777216 cargo test --locked -p codex-tui --lib background_exit
cargo test --locked -p codex-tui --lib update_versions
cargo test --locked -p codex-app-server --lib auto_retry
cargo test --locked -p codex-app-server --test all auto_retry
cargo test --locked -p codex-app-server --test all provider_switch
```

`background_exit` 包含需要较大线程栈的既有集成测试，上述命令仅对该测试进程设置 16 MiB 栈。

对应测试代码：

- [服务端策略单测](../codex-rs/app-server/src/auto_retry.rs)：退避边界、刷新消息解析、结构化额度错误、刷新后额外 1 秒、取消和 goal 状态顺序。
- [TUI 等待单测](../codex-rs/tui/src/chatwidget/auto_retry.rs)：重连截止时间、Ctrl+C 退出保留等待、Ctrl+B 不再退出、粘贴取消及迟到通知。
- [TUI 退出测试](../codex-rs/tui/src/app/tests/background_exit_tests.rs)：按键组合路由及直接输入入口不会取消等待或中断线程。
- [后台恢复集成测试](../codex-rs/app-server/tests/suite/v2/auto_retry.rs)：无客户端连接时自动续跑、重连显示和取消原倒计时、断开连接的 goal 恢复。
- [多服务商集成测试](../codex-rs/app-server/tests/suite/v2/provider_switch.rs)：订阅与 API 来回切换保留历史和认证隔离、主子代理双向混用。

集成测试使用本地模拟 API，不消耗真实订阅或 API 额度，也不代替真实限额耗尽时的端到端验证。

打包入口回归测试（在仓库根目录执行）：

```bash
PYTHONPATH=scripts python3 -m unittest codex_package.test_auto_continue
```
