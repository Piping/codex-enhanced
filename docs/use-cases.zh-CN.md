# codex-enhanced Use Cases

本文是 `codex-enhanced` 当前阶段的整合版 use case 文档。

它整合了两类来源：

- 主功能演进分析：
  - `4fd5c35 -> ce67c2093`
- rebase 后增量行为分析：
  - `8885e63e7 -> 27c48fabe`

本文采用 `effective-use-cases` 方法：

1. 先固定系统边界。
2. 再识别主要 actor 与 stakeholder。
3. 以 sea-level 的 user-goal use cases 为主组织行为。
4. 把成功保证、失败扩展、接受标准和实现切片统一整理成一份当前视图。

旧的 range 文档可继续保留，用作历史分段分析；本文件是“当前整合版”。

## Scope And Goal Level

- Scope: `codex-enhanced` 作为一个面向长期 Codex 工作的 persistent user/developer surface，覆盖 sessions、profiles、workflows、hidden helper threads、external message channels、repo-local memory、offline insight，以及 release/developer workflow。
- System boundary: 把 `codex-enhanced` 视为一个 black-box system，而不是把 TUI、app-server、protocol、workflow runtime、clawbot、Cargo 配置分别看成多个产品。
- Goal level: 以 sea-level user-goal use cases 为主，补充少量 maintainer-facing 和 contributor-facing operational use cases。

### Non-goals

- 不把每一个 refactor、fmt、rebase、局部测试修复、schema 导出细节直接写成独立 use case。
- 不把每一个快捷键、菜单项或单个 UI 控件都拆成独立 use case；只有当它们承载独立用户目标时才单列。
- 不把内部构建 backend 例外名单、feature-gating 或 test harness 行为直接当作最终用户目标。

## Actor And Stakeholder Map

### Primary actors

- Workspace user: 日常在 TUI/CLI 中使用 Codex，运行、恢复、引导、监控并自动化长生命周期工作。
- External collaborator: 从 Feishu 向一个已绑定的 Codex 会话或 workflow 发消息，并期待收到正确路由后的回复。
- Repository contributor: 在 `codex-rs` 中做本地开发、调试、验证和知识沉淀。
- Release maintainer: 将 `codex-enhanced` 发布为多平台 Python runtime release。

### Supporting actors

- Model provider / profile endpoint: 提供 inference，并可能出现 rate-limit、auth failure 或 overload。
- Workflow scheduler 和 app-server runtime: 执行 triggers、jobs、follow-up turns 和 hidden helper threads。
- Hidden helper thread runtime: 为 `/btw` 提供独立但继承上下文的隐藏线程。
- Feishu platform: 提供 inbound messages，并接受 outbound replies、reactions 与 websocket ownership coordination。
- Feishu Base: 存储 clawbot 共享 coordination state。
- Local filesystem 和 repo state: 存储 workflow YAML、progress、memory artifacts、reports、repo-local bindings 和 instructions。
- Rust toolchain / Cargo backend: 为 contributor 提供本地开发与构建路径。

### Off-stage stakeholders and interests

- 依赖 continuity 的团队成员：希望有 saved sessions、jump navigation、bound-thread continuity 和可恢复状态。
- 长会话用户：希望在不打断主线程的情况下获得辅助思考，并能看见每一轮工作完成时间与耗时。
- 需要把内容粘贴到外部系统的用户：希望得到干净纯文本，而不是带 markdown 装饰和本地绝对路径，并且终端 scrollback 保留模型原始逻辑行。
- Repository maintainers: 希望 repo-local 经验能沉淀为 memory、`AGENTS.md` 和 skills，而不是散落在临时聊天里。
- Future contributors: 希望本地 dev build 足够快、足够稳定，并有 repo-local 规则可复用。
- PyPI/runtime consumers: 希望安装到的 enhanced runtime 与 release tag 对齐且平台正确。
- 同时运行多个 Codex processes 的 users: 希望某个 Feishu websocket 任一时刻只由一个 elected owner 持有，并允许显式 preemption。

## Commit Theme Clusters

| Cluster | Representative range or commits | Inferred behavior direction |
| --- | --- | --- |
| Profile routing and session continuity | `4fd5c35 -> ce67c2093` | 让 Codex 在 profile failure、respawn、thread routing 和 resumed work 场景下继续可用。 |
| Workflow orchestration and follow-up automation | `4fd5c35 -> ce67c2093` | 把 prompts 变成可重复执行的 jobs，并支持 triggers、timeouts、retries、bound-thread routing 和 non-blocking follow-up turns。 |
| Feishu clawbot bridge and message routing | `4fd5c35 -> ce67c2093` | 把外部 chats 绑定到 Codex threads，并保持 inbound/outbound message delivery 稳定。 |
| Feishu websocket ownership coordination | `4fd5c35 -> ce67c2093` | 让多个 Codex processes 通过 Feishu Base 协调 embedded Feishu websocket ownership，包括 force-preempt 与 auto-provisioned coordination tables。 |
| Human-in-the-loop control and low-noise TUI | `4fd5c35 -> ce67c2093` | 让 user surface 在长会话中更可导航、更结构化、噪音更低。 |
| Insight and retrospective memory | `4fd5c35 -> ce67c2093` | 把 session history 转成 reports、repo memory、更新后的 instructions 和可复用 skills。 |
| 编辑回退与纯文本复制恢复 | `fe65971e4`, `0484f4d8e` | 恢复高频键盘流工作方式，让复制结果更适合外部粘贴，并避免主消息因为 viewport 宽度被预先拆成多条逻辑行。 |
| `/btw` 收敛到真实 subagent thread | `27c48fabe` | 让隐藏讨论线程真正复用现有 subagent/thread-spawn 语义。 |
| 完成态时间可观测性 | `60bb4dd67`, `27c48fabe` | 从“知道何时结束”提升到“同时知道何时结束和耗时多久”。 |
| repo-local contributor workflow | `cdc81528c`, `27c48fabe` | 把长期规则与临时进度分离，并恢复可持续的本地 nightly + cranelift 开发路径。 |
| Packaging and release hardening | `4fd5c35 -> ce67c2093`, `6bd30c30d` | 交付可靠的 tagged releases 和 multi-platform runtime wheels，并保持 release version alignment。 |

## Use Case Inventory

| ID | Use case | Primary actor | Goal |
| --- | --- | --- | --- |
| UC-1 | Keep Codex running across profiles and failures | Workspace user | 在 rate limits、auth failures 或 provider-specific outages 下继续工作。 |
| UC-2 | Resume and navigate long-running workspace sessions | Workspace user | 重新进入已保存 thread，并快速恢复 operational context。 |
| UC-3 | Define and manage workspace-local workflows | Workspace user | 把重复性工作变成保存在 repo-local YAML 中、可运行的 jobs 和 triggers。 |
| UC-4 | Run follow-up automation without blocking the main thread | Workspace user | 让 turn completion 或 file events 触发更多工作，同时保持主会话响应。 |
| UC-5 | Bridge Feishu conversations into Codex threads | External collaborator 和 workspace user | 接收来自 Feishu 的 inbound messages，把它们绑定到正确 thread，并把最终回复发回外部。 |
| UC-6 | Coordinate Feishu websocket ownership across Codex processes | Workspace user | 确保某个 Feishu app 的 embedded websocket 只由目标 Codex process 持有，并允许显式 preemption。 |
| UC-7 | Keep the user in the loop with structured control | Workspace user | 通过显式回答、隐藏状态检查、历史跳转和降噪 UI 维持操作可控。 |
| UC-8 | Use keyboard shortcuts to revise and export current content | Workspace user | 在不中断编辑流的情况下回退上一条输入，或把当前内容导出成干净纯文本。 |
| UC-9 | Start a real hidden helper discussion beside the current thread | Workspace user | 用 `/btw` 启动一个继承当前线程能力的隐藏子线程，同时不丢失主线程上下文。 |
| UC-10 | See when a turn finished and how long it took | Workspace user | 从完成态分隔信息里快速理解这轮工作完成时间和实际 turn duration。 |
| UC-11 | Inspect local session behavior offline | Workspace user | 从 rollout history 生成一个可检查的离线报告，而不是依赖 hosted analytics。 |
| UC-12 | Convert a completed thread into reusable repo memory | Workspace user | 运行 `/dream`，把 thread 结果转成 memory、`AGENTS.md` 更新和 repo-local skills。 |
| UC-13 | Maintain repo-local working rules and task progress | Repository contributor | 在 repo 内把长期规则和临时进度分层沉淀，而不是混写。 |
| UC-14 | Develop locally with a usable nightly + cranelift path | Repository contributor | 在不放弃日常开发效率的前提下继续本地构建和验证 `codex-rs`。 |
| UC-15 | Publish a correct multi-platform `codex-enhanced` release | Release maintainer | 构建、打 tag、校验并发布 wheels，确保 embedded runtime 与 release version 匹配。 |

## Fully Dressed Priority Use Cases

## UC-1 Keep Codex Running Across Profiles And Failures

- Scope: `codex-enhanced`
- Level: sea-level user goal
- Primary actor: workspace user
- Stakeholders and interests:
  - User: 不应该因为单个 profile 失败而中断当前工作。
  - 依赖 user surface 的团队成员：runtime routing 不应因为一次失败就需要手工改配置。
  - Model provider account owner: 失败必须被显式处理，而不是被隐藏。
- Preconditions:
  - User 已配置一个或多个 profiles。
  - Runtime 正在某个 repo 或 workspace session 中运行。
- Minimal guarantees:
  - Failure state 会被明确暴露。
  - 现有 routing 和 config state 会被保留。
  - Session continuity 不会因为 profile switch 而被静默破坏。
- Success guarantees:
  - 工作可以在一个可用 profile 上继续进行。
  - User 可以显式看到并管理 fallback route。
- Trigger:
  - 某个 provider 因 rate limit、auth failure 或 overload 出错，或者 user 主动想切换 runtime profile。
- Main success scenario:
  1. User 打开或使用 profile routing controls。
  2. Codex 发现当前 profile 不再适合，或 user 选择了另一条 route。
  3. Codex 在切换 active route 时保持 session 和 runtime continuity。
  4. 当前 thread 无需手工改环境变量即可继续使用。
- Extensions:
  - 2a. 没有配置 fallback route：
    Codex 报告问题，并把控制权交还给 user，而不是臆造一条 route。
  - 3a. `thread_unsubscribe` 或 session handoff 会破坏 continuity：
    Codex 保持 bound thread 和 session 行为，避免丢失当前工作面。
  - 3b. CLI 发生 respawn：
    Session arguments 和 routing context 会跨 respawn 被保留。

## UC-3 Define And Manage Workspace-Local Workflows

- Scope: `codex-enhanced`
- Level: sea-level user goal
- Primary actor: workspace user
- Stakeholders and interests:
  - User: 重复工作应该沉淀成显式的 YAML-backed automation，而不是反复手敲 prompt。
  - Repo collaborators: workflow definitions 应保持 local、可检查、可编辑。
  - Runtime: invalid、互相冲突或不可运行的 workflow conditions 必须可见地失败。
  - Main thread continuity: workflow follow-up 不能悄悄丢失、串错线程，或在 compact/fork 过程中破坏当前工作面。
- Preconditions:
  - Workspace 能存储 `.codex/workflows/*.yaml`。
  - User 可以打开 `/workflow`。
- Minimal guarantees:
  - Workflow files 保持 local 且可检查。
  - Parse 或 validation failures 不会被部分隐藏，也不会对旧字段或歧义配置做静默 fallback。
  - 失败的 workflow runs 不会无限阻塞其他 interactive surface。
- Success guarantees:
  - User 可以创建、编辑、启用、停用和运行 workflows。
  - 每个 job 都以显式 `context_strategy` 声明如何处理当前上下文。
  - 每个 job 都以显式 `execution_strategy` 声明如何继承或覆盖当前 session 的执行配置。
  - 支持的 triggers、`context_strategy`、`execution_strategy`、`response` 和 steps 会按声明的 thread/context behavior 执行。
- Trigger:
  - User 想把一类重复任务自动化，或管理已有 workflow jobs。
- Main success scenario:
  1. User 打开 `/workflow`。
  2. Codex 加载 workspace-local workflow definitions。
  3. User 创建或编辑 workflow、jobs 和 triggers，并为每个 job 指定 `context_strategy`、`execution_strategy`、`response`、`needs` 和 `steps`。
  4. Codex 严格校验 workflow，并把 YAML 持久化到本地。
  5. User 手动运行 workflow，或等待某个 trigger 触发。
  6. Codex 按 job 声明的 `context_strategy` 选择直接嵌入主线程输入、先 compact 再回灌主线程、或在 workflow child thread 中 `auto/new/fork/fork_compact` 执行。
  7. Codex 在执行 workflow 时保持主 interactive surface 可用，并把结果按 `response` 配置显示为 assistant cell 或回灌为 user follow-up。
- Extensions:
  - 3a. Workflow 使用 timeout、retry 或 background execution：
    Codex 会跨 rounds 和 runtime boundaries 保持这些语义。
  - 4a. YAML 无效、缺少必填 `context_strategy` / `execution_strategy`、仍使用旧 `context` 字段，或出现未知字段：
    Codex 明确报告失败，而不是假装 workflow 已激活。
  - 4b. Job 把 `embed` 或 `embed_compact` 与 `run` steps 组合：
    Codex 在加载期拒绝该 workflow，而不是在运行时降级或跳过这些 steps。
  - 4c. `before_turn` trigger 解析到 `embed_compact` job：
    Codex 在加载期拒绝该 workflow，因为主线程 inline compact 不支持该触发点。
  - 6a. `thread_fork` 或 `thread_fork_compact` 需要 materialized primary thread，但当前不存在可 fork 的主线程：
    Codex 明确失败，而不是偷偷退回 `thread_new`。
  - 7a. Triggered work 绑定到某个 primary thread：
    Codex 会把 follow-up 路由到对应的 bound thread，而不是生成无关上下文。
  - 7b. `embed_compact` 或 main-thread compact follow-up 在 compact 阶段失败：
    Codex 可见地报告失败，并且不会静默提交错误的 follow-up。
  - 7c. `after_turn` workflow 使用 `response: user` 且产生非空回复：
    Codex 把该回复提交回主线程，并允许它再次满足 `after_turn` 的触发条件。
  - 7d. `after_turn` workflow 返回空回复：
    Codex 停止 follow-up 链，而不是继续递归触发。
- Technology and data variations:
  - `context_strategy: embed`
    - 适用：把 workflow prompt 直接作为主线程输入继续执行。
    - 限制：只允许 prompt steps，不允许 `run`。
  - `context_strategy: embed_compact`
    - 适用：先 compact 主线程，再把 workflow prompt 回灌为主线程 follow-up。
    - 限制：只允许 prompt steps；不适用于 `before_turn`。
  - `context_strategy: thread_auto`
    - 适用：优先复用当前主线程上下文；若主线程已 materialized 则 fork，否则新开 workflow child thread。
  - `context_strategy: thread_new`
    - 适用：明确隔离 workflow 执行，不继承主线程上下文。
  - `context_strategy: thread_fork`
    - 适用：要求继承当前主线程上下文。
    - 限制：没有可 fork 的 materialized primary thread 时必须失败。
  - `context_strategy: thread_fork_compact`
    - 适用：先 fork 主线程，再 compact workflow child thread，随后执行 steps。
  - `execution_strategy: inherit_session`
    - 适用：继承当前 primary session 的 `cwd`、model、approval、sandbox 和 reviewer。
    - 限制：没有当前 primary session 时必须失败，不允许偷偷退回 app config。
  - `execution_strategy: override_yolo`
    - 适用：继承当前 primary session 的工作上下文，但把执行权限提升为 yolo。
    - 效果：`approval_policy = never`，`sandbox_policy = danger-full-access`。
  - `response: assistant`
    - Workflow 输出作为 transcript cell 展示，不继续驱动主线程 user turn。
  - `response: user`
    - Workflow 输出被重新提交为主线程 user follow-up，可继续触发 after-turn 语义。

## UC-5 Bridge Feishu Conversations Into Codex Threads

- Scope: `codex-enhanced`
- Level: sea-level user goal
- Primary actor: external collaborator，由 workspace user 支持
- Stakeholders and interests:
  - External collaborator: 消息应该到达正确的 Codex thread，并拿到回复。
  - User: bindings 应保持 local、可检查、可恢复。
  - Runtime: stale bindings 和 delivery failures 应被修复和协调，而不是持续累积。
- Preconditions:
  - Workspace 已配置 Feishu integration。
  - 一个 Codex thread 可以绑定到 Feishu session 或 channel。
- Minimal guarantees:
  - 即使 binding state 已陈旧，incoming messages 也不会被静默丢弃。
  - 发送 reactions 或 replies 失败时，错误会被暴露，并允许 retry 或 reconciliation。
  - Bindings 仍保存在 workspace-local state 中。
- Success guarantees:
  - Inbound message 会被路由进正确的 Codex operational loop。
  - Final reply 会被送回 Feishu。
  - Session jump 和 bound-thread continuity 在 reload 或 respawn 后仍可使用。
- Trigger:
  - 某个已绑定会话收到新的 Feishu message，或 user 在 `/clawbot` 中管理 bindings。
- Main success scenario:
  1. User 把一个 Feishu session 绑定到当前 Codex thread。
  2. Collaborator 在 Feishu 中发送消息。
  3. Codex runtime 接收事件，并把它映射到对应的 workspace thread。
  4. Codex 在正确的 execution mode 中处理这条 inbound message。
  5. Codex 把 final reply 发回 Feishu，并更新本地状态。
- Extensions:
  - 3a. Binding 已陈旧，或引用了尚未加载的 thread state：
    Codex 会做 binding reconciliation 或恢复 jump continuity，而不是让会话悬空。
  - 4a. Runtime 发生 respawn：
    Clawbot runtime 会重启并重新连接现有 operational state。
  - 5a. Reaction cancellation 或 delivery update 失败：
    Codex 会报告失败，而不是假装外部状态已经更新。

## UC-6 Coordinate Feishu Websocket Ownership Across Codex Processes

- Scope: `codex-clawbot` 的 Feishu coordination，属于 `codex-enhanced` 的一部分
- Level: sea-level operational goal
- Primary actor: workspace user
- Stakeholders and interests:
  - 同时运行多个 workspaces 或 terminals 的 user: 对于同一个 Feishu app，只有目标 process 应该持有 embedded websocket。
  - External collaborators: inbound messages 应持续流向一个 live owner，而不是被重复消费或直接丢失。
  - Feishu app owner: runtime ownership state 应由 app 自身拥有、可检查、可修复，而不是依赖额外的付费协调器。
  - Runtime: stale leader、split-brain 和 schema drift 必须可见，而不是被静默容忍。
- Preconditions:
  - 已配置 Feishu app credentials。
  - `feishu.coordination.base_token` 指向一个该 app 可读可写的 Base。
  - 可能存在一个或多个 Codex processes 竞争同一个 Feishu `app_id`。
- Minimal guarantees:
  - 非 leader process 不会继续假装自己拥有 websocket。
  - 过期的 force-intent 或 heartbeat state 不再影响 leadership。
  - Permission、schema 或 table-resolution failures 会带着可修复指引被暴露出来。
  - Coordination state 存在于 Feishu Base 中，并由 app credentials 持有，而不是写进一个隐藏 sidecar service。
- Success guarantees:
  - 对于当前 `app_id`，恰好有一个预期中的 process 作为 websocket owner。
  - User 可以为当前 session 显式发起 ownership preemption。
  - 如果没有提供 table IDs，clawbot 会自动发现或创建必需的 coordination tables 和 fields。
- Trigger:
  - 一个配置了 coordination 的 clawbot runtime 启动、刷新 leadership，或 user 在 `/clawbot` 中启用 forced websocket preemption。
- Main success scenario:
  1. Codex 以启用了 Feishu coordination 的方式启动 clawbot runtime。
  2. Clawbot 解析自己的 process identity，并在 Feishu Base 中发现或创建 coordination tables。
  3. Clawbot 为当前 `app_id` 和 instance 写入或刷新 heartbeat row。
  4. Clawbot 读取 active heartbeat 和 force-intent rows，并以 deterministic 规则计算 elected owner。
  5. 如果当前 process 被选为 leader，它就打开或保持 websocket，并处理 inbound Feishu events。
  6. 如果未当选 leader，它就保持 follower mode，只继续发布 heartbeat。
  7. 当 user 启用 `force connect` 时，clawbot 会持续为当前 session 刷新 force-intent row，直到该状态被关闭。
  8. 其他 contenders 会在下一次 coordination refresh 时观察到新的 intent，并让出 websocket ownership。
- Extensions:
  - 2a. 配置的 `base_token` 无效或不可访问：
    Clawbot 会暴露失败，而不是假装 coordination 已生效。
  - 2b. 配置的 table IDs 已陈旧，或 schema 出现 drift：
    Clawbot 会校验 table shape，说明具体问题，并允许 repair 或 recreation，而不是写进一个不兼容 schema。
  - 4a. 上一个 leader 没有清理就消失：
    它的 heartbeat 会在 TTL 过期后失效，随后由下一个 live contender 接管 owner 身份。
  - 4b. 多个 contenders 具有相同 priority：
    Deterministic tie-break 会回退到 `instance_id`，再回退到 `session_id`。
  - 7a. `force connect` 被关闭，或 owning process 停止刷新：
    Force-intent 会自然过期，leader election 恢复为普通的 priority-based 模式。

## UC-8 Use Keyboard Shortcuts To Revise And Export Current Content

- Scope: `codex-enhanced`
- Level: sea-level user goal
- Primary actor: workspace user
- Stakeholders and interests:
  - User: 高频编辑动作应可通过键盘完成，而不是被迫切回 shell 或手工清洗文本。
  - 需要把结果粘贴到外部系统的协作者: 导出的文本应去掉 markdown 装饰、本地绝对路径包装和多余符号。
  - 依赖终端复制的用户: 主 assistant message 即使在窄窗口下展示，也不应因为 scrollback 预换行而变成多条逻辑行。
- Preconditions:
  - 当前存在可回退的输入，或存在可导出的当前内容。
  - 用户正在支持增强快捷键的 TUI 会话中工作。
- Minimal guarantees:
  - 即使快捷键处理失败，当前草稿和已存在历史不会被静默破坏。
  - 对主 assistant message 的 scrollback 写入不会因为 viewport 宽度被改写其原始硬换行结构。
  - 视觉上的终端软折行不应被误写成新的历史内容。
- Success guarantees:
  - `Ctrl-X Ctrl-U` 可以回退上一条输入。
  - `Ctrl-X Ctrl-Y` 产出的纯文本不会保留 markdown 修饰，也不会保留括号包裹的本地绝对路径。
  - 主 assistant message 写入终端 scrollback 时保留原始逻辑行；只有模型自己输出的硬换行才会成为可复制的新行。
- Trigger:
  - 用户按下 `Ctrl-X Ctrl-U` 或 `Ctrl-X Ctrl-Y`。
- Main success scenario:
  1. 用户在当前 TUI 会话中按下一个增强编辑快捷键。
  2. Codex 识别该快捷键，并判断目标是回退输入还是导出当前文本。
  3. 如果是回退，Codex 恢复上一条输入内容，而不打断当前会话。
  4. 如果是纯文本导出，Codex 清理 markdown 修饰、本地绝对路径包装和不适合外部粘贴的格式。
  5. 当 assistant message 被写入 scrollback 时，Codex 保留原始逻辑行，而不是根据当前 viewport 宽度预先拆行。
  6. 用户复制出的结果保持可粘贴、可搜索、可再次加工。
- Extensions:
  - 2a. 当前没有可回退的上一条输入：
    Codex 保持当前状态不变，而不是插入错误内容。
  - 4a. 文本中包含括号包裹的本地绝对路径：
    Codex 在纯文本导出时移除该包装内容，而不是把本地路径暴露给外部系统。
  - 5a. 终端窗口被缩窄：
    终端可以继续做视觉软折行，但 Codex 不会把新的主消息预先写成多条逻辑历史行。

## UC-9 Start A Real Hidden Helper Discussion Beside The Current Thread

- Scope: `codex-enhanced`
- Level: sea-level user goal
- Primary actor: workspace user
- Stakeholders and interests:
  - User: 希望在不中断主线程的前提下，让模型在旁路线程里做额外思考或探查。
  - 主线程上下文所有者: `/btw` 不应擅自退化成只读假线程，也不应丢失与当前会话的父子关系。
  - Runtime: hidden thread 应与现有 subagent/thread-spawn 语义一致，便于恢复、路由、审计和后续扩展。
- Preconditions:
  - 当前至少存在一个可见线程，或用户可以从当前上下文启动一个新会话。
  - 用户有一个明确的 `/btw` 提示词。
- Minimal guarantees:
  - 即使 `/btw` 失败，主线程仍保持当前运行状态和可见上下文。
  - hidden thread 的权限不会被静默降级成另一套更弱或更强的默认值。
  - 不支持在 hidden 视图中安全承载的交互型请求会被明确拒绝，而不是悬挂。
- Success guarantees:
  - `/btw` 会创建或 fork 一个真实的 `subagent/thread_spawn` 子线程。
  - 子线程继承当前可见线程的 approval policy、sandbox policy 和 reviewer 语义。
  - 用户可以切到新的 hidden session thread id，同时原有会话仍可保持进行中。
  - 标准 command/file/permissions approval 流会沿用正常线程处理路径。
- Trigger:
  - 用户输入 `/btw <prompt>`。
- Main success scenario:
  1. 用户在当前会话中发起 `/btw`。
  2. Codex 读取当前可见线程，提取其权限和 sandbox 语义。
  3. Codex 以当前可见线程为 parent，创建或 fork 一个 hidden child thread。
  4. App-server 将该线程标记为真实的 `SubAgentSource::ThreadSpawn`。
  5. 用户界面切换到新的 hidden thread id，主线程上下文仍保留。
  6. Hidden thread 在与父线程一致的权限边界内运行，并返回临时讨论结果。
  7. 用户选择查看、插入或关闭该结果，而不破坏主线程。
- Extensions:
  - 3a. 当前线程无法被 fork：
    Codex 回退到 fresh `thread/start`，但仍附带正确的 `subAgentSpawn` 元数据。
  - 4a. `parentThreadId` 对应线程未加载：
    App-server 明确报错，而不是生成一个脱离树结构的匿名线程。
  - 6a. Hidden thread 请求标准 command/file/permissions approval：
    请求沿正常线程通道处理，而不是被 `/btw` 预先拒绝。
  - 6b. Hidden thread 请求用户输入、MCP elicitation 或其他当前客户端无法安全承载的交互：
    Codex 明确拒绝并结束该隐藏讨论。

## UC-12 Convert A Completed Thread Into Reusable Repo Memory

- Scope: `codex-enhanced`
- Level: sea-level user goal
- Primary actor: workspace user
- Stakeholders and interests:
  - Future user sessions: 应能继承此前工作沉淀出的 repo-local guidance。
  - Repo maintainers: 希望更新具备确定性，并限制在 managed sections 内。
  - Security-sensitive users: secrets 不应被复制进生成产物。
- Preconditions:
  - 存在一个包含有价值历史内容的 thread。
  - Repo-local storage 可用。
- Minimal guarantees:
  - `dream` blocks 之外的用户内容不会被覆盖。
  - 写入保持 repo-local。
  - 输出在落盘前会被校验并做 redaction。
- Success guarantees:
  - Repo-local memory artifacts 被写出。
  - `AGENTS.md` 和选定 `SKILL.md` 文件中的 managed blocks 被更新。
  - 新 session 启动时可以直接拿到一个明确的 next-session hint。
- Trigger:
  - User 对当前 thread 执行 `/dream`。
- Main success scenario:
  1. User 运行 `/dream`。
  2. Codex 加载当前 thread 及相关 local context。
  3. Codex 运行一个专门的 retrospective prompt。
  4. Codex 校验并 redacts 结构化结果。
  5. Codex 写入 memory files、managed instruction blocks 和 skill updates。
  6. Codex 重建 local memory index，并带着 next-session hint 启动一个新 thread。
- Extensions:
  - 4a. 模型返回 repo 外部路径：
    Codex 会拒绝这些路径，不写入不安全内容。
  - 5a. 目标文件不存在：
    Codex 会创建一个带 managed block 的小文件，而不是直接失败。
  - 5b. Managed markers 已存在：
    Codex 只替换 managed block，而不是重写整个文件。

## UC-15 Publish A Correct Multi-Platform `codex-enhanced` Release

- Scope: release workflow for `codex-enhanced`
- Level: sea-level operational goal
- Primary actor: release maintainer
- Stakeholders and interests:
  - End users: published wheel should install and include the correct native binary.
  - Maintainer: tag, runtime version, and embedded artifacts must match.
  - Release automation: concurrency and artifact reuse should not deadlock or publish the wrong bits.
- Preconditions:
  - Release version is chosen.
  - Tag and source state are ready.
- Minimal guarantees:
  - Version mismatch between tag and runtime package fails before publish.
  - Artifact reuse and publish flow are explicit.
  - Windows/macOS/Linux wheel packaging is separated and visible.
- Success guarantees:
  - Tagged release produces matching wheel artifacts.
  - Wheels are published to PyPI.
  - GitHub release assets are attached for the tagged version.
- Trigger:
  - Maintainer pushes a `v*.*.*` tag or dispatches the release workflow manually.
- Main success scenario:
  1. Maintainer selects the next release version.
  2. Maintainer updates the enhanced Python runtime version so it matches the intended tag.
  3. Maintainer creates or refreshes the release tag.
  4. Release automation builds platform artifacts and enhanced runtime wheels.
  5. Publish steps validate version alignment before upload.
  6. Wheels and release assets are published for the intended version.
- Extensions:
  - 2a. Tag version and `pyproject.toml` version differ:
    Release fails before publish and the mismatch is surfaced explicitly.
  - 3a. A manual dispatch races with an existing tag-triggered run:
    Concurrency cancellation is treated as expected workflow behavior, not as an unexplained publish failure.
  - 4a. A rerun only needs publish recovery:
    Artifact reuse should be considered before rebuilding every platform.

## Acceptance Criteria

- Users 可以在 profile 出错时继续工作，而不需要手工改环境。
- Saved sessions 可以被恢复，并具有足够的 navigation 和 visibility control 来快速找回 context。
- Workflows 是 repo-local、可编辑、可运行的，并且在 misconfigured 或 unrunnable 时会显式失败。
- Workflow job 必须显式声明 `context_strategy` 和 `execution_strategy`；旧 `context` 字段或未知字段不会被接受，也不会触发兼容降级。
- `embed` 与 `embed_compact` job 不能包含 `run` steps；这种组合会在加载期失败。
- `before_turn` workflows 不能使用 `context_strategy: embed_compact`。
- `thread_auto` 会在主线程可 fork 时 fork，否则新开 thread；`thread_fork` 不允许悄悄退化成 `thread_new`；`thread_fork_compact` 会在 child thread compact 后执行。
- `execution_strategy: inherit_session` 和 `override_yolo` 都要求存在当前 primary session；`override_yolo` 必须把 approval/sandbox 提升到 yolo，而不是只改文案。
- After-turn 和 background workflow activity 不会冻结主 interactive thread。
- `response: user` 的 workflow 回复会被提交回正确的 primary thread；若回复非空，它可以再次驱动 `after_turn`，若回复为空，递归链会自然停止。
- main-thread compact follow-up 失败时，错误是可见的，并且不会静默吞掉或错误提交 follow-up。
- Feishu session bindings 能在常见 runtime disruptions 下保持可用，并支持可见的 inbound/outbound handling。
- 当多个 Codex processes 共用同一个 Feishu app 时，只有 elected owner 会保持 embedded websocket active。
- Users 可以为当前 session 显式 preempt websocket ownership，并且当 force refresh 停止后，这种 preemption 会自然失效。
- 当 table IDs 未预配置时，clawbot 可以发现或 auto-provision 必需的 Feishu Base coordination tables 和 fields。
- `/dream` 通过 managed sections 生成 repo-local memory updates，而不是做 ad hoc file rewrites。
- `/insight` 可以从本地 rollout history 生成 offline report。
- `Ctrl-X Ctrl-U` 与 `Ctrl-X Ctrl-Y` 恢复可用。
- `Ctrl-X Ctrl-Y` 产出的纯文本不会保留 markdown 修饰，也不会保留括号包裹的本地绝对路径。
- 主 assistant message 写入 scrollback 时保留原始硬换行；缩窄窗口只影响视觉软折行，不应把新输出预写成多条可复制的逻辑行。
- `/btw` 创建出的 hidden thread 在 app-server 侧可观察到 `subagent/thread_spawn` source，而不是普通匿名临时线程。
- `/btw` 使用与当前可见线程一致的 approval/sandbox 语义。
- `/btw` 在主线程进行中时仍能切换到新的 hidden thread id。
- 回合完成后的历史分隔条同时展示：
  - 本地时间戳
  - 从 turn start 到 final response 的精确耗时
- `codex-rs` 仓库存在 repo-local `AGENTS.md`，并把详细任务进度导向 `progress.md`。
- 本地 `cargo build -p codex-cli` 在当前 nightly + cranelift 约定下可通过。
- Tagged `codex-enhanced` releases 会在发布 wheels 之前校验 version alignment。

## Implementation Slices

1. Profile-router continuity
   - profile fallback policy
   - respawn/session-arg preservation
   - bound-thread continuity

2. Session continuity UX
   - resume picker
   - jump-to-message
   - timestamps
   - visibility preferences

3. Workflow orchestration
   - strict YAML contract and validation
   - `context_strategy` routing and thread bootstrap semantics
   - `execution_strategy` inheritance and yolo override semantics
   - main-thread compact follow-up queue and failure handling
   - trigger semantics
   - timeout/retry handling
   - after-turn follow-up routing and retrigger behavior
   - TUI workflow controls

4. Feishu bridge and coordination
   - clawbot bridge
   - binding store
   - inbound/outbound delivery
   - heartbeat/force-intent election
   - auto-provisioned schema management

5. Structured human-in-the-loop controls
   - `question`
   - hidden state inspection
   - low-noise TUI affordances
   - keyboard revision/export shortcuts
   - hidden helper discussion threads

6. Completed-turn observability
   - completion timestamp
   - precise turn duration
   - completion-state separators

7. Retrospective and repo-local memory
   - `/dream`
   - repo-local memory storage
   - managed `AGENTS.md` updates
   - generated skill updates

8. Offline observability
   - `/insight`
   - local rollout-derived reports

9. Contributor workflow sustainability
   - repo-local `AGENTS.md`
   - `progress.md`
   - nightly + cranelift dev path
   - LLVM exceptions for incompatible crates
   - schema/export recipe consistency

10. Release automation
   - Python runtime packaging
   - artifact reuse
   - multi-platform validation
   - release version alignment

## Open Questions

- Feishu Base coordination 是否应该一直保持 `app_id`-global，还是未来要按 channel 或 bound conversation 进一步切分 ownership？
- `force-connect` 应继续作为 persistent workspace setting，还是应该演进为一个显式的 session-scoped lease，并带有更强的 expiry semantics？
- Feishu 是否会一直是唯一 external bridge，还是长期目标其实是一个更通用的 “generic external user inbox”，而 Feishu 只是第一个 adapter？
- `/dream` 是否应继续以 repo-root 为中心，还是未来要自动下钻到 nested `AGENTS.md` scopes？
- `/insight` 是否应继续保持为纯本地 offline artifact，还是未来应反过来 feeding runtime guidance，以及 profile/workflow tuning loops？
- `/btw` 目前仍明确拒绝某些客户端无法安全承载的交互型请求；后续是否要把这些请求也纳入 hidden thread UI surface？
- 完成态分隔条中的耗时目前是单 turn 级别；是否还需要一个跨多轮会话的累计耗时视图？
- repo-local `AGENTS.md` 与未来可提取成 skills 的内容边界，是否需要再补一份更严格的整理规则？
