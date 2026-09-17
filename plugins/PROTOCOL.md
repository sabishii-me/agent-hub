# Agent Bus Protocol(散文说明)

状态:**散文,不是权威**。协议的权威是 `../contract/adapter-v1.json`,由 `../tests/adapter-contract.mjs` 对照 hub 实际发出的请求、各插件实际处理的方法、各 manifest 实际声明的字段逐条检查。**两者不一致时以 adapter-v1.json 为准**,本文只保留机器合同放不下的散文(环境变量、数据归属、以及尚未被 hub 走通的配置面)。

本文写于桌面壳时期:**凡"壳"处,今天指 hub**;§8/§9/§10 记录的是已删除的 Rust 一致性与 UI,保留作为历史证据,不是今天的承诺。协议语言只有 **pass / fail**——没有"待实现"。

## 0. 总则

- 传输:stdin/stdout,每行一个 JSON-RPC 2.0 消息(UTF-8,LF 分隔)。
- 版本:manifest `protocol` 必须等于壳的 `PROTOCOL_VERSION`(当前 **0**),不等=装载拒绝。演进:**v0 内累加式扩展**,破坏性变更才升 `1`。
- 归属:协议归壳所有;事件白名单与 harness 翻译归各 adapter。
- **合同分两层**:
  1. **核心合规面**(§2/§3):任何 adapter 必须全部满足才能接单;缺件=一致性套件(adapter_conformance)fail=manifest 装载拒绝。**没有降级运行一说。**
  2. **真实 harness 能力位**(§1 `capabilities`):仅表达 harness 的本质局限;缺位=统一面板显示明确"不支持",**绝不 fallback 到别家**。
- 能力位**禁止**用作实现进度标记。§2/§3 的缺件不走能力位,走 fail。

## 1. 插件发现与清单

- 位置:`plugins/<id>/manifest.json`(插件即目录,目录名就是 id)。当前三家:pi、jouzu、deepseek。
```json
{ "id": "pi", "protocol": 0, "command": ["node", "pi-adapter.cjs"],
  "runtime": { "package": "@earendil-works/pi-coding-agent", "version": "0.85.1", "command": ["node", "runtime/dist/cli.js"] },
  "extensions": ["agent-presets", "plan"], "capabilities": ["models", "presets", "plan", "review"] }
```
- `id` 必须等于插件目录名;`protocol` 必须等于本 hub 讲的 adapter 协议版本(不等=拒绝装载);`command` argv 相对插件目录执行。
- **`runtime` 拥有 harness 的版本**——没有单独的 `pin`:一个事实写在两个字段里,总有一天会自相矛盾,而这个已经发生过。`scripts/prepare-runtimes.mjs` 按 `runtime.package`+`runtime.version` 物化运行时,hub 把 `runtime.command` 解析成绝对 argv 交给适配器(`PRTS_RUNTIME_COMMAND`)。
- 字段全集与逐字段含义见 `../contract/adapter-v1.json` 的 `manifest`(required/optional/fields)。
- `capabilities`(可选,字符串数组)= **真实 harness 局限**,合法值当前仅:`models`(harness 有可枚举模型目录)、**`providers`(配置面:§6 全部方法)**、`connectors`、`skills`(能消费对应配置)。`providerStatus` 废弃,并入 `providers`。注意:能力位声明的是"会诚实回答",不是"目录非空"——未接 provider 时 `models/list` 返回空数组+`failures` 是合法状态。
- 装载 gate:`protocol` 相等 **且** 该 adapter 通过 `adapter_conformance` 套件核心用例。未通过=不出现在 agent 列表。
- 能力位语义:决定统一面板可用状态与对应 RPC 的调用权限,**不决定管理入口是否存在**;缺位显示只读"该 harness 不支持"。

## 2. 核心合规面 A:会话生命周期(壳→插件请求,全部必需)

| 方法 | params | 应答 | 语义 |
|---|---|---|---|
| `session/start` | `{ sid, resume? }` | `result.ref` | 打开会话。`resume` 是壳逐字保存的 opaque 引用;**引用不可用必须 fail,不许静默重开新会话**。 |
| `config/set` | `{ sid, config }` | `result {}` 或 error | **首 prompt 前必调**(start 成功后)。config 至少含 `{ model?, thinkingLevel? }`,可含启用的连接器/skills。插件在成功应答前完成应用;无法应用→JSON-RPC error→**run 明确失败**,不许静默忽略、不许用 harness 默认蒙混。 |
| `session/prompt` | `{ sid, message, clientMessageId }` | turn 结束返回 | 跑一个用户 turn。`clientMessageId` 是壳生成的稳定身份,adapter **必须持久化**并以它作为该 user 消息在历史投影中的 ID(§3-C2)。 |
| `session/abort` | `{ sid }` | — | 停止当前 turn。 |
| `history/page` | `{ sid, beforeId?, limit }` | `{ messages[], hasMore }` | 分页读历史。**messages 形状 `{ id, role: user\|assistant, text, complete: true }`**,页内**旧→新**排列;无 `beforeId`=最近页;有 `beforeId`=锚点**之前**一页(**不含锚点**);锚点不存在=**error**,不许静默回第一页。 |

- **历史真相源**:adapter 在 `PRTS_AGENT_DATA_DIR` **自持久化 transcript 与 ID 映射**,不依赖 harness 原生会话文件;消息 ID 与文本**跨进程重启与 resume 不变**。
- `history/page` 只含**已完成**消息:`message_end` 已发出的 assistant 消息与已接纳的 user 消息必须可查。
- `sid` 格式 `s-{pid}-{n:x}`;v1 拓扑=一进程一会话,wire 已带 sid。
- 未知方法必须回 `-32601` error(不许静默吞掉)。

## 3. 核心合规面 B:事件(插件→壳 notification,必需)

`method: "event"`,params:`{ sid, data: { type, ... } }`。

| type | 载荷 | messageId 要求 |
|---|---|---|
| `turn_started` | — | — |
| `text_delta` | `{ messageId, text }` | **必需,非空** |
| `reasoning_delta` | `{ messageId, text }` | 必需 |
| `tool_started` / `tool_end` | 工具调用形状(harness 侧 `toolCallId` 起 ID,两端一致) | toolCallId 承担 |
| `message_end` | `{ messageId, role, text }` | **必需;`text`=该消息最终全文** |
| `turn_end` | 结束状态 | — |

- **ID 规则**:同一 turn 内每条 assistant 消息一个稳定 ID(delta 与 message_end 同 ID);跨 resume 不变。一个 turn 多条消息=多个 ID。
- user 消息**没有事件面**:其身份体现为 history 中 `id == clientMessageId` 的条目。壳发出的 user 卡与 history 条目由此天然合一。
- 白名单在 adapter 侧:合同外事件**必须丢弃**,不许透传垃圾。
- 合同外或缺必需 `messageId` 的事件:壳拒绝入账并记 adapter 违例计数(计入 conformance 判定)。

## 4. 审批(插件→壳 request)

- `method: "approval_need"`;壳应答 `{ approved, reason }`;`DEFAULT_APPROVAL_TIMEOUT`(**120 秒**)无应答=fail closed(`approved:false, "timeout"`)。
- 过期/未知 request_id 双向拒绝应答。壳侧注记(非合同):`prts_approval_request/expired/exit`。

## 5. 进程环境(壳→插件)

- `PRTS_AGENT_DATA_DIR`:每 agent 专属目录。**transcript、identity 映射、缓存必须写这里**,不许写插件代码旁。
- `PRTS_RUNTIME_COMMAND`:manifest `runtime.command` 解析成绝对路径后的 JSON argv。**插件不自己找运行时**——没有"系统全局安装"可回退,自己去找就把 pin 变成注释而不是事实。
- `PRTS_INSTALLED_EXTENSIONS_DIR`:hub 已为这个 harness 装好的扩展(装什么由 hub 的注册表决定,adapter 只负责按各自 harness 的布局摆好)。
- `PRTS_PRESETS_DIR`、`PRTS_CWD`、`PRTS_ADDITIONAL_DIRS`(JSON)、`PRTS_SESSION_ID`、`PRTS_HARNESS_CONFIG_SCOPE`(`system|private`)。
- 插件目录只读;harness 的安装位置由 manifest `runtime` 声明。

## 当前修订：会话内切换 core provider

用户修订：注入 provider 不再创建时锁定。现有 core PATCH session 接受
`modelProviderId` 和可选 `modelId`；未指定 modelId 时沿用已选模型。
在 turn 间执行 credentials/grant → config/set，保留 session ID 与原生 ref。
config/set 回报 `applied.modelProviderId`（请求身份）、`connectionId`（原生路由）、`model`。
下文历史条款中的“换 connection 必须新会话”不再适用于 core provider 切换；
harness 身份仍不能通过该操作改变。操作进行中不接纳新 turn，不终止正在运行的 turn 来强行切换。

## 6. 配置面(连接/凭据/认证/目录/生效)——capability `providers`

**这份配置面今天是 adapter 内部面**:方法在 `../contract/adapter-v1.json` 的 `providerSurface` 里声明,但 hub 没有任何路由调用它(hub 自己用一文件一 provider 管模型来源)。deepseek 实现了全套;记录在此,是为了它不被称为"hub 可达"。

产口红线:用户在壳内完成 发现→连接/登录→目录→选模型→生效,不得要求退出壳去操作其他 CLI。外部指引仅限故障排查。adapter 负责把通用信封翻译成本 harness 原生机制(RPC/非交互命令/配置文件),壳零特判。

### 6.1 方法(壳→adapter;全部必须在 `session/start` 之前可调,配置面专用进程实例不得创建聊天会话)

| 方法 | 入参 | 出参 | 红线 |
|---|---|---|---|
| `connections/schema` | `{}` | `{providers:[{id,displayName,authKind:api-key\|oauth-browser\|device-code,fields:[…],supportsCustomEndpoint,supportsCustomModels}]}` | fields 类型限白名单:text/secret/url/enum/boolean/number;禁止 HTML/脚本/命令/任意嵌套;不适用的字段不提供 |
| `connections/list` | `{}` | `{connections:[{id,providerId,label,status:connected\|needs-auth\|error\|disabled,revision,secretConfigured,endpoint?}]}` | **永不含凭据值**(value-free,对齐 dsh credentials.describe 语义) |
| `connections/validate` | `{draft:{providerId,fields:{}}}` | `{ok,fieldErrors:[{field,message}?],transportError?}` | 可有网络副作用,**绝不落盘** |
| `connections/save` | `{id?,providerId,label,fields,secretPolicy:keep\|replace\|clear,expectedRevision?}` | `{connection:{…},requires:none\|restart-session\|new-session}` | revision 乐观锁;原子替换,失败不损坏旧配置,保留未知字段;空 secret+keep ���得清除现值 |
| `connections/delete` | `{id,expectedRevision?}` | `{}` | 运行中冲突显式拒绝 |
| `auth/start` | `{providerId}` | `{operationId,next:{kind:browser,url}\|{kind:device-code,userCode,verifyUrl}}` | 壳拉起浏览器;完成由 adapter 跟踪 |
| `auth/status` | `{operationId}` | `{status:pending\|approved\|failed\|expired\|cancelled,error?}` | 可轮询;不依赖前端在线 |
| `auth/cancel` | `{operationId}` | `{}` | 幂等 |
| `models/list` | `{}` | `{models:[{id,connectionId,providerId,name?,available,unavailableReason?,reasoning?{efforts[],default?},contextWindow?,maxTokens?}],failures:[{providerId?,connectionId?,message}]}` | 空目录与读取失败分离;每条目携**真实 connectionId**(≠manifest id);一家失败不得污染他家 |
| `credentials/grant` | `{connectionId,value}` | `{}` | 仅请求方向;adapter 内存持有,重启需重授 |
| `config/set` 扩展 | `{…,connectionId?,model,configRevision?}` | `{applied:{connectionId,model,configRevision},requires}` | 会话创建时必调一次;**会话内仅允许换 model**(同连接内,turn 开始前完成应用,下 turn 生效);换 connectionId/harness → error `requires-new-session` → 壳开新会话。失败=错误=run 明确失败,绝不静默降级 |

### 6.1a 两层身份:Harness Adapter ≠ Model Provider
- **Harness Adapter**(pi/jouzu/deepseek)=提供 agent 运行时;**Model Provider**(deepseek/openai/自建网关…)=提供模型。UI/DTO/store 必须分开表达,禁止"deepseek harness"与"deepseek provider"混名;聊天窗显示 `via pi · deepseek · deepseek-chat`(harness · provider · model)。
- **壳是统一配置面**:AI providers 区维护 model provider 注册表(providerId/baseUrl/label),**密钥存 OS keychain(keyring crate),永不落系统文件明文**。用户配一次,按 harness 分发。
- **会话绑定边界**(用户定案,不搞复杂但不降级):会话创建时绑定 **harnessId + connectionId**,会话内不可切(换=**新会话**);但 **modelId 会话内可换**(同连接内换模型,下一 turn 生效,见 config/set 扩展)——四家原生都支持(pi `model.set`/codex `thread/settings/update`/dsh 逐 turn 覆盖/deepseek turn 开头读),砍掉它是降级。切换连接/harness/作用域一律新会话。

### 6.2 凭据流(单向下发 + 零明文落盘,对齐 dsh credentials 语义)
- 托管密钥的跨线只走 `credentials/grant`(内存注入,每次进程启动重授);`connections/save` 仅在首配时可携值一次。响应/事件/日志/错误一律 value-free;`secretConfigured`/掩码可报,原文不可回传回填。
- adapter 义务:**hub 托管的值任何情况下不得明文写盘**;落盘只写引用,四家原生免明文机制实测可用:pi=`{ENV_VAR}` 模板/`!command` 外部取值/`setRuntimeApiKey`(resolve-config-value.js L65-72);jouzu=捆绑同一 pi 运行时,语法同;codex=`[model_providers.x] env_key` 环境变量名;dsh="inherited process environment (read-only, wins)",env 优先于凭据文件。若未来某 harness 只能吃明文文件(当前无),schema 必须声明 `requiresPlaintextFile=true` 且 UI 明示后才落盘。
- 环境级凭据遮蔽时,writable=false,save 显式拒绝 `credential-shadowed`,不得"看起来保存成功"。
- 配置作用域是**每 harness 的用户显式选择**(`configScope: system | private`,存壳 store,UI 在 Control Center 的 harness 区):**默认 `system`(直接继承终端里已有的登录/上游,不得让用户困惑"终端配好了这里没")**;`private` 为可选隔离(首次可导现有配置)。壳经 `PRTS_HARNESS_CONFIG_SCOPE=system|private` 下发;adapter 据此决定是否设原生覆盖 env(`PI_CODING_AGENT_DIR`/`CODEX_HOME`/`JOUZU_HOME`);**env 缺席时 adapter 技术默认 `private`(确定性/测试安全),壳必须永远显式下发**。写入本身不需逐次弹窗:选择 `system` 即同意(切换作用域时需一次确认+说明影响面);UI 常驻展示当前作用域徒章。`system` 下外部改动绕过乐观锁,壳在操作前重拉 `connections/list` 并在 UI 标明"配置可能被外部工具修改"。

### 6.3 错误码(`error.data.code` 字符串,机读)
`revision-conflict` · `credential-shadowed` · `unknown-provider` · `unknown-model` · `validation-failed` · `auth-expired` · `busy-session-active` · `unsupported-for-provider` · `requires-new-session`（会话内试图换 connection/harness）。文案脱敏,不得含凭据值。

### 6.4 身份 DTO(线层面;迁移细则见执行计划 §3.1)
选择键 = `(harnessId=manifest id, connectionId(adapter 不透明 id), modelId)`。历史约束:旧 `connectionId` 曾被写作 manifest id,禁止直接解释为真实连接;重新匹配唯一才迁移,歧义让用户重选,不默认 pi。

### 6.5 saved ≠ validated
`connections/save` 成功仅表示已存储(codex 实证:伪造键也返回成功)。"可用"必须另走 `connections/validate` 或首次真实请求;UI 状态机禁止把 saved 渲染成 validated。

## 7. 数据归属

| 数据 | 归属 | 存储 |
|---|---|---|
| 项目/agent/harness identity/session 元数据/opaque ref | 壳 | domain store |
| **transcript + ID 映射(历史真相源)** | **adapter** | `PRTS_AGENT_DATA_DIR`(自持久化,跨 resume 稳定) |
| 连接器/skills 定义与启停 | 壳 | store;启用集经 `config/set` 推送 |
| 凭据(hub 托管) | **壳 OS keychain**(keyring crate) | 经 `credentials/grant` 内存注入 adapter;adapter 只写引用,**禁明文落盘** |
| 凭据(harness 自生,如 codex OAuth) | harness 自己的存储 | 只读使用,不回写不拷贝不显示 |

## 8. 【历史:Rust 时代】一致性套件(抽象的机器验收)

`tests/adapter_conformance.rs`:mock 参考 adapter 必须全绿(套件自洽证明);四家真实 adapter 各跑同一套件(真实 turn 子集在 harness 缺失时跳过并如实报告)。核心检查:

C1 config/set 必需:缺失/报错→run 失败,不发 prompt。C2 history user 条目 `id==clientMessageId`。C3 每个 delta/message_end 带非空 `messageId`,同 turn 多消息 ID 互异。C4 `message_end` 后 history 可查同 ID 同全文。C5 `beforeId` 页语义四红线。C6 重启+resume 后历史 ID 不变。C7 合同外事件不外漏。C8 缺 `messageId` 事件被壳侧识别为违例(mock 演示合规路径)。

**接入任何一家若需改壳代码=抽象缺陷,回炉壳,不加特判。**

## 9. 【历史:Rust 时代】现状记分(pass/fail,禁"待实现")

实测于 `adapter_conformance`(2026-09-09,真实 harness、真实 turn、CI 断言锁定):

| 家 | 事件面 | 审批 | C1 config | C2-6 identity/history | capabilities | 已知真实局限 |
|---|---|---|---|---|---|---|
| pi | pass | pass | pass | pass | models | — |
| codex | pass | pass | pass | pass | — | 模型名校验用固定清单(钉版本期间有效) |
| jouzu | pass | fail | pass | pass | — | 具名模型 = 显式拒绝(选择器未实现,fail-closed) |
| deepseek | pass | fail | pass | pass | — | 同上 |

审批列的 fail 是真实待办(四家审批路径未接完),不属于核心合规面装载门;装载门四家已过。

## 10. 【历史:已删除的 UI】壳→UI 统一投影(`agent-event`,UI 冻结)

形状不变:`{ agentId, runId, seq, payload }`,payload kinds 不变。**唯一内部变化**:卡片 ID 自 adapter `messageId`/`clientMessageId` 取得,壳不再造 UUID(DEC-6);UI 组件与交互不改。`seq` 按 agent 计数、跨 run 连续、**run 开始即持久化**(防 mid-run resync 回拨)。

审批语义不变:activity 呈现,120 秒 fail-closed。模型聚合(R-019)不变:枚举声明 `models` 能力位者逐家 `models/list`,保留 `connectionId` 身份,单家失败不吞别家、不回退。
