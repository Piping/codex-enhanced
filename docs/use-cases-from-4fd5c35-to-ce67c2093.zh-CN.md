# 基于 `4fd5c35` 之后 commits 推导的 Use Cases

本文总结了从以下 commit range 中可以推导出的产品与运维侧 use cases：

- base: `4fd5c35c4f6f51048f47c8680ed0f6a26c608f68`
- head: `ce67c2093`

本分析采用 `effective-use-cases` 方法。
它首先把仓库视为一个 black-box system，再从 commit themes、文档和 release flow 中反推出 user-goal behavior。

## Scope And Goal Level

- Scope: `codex-enhanced`，作为一个面向长时间运行 Codex 工作的 persistent user surface，覆盖 sessions、profiles、workflows、local memory 和 external message channels。
- Goal level: 以 sea-level 的 user-goal use cases 为主，少量补充 maintainer-facing 的 operational use cases，用于描述 packaging 或 runtime coordination 这类对外可感知的行为变化。
- Source basis: commit messages、变更后的 docs、TUI feature surfaces、release workflow changes，以及新增的 `clawbot` Feishu Base coordination 行为。

### 本次分析的 Non-goals

- 不尝试把每一个 refactor、replay commit 或 CI 修复都单独描述成一个 use case。
- 不把 internal test harness changes 或 alternate build-backend support 当作独立 end-user goal，除非它们改变了对外可感知的保证。
- 不把每一个 TUI control 或 menu item 都拆成单独 use case；如果它们只是服务于更高层 user goal，就并入更大的 use case。

## Actor And Stakeholder Map

### Primary actors

- Workspace user: 日常在 TUI/CLI 中使用 Codex，运行、恢复、引导、监控并自动化长生命周期工作。
- External collaborator: 从 Feishu 向一个已绑定的 Codex workflow 发消息，并期待拿到正确路由后的回复。
- Release maintainer: 将 `codex-enhanced` 发布为 multi-platform Python wheel。

### Supporting actors

- Model provider / profile endpoint: 提供 inference，并可能出现 rate-limit、overload 或 auth failure。
- Workflow scheduler 和 app-server runtime: 执行 triggers、jobs 和 follow-up turns。
- Feishu platform: 提供 inbound messages，并接受 outbound replies、reactions 与 websocket ownership。
- Feishu Base: 存储 clawbot 共享 coordination state，用于 leadership 选举和 forced websocket preemption。
- Local filesystem 和 repo state: 存储 workflow YAML、profile routing、memory artifacts、reports 和 workspace-local clawbot bindings。

### Off-stage stakeholders 及其 interests

- 依赖 continuity 的团队成员：希望有 saved sessions、jump navigation 和可恢复状态，而不是每次都重建 context。
- Repository maintainers: 希望有 local memory、更新后的 `AGENTS.md`，以及可复用的 repo-local skills。
- PyPI consumers: 希望拿到可安装、且内嵌 native runtime 正确匹配的平台 wheel。
- 同时运行多个 Codex processes 的 users: 希望某个 Feishu websocket 在任一时刻只被一个 process 持有，并且在需要时可以显式 preempt。
- 处于中断场景下的 users: 希望有 retries、fallback、可见失败，以及非静默降级。

## Commit Theme Clusters

下表概括了这个 commit range 中最主要的 behavioral clusters。

| Cluster | Representative commits | Inferred behavior direction |
| --- | --- | --- |
| Profile routing and session continuity | `c7d306a2a`, `d1461727f`, `ac08abeae`, `fed91c14a`, `ff71e811a`, `d6da73601` | 让 Codex 能在 profile failure、respawn、thread routing 和 resumed work 场景下持续在线。 |
| Workflow orchestration and follow-up automation | `a59a3a6a6`, `6e759655a`, `29a75c238`, `ea74b8961`, `ee7ff5e54`, `71ba01403`, `58f90f9ae`, `8049f9d1a` | 把 prompts 变成可重复执行的 jobs，并支持 triggers、timeouts、retries、bound-thread routing 和 non-blocking follow-up turns。 |
| Feishu clawbot bridge and message routing | `55ec5cdb5`, `063dfd100`, `2ee91453f`, `ff8875c17`, `9951bec11`, `acd2b529d`, `27b0b33e9` | 把外部 chats 绑定到 Codex threads，并保持 inbound/outbound message delivery 稳定。 |
| Feishu websocket ownership coordination | `5eabe81c2`, `ce67c2093` | 让多个 Codex processes 通过 Feishu Base 协调 embedded Feishu websocket ownership，包括 force-preempt 与 auto-provisioned coordination tables。 |
| Human-in-the-loop control and low-noise TUI | `cb099a038`, `329b4e1ae`, `406389863`, `0f571b53c`, `83ad3dfe2`, `5b9d0af78`, `415cad316` | 让 user surface 在长会话中更可导航、更结构化、噪音更低。 |
| Insight and retrospective memory | `55d4e11aa`, `88e75bc1d`, `ce9700ab5`, `5b71b8ecf`, `10ab81459` | 把 session history 转成 reports、repo memory、更新后的 instructions 和可复用 skills。 |
| Packaging and release hardening | `cbd9e0d7d`, `b30d46c3b`, `4194a80f1`, `d359e3ffa`, `dae2b6356` | 交付可靠的 tagged releases 和 multi-platform runtime wheels，避免 artifact 不匹配。 |
| Runtime resilience and portable validation | `b83f6f399`, `aa67cc953` | 在 retry logic 或 alternate codegen backend 存在时，保持 streaming behavior 和 validation guarantees 稳定。 |

## Use Case Inventory

| ID | Use case | Primary actor | Goal |
| --- | --- | --- | --- |
| UC-1 | Keep Codex running across profiles and failures | Workspace user | 在 rate limits、auth failures 或 provider-specific outages 下继续工作。 |
| UC-2 | Resume and navigate long-running workspace sessions | Workspace user | 重新进入一个已保存 thread，并快速恢复 operational context。 |
| UC-3 | Define and manage workspace-local workflows | Workspace user | 把重复性工作变成保存在 repo-local YAML 中、可运行的 jobs 和 triggers。 |
| UC-4 | Run follow-up automation without blocking the main thread | Workspace user | 让 turn completion 或 file events 触发更多工作，同时保持主会话响应。 |
| UC-5 | Bridge Feishu conversations into Codex threads | External collaborator 和 workspace user | 接收来自 Feishu 的 inbound messages，把它们绑定到正确 thread，并把最终回复发回外部。 |
| UC-6 | Coordinate Feishu websocket ownership across Codex processes | Workspace user | 确保某个 Feishu app 的 embedded websocket 只由目标 Codex process 持有，并允许显式 preemption。 |
| UC-7 | Keep the user in the loop with structured control | Workspace user | 通过显式回答、隐藏状态检查、历史跳转和降噪 UI 维持操作可控。 |
| UC-8 | Inspect local session behavior offline | Workspace user | 从 rollout history 生成一个可检查的离线报告，而不是依赖 hosted analytics。 |
| UC-9 | Convert a completed thread into reusable repo memory | Workspace user | 运行 `/dream`，把 thread 结果转成 memory、`AGENTS.md` 更新和 repo-local skills。 |
| UC-10 | Publish a correct multi-platform `codex-enhanced` release | Release maintainer | 构建、打 tag、校验并发布 wheels，确保其中嵌入的 runtime 与 release version 匹配。 |

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
  - Runtime: invalid 或不可运行的 workflow conditions 必须可见地失败。
- Preconditions:
  - Workspace 能存储 `.codex/workflows/*.yaml`。
  - User 可以打开 `/workflow`。
- Minimal guarantees:
  - Workflow files 保持 local 且可检查。
  - Parse 或 validation failures 不会被部分隐藏。
  - 失败的 workflow runs 不会无限阻塞其他 interactive surface。
- Success guarantees:
  - User 可以创建、编辑、启用、停用和运行 workflows。
  - 支持的 triggers 和 job settings 能按预期 thread/context behavior 执行。
- Trigger:
  - User 想把一类重复任务自动化，或管理已有 workflow jobs。
- Main success scenario:
  1. User 打开 `/workflow`。
  2. Codex 加载 workspace-local workflow definitions。
  3. User 创建或编辑 workflow、jobs 和 triggers。
  4. Codex 校验 workflow，并把 YAML 持久化到本地。
  5. User 手动运行 workflow，或等待某个 trigger 触发。
  6. Codex 执行 workflow，同时不阻塞无关的 user actions。
- Extensions:
  - 3a. Workflow 使用 timeout、retry 或 background execution：
    Codex 会跨 rounds 和 runtime boundaries 保持这些语义。
  - 4a. YAML 无效或 no-op：
    Codex 明确报告失败，而不是假装 workflow 已激活。
  - 6a. Triggered work 绑定到某个 thread：
    Codex 会把 follow-up 路由到对应的 bound thread，而不是生成无关上下文。

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

## UC-9 Convert A Completed Thread Into Reusable Repo Memory

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

## Acceptance Criteria

- Users 可以在 profile 出错时继续工作，而不需要手工改环境。
- Saved sessions 可以被恢复，并具有足够的 navigation 和 visibility control 来快速找回 context。
- Workflows 是 repo-local、可编辑、可运行的，并且在 misconfigured 或 unrunnable 时会显式失败。
- After-turn 和 background workflow activity 不会冻结主 interactive thread。
- Feishu session bindings 能在常见 runtime disruptions 下保持可用，并支持可见的 inbound/outbound handling。
- 当多个 Codex processes 共用同一个 Feishu app 时，只有 elected owner 会保持 embedded websocket active。
- Users 可以为当前 session 显式 preempt websocket ownership，并且当 force refresh 停止后，这种 preemption 会自然失效。
- 当 table IDs 未预配置时，clawbot 可以发现或 auto-provision 必需的 Feishu Base coordination tables 和 fields。
- `/dream` 通过 managed sections 生成 repo-local memory updates，而不是做 ad hoc file rewrites。
- `/insight` 可以从本地 rollout history 生成 offline report。
- Tagged `codex-enhanced` releases 会在发布 wheels 之前校验 version alignment。

## Implementation Slices Suggested By The Use Cases

- Slice 1: profile-router runtime continuity、fallback policy，以及 respawn/session-arg preservation。
- Slice 2: session continuity UX，包括 resume picker、jump-to-message、timestamps 和 visibility preferences。
- Slice 3: workflow scheduler/runtime、trigger semantics、timeout/retry handling，以及 TUI workflow controls。
- Slice 4: Feishu clawbot bridge、binding store、message delivery 和 session recovery flows。
- Slice 5: Feishu Base coordination backend、heartbeat/force-intent election、auto-provisioned schema management，以及 leader-or-follower runtime behavior。
- Slice 6: structured human-in-the-loop controls，包括 `question`、更强的 file reading/search tools，以及 lower-noise TUI affordances。
- Slice 7: retrospective stack，包括 `/dream`、repo-local memory storage、managed `AGENTS.md` updates，以及 generated skill updates。
- Slice 8: offline observability，通过 `/insight` 实现。
- Slice 9: Python runtime packaging 的 release automation、artifact reuse 和 multi-platform validation。

## Open Questions

- Feishu Base coordination 是否应该一直保持 `app_id`-global，还是未来要按 channel 或 bound conversation 进一步切分 ownership？
- `force-connect` 应继续作为 persistent workspace setting，还是应该演进为一个显式的 session-scoped lease，并带有更强的 expiry semantics？
- Feishu 是否会一直是唯一 external bridge，还是长期目标其实是一个更通用的 “generic external user inbox”，而 Feishu 只是第一个 adapter？
- `/dream` 是否应继续以 repo-root 为中心，还是未来要自动下钻到 nested `AGENTS.md` scopes？
- `/insight` 是否应继续保持为纯本地 offline artifact，还是未来应反过来 feeding runtime guidance，以及 profile/workflow tuning loops？
