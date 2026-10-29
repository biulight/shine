# 混合加密安全审计 — 2026-10-25

## 范围与结论

- 基线：`release`，`90b9202150459cf2773dc7ef91a28fc513cff82f`。
- 方法：独立从生产调用链检查协议、子进程、收件人隔离、缓存、源快照和 broker
  授权，并运行现有测试及新增对抗性探针。这是 AI 代码安全复核，不是外部机构认证；
  不据此删除公开手册和 ADR 的“尚未经过独立安全审计”声明。
- 初始审计确认 **1 项 P2 安全缺陷**：输入快照的编码存在歧义，旧缓存可绕过已变化源文件的
  secret 声明校验。另确认 **1 项 P3 加固缺口**：age 解封装的 stderr 限制并非始终生效。
- 本次未发现可复现的 AEAD 伪造、额外 GPG 收件人解密、取消后切换后端或 broker
  未授权释放。测试通过不构成这些路径不存在其他缺陷的证明。
- **修复状态：A1、A2 及 CLI 回归脚本夹具已在
  [`9787bd28`](https://github.com/biulight/shine/commit/9787bd280434567b5902d9660f186b68e2640b18)
  修复，该提交已纳入本地 `v2.7.1` 标签。**
  下文保留初始基线的发现与复现证据；代码行号均指初始基线，修复与兼容性见文末。

## A1 / P2：快照哈希没有字段边界，允许不同源文件命中同一缓存

位置：[`snapshot_input_hash`](../../../cli/src/env/workspace/compilation.rs)
104–123 行；[`read_cache`](../../../cli/src/env/workspace/local_cache.rs) 157–176 行。

`snapshot_input_hash` 将版本、模式、workspace 字节以及每个路径和内容直接串接后做
SHA-256，没有长度前缀、来源数量或存在性标签。外层 `cache_context` 虽然使用了无歧义
序列化，却只能绑定已经发生歧义的 `input_hash`。

这不是 SHA-256 碰撞攻击：两种不同结构的输入产生的是**完全相同的待哈希字节串**。
设第二个源文件的绝对路径为 `P`：

```text
X = secret = {TOKEN = true}\n#
Y = \n[payload]\ndata = "<有效的 hybrid 密文>"\n

旧 first.toml  = #
旧 second.toml = X || P || Y

新 first.toml  = # || P || X
新 second.toml = Y
```

路径没有换行时，两组源文件都能通过 TOML 和缓存前的结构验证。旧输入声明了 `TOKEN`；
新输入中，该声明被移动到了注释内。`first` 的内容、`second` 的路径和内容拼接后均为
`# || P || X || P || Y`，workspace、模式和收件人无需改变。

### 实测结果

1. 旧输入首次运行正常解密并创建 age 本地缓存。
2. 改为新输入，再运行，哈希不变，仅调用一次 age 解缓存，仍向子进程提供 `TOKEN`。
3. 仅删除隔离目录中的该缓存，再运行完全相同的新输入，则报
   `secret key list does not match encrypted payload`，子进程不启动。

```text
input_hash_equal: True
cached_run_releases_removed_TOKEN: True
cached_run_trace: decrypt-age
without_cache_exit: 1
without_cache_key_mismatch: True
```

### 前提与影响

需要可控制源文件内容，以及已经为相应旧输入建立的缓存；探针使用构造的旧输入，
没有证明可对任意既有项目直接完成同样变换。攻击不需要修改缓存或知道数据密钥。
选中后端的解密及其授权仍会发生，不构成硬件授权或 broker 审批绕过。

影响是缓存绑定和当前源声明校验失效：当前输入本应拒绝运行，却仍可将旧 secret
传入子进程。该哈希函数也供普通环境缓存使用，因此修复应覆盖两类缓存。

### 建议修复与验收

- 对每个字段采用长度前缀或确定性结构化序列化；显式包含源数量和 missing/present
  标签，路径采用可区分原生路径的编码。
- 更新缓存域或版本，使旧哈希算法的缓存失效。仅改外层 context 的拼接不能解决问题。
- 将本报告构造加入生产调用链回归：新旧哈希不同，修改后不得读取旧缓存，错误输入
  必须在启动子进程前拒绝；同时覆盖普通缓存。

## A2 / P3：age 混合解封装会直接继承未经限制的 stderr

位置：[`decrypt_age_file`](../../../cli/src/secret/age.rs) 335–348 行，
[`read_key_output`](../../../cli/src/secret/exec.rs) 154–193 行。

GPG 解封装按 `key` 参数捕获 stderr；age 只按 `quiet_phone_progress` 判断是否捕获。
普通 age identity、Secure Enclave identity，以及显式交互路径会继承 stderr。
因此，虽然随后使用 `read_key_output`，它拿到的 `child.stderr` 为空，64 KiB 限制和
不回显诊断的处理都不会生效。

受控 age 替身输出 70,000 字节诊断并返回合法 key record 时，解密成功，全部诊断
原样到达调用者 stderr：

```text
age_unwrap_exit: 0
raw_stderr_bytes: 70000
unbounded_diagnostic_forwarded: True
```

这是输出边界与 ADR 0084 声明不一致的加固缺口。替身用于验证父进程的 I/O 路径，
**没有证明真实 age 或插件会泄露密钥**；可信本地工具被替换也不是本报告的攻击模型。
继承诊断可能污染日志或终端，不能声称该路径具备统一的有界、脱敏诊断保证。

建议默认按 `key` 捕获并限制诊断；QR 等确实需要交互的情形应明确规定例外及输出策略，
并分别测试普通 identity、Secure Enclave、phone 默认、QR 和用户显式启用消息的路径。

## 其他观察与已确认设计边界

- `inspect_identity_plugins` 将整个 identity 文件读入普通 `String` 后释放。对于原生
  age 私钥文件，这会在 Shine 内存中产生未主动清零的私钥副本。建议使用受限、清零的
  缓冲区；本次仅静态确认，没有执行内存恢复攻击，未计为远程可利用漏洞。
- 初始 key unwrap 的 `Zeroizing<Vec<u8>>` 从空容量增长；若要加强“所有密钥副本清零”的
  声明，应审查重分配，考虑预分配 `limit + 1` 或固定缓冲区。`Zeroizing` 不消除历史
  重分配副本，这也是 [zeroize 上游说明](https://docs.rs/zeroize/1.9.0/zeroize/#stackheap-zeroing-notes)
  明确区分的边界。本轮已为 unwrap stdout 预分配 `limit + 1`，stderr 使用固定清零缓冲区；
  这不代表整个进程或第三方工具的内存副本都已完成审计。
- 协议为 GPG **或** age 均可解开的同一数据密钥，不是双因素、双人批准或后量子组合。
  任一授权私钥泄露即影响相应旧密文。
- 两个 wrapper 与头部均作为 AEAD associated data；格式检查先于解封装，nonce 与
  解封装材料相互校验，选定后端失败即终止。真实工具测试验证了跨 wrapper 拼接失败。
- 公钥加密不证明整个密文的作者身份；完整替换、回滚与旧密文撤销不由 AEAD 单独解决。
  ADR 0085 已说明完整缓存替换的作者认证限制，本次未将其重复列为新漏洞。
- `shine.config.local.toml` 可改变本机后端选择是已声明的可信本地输入边界；broker
  不读取该文件，使用本机配置并在释放批准后解密。
- RustCrypto 的 [0.10.1 安全说明](https://docs.rs/chacha20poly1305/0.10.1/chacha20poly1305/#security-notes)
  记载其库级审计和处理器时序限制；这不覆盖 Shine 的自定义协议及缓存整合。

## 复现入口

复现材料：[`hybrid-audit-probes.py`](evidence/2026-10-25-hybrid/hybrid-audit-probes.py)。
探针目前默认检查修复结果。在仓库根目录执行：

```sh
rtk proxy python3 docs/kb/operations/evidence/2026-10-25-hybrid/hybrid-audit-probes.py target/debug/shine
```

如需对初始基线的二进制复现旧问题，在二进制路径后添加 `--expect-vulnerable`。

探针重用当前仓库 CLI 测试的隔离夹具，不包含真实密钥，也不会读取用户配置；工具替身
故意不执行公钥加密，只测试 Shine 的真实 envelope、缓存和子进程控制流。
真实工具互操作使用临时 GPG 密钥环和 age identity；这与工具替身的控制流证据不同，
不能将两类证据混淆。

## 修复与兼容性

### 实现与兼容性

| 项目 | 修复 | 回归证据 |
| --- | --- | --- |
| A1 / P2 | 快照使用 v2 域、固定宽度长度前缀、源数量、存在性标签和原生路径字节；普通与混合本地缓存版本均升为 2 | 哈希边界、missing/empty/字面量、顺序和非 UTF-8 路径测试；CLI 对两类缓存执行相同的声明移动攻击，均拒绝启动子进程 |
| 旧缓存 | 读取前拒绝 v1 缓存，重编译源文件 | 将有效缓存外层版本改为 1、保留匹配的 hash/context，CLI 仍重新编译并写缓存 |
| A2 / P3 | 两种工具解封装均 pipe stderr；默认丢弃，手机 QR/显式消息按同一 64 KiB 上限实时转发；超限终止并回收子进程，不回退 | 普通/SE/phone 默认、QR、消息 opt-in 路径覆盖成功、失败、恰好 64 KiB 和超限；QR 在工具等待输入前可见；stdout/stderr 超限进程均被回收 |
| CLI 夹具 | 缺少 `[env]` 时追加表，已有表时插入变量 | 对缺省和已有表配置均通过实际 `env secret decrypt` 验证字节精确 stdout |

不修改 hybrid 密文格式、源文件 schema、收件人集合或 broker 授权语义。升级后第一次
使用旧缓存需要重新解密源文件，因此可能出现逐源授权；不需要重新封存已有密文。
手机 QR 和显式消息是有意保留的交互输出，不承诺对这些可见消息进行脱敏；同样受总量限制。

### 关键证据与未验证边界

- macOS / aarch64 的 CLI 探针在初始基线复现 A1、A2；修复后，同一声明移动构造不再释放
  已删除声明的 secret，70,000 字节诊断被拒绝且不原样转发。完整回归入口为
  [`scripts/test-hybrid-secrets.py`](../../../scripts/test-hybrid-secrets.py)。
- `real_gpg_age_and_implicit_recipient_isolation` 使用真实 GnuPG 2.5.24 / age 1.3.2 与
  临时密钥，验证互操作、额外 GPG 收件人隔离和跨 wrapper 拼接失败；不使用用户真实密钥环。
- 本次专项审计未覆盖 Windows/Linux 原生复现、真实 Touch ID/手机设备/QR 扫码、长时间
  模糊测试、进程内存恢复或整个依赖树的最新漏洞数据库扫描。QR 实时性证据来自受控工具，
  不代表硬件授权验收，也不构成第三方密码学证明或完整平台认证。
